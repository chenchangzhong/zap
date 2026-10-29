use std::collections::HashSet;
use std::time::Duration;

use chrono::{DateTime, Local, TimeZone, Utc};
use itertools::Itertools;
use warpui::{App, EntityId};

use crate::{
    ai::{
        agent::{
            api::ServerConversationToken, conversation::AIConversationId, AIAgentExchange,
            AIAgentExchangeId, AIAgentInput, AIAgentOutputStatus, FinishedAIAgentOutput, Shared,
            UserQueryMode,
        },
        ambient_agents::AmbientAgentTaskId,
        blocklist::{controller::RequestInput, ResponseStreamId},
        byop_readiness::{RepairRecord, RepairSource, RepairState, RepairStateStatus, ToolCallKey},
        llms::LLMId,
    },
    input_suggestions::HistoryInputSuggestion,
    persistence::{
        model::{
            AgentConversation, AgentConversationData, AgentConversationRecord,
            PersistedAutoexecuteMode,
        },
        ModelEvent,
    },
    terminal::model::session::SessionId,
    test_util::settings::initialize_settings_for_tests,
    GlobalResourceHandles, GlobalResourceHandlesProvider,
};
use warp_multi_agent_api as api;

use super::{
    convert_persisted_conversation_to_ai_conversation_with_metadata, AIConversationMetadata,
    AIQueryHistoryOutputStatus, BlocklistAIHistoryEvent, BlocklistAIHistoryModel, PersistedAIInput,
    PersistedAIInputType,
};

fn initialize_history_model_test_app(app: &mut App) {
    initialize_settings_for_tests(app);
    let global_resource_handles = GlobalResourceHandles::mock(app);
    app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
}

/// Helper function to create a PersistedAIInput for testing
fn create_persisted_query(
    query_text: &str,
    conversation_id: AIConversationId,
    start_time: DateTime<Local>,
) -> PersistedAIInput {
    PersistedAIInput {
        exchange_id: AIAgentExchangeId::new(),
        conversation_id,
        start_ts: start_time,
        inputs: vec![PersistedAIInputType::Query {
            text: query_text.to_string(),
            context: Default::default(),
            referenced_attachments: Default::default(),
        }],
        output_status: AIQueryHistoryOutputStatus::Completed,
        working_directory: None,
        model_id: LLMId::from("test-model"),
        coding_model_id: LLMId::from("test-coding-model"),
    }
}

/// Helper function to create an AIAgentExchange for testing
fn create_exchange_with_query(
    query_text: &str,
    start_time: DateTime<Local>,
    working_directory: Option<String>,
) -> AIAgentExchange {
    AIAgentExchange {
        id: AIAgentExchangeId::new(),
        input: vec![AIAgentInput::UserQuery {
            query: query_text.to_string(),
            context: Default::default(),
            static_query_type: None,
            referenced_attachments: Default::default(),
            user_query_mode: UserQueryMode::default(),
            running_command: None,
            intended_agent: None,
        }],
        output_status: AIAgentOutputStatus::Finished {
            finished_output: FinishedAIAgentOutput::Success {
                output: Shared::new(Default::default()),
            },
        },
        added_message_ids: HashSet::new(),
        start_time,
        finish_time: None,
        time_to_first_token_ms: None,
        working_directory,
        model_id: LLMId::from("test-model"),
        request_cost: None,
        coding_model_id: LLMId::from("test-coding-model"),
        cli_agent_model_id: LLMId::from("test-cli-agent-model"),
        computer_use_model_id: LLMId::from("test-computer-use-model"),
        response_initiator: None,
    }
}

fn byop_test_task(task_id: &str, messages: Vec<api::Message>) -> api::Task {
    api::Task {
        id: task_id.to_owned(),
        messages,
        dependencies: None,
        description: String::new(),
        summary: String::new(),
        server_data: String::new(),
    }
}

fn empty_agent_conversation_data_for_test() -> AgentConversationData {
    AgentConversationData {
        server_conversation_token: None,
        conversation_usage_metadata: None,
        reverted_action_ids: None,
        forked_from_server_conversation_token: None,
        artifacts_json: None,
        parent_agent_id: None,
        agent_name: None,
        parent_conversation_id: None,
        run_id: None,
        autoexecute_override: None,
        last_event_sequence: None,
        compaction_state_json: None,
        byop_repair_state_json: None,
        cli_subagent_block_snapshots_json: None,
    }
}

fn timestamp_from_local_time(time: DateTime<Local>) -> prost_types::Timestamp {
    prost_types::Timestamp {
        seconds: time.timestamp(),
        nanos: time.timestamp_subsec_nanos() as i32,
    }
}

fn persisted_agent_conversation_for_test(
    conversation_id: AIConversationId,
    last_modified_at: chrono::NaiveDateTime,
    messages: Vec<api::Message>,
) -> AgentConversation {
    AgentConversation {
        conversation: AgentConversationRecord {
            id: 1,
            conversation_id: conversation_id.to_string(),
            conversation_data: serde_json::to_string(&empty_agent_conversation_data_for_test())
                .expect("test conversation data should serialize"),
            last_modified_at,
        },
        tasks: vec![byop_test_task("root-task", messages)],
    }
}

fn byop_user_query_message(
    message_id: &str,
    request_id: &str,
    query: &str,
    current_time: Option<DateTime<Local>>,
) -> api::Message {
    api::Message {
        id: message_id.to_owned(),
        task_id: "root-task".to_owned(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::UserQuery(api::message::UserQuery {
            query: query.to_owned(),
            context: current_time.map(|time| api::InputContext {
                current_time: Some(timestamp_from_local_time(time)),
                ..Default::default()
            }),
            referenced_attachments: Default::default(),
            mode: None,
            intended_agent: Default::default(),
        })),
        request_id: request_id.to_owned(),
        timestamp: None,
    }
}

fn byop_tool_call_message(task_id: &str, message_id: &str, call_id: &str) -> api::Message {
    api::Message {
        id: message_id.to_owned(),
        task_id: task_id.to_owned(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: call_id.to_owned(),
            tool: None,
        })),
        request_id: "request-1".to_owned(),
        timestamp: None,
    }
}

fn byop_tool_result_message(task_id: &str, message_id: &str, call_id: &str) -> api::Message {
    api::Message {
        id: message_id.to_owned(),
        task_id: task_id.to_owned(),
        server_message_data: "{}".to_owned(),
        citations: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: call_id.to_owned(),
                context: None,
                result: None,
            },
        )),
        request_id: "request-1".to_owned(),
        timestamp: None,
    }
}

#[test]
fn persisted_conversation_uses_record_timestamp_when_restored_messages_have_no_time() {
    let conversation_id = AIConversationId::new();
    let fallback_time = Utc
        .with_ymd_and_hms(2026, 7, 2, 7, 13, 11)
        .unwrap()
        .naive_utc();
    let persisted_conversation = persisted_agent_conversation_for_test(
        conversation_id,
        fallback_time,
        vec![byop_user_query_message(
            "user-query-1",
            "request-1",
            "restore me",
            None,
        )],
    );

    let restored =
        convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
            .expect("persisted conversation should restore");

    let restored_time = restored
        .last_modified_at()
        .expect("restored conversation should have an exchange timestamp");
    assert_eq!(restored_time, Local.from_utc_datetime(&fallback_time));
    assert_ne!(restored_time, Local.timestamp_opt(0, 0).single().unwrap());
}

#[test]
fn persisted_conversation_keeps_restored_message_time_when_present() {
    let conversation_id = AIConversationId::new();
    let fallback_time = Utc
        .with_ymd_and_hms(2026, 7, 2, 7, 13, 11)
        .unwrap()
        .naive_utc();
    let message_time = Local.with_ymd_and_hms(2026, 7, 2, 16, 42, 0).unwrap();
    let persisted_conversation = persisted_agent_conversation_for_test(
        conversation_id,
        fallback_time,
        vec![byop_user_query_message(
            "user-query-1",
            "request-1",
            "restore me",
            Some(message_time),
        )],
    );

    let restored =
        convert_persisted_conversation_to_ai_conversation_with_metadata(persisted_conversation)
            .expect("persisted conversation should restore");

    assert_eq!(restored.last_modified_at(), Some(message_time));
}

#[test]
fn byop_fork_repair_records_cover_tool_results_omitted_by_fork() {
    let source_tasks = vec![byop_test_task(
        "source-task",
        vec![
            byop_tool_call_message("source-task", "assistant-1", "call-a"),
            byop_tool_call_message("source-task", "assistant-2", "call-b"),
            byop_tool_result_message("source-task", "result-a", "call-a"),
            byop_tool_result_message("source-task", "result-b", "call-b"),
        ],
    )];
    let forked_tasks = vec![byop_test_task(
        "fork-task",
        vec![
            byop_tool_call_message("fork-task", "assistant-1", "call-a"),
            byop_tool_call_message("fork-task", "assistant-2", "call-b"),
            byop_tool_result_message("fork-task", "result-a", "call-a"),
        ],
    )];

    let records = super::collect_byop_fork_repair_records(
        &source_tasks,
        &forked_tasks,
        &RepairStateStatus::default(),
        Some("exchange-1".to_owned()),
    );

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, RepairSource::ForkedHistory);
    assert_eq!(
        records[0].key,
        ToolCallKey::new("fork-task", "assistant-1", "call-b")
    );
    assert_eq!(records[0].exchange_id.as_deref(), Some("exchange-1"));
}

