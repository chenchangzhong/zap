//! T1 探针:CEF **windowless(OSR)** + IOSurface → CALayer,验证"能否真透明"。
//!
//! 与 windowed 探针(`hole-probe`)共用同一套洞拓扑与采样口径(probes/hole_probe.m):
//! 底层 ProbeBackgroundView(#12171F) → 容器 → 覆盖层(洞内透明、洞外 #212E42)。
//! 区别只有一处:网页像素不是 CEF 自己画进子视图,而是
//! `on_accelerated_paint` → IOSurface → 我们自建 NSView 的 `CALayer.contents`。
//!
//! 判据(配合 probes/sample_pixel):
//! - 透明页面 + `background_color` alpha=0(见 `PROBE_TRANSPARENT`/`/tmp/probe_transparent`)
//!   ⇒ 洞内应采样到 **#12171F**(下层背景透出);
//! - 若仍为 **#FFFFFF** ⇒ windowed 那套"强制不透明白"在 OSR 下依旧,本路线失败;
//! - 对照组:不设透明开关时 `background_color` 为不透明深灰,洞内应是该灰色(证明内容确实渲染)。
//!
//! 用法:
//!   osr-probe --url=<url> [--hole=x,y,w,h] [--exit-after=12] [--log=/tmp/osr.log]
//!             [--click-after=6] [--frame-rate=60]
//! 环境变量:
//!   PROBE_TRANSPARENT=1(或 touch /tmp/probe_transparent) 透明背景
//!   PROBE_OSR_EXTERNAL_BEGIN_FRAME=1 启用 external_begin_frame(需宿主按刷新节奏驱动)
//!   PROBE_OSR_FRAME_RATE=<n> 覆盖 windowless_frame_rate(默认 60)
//!   PROBE_IME_SELFTEST=1 直接调用 NSTextInputClient(确定性验证 ObjC→Rust→CEF→页面)
//!   PROBE_IME_KEYTEST=1  合成 "nihao"+空格 按键,走真实输入法

#[path = "../mac/mod.rs"]
mod mac;
#[path = "../shared/mod.rs"]
pub mod shared;

use cef::*;
use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::OnceLock;

// IOSurface 框架:osr_host.m 里读共享纹理像素要用它;spike 无对应 crate,
// 且 build.rs 的 link 指令到不了 bin 的链接行(见 build.rs 注释),故在此显式声明。
#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceGetWidth(surface: *mut c_void) -> usize;
}

// Carbon(HIToolbox):osr_host.m 打印当前输入源(TISCopyCurrentKeyboardInputSource)要用它。
#[link(name = "Carbon", kind = "framework")]
extern "C" {}

// 链接 build.rs 用 cc 编译出的 ObjC 静态库(hole_probe.m 的洞拓扑 + osr_host.m 的 OSR 宿主视图)。
#[link(name = "hole_probe_host", kind = "static")]
extern "C" {
    fn probe_create_window(hx: f64, hy: f64, hw: f64, hh: f64);
    fn probe_container_ptr() -> *mut c_void;
    fn probe_set_routing(mode: i32);
    fn probe_schedule_click(delay: f64);
    fn probe_schedule_exit(delay: f64);
    fn probe_window_number() -> i64;
    fn probe_redirect_stdout(path: *const std::ffi::c_char);
    fn probe_dump_subviews();

    fn osr_host_view_new(container: *mut c_void, x: f64, y: f64, w: f64, h: f64) -> *mut c_void;
    fn osr_host_view_set_surface(view: *mut c_void, surface: *mut c_void);
    fn osr_host_view_dump_layer(view: *mut c_void);
    fn osr_host_view_scale(view: *mut c_void) -> f64;
    fn osr_surface_dump(view: *mut c_void);
    fn osr_start_begin_frame_timer(interval: f64, callback: extern "C" fn());

    // T2:IME 链路(宿主 NSView 实现 NSTextInputClient → 转发到 CefBrowserHost::ime_*)。
    fn osr_host_view_set_ime_callbacks(callbacks: *const OsrImeCallbacks);
    fn osr_host_view_set_ime_bounds(
        view: *mut c_void,
        sel_from: i32,
        sel_to: i32,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        has_bounds: i32,
    );
    fn osr_host_view_focus(view: *mut c_void);
    fn osr_schedule_ime_selftest(delay: f64);
    fn osr_schedule_ime_keytest(delay: f64);
}

