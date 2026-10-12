# Oz 子代理（模型乙：独立子会话）— 执行计划

> **术语**：本文件用「模型甲/乙」指**载体**：甲 = 同会话子任务（`Task`／`SubagentParams`），乙 = 独立子会话（`AIConversation`／`parent_conversation_id`）。此前评估里用过的"（旧）A 档／B 档"是**范围轴**（MVP vs 对齐上游），与本文无关。
>
> **决策**：采用**模型乙**；子代理**权限与父一致**（用户 2026-10 决策）。

## Context

本 fork（Oz）是 **BYOP-only**：非 BYOP 请求直接报错（`app/src/ai/blocklist/controller/response_stream.rs:674-685`）。上游 Warp 的 multi-agent orchestration 被物理删除（`fa1063715` -4219、`7326fe2d4` -3684、`e102221b9` -1692）。

**为什么选乙**（三条硬证据）：
1. **与现成 UI 同源**：状态卡 `child_conversations_of(active_conversation_id)`（`app/src/ai/blocklist/agent_view/child_agent_status_card.rs:179`）、隐藏 pane（`child_agent_panes: HashMap<AIConversationId, PaneId>`，`app/src/pane_group/mod.rs:838`）、pill bar/topology（全 `AIConversationId` 签名）**都是对话树**。选甲这三样全部对不上。
2. **并发天然**：子会话有独立 terminal view；用户 follow-up 只取消**该 view 的 active conversation**（`app/src/ai/blocklist/controller.rs:1372-1385`）⇒ 子代理不受影响。
3. **权限**：需显式传播，但参考实现现成（`fa1063715^:app/src/pane_group/child_agent.rs` 的 `propagate_parent_agent_settings`）。

**缺口**（乙要补的活）：
- 派生侧被删：`start_new_child_conversation`（`app/src/ai/blocklist/history_model.rs:349`）**除测试外无调用方**；隐藏 pane 的 `create_hidden_child_agent_pane`（`app/src/pane_group/mod.rs:2614-2645`）只在**恢复**路径被调用，没有新建路径。
- 父侧无结果回传生产者；父的回合默认立刻结束（父会话没有子任务 ⇒ `has_active_subagent` 为 false）—— 这正是 D4/D4.1 要解决的。
- 恢复侧：`create_missing_child_agent_panes` 为启动急建，挂 `TODO(QUALITY-378)`（`app/src/pane_group/mod.rs:2588`）。

**目标**：Oz 自带 agent 可调用 `subagent` 工具派子会话，子会话跑完后结果回父任务；父对话可见子代理、可点开观摩。
**验收**：见 `## Verification`。

## 范围 / 非目标

- **本期含**：阶段一（模型乙核心能力）+ 阶段二（pill bar 线，从上游移植）。两者**同源**，阶段二在乙之下真正可用。
- **本期不含**：多级嵌套、共享会话 viewer、TUI 编排；不恢复 `crates/ai` 的 `SendMessageToAgentResult` / `RunAgentsExecutionMode` / `orchestration_config`。
- **零改动**：protobuf（git 依赖 pin，`Cargo.toml:351`）、`crates/ai` 的 action/action_result 枚举、**`Tool::Subagent` 通道与其 skip 判据**（见 D1）。

## 设计决策

**D1｜父侧用 BYOP carrier，不用 `Tool::Subagent`（关键简化）**
沿用 `websearch`/`webfetch`/`todowrite` 的"本地拦截工具"范式：父会话里的调用写成 `Message::ToolCall{ tool_call_id, tool: None }` + `server_message_data = "<fn_name>\n<args_json>"`（构造函数 `make_tool_call_carrier_message`，`app/src/ai/agent_providers/chat_stream.rs:5203-5223`）；出站多轮回放由 `serialize_outgoing_tool_call` 优先识别 `tool.is_none()` 并从 `server_message_data` 复原 `(fn_name, args)`（`chat_stream.rs:2374-2396`）。

**由此整套 proto 子任务机制都不需要碰**：不写 `Tool::Subagent`、不建 `SubagentParams`、不需要 `Task::new_subtask` 的顺序硬约束、不需要 `is_subagent_task_finished`／`has_active_subagent`、**也不需要收窄 `chat_stream.rs:1452`/`:620-623` 的 skip 判据**（该判据只对 `tc.subagent().is_some()` 触发）。原先评估中"最高回归风险项"因此消失。

