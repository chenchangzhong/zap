//! 嵌入式 WebView(WKWebView,经 wry)的生命周期管理,服务 Web preview pane。
//!
//! 与 pane 布局解耦:每个 webview 由一个 `platform_view_id` 标识,与 warpui 场景里
//! `PlatformViewElement { id }` 一一对应。每帧渲染结束后,平台层把场景里声明的
//! `PlatformView`(id + rect)经 window 的 platform-view handler 报告出来;handler
//! 位于渲染线程、没有 `AppContext`,先把数据写入静态暂存队列,再由每帧的
//! `on_frame_drawn` 回调(持有 `&mut AppContext`)消费并调用 [`BrowserWebViewManager::set_bounds`]。
//!
//! macOS 先行:非 macOS 平台编译为空壳,所有方法 no-op。

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use warpui::{Entity, ModelContext, PlatformView, SingletonEntity, WindowId};

#[cfg(target_os = "macos")]
use wry::WebViewBuilderExtDarwin;

/// platform-view handler 的暂存区:`platform_view_id -> rect` 按窗口暂存。
/// 每帧由 warpui 渲染线程写入(覆盖语义),由 `on_frame_drawn` 消费。
pub(crate) static PENDING_PLATFORM_VIEWS: std::sync::LazyLock<
    Mutex<HashMap<WindowId, Vec<PlatformView>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 页面内元素获得焦点(focusin)事件的暂存区。由 wry 的 IPC handler
/// (WKScriptMessageHandler,主线程)写入,由每帧 `on_frame_drawn` 消费。
/// 消费后地址栏释放焦点,避免与页面输入框的光标共存(双光标)。
pub(crate) static PENDING_WEBVIEW_FOCUS_EVENTS: std::sync::LazyLock<
    Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// DSH 插件 webview ID 集合。只有在此集合中的 webview 才允许发送 `zap:` IPC 消息。
/// 由 DshPane 创建 webview 时注册,webview 销毁时移除。
pub(crate) static DSH_WEBVIEW_IDS: std::sync::LazyLock<
    Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// webview 相关事件,经 singleton model 分发给各 BrowserPane。
#[derive(Debug, Clone)]
pub enum BrowserWebViewEvent {
    /// 页面内元素获得了焦点(用户点击了页面里的输入框等)。
    PageFocused(u64),
    /// 页面加载完成,URL 可能已变化(后退/前进/页面内导航)。
    UrlChanged(u64),
    /// WebContent 渲染进程已崩溃(WKWebView 空洞/死页)。runtime 与页面
    /// URL 均未变化,重载页面即可恢复;由 DshPane 据此弹出确认入口。
    WebContentCrashed(u64),
}

/// 页面加载完成事件的暂存区。由 wry 的 `on_page_load_handler`(主线程)
/// 写入,由每帧 `on_frame_drawn` 消费。消费后触发地址栏同步与 pane 标题更新。
/// CEF 浏览器创建失败(尚未登记成功)的 id,由 drain 移除对应条目。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) static PENDING_WEBVIEW_CREATE_FAILED: std::sync::LazyLock<
    parking_lot::Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashSet::new()));

pub(crate) static PENDING_WEBVIEW_URL_CHANGED: std::sync::LazyLock<
    Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// WebContent 渲染进程崩溃事件的暂存区。由 wry 的
/// `on_web_content_process_terminate_handler`(WKNavigationDelegate 回调,
/// 主线程)写入,由每帧 `on_frame_drawn` 消费。渲染进程崩溃后页面变死页,
/// 由 DshPane 弹出崩溃态 + 重新加载入口。
pub(crate) static PENDING_WEBVIEW_CRASHED: std::sync::LazyLock<
    Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// 可安全交给系统默认浏览器/应用打开的 URL scheme。`javascript:` 等伪协议
/// 一律过滤,防止把脚本串传给 NSWorkspace。
fn is_externally_openable(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https" | "mailto" | "tel"))
}

/// webview 后端选择。`WebViewBackend::Cef` 仅在 macOS + `cef_webview` feature 且
/// runtime flag 开启时生效,否则自动回退 Wry(specs/cef-webview-minimal/TECH.md 阶段 1)。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WebViewBackend {
    /// wry / WKWebView(默认路径,行为不变)。
    Wry,
    /// CEF(Chromium):dsh pane 专用。
    Cef,
}