/// 洞位置与目标 URL:`on_context_initialized` 在 CEF 回调里取用,故用全局量传递。
struct ProbeConfig {
    url: String,
    hole: (f64, f64, f64, f64),
    frame_rate: i32,
}
static CONFIG: OnceLock<ProbeConfig> = OnceLock::new();

/// 自建 OSR 宿主视图(容器坐标 = 洞的位置)。
static HOST_VIEW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
// 主线程持有的 browser 句柄(外部 begin-frame 回调用)。
thread_local! {
    static BROWSER: RefCell<Option<Browser>> = const { RefCell::new(None) };
}
static VIEW_RECT_LOGGED: AtomicBool = AtomicBool::new(false);
static PAINT_COUNT: AtomicU64 = AtomicU64::new(0);
static CPU_PAINT_LOGGED: AtomicBool = AtomicBool::new(false);
static POPUP_LOGGED: AtomicBool = AtomicBool::new(false);

fn arg_value(name: &str) -> Option<String> {
    std::env::args()
        .find_map(|arg| arg.strip_prefix(&format!("{name}=")).map(ToString::to_string))
}

/// 透明实验开关:环境变量或落文件标记(`open` 启动时环境变量不生效)。
fn transparent_probe() -> bool {
    std::env::var_os("PROBE_TRANSPARENT").is_some()
        || std::path::Path::new("/tmp/probe_transparent").exists()
}

fn external_begin_frame() -> bool {
    std::env::var_os("PROBE_OSR_EXTERNAL_BEGIN_FRAME").is_some()
}

/// T2 自检开关(与透明开关同口径:`open` 启动时环境变量不生效,可落文件)。
fn ime_selftest() -> bool {
    std::env::var_os("PROBE_IME_SELFTEST").is_some()
        || std::path::Path::new("/tmp/probe_ime_selftest").exists()
}

fn ime_keytest() -> bool {
    std::env::var_os("PROBE_IME_KEYTEST").is_some()
        || std::path::Path::new("/tmp/probe_ime_keytest").exists()
}

/// ObjC 宿主视图 → Rust 的 IME 回调表(见 `probes/osr_host.m`)。
/// 所有回调都在主线程被调用,与 `BROWSER` 所在线程一致。
#[repr(C)]
struct OsrImeCallbacks {
    set_composition: extern "C" fn(*const c_char, i32, i32, i32, i32),
    commit_text: extern "C" fn(*const c_char, i32, i32),
    finish_composing: extern "C" fn(i32),
    cancel_composition: extern "C" fn(),
}

static IME_CALLBACKS: OsrImeCallbacks = OsrImeCallbacks {
    set_composition: ime_set_composition_trampoline,
    commit_text: ime_commit_text_trampoline,
    finish_composing: ime_finish_composing_trampoline,
    cancel_composition: ime_cancel_composition_trampoline,
};

/// 在主线程持有的 browser 宿主上执行(所有 IME 转发都发生在主线程)。
/// 返回是否真的拿到了宿主 —— 顺便当"调用有没有落到 CEF"的探针。
fn with_host(action: impl FnOnce(&BrowserHost)) -> bool {
    BROWSER.with(|slot| {
        if let Some(browser) = slot.borrow().as_ref() {
            if let Some(host) = browser.host() {
                action(&host);
                return true;
            }
        }
        println!("[osr] ⚠ 拿不到 BrowserHost,本次 IME 调用被丢弃");
        false
    })
}

/// UTF-16 选区 → CEF `Range`;ObjC 侧用 -1 表示"没有要替换的既有文本"(NSNotFound)。
///
/// **不能用 NULL**:CEF 的 capi 对可空 `cef_range_t*` 会退化成默认值 `(0,0)`,渲染器随即
/// 把它当成"把选区设到 0..0"去 `SelectRange`,在 `<textarea>` 这类没有文档级编辑器的页面上
/// 会打断焦点,导致整条 composition 被静默丢弃(实测:composition 完全不上屏)。
/// CEF 自带的 mac 客户端同样用 `CefRange::InvalidRange()`(两个 0xFFFFFFFF)表示"无替换"。
fn replacement_range(from: i32, to: i32) -> Range {
    if from < 0 {
        Range {
            from: u32::MAX,
            to: u32::MAX,
        }
    } else {
        Range {
            from: from as u32,
            to: to.max(from) as u32,
        }
    }
}