#[test]
fn byop_fork_repair_records_match_duplicate_call_ids_by_assistant_message() {
    let source_tasks = vec![byop_test_task(
        "source-task",
        vec![
            byop_tool_call_message("source-task", "assistant-1", "call-a"),
            byop_tool_result_message("source-task", "result-a-1", "call-a"),
            byop_tool_call_message("source-task", "assistant-2", "call-a"),
            byop_tool_result_message("source-task", "result-a-2", "call-a"),
        ],
    )];
    let forked_tasks = vec![byop_test_task(
        "fork-task",
        vec![
            byop_tool_call_message("fork-task", "assistant-1", "call-a"),
            byop_tool_result_message("fork-task", "result-a-1", "call-a"),
            byop_tool_call_message("fork-task", "assistant-2", "call-a"),
        ],
    )];

    let records = super::collect_byop_fork_repair_records(
        &source_tasks,
        &forked_tasks,
        &RepairStateStatus::default(),
        None,
    );

    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].key,
        ToolCallKey::new("fork-task", "assistant-2", "call-a")
    );
}

#[test]
fn byop_fork_repair_records_do_not_convert_orphan_results() {
    let source_tasks = vec![byop_test_task(
        "source-task",
        vec![byop_tool_result_message(
            "source-task",
            "orphan-result",
            "call-a",
        )],
    )];
    let forked_tasks = vec![byop_test_task(
        "fork-task",
        vec![byop_tool_result_message(
            "fork-task",
            "orphan-result",
            "call-a",
        )],
    )];

    let records = super::collect_byop_fork_repair_records(
        &source_tasks,
        &forked_tasks,
        &RepairStateStatus::default(),
        None,
    );

    assert!(records.is_empty());
}

#[test]
fn byop_fork_repair_records_do_not_infer_unexplained_missing_results() {
    let source_tasks = vec![byop_test_task(
        "source-task",
        vec![byop_tool_call_message(
            "source-task",
            "assistant-1",
            "call-a",
        )],
    )];
    let forked_tasks = vec![byop_test_task(
        "fork-task",
        vec![byop_tool_call_message("fork-task", "assistant-1", "call-a")],
    )];

    let records = super::collect_byop_fork_repair_records(
        &source_tasks,
        &forked_tasks,
        &RepairStateStatus::default(),
        None,
    );

    assert!(records.is_empty());
}

#[test]
fn byop_fork_repair_records_translate_existing_repair_keys() {
    let source_tasks = vec![byop_test_task(
        "source-task",
        vec![byop_tool_call_message(
            "source-task",
            "assistant-1",
            "call-a",
        )],
    )];
    let forked_tasks = vec![byop_test_task(
        "fork-task",
        vec![byop_tool_call_message("fork-task", "assistant-1", "call-a")],
    )];
    let source_record = RepairRecord::new(
        RepairSource::RestoredLegacyHistory,
        ToolCallKey::new("source-task", "assistant-1", "call-a"),
    );

    let records = super::collect_byop_fork_repair_records(
        &source_tasks,
        &forked_tasks,
        &RepairStateStatus::Valid(RepairState::new(vec![source_record])),
        None,
    );

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].source, RepairSource::RestoredLegacyHistory);
    assert_eq!(
        records[0].key,
        ToolCallKey::new("fork-task", "assistant-1", "call-a")
    );
}

/// 带真实 tool 的 assistant tool call 消息:`tool: None` 的 tool call 不会生成
/// client output message,恢复后不属于任何 exchange,无法用来复现 rewind 截断。
fn byop_run_shell_tool_call_message(message_id: &str, call_id: &str) -> api::Message {
    use api::message::tool_call::run_shell_command::WaitUntilCompleteValue;

    api::Message {
        id: message_id.to_owned(),
        task_id: "root-task".to_owned(),
        server_message_data: String::new(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: call_id.to_owned(),
            tool: Some(api::message::tool_call::Tool::RunShellCommand(
                api::message::tool_call::RunShellCommand {
                    command: "echo hi".to_owned(),
                    is_read_only: true,
                    uses_pager: false,
                    is_risky: false,
                    citations: vec![],
                    wait_until_complete_value: Some(WaitUntilCompleteValue::WaitUntilComplete(
                        true,
                    )),
                    risk_category: 0,
                },
            )),
        })),
        request_id: "request-2".to_owned(),
        timestamp: None,
    }
}

/// 构造 rewind 用例的会话:三轮 request,第二轮里有一个带真实 tool 的 assistant tool call,
/// 第三轮用来把 rewind 点放到 tool call 之后。
fn byop_rewind_fixture_conversation() -> crate::ai::agent::conversation::AIConversation {
    crate::ai::agent::conversation::AIConversation::new_restored(
        AIConversationId::new(),
        vec![byop_test_task(
            "root-task",
            vec![
                byop_user_query_message("user-query-1", "request-1", "first turn", None),
                byop_user_query_message("user-query-2", "request-2", "second turn", None),
                byop_run_shell_tool_call_message("assistant-1", "call-a"),
                byop_user_query_message("user-query-3", "request-3", "third turn", None),
            ],
        )],
        Some(empty_agent_conversation_data_for_test()),
    )
    .expect("conversation should restore")
}

/// 恢复会话 → 模拟 controller preflight 追加真实 tool result → rewind 到第 `truncate_index`
/// 个 exchange。返回会话 id 与 root task id,供调用方断言保留下来的消息。
fn restore_and_rewind_byop_fixture(
    app: &mut warpui::App,
    history_model: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    truncate_index: usize,
) -> AIConversationId {
    let conversation = byop_rewind_fixture_conversation();
    let conversation_id = conversation.id();
    let task_id = conversation.get_root_task_id().clone();
    let exchange_ids = conversation
        .root_task_exchanges()
        .map(|exchange| exchange.id)
        .collect::<Vec<_>>();
    assert_eq!(exchange_ids.len(), 3, "expected three exchanges");

    history_model.update(app, |model, ctx| {
        model.restore_conversations(EntityId::new(), vec![conversation], ctx);
    });

    // preflight 把可用结果直接追加进 source messages(不经 response stream)。
    history_model.update(app, |model, ctx| {
        model
            .conversation_mut(&conversation_id)
            .expect("conversation should exist")
            .append_byop_preflight_messages_to_task(
                task_id,
                vec![byop_tool_result_message(
                    "root-task",
                    "preflight-result",
                    "call-a",
                )],
                ctx,
            )
            .expect("preflight result should be appended");
    });

    history_model.update(app, |model, ctx| {
        model
            .truncate_conversation_from_exchange(conversation_id, exchange_ids[truncate_index], ctx)
            .expect("truncation should succeed");
    });

    conversation_id
}

fn retained_message_ids(
    app: &warpui::App,
    history_model: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    conversation_id: AIConversationId,
) -> Vec<String> {
    history_model.read(app, |model, _| {
        model
            .conversation(&conversation_id)
            .expect("conversation should still exist")
            .get_root_task()
            .expect("root task should be retained")
            .messages()
            .map(|message| message.id.clone())
            .collect()
    })
}

/// rewind 按 exchange 截断 history:BYOP preflight 追加的 tool result 归属其 tool call 所在的
/// exchange,因此 rewind 到该 exchange 时两者一起消失(否则会留下"只有 result 没有 call"的孤立
/// 消息,使后续请求被 readiness 永久阻断 `OrphanToolResult`);rewind 到更晚的 exchange 时两者
/// 都必须保留。
#[test]
fn byop_preflight_tool_result_follows_its_exchange_on_rewind() {
    App::test((), |mut app| async move {
        // preflight 追加消息要求 sqlite sender 可用,因此不用默认的 mock 句柄。
        initialize_settings_for_tests(&mut app);
        let (sender, _receiver) = std::sync::mpsc::sync_channel(16);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // rewind 到第三轮(在 tool call 之后):tool call 与它的 result 都必须保留。
        let kept_conversation = restore_and_rewind_byop_fixture(&mut app, &history_model, 2);
        let kept_message_ids = retained_message_ids(&app, &history_model, kept_conversation);
        assert!(kept_message_ids.contains(&"assistant-1".to_owned()));
        assert!(
            kept_message_ids.contains(&"preflight-result".to_owned()),
            "rewind point after the tool call must not drop the preflight result"
        );
        assert!(!kept_message_ids.contains(&"user-query-3".to_owned()));

        // rewind 到第二轮(tool call 所在 exchange):两者必须一起被移除。
        let removed_conversation = restore_and_rewind_byop_fixture(&mut app, &history_model, 1);
        let removed_message_ids = retained_message_ids(&app, &history_model, removed_conversation);
        assert!(removed_message_ids.contains(&"user-query-1".to_owned()));
        assert!(
            !removed_message_ids.contains(&"assistant-1".to_owned()),
            "truncated tool call must be removed"
        );
        assert!(
            !removed_message_ids.contains(&"preflight-result".to_owned()),
            "preflight tool result must be removed together with its tool call, \
             otherwise it becomes an orphan result that blocks the next BYOP request"
        );
    });
}

