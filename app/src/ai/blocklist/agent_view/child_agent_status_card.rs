use std::collections::HashMap;

use warpui::elements::{Element, Empty, Flex, MouseStateHandle, ParentElement};
use warpui::platform::Cursor;
use warpui::prelude::Container;
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Entity, ModelHandle, SingletonEntity, TypedActionView, View, ViewContext, WindowId,
};

use crate::ai::agent::conversation::{AIConversation, AIConversationId, ConversationStatus};
use crate::ai::agent::AIAgentOutputMessageType;
use crate::ai::blocklist::agent_view::conversation_navigation_links::conversation_navigation_card_with_icon;
use crate::ai::blocklist::agent_view::{AgentViewController, AgentViewControllerEvent};
use crate::ai::blocklist::BlocklistAIHistoryEvent;
use crate::appearance::Appearance;
use crate::terminal::view::TerminalAction;
use crate::ui_components::buttons::icon_button;
use crate::ui_components::icons::Icon;
use crate::BlocklistAIHistoryModel;

#[derive(Debug, Clone)]
pub enum ChildAgentStatusCardAction {
    /// 停止该子代理。子代理跑在独立会话里,父对话的取消不会连带取消它,所以这是
    /// 用户在父界面里停掉跑偏子代理的入口。
    StopAgent(AIConversationId),
}

/// 每次触发后额外复评可见性的次数。
///
/// 取 2 而不是 1:新回合被追加与本卡重绘**可能落在不同帧**(取决于事件在一帧内的
/// 处理阶段,以及消息流那张卡所在 block 何时被布局/绘制),只看一帧不能保证位置已经
/// 发布。这是**有限次补偿**,不是轮询 —— 复评完仍查不到位置,说明那一行确实不在可视区。
const VISIBILITY_RECHECKS: u8 = 2;

/// 有子代理运行中时,复评可见性的间隔。
///
/// **为什么需要定期复评,而不是只挂滚动事件**(实测故障「拖滚动条滚动时不触发隐藏」):
/// agent view 的可滚动区域外面包的是框架的 `NewScrollable`,用的是 **Manual** 滚动
/// 句柄(`terminal/view.rs` 的 `add_scrollable`)。拖滚动条时偏移由框架元素直接改写,
/// 只对所属 view 发一个通用的 `ctx.notify()`,**不经过** `TerminalView::scroll` /
/// `update_scroll_position_locking`。滚轮、PageUp/Down、键盘、自动滚动等来源也各不相同
/// —— 逐个挂钩必然漏掉其中一些。改为「有子代理在跑时按固定间隔复评」一次覆盖所有
/// 来源,且**有界**:没有运行中的子代理时这条链完全停止。
///
/// 滚轮另有一条低延迟快路径(元素层滚动处派发
/// `TerminalAction::SubagentCardVisibilityMayHaveChanged`),用于当帧响应;其余来源
/// 只能靠这里。两者**互补**,不是重复实现。
const VISIBILITY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Renders a list of child agent statuses above the agent message bar.
///
/// Each row shows a status icon, agent name, and conversation title.
/// Clicking a row reveals the child agent's hidden pane.
///
/// This card is a **fallback**, not the primary home of a subagent: the in-stream
/// card (rendered next to the exchange that dispatched it, see
/// `block/view_impl/output.rs`) is where a dispatch is normally read and stopped.
/// So a row only appears here when both hold:
///
/// 1. the child is still **running** (finished dispatches leave no row), and
/// 2. its in-stream row is **not currently on screen** (scrolled out of view).
///
/// Condition 2 is why this view reads `WindowId`: the in-stream rows publish a
/// single-frame position via `SavePosition`, so "no cached position" means "not
/// rendered this frame" — which, with the block list's virtualization, means
/// "scrolled away".
pub struct ChildAgentStatusCard {
    agent_view_controller: ModelHandle<AgentViewController>,
    /// Window this card renders in; needed to look up the in-stream row positions
    /// above. Kept up to date through [`View::on_window_transferred`].
    window_id: WindowId,
    mouse_states: HashMap<AIConversationId, MouseStateHandle>,
    stop_mouse_states: HashMap<AIConversationId, MouseStateHandle>,
    /// 还剩下几次「下一帧复评」。非 0 表示已有一条复评链在跑(用于合并同一帧内的
    /// 多次触发);归零后下一个触发才重新起链。
    rechecks_remaining: u8,
    /// 是否已排下一次定期复评(避免同时存在多条轮询链)。
    poll_scheduled: bool,
}

