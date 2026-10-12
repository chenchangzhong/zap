//! 编排 pill bar 的跨 pane 共享 UI 状态单例:被置顶的子代理会话集合,以及
//! 每个编排树的横向滚动句柄(让同一编排树在不同 pane 里保持同一滚动位置)。
//!
//! **本地偏差(阶段二方案 A)**:上游把置顶状态持久化(`pinned` 列 + 启动时读回
//! `initial_pinned`),本地没有该列,所以置顶**仅内存**:
//! - 单例自身维护 `pinned` 集合(快路径 + 跨 pane 通知);
//! - 同时把状态写到 `AIConversation::set_pinned`(同样仅内存)。
//!
//! 因此 `new` 的 `initial_pinned` 在本地启动时恒为空集;保留这个入参是为了与
//! 上游签名一致、并在将来补上持久化列时不必改调用点。
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use warpui::elements::ClippedScrollStateHandle;
use warpui::{Entity, ModelContext, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::{BlocklistAIHistoryEvent, BlocklistAIHistoryModel};

/// 在集合里切换 `id` 的置顶状态 —— 不存在则插入,存在则移除。
/// 返回切换后的成员状态(`true` = 现在已置顶)。
/// 抽成自由函数是为了让切换语义可以在不启动单例的情况下单测。
pub(super) fn toggle_pin_in_set(
    pinned: &mut HashSet<AIConversationId>,
    id: AIConversationId,
) -> bool {
    if pinned.remove(&id) {
        false
    } else {
        pinned.insert(id);
        true
    }
}

/// 编排 pill bar 的跨 pane 共享 UI 状态单例。
pub struct OrchestrationPillBarModel {
    /// 哪些子会话被置顶的内存镜像。本地的「真源」也只到这里为止(见模块文档)。
    pinned: HashSet<AIConversationId>,
    /// 每个编排根会话 id 一个滚动句柄。渲染同一编排树的每个 pill bar 都克隆同一
    /// 句柄,所以在一个 pane 里拖动会同步到兄弟 pane。用 `RefCell` 是因为句柄需要
    /// 从 `&AppContext` 路径里惰性创建。
    horizontal_scroll_states: RefCell<HashMap<AIConversationId, ClippedScrollStateHandle>>,
}

impl OrchestrationPillBarModel {
    /// 构造单例,并以启动时已置顶的会话 id 集合作为种子。
    pub fn new(initial_pinned: HashSet<AIConversationId>, ctx: &mut ModelContext<Self>) -> Self {
        // 会话被删除时在全局把置顶清掉,免得每个 pane 各自处理(也免得互相覆盖
        // 兄弟 pane 的置顶状态)。
        let history_handle = BlocklistAIHistoryModel::handle(ctx);
        ctx.subscribe_to_model(&history_handle, |this, _, event, ctx| match event {
            BlocklistAIHistoryEvent::RemoveConversation {
                conversation_id, ..
            }
            | BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => {
                if this.pinned.remove(conversation_id) {
                    ctx.emit(OrchestrationPillBarEvent::PinSetChanged);
                }
                // 顺手丢掉对应的滚动句柄,避免被删掉的编排根在表里泄漏。
                // 无需发事件 —— 没有订阅方关心滚动状态。
                this.horizontal_scroll_states
                    .borrow_mut()
                    .remove(conversation_id);
            }
            _ => {}
        });

        Self {
            pinned: initial_pinned,
            horizontal_scroll_states: RefCell::new(HashMap::new()),
        }
    }

    /// 该会话当前是否已置顶。
    pub fn is_pinned(&self, conversation_id: &AIConversationId) -> bool {
        self.pinned.contains(conversation_id)
    }

    /// 取该编排树共享的横向滚动句柄,首次访问时惰性创建。同一编排树里的每个
    /// pill bar 都克隆同一个句柄,所以在一个 pane 里拖动会与兄弟 pane 保持一致。
    pub fn horizontal_scroll_state_for(
        &self,
        orchestrator_id: AIConversationId,
    ) -> ClippedScrollStateHandle {
        self.horizontal_scroll_states
            .borrow_mut()
            .entry(orchestrator_id)
            .or_default()
            .clone()
    }

    /// 切换 `conversation_id` 的置顶状态,把新状态写到会话上,并通知订阅者。
    pub fn toggle_pin(&mut self, conversation_id: AIConversationId, ctx: &mut ModelContext<Self>) {
        let now_pinned = toggle_pin_in_set(&mut self.pinned, conversation_id);
        BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, ctx| {
            history.set_conversation_pinned(conversation_id, now_pinned, ctx);
        });
        ctx.emit(OrchestrationPillBarEvent::PinSetChanged);
    }

    /// 清空内存里的置顶集合与滚动句柄缓存。登出时调用,避免下一个账号继承上一个
    /// 账号的 UI 状态。
    pub fn reset(&mut self) {
        self.pinned.clear();
        self.horizontal_scroll_states.borrow_mut().clear();
    }
}

impl Entity for OrchestrationPillBarModel {
    type Event = OrchestrationPillBarEvent;
}

impl SingletonEntity for OrchestrationPillBarModel {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OrchestrationPillBarEvent {
    /// 置顶集合发生变化(用户切换或随历史删除被清理)。
    PinSetChanged,
}

#[cfg(test)]
#[path = "orchestration_pill_bar_model_tests.rs"]
mod tests;