#[test]
fn test_ai_queries_for_terminal_view_up_arrow_history() {
    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);

        let now = Local::now();
        let terminal_view_id = EntityId::new();
        let current_session_id = SessionId::from(0);
        let all_live_session_ids = HashSet::from([current_session_id]);

        // Create initial persisted queries
        let conversation_id_1 = AIConversationId::new();
        let conversation_id_2 = AIConversationId::new();

        let persisted_queries = vec![
            create_persisted_query(
                "restored query 1",
                conversation_id_1,
                now - chrono::Duration::seconds(10),
            ),
            create_persisted_query(
                "restored query 2",
                conversation_id_2,
                now - chrono::Duration::seconds(5),
            ),
        ];

        // Create history model with persisted queries as a singleton
        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(persisted_queries, &[]));

        // Helper function to get and sort AI queries using the same logic as Input
        let get_sorted_queries = |model: &BlocklistAIHistoryModel| -> Vec<String> {
            model
                .all_ai_queries(Some(terminal_view_id))
                .map(|query| HistoryInputSuggestion::AIQuery { entry: query })
                .sorted_by(|a, b| a.cmp(b, Some(current_session_id), &all_live_session_ids))
                .map(|suggestion| suggestion.text().to_string())
                .collect()
        };

        // Test initial state with just persisted queries
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 2);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");

        // Start a new conversation and add "live query 1"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("live query 1", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test state after adding live query 1
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 3);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");

        // Start another new conversation and add "live query 2"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query(
                "live query 2",
                now + chrono::Duration::seconds(1),
                None,
            );
            let stream_id = ResponseStreamId::new_for_test();
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test state after adding live query 2
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 4);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");

        // Clear the blocklist
        history_model.update(&mut app, |history_model, ctx| {
            history_model.clear_conversations_in_terminal_view(terminal_view_id, ctx);
        });

        // Test state after clearing - should remain the same
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 4);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");

        // Start a new conversation after clearing and add "new query after clear"
        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let stream_id = ResponseStreamId::new_for_test();
            let exchange = create_exchange_with_query(
                "new query after clear",
                now + chrono::Duration::seconds(2),
                None,
            );
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id,
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Test final state
        let queries = history_model.read(&app, |model, _| get_sorted_queries(model));
        assert_eq!(queries.len(), 5);
        assert_eq!(queries[0], "restored query 1");
        assert_eq!(queries[1], "restored query 2");
        assert_eq!(queries[2], "live query 1");
        assert_eq!(queries[3], "live query 2");
        assert_eq!(queries[4], "new query after clear");
    });
}

#[test]
fn test_transcript_viewer_terminal_view_is_not_marked_historical() {
    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);

        let now = Local::now();
        let terminal_view_id = EntityId::new();

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();

            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };

            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        history_model.update(&mut app, |history_model, _| {
            history_model.mark_terminal_view_as_conversation_transcript_viewer(terminal_view_id);
            history_model.mark_conversations_historical_for_terminal_view(terminal_view_id);
        });

        let historical_count = history_model.read(&app, |history_model, _| {
            history_model.get_local_conversations_metadata().count()
        });
        assert_eq!(historical_count, 0);
    });
}

#[test]
fn test_ambient_agent_conversations_excluded_from_list_but_accessible_by_id() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let regular_id = AIConversationId::new();
        let ambient_id = AIConversationId::new();

        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        history_model.update(&mut app, |model, _| {
            let regular_metadata = AIConversationMetadata {
                id: regular_id,
                title: "Regular Conversation".to_string(),
                initial_query: String::new(),
                last_modified_at: Utc::now().naive_utc(),
                initial_working_directory: None,
                credits_spent: Some(5.0),
                server_conversation_token: Some(ServerConversationToken::new(
                    "token-regular".to_string(),
                )),
                is_restorable_locally: false,
                artifacts: Vec::new(),
                ambient_agent_task_id: None,
            };
            model
                .all_conversations_metadata
                .insert(regular_id, regular_metadata);

            let ambient_metadata = AIConversationMetadata {
                id: ambient_id,
                title: "Ambient Conversation".to_string(),
                initial_query: String::new(),
                last_modified_at: Utc::now().naive_utc(),
                initial_working_directory: None,
                credits_spent: Some(3.0),
                server_conversation_token: Some(ServerConversationToken::new(
                    "token-ambient".to_string(),
                )),
                is_restorable_locally: false,
                artifacts: Vec::new(),
                ambient_agent_task_id: Some(ambient_task_id),
            };
            model
                .all_conversations_metadata
                .insert(ambient_id, ambient_metadata);
        });

        history_model.read(&app, |model, _| {
            // get_local_conversations_metadata should exclude the ambient conversation
            let listed: Vec<&AIConversationMetadata> =
                model.get_local_conversations_metadata().collect();
            assert_eq!(listed.len(), 1);
            assert_eq!(listed[0].id, regular_id);

            // get_conversation_metadata should return both by ID
            assert!(model.get_conversation_metadata(&regular_id).is_some());
            assert!(model.get_conversation_metadata(&ambient_id).is_some());
            assert_eq!(
                model.get_conversation_metadata(&ambient_id).unwrap().title,
                "Ambient Conversation"
            );
        });
    });
}

#[test]
fn test_initialize_historical_conversations_indexes_child_conversations() {
    use crate::persistence::model::{AgentConversation, AgentConversationRecord};
    use chrono::NaiveDateTime;

    App::test((), |app| async move {
        let parent_id = AIConversationId::new();
        let child_id = AIConversationId::new();

        // Build a child AgentConversation whose conversation_data contains
        // a parent_conversation_id.  The child needs no tasks because
        // initialize_historical_conversations returns None (filters it out)
        // before inspecting tasks.
        let child_conversation_data = format!(r#"{{"parent_conversation_id":"{parent_id}"}}"#);

        let conversations = vec![AgentConversation {
            conversation: AgentConversationRecord {
                id: 1,
                conversation_id: child_id.to_string(),
                conversation_data: child_conversation_data,
                last_modified_at: NaiveDateTime::default(),
            },
            tasks: vec![],
        }];

        let history_model =
            app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &conversations));

        history_model.read(&app, |model, _| {
            // The child conversation should be indexed under its parent.
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);

            // The child should NOT appear in navigable conversation metadata.
            let metadata_ids: Vec<AIConversationId> = model
                .get_local_conversations_metadata()
                .map(|m| m.id)
                .collect();
            assert!(
                !metadata_ids.contains(&child_id),
                "child conversation should be excluded from metadata"
            );
        });
    });
}

#[test]
fn test_set_parent_for_conversation_populates_index() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        // Create parent and child conversations via start_new_conversation.
        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        // Set the parent-child relationship.
        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_id, parent_id);
        });

        // Verify the index is populated and the conversation has the parent set.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
            assert_eq!(model.child_conversations_of(parent_id).len(), 1);
            assert_eq!(model.child_conversations_of(parent_id)[0].id(), child_id);
            assert!(
                model
                    .conversation(&child_id)
                    .unwrap()
                    .parent_conversation_id()
                    == Some(parent_id)
            );
        });
    });
}

#[test]
fn test_set_parent_for_conversation_dedup() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });
        let child_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        // Set the same parent-child relationship twice.
        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_id, parent_id);
            model.set_parent_for_conversation(child_id, parent_id);
        });

        // Should have exactly one entry, not two.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_set_parent_multiple_children() {
    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });
        let child_a = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });
        let child_b = history_model.update(&mut app, |model, ctx| {
            model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |model, _| {
            model.set_parent_for_conversation(child_a, parent_id);
            model.set_parent_for_conversation(child_b, parent_id);
        });

        history_model.read(&app, |model, _| {
            let children = model.child_conversation_ids_of(&parent_id);
            assert_eq!(children.len(), 2);
            assert!(children.contains(&child_a));
            assert!(children.contains(&child_b));
            assert_eq!(model.child_conversations_of(parent_id).len(), 2);
        });
    });
}

#[test]
fn test_child_conversation_ids_of_unknown_parent() {
    App::test((), |app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let unknown_id = AIConversationId::new();

        history_model.read(&app, |model, _| {
            assert!(model.child_conversation_ids_of(&unknown_id).is_empty());
            assert!(model.child_conversations_of(unknown_id).is_empty());
        });
    });
}

#[test]
fn test_restore_conversations_maintains_children_by_parent() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversationId::new();
        let mut child_conv = AIConversation::new(false);
        child_conv.set_parent_conversation_id(parent_id);
        let child_id = child_conv.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv], ctx);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_restore_conversations_dedup_children_by_parent() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let parent_id = AIConversationId::new();
        let mut child_conv_a = AIConversation::new(false);
        child_conv_a.set_parent_conversation_id(parent_id);
        let child_id = child_conv_a.id();
        let child_conv_b = child_conv_a.clone();

        // Restore the same child conversation twice (simulates close + reopen).
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv_a], ctx);
        });
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![child_conv_b], ctx);
        });

        // Should have exactly one entry, not two.
        history_model.read(&app, |model, _| {
            assert_eq!(model.child_conversation_ids_of(&parent_id), &[child_id]);
        });
    });
}