/// composition 更新:`NSTextInputClient.setMarkedText` → `ime_set_composition`。
/// CEF 的 range 与 NSRange 同为 **UTF-16 码元**口径,故文本长度也按 `encode_utf16` 计。
extern "C" fn ime_set_composition_trampoline(
    text: *const c_char,
    sel_from: i32,
    sel_to: i32,
    rep_from: i32,
    rep_to: i32,
) {
    if text.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    if text.is_empty() {
        // 空串=清空 composition,ObjC 侧已改走 ime_cancel_composition。
        return;
    }
    let utf16_len = text.encode_utf16().count() as u32;
    println!(
        "[osr] ime_set_composition text={text:?} sel={sel_from}..{sel_to} rep={rep_from}..{rep_to} \
         utf16_len={utf16_len}"
    );
    let cef_text = CefString::from(text.as_str());
    // Blink 的 composition 至少需要一条下划线信息(参考实现同款全串实线下划线)。
    let underline = CompositionUnderline {
        size: std::mem::size_of::<CompositionUnderline>(),
        range: Range {
            from: 0,
            to: utf16_len,
        },
        color: 0xFF00_0000,
        background_color: 0,
        thick: 0,
        style: CompositionUnderlineStyle::SOLID,
    };
    let selection = Range {
        from: sel_from.max(0) as u32,
        to: sel_to.max(0) as u32,
    };
    let replacement = replacement_range(rep_from, rep_to);
    with_host(|host| {
        host.ime_set_composition(
            Some(&cef_text),
            Some(&[underline]),
            Some(&replacement),
            Some(&selection),
        );
    });
}

/// 上屏:`NSTextInputClient.insertText` → `ime_commit_text`。
extern "C" fn ime_commit_text_trampoline(text: *const c_char, rep_from: i32, rep_to: i32) {
    if text.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned();
    if text.is_empty() {
        return;
    }
    println!("[osr] ime_commit_text text={text:?} rep={rep_from}..{rep_to}");
    let cef_text = CefString::from(text.as_str());
    let replacement = replacement_range(rep_from, rep_to);
    // relative_cursor_pos=0 ⇒ 光标落在提交文本末尾(Chromium 的
    // InputMethodController::ComputeAbsoluteCaretPosition = 起点 + 长度 + relative)。
    with_host(|host| host.ime_commit_text(Some(&cef_text), Some(&replacement), 0));
}

/// `NSTextInputClient.unmarkText` → `ime_finish_composing_text`。
extern "C" fn ime_finish_composing_trampoline(keep_selection: i32) {
    println!("[osr] ime_finish_composing_text keep_selection={keep_selection}");
    with_host(|host| host.ime_finish_composing_text(keep_selection));
}

/// 空 composition → `ime_cancel_composition`。
extern "C" fn ime_cancel_composition_trampoline() {
    println!("[osr] ime_cancel_composition");
    with_host(|host| host.ime_cancel_composition());
}

fn hole() -> (f64, f64, f64, f64) {
    CONFIG.get().expect("config not set").hole
}

fn scale() -> f64 {
    let view = HOST_VIEW.load(Ordering::SeqCst);
    if view.is_null() {
        2.0
    } else {
        unsafe { osr_host_view_scale(view) }
    }
}

wrap_app! {
    struct OsrApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(OsrBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct OsrBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        /// OSR 浏览器在 context 就绪后创建(与官方模式一致:首个浏览器在
        /// OnContextInitialized 里建)。
        fn on_context_initialized(&self) {
            let config = CONFIG.get().expect("config not set");
            let (hx, hy, hw, hh) = config.hole;

            // 1) 自建宿主视图,放进 probe 容器的洞位置(与 windowed 探针的 CEF 子视图同口径)。
            let container = unsafe { probe_container_ptr() };
            let host_view = unsafe { osr_host_view_new(container, hx, hy, hw, hh) };
            HOST_VIEW.store(host_view, Ordering::SeqCst);
            let scale = unsafe { osr_host_view_scale(host_view) };
            // 证据:容器里应当出现我们自建的宿主视图(与 windowed 探针的 CefBrowserHostView 同位置)。
            unsafe { probe_dump_subviews() };

            // 2) windowless 浏览器:三开关 + 帧率;背景色决定透明与否。
            let mut window_info = WindowInfo {
                windowless_rendering_enabled: 1,
                shared_texture_enabled: 1,
                bounds: Rect {
                    x: 0,
                    y: 0,
                    width: (hw * scale).round() as i32,
                    height: (hh * scale).round() as i32,
                },
                runtime_style: RuntimeStyle::ALLOY,
                ..Default::default()
            };
            if external_begin_frame() {
                window_info.external_begin_frame_enabled = 1;
            }

            // windowless 下 alpha=0 即"启用透明绘制";不设开关时用不透明深灰做对照。
            let background = if transparent_probe() { 0 } else { 0xFF20_2020 };
            let settings = BrowserSettings {
                windowless_frame_rate: config.frame_rate,
                background_color: background,
                ..Default::default()
            };

            let mut client = Some(OsrClient::new());
            let url = CefString::from(config.url.as_str());
            let created = browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url),
                Some(&settings),
                None,
                None,
            );
            println!(
                "[osr] browser created windowless={} shared_texture={} external_begin_frame={} \
                 bg=0x{background:08X} frame_rate={} create_ret={created}",
                window_info.windowless_rendering_enabled,
                window_info.shared_texture_enabled,
                window_info.external_begin_frame_enabled,
                config.frame_rate
            );
            // client 由 CEF 持引用,这里显式泄漏以防 Rust 侧提前释放。
            std::mem::forget(client);
        }
    }
}

