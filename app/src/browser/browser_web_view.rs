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
use std::sync::atomic::{AtomicU64, Ordering};

use parking_lot::Mutex;
use pathfinder_geometry::rect::RectF;
use warpui::{Entity, ModelContext, PlatformView, SingletonEntity, WindowId};

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
}

/// 页面加载完成事件的暂存区。由 wry 的 `on_page_load_handler`(主线程)
/// 写入,由每帧 `on_frame_drawn` 消费。消费后触发地址栏同步与 pane 标题更新。
pub(crate) static PENDING_WEBVIEW_URL_CHANGED: std::sync::LazyLock<
    Mutex<std::collections::HashSet<u64>>,
> = std::sync::LazyLock::new(|| Mutex::new(std::collections::HashSet::new()));

/// 可安全交给系统默认浏览器/应用打开的 URL scheme。`javascript:` 等伪协议
/// 一律过滤,防止把脚本串传给 NSWorkspace。
fn is_externally_openable(url: &str) -> bool {
    url::Url::parse(url)
        .is_ok_and(|parsed| matches!(parsed.scheme(), "http" | "https" | "mailto" | "tel"))
}

/// 全局单例:管理所有 webview 的 create / navigate / set_bounds / destroy。
pub struct BrowserWebViewManager {
    #[cfg(target_os = "macos")]
    webviews: RefCell<HashMap<u64, WebViewEntry>>,
    /// platform_view_id 的单调递增分配器(每个 pane 创建时取一个)。
    next_id: AtomicU64,
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

impl BrowserWebViewManager {
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "macos")]
            webviews: RefCell::new(HashMap::new()),
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
    #[cfg(target_os = "macos")]
    pub fn create(
        &self,
        window: &impl raw_window_handle::HasWindowHandle,
        id: u64,
        url: &str,
        rect: RectF,
        window_id: WindowId,
    ) {
        log::info!("[browser] create webview {id} url={url} rect={rect:?}");
        // wry 的 child webview 会拦截 performKeyEquivalent(Cmd 快捷键不
        // 进 webview),在页面内监听 Cmd+R 触发刷新;同时:当地址栏聚焦时
        // webview 失焦,页面活跃元素的焦点应自动释放,避免两处光标共存。
        // 页面内元素获得焦点(focusin)时经 IPC 上报 Rust,让 Warp 释放
        // 地址栏的焦点与光标(地址栏与页面各一个光标 = 双光标)。
        let init_js = r#"
window.__ZAP_BRIDGE__ = true;
// JS 错误/警告转发到 Rust 日志(诊断用)。
window.addEventListener('error', (e) => {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:' + (e.message || 'unknown'));
});
window.addEventListener('unhandledrejection', (e) => {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:unhandledrejection:' + String(e.reason).slice(0, 200));
});
const __origLog = console.error;
console.error = function(...args) {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-js-error:console:' + args.map(String).join(' ').slice(0, 300));
  __origLog.apply(console, args);
};

document.addEventListener('keydown', (e) => {
  if (!e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
  // Cmd+R → 页面刷新(wry child webview 的 performKeyEquivalent 返回 NO,
  // 不触发 KVO 刷新,需 JS 手动处理)。
  if (e.key === 'r' || e.key === 'R') {
    e.preventDefault();
    location.reload();
  }
});
document.addEventListener('focusin', () => {
  if (document.activeElement && document.activeElement !== document.body) {
    window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-focusin');
  }
});
// 点击页面任意位置上报 Rust,让 WKWebView 同步成为 first responder。
document.addEventListener('mousedown', () => {
  window.focus();
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:webview-mousedown');
});
// 点击链接:一律用系统默认浏览器打开,不在 webview 内导航。
// 锚点(#...)与 javascript: 伪协议链接不拦截。兼容 HTML 与 SVG <a>。
document.addEventListener('click', (e) => {
  if (e.button !== 0 && e.button !== 1) return;
  let el = e.target;
  while (el && el.tagName !== 'A') el = el.parentElement;
  if (!el || el.tagName !== 'A') return;
  const rawHref = el.getAttribute('href');
  if (!rawHref || rawHref.startsWith('#') || rawHref.startsWith('javascript:')) return;
  e.preventDefault();
  // HTML <a> 的 href 是字符串;SVG <a> 的是 SVGAnimatedString,需用
  // baseURI 重新解析。解析失败(非法 URL)则吞掉点击,不导航不外部打开。
  let target;
  try {
    target = typeof el.href === 'string' ? el.href : new URL(rawHref, document.baseURI).href;
  } catch {
    return;
  }
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:open-external:' + target);
});
// window.open():同样交给系统默认浏览器,不创建新窗口也不在当前 webview 导航。
window.open = function(url) {
  window.webkit?.messageHandlers?.ipc?.postMessage('warp:open-external:' + url);
  return null;
};
// 失焦时记录并 blur 页面输入框(防与 Warp 地址栏光标共存的双光标)。
// 记录元素供重新聚焦时(__restoreFocused)恢复:WKWebView 失焦再聚焦不会自动
// 恢复页面 activeElement,不恢复则切走再切回 tab 时输入框焦点丢失。
window.__lastFocused = null;
window.__restoreFocused = function() {
  var el = window.__lastFocused;
  if (!el || !el.isConnected) { window.__lastFocused = null; return; }
  var attempts = 0;
  var tryFocus = function() {
    // makeFirstResponder 后页面 hasFocus 需等 AppKit 事件循环才变 true,
    // 故轮询等待,有限次避免死循环。
    if (el.isConnected && document.hasFocus()) {
      el.focus();
      window.__lastFocused = null;
    } else if (el.isConnected && attempts++ < 20) {
      setTimeout(tryFocus, 30);
    } else {
      window.__lastFocused = null;
    }
  };
  tryFocus();
};
setInterval(() => {
  if (!document.hasFocus() && document.activeElement && document.activeElement !== document.body) {
    window.__lastFocused = document.activeElement;
    document.activeElement.blur();
  }
}, 100);
"#;
        let current_url = std::sync::Arc::new(parking_lot::Mutex::new(Some(url.to_string())));
        let handler_url = current_url.clone();
        let ipc_id = id;
        let url_notify_id = id;
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
            .with_initialization_script_for_main_only(init_js, false)
            .with_on_page_load_handler(move |event, _url| {
                if matches!(event, wry::PageLoadEvent::Finished) {
                    PENDING_WEBVIEW_URL_CHANGED.lock().insert(url_notify_id);
                }
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

    /// 非 macOS 平台空壳。
    #[cfg(not(target_os = "macos"))]
    pub fn create(
        &self,
        _window: &impl raw_window_handle::HasWindowHandle,
        _id: u64,
        _url: &str,
        _rect: RectF,
        _window_id: WindowId,
    ) {
    }
    /// 让 id 对应的 webview 跳转到 `url`。
    pub fn navigate(&self, id: u64, url: &str) {
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
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.evaluate_script("document.activeElement?.blur();");
        }
    }

    /// 让 webview 成为窗口的 first responder(键盘输入进页面)。
    /// pane 获得焦点/attach 时调用,确保默认焦点在 webview。
    pub fn focus_webview(&self, id: u64) {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.focus();
            // 恢复之前被 setInterval blur 的页面输入框焦点(WKWebView 失焦再
            // 聚焦不会自动恢复页面 activeElement)。
            let _ = entry
                .webview
                .evaluate_script("window.__restoreFocused && window.__restoreFocused();");
        }
    }

    /// 在指定 webview 中执行 JavaScript。
    pub fn evaluate_script_on(&self, id: u64, script: &str) {
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow().get(&id) {
            let _ = entry.webview.evaluate_script(script);
        }
    }

    /// 把 id 对应的 webview 移动到 `rect`(逻辑坐标,origin 左上)。
    /// 仅在 rect 变化时调用 wry 的 `set_bounds`,避免每帧抖动。
    pub fn set_bounds(&self, id: u64, rect: RectF) {
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
        #[cfg(target_os = "macos")]
        if let Some(entry) = self.webviews.borrow_mut().remove(&id) {
            // 先置空 IPC handler 共享的裸指针,再 drop Box(webview),
            // 防止销毁后残留 IPC 回调解引用悬垂指针(use-after-free)。
            *entry.focus_ptr.lock() = std::ptr::null();
        }
    }

    /// 该 id 的 webview 是否仍存在(窗口关闭 cleanup 后可能已销毁)。
    pub fn has_webview(&self, id: u64) -> bool {
        #[cfg(target_os = "macos")]
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
        #[cfg(target_os = "macos")]
        {
            let mut webviews = self.webviews.borrow_mut();
            webviews.retain(|_, entry| entry.window_id != window_id);
        }
    }

    /// 显示/隐藏 id 对应的 webview(undo 关闭宽限期间隐藏,pane 恢复时重新可见)。
    pub fn set_visible(&self, id: u64, visible: bool) {
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
        log::debug!("[browser] drain events: focus={:?} url={:?}", focus_events, url_events);
        for event in focus_events.into_iter().chain(url_events) {
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
