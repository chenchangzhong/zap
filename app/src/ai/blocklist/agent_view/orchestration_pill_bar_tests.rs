//! `OrchestrationPillBar` 单测(本地裁剪版)。
//!
//! ## 为什么不是上游那份 760 行测试
//!
//! 上游 `orchestration_pill_bar_tests.rs` 的 12 个用例里有 5 个围绕**上游持久化
//! 形状**搭夹具:它们要构造
//! `AgentConversationData::{pinned, is_remote_child, root_task_is_optimistic,
//! orchestration_harness_type}`、`AgentConversationRecord::summary`、
//! `Message::fetched_memories`、`UserQuery::{origin, author, source_message}`,
//! 并调用 `crate::test_util::settings::initialize_history_persistence_for_tests`。
//! 本仓 `crates/persistence/src/model.rs` 没有这些字段、`test_util` 也没有该
//! helper,而改这两个文件等于顺带改持久化契约,不在本期范围。
//!
//! 因此这里保留**纯逻辑**用例,覆盖:
//! - pill 点击的动作映射(编排者 vs 子代理)
//! - breadcrumb 解析与 drill-down 锚定规则
//! - 头像初始字母 / 颜色等纯函数
//!
//! 被舍弃的上游用例(原因:依赖上游持久化/面板夹具字段):
//! - `pill_bar_data_layer_finds_restored_children_before_pane_creation`
//! - `breadcrumbs_resolve_token_only_parent_linkage_after_restore`
//! - `conversation_server_token_assignment_rerenders_the_pill_bar`
//! - `pill_bar_scrollable_finite_under_capped_drag_preview`
//!
//! ## 置顶仅内存(阶段二方案 A)
//!
//! 上游 `toggle_pin_persists_pinned_state_to_sqlite_event` 断言置顶会写 SQLite;
//! 本地没有 `pinned` 列,该断言不成立 —— 等价覆盖(内存状态)在
//! `orchestration_pill_bar_model_tests.rs`。

use warpui::{App, EntityId};

use super::*;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::blocklist::BlocklistAIHistoryModel;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

/// 本地等价于上游 `test_util::settings::initialize_history_persistence_for_tests`。
fn initialize_pill_bar_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
}

/// 建一棵三层会话树(root → mid → grandchild),返回 history 句柄与三个 id。
fn build_three_level_tree(
    app: &mut App,
) -> (
    warpui::ModelHandle<BlocklistAIHistoryModel>,
    AIConversationId,
    AIConversationId,
    AIConversationId,
) {
    initialize_pill_bar_test_app(app);
    let surface_id = EntityId::new();
    let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
    let (root_id, mid_id, grandchild_id) = history_model.update(app, |history, ctx| {
        let root_id = history.start_new_conversation(surface_id, false, false, ctx);
        let mid_id = history.start_new_child_conversation(
            surface_id,
            "mid".to_string(),
            root_id,
            AIConversationId::new(),
            false,
            ctx,
        );
        let grandchild_id = history.start_new_child_conversation(
            surface_id,
            "grandchild".to_string(),
            mid_id,
            AIConversationId::new(),
            false,
            ctx,
        );
        (root_id, mid_id, grandchild_id)
    });
    (history_model, root_id, mid_id, grandchild_id)
}

// ---------------------------------------------------------------------------
// pill 点击 → 动作映射
// ---------------------------------------------------------------------------

#[test]
fn navigation_action_for_child_pill_reveals_existing_child_pane() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Child, conversation_id),
        TerminalAction::RevealChildAgent {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}

#[test]
fn navigation_action_for_orchestrator_pill_switches_in_place() {
    let conversation_id = AIConversationId::new();

    assert!(matches!(
        navigation_action_for_pill(PillKind::Orchestrator, conversation_id),
        TerminalAction::SwitchAgentViewToConversation {
            conversation_id: actual_id,
        } if actual_id == conversation_id
    ));
}

// ---------------------------------------------------------------------------
// 下钻锚点 / breadcrumb 解析
// ---------------------------------------------------------------------------

#[test]
fn breadcrumb_ids_show_only_root_when_parent_is_root() {
    App::test((), |mut app| async move {
        let (history_model, root_id, mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(breadcrumb_ids(history, mid_id), (Some(root_id), None));
        });
    });
}

#[test]
fn breadcrumb_ids_show_root_and_parent_when_two_levels_deep() {
    App::test((), |mut app| async move {
        let (history_model, root_id, mid_id, grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(
                breadcrumb_ids(history, grandchild_id),
                (Some(root_id), Some(mid_id)),
            );
        });
    });
}

#[test]
fn breadcrumb_ids_are_empty_at_the_root() {
    App::test((), |mut app| async move {
        let (history_model, root_id, _mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        history_model.read(&app, |history, _| {
            assert_eq!(breadcrumb_ids(history, root_id), (None, None));
        });
    });
}

#[test]
fn drill_down_anchor_is_the_parent_level_for_a_leaf() {
    App::test((), |mut app| async move {
        let (_history_model, _root_id, mid_id, grandchild_id) = build_three_level_tree(&mut app);
        app.read(|ctx| {
            let grandchild = BlocklistAIHistoryModel::as_ref(ctx)
                .conversation(&grandchild_id)
                .expect("grandchild conversation exists");
            assert_eq!(
                drill_down_anchor_id(grandchild_id, grandchild, ctx),
                mid_id,
                "叶子节点锚定父层,保证兄弟导航对称",
            );
        });
    });
}

#[test]
fn drill_down_anchor_is_the_node_itself_when_it_has_children() {
    App::test((), |mut app| async move {
        let (_history_model, root_id, mid_id, _grandchild_id) = build_three_level_tree(&mut app);
        app.read(|ctx| {
            let history = BlocklistAIHistoryModel::as_ref(ctx);
            let root = history.conversation(&root_id).expect("root exists");
            let mid = history.conversation(&mid_id).expect("mid exists");
            assert_eq!(drill_down_anchor_id(root_id, root, ctx), root_id);
            assert_eq!(
                drill_down_anchor_id(mid_id, mid, ctx),
                mid_id,
                "有子节点的会话锚定自己这一层",
            );
        });
    });
}

// ---------------------------------------------------------------------------
// 纯函数:头像初始字母
// ---------------------------------------------------------------------------

#[test]
fn pill_initial_uses_first_char_and_falls_back_for_empty_names() {
    assert_eq!(pill_initial("Agent 1"), 'A');
    // 空名字必须给出稳定回退,否则头像会渲染成空白。
    assert_eq!(pill_initial(""), 'A');
}