wrap_client! {
    struct OsrClient;

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(OsrRenderHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(OsrLifeSpanHandler::new())
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(OsrLoadHandler::new())
        }
    }
}

wrap_load_handler! {
    struct OsrLoadHandler;

    impl LoadHandler {
        /// T2:焦点必须在**导航完成后**再给一次。on_after_created 时页面还在 about:blank,
        /// 导航会换掉 render widget,此前 RWHV 上的 is_active_/focus 状态不会带过去,
        /// 结果是 ime_set_composition 因 ShouldRouteEvents() 为假被静默丢弃(实测得到)。
        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            println!("[osr] on_load_end status={http_status_code}");
            let Some(browser) = browser else {
                return;
            };
            if let Some(host) = browser.host() {
                // 焦点同步(实测通过的那一组):沿用 CEF 自带 cefclient 的 Show() 顺序
                // (先 WasHidden(false) 再 SetFocus(true)),并显式关/开一次 focus,
                // 强制走一遍完整的状态变更消息 —— CEF 在导航后会**静默丢掉**焦点
                // (chromiumembedded/cef#3870),不补这一次,IME 与光标都会失效。
                host.was_hidden(0);
                host.set_focus(0);
                host.set_focus(1);
            }
            let view = HOST_VIEW.load(Ordering::SeqCst);
            unsafe { osr_host_view_focus(view) };
        }
    }
}

wrap_life_span_handler! {
    struct OsrLifeSpanHandler;

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser else {
                return;
            };
            BROWSER.with(|slot| *slot.borrow_mut() = Some(browser.clone()));
            println!("[osr] on_after_created");

            // T2 前置:windowless 下浏览器没有自己的原生视图去抢焦点,必须由我们让自建
            // 宿主视图成为 first responder,并显式告诉 CEF 浏览器已获焦(否则输入法不起来、
            // 页面里的 <textarea> 也不会被聚焦)。
            if let Some(host) = browser.host() {
                host.set_focus(1);
            }
            let view = HOST_VIEW.load(Ordering::SeqCst);
            unsafe { osr_host_view_focus(view) };

            if external_begin_frame() {
                let rate = CONFIG.get().map(|config| config.frame_rate).unwrap_or(60).max(1);
                unsafe { osr_start_begin_frame_timer(1.0 / f64::from(rate), begin_frame_trampoline) };
            }
            // T2 自检(页面加载后再跑):A=直接调用 NSTextInputClient,B=合成按键走真实输入法。
            if ime_selftest() {
                unsafe { osr_schedule_ime_selftest(2.0) };
            }
            if ime_keytest() {
                unsafe { osr_schedule_ime_keytest(2.0) };
            }
        }

        /// 由客户端完成关闭(与 zap 的 CEF 后端同策略:避免把关闭转发给顶层窗口)。
        fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
            1
        }

        fn on_before_close(&self, _browser: Option<&mut Browser>) {
            println!("[osr] on_before_close");
            let view = HOST_VIEW.load(Ordering::SeqCst);
            if !view.is_null() {
                unsafe { osr_host_view_dump_layer(view) };
                unsafe { osr_surface_dump(view) };
            }
        }
    }
}

/// 外部 begin-frame 的宿主驱动回调(NSTimer 在主 run loop 上调用)。
extern "C" fn begin_frame_trampoline() {
    BROWSER.with(|slot| {
        if let Some(browser) = slot.borrow().as_ref() {
            if let Some(host) = browser.host() {
                host.send_external_begin_frame();
            }
        }
    });
}

