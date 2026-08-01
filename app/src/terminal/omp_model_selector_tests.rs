//! OmpModelSelector 回归测试。
//!
//! 覆盖 bug:omp 模型下拉搜索框输入中文(IME 组合态)直接回车导致 Zap 闪退,
//! panic 信息为 "Circular view update"(crates/warpui_core/src/core/app.rs:4331)。
//!
//! 崩溃链(修复前):
//!   Enter 键同时触发 filter_editor 的 EditorEvent::Enter 与 Menu 的全局固定
//!   绑定 MenuAction::Enter;后者 dispatch_typed_action(SelectModel) 时 Menu
//!   正被外层 dispatch_typed_action 从 window.views 移除(尚未插回);
//!   handle_action(SelectModel) → rebuild_menu_items → dropdown.update
//!   → update_view(Menu) 时 remove 返回 None → panic。
//!
//! 本测试直接复现「dropdown 的 update 闭包内再次 update dropdown」这条崩溃路径:
//! 修复后(SelectModel 分支不再 rebuild)不应 panic,且 SelectModel 效果照常生效。
//! 若有人把 `self.rebuild_menu_items(ctx);` 加回 SelectModel 分支,本测试立即
//! panic("Circular view update")。

use warpui::{platform::WindowStyle, App, EntityId, SingletonEntity, TypedActionView, UpdateView};

use super::{OmpModelSelector, OmpModelSelectorAction};
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext, CLIAgentSessionStatus,
    CLIAgentSessionsModel,
};
use crate::terminal::CLIAgent;
use crate::test_util::terminal::initialize_app_for_terminal_view;

#[test]
fn select_model_from_menu_action_stack_does_not_panic() {
    App::test((), |mut app| async move {
        // 注册 Appearance mock 与 CLIAgentSessionsModel 等全部依赖:
        // OmpModelSelector::new 里用到了 Appearance::as_ref 与
        // CLIAgentSessionsModel::handle,缺了会 panic。
        initialize_app_for_terminal_view(&mut app);

        // omp_binary 指向不存在的路径:new() 里 spawn 的 trigger_refresh 会快速失败
        // (Command 找不到二进制),不启动真实 omp 子进程,测试不慢也不依赖外部环境。
        let (_window_id, selector) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            OmpModelSelector::new("/nonexistent/omp".into(), EntityId::new(), ctx)
        });

        // 模拟真实崩溃场景:Menu 正在被外层 update 占用(已从 window.views 移除),
        // 此时在 dropdown 的 update 闭包内触发 SelectModel —— 等价于真实链路
        // MenuAction::Enter → dispatch SelectModel → handle_action
        // → (旧代码)rebuild_menu_items → dropdown.update → 'Circular view update'。
        let dropdown = selector.read(&app, |sel, _| sel.dropdown.clone());
        // update 闭包会 move 捕获的句柄,先克隆一份供闭包内使用,
        // 闭包外的 selector 保留用于后续断言。
        let selector_in_update = selector.clone();
        dropdown.update(&mut app, |_menu, ctx| {
            ctx.update_view(&selector_in_update, |sel, ctx| {
                sel.handle_action(
                    &OmpModelSelectorAction::SelectModel { selector: "test/model".into() },
                    ctx,
                );
            });
        });

        // 修复后:SelectModel 正常生效 —— 选中模型已更新、菜单已关闭。
        // (修复前执行到这里之前就已 panic 'Circular view update'。)
        selector.read(&app, |sel, _| {
            assert_eq!(sel.selected_model.as_deref(), Some("test/model"));
            assert!(!sel.menu_is_open);
        });
    });
}

