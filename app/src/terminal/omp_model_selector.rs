use std::time::Duration;

#[cfg(unix)]
use std::os::unix::net::UnixStream;


use pathfinder_geometry::vector::vec2f;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warpui::elements::{
    Border, ChildAnchor, ChildView, ConstrainedBox, Container, CrossAxisAlignment, Flex,
    MainAxisSize, OffsetPositioning, ParentElement as _, PositionedElementAnchor,
    PositionedElementOffsetBounds, SavePosition, Shrinkable, Stack, Text,
};
use warpui::{AppContext, Element, Entity, EntityId, SingletonEntity, TypedActionView, View,
    ViewContext, ViewHandle};
use crate::editor::{
    EditorView, Event as EditorEvent, PropagateAndNoOpNavigationKeys, SingleLineEditorOptions,
    TextOptions,
};
use warp_editor::editor::NavigationKey;
use crate::menu::{Event as MenuEvent, Menu, MenuItem, MenuItemFields, MenuVariant};
use crate::terminal::cli_agent_sessions::{CLIAgentSessionsModel, CLIAgentSessionsModelEvent};
use crate::terminal::omp_models::OmpModelRegistry;
use crate::ai::blocklist::agent_view::agent_input_footer::AgentInputFooterAction;
use crate::ui_components::icons::Icon;
use crate::view_components::action_button::{
    ActionButton, ActionButtonTheme, ButtonSize, TooltipAlignment,
};

/// OMP 模型切换扩展源码，编译进二进制。
/// 安装到 ~/.omp/agent/extensions/switch-model.ts 供 OMP agent 加载。
const OMP_SWITCH_EXTENSION_SRC: &str = include_str!("../../resources/omp/switch-model.ts");

