//! DeepSeek Harness (dsh) pane:加载中 / WebUI 双态视图(+ runtime 失败覆盖态)。
//!
//! 架构:
//! - `DshPaneView`:BackingView,持有 `Loading`|`Ready(webview)` 双态,
//!   runtime 失败/停止时覆盖渲染错误态 + 重启入口
//! - `DshPane`:PaneContent,持有 `PaneView<DshPaneView>`
//!
//! 流程:tab 打开时先显示 Loading spinner → runtime 就绪后创建 webview →
//! 自动切换至 Ready 态(DshPaneView::set_ready)。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::appearance::Appearance;
use crate::app_state::LeafContents;
use crate::browser::{BrowserPaneAction, BrowserPaneView, BrowserWebViewEvent, BrowserWebViewManager};
use crate::dsh::bridge::BridgeEvent;
use crate::dsh::{DshRuntime, DshRuntimeStatus};
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, PaneView};
use crate::pane_group::pane::{BackingView, DetachType, PaneContent, ShareableLink, ShareableLinkError, IPaneType};
use crate::pane_group::{PaneConfiguration, PaneGroup, PaneId};
use crate::view_components::action_button::{ActionButton, PrimaryTheme};
use crate::workspace::WorkspaceAction;
use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::icons::Icon as WarpIcon;
use warpui::elements::*;
use warpui::event::DispatchedEvent;
use warpui::fonts::FamilyId;
use warpui::*;

/// webview 页面加载超时:超过该时长仍未收到加载完成事件,强制切换显示
/// webview(兜底,避免加载事件丢失导致 spinner 永久显示)。
const WEBVIEW_LOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

// ── 省略号打字动画 ──

/// 尾部省略号递增循环帧:`.` → `..` → `...` → 循环。
const ELLIPSIS_FRAMES: [&str; 3] = [".", "..", "..."];
const ELLIPSIS_INTERVAL_MS: u64 = 300;

/// 省略号打字动画文字:`prefix` 固定,尾部省略号每 `ELLIPSIS_INTERVAL_MS` 递增一档。
///
/// 复用 BrailleSpinner 的动画模式:用 `Arc<Mutex<Instant>>` 跨帧记录起始时刻,
/// 每帧 paint 后 `ctx.repaint_after` 请求下一次重绘(动画心跳)。
struct EllipsisText {
    prefix: String,
    family_id: FamilyId,
    font_size: f32,
    color: ColorU,
    start: Arc<Mutex<Instant>>,
    inner: Option<Text>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl EllipsisText {
    fn new(
        prefix: impl Into<String>,
        family_id: FamilyId,
        font_size: f32,
        color: impl Into<ColorU>,
        start: Arc<Mutex<Instant>>,
    ) -> Self {
        Self {
            prefix: prefix.into(),
            family_id,
            font_size,
            color: color.into(),
            start,
            inner: None,
            size: None,
            origin: None,
        }
    }

    fn dots(&self) -> &'static str {
        let start = *self.start.lock().expect("ellipsis state poisoned");
        let elapsed_ms = start.elapsed().as_millis() as u64;
        ELLIPSIS_FRAMES[((elapsed_ms / ELLIPSIS_INTERVAL_MS) % ELLIPSIS_FRAMES.len() as u64) as usize]
    }
}

impl Element for EllipsisText {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let full = format!("{}{}", self.prefix, self.dots());
        let mut text = Text::new_inline(full, self.family_id, self.font_size).with_color(self.color);
        let size = text.layout(constraint, ctx, app);
        self.inner = Some(text);
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        if let Some(t) = self.inner.as_mut() {
            t.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        if let Some(t) = self.inner.as_mut() {
            t.paint(origin, ctx, app);
        }
        // 每帧 paint 完请求 300ms 后再次重绘,触发下一档省略号——动画引擎心跳。
        ctx.repaint_after(Duration::from_millis(ELLIPSIS_INTERVAL_MS));
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }

    fn origin(&self) -> Option<Point> {
        self.origin
    }

    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}