/// 构造一个最小可用的 CLIAgentSession,只定制 session_context。
/// 其余字段取与 mod_tests.rs 一致的占位值(listener 为 None,不启动任何插件)。
fn cli_agent_session_with(session_context: CLIAgentSessionContext) -> CLIAgentSession {
    CLIAgentSession {
        agent: CLIAgent::Claude,
        status: CLIAgentSessionStatus::InProgress,
        session_context,
        input_state: CLIAgentInputState::Closed,
        should_auto_toggle_input: false,
        listener: None,
        remote_host: None,
        plugin_version: None,
        draft_text: None,
        custom_command_prefix: None,
        current_model: None,
    }
}

/// 回归测试:current_socket_id 的 socket id 选择优先级(model_switch_socket_id > session_id)。
///
/// Bug 场景:omp 恢复(resume)旧会话时,OSC777 session_start 通知带的是旧 session_id,
/// 而 socket 文件按扩展上报的 model_switch_socket_id 命名 —— 旧代码只取 session_id
/// 构造 socket 路径,导致 socket 找不到、每次切换模型都失败。修复后立即路径与
/// notify_when_ready 轮询路径统一走 current_socket_id,优先扩展上报的 socket id。
///
/// 若有人把 current_socket_id 改回「只取 session_id」或颠倒优先级,本测试立即失败。
#[test]
fn current_socket_id_prefers_model_switch_socket_id_over_session_id() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        // 用同一个合成 terminal_view_id 构造 selector 并注入 session,
        // 只测 session 上下文查找 + 优先级逻辑,不依赖真实 TerminalView。
        let terminal_view_id = EntityId::new();
        let (_window_id, selector) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            OmpModelSelector::new("/nonexistent/omp".into(), terminal_view_id, ctx)
        });

        // 恢复旧会话的典型状态:OSC777 的旧 session_id 与扩展上报的 socket id 同时存在。
        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(
                terminal_view_id,
                cli_agent_session_with(CLIAgentSessionContext {
                    session_id: Some("old-osc777-session-id".into()),
                    model_switch_socket_id: Some("extension-reported-socket-id".into()),
                    ..Default::default()
                }),
                ctx,
            );
        });

        selector.read(&app, |sel, ctx| {
            assert_eq!(
                sel.current_socket_id(ctx).as_deref(),
                Some("extension-reported-socket-id")
            );
        });
    });
}

/// 扩展尚未上报 model_switch_socket_id(旧扩展或上报晚于 session_start)时,
/// current_socket_id 必须回退到 OSC777 的 session_id —— 不能因为 socket id 缺失
/// 就返回 None 放弃切换(那会让 notify_when_ready 白等 3 秒)。
#[test]
fn current_socket_id_falls_back_to_session_id_when_socket_id_missing() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view_id = EntityId::new();
        let (_window_id, selector) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            OmpModelSelector::new("/nonexistent/omp".into(), terminal_view_id, ctx)
        });

        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(
                terminal_view_id,
                cli_agent_session_with(CLIAgentSessionContext {
                    session_id: Some("old-osc777-session-id".into()),
                    ..Default::default()
                }),
                ctx,
            );
        });

        selector.read(&app, |sel, ctx| {
            assert_eq!(
                sel.current_socket_id(ctx).as_deref(),
                Some("old-osc777-session-id")
            );
        });
    });
}

/// 两个 id 都缺失时返回 None:调用方据此走 notify_when_ready 轮询等待 session 就绪。
#[test]
fn current_socket_id_none_when_no_session_context() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);

        let terminal_view_id = EntityId::new();
        let (_window_id, selector) = app.add_window(WindowStyle::NotStealFocus, |ctx| {
            OmpModelSelector::new("/nonexistent/omp".into(), terminal_view_id, ctx)
        });

        CLIAgentSessionsModel::handle(&app).update(&mut app, |sessions, ctx| {
            sessions.set_session(
                terminal_view_id,
                cli_agent_session_with(CLIAgentSessionContext::default()),
                ctx,
            );
        });

        selector.read(&app, |sel, ctx| {
            assert_eq!(sel.current_socket_id(ctx), None);
        });
    });
}