**D2｜子代理 = 独立子会话 + 隐藏 pane**
每个子代理建一个 `AIConversation`（`parent_conversation_id` + `agent_name`），宿主是隐藏 pane（`HiddenPaneReason::ChildAgent`），**复用**既有机制：`insert_terminal_pane_hidden_for_child_agent`（`app/src/pane_group/mod.rs:3371-3397`）、`start_new_child_conversation`（`history_model.rs:349-379`）、reveal 链路（`app/src/pane_group/pane/terminal_pane.rs:945-952`）。状态卡**零改动**即可显示。

**D3｜父侧"运行中"帧：合法 JSON + 不含保留子串 + 必须原地更新**
参考 web 工具的 loading→done 两帧做法（`chat_stream.rs:4370-4415`）：先 yield 一条 `ToolCallResult{ result: None, server_message_data: <JSON{status:"running"}> }`，子会话终态时用 `UpdateTaskMessage` + `FieldMask` **原地更新同一条 message**。

已核实的三条硬约束（**勿把因果挂在 `TerminalResultKind` 上**）：
1. **auto-resume 的判据是字节级子串，与 `TerminalResultKind` 无关**：controller 对本轮新增 message 的 `server_message_data` 做**原始子串**匹配 `"_byop_intercepted":true` 或 `"error":"invalid_arguments"`（`controller.rs:3758-3773`，含引号、冒号后无空格）才触发 resume。故 running 帧不得出现这两个子串。
2. **running 帧必须是合法 JSON**：`persisted_tool_result_kind`（`chat_stream.rs:699-740`）在解析失败且原文含上述串时返回 `UnreadableLocalInterception` —— 那是**唯一** `satisfies_readiness() == false` 的 kind（`app/src/ai/byop_readiness/mod.rs:198-208`），会导致请求被 blocking（`controller.rs:2335-2348` + serializer `Err`，`chat_stream.rs:1108-1133`）。注意 `Real` 与 `LocalInterception` 的**后果完全相同**（都通过 readiness），不可用作行为区分。
3. **必须原地更新，不能另追加一条结果**：`controller.rs:2754-2778 has_persisted_tool_result` 只按 `(task_id, tool_call_id)` 判断 message 是否存在、**不看内容**，且被 `:2410-2429`（剔除 current-input 结果）与 `:2489-2497`（append 前跳过）使用 ⇒ 若先落了结果帧、真结果又以 current-input ActionResult 到达，会被判"已持久化"而**静默丢弃**。旁证：`app/src/ai/byop_compaction/message_view.rs:161` 把"非空 `server_message_data`"直接算作 `completed`，无"进行中"概念。

**决策**：不新增第三套标记（避免与既有 `TerminalResultKind` 语义纠缠），仅靠"合法 JSON + 不含两个保留子串 + 原地更新"达成。

**D4｜pending 编排（必须自写）**
乙下父的回合发完即结束，因此"派 3 个、全回来再总结"要自己实现：维护 `pending: HashMap<child_conversation_id, (parent_conversation_id, parent_tool_call_id)>`，子会话进入终态 → 更新父侧结果帧 → 从 pending 移除 → **pending 对该父会话为空时** `resume_conversation(parent)`（`app/src/ai/blocklist/controller.rs:1667`）。

`resume_conversation` 已核实可用：它是**请求生成路径**（发 `AIAgentInput::ResumeConversation`），**不要求当前存在 in-flight stream** ⇒ 父回合结束后再 resume 成立。

**为什么不复用现成编排**（逐项核实过，避免后人重走）：
- **模型甲的判定作用域封闭**：`has_active_subagent()` 遍历**本会话**的 task store（`app/src/ai/agent/conversation.rs:3054-3062`），`is_subagent_task_finished` 在**父任务 messages** 里找匹配 `tool_call_id` 的 `ToolCallResult` ⇒ 子会话是另一个 `AIConversation` + 另一个 `TaskStore`，甲对其**全盲**。
- **甲也没有"编排器"组件**：本地"子代理完成 → 父继续"是 LRC 专用钩子 + 排队 prompt 启发式（`app/src/ai/blocklist/block/cli_controller.rs:337-410`、`app/src/terminal/view.rs:4500-4521`），不是通用机制；research/advice 型子任务的"继续"在上游由服务端驱动。
- **上游 `orchestration_child_tracker.rs` 不可移植**：本地 MISSING，其模块文档头明写由 **SSE 事件**驱动（父的 SSE 流发 `child_agent_started` 才建占位）、"pushes the server cursor"、REST seed rows ⇒ 依赖服务端协议。
- **"双轨"不划算**：为借用甲的两个判定而真建 subtask，会把 `Tool::Subagent` 通道、`chat_stream.rs:1452`/`:620-623` 的 skip 收窄、`new_subtask` 顺序硬约束／静默 `params=None`／`expect` panic、conversation 单槽位全部请回来，只为省本文 D4 的工作量。