// ── DshPaneState ──

/// DshPane 自身的 typed action(从 pane 内元素派发,经 responder chain
/// 由 [`DshPaneView::handle_action`] 处理)。
#[derive(Debug, Clone)]
pub enum DshPaneAction {
    /// 用户确认后重新加载已崩溃的 webview(仅重载页面,不重启 runtime:
    /// WebContent 崩溃是渲染进程问题,dsh 服务本身仍健康)。
    ReloadWebview,
}

enum DshPaneState {
    Loading,
    Ready(ViewHandle<BrowserPaneView>),
}

pub struct DshPaneView {
    state: DshPaneState,
    pane_configuration: ModelHandle<PaneConfiguration>,
    /// 加载态省略号动画的起始时刻(跨帧稳定,避免每帧 new 让动画重置到第 0 档)。
    loading_anim_start: Arc<Mutex<Instant>>,
    current_url: String,
    /// Ready 态下 webview 页面是否已加载完成(收到 UrlChanged 事件)。
    /// 加载完成前保持 spinner,避免白屏闪烁。
    webview_loaded: bool,
    /// webview 开始加载的时刻(超时兜底,加载事件丢失时强制切换)。
    load_started_at: Option<Instant>,
    /// DSH webview ID,用于 drop 时注销。
    webview_id: Option<u64>,
    /// pane focus 句柄(BackingView required)。
    focus_handle: Option<PaneFocusHandle>,
    /// dsh runtime 失败/已停止(启动失败、崩溃放弃重启、或恢复自已停止的
    /// pane):覆盖渲染为错误态 + 重启入口,避免展示指向已死服务的僵尸页面。
    runtime_failed: bool,
    /// 重启按钮(ActionButton 是 Entity,需以 ChildView 渲染;进入失败态时懒创建)。
    restart_button: Option<ViewHandle<ActionButton>>,
    /// WebContent 渲染进程已崩溃(runtime 仍健康):覆盖渲染崩溃态 +
    /// 重新加载入口,避免展示死页/白屏。wry 的导航委托恒实现
    /// webViewWebContentProcessDidTerminate,WebKit 不会自动重载,恢复
    /// 只能靠用户确认后的重建;收到 UrlChanged(重建完成)后自动清除。
    webview_crashed: bool,
    /// 重新加载按钮(ActionButton 是 Entity,需以 ChildView 渲染;进入
    /// 崩溃态时懒创建)。
    crash_reload_button: Option<ViewHandle<ActionButton>>,
}