impl Entity for ChildAgentStatusCard {
    type Event = ();
}

impl ChildAgentStatusCard {
    pub fn new(
        agent_view_controller: ModelHandle<AgentViewController>,
        ctx: &mut ViewContext<Self>,
    ) -> Self {
        // Subscribe without terminal_view_id filtering so we receive status
        // updates for child conversations (which have a different terminal_view_id).
        let history_model = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_model, |this, _, event, ctx| match event {
            BlocklistAIHistoryEvent::UpdatedConversationStatus { .. } => {
                this.ensure_mouse_states(ctx);
                this.notify_and_recheck_next_frame(ctx);
                this.ensure_visibility_poll(ctx);
            }
            BlocklistAIHistoryEvent::AppendedExchange { .. }
            | BlocklistAIHistoryEvent::SetActiveConversation { .. } => {
                this.ensure_mouse_states(ctx);
                this.notify_and_recheck_next_frame(ctx);
                this.ensure_visibility_poll(ctx);
            }
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                this.remove_state_for_conversation(*conversation_id);
                ctx.notify();
            }
            _ => {}
        });
        ctx.subscribe_to_model(&agent_view_controller, |this, _, event, ctx| {
            // Reset all per-child state when entering a conversation so stale
            // entries from a previous conversation's children don't accumulate.
            if matches!(event, AgentViewControllerEvent::EnteredAgentView { .. }) {
                this.mouse_states.clear();
                this.stop_mouse_states.clear();
            }
            this.ensure_mouse_states(ctx);
            this.notify_and_recheck_next_frame(ctx);
            this.ensure_visibility_poll(ctx);
        });

        Self {
            agent_view_controller,
            window_id: ctx.window_id(),
            mouse_states: HashMap::new(),
            stop_mouse_states: HashMap::new(),
            rechecks_remaining: 0,
            poll_scheduled: false,
        }
    }

    /// 重绘自己,并额外安排 [`VISIBILITY_RECHECKS`] 次「下一帧复评」。
    ///
    /// **为什么必须复评**(实测故障「底部卡出现后要过一会才隐藏」):消息流里的子代理
    /// 行用 `SavePosition` 发布单帧位置,而位置快照在每帧 `build_scene` **之后**才写入
    /// (`core/app.rs`)。新派发的回合是**本帧**才追加的,所以本帧评估「那一行在不在
    /// 可视区」时位置**必然还查不到** —— 只评一次的话,底部卡会被误判成「那行不可见」
    /// 而闪现,并且要等下一次无关事件才消失。复评时位置已发布,闪现即被纠正。
    ///
    /// **有限次,不轮询**:复评额度用尽后仍查不到位置,说明那一行确实不在可视区
    /// (块列表虚拟化下没有被渲染),此时继续显示才是对的。
    fn notify_and_recheck_next_frame(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.notify();
        if self.rechecks_remaining == 0 {
            self.rechecks_remaining = VISIBILITY_RECHECKS;
            self.schedule_visibility_recheck(ctx);
        }
    }

    /// 排下一次复评:重绘自己,若额度未用完则继续排。
    fn schedule_visibility_recheck(&mut self, ctx: &mut ViewContext<Self>) {
        ctx.spawn(
            warpui::r#async::Timer::after(std::time::Duration::ZERO),
            |me, _, ctx| {
                me.rechecks_remaining = me.rechecks_remaining.saturating_sub(1);
                ctx.notify();
                if me.rechecks_remaining > 0 {
                    me.schedule_visibility_recheck(ctx);
                }
            },
        );
    }

    /// 有运行中的子代理时,按 [`VISIBILITY_POLL_INTERVAL`] 持续复评可见性。
    ///
    /// 见常量的说明:滚动来源太杂,逐个挂钩必然漏;定期复评一次覆盖所有来源。没有
    /// 运行中的子代理时不会起链,链也会在对应用户交互后自然停止。
    fn ensure_visibility_poll(&mut self, ctx: &mut ViewContext<Self>) {
        if self.poll_scheduled || !self.has_running_child(ctx) {
            return;
        }
        self.poll_scheduled = true;
        ctx.spawn(
            warpui::r#async::Timer::after(VISIBILITY_POLL_INTERVAL),
            |me, _, ctx| {
                me.poll_scheduled = false;
                ctx.notify();
                me.ensure_visibility_poll(ctx);
            },
        );
    }

    /// 当前激活会话下是否还有运行中的子代理。
    fn has_running_child(&self, app: &AppContext) -> bool {
        let Some(active) = self
            .agent_view_controller
            .as_ref(app)
            .agent_view_state()
            .active_conversation_id()
        else {
            return false;
        };
        BlocklistAIHistoryModel::as_ref(app)
            .child_conversations_of(active)
            .iter()
            .any(|child| {
                matches!(
                    child.status(),
                    ConversationStatus::InProgress | ConversationStatus::Blocked { .. }
                )
            })
    }

    fn ensure_mouse_states(&mut self, ctx: &AppContext) {
        let agent_view_controller = self.agent_view_controller.as_ref(ctx);
        let Some(active_conversation_id) = agent_view_controller
            .agent_view_state()
            .active_conversation_id()
        else {
            return;
        };
        let history_model = BlocklistAIHistoryModel::as_ref(ctx);
        for child in history_model.child_conversations_of(active_conversation_id) {
            let child_id = child.id();
            self.mouse_states.entry(child_id).or_default();
            self.stop_mouse_states.entry(child_id).or_default();
        }
    }

    fn remove_state_for_conversation(&mut self, conversation_id: AIConversationId) {
        self.mouse_states.remove(&conversation_id);
        self.stop_mouse_states.remove(&conversation_id);
    }
}

