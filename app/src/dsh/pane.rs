//! DeepSeek Harness (dsh) pane:加载中 / WebUI 双态视图。
//!
//! 架构:
//! - `DshPaneView`:BackingView,持有 `Loading`|`Ready(webview)` 双态
//! - `DshPane`:PaneContent,持有 `PaneView<DshPaneView>`
//!
//! 流程:tab 打开时先显示 Loading spinner → runtime 就绪后创建 webview →
//! 自动切换至 Ready 态(DshPaneView::set_ready)。

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::appearance::Appearance;
use crate::app_state::LeafContents;
use crate::browser::{BrowserPaneView, BrowserWebViewEvent, BrowserWebViewManager};
use crate::dsh::DshRuntime;
use crate::pane_group::focus_state::PaneFocusHandle;
use crate::pane_group::pane::view::{self, PaneView};
use crate::pane_group::pane::{BackingView, DetachType, PaneContent, ShareableLink, ShareableLinkError, IPaneType};
use crate::pane_group::{PaneConfiguration, PaneGroup, PaneId};
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

enum DshPaneState {
    Loading,
    /// 正在安装/更新 dsh npm 包。is_install=true 为首次安装("安装中"),
    /// false 为版本更新("更新中");progress_lines 是最近若干条进度行
    /// (避免单行覆盖看不清)。
    Installing {
        is_install: bool,
        progress_lines: Vec<String>,
    },
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
}

impl DshPaneView {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("DeepSeek Harness"));
        Self {
            state: DshPaneState::Loading,
            pane_configuration,
            loading_anim_start: Arc::new(Mutex::new(Instant::now())),
            current_url: String::new(),
            webview_loaded: false,
            load_started_at: None,
            webview_id: None,
            focus_handle: None,
        }
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.state, DshPaneState::Loading | DshPaneState::Installing { .. })
    }

    /// 进入安装/更新状态。is_install=true 首次安装("安装中"),false 更新("更新中")。
    pub fn set_installing(&mut self, is_install: bool) {
        log::info!("[dsh] pane set_installing: is_install={is_install}");
        self.state = DshPaneState::Installing {
            is_install,
            progress_lines: Vec::new(),
        };
    }

    /// 追加一条安装进度行,保留最近若干行。
    pub fn update_installing_progress(&mut self, line: &str) {
        const MAX_LINES: usize = 6;
        match &mut self.state {
            DshPaneState::Installing { progress_lines, .. } => {
                progress_lines.push(line.to_string());
                if progress_lines.len() > MAX_LINES {
                    progress_lines.drain(0..progress_lines.len() - MAX_LINES);
                }
            }
            DshPaneState::Loading => {
                self.state = DshPaneState::Installing {
                    is_install: true,
                    progress_lines: vec![line.to_string()],
                };
            }
            DshPaneState::Ready(_) => {}
        }
    }
    pub fn url(&self) -> &str {
        &self.current_url
    }

    /// 就绪态下返回 BrowserPaneView 句柄(崩溃重启导航用)。
    pub fn get_browser_view(&self) -> Option<&ViewHandle<BrowserPaneView>> {
        match &self.state {
            DshPaneState::Ready(bv) => Some(bv),
            DshPaneState::Loading | DshPaneState::Installing { .. } => None,
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
        // 避免 runtime 就绪但页面还在加载时白屏闪烁。
        ctx.subscribe_to_model(
            &BrowserWebViewManager::handle(ctx),
            move |view, _, event, ctx| {
                if let BrowserWebViewEvent::UrlChanged(id) = event {
                    if *id == webview_id {
                        view.webview_loaded = true;
                        // 加载完成且本 pane 仍持焦点时补一次 focus_webview:
                        // on_focus 只在焦点转移时触发,set_ready(without_focus)
                        // 期间焦点未转移、on_focus 不会再次触发,若不补,用户
                        // 加载完成后直接打字会进 Warp 而非页面。焦点已离开本
                        // pane(用户切走)则不抢,由切回时 on_focus 正常切换。
                        if ctx.is_self_focused() {
                            BrowserWebViewManager::as_ref(ctx).focus_webview(webview_id);
                        }
                        ctx.notify();
                    }
                }
            },
        );
        self.state = DshPaneState::Ready(browser_view.clone());
        ctx.notify();
        browser_view
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
            DshPaneState::Installing {
                is_install,
                progress_lines,
            } => {
                let appearance = Appearance::as_ref(app);
                // 首次安装显示"安装中",版本更新显示"更新中",带省略号循环动画。
                let label = if *is_install { "安装中" } else { "更新中" };
                let mut col = Flex::column()
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
                            label,
                            appearance.ui_font_family(),
                            16.,
                            appearance.theme().foreground(),
                            self.loading_anim_start.clone(),
                        )))
                        .with_margin_top(16.)
                        .finish(),
                    );
                // 有进度行时,下方追加多行实时进度(无动画,仅文本)。
                for line in progress_lines.iter().rev() {
                    col = col.with_child(
                        Container::new(Box::new(Text::new_inline(
                            line.clone(),
                            appearance.ui_font_family(),
                            12.,
                        )))
                        .with_margin_top(6.)
                        .finish(),
                    );
                }
                Align::new(Box::new(col)).finish()
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
    type Action = ();
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

        // 跨窗口移动后重建 webview / undo 恢复时重新显示(Ready 态才有 webview)。
        // 移动后 webview 重建重载中(spinner 显示)不抢占焦点,与 set_ready 的
        // without_focus 一致;已加载恢复(HiddenForClose)则正常 attach 带焦点。
        if let Some(bv) = self.dsh_view(ctx).as_ref(ctx).get_browser_view().cloned() {
            let webview_loaded = self.dsh_view(ctx).as_ref(ctx).webview_loaded;
            bv.update(ctx, |view, ctx| {
                if webview_loaded {
                    view.handle_attach(ctx);
                } else {
                    view.handle_attach_without_focus(ctx);
                }
            });
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