/// 目标安装路径（相对于 home 目录）。
/// 写入 switch-model.ts 供 OMP agent 的扩展加载器发现。
const OMP_EXTENSION_REL_PATH: &str = ".omp/agent/extensions/switch-model.ts";

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
    filter_query: String,
    button: ViewHandle<ActionButton>,
    filter_editor: ViewHandle<EditorView>,
    dropdown: ViewHandle<Menu<OmpModelSelectorAction>>,
    /// 模型切换失败时显示的临时错误信息。
    switch_error: Option<String>,
    /// 递增计数器，用于检测快速连选时的竞态条件。
    switch_seq: u64,
    /// 模型加载完成后自动打开菜单（在扩展安装场景下使用）。
    open_on_load: bool,
    /// `~/.omp/agent/extensions/switch-model.ts` 是否存在。
    /// 该扩展负责接收 Zap 的 socket 消息执行模型切换。
    extension_ok: bool,
}
impl OmpModelSelector {
    pub fn new(omp_binary: String, terminal_view_id: EntityId, ctx: &mut ViewContext<Self>) -> Self {
        let button = ctx.add_typed_action_view(|_ctx| {
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
        let filter_editor = ctx.add_typed_action_view(|ctx| {
            let appearance = Appearance::as_ref(ctx);
            let mut editor = EditorView::single_line(
                SingleLineEditorOptions {
                    text: TextOptions::ui_text(Some(appearance.ui_font_size()), appearance),
                    propagate_and_no_op_vertical_navigation_keys:
                        PropagateAndNoOpNavigationKeys::Always,
                    ..Default::default()
                },
                ctx,
            );
            editor.set_placeholder_text(crate::t_static!("common-search"), ctx);
            editor
        });
        ctx.subscribe_to_view(&filter_editor, |me, _, event, ctx| {
            me.handle_filter_editor_event(event, ctx);
        });
        // 搜索条放进菜单 pinned header,继承菜单背景(surface_2),避免透明悬浮在终端内容上。
        let search_editor = filter_editor.clone();
        dropdown.update(ctx, |menu, _ctx| {
            menu.set_pinned_header_builder(move |app| render_search_header(&search_editor, app));
        });
        ctx.subscribe_to_model(&CLIAgentSessionsModel::handle(ctx), |me, _, event, ctx| {
            if let CLIAgentSessionsModelEvent::ModelChanged { terminal_view_id, model, .. } = event {
                if *terminal_view_id == me.terminal_view_id {
                    me.selected_model = Some(model.clone());
                    me.rebuild_menu_items(ctx);
                    ctx.notify();
                }
            }
        });
        ctx.spawn(warpui::r#async::Timer::after(Duration::ZERO), |me, _, ctx| {
            if me.selected_model.is_none() { me.trigger_refresh(ctx); }
        });
        Self { terminal_view_id, registry: OmpModelRegistry::new(&omp_binary), omp_binary,
            selected_model: None, loading: false, menu_is_open: false, filter_query: String::new(),
            button, filter_editor, dropdown, switch_error: None, switch_seq: 0,
            open_on_load: false, extension_ok: omp_switch_extension_exists(),
        }
    }

    fn trigger_refresh(&mut self, ctx: &mut ViewContext<Self>) {
        if self.loading { return; }
        if !self.extension_ok {
            log::info!("OmpModelSelector: switch-model.ts not found, skipping refresh");
            return;
        }
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
                log::info!("OmpModelSelector: refresh success, {} models loaded", me.registry.models().len());
                if me.selected_model.is_none() { me.selected_model = model; }
                if me.open_on_load {
                    me.open_on_load = false;
                    me.menu_is_open = true;
                    me.rebuild_menu_items(ctx);
                    ctx.focus(&me.filter_editor);
                } else {
                    me.rebuild_menu_items(ctx);
                }
            } else if let Err(e) = r {
                log::warn!("OmpModelSelector: refresh failed: {e}");
                me.open_on_load = false;
            }
            ctx.notify();
        });
    }
    fn rebuild_menu_items(&mut self, ctx: &mut ViewContext<Self>) {
        let query = self.filter_query.to_lowercase();
        let filtered: Vec<_> = self.registry.models().iter()
            .filter(|m| {
                query.is_empty()
                    || m.name.to_lowercase().contains(&query)
                    || m.provider.to_lowercase().contains(&query)
            })
            .cloned()
            .collect();

        let items: Vec<MenuItem<OmpModelSelectorAction>> = filtered.iter()
            .map(|m| {
                    MenuItem::Item(
                        MenuItemFields::new(format!("{} ({})", m.name, m.provider))
                            .with_font_size_override(14.)
                            .no_highlight_on_hover()
                            .with_on_select_action(OmpModelSelectorAction::SelectModel {
                                selector: m.selector.clone(),
                            }),
                    )
                })
                .collect();
        let selected = self.selected_model.as_deref()
            .and_then(|s| filtered.iter().position(|m| m.selector == s));
        self.dropdown.update(ctx, |menu, ctx| {
            menu.set_width(MENU_WIDTH); menu.set_height(400.);
            menu.set_items(items, ctx);
            if let Some(idx) = selected { menu.set_selected_by_index(idx, ctx); }
            ctx.notify();
        });
    }

    fn handle_filter_editor_event(&mut self, event: &EditorEvent, ctx: &mut ViewContext<Self>) {
        match event {
            EditorEvent::Edited(_) => {
                self.filter_query = self.filter_editor.as_ref(ctx).buffer_text(ctx);
                self.rebuild_menu_items(ctx);
                ctx.notify();
            }
            EditorEvent::Escape => {
                self.menu_is_open = false;
                ctx.emit(OmpModelSelectorEvent::MenuClosed);
                ctx.notify();
            }
            EditorEvent::Enter => {
                let current_selected = self.dropdown.read(ctx, |dropdown, _| dropdown.selected_item());
                if let Some(MenuItem::Item(fields)) = current_selected {
                    if let Some(action) = fields.on_select_action() {
                        self.handle_action(action, ctx);
                    }
                }
            }
            EditorEvent::Navigate(NavigationKey::Up) => {
                self.dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.select_previous(ctx);
                });
                ctx.notify();
            }
            EditorEvent::Navigate(NavigationKey::Down) => {
                self.dropdown.update(ctx, |dropdown, ctx| {
                    dropdown.select_next(ctx);
                });
                ctx.notify();
            }
            _ => {}
        }
    }

    fn display_label(&self) -> String {
        if !self.extension_ok && !self.loading {
            return crate::t!("omp-model-selector-not-installed");
        }
        if let Some(err) = &self.switch_error {
            return format!("✗ {err}");
        }
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
        // 搜索条作为 pinned header 由 Menu 内部渲染(继承菜单背景)。
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
                    // 扩展缺失时：自动安装 → loading（按钮变 Loading…）→ 关闭菜单（等模型加载完再弹）。
                    if !self.extension_ok && !self.loading {
                        if let Some(home) = dirs::home_dir() {
                            let path = home.join(OMP_EXTENSION_REL_PATH);
                            if let Some(parent) = path.parent() {
                                let _ = std::fs::create_dir_all(parent);
                            }
                            if std::fs::write(&path, OMP_SWITCH_EXTENSION_SRC).is_ok() {
                                log::info!("Auto-installed OMP switch-model.ts to {path:?}");
                                self.extension_ok = true;
                            } else {
                                log::warn!("Failed to auto-install OMP switch-model.ts");
                            }
                        }
                        if self.extension_ok {
                            self.menu_is_open = false;
                            self.open_on_load = true;
                            self.trigger_refresh(ctx);
                        }
                        ctx.notify();
                        return;
                    }
                    self.switch_error = None;
                    if !CLIAgentSessionsModel::as_ref(ctx).is_input_open(self.terminal_view_id) {
                        ctx.dispatch_typed_action(&AgentInputFooterAction::ToggleRichInput);
                    }
                    if self.registry.models().is_empty() {
                        if self.loading {
                            // 模型仍在加载中，等 refresh callback 完成后弹菜单。
                            self.menu_is_open = false;
                            self.open_on_load = true;
                            ctx.notify();
                            return;
                        }
                        self.trigger_refresh(ctx);
                    }
                    self.filter_query.clear();
                    self.filter_editor.update(ctx, |editor, ctx| {
                        editor.clear_buffer(ctx);
                        ctx.notify();
                    });
                    self.rebuild_menu_items(ctx);
                    ctx.focus(&self.filter_editor);
                    ctx.emit(OmpModelSelectorEvent::MenuOpened);
                }
                ctx.notify();
            }
            OmpModelSelectorAction::SelectModel { selector } => {
                let sel = selector.clone();
                let old_selection = self.selected_model.clone();
                self.selected_model = Some(sel.clone());
                self.menu_is_open = false;
                self.switch_error = None;
                self.switch_seq += 1;
                let seq = self.switch_seq;
                ctx.emit(OmpModelSelectorEvent::ModelSelected { selector: sel.clone() });
                // 通过该 session 专属的 socket 通知运行中的 omp 进行进程内切换（不回显、按 session 定向）。
                let session_id = self.current_socket_id(ctx);
                if let Some(session_id) = session_id {
                    self.send_notification(sel, session_id, old_selection, seq, ctx);
                } else {
                    // 多开/刚启动时 session_start 事件可能晚于用户操作，轮询等待 session_id 就绪。
                    self.notify_when_ready(sel, old_selection, seq, 12, ctx);
                }
                ctx.notify();
            }
        }
    }
}

