use warpui::{AppContext, EntityId, SingletonEntity};

use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent_conversations_model::AgentConversationsModel;
use crate::ai::blocklist::history_model::BlocklistAIHistoryModel;

/// Delete a conversation from the blocklist and local storage.
pub fn delete_conversation(
    conversation_id: AIConversationId,
    terminal_view_id: Option<EntityId>,
    ctx: &mut AppContext,
) {
    delete_conversations([conversation_id], terminal_view_id, ctx);
}

/// Delete multiple conversations from the blocklist and local storage.
pub fn delete_conversations(
    conversation_ids: impl IntoIterator<Item = AIConversationId>,
    terminal_view_id: Option<EntityId>,
    ctx: &mut AppContext,
) {
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
        history.delete_conversations(conversation_ids, terminal_view_id, model_ctx);
    });

    // Make sure the agent conversations model is up to date.
    AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
        model.sync_conversations(ctx);
    });
}

/// Delete all conversations from the blocklist and local storage.
///
/// 进行中/ambient 对话会被跳过,返回被跳过的数量。
pub fn delete_all_conversations(ctx: &mut AppContext) -> usize {
    let skipped = BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
        history.delete_all_conversations(model_ctx)
    });

    // Make sure the agent conversations model is up to date.
    AgentConversationsModel::handle(ctx).update(ctx, |model, ctx| {
        model.sync_conversations(ctx);
    });

    skipped
}

/// Remove a conversation from the blocklist.
///
/// This is similar to `delete_conversation`, but it is only used for ephemeral/empty conversations
/// (i.e. conversations from closed empty agent mode views, or passive code diffs).
pub fn remove_conversation(
    conversation_id: AIConversationId,
    terminal_view_id: EntityId,
    // Kept for API compatibility with existing callers; cloud sync was removed
    // when remote conversation sharing was deleted.
    _delete_from_cloud: bool,
    ctx: &mut AppContext,
) {
    BlocklistAIHistoryModel::handle(ctx).update(ctx, |history, model_ctx| {
        history.remove_conversation(conversation_id, terminal_view_id, model_ctx);
    });
}