/// 取 `window` 句柄对应的容器 NSView 指针(warpui 的 `HasWindowHandle` 返回
/// `WebViewContainerView`,见 crates/warpui/src/platform/mac/window.rs:1110-1119)。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
fn cef_parent_view(window: &impl raw_window_handle::HasWindowHandle) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::RawWindowHandle;
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::AppKit(handle) => Some(handle.ns_view.as_ptr()),
        _ => None,
    }
}

/// 全局单例:管理所有 webview 的 create / navigate / set_bounds / destroy。
pub struct BrowserWebViewManager {
    #[cfg(target_os = "macos")]
    webviews: RefCell<HashMap<u64, WebViewEntry>>,
    /// CEF 承载的 webview 元数据(句柄在 `cef_backend` 的 UI 线程注册表里)。
    /// 与 `webviews` 分开存放:wry 路径因此**完全不改**(仅 feature 下编译)。
    #[cfg(all(target_os = "macos", feature = "cef_webview"))]
    cef_entries: RefCell<HashMap<u64, CefEntry>>,
    /// platform_view_id 的单调递增分配器(每个 pane 创建时取一个)。
    next_id: AtomicU64,
}

/// CEF webview 的宿主侧元数据(句柄/几何在 cef_backend 内)。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
struct CefEntry {
    window_id: WindowId,
    current_url: std::sync::Arc<parking_lot::Mutex<Option<String>>>,
}

#[cfg(target_os = "macos")]
struct WebViewEntry {
    /// Box 保证堆分配,地址不受 HashMap rehash 影响。存 raw pointer
    /// 供 IPC handler 同步调 focus()。
    webview: Box<wry::WebView>,
    window_id: WindowId,
    current_url: std::sync::Arc<parking_lot::Mutex<Option<String>>>,
    last_bounds: Option<wry::Rect>,
    /// 与 IPC handler 闭包共享的裸指针。destroy 时先置空再 drop Box,
    /// 避免销毁后残留的 IPC 回调解引用悬垂指针(use-after-free)。
    focus_ptr: std::sync::Arc<parking_lot::Mutex<*const wry::WebView>>,
}

/// 页面加载完成通知(供 CEF 后端调用):与 wry 的 `on_page_load_handler` 走同一条
/// 事件路径(每帧 drain → emit UrlChanged),pane 据此结束"启动中"覆盖层。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) fn notify_webview_page_loaded(id: u64) {
    PENDING_WEBVIEW_URL_CHANGED.lock().insert(id);
    // 页面可能在空闲期加载完成(此时没有渲染帧),主动请求一帧让 drain 立刻执行。
    warpui::platform::mac::Window::request_redraw_all_windows();
}

/// 渲染进程崩溃通知(供 CEF 后端调用):与 wry 的 terminate handler 同一条事件路径,
/// pane 据此展示崩溃态并提供重新加载。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) fn notify_webview_crashed(id: u64) {
    PENDING_WEBVIEW_CRASHED.lock().insert(id);
    // 崩溃回调可能发生在空闲期(没有渲染帧),主动请求一帧让 drain 立刻执行。
    warpui::platform::mac::Window::request_redraw_all_windows();
}

/// CEF 浏览器创建失败通知:暂存 id,由每帧 drain 移除条目,使 `has_webview` 归 false,
/// 让 pane 下次 attach 时重新创建,避免"永久空 pane"(评审 N10)。
#[cfg(all(target_os = "macos", feature = "cef_webview"))]
pub(crate) fn notify_webview_create_failed(id: u64) {
    PENDING_WEBVIEW_CREATE_FAILED.lock().insert(id);
    warpui::platform::mac::Window::request_redraw_all_windows();
}

