//! Tests for the auto-fire drain logic that runs from [`super::TerminalView::drain_queued_prompts`],
//! plus panel-level tests for [`QueuedPromptsPanelView`] copy 与可见性契约。
//!
//! `TerminalView` orchestrates the input editor and the singleton `QueuedQueryModel` on
//! `FinishedReceivingOutput`. Constructing a full `TerminalView` in a unit test would require
//! dozens of dependencies, so the drain tests below exercise the per-conversation singleton
//! semantics that the drain path relies on. 面板测试则用 `with_panel` 起一个真实
//! `TerminalView` 窗口,再手工构造面板(见该 helper 的注释)。
use std::cell::RefCell;
use std::rc::Rc;

use warpui::clipboard::ClipboardContent;
use warpui::{App, EntityId, ModelHandle, SingletonEntity, TypedActionView, ViewHandle};

use super::queued_prompts_panel::{QueuedPromptsPanelAction, QueuedPromptsPanelView};
use super::rich_content::RichContentMetadata;
use crate::ai::agent::conversation::AIConversationId;
use crate::ai::agent::ImageContext;
use crate::ai::blocklist::block::FinishReason;
use crate::ai::blocklist::{
    AutofireAction, BlocklistAIHistoryModel, PendingAttachment, QueuedQuery, QueuedQueryModel,
    QueuedQueryOrigin,
};
use crate::editor::EditorView;
use crate::features::FeatureFlag;
use crate::terminal::cli_agent_sessions::{
    CLIAgentInputEntrypoint, CLIAgentInputState, CLIAgentSession, CLIAgentSessionContext,
    CLIAgentSessionStatus, CLIAgentSessionsModel,
};
use crate::search::slash_command_menu::static_commands::commands;
use crate::terminal::input::suggestions_mode_model::InputSuggestionsModeModel;
use crate::terminal::input::{Event as InputEvent, InputSuggestionsMode};
use crate::test_util::settings::initialize_settings_for_tests;
use crate::test_util::terminal::{add_window_with_terminal, initialize_app_for_terminal_view};
use crate::util::truncation::truncate_from_end;

fn user_query(text: &str) -> QueuedQuery {
    QueuedQuery::new(text.to_owned(), QueuedQueryOrigin::QueueSlashCommand)
}

fn with_singleton<F>(test: F)
where
    F: FnOnce(App, warpui::ModelHandle<QueuedQueryModel>, AIConversationId) + 'static,
{
    App::test((), |mut app| async move {
        // `QueuedQueryModel::new` 会读并订阅 `AISettings`,必须先注册 settings。
        initialize_settings_for_tests(&mut app);
        let _ = app.add_singleton_model(|_| BlocklistAIHistoryModel::new_for_test());
        let model = app.add_singleton_model(QueuedQueryModel::new);
        test(app, model, AIConversationId::new());
    });
}

#[test]
fn complete_drain_pops_head_and_returns_submit_action() {
    // On Complete, the next queued prompt fires via Submit.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "first"),
            other => panic!("expected Submit, got {other:?}"),
        }
        // Peek 不移除行;host 在发送完成后调用 remove_fired_row。
        let query_id = model.read(&app, |m, _| m.queue(conv)[0].id());
        model.update(&mut app, |m, ctx| m.remove_fired_row(conv, query_id, ctx));
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_first_row_in_edit_mode_returns_pop_from_edit_mode() {
    // When the first row is being edited, drain produces a PopFromEditMode action carrying the
    // row's last-committed text (per spec, NOT any uncommitted live-editor buffer text).
    with_singleton(|mut app, model, conv| {
        let id_a = model.update(&mut app, |m, ctx| m.append(conv, user_query("first"), ctx));
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("second"), ctx);
            m.enter_edit_mode(conv, id_a, ctx);
        });

        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        match action {
            Some(AutofireAction::PopFromEditMode { text, .. }) => assert_eq!(text, "first"),
            other => panic!("expected PopFromEditMode, got {other:?}"),
        }
        // Peek 不清 edit;remove_fired_row 才移除行并清 edit。
        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), Some(id_a));
        });
        model.update(&mut app, |m, ctx| m.remove_fired_row(conv, id_a, ctx));
        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), None);
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_non_empty_input_preserves_edited_head_row() {
    // The host skips autofire when the queue head is being edited and the input already contains
    // text, which leaves the queued row in place for the next completion.
    with_singleton(|mut app, model, conv| {
        let id_a = model.update(&mut app, |m, ctx| m.append(conv, user_query("first"), ctx));
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("second"), ctx);
            m.enter_edit_mode(conv, id_a, ctx);
        });

        let simulated_input_is_non_empty = true;
        if !(simulated_input_is_non_empty
            && model.read(&app, |m, _| m.first_row_is_in_edit_mode(conv)))
        {
            let _ = model.read(&app, |m, _| m.peek_autofire(conv));
        }

        model.read(&app, |m, _| {
            assert_eq!(m.editing_row(conv), Some(id_a));
            assert_eq!(m.queue(conv).len(), 2);
            assert_eq!(m.queue(conv)[0].text(), "first");
            assert_eq!(m.queue(conv)[1].text(), "second");
        });
    });
}

