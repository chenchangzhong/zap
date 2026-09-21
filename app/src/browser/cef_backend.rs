//! CEF(Chromium)后端骨架:仅在 `cef_webview` feature 下编译(macOS)。
//!
//! 目标(specs/cef-webview-minimal/TECH.md 阶段 1):用 CEF 承载 dsh pane 的 webview,
//! 替代 wry/WKWebView。本文件当前只提供**运行时骨架**——初始化、消息泵、以子视图方式
//! 创建浏览器;尚未接入 `BrowserWebViewManager`/`DshPane`(下一步)。
//!
//! 与 zap 事件循环的关系(集成要求 #4,见 TECH.md):
//! zap 已有自己的事件循环,故 CEF 走 `external_message_pump = 1` + 主线程稳定周期
//! 调用 [`pump`];**不能**只依赖"有帧才跑"的 on_frame_drawn。
//! 宿主契约(集成要求 #3):CEF 要求进程的 NSApplication 实现 `CefAppProtocol`
//! (`isHandlingSendEvent`/`setHandlingSendEvent:`),zap 目前没有 —— 需在启用本后端时
//! 用运行时 category/swizzle 补齐(spec 已记录,未实现)。

use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::OnceLock;

use cef::*;

use crate::features::FeatureFlag;

/// CEF 泵间隔:60Hz。CEF 官方无固定值;过密浪费 CPU,过疏会拖慢流式更新。
const PUMP_INTERVAL_SECONDS: f64 = 1.0 / 60.0;

extern "C" {
    /// 见 app/src/platform/mac/objc/cef_support.m(仅 cef_webview feature 下编译)。
    fn warp_cef_start_periodic_main_timer(interval: f64, callback: extern "C" fn());
    fn warp_cef_install_app_protocol_support();
}

extern "C" fn pump_trampoline() {
    pump();
}

/// 安装 NSApplication 协议桥(集成要求 #3)。须在创建任何浏览器前、主线程调用。
pub(crate) fn install_app_protocol_support() {
    unsafe { warp_cef_install_app_protocol_support() };
}

/// 启动主线程周期泵(集成要求 #4):zap 事件循环空闲时没有帧,必须靠独立定时器
/// 推进 CEF 的消息循环。
pub(crate) fn start_pump() {
    unsafe { warp_cef_start_periodic_main_timer(PUMP_INTERVAL_SECONDS, pump_trampoline) };
}

/// CEF 是否已完成初始化(进程内一次性)。
static INITIALIZED: OnceLock<bool> = OnceLock::new();

/// 已加载的 CEF 动态库(进程生命周期持有;macOS 从 .app 内 framework 加载)。
/// 必须在 `execute_process` / `initialize` **之前**完成加载,否则 CEF 的 C API
/// 是空指针表 —— 实测在 `execute_process` 处直接 SIGSEGV。
static LIBRARY: OnceLock<cef::library_loader::LibraryLoader> = OnceLock::new();
/// 动态库是否加载成功(OnceLock 里存的是 loader,不带状态查询 API)。
static LIBRARY_LOADED: OnceLock<bool> = OnceLock::new();

/// 当前进程是否是 CEF 子进程(以 `--type=<process>` 启动)。
///
/// 必须在**加载 CEF 库之前**判断:子进程要用 helper 模式加载 framework
/// (helper 的 framework 在**外层 app** 的 Contents/Frameworks 下,非 helper 模式
/// 会在 helper 自己的 bundle 里找 → 找不到 → GPU/Renderer 启动即失败)。
fn is_cef_subprocess() -> bool {
    std::env::args().any(|arg| arg.starts_with("--type="))
}