impl TypedActionView for ChildAgentStatusCard {
    type Action = ChildAgentStatusCardAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            ChildAgentStatusCardAction::StopAgent(conversation_id) => {
                ctx.dispatch_typed_action(&TerminalAction::StopChildAgentConversation {
                    conversation_id: *conversation_id,
                });
            }
        }
    }
}

impl View for ChildAgentStatusCard {
    fn ui_name() -> &'static str {
        "ChildAgentStatusCard"
    }

    /// 标签页被拖到另一个窗口时,本视图存的位置查询窗口必须跟着换 —— 否则可见性
    /// 判断会一直去旧窗口找位置,底部卡就再也不显示了。
    fn on_window_transferred(
        &mut self,
        _source_window_id: WindowId,
        target_window_id: WindowId,
        _ctx: &mut ViewContext<Self>,
    ) {
        self.window_id = target_window_id;
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let agent_view_controller = self.agent_view_controller.as_ref(app);
        let Some(active_conversation_id) = agent_view_controller
            .agent_view_state()
            .active_conversation_id()
        else {
            return Empty::new().finish();
        };

        let history_model = BlocklistAIHistoryModel::as_ref(app);
        let mut children = history_model.child_conversations_of(active_conversation_id);
        if children.is_empty() {
            return Empty::new().finish();
        }

        // 只保留**仍在运行**的子代理,并且其消息流里那一行**当前不可见**。
        //
        // 可见性判据:消息流的行用 `SavePosition::for_single_frame` 发布位置,而单帧
        // 位置在每帧起始被清空(`presenter::PositionCache::clear_single_frame_positions`),
        // 块列表又按可视范围虚拟化 —— 所以「查不到位置」= 这一帧没渲染 = 滚出可视区。
        // 消息流里看得见时就没必要在这里重复一行。
        children.retain(|child| {
            if !matches!(
                child.status(),
                ConversationStatus::InProgress | ConversationStatus::Blocked { .. }
            ) {
                return false;
            }
            let position_id = crate::ai::agent_providers::tools::subagent::row_position_id(
                &child.id().to_string(),
            );
            app.element_position_by_id_at_last_frame(self.window_id, &position_id)
                .is_none()
        });
        if children.is_empty() {
            return Empty::new().finish();
        }

        // Sort by creation time so rows have a stable visual order.
        children.sort_by_key(|c| c.first_exchange().map(|e| e.start_time));

        let appearance = Appearance::as_ref(app);
        let mut column = Flex::column();

        for child in &children {
            let conversation_id = child.id();

            let agent_name = child
                .agent_name()
                .map(str::to_string)
                .unwrap_or_else(|| crate::t!("child-agent-default-name"));
            let raw_title = child
                .title()
                .unwrap_or_else(|| crate::t!("common-untitled"));
            let status_icon = child.status().status_icon_and_color(appearance.theme());

            // T3-7:in_progress 子代理在 title 里 surface 当前 action,
            // "Untitled · ↳ Searching codebase..." / "Refactor X · ↳ Reading files..."。
            // finished 子代理仍显示原 title 不变。
            //
            // Zap:子代理**等待审批**时必须单独说清楚。子代理的审批按钮渲染在它
            // 自己的会话里(`AIBlock::conversation_id` 把 block 绑定到唯一会话,
            // `blocked_action` 也只读该会话的 output),父界面结构上看不到那个按钮,
            // 所以这里必须明确告诉用户「在等确认」以及「点这一行可以进去处理」,
            // 否则用户只看到一个黄点而不知道该做什么。
            //
            // 注意 `Blocked` **不是** `is_in_progress()`(见 `ConversationStatus`
            // 的两个独立判定),所以它需要单独一个分支。
            let title = match child.status() {
                ConversationStatus::Blocked { blocked_action } => {
                    let action = blocked_action.trim();
                    if action.is_empty() {
                        format!(
                            "{raw_title} · {}",
                            crate::t!("child-agent-awaiting-approval")
                        )
                    } else {
                        // `blocked_action` 已由 `localized_action_name` 生成为本地化
                        // 纯文本(源不再包调试引号),这里直接展示。
                        format!("{raw_title} · ↳ {action}")
                    }
                }
                _ if child.status().is_in_progress() => {
                    if let Some(presence) = latest_action_presence(child) {
                        format!("{raw_title} · ↳ {presence}...")
                    } else {
                        raw_title
                    }
                }
                _ => raw_title,
            };

            let Some(mouse_state) = self.mouse_states.get(&conversation_id).cloned() else {
                log::error!(
                    "Missing mouse state handle for child agent card {:?}",
                    conversation_id
                );
                continue;
            };

            // 「停止」入口只在子代理**还活着**时提供:已结束的会话没有可停的东西,
            // 而且 `cancel_conversation_progress` 在无 in-flight stream 时会走
            // `cancel_all_pending_actions` 分支 —— 对已完成的会话调用语义不对。
            //
            // 未运行时仍然放一个空占位:渲染器只在 `extra_trailing` 为 `Some` 时才
            // 插入 `Expanded` 占位(`conversation_navigation_card_with_icon`),少了它
            // 各行的 chevron 位置会不一致。
            // 本卡只显示**运行中**的子代理(见上面的 retain),所以这里必定给出停止
            // 入口 —— 不需要「已完成时留空占位」的分支。
            let Some(stop_mouse_state) = self.stop_mouse_states.get(&conversation_id).cloned()
            else {
                log::error!(
                    "Missing stop mouse state handle for child agent card {:?}",
                    conversation_id
                );
                continue;
            };
            let stop_tooltip = appearance.ui_builder().clone();
            let trailing: Box<dyn Element> =
                icon_button(appearance, Icon::StopFilled, false, stop_mouse_state)
                    .with_tooltip(move || {
                        stop_tooltip
                            .tool_tip(crate::t!("child-agent-stop"))
                            .build()
                            .finish()
                    })
                    .build()
                    .on_click(move |ctx: &mut warpui::EventContext<'_>, _, _| {
                        ctx.dispatch_typed_action(ChildAgentStatusCardAction::StopAgent(
                            conversation_id,
                        ));
                    })
                    .with_cursor(Cursor::PointingHand)
                    .finish();

            let card = conversation_navigation_card_with_icon(
                Some(status_icon),
                agent_name,
                Some(title),
                move |ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::RevealChildAgent { conversation_id });
                },
                mouse_state,
                true,
                Some(trailing),
                app,
            );
            column.add_child(Container::new(card).with_margin_top(4.).finish());
        }

        column.finish()
    }
}

/// T3-7: 提取子代理 conversation 当前/最近的 action presence summary,
/// 用于在父级 ChildAgentStatusCard 上 surface "↳ Searching codebase..." 之类的副标题。
/// 对齐 opencode TUI Task 父卡 `↳ Bash $ git status` 的视觉。
///
/// 实现:遍历 latest_exchange.output_status.output().messages 反向找最后一个
/// `AIAgentOutputMessageType::Action`,取 `presence_continuous_summary`。
/// 找不到则返回 None(card 不显示 subtitle)。
fn latest_action_presence(conversation: &AIConversation) -> Option<&'static str> {
    let exchange = conversation.latest_exchange()?;
    let output = exchange.output_status.output()?;
    output
        .get()
        .messages
        .iter()
        .rev()
        .find_map(|msg| match &msg.message {
            AIAgentOutputMessageType::Action(action) => {
                Some(action.action.presence_continuous_summary())
            }
            _ => None,
        })
}