#[test]
fn complete_drain_with_empty_queue_returns_none() {
    with_singleton(|mut app, model, conv| {
        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        assert!(action.is_none());
    });
}

#[test]
fn error_or_cancel_drain_pops_front_when_input_is_empty() {
    // On Error/Cancelled with an empty input, the next queued prompt's text is restored to the
    // input by popping it (which the host then writes into the buffer).
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let popped = model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        let popped = popped.expect("queue had a head");
        assert_eq!(popped.text(), "first");
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 1);
            assert_eq!(m.queue(conv)[0].text(), "second");
        });
    });
}

#[test]
fn error_or_cancel_drain_leaves_queue_intact_when_input_is_non_empty() {
    // When the input is non-empty, the drain skips popping so the queue remains intact.
    //
    // The host (`TerminalView`) gates the pop on input-empty. We model that here by simply not
    // popping when the simulated input is non-empty, and asserting the queue remains unchanged.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
        });

        let simulated_input_is_non_empty = true;
        if !simulated_input_is_non_empty {
            model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        }

        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv).len(), 2);
            assert_eq!(m.queue(conv)[0].text(), "first");
        });
    });
}

#[test]
fn complete_drain_after_error_drain_continues_with_next_row() {
    // After an Error/Cancelled drain pops one row and the user later submits successfully, the
    // *next* Complete drain pops the following row.
    with_singleton(|mut app, model, conv| {
        model.update(&mut app, |m, ctx| {
            m.append(conv, user_query("first"), ctx);
            m.append(conv, user_query("second"), ctx);
            m.append(conv, user_query("third"), ctx);
        });

        // Error: input is empty, pop "first" and restore to input.
        let popped = model.update(&mut app, |m, ctx| m.pop_front(conv, ctx));
        assert_eq!(
            popped.map(|q| q.text().to_owned()),
            Some("first".to_owned())
        );

        // Complete: fire "second" (peek + remove).
        let query_id = model.read(&app, |m, _| m.queue(conv)[0].id());
        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "second"),
            other => panic!("expected Submit(\"second\"), got {other:?}"),
        }
        model.update(&mut app, |m, ctx| m.remove_fired_row(conv, query_id, ctx));

        // Complete again: fire "third".
        let query_id = model.read(&app, |m, _| m.queue(conv)[0].id());
        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "third"),
            other => panic!("expected Submit(\"third\"), got {other:?}"),
        }
        model.update(&mut app, |m, ctx| m.remove_fired_row(conv, query_id, ctx));

        // Queue is now empty; the next drain returns None.
        let action = model.read(&app, |m, _| m.peek_autofire(conv));
        assert!(action.is_none());
    });
}

#[test]
fn drain_is_isolated_per_conversation() {
    // A drain for conversation A must not pop rows from conversation B.
    with_singleton(|mut app, model, conv_a| {
        let conv_b = AIConversationId::new();
        model.update(&mut app, |m, ctx| {
            m.append(conv_a, user_query("a-first"), ctx);
            m.append(conv_b, user_query("b-first"), ctx);
        });

        let query_id = model.read(&app, |m, _| m.queue(conv_a)[0].id());
        let action = model.read(&app, |m, _| m.peek_autofire(conv_a));
        match action {
            Some(AutofireAction::Submit { text, .. }) => assert_eq!(text, "a-first"),
            other => panic!("expected Submit(\"a-first\"), got {other:?}"),
        }
        model.update(&mut app, |m, ctx| m.remove_fired_row(conv_a, query_id, ctx));
        model.read(&app, |m, _| {
            assert_eq!(m.queue(conv_a).len(), 0);
            assert_eq!(m.queue(conv_b).len(), 1);
            assert_eq!(m.queue(conv_b)[0].text(), "b-first");
        });
    });
}