**D4.1｜借用甲的"回合门控"语义（语义借用，不是组件复用）**
甲真正值钱的是"子代理没跑完，父回合不算完成"。乙下父回合立刻收尾，体验上会出现"父看似已说完、稍后又冒出一段总结"。借法（**不动 conversation 层查询**，避免污染 LRC 语义）：仅改 `app/src/terminal/view.rs` 的 **4 处本地判定**（`:4452`、`:4514`、`:4838`、`:4858`），把 `conversation.has_active_subagent()` 扩为 `|| 本会话存在活跃子会话`（对话树索引 `children_by_parent` 现成：`app/src/ai/blocklist/history_model.rs:201`、`:402`）。成本 0.5–1 人日。

**D5｜权限与父一致**
子会话创建后立即传播：父 profile id → child terminal view id（`AIExecutionProfilesModel::set_active_profile`）、父 base model id → child view id（`LLMPreferences::update_preferred_agent_mode_llm`），参考 `propagate_parent_agent_settings`。manifest 的 `model` 参数仅在显式给出时覆盖。因权限一致 ⇒ **子代理可写文件**，故审批可见性必须处理（步骤 7）。**注意**：仅传播 profile 不足以保证权限一致——还需 **D6** 的 autoexecute 对齐。

**D6｜子会话的 autoexecute 覆盖必须显式对齐父（安全项，勿照抄参考实现）**
参考实现 `child_agent.rs` 调的是 `start_new_child_conversation(..., auto_execute = true, ...)`，其效果是 `toggle_autoexecute_override()` ⇒ 该对话被置为 `AIConversationAutoexecuteMode::RunToCompletion`（`app/src/ai/blocklist/history_model.rs:349-379`、`:744-754`；`app/src/ai/agent/conversation.rs:3361-3367`、`:4415-4425`）。该状态**直接进权限判定**：`autoexecute_any_action()` 共 8 处消费，其中 `app/src/ai/blocklist/permissions.rs:467` 使写 PTY 直接返回 `WriteToPtyPermission::AlwaysAllow`，`:864` 还会与 `auto_approve_bypasses_command_denylist` 组合**绕过用户命令 denylist**（其余见 `permissions.rs:635`、`:712`、`:796`、`block.rs:2601`、`status_bar.rs:833`、`cli.rs:1053`、`agent_view/agent_input_footer/mod.rs:1706`）。

⇒ **照抄参考实现会让子代理权限高于父**，与"权限与父一致"的决策冲突。
**规则**：子会话用 `is_autoexecute_override = false` 创建，然后**按父对话的实际 `autoexecute_any_action()` 显式复制**（父为 `RunToCompletion` 才置 `RunToCompletion`），**不得无条件 `toggle`**。`AIConversationAutoexecuteMode` 与 `toggle_autoexecute_override` 均为 pub，可读父、写子。

## Approach

### 阶段一（模型乙核心）

#### 步骤 1 — 工具注册与 carrier 拦截
- **位置**：新增 `app/src/ai/agent_providers/tools/subagent.rs`；改 `tools/mod.rs:76-103`（REGISTRY）；`chat_stream.rs:2713`（`build_tools_array`）；`chat_stream.rs:4330` 一带（按 name 拦截）。
- **逻辑**：`OpenAiTool{ name:"subagent", description, parameters: fn()->Value, from_args: Err("intercepted at byop layer"), result_to_json }`；拦截分支解析 `{prompt, name?, model?}`。
- **验证**：单测 registry lookup + schema 快照 + 拦截后产出 carrier 与 running 帧。

#### 步骤 2 — 派发事件（跨层解耦）
- **位置**：`chat_stream.rs`（yield）+ `app/src/ai/blocklist/history_model.rs` / `controller.rs`（消费）。
- **逻辑**：chat_stream 不能直接建会话/建 pane（它只 yield api 事件），故新增一个派发事件携带 `{parent_conversation_id, tool_call_id, prompt, name, model}`，由 UI/controller 层接住执行步骤 3。是否复用既有 `BlocklistAIHistoryEvent` 或新增变体，实现时按读取结果定；**新增变体优先**（语义清晰，避免污染既有事件语义）。
- **验证**：单测断言事件载荷完整。