impl DshPaneView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("DeepSeek Harness"));
        // 订阅 runtime 事件:失败切错误态,就绪/重启成功自动恢复。
        // webview 导航到新 URL 由 workspace 的 Ready/Restarted 处理,这里只翻标志位。
        ctx.subscribe_to_model(&DshRuntime::handle(ctx), |me, _, event, ctx| {
            match event {
                BridgeEvent::Failed { .. } => {
                    me.enter_runtime_failed(ctx);
                }
                BridgeEvent::Ready { .. } | BridgeEvent::Restarted { .. } => {
                    me.runtime_failed = false;
                    ctx.notify();
                }
                BridgeEvent::Notify { .. }
                | BridgeEvent::SwitchProject { .. }
                | BridgeEvent::OpenFileExplorer { .. }
                | BridgeEvent::OpenFile { .. } => {}
            }
        });
        Self {
            state: DshPaneState::Loading,
            pane_configuration,
            loading_anim_start: Arc::new(Mutex::new(Instant::now())),
            current_url: String::new(),
            webview_loaded: false,
            load_started_at: None,
            webview_id: None,
            focus_handle: None,
            runtime_failed: false,
            restart_button: None,
            webview_crashed: false,
            crash_reload_button: None,
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, DshPaneState::Loading)
    }

    pub fn url(&self) -> &str {
        &self.current_url
    }

    /// 就绪态下返回 BrowserPaneView 句柄(崩溃重启导航用)。
    pub fn get_browser_view(&self) -> Option<&ViewHandle<BrowserPaneView>> {
        match &self.state {
            DshPaneState::Ready(bv) => Some(bv),
            DshPaneState::Loading => None,
        }
    }

    /// Runtime 就绪后:创建无地址栏的 webview pane,切换状态。
    /// 返回 BrowserPaneView 句柄。
    pub fn set_ready(&mut self, url: &str, ctx: &mut ViewContext<Self>) -> ViewHandle<BrowserPaneView> {
        // 重入保护:已 Ready 时直接返回既有句柄,避免重复创建 webview 泄漏。
        if let DshPaneState::Ready(bv) = &self.state {
            return bv.clone();
        }
        self.current_url = url.to_string();
        let browser_view = ctx.add_typed_action_view(|ctx| {
            BrowserPaneView::new_with_options(url.to_string(), false, ctx)
        });
        // 不抢占焦点:Loading 显示期间 webview 不可见,键盘焦点改由 pane
        // 聚焦时(on_focus)正常切换,避免按键进不可见 webview。
        browser_view.update(ctx, |view, ctx| view.handle_attach_without_focus(ctx));
        let webview_id = browser_view.as_ref(ctx).model().platform_view_id;
        // 注册为 DSH webview,允许发送 zap: IPC 消息。
        crate::browser::BrowserWebViewManager::register_dsh_webview(webview_id);
        self.webview_id = Some(webview_id);
        self.load_started_at = Some(Instant::now());
        self.webview_loaded = false;
        // 订阅 webview 加载完成(UrlChanged):完成后才从 spinner 切到 webview,
        // 避免 runtime 就绪但页面还在加载时白屏闪烁。WebContentCrashed 时
        // 弹出崩溃态(渲染进程崩溃,runtime 仍健康)。
        ctx.subscribe_to_model(
            &BrowserWebViewManager::handle(ctx),
            move |view, _, event, ctx| {
                match event {
                    BrowserWebViewEvent::UrlChanged(id) if *id == webview_id => {
                        view.webview_loaded = true;
                        // 加载完成且本 pane 仍持焦点时补一次 focus_webview:
                        // on_focus 只在焦点转移时触发,set_ready(without_focus)
                        // 期间焦点未转移、on_focus 不会再次触发,若不补,用户
                        // 加载完成后直接打字会进 Warp 而非页面。焦点已离开本
                        // pane(用户切走)则不抢,由切回时 on_focus 正常切换。
                        if ctx.is_self_focused() {
                            BrowserWebViewManager::as_ref(ctx).focus_webview(webview_id);
                        }
                        // 页面(重)加载完成:清除崩溃态。wry 的导航委托恒实现
                        // webViewWebContentProcessDidTerminate,WebKit 视为
                        // "客户端已接管",不会自动重载;此清除覆盖的是重建
                        // 完成后的正常揭示。
                        view.webview_crashed = false;
                        ctx.notify();
                    }
                    BrowserWebViewEvent::WebContentCrashed(id) if *id == webview_id => {
                        view.enter_webview_crashed(ctx);
                    }
                    // 其他 webview 的事件与 PageFocused:与本 pane 无关。
                    _ => {}
                }
            },
        );
        self.state = DshPaneState::Ready(browser_view.clone());
        ctx.notify();
        browser_view
    }

    /// webview 将被覆盖层换出隐藏时,把 AppKit first responder 从(可能仍
    /// 持焦的)WKWebView 还给 host view:AppKit 的 mouseMoved 按 responder
    /// chain 投递给 first responder(不像 mouseDown 走 hitTest),不还原则
    /// 覆盖层按钮无 hover 效果(点击不受影响)。判定用 is_self_or_child_
    /// focused:点击页面时 warp 焦点落在子视图 BrowserPaneView 上(其
    /// handle_webview_event 里 focus_self),严格 is_self_focused 恒为
    /// false 会漏掉主场景;且仅在本 pane(或其 webview)持焦点时才执行,
    /// 避免抢占其他 pane(如另一 webview)的键盘焦点。
    #[cfg(target_os = "macos")]
    fn restore_host_first_responder(&self, ctx: &ViewContext<Self>) {
        if ctx.is_self_or_child_focused() {
            warpui::platform::mac::Window::focus_host_view(ctx.window_id());
        }
    }

    /// 进入 runtime 失败态:置标志并确保重启按钮视图存在(ActionButton 是
    /// Entity,需以 ChildView 渲染,懒创建避免健康路径开销)。
    fn enter_runtime_failed(&mut self, ctx: &mut ViewContext<Self>) {
        // 覆盖层会换出并隐藏 webview,还原 first responder 保证按钮 hover
        // 可用(机理同 webview 崩溃态,见 restore_host_first_responder)。
        #[cfg(target_os = "macos")]
        self.restore_host_first_responder(ctx);
        self.runtime_failed = true;
        if self.restart_button.is_none() {
            self.restart_button = Some(ctx.add_typed_action_view(|_ctx| {
                ActionButton::new("重新启动", PrimaryTheme).on_click(|ctx| {
                    ctx.dispatch_typed_action(WorkspaceAction::OpenDshPane)
                })
            }));
        }
        ctx.notify();
    }

    /// 进入 webview 崩溃态:WebContent 渲染进程已终止(runtime 仍健康)。
    /// 覆盖渲染崩溃说明 + 重新加载入口,避免展示死页/白屏。用户点击重新
    /// 加载(确认)后经 [`DshPaneAction::ReloadWebview`] 重建 webview。
    fn enter_webview_crashed(&mut self, ctx: &mut ViewContext<Self>) {
        log::error!(
            "[dsh] webview crashed, showing reload prompt (webview_id={:?})",
            self.webview_id
        );
        // 覆盖层换出 webview,还原 first responder 保证按钮 hover 可用
        // (机理见 restore_host_first_responder)。
        #[cfg(target_os = "macos")]
        self.restore_host_first_responder(ctx);
        self.webview_crashed = true;
        if self.crash_reload_button.is_none() {
            self.crash_reload_button = Some(ctx.add_typed_action_view(|_ctx| {
                ActionButton::new("重新加载", PrimaryTheme).on_click(|ctx| {
                    ctx.dispatch_typed_action(DshPaneAction::ReloadWebview)
                })
            }));
        }
        ctx.notify();
    }

    /// 用户确认后执行:销毁崩溃的 webview 并按既有重建路径(Moved 同款)以
    /// 当前 URL 重建——全新 WKWebView 与全新渲染进程,不携带崩溃后残留的
    /// layer/进程状态;收到 UrlChanged(加载完成)后从 spinner 切回,与初始
    /// 打开的揭示时机一致。仅重建页面,不重启 runtime(WebContent 崩溃是
    /// 渲染进程问题,dsh 服务本身仍健康)。
    fn recreate_webview(&mut self, ctx: &mut ViewContext<Self>) {
        self.webview_crashed = false;
        if let Some(id) = self.webview_id {
            BrowserWebViewManager::as_ref(ctx).destroy(id);
            self.load_started_at = Some(Instant::now());
            self.webview_loaded = false;
            // handle_attach 检测到 webview 缺失会以当前 URL 重建并重注册
            // platform-view handler;不带焦点重建,键盘不进加载中的 webview。
            if let Some(bv) = self.get_browser_view().cloned() {
                bv.update(ctx, |view, ctx| view.handle_attach_without_focus(ctx));
            }
        }
        ctx.notify();
    }

    /// attach 时把本 pane 与当前 runtime 实例的 URL 同步:每次启动的 URL
    /// (随机端口/token)都会变。以 webview 实际 URL 为准(browser model 实时
    /// 同步)按 origin 比较——dsh 对 token URL 做 303 重定向并种 cookie,加载
    /// 完成后 model.url 已漂移为无 token 裸地址,完整 URL 相等恒不成立;只有
    /// 端口能稳定标识实例。同源则无需动作;跨实例(重启后恢复旧 pane)导航
    /// 到新 URL,避免展示指向已死旧实例的僵尸页面;Loading 态直接就绪。
    fn sync_runtime_url(&mut self, runtime_url: &str, ctx: &mut ViewContext<Self>) {
        // 一次 clone 复用句柄:两次 get_browser_view() 之间仅写 current_url,
        // state 不变,结果恒等。
        let browser_view = self.get_browser_view().cloned();
        let live_url = browser_view
            .as_ref()
            .map(|bv| bv.as_ref(ctx).model().url.clone());
        match live_url {
            Some(live) if Self::url_origin(&live) == Self::url_origin(runtime_url) => return,
            Some(_) => {
                self.current_url = runtime_url.to_string();
                if let Some(bv) = browser_view {
                    bv.update(ctx, |view, ctx| {
                        view.handle_action(
                            &BrowserPaneAction::Navigate(runtime_url.to_string()),
                            ctx,
                        );
                    });
                }
            }
            None => {
                self.set_ready(runtime_url, ctx);
            }
        }
    }

    /// 取 `scheme://host:port` 前缀:dsh 的 303 重定向只丢 token query,端口
    /// 不变;每实例端口随机,端口相同即同一实例。
    fn url_origin(url: &str) -> &str {
        match url.find("://") {
            Some(scheme_end) => {
                let after_scheme = scheme_end + 3;
                let host_end = url[after_scheme..]
                    .find('/')
                    .map(|i| after_scheme + i)
                    .unwrap_or(url.len());
                &url[..host_end]
            }
            None => url,
        }
    }

    /// runtime 失败态:图标 + 说明 + 重启入口(布局与 Loading 态一致)。
    /// 重启复用 WorkspaceAction::OpenDshPane(Stopped/Failed 态下 begin_start,
    /// 就绪后 workspace 会把本 pane 导航到新 URL)。
    fn render_runtime_failed(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Box::new(
                ConstrainedBox::new(Box::new(Icon::new(
                    WarpIcon::DeepSeek.into(),
                    appearance.theme().foreground(),
                )))
                .with_width(40.)
                .with_height(40.),
            ))
            .with_child(
                Container::new(Box::new(
                    Text::new_inline(
                        "DeepSeek Harness 已停止",
                        appearance.ui_font_family(),
                        16.,
                    )
                    .with_color(appearance.theme().foreground().into()),
                ))
                .with_margin_top(16.)
                .finish(),
            );
        if let Some(button) = &self.restart_button {
            column = column.with_child(
                Container::new(Box::new(ChildView::new(button)))
                    .with_margin_top(16.)
                    .finish(),
            );
        }
        Align::new(column.finish()).finish()
    }

    /// webview 崩溃态:图标 + 说明 + 重新加载入口(布局与 runtime 失败态一致)。
    /// 仅重载页面即可恢复,不涉及 runtime 重启。
    fn render_webview_crashed(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let mut column = Flex::column()
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Box::new(
                ConstrainedBox::new(Box::new(Icon::new(
                    WarpIcon::DeepSeek.into(),
                    appearance.theme().foreground(),
                )))
                .with_width(40.)
                .with_height(40.),
            ))
            .with_child(
                Container::new(Box::new(
                    Text::new_inline(
                        "DeepSeek Harness 页面已崩溃",
                        appearance.ui_font_family(),
                        16.,
                    )
                    .with_color(appearance.theme().foreground().into()),
                ))
                .with_margin_top(16.)
                .finish(),
            )
            .with_child(
                Container::new(Box::new(
                    Text::new_inline(
                        "页面渲染进程异常终止,重新加载即可恢复。",
                        appearance.ui_font_family(),
                        13.,
                    )
                    .with_color(appearance.theme().foreground().into()),
                ))
                .with_margin_top(8.)
                .finish(),
            );
        if let Some(button) = &self.crash_reload_button {
            column = column.with_child(
                Container::new(Box::new(ChildView::new(button)))
                    .with_margin_top(16.)
                    .finish(),
            );
        }
        Align::new(column.finish()).finish()
    }
}