/// 加载 CEF 动态库 + 初始化 API hash(幂等)。返回是否成功。
///
/// `helper=true` 用于子进程:库加载按"外层 app"解析(CefScopedLibraryLoader 的
/// helper 路径),这是 CEF 多进程布局的硬要求。
fn load_library(helper: bool) -> bool {
    let loaded = *LIBRARY_LOADED.get_or_init(|| {
        let loader = cef::library_loader::LibraryLoader::new(
            &std::env::current_exe().expect("current exe"),
            helper,
        );
        let loaded = loader.load();
        if !loaded {
            log::error!("[cef] failed to load Chromium Embedded Framework (需以 .app 形态运行)");
        }
        let _ = LIBRARY.set(loader);
        loaded
    });
    if !loaded {
        return false;
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    true
}

/// 子进程启动前置:CEF 的多进程模型要求每个子进程在这里分流执行。
/// 返回值:`true` 表示当前是 browser 进程,应继续正常启动流程;
/// `false` 表示这是 CEF 子进程(已处理完毕,调用方应立即退出)。
pub(crate) fn handle_subprocess_or_continue() -> bool {
    // 顺序关键:先加载库并初始化 API hash,再 execute_process(spike 已验证;
    // 反过来会因 C API 表为空而崩溃)。子进程判定只能靠命令行,不能靠 CEF API。
    let is_subprocess = is_cef_subprocess();
    if !load_library(is_subprocess) {
        // 子进程分支不能"继续正常启动":那会走到 ChannelState::new,而 helper 的
        // 带后缀 bundle id 会命中 AppId 三段断言而 **panic**(评审 F5)。
        if is_subprocess {
            log::error!("[cef] helper 子进程无法加载 CEF framework,直接退出");
            std::process::exit(1);
        }
        return true;
    }
    let args = cef::args::Args::new();
    let Some(cmd_line) = args.as_cmd_line() else {
        log::error!("[cef] failed to parse command line arguments");
        if is_subprocess {
            std::process::exit(1);
        }
        return true;
    };
    let switch = CefString::from("type");
    let is_browser_process = cmd_line.has_switch(Some(&switch)) != 1;
    let ret = execute_process(Some(args.as_main_args()), None, std::ptr::null_mut());
    if is_browser_process {
        debug_assert_eq!(ret, -1, "browser process must not be a CEF subprocess");
        true
    } else {
        false
    }
}

/// 初始化 CEF(browser 进程内只做一次)。返回是否可用。
///
/// 注意:`library_loader` 在 macOS 上从当前可执行文件所在的 .app 内找
/// `Chromium Embedded Framework.framework`,因此**必须**以 .app 形态运行
/// (见 TECH.md 阶段 1 的打包要求)。
pub(crate) fn initialize_runtime() -> bool {
    *INITIALIZED.get_or_init(|| initialize_inner())
}

/// CEF 的缓存目录(每实例独立,避免 Chromium ProcessSingleton 冲突)。
fn cef_cache_paths() -> (String, String) {
    let base = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("zap-cef");
    let root = base.join("root-cache");
    // CEF 要求 cache_path 必须是 root_cache_path 的子目录(否则报错并回退到内存存储)。
    let cache = root.join("cache");
    for dir in [&root, &cache] {
        if let Err(err) = std::fs::create_dir_all(dir) {
            log::warn!("[cef] 创建缓存目录 {} 失败: {err}", dir.display());
        }
    }
    (
        cache.to_string_lossy().into_owned(),
        root.to_string_lossy().into_owned(),
    )
}

fn initialize_inner() -> bool {
    if !load_library(false) {
        let _ = INIT_STATUS.set("framework 加载失败(需以 .app 形态运行且 bundle 内含 CEF)".into());
        return false;
    }

    let args = cef::args::Args::new();
    let mut app = CefApp::new();
    // 每实例独立的缓存/根缓存目录。**必须显式设置**:CEF 默认用共享目录时 Chromium 的
    // ProcessSingleton 会与其他 CEF 进程(本 app 之前的实例、spike 等)冲突 ——
    // 实测表现为新实例启动后静默消失(CEF 自身也会警告 "Please customize
    // CefSettings.root_cache_path ... unintended process singleton behavior")。
    let (cache_path, root_cache_path) = cef_cache_paths();
    // no_sandbox:zap 走 Developer ID 直发,不进 MAS(spec 决策)。
    // external_message_pump:消息泵由 [`pump`] 驱动(zap 已有自己的事件循环)。
    let settings = Settings {
        no_sandbox: 1,
        external_message_pump: 1,
        cache_path: CefString::from(cache_path.as_str()),
        root_cache_path: CefString::from(root_cache_path.as_str()),
        ..Default::default()
    };
    // 注意:这里调用的是 cef::initialize(bindings 的 re-export);本模块自己的
    // 初始化入口叫 initialize_runtime,避免同名遮蔽。
    let ok = initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if ok != 1 {
        let _ = INIT_STATUS.set(format!("CefInitialize 返回 {ok}"));
        return false;
    }
    let _ = INIT_STATUS.set("ok".into());
    true
}

/// 初始化结果(供后续在 logger 就绪后打印:启动早期的日志会丢)。
static INIT_STATUS: OnceLock<String> = OnceLock::new();

/// 是否正在关闭(关掉后消息泵必须停:继续 `do_message_loop_work` 会触碰已释放的
/// CEF 状态)。
fn is_shutting_down() -> bool {
    SHUTTING_DOWN.load(std::sync::atomic::Ordering::SeqCst)
}

static SHUTTING_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// CEF 上下文是否已初始化(`OnContextInitialized` 回调置位)。
static CONTEXT_INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// CEF 有序关闭(幂等)。CEF 要求进程退出前调用 `CefShutdown`,否则 Chromium 的
/// 线程/atexit 清理可能崩溃或挂起(评审 F6)。未初始化时是 no-op。
pub(crate) fn shutdown() {
    if !INITIALIZED.get().copied().unwrap_or(false)
        || SHUTTING_DOWN.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    // 先逐个关闭浏览器并清空注册表,再 CefShutdown:CEF 文档明确"调用本函数后不得再
    // 调用任何 CEF 函数",而我们持有的 `Browser` 是引用计数对象 —— 若留到 thread_local
    // 析构时 drop,就会在关机后调用 release()(评审 N3)。
    WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        for state in map.values_mut() {
            if let Some(browser) = state.browser.take() {
                close_and_detach(&browser);
            }
        }
        map.clear();
    });
    log::info!("[cef] 关闭 CEF(CefShutdown)");
    cef::shutdown();
}