#### 步骤 3 — 建子会话 + 隐藏 pane + 权限对齐
- **位置**：`app/src/pane_group/mod.rs`（新增 `create_hidden_child_agent_conversation` 等价函数，参考 `fa1063715^:app/src/pane_group/child_agent.rs` 的 `create_hidden_child_agent_conversation`）、`history_model.rs:349`（复用）。
- **逻辑**：`insert_terminal_pane_hidden_for_child_agent`（`NewPaneVisibility::HiddenForChildAgent`，`app/src/pane_group/mod.rs:3386-3392`）→ 取 TerminalView → 建子会话 → **D6 的 autoexecute 对齐** + **D5 的 profile/模型传播** → 登记 `child_agent_panes` 与 D4 pending。
- **不要照抄的点**：参考实现的 `start_new_child_conversation` 固定传 `auto_execute = true`（`history_model.rs:349-379`）——见 D6，**必须按父的 override 状态显式对齐**。
- **验证**：单测断言 child conversation 存在、`parent_conversation_id` 正确、`is_child_agent_conversation()==true`、profile == 父 profile、**`autoexecute_any_action()` == 父的值**（父为默认 `RespectUserSettings` 时必须为 `false`）。

#### 步骤 4 — 子会话首轮 query 提交（用**子 pane 自己的** controller）
- **位置**：`app/src/ai/blocklist/controller.rs:1141`（`send_user_query_in_conversation_no_lrc_subagent`）。
- **逻辑**：取**子 pane 的 TerminalView** 的 `ai_controller`（字段 `pub ai_controller: ModelHandle<BlocklistAIController>`，`app/src/terminal/view/load_ai_conversation.rs:166`），由它向子会话提交 prompt。
- **为什么必须用子 controller**：`BlocklistAIController` 是**每 TerminalView 一份**——字段 `terminal_view_id: EntityId`（`controller.rs:346-356`，注释 "The ID of the terminal view this controller is associated with"），并持有该 view 的 `terminal_model: Arc<FairMutex<TerminalModel>>`（`:351`），`send_user_query_in_conversation_internal` 会锁它（`:1191`）。若误用父 controller：子会话会按 `self.terminal_view_id` 注册进**父** view，并锁定父的 TerminalModel（本仓有先例：持锁期间跨层 emit 曾致死锁，见 `block/cli_controller.rs:308-316` 注释）。
- **语义精确说明**：该函数与同文件其它 3 个包装函数的唯一差异是 `skip_running_command_detection = true`（`:1148-1158`），效果是 `running_command_opt = None`（`:1203-1207`）⇒ 跳过 `create_cli_subagent_task_for_conversation`（`:1209-1224`），即**不因前台长跑命令而把该 query 送进 CLI 子代理**。这正合"子代理是独立会话、不要 LRC 子任务"的语义。
- **验证**：单测/手工：子会话 messages 增长并产出 assistant 输出；断言**未**创建 CLI 子代理任务。

#### 步骤 5 — 结果回填 + pending 编排 + 回合门控
- **位置**：`app/src/ai/blocklist/controller.rs`（新模型或既有 controller 内）、`app/src/ai/blocklist/history_model.rs`、`app/src/terminal/view.rs`。
- **逻辑**：
  - （a）订阅子会话状态变化 → 终态时用 `UpdateTaskMessage`+`FieldMask` 把父侧结果帧**原地**更新为最终 JSON（含子会话最终输出摘要）。**必须原地更新**（D3 硬约束 3：另追加一条会被 `has_persisted_tool_result` 判为"已存在"而丢弃真结果）；
  - （b）从 pending 移除；
  - （c）父会话 pending 清空 → `resume_conversation(parent)`；
  - （d）**门控借用（D4.1）**：扩 `app/src/terminal/view.rs` 的 4 处判定（`:4452`、`:4514`、`:4838`、`:4858`），使"本会话有活跃子会话"期间父回合不收尾；**不得改动** `has_active_subagent()` 本体。
- **决策点**：**全部完成才 resume（默认，支持"总结"语义）** vs 每个子代理完成即 resume（可增量播报，但会有多次 resume）。本期取默认。
- **验证**：单测 pending 计数与 resume 触发次数；LRC 排队 prompt 用例（`view.rs:4452`/`:4514` 路径）保持原行为；手工确认父模型拿到结果后继续。