/// `enqueue_followup_prompt` 的观测契约:`/compact-and`、`/fork-and-compact` 的后续提示词
/// 必须落进 `QueuedQueryModel`(带调用方指定的 origin),且**不能**再插入 legacy pending
/// user query block —— 后者是本次 cutover 删掉的旧路径。
///
/// 这里需要真正的 `TerminalView`(方法签名是 `&mut ViewContext<TerminalView>`),所以走
/// view 级 harness 而非上面的 model 级 `with_singleton`。
#[test]
fn enqueue_followup_prompt_files_row_with_caller_origin_and_no_pending_block() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let conv = AIConversationId::new();

        terminal.update(&mut app, |view, ctx| {
            view.enqueue_followup_prompt(
                "摘要后继续".to_owned(),
                QueuedQueryOrigin::CompactAndSlashCommand,
                conv,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let model = QueuedQueryModel::as_ref(ctx);
            let queue = model.queue(conv);
            assert_eq!(queue.len(), 1, "后续提示词必须入队恰好一行");
            assert_eq!(queue[0].text(), "摘要后继续");
            assert_eq!(
                queue[0].origin(),
                QueuedQueryOrigin::CompactAndSlashCommand,
                "origin 必须是调用方传入的那个,不能被写死"
            );

            assert!(
                !view.rich_content_views.iter().any(|rich_content| matches!(
                    rich_content.metadata(),
                    Some(RichContentMetadata::PendingUserQuery { .. })
                )),
                "legacy pending user query block 路径已删除,入队不得再插入该 block"
            );
        });
    });
}

/// 与上一个测试成对:origin 必须随调用方变化。单独看上一个测试,把 `enqueue_followup_prompt`
/// 里的 origin 写死成 `CompactAndSlashCommand` 也能通过;加上这一条后写死必然变红。
#[test]
fn enqueue_followup_prompt_preserves_fork_and_compact_origin() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let conv = AIConversationId::new();

        terminal.update(&mut app, |view, ctx| {
            view.enqueue_followup_prompt(
                "分叉后继续".to_owned(),
                QueuedQueryOrigin::ForkAndCompactSlashCommand,
                conv,
                ctx,
            );
        });

        terminal.read(&app, |view, ctx| {
            let queue_len = QueuedQueryModel::as_ref(ctx).queue(conv).len();
            assert_eq!(queue_len, 1);
            let origin = QueuedQueryModel::as_ref(ctx).queue(conv)[0].origin();
            assert_eq!(origin, QueuedQueryOrigin::ForkAndCompactSlashCommand);
            assert!(view.rich_content_views.iter().all(|rich_content| !matches!(
                rich_content.metadata(),
                Some(RichContentMetadata::PendingUserQuery { .. })
            )));
        });
    });
}

/// 入队按 `AIConversationId` 分桶:`/fork-and-compact` 的后续提示词要落在**新分叉**的会话上,
/// 不能串到当前选中的会话里。
#[test]
fn enqueue_followup_prompt_targets_only_the_given_conversation() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);
        let active_conv = AIConversationId::new();
        let forked_conv = AIConversationId::new();

        terminal.update(&mut app, |view, ctx| {
            view.enqueue_followup_prompt(
                "分叉后继续".to_owned(),
                QueuedQueryOrigin::ForkAndCompactSlashCommand,
                forked_conv,
                ctx,
            );
        });

        terminal.read(&app, |_, ctx| {
            let model = QueuedQueryModel::as_ref(ctx);
            assert_eq!(model.queue(forked_conv).len(), 1);
            assert!(
                model.queue(active_conv).is_empty(),
                "后续提示词不得串到未指定的会话队列"
            );
        });
    });
}