/// 确保 CEF 已初始化(幂等):用户在设置里打开"使用 Chromium 内核"后,无需重启
/// 即可在下一个 dsh pane 上生效。失败时返回 false,调用方回退 wry。
pub(crate) fn ensure_initialized() -> bool {
    if is_enabled() {
        return true;
    }
    if !initialize_runtime() {
        return false;
    }
    // CEF 的 OnContextInitialized 在 CefInitialize() 期间同步回调,故此处应已置位;
    // 未置位说明 CEF 行为变化(那时同栈建浏览器是危险的),显式记录以便排查。
    if !CONTEXT_INITIALIZED.load(std::sync::atomic::Ordering::SeqCst) {
        // 不 assert:CEF 行为若变化,应回退 wry 而不是把 debug 构建打挂(评审 N19)。
        log::warn!("[cef] CefInitialize 返回但 context 未标记初始化,回退 wry");
        return false;
    }
    install_app_protocol_support();
    start_pump();
    log::info!("[cef] 由设置项触发,已初始化 CEF 后端");
    true
}

/// 初始化结果描述("ok" 或失败原因)。
pub(crate) fn init_status() -> &'static str {
    INIT_STATUS.get().map(String::as_str).unwrap_or("未初始化")
}

/// 是否是 zap 的崩溃恢复 watchdog 子进程(以 `--crash-recovery-mechanism` 启动)。
///
/// 该子进程同样会走到 `warp::run()`;若不跳过 CEF 初始化,两个进程会共用缓存目录并
/// 触发 Chromium 的 ProcessSingleton 冲突(实测:探针日志里 `CefInitialize` 出现两次)。
/// 这里只认命令行标记,不依赖 `crash_recovery` 模块(它本身受 cfg 门控)。
pub(crate) fn is_crash_recovery_process() -> bool {
    std::env::args().any(|arg| arg.starts_with("--crash-recovery-mechanism"))
}

/// 运行时是否**请求**使用 CEF 后端:`ZAP_CEF_WEBVIEW` 环境开关(显式强制)或 feature
/// flag 任一为真。
///
/// 为什么要环境开关作为强制项:flag 会经 `USER_PREFERENCE_MAP`(settings 里的用户偏好)
/// 覆盖 —— 实测实例里 `ZAP_CEF_WEBVIEW=1` 已注入进程环境、二进制也含 CEF 代码,但
/// `FeatureFlag::CefWebview.is_enabled()` 仍为 false,导致 CEF 完全不初始化。
/// 开关只影响"是否尝试初始化",不改变默认(wry)路径。
pub(crate) fn is_requested() -> bool {
    let by_env = std::env::var_os("ZAP_CEF_WEBVIEW").is_some();
    let by_flag = FeatureFlag::CefWebview.is_enabled();
    if by_env && !by_flag {
        log::info!("[cef] 由 ZAP_CEF_WEBVIEW 环境开关强制启用(flag 未开启)");
    }
    by_env || by_flag
}

/// CEF 是否已成功初始化。为 false 时调用方(wry 路径)应回退,保证默认行为可用。
pub(crate) fn is_enabled() -> bool {
    INITIALIZED.get().copied().unwrap_or(false)
}

/// 驱动 CEF 消息泵。必须在主线程**稳定周期**调用(空闲也要),否则 CEF 内部
/// IPC/渲染任务不推进。
pub(crate) fn pump() {
    if is_shutting_down() {
        return;
    }
    if INITIALIZED.get().copied().unwrap_or(false) {
        do_message_loop_work();
        // 给受控实跑一个"泵确实在跑"的可观察信号。注意:**不能**在第一拍就记:那时
        // 文件 logger 可能还没就绪(实测首拍日志会丢),故延迟到第 60 拍(约 1s)。
        static PUMP_TICKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let ticks = PUMP_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if ticks == 60 {
            log::info!("[cef] message pump active (60 ticks)");
        }
        // 每秒扫一次"隐藏超时"的页面并冻结(见 freeze_hidden_overdue)。
        if ticks % 60 == 0 {
            freeze_hidden_overdue();
        }
    }
}

wrap_app! {
    struct CefApp;

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(CefBrowserProcessHandler::new(RefCell::new(None)))
        }
    }
}

wrap_browser_process_handler! {
    struct CefBrowserProcessHandler {
        client: RefCell<Option<Client>>,
    }

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            CONTEXT_INITIALIZED.store(true, std::sync::atomic::Ordering::SeqCst);
            // 阶段 1 下一步:在这里按 dsh pane 的请求创建子视图浏览器,
            // 并订阅加载完成/崩溃事件转发给 BrowserWebViewManager。
            log::info!("[cef] context initialized");
        }
    }
}

// ===================== dsh pane 的 CEF webview(阶段 1) =====================
//
// CEF 的 Browser/Frame 句柄只能在 UI 线程使用,故状态放在 thread_local;
// `BrowserWebViewManager` 只持有 id,CEF 分支按 id 调本模块(见 browser_web_view.rs)。

use std::collections::HashMap;

use objc2_app_kit::NSView;
use objc2_foundation::{NSPoint, NSRect, NSSize};
use pathfinder_geometry::rect::RectF;

/// 每次创建浏览器递增的代际号:用于丢弃"迟到"的旧浏览器回调
/// (同一 id 会被 pane 重建复用,旧实例的 on_before_close 晚到会误清新句柄)。
static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