#[test]
fn test_all_cleared_conversations_includes_terminal_view_id() {
    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);

        let now = Local::now();
        let terminal_view_id = EntityId::new();

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();

            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };

            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.clear_conversations_in_terminal_view(terminal_view_id, ctx);
        });

        let has_cleared = history_model.read(&app, |history_model, _| {
            history_model
                .all_cleared_conversations()
                .iter()
                .any(|(id, convo)| *id == terminal_view_id && convo.id() == conversation_id)
        });

        assert!(has_cleared);
    });
}

#[test]
fn test_toggle_autoexecute_override_persists_updated_conversation_state() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.toggle_autoexecute_override(&conversation_id, terminal_view_id, ctx);
        });

        let event = receiver.recv_timeout(Duration::from_secs(1)).unwrap();

        let ModelEvent::UpdateMultiAgentConversation {
            conversation_id: persisted_conversation_id,
            conversation_data,
            ..
        } = event
        else {
            panic!("expected UpdateMultiAgentConversation event");
        };

        assert_eq!(persisted_conversation_id, conversation_id.to_string());
        assert_eq!(
            conversation_data.autoexecute_override,
            Some(PersistedAutoexecuteMode::RunToCompletion)
        );
    });
}

#[test]
fn test_find_by_token_after_restore_conversations() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let mut conversation = AIConversation::new(false);
        conversation.set_server_conversation_token("restored-token".to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        let token = ServerConversationToken::new("restored-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_returns_none_after_remove_conversation() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        // `delete_conversation` publishes persistence events via
        // `GlobalResourceHandlesProvider`, so we need a mock sender wired up.
        let (sender, _receiver) = std::sync::mpsc::sync_channel(2);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false);
        conversation.set_server_conversation_token("removable-token".to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let token = ServerConversationToken::new("removable-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
                "token should resolve before removal",
            );
        });

        history_model.update(&mut app, |model, ctx| {
            model.delete_conversation(conversation_id, None, ctx);
        });

        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                None,
                "reverse index must be cleared when the conversation is removed",
            );
        });
    });
}

#[test]
fn test_find_by_token_returns_none_after_reset() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let mut conversation = AIConversation::new(false);
        conversation.set_server_conversation_token("reset-token".to_string());
        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(EntityId::new(), vec![conversation], ctx);
        });

        let token = ServerConversationToken::new("reset-token".to_string());

        history_model.read(&app, |model, _| {
            assert!(model.find_conversation_id_by_server_token(&token).is_some());
        });

        history_model.update(&mut app, |model, _| {
            model.reset();
        });

        history_model.read(&app, |model, _| {
            assert_eq!(model.find_conversation_id_by_server_token(&token), None);
        });
    });
}

#[test]
fn test_find_by_token_after_initialize_output_for_response_stream() {
    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);

        let now = Local::now();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            history_model.start_new_conversation(terminal_view_id, false, false, ctx)
        });

        // Prime a pending request so StreamInit can install the token.
        let stream_id = ResponseStreamId::new_for_test();
        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    stream_id.clone(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        let server_token_str = "init-token".to_string();
        history_model.update(&mut app, |history_model, ctx| {
            history_model.initialize_output_for_response_stream(
                &stream_id,
                conversation_id,
                terminal_view_id,
                warp_multi_agent_api::response_event::StreamInit {
                    request_id: String::new(),
                    conversation_id: server_token_str.clone(),
                    run_id: String::new(),
                },
                ctx,
            );
        });

        let token = ServerConversationToken::new(server_token_str);
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_assign_run_id_for_conversation() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let id = history_model.start_new_conversation(terminal_view_id, false, false, ctx);
            // Seed a token so assign_run_id has one to forward into the index.
            history_model
                .conversation_mut(&id)
                .expect("conversation should exist")
                .set_server_conversation_token("run-id-token".to_string());
            id
        });

        history_model.update(&mut app, |history_model, ctx| {
            history_model.assign_run_id_for_conversation(
                conversation_id,
                "run-1".to_string(),
                None,
                terminal_view_id,
                ctx,
            );
        });

        let token = ServerConversationToken::new("run-id-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_insert_forked_conversation_from_tasks() {
    use crate::persistence::model::AgentConversationData;

    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let forked_conversation_id = AIConversationId::new();
        let conversation_data = AgentConversationData {
            server_conversation_token: Some("forked-token".to_string()),
            conversation_usage_metadata: None,
            reverted_action_ids: None,
            forked_from_server_conversation_token: None,
            artifacts_json: None,
            parent_agent_id: None,
            agent_name: None,
            parent_conversation_id: None,
            run_id: None,
            autoexecute_override: None,
            last_event_sequence: None,
            compaction_state_json: None,
            byop_repair_state_json: None,
            cli_subagent_block_snapshots_json: None,
        };
        let tasks = vec![warp_multi_agent_api::Task {
            id: "root-task".to_string(),
            messages: vec![],
            dependencies: None,
            description: String::new(),
            summary: String::new(),
            server_data: String::new(),
        }];

        history_model.update(&mut app, |model, _| {
            model
                .insert_forked_conversation_from_tasks(
                    forked_conversation_id,
                    tasks,
                    conversation_data,
                )
                .expect("forked conversation should insert");
        });

        let token = ServerConversationToken::new("forked-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(forked_conversation_id),
            );
        });
    });
}

#[test]
fn test_find_by_token_after_mark_conversations_historical_for_terminal_view() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);

        let now = Local::now();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        // Needs a real exchange to pass `conversation_would_render_in_blocklist`.
        let mut conversation = AIConversation::new(false);
        conversation.set_server_conversation_token("historical-token".to_string());
        let conversation_id = conversation.id();

        history_model.update(&mut app, |model, ctx| {
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
        });

        history_model.update(&mut app, |history_model, ctx| {
            let exchange = create_exchange_with_query("historical query", now, None);
            let task_id = history_model
                .conversation(&conversation_id)
                .unwrap()
                .get_root_task_id()
                .clone();
            let request_input = RequestInput {
                conversation_id,
                input_messages: std::collections::HashMap::from([(task_id, exchange.input)]),
                working_directory: exchange.working_directory,
                model_id: exchange.model_id,
                coding_model_id: exchange.coding_model_id,
                cli_agent_model_id: exchange.cli_agent_model_id,
                computer_use_model_id: exchange.computer_use_model_id,
                shared_session_response_initiator: exchange.response_initiator,
                request_start_ts: exchange.start_time,
                supported_tools_override: None,
            };
            history_model
                .update_conversation_for_new_request_input(
                    request_input,
                    ResponseStreamId::new_for_test(),
                    terminal_view_id,
                    ctx,
                )
                .unwrap();
        });

        // Sanity check: token resolves after restore_conversations.
        let token = ServerConversationToken::new("historical-token".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
        });

        history_model.update(&mut app, |model, _| {
            model.mark_conversations_historical_for_terminal_view(terminal_view_id);
        });

        // Token still resolves via the metadata-side index entry.
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&token),
                Some(conversation_id),
            );
            assert!(
                model.get_conversation_metadata(&conversation_id).is_some(),
                "metadata entry must exist so the reverse index is not dangling",
            );
        });
    });
}

#[test]
fn test_set_server_conversation_token_rebinds_reverse_index() {
    App::test((), |mut app| async move {
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let conversation_id = history_model.update(&mut app, |history_model, ctx| {
            let id = history_model.start_new_conversation(terminal_view_id, false, false, ctx);
            history_model.set_server_conversation_token_for_conversation(id, "old".to_string());
            id
        });

        let old_token = ServerConversationToken::new("old".to_string());
        history_model.read(&app, |model, _| {
            assert_eq!(
                model.find_conversation_id_by_server_token(&old_token),
                Some(conversation_id),
            );
        });

        history_model.update(&mut app, |history_model, _| {
            history_model
                .set_server_conversation_token_for_conversation(conversation_id, "new".to_string());
        });

        let new_token = ServerConversationToken::new("new".to_string());
        history_model.read(&app, |model, _| {
            // Stale lookups must not resolve to the rebound conversation.
            assert_eq!(model.find_conversation_id_by_server_token(&old_token), None);
            assert_eq!(
                model.find_conversation_id_by_server_token(&new_token),
                Some(conversation_id),
            );
        });
    });
}

/// Builds an `AIConversationMetadata` entry with an optional ambient agent task id.
fn create_conversation_metadata(
    id: AIConversationId,
    ambient_agent_task_id: Option<AmbientAgentTaskId>,
) -> AIConversationMetadata {
    AIConversationMetadata {
        id,
        title: "Test Conversation".to_string(),
        initial_query: String::new(),
        last_modified_at: Utc::now().naive_utc(),
        initial_working_directory: None,
        credits_spent: Some(5.0),
        server_conversation_token: Some(ServerConversationToken::new(format!("token-{id}"))),
        is_restorable_locally: false,
        artifacts: Vec::new(),
        ambient_agent_task_id,
    }
}

/// Collects the `DeletedConversation` ids emitted on the model.
fn emitted_deleted_conversation_ids(
    emitted_events: &std::cell::RefCell<Vec<BlocklistAIHistoryEvent>>,
) -> Vec<AIConversationId> {
    emitted_events
        .borrow()
        .iter()
        .filter_map(|event| match event {
            BlocklistAIHistoryEvent::DeletedConversation {
                conversation_id, ..
            } => Some(*conversation_id),
            _ => None,
        })
        .collect()
}

