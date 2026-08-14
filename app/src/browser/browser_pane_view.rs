//! Web preview pane:地址栏 + 嵌入式 WebView(WKWebView)。
//!
//! 布局:pane 顶部固定高度地址栏,下方 `PlatformViewElement` 占满剩余空间。
//! 该元素的 rect 每帧经 `Scene.platform_views` 上报,由平台层定位真实 webview。

use pathfinder_geometry::rect::RectF;
use url::Url;
use warpui::{
    elements::{
        ChildView, Container, Expanded, Flex, MouseStateHandle,
        ParentElement, PlatformViewElement,
    },
    platform::Cursor,
    ui_components::components::UiComponent,
    AppContext, BlurContext, Element, Entity, FocusContext, ModelHandle, SingletonEntity,
    TypedActionView, View, ViewContext, ViewHandle, WindowId,
};

use crate::{
    appearance::Appearance,
    browser::browser_web_view::{BrowserWebViewEvent, BrowserWebViewManager, PENDING_PLATFORM_VIEWS},
    pane_group::{
        pane::view::{self, PaneView},
        pane::{DetachType, PaneContent, ShareableLink, ShareableLinkError},
        focus_state::PaneFocusHandle, BackingView, PaneConfiguration, PaneEvent, PaneId,
    },
    ui_components::{buttons::icon_button, icons::Icon},
    view_components::{SubmittableTextInput, SubmittableTextInputEvent},
};

/// 每个 BrowserPane 的状态:当前 URL 与 webview 的 platform-view id。
#[derive(Clone, Debug)]
pub struct BrowserPaneModel {
    pub url: String,
    pub platform_view_id: u64,
}

/// View for a web preview pane:一个地址栏 + 一个嵌入式 webview。
pub struct BrowserPaneView {
    model: BrowserPaneModel,
    address_bar: ViewHandle<SubmittableTextInput>,
    back_button: MouseStateHandle,
    forward_button: MouseStateHandle,
    reload_button: MouseStateHandle,
    pane_configuration: ModelHandle<PaneConfiguration>,
    window_id: WindowId,
    /// True after the pane was moved to another window (`DetachType::Moved`),
    /// which destroys the old window's webview. On re-attach the webview is
    /// recreated and the platform-view handler re-registered for the new window.
    needs_recreate: bool,
}

#[derive(Debug, Clone)]
pub enum BrowserPaneEvent {
    Pane(PaneEvent),
}

impl From<PaneEvent> for BrowserPaneEvent {
    fn from(event: PaneEvent) -> Self {
        BrowserPaneEvent::Pane(event)
    }
}

#[derive(Debug, Clone)]
pub enum BrowserPaneAction {
    Navigate(String),
    GoBack,
    GoForward,
    Reload,
    Focus,
    Close,
}

