use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;

use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    ChildAnchor, ChildView, Container, CrossAxisAlignment, Flex, MainAxisSize,
    OffsetPositioning, ParentElement as _, PositionedElementAnchor,
    PositionedElementOffsetBounds, SavePosition, Stack, Text,
};
use warpui::{AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle};
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::terminal::cli_agent_sessions::CLIAgentSessionsModel;
use crate::terminal::omp_models::OmpModelRegistry;
use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, TooltipAlignment,
};

const MENU_WIDTH: f32 = 280.;
const LABEL_FONT_SIZE: f32 = 11.;

#[derive(Clone)]
struct OmpButtonTheme;

impl ActionButtonTheme for OmpButtonTheme {
    fn background(&self, _: bool, appearance: &Appearance) -> Option<warp_core::ui::theme::Fill> {
        Some(appearance.theme().surface_1())
    }
    fn text_color(&self, _: bool, _: Option<warp_core::ui::theme::Fill>, appearance: &Appearance) -> pathfinder_color::ColorU {
        internal_colors::neutral_7(appearance.theme())
    }
}

#[derive(Debug, Clone)]
pub enum OmpModelSelectorAction {
    SelectModel { selector: String },
    ToggleMenu,
}

#[derive(Debug, Clone)]
pub enum OmpModelSelectorEvent {
    ModelSelected { selector: String },
    /// Menu dismissed via ESC or click-outside.
    MenuClosed,
    /// Menu opened.
    MenuOpened,
}

pub struct OmpModelSelector {
    terminal_view_id: EntityId,
    omp_binary: String,
    registry: OmpModelRegistry,
    selected_model: Option<String>,
    loading: bool,
    menu_is_open: bool,
    button: ViewHandle<ActionButton>,
    dropdown: ViewHandle<Menu<OmpModelSelectorAction>>,
}

impl OmpModelSelector {
    pub fn new(omp_binary: String, terminal_view_id: EntityId, ctx: &mut ViewContext<Self>) -> Self {
        let button = ctx.add_typed_action_view(|ctx| {
            ActionButton::new("omp", OmpButtonTheme).with_icon(Icon::OhMyPiLogo)
                .with_size(ButtonSize::AgentInputButton).with_tooltip("Select omp model")
                .with_tooltip_alignment(TooltipAlignment::Left)
                .on_click(|ctx| ctx.dispatch_typed_action(OmpModelSelectorAction::ToggleMenu))
        });
        let dropdown = ctx.add_typed_action_view(|_ctx| {
            Menu::<OmpModelSelectorAction>::new().with_ignore_hover_when_covered()
                .with_safe_triangle().prevent_interaction_with_other_elements()
                .with_drop_shadow().with_menu_variant(MenuVariant::scrollable())
        });
        ctx.subscribe_to_view(&dropdown, |me, _, event, ctx| {
            if matches!(event, MenuEvent::Close { .. }) {
                me.menu_is_open = false;
                ctx.emit(OmpModelSelectorEvent::MenuClosed);
                ctx.notify();
            }
        });
        ctx.spawn(warpui::r#async::Timer::after(Duration::ZERO), |me, _, ctx| {
            if me.selected_model.is_none() { me.trigger_refresh(ctx); }
        });
        Self { terminal_view_id, registry: OmpModelRegistry::new(&omp_binary), omp_binary,
            selected_model: None, loading: false, menu_is_open: false, button, dropdown }
    }

    fn trigger_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if self.loading { return; }
        self.loading = true;
        let omp_binary = self.omp_binary.clone();
        ctx.spawn(async move {
            let mut registry = OmpModelRegistry::new(&omp_binary);
            let r = registry.refresh().await;
            let model = if r.is_ok() { registry.read_current_model().await.ok() } else { None };
            (r, model, registry)
        }, |me, (r, model, registry), ctx| {
            me.registry = registry; me.loading = false;
            if let Ok(()) = r {
                if me.selected_model.is_none() { me.selected_model = model; }
                let items: Vec<MenuItem<OmpModelSelectorAction>> = me.registry.models().iter()
                    .map(|m| MenuItem::Item(MenuItemFields::new(format!("{} ({})", m.name, m.provider))
                        .with_on_select_action(OmpModelSelectorAction::SelectModel { selector: m.selector.clone() })))
                    .collect();
                let selected = me.selected_model.as_deref()
                    .and_then(|s| me.registry.models().iter().position(|m| m.selector == s));
                me.dropdown.update(ctx, |menu, ctx| {
                    menu.set_width(MENU_WIDTH); menu.set_height(400.);
                    menu.set_items(items, ctx);
                    if let Some(idx) = selected { menu.set_selected_by_index(idx, ctx); }
                    ctx.notify();
                });
            } else if let Err(e) = r {
                log::warn!("OmpModelSelector: refresh failed: {e}");
            }
            ctx.notify();
        });
    }

    fn display_label(&self) -> String {
        let Some(ref selector) = self.selected_model else { return "Select model".into(); };
        if selector.is_empty() { return "No model selected".into(); }
        self.registry.models().iter().find(|m| m.selector == *selector)
            .map(|m| m.name.clone()).unwrap_or_else(|| format!("Unknown ({selector})"))
    }
}