/// Receives exactly `expected_count` messages from the mock model event channel,
/// then asserts no further messages are pending. Returns the received events.
fn collect_model_events(
    receiver: &std::sync::mpsc::Receiver<ModelEvent>,
    expected_count: usize,
) -> Vec<ModelEvent> {
    let mut events = Vec::new();
    for _ in 0..expected_count {
        events.push(
            receiver
                .recv_timeout(Duration::from_secs(1))
                .expect("expected a model event"),
        );
    }
    match receiver.try_recv() {
        Err(std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected) => {
        }
        Ok(extra) => panic!("unexpected extra model event: {extra:?}"),
    }
    events
}

#[test]
fn test_delete_all_conversations_dedups_conversation_present_in_both_maps() {
    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let emitted_events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let emitted_events = emitted_events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&history_model, move |_, event, _| {
                    emitted_events.borrow_mut().push(event.clone());
                });
            });
        }

        // One conversation that exists BOTH as a live conversation (restored,
        // completed) and as a historical metadata entry.
        let conversation_id = history_model.update(&mut app, |model, ctx| {
            let conversation = AIConversation::new(false);
            let id = conversation.id();
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model
                .conversations_by_id
                .get_mut(&id)
                .unwrap()
                .update_status(ConversationStatus::Success, terminal_view_id, ctx);
            model
                .all_conversations_metadata
                .insert(id, create_conversation_metadata(id, None));
            id
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(skipped, 0, "no conversations should be skipped");

        // The overlapping conversation must be deleted exactly once: one
        // DeleteAIConversation event and one DeleteMultiAgentConversations
        // batch containing the id a single time.
        let events = collect_model_events(&receiver, 2);
        let delete_ai_events: Vec<&ModelEvent> = events
            .iter()
            .filter(|event| matches!(event, ModelEvent::DeleteAIConversation { .. }))
            .collect();
        assert_eq!(
            delete_ai_events.len(),
            1,
            "dedup failed: duplicate delete events"
        );
        assert!(matches!(
            delete_ai_events[0],
            ModelEvent::DeleteAIConversation {
                conversation_id: deleted_id,
            } if deleted_id == &conversation_id.to_string()
        ));
        let multi_agent_events: Vec<&ModelEvent> = events
            .iter()
            .filter(|event| matches!(event, ModelEvent::DeleteMultiAgentConversations { .. }))
            .collect();
        assert_eq!(multi_agent_events.len(), 1);
        assert!(matches!(
            multi_agent_events[0],
            ModelEvent::DeleteMultiAgentConversations { conversation_ids }
                if conversation_ids == &vec![conversation_id.to_string()]
        ));

        // Exactly one DeletedConversation event for the deduped id.
        let deleted_ids = emitted_deleted_conversation_ids(&emitted_events);
        assert_eq!(deleted_ids, vec![conversation_id]);

        // The owning terminal view must be resolved and cleaned up: the live
        // conversation is removed from the per-view index, and a
        // RemoveConversation event fires with the resolved view.
        let remove_events: Vec<EntityId> = emitted_events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BlocklistAIHistoryEvent::RemoveConversation {
                    terminal_view_id, ..
                } => Some(*terminal_view_id),
                _ => None,
            })
            .collect();
        assert_eq!(remove_events, vec![terminal_view_id]);

        history_model.read(&app, |model, _| {
            assert!(model.conversations_by_id.is_empty());
            assert!(model.all_conversations_metadata.is_empty());
            assert!(
                model
                    .live_conversation_ids_for_terminal_view
                    .get(&terminal_view_id)
                    .is_none_or(|ids| ids.is_empty()),
                "live conversation index must be cleaned up for the owning terminal view"
            );
        });
    });
}

#[test]
fn test_delete_all_conversations_skips_ambient_conversations() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let regular_id = AIConversationId::new();
        let ambient_id = AIConversationId::new();
        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        history_model.update(&mut app, |model, _| {
            model
                .all_conversations_metadata
                .insert(regular_id, create_conversation_metadata(regular_id, None));
            model.all_conversations_metadata.insert(
                ambient_id,
                create_conversation_metadata(ambient_id, Some(ambient_task_id)),
            );
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(skipped, 1, "the ambient conversation must count as skipped");

        // Only the regular conversation is deleted.
        let events = collect_model_events(&receiver, 2);
        let delete_ai_events: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::DeleteAIConversation { conversation_id } => {
                    Some(conversation_id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(delete_ai_events, vec![regular_id.to_string()]);

        history_model.read(&app, |model, _| {
            assert!(
                model.get_conversation_metadata(&ambient_id).is_some(),
                "ambient conversation must not be deleted"
            );
            assert!(
                model.get_conversation_metadata(&regular_id).is_none(),
                "regular conversation must be deleted"
            );
        });
    });
}

#[test]
fn test_delete_all_conversations_skips_live_ambient_conversation_without_metadata() {
    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();
        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        // A live, completed conversation owned by an ambient agent task.
        // `restore_conversations` never writes `all_conversations_metadata`, so
        // this conversation has NO metadata entry: only the live task_id check
        // can identify it as ambient.
        let conversation_id = history_model.update(&mut app, |model, ctx| {
            let conversation = AIConversation::new(false);
            let id = conversation.id();
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model
                .conversations_by_id
                .get_mut(&id)
                .unwrap()
                .update_status(ConversationStatus::Success, terminal_view_id, ctx);
            model
                .conversations_by_id
                .get_mut(&id)
                .unwrap()
                .set_task_id(ambient_task_id);
            id
        });

        history_model.read(&app, |model, _| {
            assert!(
                !model
                    .all_conversations_metadata
                    .contains_key(&conversation_id),
                "precondition: restore path must not write all_conversations_metadata"
            );
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(
            skipped, 1,
            "the live ambient conversation must count as skipped"
        );

        // Nothing is deleted: no DeleteAIConversation event and no
        // DeleteMultiAgentConversations batch payload at all.
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected)
        ));

        history_model.read(&app, |model, _| {
            let conversation = model
                .conversations_by_id
                .get(&conversation_id)
                .expect("live ambient conversation must remain in conversations_by_id");
            assert_eq!(conversation.task_id(), Some(ambient_task_id));
        });
    });
}

#[test]
fn test_delete_all_conversations_skips_live_ambient_conversation_and_deletes_companion() {
    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();
        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        let companion_id = AIConversationId::new();
        let ambient_id = history_model.update(&mut app, |model, ctx| {
            // 1 deletable companion: historical-only, non-ambient metadata.
            model.all_conversations_metadata.insert(
                companion_id,
                create_conversation_metadata(companion_id, None),
            );

            // 1 live ambient conversation: restored (no metadata), completed,
            // owned by an ambient agent task.
            let conversation = AIConversation::new(false);
            let id = conversation.id();
            model.restore_conversations(terminal_view_id, vec![conversation], ctx);
            model
                .conversations_by_id
                .get_mut(&id)
                .unwrap()
                .update_status(ConversationStatus::Success, terminal_view_id, ctx);
            model
                .conversations_by_id
                .get_mut(&id)
                .unwrap()
                .set_task_id(ambient_task_id);
            id
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(
            skipped, 1,
            "only the live ambient conversation must be skipped"
        );

        // The companion is deleted: one DeleteAIConversation event and one
        // DeleteMultiAgentConversations batch containing ONLY the companion id.
        let events = collect_model_events(&receiver, 2);
        let delete_ai_events: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::DeleteAIConversation { conversation_id } => {
                    Some(conversation_id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(delete_ai_events, vec![companion_id.to_string()]);
        let multi_agent_events: Vec<&ModelEvent> = events
            .iter()
            .filter(|event| matches!(event, ModelEvent::DeleteMultiAgentConversations { .. }))
            .collect();
        assert_eq!(multi_agent_events.len(), 1);
        assert!(matches!(
            multi_agent_events[0],
            ModelEvent::DeleteMultiAgentConversations { conversation_ids }
                if conversation_ids == &vec![companion_id.to_string()]
        ));

        history_model.read(&app, |model, _| {
            assert!(
                model.conversations_by_id.contains_key(&ambient_id),
                "live ambient conversation must remain live"
            );
            assert!(
                model.get_conversation_metadata(&companion_id).is_none(),
                "companion conversation must be deleted"
            );
        });
    });
}

#[test]
fn test_delete_all_conversations_skips_in_progress_conversations() {
    use crate::ai::agent::conversation::{AIConversation, ConversationStatus};

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let in_progress_id = history_model.update(&mut app, |model, ctx| {
            let in_progress = AIConversation::new(false); // status is InProgress
            let id = in_progress.id();
            model.restore_conversations(terminal_view_id, vec![in_progress], ctx);

            // A separate, completed conversation that should be deleted.
            let completed = AIConversation::new(false);
            let completed_id = completed.id();
            model.restore_conversations(terminal_view_id, vec![completed], ctx);
            model
                .conversations_by_id
                .get_mut(&completed_id)
                .unwrap()
                .update_status(ConversationStatus::Success, terminal_view_id, ctx);
            model.all_conversations_metadata.insert(
                completed_id,
                create_conversation_metadata(completed_id, None),
            );
            id
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(
            skipped, 1,
            "the in-progress conversation must count as skipped"
        );

        let events = collect_model_events(&receiver, 2);
        let delete_ai_events: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::DeleteAIConversation { conversation_id } => {
                    Some(conversation_id.clone())
                }
                _ => None,
            })
            .collect();
        assert!(
            !delete_ai_events
                .iter()
                .any(|id| *id == in_progress_id.to_string()),
            "in-progress conversation must not be deleted"
        );
        assert_eq!(
            delete_ai_events.len(),
            1,
            "only the completed conversation is deleted"
        );

        history_model.read(&app, |model, _| {
            assert!(
                model.conversations_by_id.contains_key(&in_progress_id),
                "in-progress conversation must remain live"
            );
        });
    });
}