fn is_http_url(input: &str) -> bool {
    Url::parse(input).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

impl BrowserPaneView {
    /// Create a new web preview pane, opening `url` in an embedded webview.
    pub fn new(url: String, ctx: &mut ViewContext<Self>) -> Self {
        let pane_configuration = ctx.add_model(|_ctx| PaneConfiguration::new("Browser"));
        let platform_view_id = BrowserWebViewManager::as_ref(ctx).allocate_id();
        let window_id = ctx.window_id();

        let address_bar = ctx.add_typed_action_view(|ctx| {
            let mut input = SubmittableTextInput::new(ctx)
                .validate_on_submit(is_http_url)
                .with_border(false)
                .with_on_focus_callback({
                    let id = platform_view_id;
                    move |app| {
                        // 地址栏聚焦:blur 页面活跃元素(避免双光标)。first
                        // responder 由 host_view 的 mouseDown 处理切回 Warp。
                        BrowserWebViewManager::as_ref(app).blur_webview_page(id);
                    }
                });
            input.set_placeholder_text("Enter a URL, e.g. https://example.com", ctx);
            input.set_outer_margins(4., 4., ctx);
            input
        });

        let view = Self {
            model: BrowserPaneModel {
                url: url.clone(),
                platform_view_id,
            },
            address_bar,
            back_button: MouseStateHandle::default(),
            forward_button: MouseStateHandle::default(),
            reload_button: MouseStateHandle::default(),
            pane_configuration,
            window_id,
            needs_recreate: false,
        };

        ctx.subscribe_to_view(&view.address_bar, Self::handle_address_bar_event);

        // 订阅 webview 焦点事件:页面内元素获得焦点(focusin)时,释放地址栏
        // 的焦点与光标,避免两个光标共存。
        ctx.subscribe_to_model(
            &BrowserWebViewManager::handle(ctx),
            Self::handle_webview_event,
        );

        // 先以 0 尺寸创建 webview(不可见),随后 platform-view handler 会按
        // 每帧上报的 rect 定位它。
        view.create_webview(&url, ctx);
        // 地址栏显示当前地址。
        view.update_address_bar(&url, ctx);
        // 把本窗口每帧的 platform views 写入暂存队列,由 on_frame_drawn 消费。
        // 同一窗口多个 pane 重复注册同一逻辑,幂等。
        view.register_platform_view_handler(ctx);
        // 首次渲染由 `BrowserPane::attach` 挂载后的 notify 触发(此时视图树
        // 才包含本 pane);此处 notify 时 pane 尚未挂载,无法产生有效帧。


        view
    }

    pub fn model(&self) -> &BrowserPaneModel {
        &self.model
    }

    pub fn pane_configuration(&self) -> ModelHandle<PaneConfiguration> {
        self.pane_configuration.clone()
    }

    fn create_webview(&self, url: &str, ctx: &mut ViewContext<Self>) {
        if let Some(platform_window) = ctx.windows().platform_window(self.window_id) {
            if let Ok(handle) = platform_window.as_ref().window_handle() {
                BrowserWebViewManager::as_ref(ctx).create(
                    &handle,
                    self.model.platform_view_id,
                    url,
                    RectF::default(),
                    self.window_id,
                );
            }
        }
    }

    fn register_platform_view_handler(&self, ctx: &mut ViewContext<Self>) {
        if let Some(platform_window) = ctx.windows().platform_window(self.window_id) {
            let window_id = self.window_id;
            platform_window.as_ref().set_platform_view_handler(Box::new(move |views| {
                *PENDING_PLATFORM_VIEWS.lock().entry(window_id).or_default() = views;
            }));
        }
    }

    fn handle_address_bar_event(
        &mut self,
        _handle: ViewHandle<SubmittableTextInput>,
        event: &SubmittableTextInputEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        if let SubmittableTextInputEvent::Submit(url) = event {
            self.navigate(url.clone(), ctx);
        }
    }

    /// 释放/隐藏 webview,按 detach 类型区分:
    /// - `Closed`:永久关闭,销毁 webview(避免幽灵视图与泄漏)。
    /// - `HiddenForClose`:undo 宽限期,隐藏 webview(保留以便恢复)。
    /// - `Moved`:跨窗口移动,销毁源窗口 webview,待 attach 到新窗口时重建。
    fn handle_detach(&mut self, detach_type: DetachType, ctx: &mut ViewContext<Self>) {
        let manager = BrowserWebViewManager::as_ref(ctx);
        match detach_type {
            DetachType::Closed => manager.destroy(self.model.platform_view_id),
            DetachType::HiddenForClose => manager.set_visible(self.model.platform_view_id, false),
            DetachType::Moved => {
                manager.destroy(self.model.platform_view_id);
                self.needs_recreate = true;
            }
        }
    }

    /// Pane 附加(首次或恢复):跨窗口移动后在新窗口重建 webview 并重新注册
    /// handler;undo 恢复(HiddenForClose)时重新显示 webview。若 webview 已被
    /// 窗口关闭时的 cleanup 销毁,同样重建。
    fn handle_attach(&mut self, ctx: &mut ViewContext<Self>) {
        let manager = BrowserWebViewManager::as_ref(ctx);
        let webview_exists = manager.has_webview(self.model.platform_view_id);
        if self.needs_recreate || !webview_exists {
            self.needs_recreate = false;
            self.window_id = ctx.window_id();
            self.create_webview(&self.model.url, ctx);
            self.register_platform_view_handler(ctx);
        } else {
            manager.set_visible(self.model.platform_view_id, true);
        }
        self.focus_webview(ctx);
    }

    /// Pane 挂载完成后的收尾:默认焦点在 webview,把 AppKit first responder
    /// 切到 webview,键盘输入进页面(地址栏无光标)。pane 的 Warp 焦点由
    /// `focus_contents` 触发(on_focus 不再转移到地址栏)。
    fn focus_webview(&self, ctx: &mut ViewContext<Self>) {
        BrowserWebViewManager::as_ref(ctx).focus_webview(self.model.platform_view_id);
    }

    /// 让地址栏显示 `url`(创建时与每次导航后调用)。
    fn update_address_bar(&self, url: &str, ctx: &mut ViewContext<Self>) {
        self.address_bar.update(ctx, |input, ctx| {
            input.editor().update(ctx, |editor, ctx| {
                editor.set_buffer_text(url, ctx);
            });
        });
    }

    fn navigate(&mut self, url: String, ctx: &mut ViewContext<Self>) {
        BrowserWebViewManager::as_ref(ctx).navigate(self.model.platform_view_id, &url);
        self.model.url = url.clone();
        self.pane_configuration.update(ctx, |pane_config, ctx| {
            pane_config.set_title(url.clone(), ctx);
        });
        self.update_address_bar(&url, ctx);
        ctx.notify();
    }

    fn focus(&self, ctx: &mut ViewContext<Self>) {
        ctx.focus_self();
    }
    /// 后退/前进/页面加载后,把 webview 当前导航到的 URL 同步到地址栏与标题。
    /// 使用 `webview_url()`(主 frame URL),避免后退/前进时的竞态与 subframe 干扰。
    fn sync_url_from_webview(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(url) =
            BrowserWebViewManager::as_ref(ctx).webview_url(self.model.platform_view_id)
        {
            let url = url.trim_end_matches('/').to_string();
            if url != self.model.url {
                self.model.url = url.clone();
                self.pane_configuration.update(ctx, |pane_config, ctx| {
                    pane_config.set_title(url.clone(), ctx);
                });
                self.update_address_bar(&url, ctx);
            }
        }
    }

    /// 页面内元素获得焦点(focusin)→ 释放地址栏的 Warp 焦点,触发
    /// EditorView::on_blur 隐藏地址栏光标,避免与页面光标共存(双光标)。
    /// 页面加载完成(UrlChanged)→ 同步地址栏与 pane 标题。
    fn handle_webview_event(
        &mut self,
        _handle: ModelHandle<BrowserWebViewManager>,
        event: &BrowserWebViewEvent,
        ctx: &mut ViewContext<Self>,
    ) {
        match event {
            BrowserWebViewEvent::PageFocused(id) if *id == self.model.platform_view_id => {
                ctx.focus_self();
            }
            BrowserWebViewEvent::UrlChanged(id) if *id == self.model.platform_view_id => {
                self.sync_url_from_webview(ctx);
            }
            _ => {}
        }
    }
}
impl Entity for BrowserPaneView {
    type Event = BrowserPaneEvent;
}

impl View for BrowserPaneView {
    fn ui_name() -> &'static str {
        "BrowserPaneView"
    }
    fn on_focus(&mut self, focus_ctx: &FocusContext, ctx: &mut ViewContext<Self>) {
        // 默认焦点在 webview:pane 自身获得焦点(切 tab/切 pane 回来、页面
        // focusin 释放地址栏后)时,把 AppKit first responder 切到 webview,
        // 键盘输入进页面。
        if focus_ctx.is_self_focused() {
            BrowserWebViewManager::as_ref(ctx).focus_webview(self.model.platform_view_id);
        } else {
            // pane 的子视图(地址栏)获得焦点时,blur 掉 webview 页面内的活跃
            // 元素(如文本输入框),避免两个光标共存。地址栏自己的 callback 也
            // 做一件事,这里兜底,避免回调链路异常导致 blur 不生效。
            BrowserWebViewManager::as_ref(ctx)
                .blur_webview_page(self.model.platform_view_id);
        }
    }


    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        let can_go_back =

            BrowserWebViewManager::as_ref(app).can_go_back(self.model.platform_view_id);
        let can_go_forward =
            BrowserWebViewManager::as_ref(app).can_go_forward(self.model.platform_view_id);

        let back_button = icon_button(appearance, Icon::ArrowLeft, false, self.back_button.clone());
        let back_button = if can_go_back {
            back_button
        } else {
            back_button.disabled()
        };
        let back = back_button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BrowserPaneAction::GoBack);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
        let forward_button =
            icon_button(appearance, Icon::ArrowRight, false, self.forward_button.clone());
        let forward_button = if can_go_forward {
            forward_button
        } else {
            forward_button.disabled()
        };
        let forward = forward_button
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BrowserPaneAction::GoForward);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();
        let reload = icon_button(appearance, Icon::Refresh, false, self.reload_button.clone())
            .build()
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(BrowserPaneAction::Reload);
            })
            .with_cursor(Cursor::PointingHand)
            .finish();

        Flex::column()
            .with_child(
                Flex::row()
                    .with_spacing(6.)
                    .with_cross_axis_alignment(warpui::elements::CrossAxisAlignment::Center)
                    .with_child(
                        Container::new(back).with_margin_left(12.).finish(),
                    )
                    .with_child(forward)
                    .with_child(reload)
                    .with_child(
                        Expanded::new(
                            1.0,
                            Container::new(ChildView::new(&self.address_bar).finish())
                                .with_margin_left(10.)
                                .finish(),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_child(
                Expanded::new(
                    1.0,
                    PlatformViewElement::new(self.model.platform_view_id).finish(),
                )
                .finish(),
            )
            .finish()
    }
}

impl TypedActionView for BrowserPaneView {
    type Action = BrowserPaneAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            BrowserPaneAction::Navigate(url) => self.navigate(url.clone(), ctx),
            BrowserPaneAction::GoBack => {
                BrowserWebViewManager::as_ref(ctx).go_back(self.model.platform_view_id);
                self.sync_url_from_webview(ctx);
            }
            BrowserPaneAction::GoForward => {
                BrowserWebViewManager::as_ref(ctx).go_forward(self.model.platform_view_id);
                self.sync_url_from_webview(ctx);
            }
            BrowserPaneAction::Reload => {
                BrowserWebViewManager::as_ref(ctx).reload(self.model.platform_view_id)
            }
            BrowserPaneAction::Focus => self.focus(ctx),
            BrowserPaneAction::Close => self.close(ctx),
        }
    }
}

