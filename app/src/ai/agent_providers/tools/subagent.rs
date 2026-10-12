//! `subagent` BYOP 工具 descriptor + 父侧 carrier / 状态帧编解码。
//!
//! ## 不走 protobuf executor
//!
//! 与 `websearch` / `webfetch` / `todowrite` 同属「BYOP 本地拦截工具」:
//! `from_args` 永远返回 `Err`,真正的分发在 `chat_stream::generate_byop_output`
//! 的拦截分支里按 tool name 命中后完成。`result_to_json` 同理永远返回 `None`
//! (没有对应的 protobuf result variant)。这两个 stub 仅满足 [`OpenAiTool`]
//! 的字段约束。
//!
//! ## 消息载体(D1)
//!
//! 拦截分支合成一对消息,都进 `final_messages` 给下一轮模型推理与 UI 用:
//!
//! 1. **carrier ToolCall**:沿用 `make_tool_call_carrier_message`,`tool` oneof
//!    留 `None`,`server_message_data = "<fn_name>\n<args_json>"`。出站回放由
//!    `serialize_outgoing_tool_call` 的 carrier 分支还原 `(fn_name, args)`。
//! 2. **running 帧 ToolCallResult**:`result` 留 `None`,`server_message_data`
//!    是**合法 JSON**(见 [`running_frame_json`])。必须是合法 JSON,否则
//!    `persisted_tool_result_kind` 会因原文含保留串返回
//!    `UnreadableLocalInterception` 而 blocking 整个请求(D3 硬约束 2);
//!    且**不得**含字节级子串 `"_byop_intercepted":true` /
//!    `"error":"invalid_arguments"`,否则会误触发 BYOP auto-resume(D3 硬约束 1)。
//!
//! running 帧还额外携带 `child_conversation_id` —— 子会话 id 由 chat_stream
//! 预生成并写进这里,而不是等控制层创建后再回填:消息体本身随 conversation
//! 落库,重启恢复时渲染层可以直接从消息里拿到子会话 id,无需额外持久化状态。
//!
//! ## 状态推进(步骤 5)
//!
//! 子会话进入终态后,父侧同一条 ToolCallResult 的 `server_message_data` 被
//! **原地**改写为 [`completed_frame_json`]。**不能另追加一条结果**:控制层的
//! `has_persisted_tool_result` 只按 `(task_id, tool_call_id)` 判存在性、不看内容,
//! 两条同 id 结果还会让就绪性检查报 `DuplicateToolResults`。
//!
//! ## 一次调用派发多个子代理(批量契约)
//!
//! `tasks` 是**数组**:一次 `subagent` 调用可以派发 N 个子代理。这是为了让模型
//! 天然地「一次派发完」—— 在此之前入参只有一个 `prompt`,模型每轮只会发一个
//! call,用户看到的是子代理一个接一个地串行出现(实测每轮恰好一条 call)。
//!
//! 关键约束:**一个 call 仍然只对应一对消息**(一条 carrier + 一条结果帧),
//! 因为 D1/D3 的判重与就绪性都以 `(task_id, tool_call_id)` 为单位,给一个 call
//! 写多条结果会被判 `DuplicateToolResults`。所以 N 个子代理共享**同一个**
//! `result_message_id`,帧内部用 `children` 数组逐个记录状态与结果。
//!
//! 这带来两条推论:
//!
//! - 子会话 id 不能在 carrier 合成时把所有 N 个都预生成后只留一个 —— 帧里
//!   必须带满 N 个,渲染层/控制层逐个认领(见 [`SubagentStatusFrame::children`])。
//! - 「这个 call 是否全部完成」= 帧里**所有** child 都到了终态,而不是某一条
//!   子会话状态事件。控制层按 child 粒度回填,全部终态后 resume 父会话。
//!
//! 兼容旧的单子代理帧:解析层同时接受 `{"subagent": .., "child_conversation_id": ..}`
//! 这种扁平形状(老消息已经落库,重启恢复时必须仍能渲染出卡片)。

use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use warp_multi_agent_api as api;

use super::OpenAiTool;

pub const TOOL_NAME: &str = "subagent";

