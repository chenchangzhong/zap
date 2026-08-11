use warpui::{SingletonEntity, ViewContext};

use crate::{
    ai::{
        agent::conversation::AIConversationId,
        blocklist::block::{PendingUserQueryBlock, PendingUserQueryBlockEvent},
    },
    auth::AuthStateProvider,
    terminal::TerminalView,
};

use super::rich_content::RichContentMetadata;

impl TerminalView {
    pub(super) fn pending_user_query_conversation_id(&self) -> Option<AIConversationId> {
        let view_id = self.pending_user_query_view_id?;
        self.rich_content_views
            .iter()
            .find(|rich_content| rich_content.view_id() == view_id)
            .and_then(|rich_content| rich_content.agent_view_conversation_id())
    }

    /// 为本地 ambient agent run 插入一个 pending user query block,直到 harness CLI 启动。
    /// 这个 block 只展示用户 prompt 和 queued 状态,没有行内动作按钮 ——
    /// `/compact-and`、`/fork-and-compact`、`/queue` 的排队提示词现在统一走
    /// [`QueuedQueryModel`] 与 queued prompts 面板。
    pub(in crate::terminal::view) fn insert_ambient_agent_queued_user_query_block(
        &mut self,
        prompt: String,
        ctx: &mut ViewContext<Self>,
    ) {
        self.remove_pending_user_query_block(ctx);
        let auth_state = AuthStateProvider::as_ref(ctx).get().clone();
        let user_display_name = auth_state
            .username_for_display()
            .unwrap_or_else(|| "User".to_owned());
        let profile_image_path = auth_state.user_photo_url();

        let handle = ctx.add_typed_action_view(|ctx| {
            PendingUserQueryBlock::new(prompt, user_display_name, profile_image_path, ctx)
        });
        ctx.subscribe_to_view(&handle, move |me, block, event, ctx| match event {
            PendingUserQueryBlockEvent::TextSelected => {
                // 确保整个终端 view 内只有一个活跃文字选区。
                me.clear_selected_text_except(Some(block.id()), ctx);
            }
        });
        let view_id = handle.id();

        self.insert_rich_content(
            None,
            handle.clone(),
            Some(RichContentMetadata::PendingUserQuery {
                pending_user_query_block_handle: handle,
            }),
            super::rich_content::RichContentInsertionPosition::PinToBottom,
            ctx,
        );
        self.pending_user_query_view_id = Some(view_id);
    }

    /// Removes the pending user query block, if one exists. No-op if none is present.
    pub(super) fn remove_pending_user_query_block(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(view_id) = self.pending_user_query_view_id.take() {
            self.model
                .lock()
                .block_list_mut()
                .remove_rich_content(view_id);
            self.rich_content_views.retain(|rc| rc.view_id() != view_id);
            ctx.notify();
        }
    }
}