impl OmpModelSelector {
    /// 取当前会话用于 socket 通知的 id：优先扩展上报的 model_switch_socket_id
    /// （该 id 一定对应存在的 socket），缺失时才回退到 OSC777 session_id
    /// （omp 恢复旧会话时 OSC777 的 session_id 可能没有对应 socket）。
    fn current_socket_id(&self, ctx: &AppContext) -> Option<String> {
        CLIAgentSessionsModel::as_ref(ctx)
            .session(self.terminal_view_id)
            .and_then(|s| {
                s.session_context
                    .model_switch_socket_id
                    .clone()
                    .or_else(|| s.session_context.session_id.clone())
            })
    }


    /// 通过 session 专属 socket 发送模型切换通知。
    fn send_notification(&mut self, selector: String, session_id: String, old_selection: Option<String>, seq: u64, ctx: &mut ViewContext<Self>) {
        ctx.spawn(async move {
            let result = notify_running_omp(&selector, Some(&session_id)).await;
            (result, old_selection, seq)
        }, |me, (result, old_selection, seq), ctx| {
            me.finish_notify(result, old_selection, seq, ctx);
        });
    }

    /// session_id 未就绪时的轮询通知：每 250ms 重取一次，最多 attempts_left 次（约 3 秒）。
    /// 优先使用扩展上报的 model_switch_socket_id（该 id 一定对应存在的 socket），
    fn notify_when_ready(&mut self, selector: String, old_selection: Option<String>, seq: u64, attempts_left: u32, ctx: &mut ViewContext<Self>) {
        let session_id = self.current_socket_id(ctx);
        if let Some(session_id) = session_id {
            self.send_notification(selector, session_id, old_selection, seq, ctx);
        } else if attempts_left > 0 {
            ctx.spawn(
                warpui::r#async::Timer::after(Duration::from_millis(250)),
                move |me, _, ctx| {
                    // 期间用户又切换了模型，放弃过期轮询。
                    if seq != me.switch_seq { return; }
                    me.notify_when_ready(selector, old_selection, seq, attempts_left - 1, ctx);
                },
            );
        } else {
            self.finish_notify(
                Err("omp session not ready yet, try again".into()),
                old_selection, seq, ctx,
            );
        }
    }