#[test]
fn test_delete_all_conversations_returns_skipped_count_for_mixed_state() {
    use crate::ai::agent::conversation::AIConversation;

    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let terminal_view_id = EntityId::new();

        let deletable_id = AIConversationId::new();
        let ambient_id = AIConversationId::new();
        let ambient_task_id: AmbientAgentTaskId = uuid::Uuid::new_v4().to_string().parse().unwrap();

        let in_progress_id = history_model.update(&mut app, |model, ctx| {
            // 1 deletable: historical-only, non-ambient metadata.
            model.all_conversations_metadata.insert(
                deletable_id,
                create_conversation_metadata(deletable_id, None),
            );
            // 1 ambient: metadata owned by an ambient agent task.
            model.all_conversations_metadata.insert(
                ambient_id,
                create_conversation_metadata(ambient_id, Some(ambient_task_id)),
            );
            // 1 in-progress: live conversation that has not finished.
            let in_progress = AIConversation::new(false);
            let id = in_progress.id();
            model.restore_conversations(terminal_view_id, vec![in_progress], ctx);
            id
        });

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(
            skipped, 2,
            "ambient + in-progress conversations must be skipped"
        );

        // Only the deletable conversation produces a DeleteAIConversation event.
        let events = collect_model_events(&receiver, 2);
        let delete_ai_events: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                ModelEvent::DeleteAIConversation { conversation_id } => {
                    Some(conversation_id.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(delete_ai_events, vec![deletable_id.to_string()]);

        history_model.read(&app, |model, _| {
            assert!(model.get_conversation_metadata(&deletable_id).is_none());
            assert!(model.get_conversation_metadata(&ambient_id).is_some());
            assert!(model.conversations_by_id.contains_key(&in_progress_id));
        });
    });
}

#[test]
fn test_delete_all_conversations_empty_model_returns_zero() {
    App::test((), |mut app| async move {
        initialize_settings_for_tests(&mut app);

        let (sender, receiver) = std::sync::mpsc::sync_channel(8);
        let mut global_resource_handles = GlobalResourceHandles::mock(&mut app);
        global_resource_handles.model_event_sender = Some(sender);
        app.add_singleton_model(|_| GlobalResourceHandlesProvider::new(global_resource_handles));

        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));

        let emitted_events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let emitted_events = emitted_events.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&history_model, move |_, event, _| {
                    emitted_events.borrow_mut().push(event.clone());
                });
            });
        }

        let skipped =
            history_model.update(&mut app, |model, ctx| model.delete_all_conversations(ctx));
        assert_eq!(skipped, 0);

        // No persistence events and no emitted deletion events.
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty | std::sync::mpsc::TryRecvError::Disconnected)
        ));
        assert!(emitted_deleted_conversation_ids(&emitted_events).is_empty());
    });
}

// ---------------------------------------------------------------------------
// Oz 子代理(模型乙):派发事件 + 启动收敛
// ---------------------------------------------------------------------------

/// 建一个父会话并挂上一个在飞的 response stream(否则 `AddMessagesToTask`
/// 会因为 `added_exchanges_by_response` 里没有条目而报 `NoPendingRequest`)。
/// 返回 `(parent_id, task_id, stream_id)`。
fn prime_parent_conversation_for_subagent_test(
    app: &mut App,
    history_model: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    terminal_view_id: EntityId,
) -> (
    AIConversationId,
    crate::ai::agent::task::TaskId,
    ResponseStreamId,
) {
    let parent_id = history_model.update(app, |model, ctx| {
        model.start_new_conversation(terminal_view_id, false, false, ctx)
    });
    let stream_id = ResponseStreamId::new_for_test();
    let task_id = history_model.read(app, |model, _| {
        model
            .conversation(&parent_id)
            .expect("parent exists")
            .get_root_task_id()
            .clone()
    });
    history_model.update(app, |model, ctx| {
        let exchange = create_exchange_with_query("parent query", Local::now(), None);
        let request_input = RequestInput {
            conversation_id: parent_id,
            input_messages: std::collections::HashMap::from([(task_id.clone(), exchange.input)]),
            working_directory: exchange.working_directory,
            model_id: exchange.model_id,
            coding_model_id: exchange.coding_model_id,
            cli_agent_model_id: exchange.cli_agent_model_id,
            computer_use_model_id: exchange.computer_use_model_id,
            shared_session_response_initiator: exchange.response_initiator,
            request_start_ts: exchange.start_time,
            supported_tools_override: None,
        };
        model
            .update_conversation_for_new_request_input(
                request_input,
                stream_id.clone(),
                terminal_view_id,
                ctx,
            )
            .expect("request input registered");
    });
    // StreamInit 会把乐观 root task 升级成可写的 server task(`TaskNotInitialized`
    // 在此之前会拒掉 `AddMessagesToTask`)。
    history_model.update(app, |model, ctx| {
        model.initialize_output_for_response_stream(
            &stream_id,
            parent_id,
            terminal_view_id,
            warp_multi_agent_api::response_event::StreamInit {
                request_id: String::new(),
                conversation_id: "subagent-test-server-token".to_owned(),
                run_id: String::new(),
            },
            ctx,
        );
    });
    // 乐观 root task 只有收到 `CreateTask` 才会变成 server task。
    history_model.update(app, |model, ctx| {
        model
            .apply_client_actions(
                &stream_id,
                vec![api::ClientAction {
                    action: Some(api::client_action::Action::CreateTask(
                        api::client_action::CreateTask {
                            task: Some(api::Task {
                                id: task_id.to_string(),
                                description: String::new(),
                                dependencies: None,
                                messages: vec![],
                                summary: String::new(),
                                server_data: String::new(),
                            }),
                        },
                    )),
                }],
                parent_id,
                terminal_view_id,
                ctx,
            )
            .expect("root task upgraded to server task");
    });
    (parent_id, task_id, stream_id)
}

fn apply_messages_to_parent(
    app: &mut App,
    history_model: &warpui::ModelHandle<BlocklistAIHistoryModel>,
    conversation_id: AIConversationId,
    stream_id: &ResponseStreamId,
    terminal_view_id: EntityId,
    task_id: &crate::ai::agent::task::TaskId,
    messages: Vec<api::Message>,
) {
    let stream_id = stream_id.clone();
    let task_id = task_id.clone();
    history_model.update(app, |model, ctx| {
        model
            .apply_client_actions(
                &stream_id,
                vec![api::ClientAction {
                    action: Some(api::client_action::Action::AddMessagesToTask(
                        api::client_action::AddMessagesToTask {
                            task_id: task_id.to_string(),
                            messages,
                        },
                    )),
                }],
                conversation_id,
                terminal_view_id,
                ctx,
            )
            .expect("client action applies");
    });
}

fn subagent_carrier_message(
    task_id: &crate::ai::agent::task::TaskId,
    message_id: &str,
    tool_call_id: &str,
    args_json: &str,
) -> api::Message {
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    api::Message {
        id: message_id.to_owned(),
        task_id: task_id.to_string(),
        server_message_data: format!("{}\n{}", subagent_tool::TOOL_NAME, args_json),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: tool_call_id.to_owned(),
            tool: None,
        })),
        request_id: "req-1".to_owned(),
        timestamp: None,
    }
}

fn subagent_status_frame_message(
    task_id: &crate::ai::agent::task::TaskId,
    message_id: &str,
    tool_call_id: &str,
    payload: String,
) -> api::Message {
    api::Message {
        id: message_id.to_owned(),
        task_id: task_id.to_string(),
        server_message_data: payload,
        citations: vec![],
        message: Some(api::message::Message::ToolCallResult(
            api::message::ToolCallResult {
                tool_call_id: tool_call_id.to_owned(),
                context: None,
                result: None,
            },
        )),
        request_id: "req-1".to_owned(),
        timestamp: None,
    }
}

