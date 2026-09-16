/// At the app level, we have structs for representing the UI framework level
/// notification structs, but with the data parsed to our liking.
/// The similar structs at the UI framework layer are lower-level (mostly strings).
use serde::{Deserialize, Serialize};
use warpui::{EntityId, WindowId};

use crate::pane_group::PaneId;

/// This data is passed along to the MacOS notification delegate and returned
/// to us when the notification is interacted with.
#[derive(Debug, Deserialize, Serialize)]
pub enum NotificationContext {
    /// For block-specific notifications
    BlockOrigin {
        window_id: WindowId,
        pane_group_id: EntityId,
        pane_id: PaneId,
    },
    /// dsh 插件通知:点击后切到 dsh pane(`pane_view_id` 为 PaneId 的
    /// creation_order_id)并在 dsh 内切到 `session_id` 对应会话。
    DshSession {
        window_id: WindowId,
        pane_view_id: EntityId,
        session_id: String,
    },
}