impl Entity for DshPaneView {
    type Event = ();
}

impl Drop for DshPaneView {
    fn drop(&mut self) {
        // 注销 DSH webview ID,避免 IPC handler 仍接受已销毁 webview 的消息。
        if let Some(id) = self.webview_id {
            crate::browser::BrowserWebViewManager::unregister_dsh_webview(id);
        }
    }
}

impl View for DshPaneView {
    fn ui_name() -> &'static str {
        "DshPaneView"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        // runtime 失败/已停止:显示错误态 + 重启入口,而非指向已死服务的
        // 僵尸页面或无限转圈。
        if self.runtime_failed {
            return self.render_runtime_failed(app);
        }
        // webview 渲染进程崩溃(runtime 仍健康):显示崩溃态 + 重新加载入口。
        // 与 runtime 失败态区分:重载页面即可恢复,不重启 dsh 服务。
        if self.webview_crashed {
            return self.render_webview_crashed(app);
        }
        // webview 已加载完成(或加载超时兜底)才显示 webview;否则保持 spinner。
        let show_webview = matches!(self.state, DshPaneState::Ready(_))
            && (self.webview_loaded
                || self
                    .load_started_at
                    .is_some_and(|t| t.elapsed() > WEBVIEW_LOAD_TIMEOUT));
        match &self.state {
            DshPaneState::Ready(browser_view) if show_webview => {
                ChildView::new(browser_view).finish()
            }
            DshPaneState::Ready(_) => {
                // webview 加载中或未达超时兜底:显示省略号动画(与 Loading 一致,
                // 避免"启动中"静态文字让用户误以为卡死)。
                let appearance = Appearance::as_ref(app);
                Align::new(
                    Flex::column()
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Box::new(
                            ConstrainedBox::new(Box::new(Icon::new(
                                WarpIcon::DeepSeek.into(),
                                appearance.theme().foreground(),
                            )))
                            .with_width(40.)
                            .with_height(40.),
                        ))
                        .with_child(
                            Container::new(Box::new(EllipsisText::new(
                                "DeepSeek Harness 启动中",
                                appearance.ui_font_family(),
                                16.,
                                appearance.theme().foreground(),
                                self.loading_anim_start.clone(),
                            )))
                            .with_margin_top(16.)
                            .finish(),
                        )
                        .finish(),
                )
                .finish()
            }
            DshPaneState::Loading => {
                let appearance = Appearance::as_ref(app);
                Align::new(
                    Flex::column()
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(Box::new(
                            ConstrainedBox::new(Box::new(Icon::new(
                                WarpIcon::DeepSeek.into(),
                                appearance.theme().foreground(),
                            )))
                            .with_width(40.)
                            .with_height(40.),
                        ))
                        .with_child(
                            Container::new(Box::new(EllipsisText::new(
                                "DeepSeek Harness 启动中",
                                appearance.ui_font_family(),
                                16.,
                                appearance.theme().foreground(),
                                self.loading_anim_start.clone(),
                            )))
                            .with_margin_top(16.)
                            .finish(),
                        )
                        .finish(),
                )
                .finish()
            }
        }
    }
}