wrap_render_handler! {
    struct OsrRenderHandler;

    impl RenderHandler {
        /// 视图尺寸:**DIP(逻辑点)**,不是像素 —— CEF 自己会乘 `device_scale_factor`。
        /// 若这里返回像素、`screen_info` 又报 scale=2,就会**重复缩放**(实测 surface 变成
        /// 2400x1600 而非 1200x800,浪费显存且可能模糊)。
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            let Some(rect) = rect else {
                return;
            };
            let (_, _, w, h) = hole();
            rect.x = 0;
            rect.y = 0;
            rect.width = w.round() as i32;
            rect.height = h.round() as i32;
            if !VIEW_RECT_LOGGED.swap(true, Ordering::SeqCst) {
                println!(
                    "[osr] view_rect={}x{} DIP (scale={}, 洞={w}x{h} 点)",
                    rect.width,
                    rect.height,
                    scale()
                );
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> i32 {
            let Some(info) = screen_info else {
                return 0;
            };
            let (_, _, w, h) = hole();
            let scale = scale();
            info.device_scale_factor = scale as f32;
            info.depth = 32;
            info.depth_per_component = 8;
            info.is_monochrome = 0;
            // rect/available_rect 同样按 CEF 约定用 DIP。
            info.rect = Rect {
                x: 0,
                y: 0,
                width: w.round() as i32,
                height: h.round() as i32,
            };
            info.available_rect = Rect {
                x: info.rect.x,
                y: info.rect.y,
                width: info.rect.width,
                height: info.rect.height,
            };
            1
        }

        /// 加速绘制:macOS 上是 IOSurface 指针 → 直接贴进 CALayer.contents(零拷贝)。
        fn on_accelerated_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            info: Option<&AcceleratedPaintInfo>,
        ) {
            let Some(info) = info else {
                return;
            };
            let surface = info.shared_texture_io_surface;
            if type_ == PaintElementType::POPUP {
                if !POPUP_LOGGED.swap(true, Ordering::SeqCst) {
                    println!("[osr] on_accelerated_paint(POPUP) surface={surface:p}(T1 不处理弹层)");
                }
                return;
            }
            let view = HOST_VIEW.load(Ordering::SeqCst);
            unsafe { osr_host_view_set_surface(view, surface) };
            let count = PAINT_COUNT.fetch_add(1, Ordering::SeqCst) + 1;
            if count == 1 || count % 60 == 0 {
                println!(
                    "[osr] accelerated paint #{count} surface={surface:p} view={view:p} \
                     dirty={}",
                    _dirty_rects.map(<[Rect]>::len).unwrap_or(0)
                );
                if count == 1 {
                    unsafe { osr_host_view_dump_layer(view) };
                }
                // 第 120 帧读回 IOSurface 像素(内容/alpha 的硬证据,不受窗口遮挡影响)。
                if count == 120 {
                    unsafe { osr_surface_dump(view) };
                }
            }
        }

        /// CPU 兜底:OSR 若拿不到共享纹理会走这里(T1 只记录,不做位图上传)。
        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            _buffer: *const u8,
            width: i32,
            height: i32,
        ) {
            if !CPU_PAINT_LOGGED.swap(true, Ordering::SeqCst) {
                println!("[osr] on_paint(CPU 兜底) type={type_:?} {width}x{height} —— 未实现上传");
            }
        }

        fn on_popup_show(&self, _browser: Option<&mut Browser>, show: i32) {
            println!("[osr] on_popup_show show={show}(T1 不处理)");
        }

        fn on_popup_size(&self, _browser: Option<&mut Browser>, rect: Option<&Rect>) {
            if let Some(rect) = rect {
                println!(
                    "[osr] on_popup_size {}x{} @({},{})",
                    rect.width, rect.height, rect.x, rect.y
                );
            }
        }

        /// T2:CEF 回报 composition 的选区与逐字位置(DIP、**左上原点**),
        /// 宿主侧缓存后用于 `firstRectForCharacterRange:` 给候选框定位。
        fn on_ime_composition_range_changed(
            &self,
            _browser: Option<&mut Browser>,
            selected_range: Option<&Range>,
            character_bounds: Option<&[Rect]>,
        ) {
            let selected = selected_range.map(|range| (range.from, range.to));
            let first = character_bounds.and_then(|bounds| bounds.first()).cloned();
            println!(
                "[osr] on_ime_composition_range_changed sel={selected:?} bounds={} {:?}",
                character_bounds.map(<[Rect]>::len).unwrap_or(0),
                first
            );
            let (sel_from, sel_to) = selected
                .map(|(from, to)| (from as i32, to as i32))
                .unwrap_or((-1, -1));
            let (x, y, w, h, has_bounds) = match &first {
                Some(rect) => (
                    f64::from(rect.x),
                    f64::from(rect.y),
                    f64::from(rect.width),
                    f64::from(rect.height),
                    1,
                ),
                None => (0.0, 0.0, 0.0, 0.0, 0),
            };
            let view = HOST_VIEW.load(Ordering::SeqCst);
            unsafe {
                osr_host_view_set_ime_bounds(view, sel_from, sel_to, x, y, w, h, has_bounds)
            };
        }
    }
}