/// 保留子串(D3 硬约束 1):`server_message_data` 一旦包含即会被控制层的
/// auto-resume 字节级匹配命中。本模块所有生成的帧都必须绕开。
const RESERVED_SUBSTRINGS: [&str; 2] = [
    r#""_byop_intercepted":true"#,
    r#""error":"invalid_arguments""#,
];

/// 状态帧里 `status` 字段的取值。
pub const STATUS_RUNNING: &str = "running";
pub const STATUS_COMPLETED: &str = "completed";
pub const STATUS_ERROR: &str = "error";
pub const STATUS_CANCELLED: &str = "cancelled";

/// 一次调用最多派发的子代理数。
///
/// 上限存在的理由是**资源**:每个子代理会建一个隐藏 pane + 一条独立会话(各自
/// 一份 `TerminalModel`、PTY、请求流)。模型偶尔会把「一个任务拆成 20 小步」当成
/// 20 个子代理,那种规模既没有并行收益(上游请求也是串行的排队模型)又会把
/// 用户机器压满。超限直接在解析层报错,让模型下一轮自己收敛。
pub const MAX_TASKS_PER_CALL: usize = 8;

fn parameters() -> Value {
    json!({
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
    })
}

fn from_args(_args: &str) -> Result<api::message::tool_call::Tool> {
    Err(anyhow!(
        "subagent is intercepted by chat_stream BYOP tool dispatcher; \
         from_args should never be called"
    ))
}

fn result_to_json(_result: &api::message::tool_call_result::Result) -> Option<Value> {
    None
}

pub static SUBAGENT: OpenAiTool = OpenAiTool {
    name: TOOL_NAME,
    description: include_str!("../prompts/tool_descriptions/subagent.md"),
    parameters,
    from_args,
    result_to_json,
};

/// `subagent` 工具入参(批量契约):一次调用带一个 `tasks` 数组。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentArgs {
    pub tasks: Vec<SubagentTask>,
}

/// 批量入参里的单个任务。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentTask {
    pub prompt: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

impl SubagentTask {
    /// UI / 状态帧里展示的子代理名。没显式给 `name` 时取 prompt 首个非空行截断。
    pub fn display_name(&self) -> String {
        if let Some(name) = self
            .name
            .as_ref()
            .map(|n| n.trim())
            .filter(|n| !n.is_empty())
        {
            return name.to_owned();
        }
        let first_line = self
            .prompt
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("subagent");
        truncate_chars(first_line, 40)
    }
}

impl SubagentArgs {
    pub fn parse(args_str: &str) -> Result<Self> {
        let raw: Value = serde_json::from_str(args_str)
            .map_err(|e| anyhow!("subagent arguments did not match the tool schema: {e}"))?;
        Self::parse_value(raw)
    }