#### 步骤 6 — 父侧展示（两段式：先造 client representation，再渲染）
- **位置**：`app/src/ai/agent/api/convert_from.rs`（**必须先改这里**）、`app/src/ai/blocklist/block/view_impl/output.rs`。
- **已核实的前提（勿想当然）**：carrier（`tool: None`）当前**完全不渲染**——`convert_from.rs:542-552` 对它返回 `MaybeAIAgentAction::NoClientRepresentation`（原注释："UI 不渲染这条空 ToolCall"），`Message::ToolCallResult` 同样无表示（`:495-503`），随后被 `app/src/ai/agent/task.rs:1012-1016` 过滤，不进 `output.messages`；且 `output.rs` **完全不匹配** `ToolCall`（该文件 grep 零命中），只按 `AIAgentOutputMessageType` 渲染。
- **"通用工具卡"不可直接复用**：`render_inline_tool_card`（`app/src/ai/blocklist/inline_action/tool_card.rs:117`）、`render_block_tool_card`（`:171`）与 `ToolCardSpec` 全仓 **0 个调用点**（仅自身定义/注释），属死 API；只能作为**渲染风格参考**，不是接入点。
- **逻辑（两段式）**：(a) 在 `convert_from.rs` 为我们的 carrier 造一个 client representation（建议按 `server_message_data` 首行的 `fn_name` 判别，产出"子代理名 + 状态"的展示消息）；(b) 在 `output.rs` 加最小渲染分支（子代理名 + 状态 + "打开"入口；行卡片可复用 `conversation_navigation_card_with_icon`，样式参考 `tool_card.rs`）。
- **不要走的路**：`MessagesReceivedFromAgents` 在本地**只有 debug 文本渲染**（`app/src/ai/agent/mod.rs:641` 在 `[DEBUG]` 分支，输出 `"Received {} messages"`），不可用作正式 UI。
- **验证**：手工最小验证（点一次能跳到子会话）。
- **工时**：**3–4 人日**（此前估 2 偏乐观——含 `convert_from.rs` 表示层改动）。

#### 步骤 7 — 并发/取消/超时 + 审批可见性
- **位置**：`controller.rs`、状态卡、`child_agent_status_card.rs`。
- **逻辑**：多子会话**天然并发**（各自 terminal view，互不取消）；补：父取消时的策略（本期：父取消**不**连带取消子，附"停止"入口）、子代理**超时**（仓库当前无任何 subagent timeout）；因 D5 允许写文件，子代理阻塞时状态卡已显示黄色 `Blocked`（`app/src/ai/agent/conversation.rs:4492`）+ 点开可审批，追加**父侧卡片标注"等待确认"**避免漏看。
- **验证**：单测取消/超时；手工制造一次需确认的子代理操作并确认可见可审批。

#### 步骤 8 — 持久化与恢复
- **位置**：`app/src/ai/blocklist/history_model/conversation_loader.rs:233-252`、`app/src/pane_group/mod.rs:2550-2610`。
- **逻辑**：子会话本就随 conversation 落库（`parent_conversation_id` 已序列化，`app/src/ai/agent/conversation.rs:3173`）；恢复时被剔出历史列表（`conversation.rs:1255`）并按 parent 索引重建隐藏 pane。**需处理**：启动急建（`TODO(QUALITY-378)`）与"恢复后 pending 是否还成立"（重启后未完成的 pending 应按终态处理或标失败，避免父永久等待）。
- **验证**：重启往返；中间重启一次确认父不会永久挂起。

#### 步骤 9 — 端到端验证
- 跑 `## Verification` 全套，记录结论。

### 阶段二（pill bar 线，与乙同源）

#### 步骤 10 — 移植 pill bar 三件套
- **来源** `git show upstream/master:<path>`：
  - `agent_view/orchestration_pill_bar.rs`（2917）
  - `agent_view/orchestration_pill_bar_model.rs`（133，零云端依赖，入参 `HashSet<AIConversationId>`）
  - `blocklist/orchestration_topology.rs`（492，全部 `AIConversationId` 签名）
  - `agent_view/orchestration_avatar.rs`（41）
  - 配套测试（760 + 1317 + 147 + 11）一并移植，作为验证资产
