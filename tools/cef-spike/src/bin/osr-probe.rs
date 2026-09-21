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

#[path = "../mac/mod.rs"]
mod mac;
#[path = "../shared/mod.rs"]
pub mod shared;

use cef::*;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::sync::OnceLock;

// IOSurface 框架:osr_host.m 里读共享纹理像素要用它;spike 无对应 crate,
// 且 build.rs 的 link 指令到不了 bin 的链接行(见 build.rs 注释),故在此显式声明。
#[link(name = "IOSurface", kind = "framework")]
extern "C" {
    fn IOSurfaceGetWidth(surface: *mut c_void) -> usize;
}

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

            if external_begin_frame() {
                let rate = CONFIG.get().map(|config| config.frame_rate).unwrap_or(60).max(1);
                unsafe { osr_start_begin_frame_timer(1.0 / f64::from(rate), begin_frame_trampoline) };
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
        "[osr] 透明开关={} 外部 begin-frame={}",
        transparent_probe(),
        external_begin_frame()
    );

    // 顺序约束:load_cef 会断言 NSApp 之前未被触碰(cef-rs mac 模块),故必须最先执行。
    let _library = shared::load_cef();
    unsafe {
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
    // windowless_rendering_enabled 是**进程级**开关,必须在 initialize 之前设置。
    let settings = Settings {
        no_sandbox: 1,
        windowless_rendering_enabled: 1,
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