struct CefWebview {
    browser: Option<Browser>,
    /// 当前句柄属于哪一代。
    generation: u64,
    /// AppKit frame 坐标(父视图内、底部原点)。
    rect: NSRect,
    visible: bool,
    init_js: String,
    /// 隐藏即销毁后重建所需:容器视图指针与当前 URL。
    parent_view: *mut c_void,
    url: String,
    /// 最近一次收到的逻辑坐标(挂起期间也更新;重建时据此换算)。
    pending_rect: Option<RectF>,
    /// 隐藏起始时刻(用于"隐藏超过 N 秒后冻结")。
    hidden_since: Option<std::time::Instant>,
    /// 页面是否已被冻结(CDP Page.setWebLifecycleState=frozen)。
    frozen: bool,
    /// 浏览器背景色(0xAARRGGBB,opaque)。重建时复用。
    background: u32,
}

/// 隐藏多久后冻结页面。来源:`CefWebviewSettings::freeze_after_secs`(默认 300s,
/// 见 app/src/settings/cef_webview.rs),由 app 每帧推入;
/// `ZAP_CEF_FREEZE_AFTER_SECS` 可覆盖(dev 排查用)。0 = 不冻结。
static FREEZE_AFTER_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(300);

/// 由 app 推入设置值(每帧调用,代价是一次原子写)。
pub(crate) fn set_freeze_after_secs(secs: u32) {
    FREEZE_AFTER_SECS.store(u64::from(secs), std::sync::atomic::Ordering::Relaxed);
}