    fn parse_value(raw: Value) -> Result<Self> {
        let tasks = match raw {
            Value::Array(tasks) => tasks,
            Value::Object(mut object) => match object.remove("tasks") {
                Some(Value::Array(tasks)) => tasks,
                // 兼容旧的单任务形状 `{"prompt": .., "name": ..}` —— 老消息已经落库,
                // 重启恢复时仍要能派发/渲染(与帧的扁平形状兼容成对)。
                Some(other) => bail!("subagent `tasks` must be an array, got: {other}"),
                None => vec![Value::Object(object)],
            },
            other => {
                bail!("subagent arguments must be an object with a `tasks` array, got: {other}")
            }
        };

        if tasks.is_empty() {
            bail!("subagent `tasks` must not be empty");
        }
        if tasks.len() > MAX_TASKS_PER_CALL {
            bail!(
                "subagent accepts at most {MAX_TASKS_PER_CALL} tasks per call, got {}",
                tasks.len()
            );
        }

        let tasks = tasks
            .into_iter()
            .enumerate()
            .map(|(index, task)| {
                let task: SubagentTask = serde_json::from_value(task).map_err(|e| {
                    anyhow!("subagent `tasks[{index}]` did not match the schema: {e}")
                })?;
                if task.prompt.trim().is_empty() {
                    bail!("subagent `tasks[{index}].prompt` must not be empty");
                }
                Ok(task)
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { tasks })
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in s.chars().enumerate() {
        if idx == max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

/// running 帧的 `server_message_data`(合法 JSON,不含保留子串)。
///
/// `children` 顺序即派发顺序,与 `SubagentArgs::tasks` 一一对应;渲染层依赖这个
/// 顺序把卡片行和子会话对上。
pub fn running_frame_json(children: &[SubagentFrameChild]) -> String {
    let value = json!({
        "status": STATUS_RUNNING,
        "tool": TOOL_NAME,
        "children": children
            .iter()
            .map(|child| json!({
                "child_conversation_id": child.child_conversation_id,
                "subagent": child.name,
            }))
            .collect::<Vec<_>>(),
        "message": "Subagent started in a separate conversation; its result will be delivered here.",
    });
    encode_frame(&value)
}

/// 终态帧的 `server_message_data`。
///
/// `result` 是子会话最终 assistant 输出(摘要)。整帧的 `status` 由**所有**子代理
/// 的状态聚合而来(见 [`aggregate_child_status`]):一个 call 派发 N 个子代理,
/// 父侧只有一条结果帧,所以帧级状态只能是聚合值;每个子代理自己的结局记在
/// `children[i].status` 里。
pub fn completed_frame_json(status: &str, children: &[SubagentFrameChild]) -> String {
    let value = json!({
        "status": status,
        "tool": TOOL_NAME,
        "children": children
            .iter()
            .map(|child| {
                let mut entry = json!({
                    "child_conversation_id": child.child_conversation_id,
                    "subagent": child.name,
                    "status": child.status,
                });
                if let Some(result) = child.result.as_ref() {
                    if let Some(fields) = entry.as_object_mut() {
                        fields.insert("result".to_owned(), Value::String(result.clone()));
                    }
                }
                entry
            })
            .collect::<Vec<_>>(),
    });
    encode_frame(&value)
}

/// 帧里的一个子代理条目。running 帧只填 id + name;终态帧还填 `status` / `result`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentFrameChild {
    pub child_conversation_id: String,
    pub name: String,
    pub status: String,
    pub result: Option<String>,
}

impl SubagentFrameChild {
    /// running 帧用的条目。
    pub fn running(child_conversation_id: String, name: String) -> Self {
        Self {
            child_conversation_id,
            name,
            status: STATUS_RUNNING.to_owned(),
            result: None,
        }
    }

    /// 已到终态的条目。
    pub fn finished(
        child_conversation_id: String,
        name: String,
        status: &str,
        result: String,
    ) -> Self {
        Self {
            child_conversation_id,
            name,
            status: status.to_owned(),
            result: Some(result),
        }
    }
}

/// 把 N 个子代理的状态聚合成父侧帧的帧级 `status`。
///
/// 优先暴露**坏消息**:只要有子代理失败,帧级就是 error —— 让父模型一眼看出
/// 「这轮派发里有东西坏了」,而不是被一个笼统的 completed 糊过去。全部成功才是
/// completed;全部取消是 cancelled;还有在跑的则是 running(等待中)。
pub fn aggregate_child_status(statuses: &[String]) -> &'static str {
    if statuses.iter().any(|s| s == STATUS_ERROR) {
        return STATUS_ERROR;
    }
    if statuses.iter().any(|s| s == STATUS_RUNNING) {
        return STATUS_RUNNING;
    }
    if statuses.iter().all(|s| s == STATUS_CANCELLED) && !statuses.is_empty() {
        return STATUS_CANCELLED;
    }
    STATUS_COMPLETED
}

/// 序列化并保证不含保留子串(D3)。理论上 `result` 里出现保留串的概率极低,
/// 但一旦出现会让父侧被误判为「本地拦截结果」而触发多余的 auto-resume,
/// 所以在编码层兜住:命中时把 `_` 转义掉,不改变语义可读性。
fn encode_frame(value: &Value) -> String {
    let mut encoded = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    for reserved in RESERVED_SUBSTRINGS {
        if encoded.contains(reserved) {
            log::warn!(
                "[byop][subagent] encoded status frame collided with a reserved BYOP substring; \
                 escaping to avoid a spurious auto-resume"
            );
            encoded = encoded.replace(reserved, &reserved.replace('_', "\\u005f"));
        }
    }
    encoded
}

/// 从一批刚应用的消息里识别出的子代理派发请求。
///
/// **一个 dispatch = 一次工具调用**,可能包含多个子代理(见 [`Self::children`]),
/// 因为父侧一个 call 只有一对消息、只能有一条结果帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDispatch {
    pub tool_call_id: String,
    /// running 帧的消息 id;子代理终态时按此 id 原地改写。
    pub result_message_id: String,
    pub children: Vec<SubagentDispatchChild>,
}

/// 一次派发里的单个子代理。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentDispatchChild {
    pub child_conversation_id: String,
    pub prompt: String,
    pub name: String,
    pub model: Option<String>,
}