    /// socket 通知结果回调：seq 过期则忽略；失败恢复旧选择并显示临时错误（3 秒后清除）。
    fn finish_notify(&mut self, result: Result<(), String>, old_selection: Option<String>, seq: u64, ctx: &mut ViewContext<Self>) {
        // 如果 seq 不匹配，说明有更新的切换已发起，忽略过期的回调
        if seq != self.switch_seq { return; }
        if let Err(e) = result {
            log::warn!("OmpModelSelector: switch model failed: {e}");
            self.selected_model = old_selection;
            self.switch_error = Some(e);
            self.rebuild_menu_items(ctx);
            // 3 秒后自动清除错误提示
            ctx.spawn(
                warpui::r#async::Timer::after(Duration::from_secs(3)),
                |me, _, ctx| {
                    me.switch_error = None;
                    ctx.notify();
                },
            );
        }
        ctx.notify();
    }
}

/// OMP 模型切换扩展源码中的唯一标记，用于检测是否已安装。
/// v2 起包含 socket 就绪上报，旧版扩展检测不到会触发重新安装。
const OMP_EXTENSION_MARKER: &str = "zap-omp-switch-model-v2";

/// 检查 `~/.omp/agent/extensions/` 下是否有包含标记的扩展文件。
/// 读每个 .ts 文件头几字节匹配标记，避免大文件误读。
fn omp_switch_extension_exists() -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    let dir = home.join(".omp/agent/extensions");
    let Ok(entries) = std::fs::read_dir(&dir) else { return false };
    let mut buf = [0u8; 64];
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("ts") {
            continue;
        }
        if let Ok(mut f) = std::fs::File::open(&path) {
            use std::io::Read;
            let n = f.read(&mut buf).unwrap_or(0);
            if n > 0 && buf[..n].windows(OMP_EXTENSION_MARKER.len()).any(|w| w == OMP_EXTENSION_MARKER.as_bytes()) {
                return true;
            }
        }
    }
    false
}

#[cfg(unix)]
async fn notify_running_omp(selector: &str, session_id: Option<&str>) -> Result<(), String> {
    let Some(home) = dirs::home_dir() else { return Err("no home directory".into()) };
    // 优先连该 session 专属 socket；缺 session_id 时回退到全局 socket（兼容旧扩展）。
    let path = match session_id {
        Some(id) => home.join(format!(".omp/agent/model-switch-{id}.sock")),
        None => home.join(".omp/agent/model-switch.sock"),
    };
    // omp 启动后 socket 文件创建有时序，短暂重试后再报错。
    let mut attempts = 0;
    while !path.exists() && attempts < 5 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        attempts += 1;
    }
    if !path.exists() { return Err("socket not found (omp not running?)".into()) }
    let msg = serde_json::json!({"model": selector}).to_string() + "\n";
    let result = tokio::task::spawn_blocking(move || -> Result<(), String> {
        use std::io::Write;
        let mut stream = UnixStream::connect(&path).map_err(|e| format!("connect: {e}"))?;
        stream.write_all(msg.as_bytes()).map_err(|e| format!("write: {e}"))
    }).await;
    result.map_err(|e| format!("spawn_blocking: {e}"))?
}

#[cfg(not(unix))]
async fn notify_running_omp(_selector: &str, _session_id: Option<&str>) -> Result<(), String> {
    Err("socket notify not supported on this platform".into())
}

/// 渲染菜单顶部的搜索 header（pinned header builder 调用），继承菜单背景，
/// 与模型列表视觉连成一体，避免搜索条透明悬浮在终端内容上。
fn render_search_header(editor: &ViewHandle<EditorView>, app: &AppContext) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let search_icon = ConstrainedBox::new(
        Icon::SearchSmall
            .to_warpui_icon(theme.sub_text_color(theme.surface_2()))
            .finish(),
    )
    .with_width(16.)
    .with_height(16.)
    .finish();
    let search_row = Flex::row()
        .with_child(Container::new(search_icon).with_margin_right(8.).finish())
        .with_child(Shrinkable::new(1., ChildView::new(editor).finish()).finish())
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .finish();
    Container::new(search_row)
        .with_padding_left(8.)
        .with_padding_right(8.)
        .with_padding_top(6.)
        .with_padding_bottom(6.)
        .with_border(Border::bottom(1.).with_border_fill(theme.surface_3()))
        .finish()
}

#[cfg(test)]
#[path = "omp_model_selector_tests.rs"]
mod tests;