impl BackingView for BrowserPaneView {
    type PaneHeaderOverflowMenuAction = BrowserPaneAction;
    type CustomAction = ();
    type AssociatedData = ();

    /// 浏览器 pane 自带导航工具栏(后退/前进/刷新 + 地址栏),不显示
    /// PaneView 的标准 header,避免标题栏与地址栏重复。
    fn should_render_header(&self, _app: &AppContext) -> bool {
        false
    }

    fn handle_pane_header_overflow_menu_action(
        &mut self,
        action: &Self::PaneHeaderOverflowMenuAction,
        ctx: &mut ViewContext<Self>,
    ) {
        self.handle_action(action, ctx);
    }

    fn close(&mut self, ctx: &mut ViewContext<Self>) {
        // 不在这里销毁 webview:undo 关闭时会重建 pane,webview 由
        // BrowserWebViewManager 在场景中消失后统一清理。
        ctx.emit(BrowserPaneEvent::Pane(PaneEvent::Close));
    }

    fn focus_contents(&mut self, ctx: &mut ViewContext<Self>) {
        self.focus(ctx);
    }

    fn render_header_content(
        &self,
        _ctx: &view::HeaderRenderContext<'_>,
        _app: &AppContext,
    ) -> view::HeaderContent {
        view::HeaderContent::simple(self.model.url.clone())
    }