impl Entity for OmpModelSelector {
    type Event = OmpModelSelectorEvent;
}

impl View for OmpModelSelector {
    fn ui_name() -> &'static str { "OmpModelSelector" }
    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let theme = appearance.theme();
        let font_family = appearance.ui_font_family();
        let label: String = if self.loading { "Loading...".into() } else { self.display_label() };
        let button_row = Flex::row().with_spacing(4.).with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(ChildView::new(&self.button).finish())
            .with_child(Text::new(label, font_family, LABEL_FONT_SIZE)
                .with_color(internal_colors::neutral_7(theme)).finish()).finish();
        let content = Container::new(button_row)
            .with_vertical_padding(2.).with_padding_left(8.).with_padding_right(8.).finish();
        if !self.menu_is_open { return content; }
        let saved = SavePosition::new(content, "omp_selector_btn").finish();
        let mut stack = Stack::new();
        stack.add_child(saved);
        stack.add_positioned_overlay_child(
            ChildView::new(&self.dropdown).finish(),
            OffsetPositioning::offset_from_save_position_element(
                "omp_selector_btn", vec2f(0., -4.),
                PositionedElementOffsetBounds::WindowByPosition,
                PositionedElementAnchor::TopLeft, ChildAnchor::BottomLeft,
            ),
        );
        stack.finish()
    }
}

impl TypedActionView for OmpModelSelector {
    type Action = OmpModelSelectorAction;
    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            OmpModelSelectorAction::ToggleMenu => {
                let was_open = self.menu_is_open;
                self.menu_is_open = !self.menu_is_open;
                if self.menu_is_open && !was_open {
                    if !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id) {
                        ctx.dispatch_typed_action(&AgentInputFooterAction::ToggleRichInput);
                    }
                    if self.registry.models().is_empty() { self.trigger_refresh(ctx); }
                    ctx.focus(&self.dropdown);
                    ctx.emit(OmpModelSelectorEvent::MenuOpened);
                }
                ctx.notify();
            }
            OmpModelSelectorAction::SelectModel { selector } => {
                let sel = selector.clone();
                self.selected_model = Some(sel.clone());
                self.menu_is_open = false;
                ctx.emit(OmpModelSelectorEvent::ModelSelected { selector: sel.clone() });
                let omp_binary = self.omp_binary.clone();
                ctx.spawn(async move {
                    notify_running_omp(&sel).await;
                    let registry = OmpModelRegistry::new(&omp_binary);
                    registry.set_model(&sel).await
                }, |_me, result, ctx| {
                    if let Err(e) = result { log::warn!("OmpModelSelector: set_model failed: {e}"); }
                    ctx.notify();
                });
                ctx.notify();
            }
        }
    }
}

#[cfg(unix)]
async fn notify_running_omp(selector: &str) {
    let Some(home) = dirs::home_dir() else { return };
    let path = home.join(".omp/agent/model-switch.sock");
    if !path.exists() { return }
    let msg = serde_json::json!({"model": selector}).to_string() + "\n";
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::Write;
        let Ok(mut stream) = UnixStream::connect(&path) else { return Err("connect failed".into()) };
        stream.write_all(msg.as_bytes()).map_err(|e| format!("{e}"))
    }).await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => log::debug!("OmpModelSelector: socket notify: {e}"),
        Err(e) => log::debug!("OmpModelSelector: spawn_blocking: {e}"),
    }
}

#[cfg(not(unix))]
async fn notify_running_omp(_selector: &str) {}