/// 当前生效的冻结阈值;`None` 表示不冻结(设置为 0)。
fn freeze_after() -> Option<std::time::Duration> {
    let secs = std::env::var("ZAP_CEF_FREEZE_AFTER_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_else(|| FREEZE_AFTER_SECS.load(std::sync::atomic::Ordering::Relaxed));
    (secs > 0).then(|| std::time::Duration::from_secs(secs))
}

/// 向页面发一条 CDP 命令(冻结/解冻用)。
fn send_cdp(browser: &Browser, method: &str, params: &str) -> bool {
    let Some(host) = browser.host() else {
        return false;
    };
    let message = format!(r#"{{"id":1,"method":"{method}","params":{params}}}"#);
    let ok = host.send_dev_tools_message(Some(message.as_bytes())) == 1;
    if !ok {
        log::warn!("[cef] CDP {method} 下发失败(浏览器可能尚未就绪)");
    }
    ok
}

/// 隐藏超时后冻结页面(由 pump 周期调用)。
fn freeze_hidden_overdue() {
    let Some(threshold) = freeze_after() else {
        return; // 设置为 0:不冻结
    };
    // 先只挑出"确实有浏览器句柄"的候选,再在下发成功后置 frozen:否则 browser 尚未
    // 创建就被标记冻结,该实例此后永远不会被冻结(且解冻时会发多余的 active)。
    let candidates: Vec<(u64, Browser)> = WEBVIEWS.with(|map| {
        map.borrow()
            .iter()
            .filter(|(_, state)| {
                !state.visible
                    && !state.frozen
                    && state
                        .hidden_since
                        .is_some_and(|since| since.elapsed() >= threshold)
            })
            .filter_map(|(id, state)| state.browser.clone().map(|browser| (*id, browser)))
            .collect()
    });
    for (id, browser) in candidates {
        if send_cdp(&browser, "Page.setWebLifecycleState", r#"{"state":"frozen"}"#) {
            log::info!("[cef] webview {id}: 隐藏超时,已冻结页面(CDP setWebLifecycleState=frozen)");
            WEBVIEWS.with(|map| {
                if let Some(state) = map.borrow_mut().get_mut(&id) {
                    state.frozen = true;
                }
            });
        }
    }
}

/// 解冻(重新可见时调用)。
/// 解冻;返回是否真正下发成功(失败则保留 frozen,下一帧重试)。
fn thaw(browser: &Browser) -> bool {
    send_cdp(browser, "Page.setWebLifecycleState", r#"{"state":"active"}"#)
}

thread_local! {
    static WEBVIEWS: RefCell<HashMap<u64, CefWebview>> = RefCell::new(HashMap::new());
}

/// 该 id 是否由 CEF 承载(manager 据此区分后端)。
pub(crate) fn webview_exists(id: u64) -> bool {
    WEBVIEWS.with(|map| map.borrow().contains_key(&id))
}

/// 场景逻辑坐标(左上原点)→ 父视图 AppKit 坐标(底部原点)。
/// 翻转公式与 wry 的 `window_position` 对齐(`parentHeight - y - h`),见
/// TECH.md 集成要求 #2。
fn to_appkit_rect(parent_view: *mut c_void, rect: RectF) -> NSRect {
    let parent_height = unsafe { (*(parent_view as *const NSView)).frame().size.height };
    flip_rect_to_appkit(rect, parent_height)
}

/// 场景逻辑坐标(左上原点)→ AppKit frame 坐标(底部原点)的纯计算。
/// 公式与 wry 的 `window_position` 对齐(`parentHeight - y - h`),见集成要求 #2。
/// 抽成纯函数以便单测(窗口几何在测试环境里无法构造)。
fn flip_rect_to_appkit(rect: RectF, parent_height: f64) -> NSRect {
    NSRect {
        origin: NSPoint {
            x: rect.origin_x() as f64,
            y: parent_height - rect.origin_y() as f64 - rect.height() as f64,
        },
        size: NSSize {
            width: rect.width() as f64,
            height: rect.height() as f64,
        },
    }
}

fn cef_rect(ns_rect: NSRect) -> Rect {
    Rect {
        x: ns_rect.origin.x as i32,
        y: ns_rect.origin.y as i32,
        width: ns_rect.size.width as i32,
        height: ns_rect.size.height as i32,
    }
}

/// 创建 CEF 子视图 webview(挂在 `parent_view` 内,即 warpui 的 WebViewContainerView)。
/// `init_js` 在主 frame 加载完成后注入一次(回环 IPC shim)。
pub(crate) fn create_webview(
    id: u64,
    parent_view: *mut c_void,
    rect: RectF,
    url: &str,
    init_js: &str,
    background: u32,
) {
    let ns_rect = to_appkit_rect(parent_view, rect);
    // 诊断:父视图必须已在某个窗口内,否则 CEF 行为不可预期(排查"多出一个窗口")。
    {
        let parent = unsafe { &*(parent_view as *const NSView) };
        let parent_window = parent.window();
        log::info!(
            "[cef] webview {id} parent={:p} in_window={} parent_frame={:?} rect={:?}",
            parent_view,
            parent_window.is_some(),
            parent.frame(),
            ns_rect
        );
    }
    WEBVIEWS.with(|map| {
        map.borrow_mut().insert(
            id,
            CefWebview {
                browser: None,
                generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                rect: ns_rect,
                visible: true,
                init_js: init_js.to_owned(),
                parent_view,
                url: url.to_owned(),
                pending_rect: Some(rect),
                hidden_since: None,
                frozen: false,
                background,
            },
        );
    });
    let generation = WEBVIEWS.with(|map| map.borrow().get(&id).map(|s| s.generation));
    if let Some(generation) = generation {
        spawn_browser(id, generation, parent_view, ns_rect, url, background);
    }
}

/// 创建 CEF 浏览器(首次创建与"隐藏后重现"共用)。
fn spawn_browser(
    id: u64,
    generation: u64,
    parent_view: *mut c_void,
    ns_rect: NSRect,
    url: &str,
    background: u32,
) {
    let window_info = WindowInfo {
        runtime_style: RuntimeStyle::ALLOY,
        ..WindowInfo::default()
            .set_as_child(parent_view as sys::cef_window_handle_t, &cef_rect(ns_rect))
    };
    let mut client = WebviewClient::new(id, generation);
    // **CEF 的 windowed 浏览器做不到真透明**:按 cef_types.h,浏览器背景 alpha 透明时
    // 会回退到 CefSettings.background_color,而那个再透明就退化成"不透明白"。wry 那套
    // (WKWebView 私有 KVC `drawsBackground`)在 CEF 里没有对应键 —— 真透明只有 windowless
    // (OSR)渲染一条路。所以这里显式填 zap 的工作区底色,避免白底。
    let settings = BrowserSettings {
        background_color: background,
        ..Default::default()
    };
    let url = CefString::from(url);
    let created = browser_host_create_browser(
        Some(&window_info),
        Some(&mut client),
        Some(&url),
        Some(&settings),
        None,
        None,
    );
    if created != 1 {
        // 创建失败会留下 has_webview=true 但 browser=None 的 pane(不会重建、也无 wry
        // 回退),故移除条目让对方下次 attach 时重试(评审 N10)。
        log::error!("[cef] webview {id}: browser 创建失败(返回 {created}),移除条目以便重试");
        crate::browser::browser_web_view::notify_webview_create_failed(id);
    }
}

/// 把浏览器对应的原生视图从父视图里摘掉。
///
/// **必须做**:`do_close` 返回 1 表示"由客户端负责完成关闭",CEF 因此不会自己
/// 拆视图/发关闭通知;若我们只调 `close_browser` 而不管视图,浏览器会成为僵尸
/// ——实测表现为切到别的 tab 后底下仍显示着旧 webview,且重建时叠加第二层。
fn detach_view(browser: &Browser) {
    if let Some(host) = browser.host() {
        let view = host.window_handle() as *mut NSView;
        if !view.is_null() {
            unsafe { (*view).removeFromSuperview() };
        }
    }
}

/// 关闭并拆除一个浏览器(隐藏即销毁 / 销毁 / 代际淘汰共用)。
fn close_and_detach(browser: &Browser) {
    detach_view(browser);
    if let Some(host) = browser.host() {
        host.close_browser(1);
    }
}

/// 取浏览器句柄执行操作;浏览器尚未创建(`on_after_created` 未到)时静默跳过。
fn with_browser<R>(id: u64, f: impl FnOnce(&Browser, &mut CefWebview) -> R) -> Option<R> {
    // 关机后不得再调用任何 CEF 函数(cef_app_capi.h 明示)。这里是最集中的入口,
    // 统一早返回,避免各调用点漏加门控(评审 N3)。
    if is_shutting_down() {
        return None;
    }
    WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        let state = map.get_mut(&id)?;
        let browser = state.browser.clone()?;
        Some(f(&browser, state))
    })
}

pub(crate) fn navigate(id: u64, url: &str) {
    WEBVIEWS.with(|map| {
        if let Some(state) = map.borrow_mut().get_mut(&id) {
            state.url = url.to_owned();
        }
    });
    with_browser(id, |browser, _| {
        if let Some(frame) = browser.main_frame() {
            frame.load_url(Some(&CefString::from(url)));
        }
    });
}

/// 当前 URL(主 frame 最近一次加载/导航后的地址)。
pub(crate) fn current_url(id: u64) -> Option<String> {
    WEBVIEWS.with(|map| map.borrow().get(&id).map(|state| state.url.clone()))
}

pub(crate) fn reload(id: u64) {
    with_browser(id, |browser, _| browser.reload());
}

pub(crate) fn evaluate(id: u64, js: &str) {
    with_browser(id, |browser, _| {
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&CefString::from(js)), None, 0);
        }
    });
}