/// 父侧 `subagent` carrier + running 帧应用后,必须发出恰好一条
/// `SubagentRequested`,且载荷与 running 帧里预生成的子会话 id 完全一致。
#[test]
fn subagent_dispatch_emits_requested_event_from_running_frame() {
    use crate::ai::agent::task::TaskId;
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_id = AIConversationId::new();
        let messages = vec![
            subagent_carrier_message(
                &task_id,
                "carrier-1",
                "call-1",
                r#"{"tasks":[{"prompt":"investigate A","name":"A"}]}"#,
            ),
            subagent_status_frame_message(
                &task_id,
                "running-frame-1",
                "call-1",
                subagent_tool::running_frame_json(&[subagent_tool::SubagentFrameChild::running(
                    child_id.to_string(),
                    "A".to_owned(),
                )]),
            ),
        ];

        let emitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let emitted = emitted.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&history_model, move |_, event, _| {
                    emitted.borrow_mut().push(event.clone());
                });
            });
        }

        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            messages,
        );

        let requested: Vec<_> = emitted
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BlocklistAIHistoryEvent::SubagentRequested {
                    parent_conversation_id,
                    parent_task_id,
                    tool_call_id,
                    result_message_id,
                    children,
                    ..
                } => Some((
                    *parent_conversation_id,
                    parent_task_id.clone(),
                    tool_call_id.clone(),
                    result_message_id.clone(),
                    children.clone(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(requested.len(), 1, "应恰好发出一次派发事件");
        let (event_parent, event_task, tool_call_id, result_message_id, children) =
            requested.into_iter().next().unwrap();
        assert_eq!(event_parent, parent_id);
        assert_eq!(event_task, TaskId::new(task_id.to_string()));
        assert_eq!(tool_call_id, "call-1");
        assert_eq!(result_message_id, "running-frame-1");
        assert_eq!(children.len(), 1);
        assert_eq!(
            children[0].child_conversation_id, child_id,
            "子会话 id 必须取自 running 帧,渲染层才能从消息体还原"
        );
        assert_eq!(children[0].prompt, "investigate A");
        assert_eq!(children[0].name, "A");
        assert_eq!(children[0].model, None);
    });
}

/// 多子代理批量契约:一次调用派发 N 个子代理时,事件里必须带齐**全部**子代理
/// (顺序与 args/frame 一致),且它们共享同一条 `result_message_id`。
///
/// 这是「一次派发完、卡片渲染 N 行」的数据前提:少带一个子代理就会漏建子会话;
/// `result_message_id` 若被拆成 N 条,就会撞上 D3 的 `DuplicateToolResults`。
#[test]
fn subagent_dispatch_event_carries_every_child_of_one_call() {
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_a = AIConversationId::new();
        let child_b = AIConversationId::new();
        let messages = vec![
            subagent_carrier_message(
                &task_id,
                "carrier-1",
                "call-1",
                r#"{"tasks":[{"prompt":"A 的任务","name":"A"},{"prompt":"B 的任务","name":"B"}]}"#,
            ),
            subagent_status_frame_message(
                &task_id,
                "frame-1",
                "call-1",
                subagent_tool::running_frame_json(&[
                    subagent_tool::SubagentFrameChild::running(child_a.to_string(), "A".to_owned()),
                    subagent_tool::SubagentFrameChild::running(child_b.to_string(), "B".to_owned()),
                ]),
            ),
        ];

        let emitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let emitted = emitted.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&history_model, move |_, event, _| {
                    emitted.borrow_mut().push(event.clone());
                });
            });
        }

        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            messages,
        );

        let requested: Vec<_> = emitted
            .borrow()
            .iter()
            .filter_map(|event| match event {
                BlocklistAIHistoryEvent::SubagentRequested {
                    result_message_id,
                    children,
                    ..
                } => Some((result_message_id.clone(), children.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(requested.len(), 1, "一次调用只应发出一个派发事件");
        let (result_message_id, children) = &requested[0];
        assert_eq!(
            result_message_id, "frame-1",
            "N 个子代理共享同一条结果帧(否则会撞 DuplicateToolResults)"
        );
        assert_eq!(children.len(), 2, "必须带齐全部子代理,实际: {children:?}");
        assert_eq!(children[0].child_conversation_id, child_a);
        assert_eq!(children[0].prompt, "A 的任务");
        assert_eq!(children[0].name, "A");
        assert_eq!(children[1].child_conversation_id, child_b);
        assert_eq!(children[1].prompt, "B 的任务");
        assert_eq!(children[1].name, "B");
    });
}

/// 多子代理的结果**逐个累积**进同一条帧:一个子代理结束时只改它自己那一项,
/// 另一个的结果与状态必须原样保留。
///
/// 这是整条链路上最容易写错的一步(把「子代理粒度」误写成「工具调用粒度」会让
/// 后完成的覆盖先完成的),而它只在真实并发下才暴露,所以在这里锁住。
#[test]
fn subagent_frame_accumulates_each_child_outcome_without_clobbering_siblings() {
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    let child_a = AIConversationId::new();
    let child_b = AIConversationId::new();
    let running = subagent_tool::running_frame_json(&[
        subagent_tool::SubagentFrameChild::running(child_a.to_string(), "A".to_owned()),
        subagent_tool::SubagentFrameChild::running(child_b.to_string(), "B".to_owned()),
    ]);

    // A 先完成:帧里 A 变 completed,并带上 A 的结果;B 必须还是 running。
    let after_a = subagent_tool::parse_status_frame(&running).expect("可解析");
    let children = after_a
        .children
        .into_iter()
        .map(|child| {
            if child.child_conversation_id == child_a.to_string() {
                subagent_tool::SubagentFrameChild::finished(
                    child.child_conversation_id,
                    child.name,
                    subagent_tool::STATUS_COMPLETED,
                    "A 的结论".to_owned(),
                )
            } else {
                child
            }
        })
        .collect::<Vec<_>>();
    let payload_after_a = subagent_tool::completed_frame_json(
        subagent_tool::aggregate_child_status(
            &children
                .iter()
                .map(|c| c.status.clone())
                .collect::<Vec<_>>(),
        ),
        &children,
    );

    let frame = subagent_tool::parse_status_frame(&payload_after_a).expect("可解析");
    assert_eq!(frame.children[0].status, subagent_tool::STATUS_COMPLETED);
    assert_eq!(frame.children[0].result.as_deref(), Some("A 的结论"));
    assert_eq!(
        frame.children[1].status,
        subagent_tool::STATUS_RUNNING,
        "B 还没结束,不得被顺手标成完成"
    );
    assert_eq!(frame.children[1].result, None);
    assert_eq!(
        frame.status,
        subagent_tool::STATUS_RUNNING,
        "还有子代理在跑,帧级状态必须是 running"
    );

    // B 随后完成:在 A 的结果之上累积,B 自己的结果也进来,A 的不被抹掉。
    let children = frame
        .children
        .into_iter()
        .map(|child| {
            if child.child_conversation_id == child_b.to_string() {
                subagent_tool::SubagentFrameChild::finished(
                    child.child_conversation_id,
                    child.name,
                    subagent_tool::STATUS_COMPLETED,
                    "B 的结论".to_owned(),
                )
            } else {
                child
            }
        })
        .collect::<Vec<_>>();
    let payload_after_b = subagent_tool::completed_frame_json(
        subagent_tool::aggregate_child_status(
            &children
                .iter()
                .map(|c| c.status.clone())
                .collect::<Vec<_>>(),
        ),
        &children,
    );

    let frame = subagent_tool::parse_status_frame(&payload_after_b).expect("可解析");
    assert_eq!(
        frame.children[0].result.as_deref(),
        Some("A 的结论"),
        "先完成的子代理结果不得被后完成的覆盖"
    );
    assert_eq!(frame.children[1].result.as_deref(), Some("B 的结论"));
    assert_eq!(
        frame.status,
        subagent_tool::STATUS_COMPLETED,
        "全部完成才是 completed"
    );
}

/// 非 subagent 的 carrier(其它本地拦截工具 / 解析失败兜底)不得触发派发事件。
#[test]
fn subagent_dispatch_event_is_not_emitted_for_other_carriers() {
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_id = AIConversationId::new();
        // webfetch 风格的 carrier + 一条普通 running 帧:既不是 subagent 工具,
        // 也没有可识别的子会话 id。
        let messages = vec![
            api::Message {
                id: "carrier-web".to_owned(),
                task_id: task_id.to_string(),
                server_message_data: "webfetch\n{\"url\":\"https://example.com\"}".to_owned(),
                citations: vec![],
                message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                    tool_call_id: "call-web".to_owned(),
                    tool: None,
                })),
                request_id: "req-1".to_owned(),
                timestamp: None,
            },
            subagent_status_frame_message(
                &task_id,
                "frame-web",
                "call-web",
                subagent_tool::running_frame_json(&[subagent_tool::SubagentFrameChild::running(
                    child_id.to_string(),
                    "A".to_owned(),
                )]),
            ),
        ];

        let emitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        {
            let emitted = emitted.clone();
            app.update(|ctx| {
                ctx.subscribe_to_model(&history_model, move |_, event, _| {
                    emitted.borrow_mut().push(event.clone());
                });
            });
        }

        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            messages,
        );

        assert!(
            !emitted
                .borrow()
                .iter()
                .any(|event| matches!(event, BlocklistAIHistoryEvent::SubagentRequested { .. })),
            "非 subagent carrier 不得触发派发"
        );
    });
}