- **注意**：本地 `orchestration_conversation_links.rs` 已被改名裁剪为 `conversation_navigation_links.rs`（仅 1 个函数），须**合并**上游 4 个函数（`is_conversation_open_in_other_visible_view`、`pane_group_id_containing_terminal_view`、`dispatch_focus_or_open_child_agent_pane`、`parent_conversation_id`），**不要覆盖**本地文件。

#### 步骤 11 — 组件适配
- `app/src/ui_components/icon_with_status.rs`：本地已重构（260 行，含 `render_with_optional_status_badge`/`IconWithStatusSizing`），上游 503 行有额外徽章几何（`StatusBadgeStyle`/`BadgeInnerShape`）⇒ **改调用点适配，不复制文件**。
- 补 `TerminalAction::{OpenChildAgentInNewPane, OpenChildAgentInNewTab}` 变体 + handler（当前全仓不存在）。
- 剥掉 5 个遥测类型（`BlocklistOrchestrationTelemetryEvent` 等）而非移植。

#### 步骤 12 — 接线
- `agent_view/mod.rs` 声明新模块；pane/workspace action handler；pill 与父子会话联动。

#### 步骤 13 — 阶段二验证
- 移植测试全绿 + 手工点开/切 pane。

## Critical files & anchors

- `app/src/ai/agent_providers/tools/mod.rs:76-103` — BYOP REGISTRY（步骤 1 插入点）；`:98-102` 本地拦截范式
- `app/src/ai/agent_providers/chat_stream.rs:5203-5223` — `make_tool_call_carrier_message`（D1 载体）
- `app/src/ai/agent_providers/chat_stream.rs:2374-2396` — 出站从 `server_message_data` 复原工具名（D1 回放）
- `app/src/ai/agent_providers/chat_stream.rs:4330-4420` — web 工具拦截范式（两帧 + FieldMask 原地更新，D3 参考）
- `app/src/ai/blocklist/controller.rs:3758-3773` — auto-resume 触发条件（**D3：字节级子串匹配**，非 `TerminalResultKind`）
- `app/src/ai/byop_readiness/mod.rs:198-208` — `satisfies_readiness`（`Real`/`LocalInterception` 同后果；仅 `UnreadableLocalInterception` 阻塞）
- `app/src/ai/blocklist/controller.rs:2754-2778` — `has_persisted_tool_result` 只看 `(task_id, tool_call_id)` 存在性（**D3 硬约束 3**）
- `app/src/ai/byop_compaction/message_view.rs:161` — 非空 `server_message_data` 即算 `completed`
- `app/src/ai/blocklist/permissions.rs:467`、`:864` — `autoexecute_any_action()` ⇒ `AlwaysAllow` / 可能绕过 denylist（**D6 安全依据**）
- `app/src/ai/blocklist/controller.rs:346-356`、`:351` — `BlocklistAIController` 的 `terminal_view_id` / `terminal_model`（**每 TerminalView 一份**；步骤 4 判据）
- `app/src/terminal/view/load_ai_conversation.rs:166` — `pub ai_controller`（步骤 4 取子 controller）
- `app/src/ai/agent/api/convert_from.rs:542-552`、`:495-503` — carrier 与 ToolCallResult 的 `NoClientRepresentation`（**步骤 6 依据**）
- `app/src/ai/agent/task.rs:1012-1016` — 无表示消息被过滤
- `app/src/ai/blocklist/inline_action/tool_card.rs:117`、`:171` — 工具卡入口，**全仓 0 调用点**（仅作风格参考）
- `app/src/ai/blocklist/history_model.rs:349-379` — `start_new_child_conversation`（复用；注意其 `auto_execute` 参数，见 D6）
- `app/src/pane_group/mod.rs:2614-2645`、`:3371-3397` — 隐藏 child pane 恢复/插入（步骤 3 挂点）
- `app/src/ai/blocklist/controller.rs:1141` — `send_user_query_in_conversation_no_lrc_subagent`（步骤 4 复用）
- `app/src/ai/blocklist/agent_view/child_agent_status_card.rs:179-255` — 列表卡（**零改动**）
- `app/src/pane_group/pane/terminal_pane.rs:945-952` — reveal 链路（**零改动**）
- `app/src/ai/agent/conversation.rs:1066`、`:1255`、`:3173` — child 判定 / 导航排除 / 持久化
- `app/src/ai/agent/conversation.rs:3054-3062` — `has_active_subagent`（**D4 证据：作用域封闭，乙不可复用**）
- `app/src/ai/blocklist/controller.rs:1667-1710` — `resume_conversation`（D4：请求生成路径，不要求 in-flight stream）
- `app/src/terminal/view.rs:4452`、`:4514`、`:4838`、`:4858` — 门控的 **4 个消费点**（D4.1 改动点；勿动 `has_active_subagent` 本体）
- `app/src/ai/blocklist/history_model.rs:201`、`:402` — `children_by_parent` 索引（D4.1 判定数据源，现成）
- `app/src/pane_group/mod.rs:2550-2610` — 启动急建（`TODO(QUALITY-378)`，步骤 8）
- 不可移植参照（上游，本地 MISSING）：`app/src/ai/blocklist/orchestration_child_tracker.rs` — 模块头明写 SSE 驱动（`child_agent_started`／server cursor／REST seed），依赖服务端协议
- 参考实现（已删，仅供对照）：`fa1063715^:app/src/pane_group/child_agent.rs`（`create_hidden_child_agent_conversation`、`propagate_parent_agent_settings`）