pub(crate) fn focus(id: u64, focused: bool) {
    with_browser(id, |browser, _| {
        if let Some(host) = browser.host() {
            host.set_focus(i32::from(focused));
        }
    });
}

/// 断言当前在 CEF UI 线程(= 主线程)。`WEBVIEWS` 是 thread_local:从别的线程访问
/// 会拿到另一个空 map 而静默 no-op(比 panic 更难查,评审 F18)。
fn debug_assert_ui_thread() {
    debug_assert_ne!(
        cef::currently_on(cef::ThreadId::UI),
        0,
        "CEF 后端的状态访问必须在 UI(主)线程"
    );
}

/// 可见性:**仅隐藏原生视图,不销毁浏览器**(2026-09-21 按用户要求取消"隐藏即销毁")。
///
/// 隐藏必须真的把 NSView 藏起来(`setHidden:`):CEF 的 `was_hidden` 不足以让视图从
/// 屏幕上消失 —— 隐藏没生效时,切到别的 tab 会在底下看到残留的 webview(实测)。
/// renderer 保活的好处:切回不需要重载,页面状态/输入内容不丢。
pub(crate) fn set_visible(id: u64, visible: bool) {
    debug_assert_ui_thread();
    let known = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        match map.get_mut(&id) {
            Some(state) => {
                state.visible = visible;
                true
            }
            None => false,
        }
    });
    if !known {
        return;
    }
    // 隐藏:记开始时刻(供 pump 判超时冻结);可见:清计时并解冻。
    let was_frozen = WEBVIEWS.with(|map| {
        let mut map = map.borrow_mut();
        match map.get_mut(&id) {
            Some(state) => {
                if visible {
                    // 只清计时;`frozen` 是否清掉取决于解冻是否下发成功(见下)。
                    state.hidden_since = None;
                    state.frozen
                } else {
                    state.hidden_since.get_or_insert_with(std::time::Instant::now);
                    false
                }
            }
            None => false,
        }
    });
    with_browser(id, |browser, _| {
        if visible && was_frozen {
            log::info!("[cef] webview {id}: 重新可见,解冻页面");
            // 解冻失败则保留 frozen:下一帧(仍不可见时的冻结扫描不会再碰它,
            // 但下次可见时会再次尝试)重试,避免"永久冻结"(评审 N6)。
            if thaw(browser) {
                WEBVIEWS.with(|map| {
                    if let Some(state) = map.borrow_mut().get_mut(&id) {
                        state.frozen = false;
                    }
                });
            }
        }
        if let Some(host) = browser.host() {
            let view = host.window_handle() as *mut NSView;
            if !view.is_null() {
                unsafe { (*view).setHidden(!visible) };
            }
            host.was_hidden(i32::from(!visible));
        }
    });
}

/// 每帧布局驱动(集成要求 #2):CEF 子视图不跟随洞 rect,必须显式设 frame 并
/// 通知宿主 `was_resized()`。
pub(crate) fn set_bounds(id: u64, rect: RectF) {
    debug_assert_ui_thread();
    // 先把最新逻辑坐标记进状态:挂起(无浏览器)期间也要更新,否则"隐藏即销毁"
    // 重建时会用旧几何落位。
    WEBVIEWS.with(|map| {
        if let Some(state) = map.borrow_mut().get_mut(&id) {
            state.pending_rect = Some(rect);
        }
    });
    with_browser(id, |browser, state| {
        let Some(host) = browser.host() else {
            return;
        };
        let view = host.window_handle() as *mut NSView;
        if view.is_null() {
            return;
        }
        let parent_height = unsafe { (*view).superview() }
            .map(|superview| superview.frame().size.height);
        let Some(parent_height) = parent_height else {
            return;
        };
        let ns_rect = flip_rect_to_appkit(rect, parent_height);
        // 调用点每帧都会上报几何;wry 分支同样"先比较再下发"。每帧无条件
        // setFrame + was_resized 会让 Chromium 做无意义重排(评审 F8)。
        if ns_rect == state.rect {
            return;
        }
        unsafe { (*view).setFrame(ns_rect) };
        state.rect = ns_rect;
        host.was_resized();
    });
}