impl TypedActionView for DshPaneView {
    type Action = DshPaneAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            DshPaneAction::ReloadWebview => self.recreate_webview(ctx),
        }
    }
}

impl BackingView for DshPaneView {
    type PaneHeaderOverflowMenuAction = ();
    type CustomAction = ();
    type AssociatedData = ();

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        _action: &Self::PaneHeaderOverflowMenuAction,
        _ctx: &mut ViewContext<Self>,
    ) {
    }

    // dsh WebUI 自带完整界面,隐藏 in-pane header(避免出现标题栏),
    // 与 BrowserPane 一致;tab 栏标题由 PaneConfiguration("DeepSeek") 提供。
    fn should_render_header(&self, _app: &AppContext) -> bool {
        false
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // 关闭 pane 时停止 dsh runtime,避免子进程泄漏。
        crate::dsh::DshRuntime::handle(ctx).update(ctx, |runtime, _ctx| {
            runtime.request_stop();
        });
    }

    fn focus_contents(&mut self, _ctx: &mut ViewContext<Self>) {
        // 无特殊 focus 行为;PaneView 已处理 pane 级别 focus。
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple("DeepSeek Harness")
    }

    fn set_focus_handle(&mut self, focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {
        self.focus_handle = Some(focus_handle);
    }
}

