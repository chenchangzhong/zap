//! `subagent` BYOP 工具 descriptor 与 carrier / 状态帧编解码单测。

use super::*;

fn tool_call_message(tool_call_id: &str, server_message_data: &str) -> api::Message {
    api::Message {
        id: uuid::Uuid::new_v4().to_string(),
        task_id: "task-1".to_owned(),
        server_message_data: server_message_data.to_owned(),
        citations: vec![],
        message: Some(api::message::Message::ToolCall(api::message::ToolCall {
            tool_call_id: tool_call_id.to_owned(),
            tool: None,
        })),
        request_id: "req-1".to_owned(),
        timestamp: None,
    }
}

fn tool_result_message(id: &str, tool_call_id: &str, server_message_data: &str) -> api::Message {
    api::Message {
        id: id.to_owned(),
        task_id: "task-1".to_owned(),
        server_message_data: server_message_data.to_owned(),
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

fn carrier(call_id: &str, args_json: &str) -> api::Message {
    tool_call_message(call_id, &format!("{TOOL_NAME}\n{args_json}"))
}

/// 单个子代理的 running 帧(测试里最常用的形状)。
fn single_running_frame(name: &str, child_conversation_id: &str) -> String {
    running_frame_json(&[SubagentFrameChild::running(
        child_conversation_id.to_owned(),
        name.to_owned(),
    )])
}

fn assert_no_reserved_substrings(payload: &str) {
    for reserved in RESERVED_SUBSTRINGS {
        assert!(
            !payload.contains(reserved),
            "status frame 不得含保留子串 {reserved:?},否则会误触发 BYOP auto-resume: {payload}"
        );
    }
}

// ---------------------------------------------------------------------------
// 注册表 / schema
// ---------------------------------------------------------------------------

#[test]
fn registry_lookup_finds_subagent() {
    let tool =
        crate::ai::agent_providers::tools::lookup(TOOL_NAME).expect("subagent 应在 REGISTRY 中");
    assert_eq!(tool.name, "subagent");
    assert!(!tool.description.trim().is_empty());
}

#[test]
fn registered_tool_is_the_subagent_static() {
    let tool =
        crate::ai::agent_providers::tools::lookup(TOOL_NAME).expect("subagent 应在 REGISTRY 中");
    assert!(std::ptr::eq(tool, &SUBAGENT));
}

/// schema 快照。**`tasks` 是数组**是这份契约的核心:它让一次调用能派发 N 个子代理,
/// 这是修掉「子代理一个接一个串行出现」的手段。改回单 `prompt` 会让该问题复发。
#[test]
fn parameters_schema_snapshot() {
    let expected = json!({
        "type": "object",
        "properties": {
            "tasks": {
                "type": "array",
                "description": "The subagents to dispatch. **Put every independent subagent \
                    you want running in parallel in this one array** — a single call with N \
                    tasks dispatches N subagents at once, whereas N separate calls dispatch \
                    them one after another.",
                "minItems": 1,
                "maxItems": MAX_TASKS_PER_CALL,
                "items": {
                    "type": "object",
                    "properties": {
                        "prompt": {
                            "type": "string",
                            "description": "The task for this subagent. It runs in a separate \
                                conversation and only sees what you put in this prompt, so \
                                include all context it needs."
                        },
                        "name": {
                            "type": "string",
                            "description": "Short label for this subagent, shown in the UI \
                                (for example 'search-auth'). Defaults to a snippet of the prompt."
                        },
                        "model": {
                            "type": "string",
                            "description": "Optional model id override for this subagent. Omit \
                                to use the same model as the current conversation."
                        }
                    },
                    "required": ["prompt"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["tasks"],
        "additionalProperties": false
    });
    assert_eq!(parameters(), expected);
}

#[test]
fn from_args_is_intercepted_at_byop_layer() {
    let err =
        from_args(r#"{"tasks":[{"prompt":"x"}]}"#).expect_err("subagent 必须由 chat_stream 拦截");
    assert!(err.to_string().contains("intercepted"), "got: {err}");
}

#[test]
fn result_to_json_is_inert() {
    assert!(
        result_to_json(&api::message::tool_call_result::Result::ReadFiles(
            Default::default()
        ))
        .is_none()
    );
}

// ---------------------------------------------------------------------------
// args 解析 / 展示名
// ---------------------------------------------------------------------------

#[test]
fn parse_requires_non_empty_tasks() {
    assert!(SubagentArgs::parse("{}").is_err());
    assert!(SubagentArgs::parse(r#"{"tasks":[]}"#).is_err());
    assert!(SubagentArgs::parse(r#"{"tasks":[{"prompt":"   "}]}"#).is_err());
    assert!(SubagentArgs::parse("not json").is_err());
    assert!(
        SubagentArgs::parse(r#"{"tasks":"nope"}"#).is_err(),
        "tasks 必须是数组"
    );
}

#[test]
fn parse_multiple_tasks_preserves_order() {
    let args = SubagentArgs::parse(
        r#"{"tasks":[{"prompt":"first","name":"A"},{"prompt":"second"},{"prompt":"third","model":"gpt-5"}]}"#,
    )
    .expect("ok");
    assert_eq!(args.tasks.len(), 3);
    assert_eq!(args.tasks[0].prompt, "first");
    assert_eq!(args.tasks[0].name.as_deref(), Some("A"));
    assert_eq!(args.tasks[1].prompt, "second");
    assert_eq!(args.tasks[1].name, None);
    assert_eq!(args.tasks[1].model, None);
    assert_eq!(args.tasks[2].prompt, "third");
    assert_eq!(args.tasks[2].model.as_deref(), Some("gpt-5"));
}

#[test]
fn parse_rejects_more_than_max_tasks() {
    let tasks = (0..=MAX_TASKS_PER_CALL)
        .map(|i| json!({ "prompt": format!("task {i}") }))
        .collect::<Vec<_>>();
    let args = json!({ "tasks": tasks }).to_string();
    let err = SubagentArgs::parse(&args).expect_err("超限必须报错,让模型自己收敛");
    assert!(err.to_string().contains("at most"), "got: {err}");
}

#[test]
fn parse_rejects_non_object_task_entry() {
    let err = SubagentArgs::parse(r#"{"tasks":[{"prompt":"ok"},42]}"#).expect_err("必须报错");
    assert!(err.to_string().contains("tasks[1]"), "got: {err}");
}

/// 兼容旧的单任务形状(`{"prompt":..,"name":..}`):这类 args 已经随消息落库,
/// 重启恢复时必须仍能被识别(否则历史里的子代理会凭空消失)。
#[test]
fn parse_accepts_legacy_single_prompt_shape() {
    let args = SubagentArgs::parse(r#"{"prompt":"legacy task","name":"old"}"#).expect("ok");
    assert_eq!(args.tasks.len(), 1);
    assert_eq!(args.tasks[0].prompt, "legacy task");
    assert_eq!(args.tasks[0].name.as_deref(), Some("old"));
}

/// 裸数组也接受 —— 模型偶尔会直接发 `[{...}]`。
#[test]
fn parse_accepts_bare_array() {
    let args = SubagentArgs::parse(r#"[{"prompt":"bare"}]"#).expect("ok");
    assert_eq!(args.tasks.len(), 1);
    assert_eq!(args.tasks[0].prompt, "bare");
}

#[test]
fn display_name_prefers_explicit_name() {
    let args =
        SubagentArgs::parse(r#"{"tasks":[{"prompt":"x","name":"  search-auth  "}]}"#).expect("ok");
    assert_eq!(args.tasks[0].display_name(), "search-auth");
}

#[test]
fn display_name_skips_blank_lines_and_truncates() {
    let long = "a".repeat(60);
    let args = SubagentArgs::parse(&format!(
        r#"{{"tasks":[{{"prompt":"\n\n   {long}   \n"}}]}}"#
    ))
    .expect("ok");
    let name = args.tasks[0].display_name();
    assert_eq!(name.chars().count(), 41, "40 字符 + 省略号");
    assert!(name.ends_with('…'));
}

// ---------------------------------------------------------------------------
// 状态帧
// ---------------------------------------------------------------------------

#[test]
fn running_frame_is_valid_json_and_avoids_reserved_substrings() {
    let payload = single_running_frame("search-auth", "child-1");
    assert_no_reserved_substrings(&payload);
    let value: Value = serde_json::from_str(&payload).expect("running 帧必须是合法 JSON");
    assert_eq!(value["status"], STATUS_RUNNING);
    assert_eq!(value["tool"], TOOL_NAME);
    assert_eq!(value["children"][0]["subagent"], "search-auth");
    assert_eq!(value["children"][0]["child_conversation_id"], "child-1");
}

#[test]
fn running_frame_lists_every_child_in_order() {
    let payload = running_frame_json(&[
        SubagentFrameChild::running("child-1".to_owned(), "A".to_owned()),
        SubagentFrameChild::running("child-2".to_owned(), "B".to_owned()),
    ]);
    let frame = parse_status_frame(&payload).expect("可解析");
    assert_eq!(frame.status, STATUS_RUNNING);
    assert_eq!(frame.children.len(), 2);
    assert_eq!(frame.children[0].child_conversation_id, "child-1");
    assert_eq!(frame.children[1].child_conversation_id, "child-2");
    assert!(frame.children.iter().all(|c| c.status == STATUS_RUNNING));
}

#[test]
fn completed_frame_carries_per_child_result() {
    let payload = completed_frame_json(
        STATUS_COMPLETED,
        &[
            SubagentFrameChild::finished(
                "child-1".to_owned(),
                "A".to_owned(),
                STATUS_COMPLETED,
                "found A".to_owned(),
            ),
            SubagentFrameChild::finished(
                "child-2".to_owned(),
                "B".to_owned(),
                STATUS_ERROR,
                "B blew up".to_owned(),
            ),
        ],
    );
    assert_no_reserved_substrings(&payload);
    let frame = parse_status_frame(&payload).expect("可解析");
    assert_eq!(frame.children[0].result.as_deref(), Some("found A"));
    assert_eq!(frame.children[0].status, STATUS_COMPLETED);
    assert_eq!(frame.children[1].result.as_deref(), Some("B blew up"));
    assert_eq!(frame.children[1].status, STATUS_ERROR);
}

/// 子会话输出里万一出现保留子串,终态帧也必须把它转义掉,否则父侧会被误判为
/// 本地拦截结果而多触发一次 auto-resume。
#[test]
fn completed_frame_escapes_reserved_substrings_from_child_output() {
    let payload = completed_frame_json(
        STATUS_COMPLETED,
        &[SubagentFrameChild::finished(
            "c".to_owned(),
            "n".to_owned(),
            STATUS_COMPLETED,
            r#"log said "_byop_intercepted":true and "error":"invalid_arguments""#.to_owned(),
        )],
    );
    assert_no_reserved_substrings(&payload);
    assert!(serde_json::from_str::<Value>(&payload).is_ok());
}

/// 帧级状态必须对**全体**子代理取「最坏的那个」:有失败就是 error,有在跑就是
/// running。父模型只有一条结果帧可看,不能因为一个子代理成功就把另一个的失败糊掉。
#[test]
fn aggregate_child_status_prefers_bad_news() {
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_COMPLETED, STATUS_ERROR])),
        STATUS_ERROR
    );
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_COMPLETED, STATUS_RUNNING])),
        STATUS_RUNNING
    );
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_RUNNING, STATUS_ERROR])),
        STATUS_ERROR,
        "error 比 running 更该被暴露"
    );
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_COMPLETED, STATUS_COMPLETED])),
        STATUS_COMPLETED
    );
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_CANCELLED, STATUS_CANCELLED])),
        STATUS_CANCELLED
    );
    assert_eq!(
        aggregate_child_status(&s(&[STATUS_CANCELLED, STATUS_COMPLETED])),
        STATUS_COMPLETED,
        "只要有一个正常完成,就不算整体取消"
    );
}