/// 在一批消息里找出 `subagent` 派发请求。
///
/// 判定方式:某条 `Message::ToolCall{tool: None}` 的 `server_message_data` 首行为
/// [`TOOL_NAME`](与 web 工具约定一致:首行 fn_name,次行起为 args JSON);
/// 再在同批消息里找 `tool_call_id` 相同的 `ToolCallResult` 拿到 running 帧 id
/// 与预生成的子会话 id。任一步缺失都返回 `None`(不会 panic,也不会误认
/// 其它 carrier —— 例如 from_args 失败兜底或 web 工具的 carrier)。
///
/// carrier 的 `tasks[i]` 与帧的 `children[i]` 按**位置**配对:两者都由
/// chat_stream 在同一次拦截里按同一顺序生成,是唯一的对应关系来源(帧里不带
/// prompt,args 里不带子会话 id,单独任何一边都不完整)。
pub fn dispatch_requests_in_batch(messages: &[api::Message]) -> Vec<SubagentDispatch> {
    let mut dispatch = Vec::new();
    for carrier in messages {
        let Some(api::message::Message::ToolCall(tool_call)) = carrier.message.as_ref() else {
            continue;
        };
        if tool_call.tool.is_some() {
            continue;
        }
        let Some(args_str) = carrier_server_message_data(carrier) else {
            continue;
        };
        let Ok(args) = SubagentArgs::parse(&args_str) else {
            continue;
        };
        let Some(frame) = messages.iter().find(|msg| {
            matches!(
                msg.message.as_ref(),
                Some(api::message::Message::ToolCallResult(result))
                    if result.tool_call_id == tool_call.tool_call_id
            )
        }) else {
            continue;
        };
        let Some(status_frame) = parse_status_frame(&frame.server_message_data) else {
            continue;
        };
        if status_frame.children.is_empty() {
            log::warn!(
                "[byop][subagent] dispatch carrier for tool_call_id={} has no children in its \
                 status frame; skipping dispatch",
                tool_call.tool_call_id
            );
            continue;
        }
        if status_frame.children.len() != args.tasks.len() {
            // 不 panic 也不猜:两者不一致说明这条消息不是本进程生成的(或生成逻辑
            // 有 bug),按缺失处理让父会话走失败收敛,而不是派发出错位的子代理。
            log::warn!(
                "[byop][subagent] dispatch carrier for tool_call_id={} has {} tasks but {} \
                 children in its status frame; skipping dispatch",
                tool_call.tool_call_id,
                args.tasks.len(),
                status_frame.children.len()
            );
            continue;
        }
        let children = args
            .tasks
            .into_iter()
            .zip(status_frame.children)
            .map(|(task, child)| SubagentDispatchChild {
                child_conversation_id: child.child_conversation_id,
                name: task.display_name(),
                model: task.model,
                prompt: task.prompt,
            })
            .collect();
        dispatch.push(SubagentDispatch {
            tool_call_id: tool_call.tool_call_id.clone(),
            result_message_id: frame.id.clone(),
            children,
        });
    }
    dispatch
}

/// 从 carrier 的 `server_message_data` 里取出 args JSON 部分。
///
/// 约定是 `<fn_name>\n<args_json>`,`split_once` 保证只切一次 —— args 内部
/// 含换行也不会被破坏(A2)。
fn carrier_server_message_data(carrier: &api::Message) -> Option<String> {
    let (fn_name, args) = carrier.server_message_data.split_once('\n')?;
    if fn_name != TOOL_NAME {
        return None;
    }
    Some(args.to_owned())
}

