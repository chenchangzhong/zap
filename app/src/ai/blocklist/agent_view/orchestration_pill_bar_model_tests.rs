//! `OrchestrationPillBarModel` 单测。
//!
//! **本地偏差(阶段二方案 A)**:置顶仅内存,所以上游「toggle 会写 SQLite
//! `UpdateMultiAgentConversation` 事件并带 `pinned: true`」那个用例在本仓
//! **无法成立**(没有 `pinned` 列)。这里改为断言「内存单例 + `AIConversation`
//! 的 `pinned` 标记都被更新」,并把「不落库」这一事实显式锁进用例名,避免后人
//! 误以为置顶会跨重启保留。

use std::collections::HashSet;

use warpui::App;

use super::*;
use crate::ai::agent::conversation::AIConversation;
use crate::test_util::settings::initialize_settings_for_tests;
use crate::{GlobalResourceHandles, GlobalResourceHandlesProvider};

fn initialize_pill_bar_model_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
}

#[test]
fn toggle_pin_in_set_flips_membership_for_each_call() {
    let mut pinned: HashSet<AIConversationId> = HashSet::new();
    let id = AIConversationId::new();

    assert!(!pinned.contains(&id));

    // 第一次切换置顶,并返回切换后的状态。
    assert!(toggle_pin_in_set(&mut pinned, id));
    assert!(pinned.contains(&id));
    assert_eq!(pinned.len(), 1);

    // 第二次切换取消置顶。
    assert!(!toggle_pin_in_set(&mut pinned, id));
    assert!(!pinned.contains(&id));
    assert!(pinned.is_empty());

    // 第三次再置顶,确认成对调用不会累积状态。
    assert!(toggle_pin_in_set(&mut pinned, id));
    assert!(pinned.contains(&id));
}

#[test]
fn toggle_pin_in_set_only_affects_target_id() {
    let mut pinned: HashSet<AIConversationId> = HashSet::new();
    let a = AIConversationId::new();
    let b = AIConversationId::new();

    toggle_pin_in_set(&mut pinned, a);
    toggle_pin_in_set(&mut pinned, b);
    assert!(pinned.contains(&a));
    assert!(pinned.contains(&b));

    // 取消 `a` 不影响 `b`。
    toggle_pin_in_set(&mut pinned, a);
    assert!(!pinned.contains(&a));
    assert!(pinned.contains(&b));
}

/// 端到端:切换置顶会同时更新 (a) 单例的内存置顶集合与 (b) history model 上
/// 会话的 `is_pinned()`。
///
/// **注意**:本地**不校验**持久化 —— 没有 `pinned` 列,置顶跨重启不保留。
#[test]
fn toggle_pin_updates_memory_state_without_persistence() {
    App::test((), |mut app| async move {
        initialize_pill_bar_model_test_app(&mut app);
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation = AIConversation::new(false);
        let conversation_id = conversation.id();
        let terminal_view_id = warpui::EntityId::new();
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        // 置顶单例必须在 history model 之后注册 —— 它在构造时订阅 history 事件。
        let pin_model =
            app.add_singleton_model(|ctx| OrchestrationPillBarModel::new(HashSet::new(), ctx));

        pin_model.update(&mut app, |model, ctx| {
            model.toggle_pin(conversation_id, ctx);
        });

        pin_model.read(&app, |model, _| {
            assert!(
                model.is_pinned(&conversation_id),
                "切换后置顶集合应包含该会话"
            );
        });
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.conversation(&conversation_id).map(|c| c.is_pinned()),
                Some(true),
                "AIConversation.is_pinned() 应反映这次切换"
            );
        });

        // 再切一次应取消置顶。
        pin_model.update(&mut app, |model, ctx| {
            model.toggle_pin(conversation_id, ctx);
        });
        pin_model.read(&app, |model, _| {
            assert!(!model.is_pinned(&conversation_id), "第二次切换应取消置顶");
        });
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.conversation(&conversation_id).map(|c| c.is_pinned()),
                Some(false)
            );
        });
    });
}

/// `reset()` 必须同时清空置顶集合与滚动句柄缓存(登出路径)。
#[test]
fn reset_clears_pins_and_scroll_states() {
    App::test((), |mut app| async move {
        initialize_pill_bar_model_test_app(&mut app);
        app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let pin_model =
            app.add_singleton_model(|ctx| OrchestrationPillBarModel::new(HashSet::new(), ctx));

        let orchestrator_id = AIConversationId::new();
        pin_model.read(&app, |model, _| {
            // 惰性建一个滚动句柄,证明 `reset` 前缓存非空。
            let _ = model.horizontal_scroll_state_for(orchestrator_id);
        });

        pin_model.update(&mut app, |model, _| {
            model.reset();
        });

        pin_model.read(&app, |model, _| {
            assert!(!model.is_pinned(&orchestrator_id));
            // 缓存被清掉后再次取用会重新创建,不应 panic 且依旧可用。
            let _ = model.horizontal_scroll_state_for(orchestrator_id);
        });
    });
}