pub struct DshPane {
    view: ViewHandle<PaneView<DshPaneView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}


impl DshPane {
    /// 创建 DeepSeek tab,立即显示 Loading 态。Runtime 异步启动。
    pub fn new<V: View>(ctx: &mut ViewContext<V>) -> Self {
        let dsh_view = ctx.add_typed_action_view(move |ctx| DshPaneView::new(ctx));
        Self::from_view(dsh_view, ctx)
    }

    fn from_view(
        dsh_view: ViewHandle<DshPaneView>,
        ctx: &mut AppContext,
    ) -> Self {
        let pane_configuration = dsh_view.as_ref(ctx).pane_configuration();
        let window_id = dsh_view.window_id(ctx);
        let pane_view: ViewHandle<PaneView<DshPaneView>> =
            ctx.add_typed_action_view(window_id, |ctx| {
                let pane_id = PaneId::new_from_ctx(IPaneType::DeepSeek, ctx);
                PaneView::new(pane_id, dsh_view.clone(), (), pane_configuration.clone(), ctx)
            });
        Self {
            view: pane_view,
            pane_configuration,
        }
    }

    pub fn dsh_view(&self, ctx: &AppContext) -> ViewHandle<DshPaneView> {
        self.view.as_ref(ctx).child(ctx)
    }