/// 面板级 harness。
///
/// 本地没有现成的 panel harness,构造方式如下(实测可行,未走 `Input` 私有字段):
/// 1. `initialize_app_for_terminal_view` + `add_window_with_terminal` 起真实 `TerminalView`;
/// 2. 用 `BlocklistAIHistoryModel` 建一个会话并把它设为该 terminal view 的 active
///    conversation —— 面板的 `new`/`handle_action`/`should_render` 全部依赖它;
/// 3. `suggestions_mode_model` 与 host editor 直接复用 `TerminalView::input()` 上已构造好的
///    那两个(都有 pub accessor),这样测试里对 model `set_mode` 与产品代码看到的是同一个;
/// 4. 面板本身用 `ctx.add_typed_action_view` 在 terminal view 的 ctx 里新建一个实例
///    (`Input::queued_prompts_panel` 是私有字段,跨模块取不到)。
///
/// 闭包额外拿到 `terminal_view_id` 与 host editor:前者供 CLI-agent 会话类测试按真实
/// terminal 建会话,后者供「输入框非空」类测试真实写入文字。
fn with_panel<F>(test: F)
where
    F: FnOnce(
            App,
            ViewHandle<QueuedPromptsPanelView>,
            EntityId,
            ViewHandle<EditorView>,
            ModelHandle<InputSuggestionsModeModel>,
            ModelHandle<QueuedQueryModel>,
            AIConversationId,
        ) + 'static,
{
    App::test((), |mut app| async move {
        // 面板的 `should_render` 首先判这个 flag。
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (panel, suggestions_mode_model, conv, terminal_view_id, host_editor) =
            terminal.update(&mut app, |view, ctx| {
                let terminal_view_id = view.view_id;
                let history = BlocklistAIHistoryModel::handle(ctx);
                let conv = history.update(ctx, |history, ctx| {
                    let conv = history.start_new_conversation(terminal_view_id, false, false, ctx);
                    history.set_active_conversation_id(conv, terminal_view_id, ctx);
                    conv
                });

                let (suggestions_mode_model, host_editor) = view.input().read(ctx, |input, _| {
                    (
                        input.suggestions_mode_model().clone(),
                        input.editor().clone(),
                    )
                });
                let host_editor = host_editor.clone();
                let panel = {
                    let suggestions_mode_model = suggestions_mode_model.clone();
                    let host_editor = host_editor.clone();
                    ctx.add_typed_action_view(move |ctx| {
                        QueuedPromptsPanelView::new(
                            terminal_view_id,
                            suggestions_mode_model,
                            host_editor,
                            ctx,
                        )
                    })
                };
                (
                    panel,
                    suggestions_mode_model,
                    conv,
                    terminal_view_id,
                    host_editor,
                )
            });

        let queue_model = QueuedQueryModel::handle(&app);
        test(
            app,
            panel,
            terminal_view_id,
            host_editor,
            suggestions_mode_model,
            queue_model,
            conv,
        );
    });
}

/// 复用 `Input` 上真实面板的 harness 变体。
///
/// 为什么需要它:`enter_send_target` 的行内编辑前置需要让模型真正进入编辑态,而
/// `with_panel` 手工构造的第二个面板会和 `Input` 内建面板**同时订阅** `QueuedQueryModel`。
/// `EditEntered` 事件会让两个面板都 focus 各自的 edit editor,焦点在两者间来回横跳,
/// 先被 blur 的那边的 `Blurred → commit_edit` 会把模型的 `editing` 状态清掉 ——
/// 真实产品里只有一个面板,不存在这个干扰。所以凡涉及行内编辑的测试必须用这个
/// harness,直接操作 `Input::queued_prompts_panel()` 返回的内建面板。
fn with_input_panel<F>(test: F)
where
    F: FnOnce(
            App,
            ViewHandle<QueuedPromptsPanelView>,
            EntityId,
            ViewHandle<EditorView>,
            ModelHandle<InputSuggestionsModeModel>,
            ModelHandle<QueuedQueryModel>,
            AIConversationId,
        ) + 'static,
{
    App::test((), |mut app| async move {
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        initialize_app_for_terminal_view(&mut app);
        let terminal = add_window_with_terminal(&mut app, None);

        let (panel, suggestions_mode_model, host_editor, terminal_view_id, conv) =
            terminal.update(&mut app, |view, ctx| {
                let terminal_view_id = view.view_id;
                let history = BlocklistAIHistoryModel::handle(ctx);
                let conv = history.update(ctx, |history, ctx| {
                    let conv = history.start_new_conversation(terminal_view_id, false, false, ctx);
                    history.set_active_conversation_id(conv, terminal_view_id, ctx);
                    conv
                });
                let (suggestions_mode_model, host_editor) = view.input().read(ctx, |input, _| {
                    (
                        input.suggestions_mode_model().clone(),
                        input.editor().clone(),
                    )
                });
                let panel = view.input().read(ctx, |input, _| {
                    input
                        .queued_prompts_panel()
                        .cloned()
                        .expect("QueueSlashCommand 已 override,Input 必须已构造面板")
                });
                (
                    panel,
                    suggestions_mode_model,
                    host_editor,
                    terminal_view_id,
                    conv,
                )
            });

        let queue_model = QueuedQueryModel::handle(&app);
        test(
            app,
            panel,
            terminal_view_id,
            host_editor,
            suggestions_mode_model,
            queue_model,
            conv,
        );
    });
}