/// 启动收敛:停在 running 的父侧帧被改成 error,子会话的 InProgress 也被收敛。
#[test]
fn reconcile_interrupted_subagent_dispatch_marks_frame_and_child_failed() {
    use crate::ai::agent::conversation::ConversationStatus;
    use crate::ai::agent::task::TaskId;
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_conversation_id = AIConversationId::new();
        history_model.update(&mut app, |model, ctx| {
            model.start_new_child_conversation(
                terminal_view_id,
                "A".to_owned(),
                parent_id,
                child_conversation_id,
                false,
                ctx,
            );
        });

        // 父侧落下一条仍停在 running 的结果帧。
        let frame_message_id = "running-frame-1";
        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            vec![subagent_status_frame_message(
                &task_id,
                frame_message_id,
                "call-1",
                subagent_tool::running_frame_json(&[subagent_tool::SubagentFrameChild::running(
                    child_conversation_id.to_string(),
                    "A".to_owned(),
                )]),
            )],
        );

        history_model.update(&mut app, |model, ctx| {
            model.reconcile_interrupted_subagent_dispatches(terminal_view_id, ctx);
        });

        history_model.read(&app, |model, _| {
            let parent = model.conversation(&parent_id).expect("parent exists");
            let message = parent
                .get_task(&TaskId::new(task_id.to_string()))
                .expect("task exists")
                .messages()
                .find(|message| message.id == frame_message_id)
                .expect("frame still present");
            let frame = subagent_tool::parse_status_frame(&message.server_message_data)
                .expect("frame still parses");
            assert_eq!(
                frame.status,
                subagent_tool::STATUS_ERROR,
                "重启后仍在 running 的派发必须被收敛成失败"
            );
            assert_eq!(
                model
                    .conversation(&child_conversation_id)
                    .expect("child exists")
                    .status(),
                &ConversationStatus::Error,
                "子会话不能一直停在 InProgress"
            );
        });
    });
}

/// 收敛必须幂等:已终态的帧再跑一次不会被改写。
#[test]
fn reconcile_interrupted_subagent_dispatch_is_idempotent_for_terminal_frames() {
    use crate::ai::agent::task::TaskId;
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_id = AIConversationId::new();
        let done_payload = subagent_tool::completed_frame_json(
            subagent_tool::STATUS_COMPLETED,
            &[subagent_tool::SubagentFrameChild::finished(
                child_id.to_string(),
                "A".to_owned(),
                subagent_tool::STATUS_COMPLETED,
                "done".to_owned(),
            )],
        );
        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            vec![subagent_status_frame_message(
                &task_id,
                "done-frame",
                "call-1",
                done_payload.clone(),
            )],
        );

        history_model.update(&mut app, |model, ctx| {
            model.reconcile_interrupted_subagent_dispatches(terminal_view_id, ctx);
        });

        history_model.read(&app, |model, _| {
            let message = model
                .conversation(&parent_id)
                .unwrap()
                .get_task(&TaskId::new(task_id.to_string()))
                .unwrap()
                .messages()
                .find(|message| message.id == "done-frame")
                .unwrap();
            assert_eq!(
                message.server_message_data, done_payload,
                "已完成的帧不得被收敛改写"
            );
        });
    });
}

/// Oz 子代理(模型乙)回归:父侧「carrier ToolCall + 状态帧 ToolCallResult」在
/// **从持久化恢复**之后,必须仍然产出 `AIAgentOutputMessageType::SubagentRun`
/// (子代理派发记录,供 SDK/导出消费;底部状态卡另从子会话索引取数据)。
///
/// 这条用例来自实测故障:恢复路径里 `ToolCallResult` 原本被无条件当成 exchange
/// 输入消费(`convert_conversation.rs` 对该 arm 恒返回 `true`),于是表示层根本
/// 不跑 ⇒ 重启后卡片消失、还会被兜底合成为一条假的「已取消」结果。
#[test]
fn restored_subagent_status_frame_yields_subagent_run_output() {
    use crate::ai::agent::AIAgentOutputMessageType;
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    let child_conversation_id = AIConversationId::new();

    let messages = vec![
        byop_user_query_message("msg-user", "req-1", "用子代理查 A", None),
        api::Message {
            id: "msg-carrier".to_owned(),
            task_id: "root-task".to_owned(),
            server_message_data: format!(
                "{}\n{}",
                subagent_tool::TOOL_NAME,
                r#"{"tasks":[{"prompt":"查 A","name":"A"}]}"#
            ),
            citations: vec![],
            message: Some(api::message::Message::ToolCall(api::message::ToolCall {
                tool_call_id: "call-1".to_owned(),
                tool: None,
            })),
            request_id: "req-1".to_owned(),
            timestamp: None,
        },
        api::Message {
            id: "msg-frame".to_owned(),
            task_id: "root-task".to_owned(),
            server_message_data: subagent_tool::running_frame_json(&[
                subagent_tool::SubagentFrameChild::running(
                    child_conversation_id.to_string(),
                    "A".to_owned(),
                ),
            ]),
            citations: vec![],
            message: Some(api::message::Message::ToolCallResult(
                api::message::ToolCallResult {
                    tool_call_id: "call-1".to_owned(),
                    context: None,
                    result: None,
                },
            )),
            request_id: "req-1".to_owned(),
            timestamp: None,
        },
    ];

    let persisted = persisted_agent_conversation_for_test(
        AIConversationId::new(),
        chrono::NaiveDateTime::default(),
        messages,
    );

    let restored = convert_persisted_conversation_to_ai_conversation_with_metadata(persisted)
        .expect("persisted conversation should restore");

    let runs: Vec<(String, String)> = restored
        .all_exchanges()
        .into_iter()
        .filter_map(|exchange| exchange.output_status.output())
        .flat_map(|output| output.get().messages.clone())
        .filter_map(|message| match message.message {
            AIAgentOutputMessageType::SubagentRun(call) => Some((
                call.runs[0].name.clone(),
                call.runs[0].conversation_id.to_string(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        runs.len(),
        1,
        "恢复后必须恰好有一条子代理派发记录,实际: {runs:?}"
    );
    assert_eq!(runs[0].0, "A");
    assert_eq!(
        runs[0].1,
        child_conversation_id.to_string(),
        "派发记录必须指回 running 帧里预生成的子会话 id"
    );
}

/// Oz 子代理回归:**会话被激活时**必须收敛它名下过期的派发。
///
/// 实测故障:启动收敛只覆盖「启动时已 live」的父会话,用户**事后从历史打开**的
/// 会话走不到那里 ⇒ 卡片一直显示「运行中」,直到点开子会话才被按需修正。
/// 因此收敛挂在 `set_active_conversation_id` 上。
#[test]
fn activating_conversation_reconciles_its_stale_subagent_dispatches() {
    use crate::ai::agent::conversation::ConversationStatus;
    use crate::ai::agent::task::TaskId;
    use crate::ai::agent_providers::tools::subagent as subagent_tool;

    App::test((), |mut app| async move {
        initialize_history_model_test_app(&mut app);
        let terminal_view_id = EntityId::new();
        let history_model = app.add_singleton_model(|_| BlocklistAIHistoryModel::new(vec![], &[]));
        let (parent_id, task_id, stream_id) =
            prime_parent_conversation_for_subagent_test(&mut app, &history_model, terminal_view_id);

        let child_id = AIConversationId::new();
        history_model.update(&mut app, |model, ctx| {
            model.start_new_child_conversation(
                terminal_view_id,
                "A".to_owned(),
                parent_id,
                child_id,
                false,
                ctx,
            );
        });

        // 父侧一条仍停在 running 的帧(模拟上次退出前没跑完)。
        apply_messages_to_parent(
            &mut app,
            &history_model,
            parent_id,
            &stream_id,
            terminal_view_id,
            &task_id,
            vec![subagent_status_frame_message(
                &task_id,
                "frame-running",
                "call-1",
                subagent_tool::running_frame_json(&[subagent_tool::SubagentFrameChild::running(
                    child_id.to_string(),
                    "A".to_owned(),
                )]),
            )],
        );

        // 激活会话 —— 收敛应当在这里发生,而不是等用户点开子会话。
        history_model.update(&mut app, |model, ctx| {
            model.set_active_conversation_id(parent_id, terminal_view_id, ctx);
        });

        history_model.read(&app, |model, _| {
            let frame = model
                .conversation(&parent_id)
                .expect("parent exists")
                .get_task(&TaskId::new(task_id.to_string()))
                .expect("task exists")
                .messages()
                .find(|message| message.id == "frame-running")
                .expect("frame present");
            let parsed = subagent_tool::parse_status_frame(&frame.server_message_data)
                .expect("frame still parses");
            assert_eq!(
                parsed.status,
                subagent_tool::STATUS_ERROR,
                "激活会话后仍停在 running 的派发必须被收敛成失败"
            );
            assert_eq!(
                model
                    .conversation(&child_id)
                    .expect("child exists")
                    .status(),
                &ConversationStatus::Error,
                "子会话不能一直停在 InProgress"
            );

            // 关键:卡片读的是 exchange 里的 **output 表示**,不是 source message。
            // 若 output 没跟着刷新,UI 就会继续显示「运行中」(实测故障)。
            let parent = model.conversation(&parent_id).expect("parent exists");
            let run_statuses: Vec<ConversationStatus> = parent
                .all_exchanges()
                .into_iter()
                .filter_map(|exchange| exchange.output_status.output())
                .flat_map(|output| output.get().messages.clone())
                .filter_map(|message| match message.message {
                    crate::ai::agent::AIAgentOutputMessageType::SubagentRun(call) => {
                        Some(call.runs[0].fallback_status.clone())
                    }
                    _ => None,
                })
                .collect();
            assert_eq!(
                run_statuses,
                vec![ConversationStatus::Error],
                "父侧卡片的 output 表示必须同步为失败,否则 UI 会一直显示运行中"
            );
        });
    });
}