## 写入域与并行

| Owner | 独占写入域 | 覆盖步骤 |
|---|---|---|
| Owner-1 | `app/src/ai/agent_providers/**`（尤其 `chat_stream.rs`） | 1、2(发送侧) |
| Owner-2 | `app/src/ai/blocklist/**`（`controller.rs`、`history_model.rs`、`block/view_impl/**`）、`app/src/ai/agent/api/**`（步骤 6 的 `convert_from.rs`）、`app/src/terminal/view.rs` | 2(接收侧)、5、6、7 |
| Owner-3 | `app/src/pane_group/**` | 3、4、8 |
| Owner-4（阶段二） | `app/src/ai/blocklist/agent_view/**`、`ui_components/**`、`pane_group/**` | 10–12 |

⚠️ `chat_stream.rs`、`controller.rs` 为热点，**单一 owner**。接口先冻结（D1 载体格式、D4 pending 结构、步骤 2 事件载荷），再并行；否则阶段一按 **inline 单 owner** 推进，仅步骤 6 与阶段二做并行。

## Verification

**命令**
```bash
cargo check
cargo nextest run -p warp -E 'test(subagent)'
cargo nextest run -p warp -E 'test(cli_subagent)'   # LRC 回归，必须全绿
```
（本仓一律用 nextest；`cargo test` 宽过滤会因全局静态假失败）

**手工最小验证（点击级）**：配好 BYOP provider → 打开 Oz agent → 发"用子代理分头查 A 和 B，然后总结" → 期望：父对话出现子代理卡 → 状态卡出现 2 行子代理 → 点一次跳到子会话且能看到其完整过程 → 父对话收到子代理结果 → 父给出总结。

**权限核对（D6，必做一次）**：父为默认 `RespectUserSettings` 时，让子代理做一次需确认的写操作（如改文件）→ 期望**仍需确认**（与父一致）；若子代理静默执行或绕过 denylist，即为 F1 回归，按 Risks 停线。

**必守回归**：LRC tag-in 浮窗行为不变；历史会话恢复不错位（该区域此前踩过坑，见 `specs/upstream-merge-lessons.md` §43）；`agent_conversations`/`agent_tasks` 与会话恢复契约不被破坏。

## Risks & stop conditions

| 风险 | 停线条件 |
|---|---|
| running 帧被误判触发 auto-resume | 父在子代理未完成时就继续 → 停下，检查 `server_message_data` 是否含**字节级** `"_byop_intercepted":true`/`"error":"invalid_arguments"`（`controller.rs:3758-3773` 按子串匹配，与 `TerminalResultKind` 无关） |
| running 帧非法 JSON → 请求被 blocking | 请求直接失败（`UnreadableLocalInterception`，`byop_readiness/mod.rs:198-208` + `controller.rs:2335-2348`）→ 停下，先保证 JSON 合法且不含两个保留串 |
| **真结果被静默丢弃** | 父永久停在 running、真结果不出现 → 停下，确认结果只走"原地更新同一条 message"（`controller.rs:2754-2778` 只看存在性，不看内容） |
| **子代理权限高于父（D6）** | 手工验证发现子代理未确认即写文件、或绕过命令 denylist → 停下，核对子会话 `autoexecute_any_action()` 是否等于父 |
| 父永久等待（pending 不收敛） | 手工验证卡住 → 先修步骤 5 的收敛/终态判定 |
| 重启后 pending 残留 | 重启往返失败 → 停下，补终态/失败兜底 |
| 隐藏 pane 内审批用户漏看 | 手工验证不可见 → 先补可见性，再继续 |
| 子会话历史恢复错位 | 恢复用例失败 → 停下，不带病推进 |
| 隐藏 pane 急建导致启动变慢 | 子代理数量 ≥5 时明显卡顿 → 先做按需加载 |
| **D4.1 门控改动影响 LRC** | 4 处判定改动后，LRC 排队 prompt（`view.rs:4452`/`:4514` 路径）行为变化 → 停下，回退为新增并列判定而非改写原判定 |
| 用错 controller（步骤 4） | 子会话被注册进父 view、或父 TerminalModel 被锁 → 停下，改用子 pane 的 `ai_controller` |