/// 下载前的同步保存面板:预填默认路径(`~/Downloads` + 建议文件名),用户确认后
/// 返回选中路径,取消返回 None(调用方拒绝下载)。
///
/// 两个后端共用:wry 侧由 `WKDownloadDelegate` 回调触发(见下方 download handler),
/// CEF 侧由 `DownloadHandler::on_before_download` 触发。两者都在主线程,故直接
/// `runModal`(与 NSAlert 的 modal 用法同理)。
///
/// **关闭后必须把 key window 还回去**:`runModal` 是 app-modal,面板关闭时 AppKit
/// 不保证把 key 状态还给原来的窗口(未验证推测);没有 key window,后面无论怎么设
/// first responder 都收不到键盘(实机:导出 → 取消后页面无法输入,得先用鼠标点一下)。
#[cfg(target_os = "macos")]
pub(crate) fn run_download_save_panel(default_path: &Path) -> Option<PathBuf> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSModalResponseOK, NSSavePanel};
    use objc2_foundation::{NSString, NSURL};

    let mtm = MainThreadMarker::new().expect("download handler must run on main thread");
    let panel = NSSavePanel::savePanel(mtm);
    if let Some(name) = default_path.file_name() {
        panel.setNameFieldStringValue(&NSString::from_str(&name.to_string_lossy()));
    }
    if let Some(dir) = default_path.parent() {
        let dir = NSString::from_str(&dir.to_string_lossy());
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath_isDirectory(&dir, true)));
    }
    let app = NSApplication::sharedApplication(mtm);
    // **必须在 activate() 之前取**:Apple 文档明说 activate 不保证立即生效(甚至不保证一定激活),
    // 而失活的 app 没有 key window ⇒ 先 activate 再取,最需要这条兜底的场景恰好取到 None。
    let previous_key_window = app.keyWindow();
    // 面板若被压到其他 app 后面会不可见,先激活(对齐 alert 的做法)。
    app.activate();
    let response = panel.runModal();
    // 只在本 app 仍处于激活态时把窗口拉回 key:否则会把用户切走后的前台硬抢回来。
    if app.isActive() {
        if let Some(window) = previous_key_window.or_else(|| app.mainWindow()) {
            window.makeKeyAndOrderFront(None);
        }
    }
    if response == NSModalResponseOK {
        panel
            .URL()
            .and_then(|url| url.path())
            .map(|path| PathBuf::from(path.to_string()))
    } else {
        None
    }
}

impl BrowserWebViewManager {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            webviews: RefCell::new(HashMap::new()),
            #[cfg(all(target_os = "macos", feature = "cef_webview"))]
            cef_entries: RefCell::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    /// 分配一个唯一的 platform-view id,供新 webview 使用。
    pub fn allocate_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// 注册 webview ID 为 DSH 插件,允许发送 `zap:` IPC 消息。
    pub fn register_dsh_webview(id: u64) {
        DSH_WEBVIEW_IDS.lock().insert(id);
    }

    /// 注销 DSH webview ID。
    pub fn unregister_dsh_webview(id: u64) {
        DSH_WEBVIEW_IDS.lock().remove(&id);
    }

