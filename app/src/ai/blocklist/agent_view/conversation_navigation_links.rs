use pathfinder_color::ColorU;
use pathfinder_geometry::vector::Vector2F;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::Fill;
use warpui::{
    elements::{
        ConstrainedBox, Container, CornerRadius, CrossAxisAlignment, Empty, Expanded, Flex,
        Hoverable, MainAxisSize, MouseStateHandle, ParentElement, Radius, Shrinkable, Text,
    },
    fonts::{Properties, Weight::Bold},
    platform::Cursor,
    text_layout::ClipConfig,
    AppContext, Element, EventContext, SingletonEntity,
};

use crate::ui_components::{blended_colors, icons::Icon};

use crate::ai::agent::conversation::{AIConversation, AIConversationId};
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::terminal::view::TerminalAction;
use crate::workspace::{WorkspaceAction, WorkspaceRegistry};
use warpui::EntityId;

// ---------------------------------------------------------------------------
// 会话导航判定(自上游 `orchestration_conversation_links.rs` 合并)
// ---------------------------------------------------------------------------

/// 该会话是否已在**其它可见** pane 中打开。隐藏的 child-agent pane 会被排除,
/// 否则「还没点开过的子代理」会被误判成「已经打开」。
pub(crate) fn is_conversation_open_in_other_visible_view(
    conversation_id: AIConversationId,
    self_terminal_view_id: EntityId,
    app: &AppContext,
) -> bool {
    let Some(owner) =
        BlocklistAIHistoryModel::as_ref(app).terminal_view_id_for_conversation(&conversation_id)
    else {
        return false;
    };
    if owner == self_terminal_view_id {
        return false;
    }
    pane_group_id_containing_terminal_view(owner, app).is_some()
}

/// 在所有可见 pane / tab 中查找承载该 terminal view 的 pane group。
/// 用于区分「同 tab 聚焦」与「跨 tab 激活」。
pub(crate) fn pane_group_id_containing_terminal_view(
    terminal_view_id: EntityId,
    app: &AppContext,
) -> Option<EntityId> {
    let registry = WorkspaceRegistry::as_ref(app);
    for (_, workspace_handle) in registry.all_workspaces(app) {
        let workspace = workspace_handle.as_ref(app);
        for pane_group_handle in workspace.tab_views() {
            let pane_group = pane_group_handle.as_ref(app);
            for pane_id in pane_group.visible_pane_ids() {
                if let Some(terminal_view) = pane_group.terminal_view_from_pane_id(pane_id, app) {
                    if terminal_view.id() == terminal_view_id {
                        return Some(pane_group_handle.id());
                    }
                }
            }
        }
    }
    None
}

/// 导航到子代理的 pane:同 pane group 内复用已有隐藏 pane,跨 pane group 时激活
/// 其所属 tab,都没有才新开一个 pane。
///
/// **本地偏差(阶段二方案 A)**:上游在没有所属 view 时 dispatch
/// `TerminalAction::OpenChildAgentInNewPane`,该动作的 handler 依赖上游
/// `pane_group/child_agent/`(本地不存在)。这里改为
/// `TerminalAction::RevealChildAgent` —— 复用本仓既有的隐藏 pane reveal 链路,
/// 效果是「显示并聚焦该子会话」。新开 tab 的语义(上游 `InNewTab` 同样是 stub)
/// 本期不交付。
pub(crate) fn dispatch_focus_or_open_child_agent_pane(
    conversation_id: AIConversationId,
    self_terminal_view_id: EntityId,
    ctx: &mut EventContext,
    app: &AppContext,
) {
    let owner_view_id =
        BlocklistAIHistoryModel::as_ref(app).terminal_view_id_for_conversation(&conversation_id);
    if let Some(owner_view_id) = owner_view_id {
        if owner_view_id != self_terminal_view_id {
            if let Some(owner_pane_group_id) =
                pane_group_id_containing_terminal_view(owner_view_id, app)
            {
                let self_pane_group_id =
                    pane_group_id_containing_terminal_view(self_terminal_view_id, app);
                if Some(owner_pane_group_id) == self_pane_group_id {
                    ctx.dispatch_typed_action(TerminalAction::RevealChildAgent { conversation_id });
                } else {
                    ctx.dispatch_typed_action(WorkspaceAction::FocusTerminalViewInWorkspace {
                        terminal_view_id: owner_view_id,
                    });
                }
                return;
            }
        }
    }
    ctx.dispatch_typed_action(TerminalAction::RevealChildAgent { conversation_id });
}