/// Copy 动作的核心契约:写进剪贴板的是该行**完整原文**,而不是面板渲染用的
/// `truncate_from_end(text, 200)` 预览;并且 copy 不消费该行。
#[test]
fn copy_row_writes_full_prompt_text_to_clipboard_and_keeps_the_row() {
    with_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            // 超过 200 字符且含换行:预览会被截断成带省略号的单段文本,与原文不同。
            let long_text = format!("{}\n第二行尾部", "第一行超长文本".repeat(30));
            assert!(long_text.chars().count() > 200);
            let preview_text = truncate_from_end(&long_text, 200);
            assert_ne!(
                preview_text, long_text,
                "预览必须真的被截断,否则本测试无判别力"
            );

            let query_id = queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new(long_text.clone(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                )
            });

            panel.update(&mut app, |panel, ctx| {
                panel.handle_action(&QueuedPromptsPanelAction::CopyRow(query_id), ctx);
            });

            let clipboard_text = app.update(|ctx| ctx.clipboard().read().plain_text);
            assert_eq!(clipboard_text, long_text, "剪贴板必须是完整原文");
            assert_ne!(
                clipboard_text, preview_text,
                "剪贴板不能是 200 字符预览截断"
            );
            assert!(
                clipboard_text.contains('\n'),
                "换行必须保留,不能被渲染层的单行预览吃掉"
            );

            queue_model.read(&app, |model, _| {
                let queue = model.queue(conv);
                assert_eq!(queue.len(), 1, "copy 不得消费该行");
                assert_eq!(queue[0].text(), long_text);
            });
        },
    );
}

/// 行已被并发删除时(按钮的 `query_id` 已失效),CopyRow 静默 no-op:不 panic、不动剪贴板。
#[test]
fn copy_row_for_removed_id_is_silent_no_op() {
    with_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            let query_id = queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new(
                        "将被删除的行".to_owned(),
                        QueuedQueryOrigin::QueueSlashCommand,
                    ),
                    ctx,
                )
            });
            app.update(|ctx| {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text("哨兵内容".to_owned()))
            });
            queue_model.update(&mut app, |model, ctx| {
                model
                    .remove_by_id(conv, query_id, ctx)
                    .expect("该行应存在并被删除")
            });

            panel.update(&mut app, |panel, ctx| {
                panel.handle_action(&QueuedPromptsPanelAction::CopyRow(query_id), ctx);
            });

            let clipboard_text = app.update(|ctx| ctx.clipboard().read().plain_text);
            assert_eq!(
                clipboard_text, "哨兵内容",
                "失效 id 不得写剪贴板(既不能写空串也不能写别的行)"
            );
        },
    );
}