    /// 此 pane 是否仍在 Loading 态(runtime 未就绪)。
    pub fn is_loading(&self, ctx: &AppContext) -> bool {
        self.dsh_view(ctx).as_ref(ctx).is_loading()
    }

    /// 恢复 webview(Ready 态才有):默认已加载恢复(HiddenForClose)走带焦点
    /// attach、重建重载中(spinner)不抢占焦点;`force_without_focus` 用于
    /// 停止态(错误态即将换出 webview),一律不抢占焦点——同时保持缺失
    /// webview 的重建(lib.rs「undo 恢复时 handle_attach 检测到 webview 缺失
    /// 会重建」不变量),否则重启成功后会渲染指向不存在 webview 的空白 pane。
    fn attach_webview(&self, force_without_focus: bool, ctx: &mut ViewContext<PaneGroup>) {
        if let Some(bv) = self.dsh_view(ctx).as_ref(ctx).get_browser_view().cloned() {
            let webview_loaded = self.dsh_view(ctx).as_ref(ctx).webview_loaded;
            let with_focus = !force_without_focus && webview_loaded;
            bv.update(ctx, |view, ctx| {
                if with_focus {
                    view.handle_attach(ctx);
                } else {
                    view.handle_attach_without_focus(ctx);
                }
            });
        }
    }
}

impl PaneContent for DshPane {
    fn id(&self) -> PaneId {
        PaneId::new(IPaneType::DeepSeek, &self.view)
    }