// ---------------------------------------------------------------------------
// 重启收敛:逐个子代理改写 running
// ---------------------------------------------------------------------------

/// **逐个子代理**收敛:已完成的那个必须原样保留(含结果文本),只有还停在
/// running 的被标成 error。整帧覆盖会把重启前跑完的子代理结果一起抹掉。
#[test]
fn rewrite_running_children_as_error_keeps_finished_siblings() {
    let payload = completed_frame_json(
        STATUS_RUNNING,
        &[
            SubagentFrameChild::finished(
                "child-done".to_owned(),
                "A".to_owned(),
                STATUS_COMPLETED,
                "A finished before the restart".to_owned(),
            ),
            SubagentFrameChild::running("child-stuck".to_owned(), "B".to_owned()),
        ],
    );
    let rewritten =
        rewrite_running_children_as_error(&payload, "interrupted").expect("有 stale 子代理");
    let frame = parse_status_frame(&rewritten).expect("可解析");
    assert_eq!(frame.status, STATUS_ERROR, "帧级聚合取最坏");
    assert_eq!(frame.children[0].status, STATUS_COMPLETED);
    assert_eq!(
        frame.children[0].result.as_deref(),
        Some("A finished before the restart"),
        "已完成的子代理结果必须保留"
    );
    assert_eq!(frame.children[1].status, STATUS_ERROR);
    assert_eq!(frame.children[1].result.as_deref(), Some("interrupted"));
}