    fn set_focus_handle(&mut self, _focus_handle: PaneFocusHandle, _ctx: &mut ViewContext<Self>) {}
}

/// A pane that displays a web page in an embedded webview.
pub struct BrowserPane {
    view: ViewHandle<PaneView<BrowserPaneView>>,
    pane_configuration: ModelHandle<PaneConfiguration>,
}

impl BrowserPane {
    fn from_view(browser_view: ViewHandle<BrowserPaneView>, ctx: &mut AppContext) -> Self {
        let pane_configuration = browser_view.as_ref(ctx).pane_configuration();

        let view = ctx.add_typed_action_view(browser_view.window_id(ctx), |ctx| {
            let pane_id = PaneId::from_browser_pane_ctx(ctx);
            PaneView::new(pane_id, browser_view, (), pane_configuration.clone(), ctx)
        });

        Self {
            view,
            pane_configuration,
        }
    }

    /// Create a new web preview pane for the given URL.
    pub fn new<V: View>(url: String, ctx: &mut ViewContext<V>) -> Self {
        let view = ctx.add_typed_action_view(move |ctx| BrowserPaneView::new(url, ctx));
        Self::from_view(view, ctx)
    }

    pub fn browser_view(&self, ctx: &AppContext) -> ViewHandle<BrowserPaneView> {
        self.view.as_ref(ctx).child(ctx)
    }
}