/// 销毁 webview。CEF 的 `on_before_close` 会再清一次(幂等)。
pub(crate) fn destroy(id: u64) {
    debug_assert_ui_thread();
    let browser = WEBVIEWS.with(|map| map.borrow().get(&id).and_then(|state| state.browser.clone()));
    if let Some(browser) = browser {
        close_and_detach(&browser);
    }
    WEBVIEWS.with(|map| {
        map.borrow_mut().remove(&id);
    });
}

/// `on_after_created` 后把几何/可见性落到刚创建的浏览器上。
fn apply_geometry(id: u64) {
    let pending = WEBVIEWS.with(|map| {
        map.borrow().get(&id).map(|state| (state.rect, state.pending_rect))
    });
    let Some((stored_rect, pending_rect)) = pending else {
        return;
    };
    // 有更新的逻辑坐标就以它为准:创建用的 rect 常是 RectF::default(),真实几何由
    // 首帧 set_bounds 给出(那时浏览器可能还没建好)。
    let rect = match pending_rect {
        Some(logical) => {
            let parent = WEBVIEWS.with(|map| map.borrow().get(&id).map(|state| state.parent_view));
            match parent {
                Some(parent_view) => flip_rect_to_appkit(
                    logical,
                    unsafe { (*(parent_view as *const NSView)).frame().size.height },
                ),
                None => stored_rect,
            }
        }
        None => stored_rect,
    };
    with_browser(id, |browser, state| {
        if let Some(host) = browser.host() {
            let view = host.window_handle() as *mut NSView;
            if !view.is_null() {
                unsafe { (*view).setFrame(rect) };
            }
            host.was_hidden(i32::from(!state.visible));
            host.was_resized();
        }
    });
}

wrap_client! {
    struct WebviewClient {
        id: u64,
        generation: u64,
    }

    impl Client {
        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            Some(WebviewContextMenu::new(self.id))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(WebviewRequest::new(self.id, self.generation))
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(WebviewLifeSpan::new(self.id, self.generation))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(WebviewLoad::new(self.id, self.generation))
        }
    }
}

/// 上下文菜单自定义命令 id(CEF 约定用户 id 从 26500 起)。
const MENU_ID_RELOAD: i32 = 26500;
const MENU_ID_INSPECT_ELEMENT: i32 = 26501;

wrap_context_menu_handler! {
    struct WebviewContextMenu {
        id: u64,
    }

    impl ContextMenuHandler {
        /// 精简 CEF 默认菜单:清空后只放"重新加载"与"检查元素"。
        ///
        /// 实测 CEF(Alloy)默认项只有 Back/Forward/分隔符/Print…/View Page Source
        /// ——**没有**"自动填充",故无须按标题保留(标题匹配还会随语言失效,评审 F11)。
        /// 代价:右键菜单里不再有复制/粘贴,但 Cmd+C/V 等快捷键不受影响。
        fn on_before_context_menu(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _params: Option<&mut ContextMenuParams>,
            model: Option<&mut MenuModel>,
        ) {
            let Some(model) = model else {
                return;
            };
            let removed = model.count();
            let labels: Vec<String> = (0..removed)
                .map(|index| CefString::from(&model.label_at(index)).to_string())
                .collect();
            model.clear();
            log::debug!("[cef] 右键菜单:清空 {removed} 个默认项 {labels:?}");
            model.add_item(MENU_ID_RELOAD, Some(&CefString::from("重新加载")));
            model.add_item(
                MENU_ID_INSPECT_ELEMENT,
                Some(&CefString::from("检查元素")),
            );
        }

        /// 处理"检查元素":打开 DevTools 并定位到右键点中的元素。
        /// 返回 1 = 已处理;返回 0 = 交回 CEF 处理默认命令。
        fn on_context_menu_command(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            command_id: i32,
            _event_flags: EventFlags,
        ) -> i32 {
            let Some(browser) = browser else {
                return 1;
            };
            if command_id == MENU_ID_RELOAD {
                log::info!("[cef] webview {} 右键重新加载", self.id);
                browser.reload();
                return 1;
            }
            if command_id != MENU_ID_INSPECT_ELEMENT {
                return 0;
            }
            let inspect_at = params.map(|params| Point {
                x: params.xcoord(),
                y: params.ycoord(),
            });
            if let Some(host) = browser.host() {
                log::info!("[cef] webview {} 右键检查元素", self.id);
                host.show_dev_tools(None, None, None, inspect_at.as_ref());
            }
            1
        }
    }
}