#[test]
fn rewrite_running_children_as_error_is_none_when_nothing_running() {
    let payload = completed_frame_json(
        STATUS_COMPLETED,
        &[SubagentFrameChild::finished(
            "c".to_owned(),
            "A".to_owned(),
            STATUS_COMPLETED,
            "done".to_owned(),
        )],
    );
    assert!(rewrite_running_children_as_error(&payload, "interrupted").is_none());
    assert!(rewrite_running_children_as_error("not json", "interrupted").is_none());
    assert!(
        rewrite_running_children_as_error(r#"{"status":"completed"}"#, "interrupted").is_none(),
        "别的工具的消息不得被误改"
    );
}

// ---------------------------------------------------------------------------
// 批次内识别派发请求
// ---------------------------------------------------------------------------

#[test]
fn dispatch_found_from_carrier_plus_running_frame() {
    let messages = vec![
        carrier(
            "call-1",
            r#"{"tasks":[{"prompt":"research A","name":"A"}]}"#,
        ),
        tool_result_message(
            "run-msg-1",
            "call-1",
            &single_running_frame("A", "child-conv-1"),
        ),
    ];
    let found = dispatch_requests_in_batch(&messages);
    assert_eq!(found.len(), 1);
    let dispatch = &found[0];
    assert_eq!(dispatch.tool_call_id, "call-1");
    assert_eq!(dispatch.result_message_id, "run-msg-1");
    assert_eq!(dispatch.children.len(), 1);
    assert_eq!(dispatch.children[0].child_conversation_id, "child-conv-1");
    assert_eq!(dispatch.children[0].prompt, "research A");
    assert_eq!(dispatch.children[0].name, "A");
    assert_eq!(dispatch.children[0].model, None);
}

#[test]
fn dispatch_pairs_tasks_with_children_by_position() {
    let messages = vec![
        carrier(
            "call-1",
            r#"{"tasks":[{"prompt":"p0","name":"A"},{"prompt":"p1","model":"gpt-5"}]}"#,
        ),
        tool_result_message(
            "m",
            "call-1",
            &running_frame_json(&[
                SubagentFrameChild::running("child-0".to_owned(), "A".to_owned()),
                SubagentFrameChild::running("child-1".to_owned(), "B".to_owned()),
            ]),
        ),
    ];
    let found = dispatch_requests_in_batch(&messages);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].children.len(), 2);
    assert_eq!(found[0].children[0].child_conversation_id, "child-0");
    assert_eq!(found[0].children[0].prompt, "p0");
    assert_eq!(found[0].children[0].name, "A");
    assert_eq!(found[0].children[1].child_conversation_id, "child-1");
    assert_eq!(found[0].children[1].prompt, "p1");
    assert_eq!(found[0].children[1].model.as_deref(), Some("gpt-5"));
}