fn main() {
    let url = arg_value("--url").unwrap_or_else(|| "file:///tmp/hole/index.html".to_string());
    let routing = arg_value("--routing").unwrap_or_else(|| "plain".to_string());
    let hole_arg = arg_value("--hole").unwrap_or_else(|| "100,100,600,400".to_string());
    let click_after: f64 = arg_value("--click-after")
        .and_then(|value| value.parse().ok())
        .unwrap_or(6.0);
    let exit_after: f64 = arg_value("--exit-after")
        .and_then(|value| value.parse().ok())
        .unwrap_or(12.0);
    let frame_rate: i32 = arg_value("--frame-rate")
        .or_else(|| std::env::var("PROBE_OSR_FRAME_RATE").ok())
        .and_then(|value| value.parse().ok())
        .unwrap_or(60);

    let parts: Vec<f64> = hole_arg
        .split(',')
        .filter_map(|value| value.trim().parse().ok())
        .collect();
    let (hx, hy, hw, hh) = match parts.as_slice() {
        [x, y, w, h] => (*x, *y, *w, *h),
        _ => panic!("--hole 需为 x,y,w,h"),
    };

    CONFIG
        .set(ProbeConfig {
            url,
            hole: (hx, hy, hw, hh),
            frame_rate,
        })
        .ok();

    if let Some(log) = arg_value("--log") {
        let path = std::ffi::CString::new(log).expect("log path");
        unsafe { probe_redirect_stdout(path.as_ptr()) };
    }

    println!(
        "[osr] 透明开关={} 外部 begin-frame={} IME 自检A={} 自检B={}",
        transparent_probe(),
        external_begin_frame(),
        ime_selftest(),
        ime_keytest()
    );

    // 顺序约束:load_cef 会断言 NSApp 之前未被触碰(cef-rs mac 模块),故必须最先执行。
    let _library = shared::load_cef();
    unsafe {
        // T2:宿主视图的 IME 回调必须在浏览器创建前挂好(setMarkedText 等随时可能被调)。
        osr_host_view_set_ime_callbacks(&IME_CALLBACKS);
        probe_create_window(hx, hy, hw, hh);
        let mode = match routing.as_str() {
            "plain" => 0,
            "zap" => 1,
            "embedded" => 2,
            other => panic!("--routing 需为 plain|zap|embedded,收到 {other}"),
        };
        probe_set_routing(mode);
    }
    println!("[probe] WINDOW_NUMBER={}", unsafe { probe_window_number() });

    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        panic!("Failed to parse command line arguments");
    };
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;
    let ret = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if is_browser_process {
        assert_eq!(ret, -1, "cannot execute browser process");
    } else {
        return;
    }

    let mut app = OsrApp::new();
    // 探针的 CEF 缓存根默认落在 ~/Library/Application Support/CEF/User Data,需要家目录写权限
    // (受限环境下会被拒),且会与其他 CEF 实例抢进程 singleton。改放系统临时目录,让探针自包含。
    let cache_dir = arg_value("--cache-dir")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("cef-spike-osr-cache"));
    std::fs::create_dir_all(&cache_dir).ok();
    println!("[osr] root_cache_path={}", cache_dir.display());
    // windowless_rendering_enabled 是**进程级**开关,必须在 initialize 之前设置。
    let settings = Settings {
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
        root_cache_path: CefString::from(cache_dir.to_string_lossy().as_ref()),
        ..Default::default()
    };
    assert_eq!(
        initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        ),
        1
    );

    unsafe {
        probe_schedule_click(click_after);
        probe_schedule_exit(exit_after);
    }
    run_message_loop();
    // 退出前再读一次共享纹理:即使没到第 120 帧也保证有像素证据。
    let view = HOST_VIEW.load(Ordering::SeqCst);
    if !view.is_null() {
        unsafe { osr_surface_dump(view) };
    }
    shutdown();
}