wrap_life_span_handler! {
    struct WebviewLifeSpan {
        id: u64,
        generation: u64,
    }

    impl LifeSpanHandler {
        /// 子视图浏览器的关闭契约:CEF 在 windowed 模式下的默认行为是向 browser 的
        /// **顶层父窗口**发标准关闭通知(macOS 即 `performClose:`),而我们的顶层父窗口
        /// 就是 Zap 主窗 —— 那会误关主窗。这里显式接管(true = 由我们完成关闭),
        /// 实际销毁由 `close_browser(1)` + `on_before_close` 里的视图移除完成。
        fn do_close(&self, _browser: Option<&mut Browser>) -> i32 {
            1
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let Some(browser) = browser.cloned() else {
                return;
            };
            let accepted = WEBVIEWS.with(|map| {
                let mut map = map.borrow_mut();
                match map.get_mut(&self.id) {
                    // 只接受当前代际:被取代的旧实例直接关掉。
                    Some(state) if state.generation == self.generation => {
                        state.browser = Some(browser.clone());
                        true
                    }
                    _ => false,
                }
            });
            if !accepted {
                close_and_detach(&browser);
                log::info!(
                    "[cef] webview {} 丢弃过期代际 {} 的浏览器",
                    self.id,
                    self.generation
                );
                return;
            }
            apply_geometry(self.id);
            log::info!("[cef] webview {} browser created", self.id);
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            // 防御性移除原生视图:CEF 的 child-view 归属由宿主负责,残留的
            // CefBrowserHostView 会在洞被别的 pane 复用时透出旧页面。
            if let Some(browser) = browser {
                if let Some(host) = browser.host() {
                    let view = host.window_handle() as *mut NSView;
                    if !view.is_null() {
                        unsafe { (*view).removeFromSuperview() };
                    }
                }
            }
            // 只清句柄,不删状态:"隐藏即销毁"下同一 id 之后还会重建;
            // **只清当前代际**(否则迟到的旧回调会清掉新句柄)。
            WEBVIEWS.with(|map| {
                if let Some(state) = map.borrow_mut().get_mut(&self.id) {
                    if state.generation == self.generation {
                        state.browser = None;
                    }
                }
            });
            log::info!("[cef] webview {} closed (gen {})", self.id, self.generation);
        }
    }
}

wrap_request_handler! {
    struct WebviewRequest {
        id: u64,
        generation: u64,
    }

    impl RequestHandler {
        /// 渲染进程崩溃/被杀:与 wry 的 terminate handler 走**同一条**事件路径
        /// (pane 弹崩溃态 + 重新加载入口),否则 CEF pane 会停在死页且不再重建(评审 F7)。
        fn on_render_process_terminated(
            &self,
            _browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: i32,
            _error_string: Option<&CefString>,
        ) {
            // 代际过滤:旧实例的终止事件不得影响新实例。
            let current = WEBVIEWS.with(|map| {
                map.borrow()
                    .get(&self.id)
                    .map(|state| state.generation)
            });
            if current != Some(self.generation) {
                return;
            }
            log::error!("[cef] webview {} renderer 终止 status={status:?} code={error_code}", self.id);
            crate::browser::browser_web_view::notify_webview_crashed(self.id);
        }
    }
}

wrap_load_handler! {
    struct WebviewLoad {
        id: u64,
        generation: u64,
    }

    impl LoadHandler {
        fn on_load_end(
            &self,
            _browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _http_status_code: i32,
        ) {
            // 顺序敏感:先取 URL 与主 frame 判定(is_main 会移动 frame),再做后续操作。
            let loaded_url = frame
                .as_ref()
                .map(|frame| CefString::from(&frame.url()).to_string())
                .filter(|url| !url.is_empty());
            let is_main = frame
                .as_ref()
                .map(|frame| frame.is_main() != 0)
                .unwrap_or(true);
            if !is_main {
                return;
            }

            // 记录主 frame 的真实 URL(dsh 首载是 ?token=…,303 后变裸地址;pane 的
            // origin 比较与工具栏同步都依赖它)。
            WEBVIEWS.with(|map| {
                let mut map = map.borrow_mut();
                if let Some(state) = map.get_mut(&self.id) {
                    if state.generation == self.generation {
                        if let Some(url) = loaded_url {
                            state.url = url;
                        }
                    }
                }
            });

            // **每次**主 frame 加载完成都注入 shim:同一 browser 内的第二次文档
            // (reload / 服务端重定向后的硬跳转 / location.replace)是全新 DOM,
            // 一次性注入会让 IPC 在第二次加载后静默失效;shim 自带
            // `if (window.__ZAP_LOOPBACK_IPC__) return;` 幂等守卫,重复注入无副作用。
            let script = WEBVIEWS.with(|map| {
                let map = map.borrow();
                match map.get(&self.id) {
                    // 代际过滤:旧实例的 load_end 不得影响新实例。
                    Some(state) if state.generation == self.generation => {
                        Some(state.init_js.clone())
                    }
                    _ => None,
                }
            });
            if let Some(script) = script {
                log::info!("[cef] webview {} loaded (shim 注入)", self.id);
                evaluate(self.id, &script);

                // **关键**:wry 路径靠 `on_page_load_handler` 发 UrlChanged 事件,pane 据此
                // 结束"启动中"覆盖层;CEF 侧没有该回调,必须自己回传,否则页面已渲染但
                // 面板一直显示"启动中"(只能等 15s 超时兜底)——实测踩到。
                // 放在上面的代际判定内:旧实例的 load_end 不得影响新实例(评审 F9)。
                crate::browser::browser_web_view::notify_webview_page_loaded(self.id);
            }
        }
    }
}

#[cfg(test)]
#[path = "cef_backend_tests.rs"]
mod tests;