/// 会话的规范父会话 id(薄包装,判定收敛在 history model)。
pub(crate) fn parent_conversation_id(
    active_conversation: &AIConversation,
    app: &AppContext,
) -> Option<AIConversationId> {
    BlocklistAIHistoryModel::as_ref(app)
        .resolved_parent_conversation_id_for_conversation(active_conversation)
}

/// Renders a clickable card with an optional leading icon, title/subtitle,
/// a trailing chevron, and an optional extra trailing element (e.g. a dismiss
/// button). When `extra_trailing` is provided, the card's Hoverable uses
/// `defer_events_to_children` so the trailing element can handle its own
/// click without also triggering the card's `on_click`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn conversation_navigation_card_with_icon(
    icon: Option<(Icon, ColorU)>,
    title: String,
    subtitle: Option<String>,
    on_click: impl FnMut(&mut EventContext, &AppContext, Vector2F) + 'static,
    mouse_state: MouseStateHandle,
    expands_to_max_width: bool,
    extra_trailing: Option<Box<dyn Element>>,
    app: &AppContext,
) -> Box<dyn Element> {
    let appearance = Appearance::as_ref(app);
    let theme = appearance.theme();
    let has_extra_trailing = extra_trailing.is_some();

    let mut hoverable = Hoverable::new(mouse_state, move |hover_state| {
        let background = if hover_state.is_hovered() {
            blended_colors::fg_overlay_2(theme)
        } else {
            blended_colors::fg_overlay_1(theme)
        };

        let mut text_column = Flex::column().with_child(
            Text::new(
                title.clone(),
                appearance.ui_font_family(),
                appearance.monospace_font_size(),
            )
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .with_style(Properties {
                weight: Bold,
                ..Default::default()
            })
            .with_color(blended_colors::text_main(
                theme,
                appearance.theme().background(),
            ))
            .finish(),
        );
        if let Some(subtitle) = subtitle.as_ref() {
            text_column.add_child(
                Text::new(
                    subtitle.clone(),
                    appearance.ui_font_family(),
                    (appearance.monospace_font_size() - 2.).max(10.),
                )
                .soft_wrap(false)
                .with_clip(ClipConfig::ellipsis())
                .with_color(blended_colors::text_sub(
                    theme,
                    appearance.theme().background(),
                ))
                .finish(),
            );
        }
        let text_column = text_column.finish();

        let mut row = Flex::row().with_cross_axis_alignment(CrossAxisAlignment::Center);
        if let Some((icon, color)) = icon {
            row.add_child(
                Container::new(
                    ConstrainedBox::new(icon.to_warpui_icon(Fill::Solid(color)).finish())
                        .with_width(16.)
                        .with_height(16.)
                        .finish(),
                )
                .with_margin_right(6.)
                .finish(),
            );
        }
        if expands_to_max_width {
            row = row
                .with_main_axis_size(MainAxisSize::Max)
                .with_child(Shrinkable::new(1., text_column).finish());
        } else {
            row = row.with_child(text_column);
        }
        row.add_child(
            Container::new(
                ConstrainedBox::new(
                    Icon::ChevronRight
                        .to_warpui_icon(blended_colors::text_sub(theme, theme.background()).into())
                        .finish(),
                )
                .with_height(20.)
                .with_width(20.)
                .finish(),
            )
            .with_margin_left(8.)
            .finish(),
        );
        if let Some(trailing) = extra_trailing {
            // Spacer pushes the trailing element to the far right edge.
            row.add_child(Expanded::new(1., Empty::new().finish()).finish());
            row.add_child(trailing);
        }
        let row = row.finish();

        Container::new(row)
            .with_background_color(background.into())
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_horizontal_padding(10.)
            .with_vertical_padding(8.)
            .finish()
    })
    .with_cursor(Cursor::PointingHand)
    .on_click(on_click);

    // When an extra trailing element is present (e.g. dismiss button), defer
    // click events to children so the trailing element's click handler takes
    // precedence over the card's navigation handler.
    if has_extra_trailing {
        hoverable = hoverable.with_defer_events_to_children();
    }

    hoverable.finish()
}