    /// 在 `window` 的 contentView 内创建 id 对应的 webview,初始位置 `rect`。
    /// `incognito` 为 true 时 webview 使用非持久化数据存储(cookie 不落盘、
    /// 不跨实例累积)——dsh 每次启动都换端口与 token,其 cookie 无需持久化。
    #[cfg(target_os = "macos")]
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        window: &impl raw_window_handle::HasWindowHandle,
        id: u64,
        url: &str,
        rect: RectF,
        window_id: WindowId,
        incognito: bool,
        backend: WebViewBackend,
        background_color: Option<warpui::color::ColorU>,
    ) {
        log::info!(
            "[browser] create webview {id} url={url} rect={rect:?} incognito={incognito} backend={backend:?}"
        );
        // CEF 分支:句柄由 cef_backend 持有,这里只记元数据。仅在 feature + flag
        // 同时满足时进入;否则落到下方既有的 wry 路径(行为不变)。
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if backend == WebViewBackend::Cef && crate::browser::cef_backend::is_enabled() {
            match cef_parent_view(window) {
                Some(parent_view) => {
                    let init_js = crate::dsh::loopback_ipc::shim_script();
                    let background = background_color.map_or(0xFF1E1E1E, |color| {
                        0xFF00_0000
                            | (u32::from(color.r) << 16)
                            | (u32::from(color.g) << 8)
                            | u32::from(color.b)
                    });
                    crate::browser::cef_backend::create_webview(
                        id, parent_view, rect, url, &init_js, background,
                    );
                    self.cef_entries.borrow_mut().insert(
                        id,
                        CefEntry {
                            window_id,
                            current_url: std::sync::Arc::new(parking_lot::Mutex::new(Some(
                                url.to_string(),
                            ))),
                        },
                    );
                    return;
                }
                None => log::warn!("[cef] webview {id}: 拿不到容器视图,回退 wry 后端"),
            }
        }
        #[cfg(not(all(target_os = "macos", feature = "cef_webview")))]
        {
            let _ = backend;
            let _ = background_color;
        }
        // wry 的 child webview 会拦截 performKeyEquivalent(Cmd 快捷键不
        // 进 webview),在页面内监听 Cmd+R 触发刷新;同时:当地址栏聚焦时
        // webview 失焦,页面活跃元素的焦点应自动释放,避免两处光标共存。
        // 页面内元素获得焦点(focusin)时经 IPC 上报 Rust,让 Warp 释放
        // 地址栏的焦点与光标(地址栏与页面各一个光标 = 双光标)。
        let init_js = crate::browser::webview_init_js::WEBVIEW_INIT_JS;
        let current_url = std::sync::Arc::new(parking_lot::Mutex::new(Some(url.to_string())));
        let handler_url = current_url.clone();
        let ipc_id = id;
        let url_notify_id = id;
        let terminate_id = id;
        // IPC handler 里同步调 makeFirstResponder,不等下一帧。
        // 用 raw pointer + Box(堆分配,地址不受 HashMap rehash 影响);
        // focus_ptr 同时存入 WebViewEntry,destroy 时置空防止悬垂。
        let focus_ptr: std::sync::Arc<parking_lot::Mutex<*const wry::WebView>> =
            std::sync::Arc::new(parking_lot::Mutex::new(std::ptr::null()));
        let holder = focus_ptr.clone();
        match wry::WebViewBuilder::new()
            .with_url(url)
            // 透明背景:让 WebView 透出下层 WarpUI 画面(深色主题下避免白底)。
            // 注意:页面自身背景仍需透明(如 body { background: transparent }),否则仍是白底。
            .with_transparent(true)
            .with_incognito(incognito)
            // 启用右键菜单"检查元素"(WebKit Web Inspector)。wry 默认值
            // debug=true/release=false,这里显式打开;release 下还需
            // wry 的 "devtools" feature 才会编译启用路径(见 app/Cargo.toml)。
            .with_devtools(true)
            .with_initialization_script_for_main_only(init_js, false)
            .with_on_page_load_handler(move |event, _url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    PENDING_WEBVIEW_URL_CHANGED.lock().insert(url_notify_id);
                }
            })
            // WebContent 渲染进程崩溃(如 macOS beta 的 JSC JIT bug,页面变
            // 死页)。入队经每帧 drain 分发,DshPane 弹出崩溃态 + 重新加载。
            .with_on_web_content_process_terminate_handler(move || {
                log::error!("[browser] webview {terminate_id} WebContent process terminated");
                PENDING_WEBVIEW_CRASHED.lock().insert(terminate_id);
            })
            .with_ipc_handler(move |request| {
                let body = request.body();
                log::debug!("[browser] ipc msg: {}", body);
                if body.starts_with("warp:webview-js-error:") {
                    // 页面 JS 错误(诊断):转发到日志。
                    log::warn!("[browser] webview {ipc_id} JS error: {}", &body["warp:webview-js-error:".len()..]);
                    return;
                }
                if let Some(url) = body.strip_prefix("warp:open-external:") {
                    // 页面链接点击:立即用系统默认浏览器打开。不能经每帧
                    // drain——点击发生在 WKWebView 内不产生 Warp 渲染帧,
                    // 帧回调不触发会把打开延迟到下次重绘(表现为切到浏览器
                    // 后才打开)。IPC handler 在主线程,直接同步调用平台
                    // open_url(NSWorkspace)。
                    if is_externally_openable(url) {
                        log::info!("[browser] webview {ipc_id} open external: {url}");
                        warpui::platform::mac::Window::open_url(url);
                    } else {
                        log::warn!("[browser] webview {ipc_id} skip non-openable url: {url}");
                    }
                    return;
                }
                // dsh 插件 IPC:zap.switch_project 等通知。
                // 仅允许已注册的 DSH webview 发送 zap: 消息。
                if let Some(payload) = body.strip_prefix("zap:") {
                    if DSH_WEBVIEW_IDS.lock().contains(&ipc_id) {
                        crate::dsh::bridge::handle_zap_ipc(payload);
                        // 推送事件后强制重绘，确保 on_frame_drawn → drain_events 执行。
                        // IPC 在事件循环空闲期到达时，on_frame_drawn 不会自然触发。
                        #[cfg(target_os = "macos")]
                        warpui::platform::mac::Window::request_redraw_all_windows();
                    } else {
                        log::warn!("[browser] webview {ipc_id} rejected zap: IPC (not a DSH pane)");
                    }
                    return;
                }
                if matches!(
                    body.as_str(),
                    "warp:webview-focusin" | "warp:webview-mousedown"
                ) {
                    log::debug!("[browser] ipc -> PENDING_WEBVIEW_FOCUS_EVENTS id={}", ipc_id);
                    PENDING_WEBVIEW_FOCUS_EVENTS.lock().insert(ipc_id);
                    // 立即请求重绘,让 PENDING 事件在本帧被 drain:否则事件要等
                    // 下一次自然重绘(实测可延迟 1-2s),延迟执行的焦点恢复会落在
                    // 用户后续交互中间(如刚打开的弹层被抢焦关闭)。
                    #[cfg(target_os = "macos")]
                    warpui::platform::mac::Window::request_redraw_all_windows();
                    let ptr = *holder.lock();
                    if !ptr.is_null() {
                        let _ = unsafe { (*ptr).focus() };
                    }
                }
            })
            .with_navigation_handler(move |target_url| {
                *handler_url.lock() = Some(target_url);
                true
            })
            // 下载:dsh 的 <a download>(如 Session 日志导出)由 WKWebView
            // 原生下载管道处理。wry 仅在配置了 download handler 后才挂
            // WKDownloadDelegate,否则下载请求无人应答、静默失败。下载继承
            // 页面的会话 cookie(dsh 的登录 token 只在 GET / 种 cookie,裸
            // URL 转系统浏览器只会 401)。
            .with_download_started_handler(|url, path| {
                log::info!("[browser] download started: {url} -> {}", path.display());
                // 对齐 Electron(未设 setSavePath 的默认例程):弹原生保存
                // 对话框让用户选位置,取消则拒绝本次下载。
                match run_download_save_panel(path) {
                    Some(chosen) => {
                        *path = chosen;
                        true
                    }
                    None => false,
                }
            })
            .with_download_completed_handler(|url, _result, success| {
                // macOS 上 result 恒为 None(wry 的 API 限制),成败以第三个参数为准。
                if success {
                    log::info!("[browser] download completed: {url}");
                } else {
                    log::error!("[browser] download failed: {url}");
                }
            })
            .with_bounds(Self::to_wry_rect(rect))
            .build_as_child(window)
        {
            Ok(webview) => {
                let webview = Box::new(webview);
                *focus_ptr.lock() = &*webview as *const wry::WebView;
                self.webviews.borrow_mut().insert(
                    id,
                    WebViewEntry {
                        webview,
                        window_id,
                        current_url,
                        last_bounds: None,
                        focus_ptr,
                    },
                );
            }
            Err(err) => {
                log::warn!("Failed to create webview {id}: {err}");
            }
        }
    }

    /// 非 macOS 平台空壳。**形参必须与 macOS 版一致**:调用点
    /// `BrowserPaneView::create_webview` 未按平台门控,少一个参数就会在
    /// Linux/Windows 上 E0061(评审 B2)。
    #[cfg(not(target_os = "macos"))]
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        &self,
        _window: &impl raw_window_handle::HasWindowHandle,
        _id: u64,
        _url: &str,
        _rect: RectF,
        _window_id: WindowId,
        _incognito: bool,
        _backend: WebViewBackend,
        _background_color: Option<warpui::color::ColorU>,
    ) {
    }

    /// 让 id 对应的 webview 跳转到 `url`。
    pub fn navigate(&self, id: u64, url: &str) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if let Some(entry) = self.cef_entries.borrow_mut().get_mut(&id) {
            *entry.current_url.lock() = Some(url.to_string());
            crate::browser::cef_backend::navigate(id, url);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            if let Err(err) = entry.webview.load_url(url) {
                log::warn!("Failed to navigate webview {id} to {url}: {err}");
            }
        }
    }

    /// 后退到上一页。
    pub fn go_back(&self, id: u64) {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            if let Err(err) = entry.webview.go_back() {
                log::warn!("Failed to go back in webview {id}: {err}");
            }
        }
    }

    /// 前进到下一页。
    pub fn go_forward(&self, id: u64) {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            if let Err(err) = entry.webview.go_forward() {
                log::warn!("Failed to go forward in webview {id}: {err}");
            }
        }
    }

    /// 重新加载当前页面。
    pub fn reload(&self, id: u64) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::reload(id);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            if let Err(err) = entry.webview.reload() {
                log::warn!("Failed to reload webview {id}: {err}");
            }
        }
    }

    /// 是否有可后退的历史记录(工具栏后退按钮可用性)。
    pub fn can_go_back(&self, id: u64) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            return entry.webview.can_go_back().unwrap_or(false);
        }
        false
    }

    /// 是否有可前进的历史记录(工具栏前进按钮可用性)。
    pub fn can_go_forward(&self, id: u64) -> bool {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            return entry.webview.can_go_forward().unwrap_or(false);
        }
        false
    }

    /// 最近一次导航的目标 URL(后退/前进后同步地址栏用)。
    pub fn current_url(&self, id: u64) -> Option<String> {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if let Some(entry) = self.cef_entries.borrow().get(&id) {
            return entry.current_url.lock().clone();
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            return entry.current_url.lock().clone();
        }
        None
    }


    /// 获取 webview 当前实际 URL(主 frame,非 subframe)。
    /// 使用 `wry::WebView::url()` 而非导航 handler 缓存的 current_url,
    /// 避免后退/前进时的竞态与 subframe 干扰。
    pub fn webview_url(&self, id: u64) -> Option<String> {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if let Some(entry) = self.cef_entries.borrow().get(&id) {
            // 优先用 CEF 侧记录的真实 URL(首载会经历 ?token= → 303 → 裸地址)。
            if let Some(url) = crate::browser::cef_backend::current_url(id) {
                return Some(url);
            }
            return entry.current_url.lock().clone();
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            return entry.webview.url().ok();
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = id;
        }
        None
    }
    /// 让 webview 页面内的活跃元素(如文本输入框)失去焦点。当地址栏聚焦时
    /// 调用,避免页面内的光标与地址栏光标共存。
    pub fn blur_webview_page(&self, id: u64) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::evaluate(id, "document.activeElement?.blur();");
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.evaluate_script("document.activeElement?.blur();");
        }
    }

    /// 让 webview 成为窗口的 first responder(键盘输入进页面)。
    /// 仅做 AppKit 层抢占,不改页面内 DOM 焦点:页面内每次 focusin/mousedown
    /// 都会经 IPC 走到这里,若在此恢复输入框焦点,会把用户正开着的模型菜单等
    /// 弹层的焦点抢走(菜单 onBlur 即关,点击落空,表现为切换失败)。
    pub fn focus_webview(&self, id: u64) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::focus(id, true);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.focus();
        }
    }

    /// [`Self::focus_webview`] + 恢复页面输入框焦点(WKWebView 失焦再聚焦
    /// 不会自动恢复页面 activeElement,`__restoreFocused` 按约定把光标折叠
    /// 到内容末尾)。只在可见性/焦点转变时机调用(attach、pane 获得焦点、
    /// 页面加载完成),不用于页面内的点击路径。
    pub fn focus_webview_restoring_input(&self, id: u64) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::focus(id, true);
            crate::browser::cef_backend::evaluate(
                id,
                "window.__restoreFocused && window.__restoreFocused();",
            );
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.focus();
            let _ = entry
                .webview
                .evaluate_script("window.__restoreFocused && window.__restoreFocused();");
        }
    }

    /// 将文本插入已注册的 DSH 插件 webview 的输入框(供 code review 等视图在 DSH
    /// 集成模式下把"添加到上下文"内容送到 DSH 会话,而非终端)。
    /// 找到首个已注册的 DSH webview 即注入;未就绪则静默放弃(由调用方决定是否提示)。
    pub fn insert_text_into_dsh_input(&self, text: &str) {
        let id = DSH_WEBVIEW_IDS.lock().iter().next().copied();
        let Some(id) = id else {
            log::warn!("[dsh] insert_text_into_dsh_input: no registered dsh webview");
            return;
        };
        // 用 serde_json 转义,避免文本含引号/换行破坏 JS 字符串字面量。
        let Ok(escaped) = serde_json::to_string(&text.to_string()) else {
            log::warn!("[dsh] insert_text_into_dsh_input: failed to escape text, skipping");
            return;
        };
        let js = format!(
            r#"
            (function() {{
                var text = {escaped};
                var el = document.querySelector('textarea[data-testid="dsh-input"]')
                         || document.querySelector('textarea[placeholder]')
                         || document.querySelector('div[contenteditable="true"][role="textbox"]')
                         || document.querySelector('div.ProseMirror')
                         || document.querySelector('.cm-content[contenteditable]');
                if (!el) {{ console.warn('[dsh] No input element found for attach_as_context'); return; }}
                if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {{
                    el.focus();
                    var start = el.selectionStart || el.value.length;
                    var end = el.selectionEnd || el.value.length;
                    el.setRangeText(text, start, end, 'end');
                    el.dispatchEvent(new InputEvent('input', {{ bubbles: true, cancelable: true }}));
                }} else {{
                    el.focus();
                    document.execCommand('insertText', false, text);
                }}
                // 光标由此处决定;随后的 focus_webview_restoring_input →
                // __restoreFocused 统一折叠到内容末尾,与「追加到末尾」的插入场景一致。
            }})()
            "#
        );
        self.evaluate_script_on(id, &js);
        self.focus_webview_restoring_input(id);
    }

    /// 在指定 webview 中执行 JavaScript。
    pub fn evaluate_script_on(&self, id: u64, script: &str) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::evaluate(id, script);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.evaluate_script(script);
        }
    }

    /// 把 id 对应的 webview 移动到 `rect`(逻辑坐标,origin 左上)。
    /// 仅在 rect 变化时调用 wry 的 `set_bounds`,避免每帧抖动。
    pub fn set_bounds(&self, id: u64, rect: RectF) {
        // CEF 子视图不跟随洞 rect,必须每帧驱动 frame + was_resized(集成要求 #2)。
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::set_bounds(id, rect);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow_mut().get_mut(&id) {
            let bounds = Self::to_wry_rect(rect);
            if entry.last_bounds != Some(bounds) {
                match entry.webview.set_bounds(bounds) {
                    Ok(()) => entry.last_bounds = Some(bounds),
                    Err(err) => {
                        // 失败时不更新 last_bounds,下一帧 rect 未变也会重试。
                        log::warn!("Failed to set bounds for webview {id}: {err}");
                    }
                }
            }
        }
    }


    /// 销毁 id 对应的 webview(pane 关闭时调用)。
    pub fn destroy(&self, id: u64) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow_mut().remove(&id).is_some() {
            crate::browser::cef_backend::destroy(id);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow_mut().remove(&id) {
            // 先置空 IPC handler 共享的裸指针,再 drop Box(webview),
            // 防止销毁后残留 IPC 回调解引用悬垂指针(use-after-free)。
            *entry.focus_ptr.lock() = std::ptr::null();
        }
    }

    /// 该 id 的 webview 是否仍存在(窗口关闭 cleanup 后可能已销毁)。
    pub fn has_webview(&self, id: u64) -> bool {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        {
            return self.cef_entries.borrow().contains_key(&id)
                || self.webviews.borrow().contains_key(&id);
        }
        #[cfg(all(target_os = "macos", not(feature = "cef_webview")))]
        {
            return self.webviews.borrow().contains_key(&id);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = id;
            false
        }
    }

    /// 销毁 `window_id` 窗口的所有 webview(窗口关闭时调用)。
    ///
    /// 窗口关闭走 `DetachType::HiddenForClose`(仅隐藏,供 undo 恢复),但窗口
    /// 一旦真正关闭且不恢复,pane 的 `Closed` detach 不会发生,webview 会
    /// 永久残留在全局 manager 中。窗口关闭钩子调用本方法兜底释放。
    pub fn cleanup_window(&self, window_id: WindowId) {
        // CEF 承载的条目同样要随窗口销毁(句柄在 cef_backend 内)。
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        {
            let ids: Vec<u64> = self
                .cef_entries
                .borrow()
                .iter()
                .filter(|(_, entry)| entry.window_id == window_id)
                .map(|(id, _)| *id)
                .collect();
            for id in ids {
                self.cef_entries.borrow_mut().remove(&id);
                crate::browser::cef_backend::destroy(id);
            }
        }
        #[cfg(target_os = "macos")]
        {
            let mut webviews = self.webviews.borrow_mut();
            webviews.retain(|_, entry| entry.window_id != window_id);
        }
    }

    /// 显示/隐藏 id 对应的 webview(undo 关闭宽限期间隐藏,pane 恢复时重新可见)。
    pub fn set_visible(&self, id: u64, visible: bool) {
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        if self.cef_entries.borrow().contains_key(&id) {
            crate::browser::cef_backend::set_visible(id, visible);
            return;
        }
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            if let Err(err) = entry.webview.set_visible(visible) {
                log::warn!("Failed to set visibility for webview {id}: {err}");
            }
        }
    }

    /// 消费 `window_id` 窗口上一帧暂存的 platform views,把每个 webview
    /// 定位到其声明 rect。由每帧 `on_frame_drawn` 回调调用。
    ///
    /// 本帧未上报的 webview(其 pane 不在当前可见树中,例如切到其他 tab)
    /// 会被隐藏,避免原生视图残留覆盖其它 UI。
    pub fn drain_pending_platform_views(&self, window_id: WindowId) {
        let views = PENDING_PLATFORM_VIEWS
            .lock()
            .remove(&window_id)
            .unwrap_or_default();
        #[cfg(target_os = "macos")]
        {
            let mut seen = std::collections::HashSet::new();
            for view in &views {
                // 重新可见(可能之前因未上报被隐藏),再定位。
                self.set_visible(view.id, true);
                self.set_bounds(view.id, view.rect);
                seen.insert(view.id);
            }
            let webviews = self.webviews.borrow();
            for (id, entry) in webviews.iter() {
                // 只隐藏本窗口的 webview:其他窗口的 webview 由各自窗口的
                // drain 管理,跨窗口隐藏会导致闪烁/误隐。
                if entry.window_id == window_id && !seen.contains(id) {
                    if let Err(err) = entry.webview.set_visible(false) {
                        log::warn!("Failed to hide webview {id}: {err}");
                    }
                }
            }
            // CEF 承载的条目同样要参与"未上报即隐藏":否则切 tab/离开可见树时
            // 既不释放 renderer(违背阶段 1.4 的隐藏即销毁),残留的原生视图还会
            // 在洞被别的 pane 复用时透出。
            #[cfg(all(target_os = "macos", feature = "cef_webview"))]
            {
                let hidden: Vec<u64> = self
                    .cef_entries
                    .borrow()
                    .iter()
                    .filter(|(id, entry)| entry.window_id == window_id && !seen.contains(id))
                    .map(|(id, _)| *id)
                    .collect();
                for id in hidden {
                    crate::browser::cef_backend::set_visible(id, false);
                }
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = views;
            let _ = window_id;
        }
    }
    /// 消费暂存的页面 focusin 事件与 URL 变更事件,经 model emit 分发。
    /// (外部打开请求在 IPC handler 内同步处理,不经此处。)
    pub fn drain_pending_webview_focus(&self, ctx: &mut ModelContext<Self>) {
        // CEF 创建失败的条目先移除:这样 pane 侧的 has_webview 归 false,下次 attach
        // 会重新创建(与下面的事件分发无关,故放在最前)。
        #[cfg(all(target_os = "macos", feature = "cef_webview"))]
        {
            let failed: Vec<u64> = PENDING_WEBVIEW_CREATE_FAILED.lock().drain().collect();
            if !failed.is_empty() {
                let mut entries = self.cef_entries.borrow_mut();
                for id in failed {
                    entries.remove(&id);
                    // 同时清 cef_backend 侧的状态:OSR 下自建宿主视图**先于浏览器**创建,
                    // 只删这里的条目会让那个 NSView 永久留在容器里(泄漏,且重建时再叠一层)。
                    crate::browser::cef_backend::destroy(id);
                    log::warn!("[cef] webview {id}: 创建失败已移除条目,等待重建");
                }
            }
        }

        let focus_events: Vec<BrowserWebViewEvent> = PENDING_WEBVIEW_FOCUS_EVENTS
            .lock()
            .drain()
            .map(BrowserWebViewEvent::PageFocused)
            .collect();
        let url_events: Vec<BrowserWebViewEvent> = PENDING_WEBVIEW_URL_CHANGED
            .lock()
            .drain()
            .map(BrowserWebViewEvent::UrlChanged)
            .collect();
        let crashed_events: Vec<BrowserWebViewEvent> = PENDING_WEBVIEW_CRASHED
            .lock()
            .drain()
            .map(BrowserWebViewEvent::WebContentCrashed)
            .collect();
        log::debug!(
            "[browser] drain events: focus={:?} url={:?} crashed={:?}",
            focus_events,
            url_events,
            crashed_events
        );
        for event in focus_events
            .into_iter()
            .chain(url_events)
            .chain(crashed_events)
        {
            ctx.emit(event);
        }
    }

    #[cfg(target_os = "macos")]
    fn to_wry_rect(rect: RectF) -> wry::Rect {
        wry::Rect {
            position: wry::dpi::LogicalPosition::new(rect.min_x() as f64, rect.min_y() as f64)
                .into(),
            size: wry::dpi::LogicalSize::new(rect.width() as f64, rect.height() as f64).into(),
        }
    }
}

impl Default for BrowserWebViewManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Entity for BrowserWebViewManager {
    type Event = BrowserWebViewEvent;
}

impl SingletonEntity for BrowserWebViewManager {}