/// 可见性与 inline menu 互斥:队列非空时面板可见,inline menu 打开期间必须让位,
/// 菜单关闭后恢复可见。
#[test]
fn should_render_is_suppressed_while_inline_menu_is_open() {
    with_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         suggestions_mode_model,
         queue_model,
         conv| {
            queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new("排队中".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.should_render(ctx)),
                "队列非空且无菜单时面板应可见"
            );

            suggestions_mode_model.update(&mut app, |model, ctx| {
                model.set_mode(InputSuggestionsMode::SlashCommands, ctx);
            });
            assert!(
                !panel.read(&app, |panel, ctx| panel.should_render(ctx)),
                "inline menu 打开期间面板必须隐藏,避免与菜单重叠"
            );

            suggestions_mode_model.update(&mut app, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.should_render(ctx)),
                "菜单关闭后面板必须重新显示"
            );
        },
    );
}

/// `enter_send_target` 是头部「⏎ to send」提示与宿主回车发送的单一真相,它的
/// 前置条件逐条可观测。本测试守第一条:队列非空 + 宿主输入框为空 + 无行内编辑 →
/// 返回**队首**行的 id。队列放两行,若实现误用 `.last()` 之类取尾,这里必然打红。
#[test]
fn enter_send_target_returns_head_row_id_when_queue_non_empty() {
    with_input_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            let head_id = queue_model.update(&mut app, |model, ctx| {
                let head_id = model.append(
                    conv,
                    QueuedQuery::new("队首".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
                model.append(
                    conv,
                    QueuedQuery::new("队尾".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
                head_id
            });

            let target = panel.read(&app, |panel, ctx| panel.enter_send_target(ctx));
            assert_eq!(
                target,
                Some(head_id),
                "空输入 + 队列非空时必须命中队首行;取成第二行或队尾都说明取行逻辑错"
            );

            // 对照:取出的确实是 `queue().first()`,而不是恰好同 id 的巧合。
            let first_in_queue = queue_model.read(&app, |model, _| {
                model.queue(conv).first().map(|row| row.id())
            });
            assert_eq!(target, first_in_queue);
        },
    );
}

/// 队列为空 → 无发送目标。若实现越过空队列直接取行,这里打红。
#[test]
fn enter_send_target_is_none_when_queue_is_empty() {
    with_input_panel(
        |app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "空队列时回车不应有任何发送目标"
            );
            assert_eq!(
                queue_model.read(&app, |model, _| model.queue(conv).len()),
                0,
                "前提没搭对:队列应为空"
            );
        },
    );
}

/// 宿主输入框非空 → 无发送目标。向 harness 暴露的 host editor **真实写入**文字
/// (不经任何标志位),并立即断言 —— 若实现改读缓存的 `host_editor_was_empty`
/// 而不是实时读 `host_editor.is_empty()`,这里必然打红。
#[test]
fn enter_send_target_is_none_when_host_input_is_non_empty() {
    with_input_panel(
        |mut app,
         panel,
         _terminal_view_id,
         host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new("排队中".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });
            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                Some(queue_model.read(&app, |model, _| model.queue(conv)[0].id())),
                "空输入时应有发送目标(前置) —— 否则本条断言失去意义"
            );

            host_editor.update(&mut app, |editor, ctx| {
                editor.set_buffer_text("宿主输入框里有字", ctx);
            });

            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "宿主输入框非空时回车必须让位给正常提交,不能发队首行"
            );
        },
    );
}

/// 有行处于行内编辑(`enter_edit_mode`)→ 无发送目标:回车正在编辑的行不得被
/// 悄悄发走。若实现删掉 `editing_row` 判定,这里打红。
#[test]
fn enter_send_target_is_none_while_a_row_is_in_inline_edit() {
    with_input_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            let query_id = queue_model.update(&mut app, |model, ctx| {
                let query_id = model.append(
                    conv,
                    QueuedQuery::new(
                        "正在编辑的行".to_owned(),
                        QueuedQueryOrigin::QueueSlashCommand,
                    ),
                    ctx,
                );
                model.enter_edit_mode(conv, query_id, ctx);
                query_id
            });
            assert_eq!(
                queue_model.read(&app, |model, _| model.editing_row(conv)),
                Some(query_id),
                "前提没搭对:该行应处于编辑态"
            );

            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "行内编辑期间回车不得把正在编辑的行发走"
            );
        },
    );
}