impl PaneContent for BrowserPane {
    fn id(&self) -> PaneId {
        PaneId::from_browser_pane_view(&self.view)
    }

    fn attach(
        &self,
        _group: &crate::pane_group::PaneGroup,
        focus_handle: PaneFocusHandle,
        ctx: &mut ViewContext<crate::pane_group::PaneGroup>,
    ) {
        self.view
            .update(ctx, |view, ctx| view.set_focus_handle(focus_handle, ctx));

        // 跨窗口移动后重建 webview / undo 恢复时重新显示。
        self.browser_view(ctx)
            .update(ctx, |view, ctx| view.handle_attach(ctx));

        let pane_id = self.id();

        ctx.subscribe_to_view(
            &self.browser_view(ctx),
            move |pane_group, _, event, ctx| match event {
                BrowserPaneEvent::Pane(pane_event) => {
                    pane_group.handle_pane_event(pane_id, pane_event, ctx)
                }
            },
        );

        ctx.subscribe_to_view(&self.view, move |group, _, event, ctx| {
            group.handle_pane_view_event(pane_id, event, ctx);
        });

        // 请求一帧渲染:pane 挂载完成后再触发,此时视图树已包含本 pane,
        // 渲染帧会把 platform view 的真实 rect 上报给 BrowserWebViewManager,
        // 否则 webview 要等下一次交互(用户再点一下)才定位显示。
        ctx.notify();
    }

    fn detach(
        &self,
        _group: &crate::pane_group::PaneGroup,
        detach_type: DetachType,
        ctx: &mut ViewContext<crate::pane_group::PaneGroup>,
    ) {
        let browser_view = self.browser_view(ctx);
        // 按 detach 类型释放/隐藏 webview,见 `BrowserPaneView::handle_detach`。
        browser_view.update(ctx, |view, ctx| view.handle_detach(detach_type, ctx));
        ctx.unsubscribe_to_view(&browser_view);
        ctx.unsubscribe_to_view(&self.view);
    }

    fn snapshot(&self, app: &AppContext) -> crate::app_state::LeafContents {
        crate::app_state::LeafContents::Browser {
            url: self.browser_view(app).as_ref(app).model().url.clone(),
        }
    }

    fn has_application_focus(&self, ctx: &mut ViewContext<crate::pane_group::PaneGroup>) -> bool {
        self.view.is_self_or_child_focused(ctx)
    }

    fn focus(&self, ctx: &mut ViewContext<crate::pane_group::PaneGroup>) {
        self.browser_view(ctx).update(ctx, |view, ctx| view.focus(ctx));
    }

    fn shareable_link(
        &self,
        _ctx: &mut ViewContext<crate::pane_group::PaneGroup>,
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