## 退役与保留

- **保留**：LRC CLI subagent 全链路（生产在用，与本期两条独立通道互不影响）；`Tool::Subagent` 通道维持现状（本期不用、不改其 skip 判据）。
- **退役**：阶段二落地后，用上游版本替换并删除本地孤文件 `app/src/ai/blocklist/agent_view/orchestration_pill_bar.rs` 的旧副本（避免两份），并补 `agent_view/mod.rs` 声明。
- **不做**：不恢复云端类型；不引入 orchestration config 层。

## Assumptions & contingencies

- **A1｜并发**：子会话天然并发，但**同一次派发的多个子代理全部完成后才 resume 父**（summary 语义）。若改为"逐个完成逐个播报"，只需改 D4 的收敛条件，不影响其余设计。
- **A2｜载体**：`server_message_data` 首行 `fn_name` + 次行 JSON 的编码沿用 web 工具约定；若与既有多行 args 约定冲突（args 内含换行），改用 `\n` 只切一次的实现（`split_once` 已保证），args 解析失败时按既有 fallback 传 `{}`。
- **A3｜子会话上下文**：乙的子代理是**全新会话、但不必然空上下文** —— 首轮由我们提交 prompt（父上下文靠 prompt 传递，与 Claude Code `Task` 一致）；此后若用户或父再向该子会话发消息（D4 的继续对话），它会自然带着自己此前的历史。若需要"子代理首轮即继承父的完整前置上下文"（模型甲的优势），本期不做，留后续评估。
- **A4｜恢复**：子会话随 conversation 落库，判定其"终态"以 `ConversationStatus` 为准（Success/Error/Cancelled/Blocked）。
- **A5｜门控借用**：假设扩 `terminal/view.rs` 的 4 处判定即可让父回合在子代理跑完前不收尾，且不改动 LRC 语义（因原判定在 `has_active_subagent()` 本体之外、LRC 依赖的是本会话子任务判定）。**未实测**；若 LRC 行为变化，回退为"新增并列判定"（见 Risks）。
- **A6｜父侧展示路线**：步骤 6 的"在 `convert_from.rs` 为 carrier 造 client representation"是实现路线推断（**未实测**该层改动的波及面）；若该层代价过大或波及既有 tool 表示，回退方案是走另一条通道（例如由步骤 2 的派发事件顺带自建一条展示消息），保底仍能"点开子会话"。
- **A7｜权限对齐**：D6 依赖"父对话的 `autoexecute_any_action()` 可读、子可写"（两者均 pub）。**未实测**；若子会话在创建瞬间尚无平台可写，则在子会话首次提交 query 前完成对齐。
- **未验证推测**：阶段一 26–31 人日、阶段二 8–12 人日、合计 34–43 人日，为基于改动面的推导估算，未编译未运行；"同一次派发多子会话并发"、A5 门控借用、A6 表示层路线、A7 权限对齐时机均未实测。

## 工时

| 阶段 | 内容 | 人日 |
|---|---|---|
| 一 | 步骤 1–4：工具注册／carrier／派发事件／子会话+隐藏 pane／首轮 query | 9–10 |
| 一 | 步骤 5：结果回填 + pending 编排 + 回合门控（**含 D4.1 的 0.5–1**） | 4–4.5 |
| 一 | 步骤 6–8：父侧展示（**3–4**，含 `convert_from.rs` 表示层）／并发取消超时与审批可见性／持久化恢复 | 9–10.5 |
| 一 | 步骤 9：端到端验证 | 2.5–3 |
| 一 | **小计（含缓冲）** | **26–31** |
| 二 | 步骤 10–13（pill bar 线，可独立验收/可裁剪） | 8–12 |
| — | **合计** | **34–43** |