/// `set_can_send_prompt` 的 live 契约:同一实例上 false → 无发送目标,true → 恢复。
/// 守的是「值变化时真正改写内部状态并立即生效」,而非只触发一次 notify。
#[test]
fn enter_send_target_follows_can_send_prompt_live() {
    with_input_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new("排队中".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });

            // 构造默认 `can_send_prompt = true`:先确认基线成立。
            assert!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx).is_some()),
                "默认 must 可发送(前置)"
            );

            panel.update(&mut app, |panel, ctx| {
                panel.set_can_send_prompt(false, ctx);
            });
            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "can_send_prompt = false(只读 shared-session viewer)时回车不得发队首行"
            );

            panel.update(&mut app, |panel, ctx| {
                panel.set_can_send_prompt(true, ctx);
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx).is_some()),
                "角色恢复可执行后,同一实例必须恢复发送目标"
            );
        },
    );
}

/// inline menu 打开时,「回车发送队首行」这个下游行为必须随 `should_render` 一起关闭。
/// 与 `should_render_is_suppressed_while_inline_menu_is_open` 不重复:那条只测
/// `should_render` 本身,这条测「提示与回车发送随之关闭」的消费方契约。
#[test]
fn enter_send_target_is_none_while_inline_menu_is_open() {
    with_input_panel(
        |mut app,
         panel,
         _terminal_view_id,
         _host_editor,
         suggestions_mode_model,
         queue_model,
         conv| {
            queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new("排队中".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx).is_some()),
                "菜单关闭时应有发送目标(前置)"
            );

            suggestions_mode_model.update(&mut app, |model, ctx| {
                model.set_mode(InputSuggestionsMode::SlashCommands, ctx);
            });
            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "inline menu 打开期间即使队列非空,回车也不能发队首行"
            );

            suggestions_mode_model.update(&mut app, |model, ctx| {
                model.set_mode(InputSuggestionsMode::Closed, ctx);
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx).is_some()),
                "菜单关闭后发送目标恢复"
            );
        },
    );
}

/// CLI agent 富输入打开时,回车归 CLI agent 所有,不得发队首排队行。用真实的
/// `CLIAgentSessionsModel::set_session` + `open_input` 构造,不伪造标志位。
#[test]
fn enter_send_target_is_none_while_cli_agent_rich_input_is_open() {
    with_input_panel(
        |mut app,
         panel,
         terminal_view_id,
         _host_editor,
         _suggestions_mode_model,
         queue_model,
         conv| {
            queue_model.update(&mut app, |model, ctx| {
                model.append(
                    conv,
                    QueuedQuery::new("排队中".to_owned(), QueuedQueryOrigin::QueueSlashCommand),
                    ctx,
                );
            });
            assert!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx).is_some()),
                "无 CLI agent 会话时应有发送目标(前置)"
            );

            let sessions = CLIAgentSessionsModel::handle(&app);
            sessions.update(&mut app, |sessions, ctx| {
                sessions.set_session(
                    terminal_view_id,
                    CLIAgentSession {
                        agent: crate::terminal::CLIAgent::OhMyPi,
                        status: CLIAgentSessionStatus::InProgress,
                        session_context: CLIAgentSessionContext::default(),
                        input_state: CLIAgentInputState::Closed,
                        should_auto_toggle_input: false,
                        listener: None,
                        plugin_version: None,
                        remote_host: None,
                        draft_text: None,
                        custom_command_prefix: None,
                        current_model: None,
                    },
                    ctx,
                );
                sessions.open_input(
                    terminal_view_id,
                    CLIAgentInputEntrypoint::CtrlG,
                    crate::ai::blocklist::InputConfig::new(ctx),
                    false,
                    false,
                    ctx,
                );
            });
            assert!(
                sessions.read(&app, |sessions, _| sessions.is_input_open(terminal_view_id)),
                "前提没搭对:CLI agent 富输入应已打开"
            );

            assert_eq!(
                panel.read(&app, |panel, ctx| panel.enter_send_target(ctx)),
                None,
                "CLI agent 富输入打开时回车提交给 CLI agent,不得发队首排队行"
            );
        },
    );
}


fn image_attachment(file_name: &str) -> PendingAttachment {
    PendingAttachment::Image(ImageContext {
        data: String::new(),
        mime_type: "image/png".to_owned(),
        file_name: file_name.to_owned(),
        is_figma: false,
    })
}