/// 把一条状态帧里仍停在 `running` 的子代理改写成 error,其余原样保留。
///
/// 专供重启收敛用:**必须逐个子代理改**,不能整帧覆盖 —— 一个 call 可能派发了
/// N 个子代理,重启时其中一部分可能早就跑完并把结果写进帧里了,整帧覆盖会把
/// 那些成功结果连同它们的文本一起抹掉。
///
/// 输入不是本工具的状态帧(或没有 stale 子代理)时返回 `None`,调用方据此跳过。
pub fn rewrite_running_children_as_error(
    server_message_data: &str,
    reason: &str,
) -> Option<String> {
    let frame = parse_status_frame(server_message_data)?;
    if !frame
        .children
        .iter()
        .any(|child| child.status == STATUS_RUNNING)
    {
        return None;
    }
    let children = frame
        .children
        .into_iter()
        .map(|child| {
            if child.status != STATUS_RUNNING {
                return child;
            }
            SubagentFrameChild::finished(
                child.child_conversation_id,
                child.name,
                STATUS_ERROR,
                reason.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let status = aggregate_child_status(
        &children
            .iter()
            .map(|child| child.status.clone())
            .collect::<Vec<_>>(),
    );
    Some(completed_frame_json(status, &children))
}

/// 一个子代理在帧里的状态(父侧状态帧的解析结果)。
pub type SubagentChildStatus = SubagentFrameChild;

/// 父侧状态帧的解析结果(表示层 / 渲染层 / 控制层用)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentStatusFrame {
    /// 帧级聚合状态(一个 call 派发 N 个子代理,父侧只有一条结果帧)。
    pub status: String,
    /// 按派发顺序排列的子代理。空表示这不是一个有效的状态帧。
    pub children: Vec<SubagentChildStatus>,
}

/// 解析父侧状态帧的 `server_message_data`;不是本工具的状态帧时返回 `None`。
///
/// 这是**唯一**允许把一条 `ToolCallResult` 认成子代理状态帧的入口 —— 判定
/// 依赖帧里的 `tool` 字段,所以本地拦截工具 / 解析失败兜底等其它
/// `result: None` 的 ToolCallResult 不会被误认。
///
/// 同时接受两种形状:
/// - 批量(当前):`{"tool":"subagent","status":..,"children":[{child_conversation_id,
///   subagent, status, result}]}`
/// - 扁平(旧消息,已在用户历史里):`{"tool":"subagent","status":..,
///   "subagent":..,"child_conversation_id":..,"result":..}` —— 老帧只有一个子
///   代理,补一个缺省的 running 状态即可,语义等价。
pub fn parse_status_frame(server_message_data: &str) -> Option<SubagentStatusFrame> {
    let value: Value = serde_json::from_str(server_message_data.trim()).ok()?;
    if value.get("tool").and_then(Value::as_str) != Some(TOOL_NAME) {
        return None;
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let children = match value.get("children").and_then(Value::as_array) {
        Some(children) => children
            .iter()
            .map(|child| {
                let field = |key: &str| {
                    child
                        .get(key)
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned()
                };
                SubagentChildStatus {
                    child_conversation_id: field("child_conversation_id"),
                    name: field("subagent"),
                    // 逐个子代理的 `status` 缺省继承帧级状态:旧形状没有 children,
                    // 即使将来出现「部分字段缺失」的帧也不该把子代理谎报成运行中。
                    status: child
                        .get("status")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_else(|| status.clone()),
                    result: child
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }
            })
            .filter(|child| !child.child_conversation_id.is_empty())
            .collect(),
        None => {
            let child_conversation_id = value
                .get("child_conversation_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            if child_conversation_id.is_empty() {
                Vec::new()
            } else {
                vec![SubagentChildStatus {
                    child_conversation_id,
                    name: value
                        .get("subagent")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    status: status.clone(),
                    result: value
                        .get("result")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                }]
            }
        }
    };
    if children.is_empty() {
        return None;
    }
    Some(SubagentStatusFrame { status, children })
}

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod tests;