    fn attach(
        &self,
        _group: &PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // 先读 runtime 状态再决定 webview attach / 失败态:停止态下不让即将
        // 被错误视图换出的 webview 抢占焦点(避免重引入 2848eaeec 修掉的
        // 「按键进不可见 webview」缺陷)。
        let (runtime_status, runtime_url) = DshRuntime::handle(ctx).read(ctx, |runtime, _| {
            (runtime.status(), runtime.url().map(str::to_string))
        });
        match runtime_status {
            DshRuntimeStatus::Starting => {
                // 启动/重启进行中:webview 按既有逻辑恢复,等 Ready/Restarted
                // 事件导航到新 URL;Loading 态等待就绪。
                self.attach_webview(false, ctx);
            }
            DshRuntimeStatus::Ready => {
                // 先同步 URL 再 attach:需重建的 webview(Moved/窗口清理销毁
                // 后)直接以当前实例 URL 创建,避免先加载已死旧地址再被导航
                // (白费一次加载)。同源(同端口)则不动作。
                if let Some(url) = runtime_url {
                    self.dsh_view(ctx).update(ctx, |view, ctx| {
                        view.sync_runtime_url(&url, ctx);
                    });
                }
                self.attach_webview(false, ctx);
            }
            DshRuntimeStatus::Stopped | DshRuntimeStatus::Failed => {
                // undo 恢复/复用既有 pane 而 runtime 已停止(detach 时
                // request_stop):置失败态显示重启入口,避免展示指向已死服务
                // 的僵尸页面或永久转圈(Loading 态无 webview 也一并覆盖)。
                // 新建 pane 的 attach 发生在 open_dsh_pane 的 begin_start 之后,
                // 正常启动流程不会走到这里。webview 仍以无焦点方式 attach:
                // Moved/窗口清理销毁后在这里重建(保持 lib.rs「undo 恢复时
                // handle_attach 检测到 webview 缺失会重建」不变量),否则重启
                // 成功清掉错误态后会渲染指向不存在 webview 的空白 pane。
                self.attach_webview(true, ctx);
                self.dsh_view(ctx).update(ctx, |view, ctx| {
                    if !view.runtime_failed {
                        view.enter_runtime_failed(ctx);
                    }
                });
            }
        }

        let pane_id = self.id();
        // 订阅 PaneView 事件(拖拽/移动/焦点恢复),与 BrowserPane 一致。
        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        ctx.notify();
    }

    fn detach(&self, _group: &PaneGroup, detach_type: DetachType, ctx: &mut ViewContext<PaneGroup>) {
        log::info!("[dsh] DshPane::detach: {detach_type:?}");
        // Ready 态下转发 webview 生命周期:Closed 销毁 / HiddenForClose 隐藏 /
        // Moved 销毁并重建,避免关闭 tab 泄漏 WKWebView。
        if let Some(bv) = self.dsh_view(ctx).as_ref(ctx).get_browser_view().cloned() {
            bv.update(ctx, |view, ctx| view.handle_detach(detach_type, ctx));
        }
        ctx.unsubscribe_to_view(&self.view);
        if matches!(detach_type, DetachType::Moved) {
            // Moved 会销毁 webview,重 attach 时以同 id 重建并重新加载页面:
            // 重置加载时钟与完成标志,否则旧的 load_started_at 起点接近
            // WEBVIEW_LOAD_TIMEOUT 时,新加载刚开始就被兜底判超时、提前白屏。
            self.dsh_view(ctx).update(ctx, |view, _ctx| {
                view.load_started_at = Some(Instant::now());
                view.webview_loaded = false;
            });
        }
        if matches!(detach_type, DetachType::Closed | DetachType::HiddenForClose) {
            log::info!("[dsh] DshPane detached; requesting runtime stop");
            DshRuntime::handle(ctx).update(ctx, |runtime, _ctx| {
                runtime.request_stop();
            });
        }
    }

    fn snapshot(&self, app: &AppContext) -> LeafContents {
        // Ready 态读 live URL(webview 导航实时同步),避免 Loading 态/导航后
        // 快照拿到静态 current_url 导致恢复出陈旧或空白 URL。
        let url = match self.dsh_view(app).as_ref(app).get_browser_view() {
            Some(bv) => bv.as_ref(app).model().url.clone(),
            None => self.dsh_view(app).as_ref(app).url().to_string(),
        };
        LeafContents::Browser { url }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<PaneGroup>) {
        // 直接 focus DshPaneView(BackingView),而非 PaneView。
        self.dsh_view(ctx).update(ctx, |view, ctx| {
            view.focus_contents(ctx);
        });
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<PaneGroup>,
    ) -> Result<ShareableLink, ShareableLinkError> {
        Ok(ShareableLink::Base)
    }

    fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn is_pane_being_dragged(&self, ctx: &AppContext) -> bool {
        self.view.as_ref(ctx).is_being_dragged()
    }
}