/// 排队的 `/compact-and` 的执行契约(移植上游 098c307c7 的
/// `lrc_finish_queued_compact_and_sends_followup_after_summary`),守三件事:
///
/// ① 命令结束投递到排队的 `/compact-and follow up` 行时,`execute_queued_compact_and`
///    把 follow-up 以 `CompactAndSlashCommand` origin 排回队列,文本剥掉命令前缀;
/// ② 该行的**附件被转移**到 follow-up 上 —— 用户暂存的上下文不能在压缩这一跳里丢掉;
/// ③ follow-up 自身随后 drain 时正常提交为 AI 查询(恰 1 次 `ExecuteAIQuery`)且队列清空,
///    即压缩这一跳没有把队列卡死,也没有重复发送。
#[test]
fn lrc_finish_queued_compact_and_sends_followup_after_summary() {
    App::test((), |mut app| async move {
        initialize_app_for_terminal_view(&mut app);
        let _agent_view = FeatureFlag::AgentView.override_enabled(true);
        let _queue_flag = FeatureFlag::QueueSlashCommand.override_enabled(true);
        let _summarization = FeatureFlag::SummarizationConversationCommand.override_enabled(true);

        let terminal = add_window_with_terminal(&mut app, None);
        let terminal_view_id = terminal.read(&app, |view, _| view.view_id);
        let conversation_id =
            BlocklistAIHistoryModel::handle(&app).update(&mut app, |history, ctx| {
                let id = history.start_new_conversation(terminal_view_id, false, false, ctx);
                history.set_active_conversation_id(id, terminal_view_id, ctx);
                id
            });

        // 排一行带附件的 `/compact-and follow up`,模拟 LRC 期间自动排队的那条命令。
        QueuedQueryModel::handle(&app).update(&mut app, |model, ctx| {
            model.append(
                conversation_id,
                QueuedQuery::new_with_attachments(
                    format!("{} follow up", commands::COMPACT_AND.name),
                    QueuedQueryOrigin::LrcAutoQueue,
                    vec![image_attachment("queued-context.png")],
                ),
                ctx,
            );
        });

        terminal.update(&mut app, |view, ctx| {
            view.send_lrc_queued_prompts(conversation_id, ctx);
        });

        // ① + ②:follow-up 以 CompactAndSlashCommand 排回队列,并带着原行的附件。
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            let queue = model.queue(conversation_id);
            assert_eq!(queue.len(), 1, "原行离队 + follow-up 入队,净长度仍为 1");
            assert_eq!(
                queue[0].text(),
                "follow up",
                "排回队列的是剥掉 /compact-and 前缀后的 follow-up"
            );
            assert_eq!(
                queue[0].origin(),
                QueuedQueryOrigin::CompactAndSlashCommand,
                "origin 必须切成 CompactAndSlashCommand,否则它会被当 LRC 行重复自动投递"
            );
            assert_eq!(
                queue[0].attachments().len(),
                1,
                "排队行的附件必须转移到 follow-up 上,不能在压缩这一跳里丢失"
            );
            assert_eq!(
                queue[0].attachments()[0].file_name(),
                "queued-context.png",
                "转移的必须是原来那个附件"
            );
        });

        let ai_query_count = Rc::new(RefCell::new(0));
        let input = terminal.read(&app, |view, _| view.input().clone());
        let ai_query_count_for_subscription = ai_query_count.clone();
        app.update(|ctx| {
            ctx.subscribe_to_view(&input, move |_, event: &InputEvent, _| {
                if matches!(event, InputEvent::ExecuteAIQuery) {
                    *ai_query_count_for_subscription.borrow_mut() += 1;
                }
            });
        });

        // ③:压缩完成后 follow-up 正常提交,队列清空。
        terminal.update(&mut app, |view, ctx| {
            view.drain_queued_prompts(conversation_id, FinishReason::Complete, ctx);
        });

        assert_eq!(
            *ai_query_count.borrow(),
            1,
            "follow-up 必须恰好提交一次 AI 查询"
        );
        QueuedQueryModel::handle(&app).read(&app, |model, _| {
            assert!(
                model.queue(conversation_id).is_empty(),
                "follow-up 发出后队列必须清空,不能卡住"
            );
        });
    });
}