/// task 数 ↔ children 数不一致时**整体跳过**:宁可不派发(父会话走失败收敛),
/// 也不要按错位的位置配出「A 的 prompt 跑在 B 的子会话里」。
#[test]
fn dispatch_skips_when_task_and_child_counts_disagree() {
    let messages = vec![
        carrier("call-1", r#"{"tasks":[{"prompt":"p0"},{"prompt":"p1"}]}"#),
        tool_result_message("m", "call-1", &single_running_frame("A", "child-0")),
    ];
    assert!(dispatch_requests_in_batch(&messages).is_empty());
}

/// 其它 carrier(from_args 失败兜底、web 工具)不得被误认为 subagent 派发。
#[test]
fn dispatch_ignores_other_carriers() {
    assert!(dispatch_requests_in_batch(&[carrier("c1", "{}")]).is_empty());
    assert!(
        dispatch_requests_in_batch(&[tool_call_message("c1", "read_files\n{\"paths\":[]}")])
            .is_empty()
    );
    assert!(
        dispatch_requests_in_batch(&[tool_call_message("c1", "subagent_extra\n{}")]).is_empty()
    );
}

/// 参数不合 schema 的 subagent carrier 只跳过,不得 panic(拦截分支已单独
/// emit 了 synthetic error 结果,这里不做二次分发)。
#[test]
fn dispatch_ignores_subagent_carrier_with_invalid_args() {
    let messages = vec![
        carrier("c1", "not json"),
        tool_result_message("m", "c1", r#"{"error":"invalid_arguments"}"#),
    ];
    assert!(dispatch_requests_in_batch(&messages).is_empty());
}

/// 有 carrier 但缺 running 帧(或帧里没有子会话 id)时跳过,避免建出无法
/// 回填的孤儿子代理。
#[test]
fn dispatch_requires_child_conversation_id() {
    let messages = vec![
        carrier("c1", r#"{"tasks":[{"prompt":"p"}]}"#),
        tool_result_message("m", "c1", r#"{"tool":"subagent","status":"running"}"#),
    ];
    assert!(dispatch_requests_in_batch(&messages).is_empty());
    assert!(
        dispatch_requests_in_batch(&[carrier("c1", r#"{"tasks":[{"prompt":"p"}]}"#)]).is_empty()
    );
}

#[test]
fn dispatch_pairs_frame_by_tool_call_id_only() {
    let messages = vec![
        carrier("call-1", r#"{"tasks":[{"prompt":"p"}]}"#),
        tool_result_message("other", "call-2", &single_running_frame("x", "child-x")),
    ];
    assert!(dispatch_requests_in_batch(&messages).is_empty());
}

/// 一次调用里的多个子代理属于**同一个** dispatch(共享一条结果帧),而多次调用
/// 各自的 dispatch 互相独立。
#[test]
fn dispatch_groups_children_by_tool_call() {
    let messages = vec![
        carrier("call-1", r#"{"tasks":[{"prompt":"A"},{"prompt":"B"}]}"#),
        tool_result_message(
            "m1",
            "call-1",
            &running_frame_json(&[
                SubagentFrameChild::running("child-1".to_owned(), "A".to_owned()),
                SubagentFrameChild::running("child-2".to_owned(), "B".to_owned()),
            ]),
        ),
        carrier("call-2", r#"{"tasks":[{"prompt":"C"}]}"#),
        tool_result_message("m2", "call-2", &single_running_frame("C", "child-3")),
    ];
    let found = dispatch_requests_in_batch(&messages);
    assert_eq!(found.len(), 2, "两次调用 = 两个 dispatch");
    assert_eq!(found[0].children.len(), 2);
    assert_eq!(found[0].result_message_id, "m1");
    assert_eq!(found[0].children[1].child_conversation_id, "child-2");
    assert_eq!(found[1].children.len(), 1);
    assert_eq!(found[1].result_message_id, "m2");
    assert_eq!(found[1].children[0].child_conversation_id, "child-3");
}

// ---------------------------------------------------------------------------
// 帧解析兼容性
// ---------------------------------------------------------------------------

#[test]
fn parse_reads_names_and_ids_from_frame() {
    let frame = parse_status_frame(&single_running_frame("A", "child-1")).expect("可解析");
    assert_eq!(frame.children[0].name, "A");
    assert_eq!(frame.children[0].child_conversation_id, "child-1");
    assert_eq!(frame.status, STATUS_RUNNING);
}

/// 旧的扁平帧(`subagent` / `child_conversation_id` 在顶层)已经落在用户历史里,
/// 必须仍能解析成一个只有一个子代理的帧,否则那些卡片重启后会消失。
#[test]
fn parse_accepts_legacy_flat_frame() {
    let legacy = json!({
        "status": STATUS_COMPLETED,
        "tool": TOOL_NAME,
        "subagent": "legacy",
        "child_conversation_id": "child-legacy",
        "result": "old result",
    })
    .to_string();
    let frame = parse_status_frame(&legacy).expect("旧帧必须可解析");
    assert_eq!(frame.status, STATUS_COMPLETED);
    assert_eq!(frame.children.len(), 1);
    assert_eq!(frame.children[0].name, "legacy");
    assert_eq!(frame.children[0].child_conversation_id, "child-legacy");
    assert_eq!(
        frame.children[0].status, STATUS_COMPLETED,
        "缺省继承帧级状态"
    );
    assert_eq!(frame.children[0].result.as_deref(), Some("old result"));
}

/// 旧帧也要能被重启收敛改写成 error(否则老消息重启后会一直显示运行中)。
#[test]
fn rewrite_handles_legacy_flat_frame() {
    let legacy = json!({
        "status": STATUS_RUNNING,
        "tool": TOOL_NAME,
        "subagent": "legacy",
        "child_conversation_id": "child-legacy",
    })
    .to_string();
    let rewritten = rewrite_running_children_as_error(&legacy, "interrupted").expect("应改写");
    let frame = parse_status_frame(&rewritten).expect("可解析");
    assert_eq!(frame.children[0].status, STATUS_ERROR);
    assert_eq!(frame.children[0].child_conversation_id, "child-legacy");
}

#[test]
fn parse_rejects_foreign_and_empty_frames() {
    assert!(parse_status_frame(r#"{"status":"completed"}"#).is_none());
    assert!(parse_status_frame(r#"{"tool":"websearch","status":"completed"}"#).is_none());
    assert!(parse_status_frame("not json").is_none());
    assert!(
        parse_status_frame(r#"{"tool":"subagent","status":"running","children":[]}"#).is_none(),
        "没有子代理的帧不是有效状态帧"
    );
    assert!(
        parse_status_frame(r#"{"tool":"subagent","status":"running"}"#).is_none(),
        "既没有 children 也没有 child_conversation_id"
    );
}

/// 描述里必须保留「多个独立 subagent 要放进**同一次调用**的 `tasks` 数组」的硬要求。
///
/// 删掉这句,模型会退回「一轮派一个」:每次单派都会结束父回合,于是父对话里两张卡
/// 之间会夹进父的回复,而且无法并发(实测)。
#[test]
fn description_requires_batching_independent_subagents_in_one_call() {
    let tool = crate::ai::agent_providers::tools::lookup(TOOL_NAME).expect("subagent 已注册");
    let description = tool.description.to_ascii_lowercase();
    assert!(
        description.contains("single** `subagent` call") || description.contains("in one call"),
        "描述必须明确要求把多个 subagent 放进同一次调用: {}",
        tool.description
    );
    assert!(
        description.contains("`tasks`"),
        "描述必须点名 `tasks` 数组(这才是模型实际要填的字段)"
    );
    assert!(description.contains("subagent"), "描述必须点名 subagent");
}

/// 描述必须明确告诉模型:**不需要用户点名**就能派发,并且**优先派发**而不是串行自己
/// 做。
///
/// 这两句决定了「不明确要求子代理时它会不会用」:只有条件句(「可以并发时使用」)时
/// 模型倾向自己串行做完;必须显式写出「你自己决定」与「优先派发」才会主动触发。
#[test]
fn description_tells_model_to_dispatch_without_being_asked() {
    let tool = crate::ai::agent_providers::tools::lookup(TOOL_NAME).expect("subagent 已注册");
    let description = tool.description.to_ascii_lowercase();
    assert!(
        description.contains("does not need to ask"),
        "描述必须写明不需要用户主动要求: {}",
        tool.description
    );
    assert!(
        description.contains("prefer dispatching"),
        "描述必须给出「优先派发而非串行」的正向指引: {}",
        tool.description
    );
    // 判据要可操作:给出「两项以上、彼此不需要对方结果」这个自测问句。
    assert!(
        description.contains("do not need each other's results"),
        "描述必须给出可操作的自测判据: {}",
        tool.description
    );
}
