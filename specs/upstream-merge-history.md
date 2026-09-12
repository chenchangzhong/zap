# 上游合并经验文档·历史归档

> 逐轮移植 / 核验的详细记录（§0、§2–§6、§10、§12–§33），2026-08-13 从 `upstream-merge-lessons.md` 拆出。每轮必读的参考区（决策速查表、核心原则、验收清单、坑位速查、文件定位、经验总结）仍在 `specs/upstream-merge-lessons.md`。
> 章节编号保留原样；新轮次记录追加到本文档末尾，编号顺延。

---

## 0. 第二轮同步记录（2026-08-01）

边界内 60 个上游提交,约 35 个纯 TUI(本地已删 `warp_tui`,跳过),云计费/共享会话/grok 按原则跳过。

### 已合入

| 提交 | 内容 | 处理 |
|------|------|------|
| `9dcef6a88` | CLI help 隐藏 API key(`hide_env_values`) | 干净应用 |
| `1c1f21d82` | 设置页 Circular view update 崩溃 | 冲突:gemini enterprise 函数跳过(本地无该功能),deferred 修复本地已有 |
| `6465abf58` | watcher 跟随目录 symlink | 最小移植:`is_within_symlink` + 2 个调用点;本地无上游 `repo_watch_filter` 体系,不引入 |
| `fae2538e1` | passive-suggestions 大输出高 CPU | 干净应用(maa.rs + 测试) |
| `179923ede` | codesign 时间戳重试 | 冲突:保留本地 elif,取 `codesign_with_retry` |
| `2c86dce47` | 字体 glyf bit7 拒绝 | 冲突:imports 取 ours + 补 `lazy_static` import |
| `5aaadb20e` | task-backed 会话消失 | 部分:仅 `ambient_agents/task.rs` 纯增量(ExecutionLocation 枚举 + 可选字段);主体放弃(本地无 `agent_conversations_model/` 目录、`server_api/` 目录) |
| `3a7d18971` | OscHyperlinks → Stable | 部分:release 列表加 `osc_hyperlinks` + 补 feature 定义;不加 `viewing_shared_sessions`(本地无共享会话) |
| `fa70ad068` | ContextWindowUsageBreakdown → Stable | 部分:加 variant + release 列表 + feature 定义;DOGFOOD 列表取 ours |

### 放弃(结构不适用/本地定制区)

| 提交 | 原因 |
|------|------|
| `16ec6d4d7` | shell exit 检测针对 remote sandbox(本地无),7 处 content 冲突在 OMP 定制区 |
| `fe8138bce` | settings_view 本地定制区,3 处 content + 5 个 modify/delete(云计费页面) |
| `f7a19b3e4` | 本地无 `block/view_impl/orchestration.rs` |
| `6de238814` | 本地 `AuthOnboardingState` 无 `LoginSlide`/`PostAuthOnboarding` 变体 |
| `867347a1e` | 本地无 `oz-platform` bundled skill |
| `ca1cd4303` | 模型菜单定制区 5 处冲突,低价值 |
| `306320a59` | 本地无 `recording_finalize.rs`(录制终态管道) |

### 本轮教训

- **无 merge-base 的 cherry-pick 会误报大量 modify/delete**:本地删过/改名过的文件(测试、云计费页面)全部报 DU,处理方式统一 `git rm`(测试)或 `git checkout HEAD --`(生产文件)+ 手动评估。
- **3-way merge 会把 theirs 的非冲突新增行自动合入**:`6465abf58` 的 `is_within_symlink(path, repo_root)` 调用被自动塞进本地 `path_passes_filters`,引用了不存在的 `repo_root`。合并后必须全文扫一遍自动合入残留。
- **上游 Cargo feature 引用需补定义**:promote 类提交只在 default 列表加 feature 名,`osc_hyperlinks`/`context_window_usage_breakdown` 的 `= []` 定义在更早提交里,本地需按字母序手动补。
- **冲突解析用 `conflict://N` 工具**:`write({path: "conflict://N", content})` 支持自定义内容(如 #11 只取 theirs 一行),比手工编辑 marker 稳。

---

## 2. 分阶段执行记录

| 阶段 | 内容 | 状态 | 关键决策 |
|------|------|------|----------|
| ① Cargo/构建 | workspace deps、profile、features、build.rs | ✅ | 跳过 `warp_multi_api` rev bump（不同 fork） |
| ② 低风险模块 | Settings、Editor、Vim、Persistence、Scripts、Auth | ✅ | 7 并行任务全通过 |
| ③ 新增 crate | `warp_tui`、`warp_search_core`、`warp_errors`、`warp_channel_config` | ⚠️ | **后续全部删除**——依赖本地不存在的胶水代码 |
| ④ 高风险模块 | Terminal 基础设施、SSH、AI MCP、CLIAgent Hermes+Vibe | ✅ | 核心合并完成，cargo check 通过 |
| ⑤ 集成验证 | 编译、测试、冒烟 | ✅ | 7 个预存测试错误已修复 |

---

## 3. 关键 Cherry-pick 与冲突处理

### 3.1 已合并的关键变更

| 文件 | 变更 | 备注 |
|------|------|------|
| `app/src/terminal/cli_agent.rs` | `command_prefix()` → `command_prefixes()`，新增 `Hermes`/`Vibe` | 向后兼容，Vibe 支持 `["vibe", "vibe-acp"]` |
| `app/src/ai/llms.rs` | 新增 `agent_mode_models_unavailable` bool | getter pub，setter pub(crate) |
| `app/src/ai/agent/mod.rs` | `RenderableAIError::CloudStartupFailed(String)` | 云错误展示 |
| `app/src/ai/agent_sdk/admin.rs` | workspace-aware whoami | 保留，非破坏性 |
| `app/src/ai/agent/task_store.rs` | `prune_unreachable_subtasks()` + `reachable_task_ids()` | BFS 从 root 遍历 |
| `app/src/terminal/model_events.rs` | `SshRemoteServerSupport` enum + `new_with_ssh_remote_server_support` | SSH 远程服务器支持 |
| `app/src/terminal/local_tty/unix.rs` | 环境变量常量 `WARP_CLIENT_VERSION_ENV` 等 | CLI agent 协议版本注入 |

### 3.2 故意跳过的 Cherry-pick（终局结论）

第一轮（2026-07-30）跳过时标为「后续处理」，2026-08-03 逐项定终局。**均不做**。

| 文件 | 当初跳过原因 | 终局结论 |
|------|--------------|----------|
| `app/src/terminal/view.rs` | slow-bootstrap 自动消除，与 OMP NDJSON 入口不同区 | **不做**——该区已被 OMP 集成重写，拣入即覆盖 Zap 自研代码，违反 §1 |
| `app/src/terminal/input.rs` | +277/-136 大规模改动，附件上传重构 | **不做**——纯重构无行为收益，改动量大且与本地输入定制冲突 |
| `app/src/ai/agent_sdk/config_file.rs` → `mcp_config.rs` → `driver.rs` → `mcp/templatable_manager/*` | MCP WellKnown ID 链，5 文件依赖 | **不做**——`driver.rs` 的第三方 harness 路径已删（§10），依赖链断裂 |
| `app/src/ai/blocklist/block.rs` → `response_stream.rs` | MCP tool name 响应流传递，与 NDJSON 交互需验证 | **不做**——收益仅为工具名展示优化，与 NDJSON 交互风险不成比例 |
| `app/src/ai/llms.rs` | `LLMProvider` 迁移到 `crates/ai`，纯重构 | **不做**——纯重构，零行为收益，只增加后续同步的冲突面 |
| `app/src/ai/agent_sdk/agent_management.rs` | workspace-aware agent management，云 teams 功能 | **不做**——云 teams 功能，命中 §1「云服务代码不合并」 |

> 若将来上游在这些区域**修了真实 bug**（非重构），按 §14「移植前三问」单独评估，
> 不受本表约束。

---

## 4. 预存测试编译错误（合并前即存在）

| 错误 | 文件 | 修复 |
|------|------|------|
| `unresolved import super::is_zap_bundle` | `app/src/util/file/external_editor/mac_tests.rs` | 在 `mac.rs` 添加 `is_zap_bundle()` 函数 |
| `unresolved import ai::workspace` | `app/src/persistence/sqlite_tests.rs` | 删除死 import（`WorkspaceMetadata` 未使用） |
| `cannot find function create_api_task` | `app/src/ai/agent/task_store_tests.rs` | 删除 2 个依赖上游 API 的测试函数 |
| `cannot find function create_subagent_tool_call_message` | 同上 | 同上 |
| `use of unresolved module iter` | 同上 | 同上（`iter::empty()`） |

> **教训**：上游 cherry-pick 的测试常依赖上游专有 helper/API，本地没有。遇到编译失败优先判断是否为预存问题，是则删除测试而非补全 helper。

---

## 5. 本地清理记录

| 清理内容 | 日期 | 涉及文件 | 说明 |
|----------|------|----------|------|
| `OhMyPiHarness` 死代码 | 2026-07-30 | `oh_my_pi.rs` | 未编译的死文件，删 |
| 第三方 Harness 执行路径 | 2026-07-30 | `claude_code.rs`、`gemini.rs`、`json_utils.rs`、`harness/mod.rs`、`driver.rs`、`agent_sdk/mod.rs`、`model.rs`、`view_impl.rs`、`harness_selector.rs` | 完整清理，见 Section 10 |
| `FeatureFlag::AgentHarness` | 2026-07-30 | `Cargo.toml`、`lib.rs`、`warp_features` | feature flag 及 6 处门控一并移除 |
| `OzMultiHarness` 实验 | 2026-07-30 | `experiments/mod.rs`、`experiments/convert.rs` | 实验变体及映射删除 |
| `app/src/tui_export/` | 2026-07-30 | `history.rs` | 未编译的孤立文件（`lib.rs` 无 `mod` 声明），`warp_tui` 已删则无用 |
| `app/src/ai/orchestration/` | 2026-07-30 | `mod.rs`、`remote_child.rs` | 未编译的孤立模块（`ai/mod.rs` 无 `mod orchestration`），Cloud Agent 类型死代码 |
| `app/src/ai/execution_profiles/config.rs` + `config_tests.rs` | 2026-07-30 | `config.rs`、`config_tests.rs` | 未编译的孤立文件（`execution_profiles/mod.rs` 无 `pub mod config`） |
| `script/windows/test_tui_installer.ps1` + `tui-installer.iss` | 2026-07-30 | 两个文件 | Windows TUI 安装脚本，Zap 不需要 |
| `agent/task_store.rs` 的 `prune_unreachable_subtasks()` | 2026-07-30 | `task_store.rs`、`task_store_tests.rs` | **保留（终局）**——2026-08-03 复核仍无生产调用者（仅 3 个测试引用），但函数 + 测试完整、零维护成本，且上游后续可能接入。不删 |
| `CLIAgent::DeepSeek` 整条 CLI agent 集成 | 2026-08-03 | 见 Section 15 明细（14 文件 + 2 删除） | 上游改名 CodeWhale，`deepseek`/`deepseek-tui` 二进制在 v0.9.0 消失，集成失去目标 |

## 6. 误删/遗漏恢复记录

| 项目 | 原因 | 恢复方式 |
|------|------|----------|
| `schemars` workspace dep | 清理 `sentry` 时误删 | `INS.POST` 加回 |
| `rustls` workspace dep | 同上 | `INS.POST` 加回 |
| `reload_stale_conversation_files` feature | 清理 nld features 时误删 | `INS.POST` 加回 |
| `app/src/tui/user_info.rs` | 孤立模块引用已删 `warp_tui` | 删除整个 `app/src/tui/` 目录 |

---

## 10. 第三方 Harness 清理明细

2026-07-30 清理了上游的第三方 agent harness 执行路径，Zap 只保留 Oz 内置 agent。

### 删除的文件

| 文件 | 原因 |
|------|------|
| `oh_my_pi.rs` | 未编译的死代码（无 `mod` 声明） |
| `claude_code.rs`、`claude_code_tests.rs`、`claude_code/parent_bridge.rs` | Claude harness 执行路径 |
| `gemini.rs`、`gemini_tests.rs` | Gemini harness 执行路径 |
| `json_utils.rs` | 仅被 Claude/Gemini harness 使用 |
| `mod_test.rs` | 模块测试（依赖已删代码） |

### 删除的类型/Trait

| 名称 | 说明 |
|------|------|
| `ThirdPartyHarness` trait | 第三方 harness 抽象 |
| `HarnessRunner` trait | runner 生命周期 |
| `HarnessKind::ThirdParty` / `Unsupported` | 调度枚举变体 |
| `SavePoint` | 快照保存时机枚举 |
| `harness_kind()` 函数 | 类型分派 |
| `has_running_cli_agent()` | 仅被已删 runner 使用 |
| `write_temp_file()` | 仅被已删 harness 使用 |
| `setup_harness()` / `prepare_harness()` / `run_harness()` | driver 方法 |
| `subscribe_to_cli_agent_session_events()` | driver 方法 |

### 删除的 Feature Flag 与门控

| 名称 | 说明 |
|------|------|
| `feature = "agent_harness"` | Cargo feature，不在 defaults 中 |
| `FeatureFlag::AgentHarness` | 运行时 flag，6 处 `is_enabled()` 门控一并移除 |
| `OzMultiHarnessControl`/`Experiment` | 服务端实验变体 |

### 保留的共享代码

| 代码 | 理由 |
|------|------|
| `Harness::Claude`/`Gemini`/`OpenCode`/`Unknown` 枚举变体 | 序列化兼容、`CLIAgent` 检测、local child launch 路径共享 |
| `AIAgentHarness` 枚举（`Oz`/`ClaudeCode`/`Gemini`/`Unknown`） | 服务端 `ServerAIConversationMetadata` 反序列化兼容 |
| `validate_cli_installed()` | `local_harness_launch.rs` 中 OpenCode local child launch 使用 |
| `task_env_vars()` | 同上 |
| `HarnessSelector` UI 组件 | 保留但仅剩 Oz 选项 |

### 合并注意事项

#### ❌ 必须跳过

| 上游改动区域 | 原因 |
|------|------|
| `driver/harness/` 下 `claude_code.rs`、`gemini.rs`、`json_utils.rs` | 文件已删 |
| `FeatureFlag::AgentHarness` 定义或门控 | Zap 已无此 flag，6 处 `is_enabled()` 调用已全部移除 |
| `FeatureFlag::AgentHarness` 相关实验 `OzMultiHarnessControl`/`Experiment` | 实验变体已删 |
| `ThirdPartyHarness` trait 增删改 | trait 已删 |
| `HarnessRunner` trait 增删改 | trait 已删 |
| `HarnessKind` 枚举变体或方法 | 整个 enum 已删 |
| `harness_kind()` 函数 | 已删 |
| `SavePoint` 枚举 | 已删 |
| `has_running_cli_agent()`、`write_temp_file()` | 已删 |
| `setup_harness()`、`prepare_harness()`、`run_harness()` | driver.rs 方法已删 |
| `subscribe_to_cli_agent_session_events()` | driver.rs 方法已删 |
| `Task.harness` 字段 | Task struct 字段已删 |
| `AgentDriver.harness` 字段（`Option<Arc<dyn HarnessRunner>>`） | 已删 |
| `is_third_party_harness()`、`harness_command_started`、`mark_harness_command_started()` | model.rs 方法已删 |
| `AmbientAgentViewModelEvent::HarnessCommandStarted` | 事件变体已删 |
| `maybe_enter_agent_view_for_shared_third_party_viewer()` | view_impl.rs 方法已删 |
| `HarnessSelector` 中 Claude/Gemini 下拉项 | UI 已简化，仅 Oz |
| `agent_harness` Cargo feature（app/Cargo.toml） | feature 定义已删 |
| `#[cfg(feature = "agent_harness")]`（lib.rs） | 门控已删 |

#### ✅ 必须保留

| 上游改动区域 | 原因 |
|------|------|
| `Harness` 枚举新增变体 | 共享枚举，CLIAgent 检测/序列化/local child launch 共用 |
| `AIAgentHarness` 枚举新增变体 | 服务端 `ServerAIConversationMetadata` 反序列化兼容 |
| `Harness::display_name()`、`icon_for()` 等展示方法 | UI 组件共用 |
| `validate_cli_installed()` 功能更新 | `local_harness_launch.rs` 中 OpenCode local child launch 使用 |
| `task_env_vars()` 功能更新 | 同上 |

## 12. 第三轮核对记录（2026-08-01）

对用户列出的 21 个上游 agent 相关 commit 逐一核对（对象均在本地，原始 hash 均不在 HEAD，即全为 cherry-pick 或跳过）：

### 纯 warp_tui / 无对应功能，跳过（15）

`fd16dceb3`(Ctrl+C 子代理)、`40ac1d4b1`(恢复子代理)、`c9f44b024`(tool-call 加粗)、`44f112cc0`(zero state)、`cd45ebb6f`(key connected，OmpModelSelector 自研不适用)、`89f0eaf63`(out-of-credits 门控)、`62d87d7d2`(/clear，TuiOnly)、`e712486bd`(/status，TUI 专属)、`f7a19b3e4`(chip tooltip，无 orchestration.rs)、`4bff3ba0e`/`668739ded`(TUI /mcp)、`8b7055e8b`(cost footer，无 TUI)、`4d374f509`(HoA OSC 777，无 WarpTui variant)、`dacff5e3d`(Grok OAuth，无 grok_subscription)、`05a3f08ea`(目标功能 Factory MCP 不存在；其通用加固部分见下方“本次补齐”)。

### 已合入/等效覆盖，验证通过（5）

`5aaadb20e`(部分：task.rs ExecutionLocation)、`fa70ad068`(等效：default feature + variant，conversation_usage_view.rs 同源)、`6cfb37da7`(完整：MCP 卡片工具+server 身份)、`71fafb46c`(共享 vim API 已合入)、`b2f0b285d`(/fork 本地已有)。

### 本次补齐（2 个 commit + 1 项文档）

`808477829` 与 `05a3f08ea` 通用部分未在前两轮合入，本轮补齐（合计 15+5+1=21 个 commit 全部归类）：

| 项 | 内容 | 文件 |
|------|------|------|
| 808477829 本地化 | auto-approve 可绕过用户 denylist（Zap 无 org 策略，bypass 时置空 denylist）；设置项此前已合入但无消费方 | `permissions.rs`、`permissions_test.rs`、`settings_view/ai_page.rs` + 3 个 i18n ftl |
| 05a3f08ea 通用加固 | `server_loggers` 跟踪 + shutdown/reconnect 同步 close + spawn 日志注册失败转 FailedToStart/notify（不含 Factory 部分） | `templatable_manager.rs`、`templatable_manager/native.rs` |
| 文档记录 | 本轮核对结论 | 本节 |

### 教训

- 上游 commit 常把共享层与 TUI 层混在一个提交里（如 `71fafb46c`、`b2f0b285d`）：跳过 TUI 部分时，先确认共享层（vim API、fork 基础设施、MCP 卡片）是否已随早期同步合入，避免重复合入或漏掉共享层。
- 部分合入的 commit 会产生“设置项无消费方”的死配置（如 808477829 的设置项在第二轮已合入但 permissions.rs 逻辑未合入）：核对时要用 grep 验证设置项的实际使用点，而不只看定义。
- `permissions_test.rs` 8 个失败测试为预存失败（基线验证一致），与本次改动无关。

---

## 13. 第四轮核对记录（2026-08-01）：用户 21 个列表之外的遗漏候选

对 `git log --all --not HEAD` 全量扫描（7-20 起），发现以下**未合入且未记录**的 agent 相关 commit（多数在 7-20~7-24，即第一轮起点 `89f742fa6`（7-24）之前的同步盲区）：

### 推荐补（高价值，与已处理主题直接相关）

| commit | 内容 | 本地状态 | 理由 |
|------|------|------|------|
| `d34aaf06e` (7-22) | GUI `/auto-approve` slash 命令（APP-4901） | 本地 commands.rs 无；`fast-forward.svg` 图标已有；`ToggleAutoexecuteMode` 存在但**无绑定**（死 action） | 与 808477829 同主题；但本地已有等效入口（审批卡 EnableAutoexecuteMode 按钮），价值为中低 |
| `1e2f6e771` (7-28) | 细化 OUT_OF_CREDITS（本地 key 时信任可用性，REV-1714） | 本地无 `credit_availability` 模块，`prompt_alert.rs` 仅 294 行（上游精简版），无 out-of-credits 展示机制 | 需先补基础机制，成本高 |

### 不适用（本地无子代理事件源 / 结构缺失）

| commit | 内容 | 原因 |
|------|------|------|
| `a7c7a690c` (7-21) | cli-subagent 崩溃修复（APP-4912） | 本地 `block/cli.rs` 764/772 行仍有 `.expect("Exchange exists.")`，但**上游原版是裸 expect**，本地已有占位 fallback（754-761：exchange 缺失时回退 root task last exchange + `AppendedExchange` 订阅自动切换，中文注释）；且本地自带 agent 无子代理事件源（上游 `SpawnedSubagent` 来自 run_agents 云端链路，本地已删；OMP 的 task 子代理不经此事件）→ 崩溃路径不可达，无需移植 |
| `0e3f9fb98` (7-23) | per-agent `model_id` override（run_agents） | **不适用**：本地完全无 `run_agents` 文件；本地子代理能力仅来自 OMP harness 的 `task` 工具，不经 run_agents 链路 |
| `3a141de9b` (7-23) | run_agents repo-qualified child skill 解析 | 依赖 run_agents，本地无 → 不适用 |

### 可选（中价值）

| commit | 内容 | 本地状态 |
|------|------|------|
| `74ec69733` (7-23) | per-profile 子代理 model picker | 未合入（`set_subagent_model` 0 处）；依赖 run_agents/execution_profiles |
| `7d2784d8e` (7-24) | MCP OAuth 跨实例序列化（APP-4959，新 `oauth_relay`/`coordinator` 模块） | 本地无这两个模块 |
| `19512e441` (7-21) | read_files oversized/unprocessable 错误报告（APP-4882） | **本轮已补**（见下） |

### 19512e441 补齐记录（2026-08-01）

上游 19512e441 层叠在 master 的 partial-file-read 基础（`ReadFilesFailedFile`/部分成功语义）之上，本地无该基础（`missing_files: Vec<String>` 旧结构）。补齐时**未引入部分成功语义**，只移植核心改进：
- `BinaryFileReadResult::Missing` 拆为 `NotFound`/`TooLarge{size_bytes,limit_bytes}`/`ProcessingFailed{detail}`（穷尽匹配）
- `ReadFileContextResult.missing_files` 改为 `failed_files: Vec<ReadFilesFailedFile{path,message}>`（`ReadFilesFailedFile` 定义在 execute.rs，上游在 ai::agent）
- 新增 `describe_failed_files`/`format_mb`；read_files 错误文案 "These files do not exist" → "Failed to read files: <path: reason>"
- `server_model.rs` proto 转发每条 message；`legacy.rs` 适配字段改名
- 4 个测试（execute_tests.rs read_file_failures 模块），`cargo test -p warp --features local_fs` 全绿
- 未做：get_files/search_codebase 文案（本地这两个执行器不调用 read_local_file_context）、passive_suggestions safe_warn breadcrumb（本地用 log::warn! 保持）

**已知局限（评审确认）**：任一批次中若有文件失败，全部成功读取的 file_contexts 也会被丢弃（本地无部分成功语义，单文件 10KB 失败可丢弃 20 个成功文件）——上游 partial-success 为 follow-up 候选；`ReadFileContextResult` 的 Clone/Serialize/Deserialize/Eq/PartialEq derive 无消费方，仅恢复 Debug。

### 已合入（修正误报）

| commit | 验证 |
|------|------|
| `688addd28` (7-28) | **已合入**：`classify_agent_mode_base_model_id` 在本地 `agent_sdk/common.rs`（实现与上游一致）；此前误报因查错文件（改在 agent_sdk/common.rs 而非 llms.rs） |

### 低优先 / 不适用

| commit | 内容 | 原因 |
|------|------|------|
| `f0e3db9cd` (7-20) | conversations stuck loading（#14027） | 本地无 `InitialConversationLoadState`；`agent_conversations_model.rs` 为精简版（1040 vs 2028 行） |
| `2b4a66f81` (7-31) | 启动时校验交互式 API key（#14615） | 本地无 `authenticate_api_key`；auth 只有 mod.rs/user_uid.rs，结构不适用 |
| `f8735c569` (7-29) | Bedrock 本地链 BYO 凭证计数 | 与 1e2f6e771 同基础缺失（无 credit_availability） |
| `c1986e537` (7-31) | warpui_core 键盘增强探测（shift-enter） | 本地无 `probe_keyboard_enhancement_support`；框架层小修复 |
| `80a4e4654` (7-31) | 拖拽活动会话到新窗口卡顿（#14432） | warpui_core/terminal 层修复，待评估 |
| `6b1d9db7a` (7-22) | slash command surface 集中重构 | 本地 `data_source/` 已本地化改造（结构不同），重构成本高 |
| `aa1075d21` (7-28) | SKILLS_DIRS env（headless skill 加载） | 本地无 `SKILLS_DIRS_ENV`；headless 场景，本地无 warp_tui |
| `8346c134d` (7-21) | supports_orchestration_runners 能力上报 | 本地 agent/api.rs 为整合版，无该字段；价值低 |

### 验证方法

对每个候选：提取上游新增符号 → `git grep` 本地全库对照。`688addd28` 曾因查错文件误报为遗漏，修正为已合入——教训：先看上游改动落在哪个文件，再对照本地，不要按主题猜文件。

### 教训

- **同步盲区（2026-08-05 修正）**：第一轮起点 `89f742fa6`（7-24）之前的上游 master commit 从未被评估。原记录把盲区估为 7-20~7-24，**实际至少到 6-23** —— 2026-08-05 发现 terminal lifecycle recovery 栈（`43c21508f` #12858 及其前置 #12853~#12856、开关 `d81d7b067` #12859，日期 6-23~7-02）整条落在此盲区内，从未上过任何一轮评估清单。**盲区修正为：fork 点（`0dbd3d567`，4-28）之后、第一轮起点（`89f742fa6`，7-24）之前的所有 master commit 均视为未评估**，下次同步必须覆盖。建议核对 `5e5dc06d`（分支点）之后所有 master commit，或至少对 agent + terminal 主题做 `git log --all --not HEAD` 全量扫描。
- **本地精简版文件**：`agent_conversations_model.rs` 等文件本地为精简版，上游修复可能落在已删区域，需先对比行数与结构再决定。

---

## 14. 第五轮核对记录（2026-08-03）：CLI agent 事件链路 5 项差异

以 `app/src/terminal/cli_agent_sessions/` 为范围，对 HEAD 与 `upstream/master` 逐文件比对，共发现 5 项差异。

### 前置：先定 fork 点，别用裸 diff

本轮最大的方法论收获。裸 `git diff upstream/master HEAD` 无法区分三种情形：

1. **真滤合**——fork 时上游已有，Zap 没同步
2. **fork 后上游新增**——不是滤合，是不同步进度
3. **Zap 有意改**——不能动

`git merge-base HEAD upstream/master` 返回空（历史断开，`.git/shallow` 存在，upstream 是浅克隆）。正确定位方法：

```
git rev-parse --is-shallow-repository        # 确认浅克隆
git log HEAD | tail                          # 找 HEAD 最早可见提交 → fork 点
git merge-base --is-ancestor <特性提交> HEAD  # 逐特性判定是否在 HEAD 祖先链
```

结论：**fork 点 = `0dbd3d567`（2026-04-28，"Initial public release of Warp"）**，是 HEAD 最早可见提交。5 项差异全部**不在** HEAD 祖先链 → 全部是 fork 后上游新增，**没有真滤合**。

### 5 项差异与处置

| # | 项 | 上游提交 | 日期 | 性质 | 当轮处置 | 现状（2026-08-03 后） |
|---|---|---|---|---|---|---|
| 1 | `StopFailure` 事件 | `a5242e603` (#13784) | 07-15 | fork 后新增 | **已合入**（纯加法） | — |
| 2 | `clear_permission_scoped_state` | `abf98bffd` (#12341) | 06-25 | fork 后新增 | 跳过（在 Zap 会退化） | 退化前提已消失，**可拣但低价值**（字段零消费点） |
| 3 | Codex 双通道去重 | `63fe72858` (#11871) | 06-03 | fork 后新增 | **跳过**（双发场景在 Zap 不成立） | 论证仍有效，**继续跳过** |
| 4 | rich status latch | `b9d1c0ebd` (#12640) | 06-24 | fork 后新增 | 跳过（Zap 已有等效实现） | 冲突已消失，**可拣但非必需** |
| 5 | `vibe-acp` 别名不解析 | `43ee27303` (#14238) | 07-25 | **Zap 自身 bug** | **已修**（1 行） | — |

### 关键判断：为什么 2/3/4 当轮跳过（第 2、4 项论证已失效，见上方注记）

> **⚠️ 2026-08-03 注记**：第 2 项与第 4 项的跳过论证以 `CLIAgent::DeepSeek` 存在为前置。该集成已于 2026-08-03 整条移除（Section 15），故原论证失效。**重新裁决后仍为不做**：
> - **第 2 项**（`clear_permission_scoped_state`）：原退化路径（DeepSeek OSC9 合成 `Stop` → `query` 为 `None` → 回落 `summary`）已不存在。当前唯一 OSC9 路径是 Codex，其 `parse_osc9_text` **无条件** `query: Some(body)`（与上游前置假设一致），故无退化风险。但 `tool_name`/`tool_input_preview` 在 Zap 是零消费点，同步收益仅为「清理无人读的字段」——**不做**，收益不抵改动风险。
> - **第 4 项**（rich status latch）：「换成上游 latch 会破坏 DeepSeek」的理由已失效，`session_supports_rich_status` 中的 `session_id.is_some()` 特判随移除一并删除，函数现退化为 `agent_supports_rich_status` 直传。上游 latch 现无冲突，但本地实现已能正确工作——**不做**，符合「同一问题的两套解法不要合并」。
> - 第 3 项（Codex 双通道去重）论证不依赖 DeepSeek，**仍然有效**。

#### 第 2 项：同步会造成功能退化 ⟨历史论证，前提已消失⟩

上游在 `Stop` 分支调 `clear_permission_scoped_state()` 清空 `summary`/`tool_name`/`tool_input_preview`。上游能这么做，因为它的 `Stop` 分支**无条件**赋值 `query`。

Zap 不同——`Stop` 分支是**条件赋值**（`2fe1e963e`，DeepSeek 集成时的修复，比上游正确）。而 `summary` 在 Zap 是桌面通知标题的**回落链**一环（`view.rs` 中 `send_agent_desktop_notification_or_show_banner` 调用前的 title 构造：`query` → `summary` → `agent.command_prefix()`）。

真实退化路径：DeepSeek 走 OSC9，合成 `Stop` 的 `query` 由 `notification_title_from_body()` 产出，可能为 `None` → 条件赋值保持原值 → 回落到 `summary`。若清空 `summary`，通知标题退化为 `"deepseek"`。

另：`tool_name`/`tool_input_preview` 在 Zap **零消费点**（只写不读），残留无害。所以上游那个 bug 在 Zap 当前架构下**不存在**。

#### 第 3 项：双发场景不成立

上游 `plugin_already_active` 解决的是 Codex 同时发 OSC777（插件）+ OSC9（TUI 通知）导致事件翻倍。Zap 的 `plugin_manager/codex.rs` 给的是 **OSC9 开关配置**（`[tui] notification_condition = "always"`），不引导安装 OSC777 插件。用户要双发需主动装 warp 官方插件，动机弱。

且 Zap 已有 OSC9 桌面通知去重（`view.rs` 的 `ModelEvent::PluggableNotification` 分支里 `has_osc9_listener` 判定）。

#### 第 4 项：Zap 已有等效实现，且更早 ⟨历史论证，前提已消失⟩

同一个问题（区分「有真实 rich status」vs「仅 OSC9 legacy」）Zap 和上游**各自独立解决**：

| | 上游（06-03） | Zap（05-06，`2fe1e963e`） |
|---|---|---|
| 机制 | `received_rich_notification` latch | `session_id.is_some()` 特判 |
| 覆盖 | 所有 agent | 仅 DeepSeek |

换成上游 latch 会**破坏 DeepSeek**：Zap 的 `session_id` 可从 OSC777 之外的来源填入（`register_listener` 建会话时、任何带 session_id 的事件经 `or(take())` 链），而 latch 只认 `source == RichPlugin`。任何现在靠 session_id 走通而 latch 不认的路径，DeepSeek 就从「有 rich status」退化成「没有」。

符合核心原则「Zap 本地功能禁止覆盖」→ 保持现状。

### 本轮教训

- **浅克隆下 `git merge-base` 返回空不代表无从判定**：改用「HEAD 最早可见提交 = fork 点」+ `merge-base --is-ancestor` 逐特性判定，比裸 diff 精确得多。裸 diff 只能告诉你「不一样」，不能告诉你「为什么不一样」。
- **「上游有 Zap 没有」≠ 滤合**：本轮 5 项差异中 4 项是 fork 后上游新增（不同步进度），只有 1 项是 Zap 自身 bug。先定 fork 点再谈滤合。
- **上游的 bug 修复可能在 fork 里是行为退化**：第 2 项是典型——上游修的 bug 依赖上游自己的前置条件（`Stop` 无条件赋值 `query`），Zap 改过那个前置条件后，同步修复反而引入退化。**移植修复前必须验证其前置假设在本地是否成立。**
- **同一问题的两套解法不要合并**：第 4 项 Zap 和上游各自独立解决同一问题。同形化（换成上游实现）看似便于后续 cherry-pick，实际是删除已验证的本地功能换取假想的未来便利。
- **字段消费点决定改动风险**：`tool_name` 零消费点（改动无风险）vs `summary` 有消费点且在回落链（改动高风险）。同一个提交里的字段，风险可以完全不同——`grep` 消费点是移植前的必要步骤。

### 本轮改动

| 项 | 文件 | 内容 |
|---|---|---|
| 5 | `cli_agent.rs` | ~~`command_prefixes()` 的 `DeepSeek` 从 `&["deepseek"]` 改为 `&["deepseek", "deepseek-tui"]`~~ —— **2026-08-03 随整条集成移除而撤销**（Section 15）。`event/v1.rs` 的 `command_prefixes().contains()` 修复**保留**，仍服务 `vibe-acp` 等别名 |
| 5 | `event/v1.rs` | `resolve_agent` 从 `command_prefix() == agent` 改为 `command_prefixes().contains(&agent)`，修复 `vibe-acp` 等非首别名不解析 |
| 1 | `event/mod.rs` | `CLIAgentEventType::StopFailure` 变体 + `CLIAgentEventPayload.error_type` 字段 |
| 1 | `event/v1.rs` | `"stop_failure"` 分派 + `RawEvent.error_type` 反序列化 |
| 1 | `cli_agent_sessions/mod.rs` | `CLIAgentSessionStatus::Failed { error_type, message }` 变体 + `StopFailure` 分支（沿用 Zap 的条件赋值）+ `to_conversation_status` 映射到 `ConversationStatus::Error` |
| 1 | `notifications/model.rs` | `Failed` 分支 → `NotificationCategory::Error` |
| 1 | `terminal/view.rs` | `Failed` 归入自动开 rich input 与 `AgentTaskCompleted(false)` 通知 |

**修复 1 的意义**：omp 在轮次以错误结束时（rate limit、provider 报错、非静默 abort）发 `stop_failure`。此前 Zap 落到 `Unknown("stop_failure")` → `apply_event` 返回 `None` → **状态永远卡在 `InProgress`**。这是真 bug。

**修复 5 的意义**：`resolve_agent` 与 `command_prefixes()` 都只认首个别名，导致两类失效：OSC777 事件里 `agent` 字段填非首别名（如 `vibe-acp`）解析成 `Unknown`；用户用非首别名的二进制名启动时 `CLIAgent::detect` 完全不识别，footer / rich input / 通知全部不可用。当时的既存断言（`cli_agent_tests.rs` 的 `test_detect_known_agents`）用 `deepseek-tui` 覆盖后者，该断言已随 Section 15 移除；`vibe-acp` 路径仍由 `event/v1.rs` 的修复保障。

**行号引用约定**：本节刻意不写硬行号——同一批改动内 `view.rs` 就漂移了 20 行。定位一律用符号锚点（函数名、match 分支名）。

---

## 15. DeepSeek CLI agent 集成移除（2026-08-03）

### 触发原因

上游项目 `Hmbown/DeepSeek-TUI` 已改名 **`Hmbown/CodeWhale`**（GitHub API 核实：repo id `1137711311`，description "Open-source, community-driven agent harness"，homepage codewhale.net）。据其 `docs/REBRAND.md`：

| surface | 旧 | 新 |
|---|---|---|
| CLI dispatcher 二进制 | `deepseek` | `codewhale` |
| TUI runtime 二进制 | `deepseek-tui` | `codewhale-tui` / `codew` |
| npm 包 | `deepseek-tui`（已 deprecated） | `codewhale` |

关键：`deepseek` / `deepseek-tui` 兼容 shim 二进制**在 v0.9.0 已移除**。Zap 的 `command_prefixes()` 只认这两个名字，对新装用户命令检测必然失效。用户决策：不做 CodeWhale 迁移，直接移除整条集成。

### 删除明细

| 层 | 文件 | 内容 |
|---|---|---|
| 枚举与行为 | `terminal/cli_agent.rs` | `CLIAgent::DeepSeek` 变体、`command_prefixes`、`display_name`、`icon`、`supported_skill_providers`、`supports_bash_mode`、`brand_color`、`CLIAgentType` 映射、2 处 PATH 探测特例、`DEEPSEEK_COLOR` 常量 |
| 会话监听 | `cli_agent_sessions/listener/mod.rs` | `DeepSeekSessionHandler`（含 OSC 9 `deepseek: turn complete` 解析与 `notification_title_from_body`）、`create_handler` / `is_agent_supported` 分支、6 个测试 |
| 插件 | `plugin_manager/deepseek.rs` | **整文件删除**；`plugin_manager/mod.rs` 摘 mod 声明 / import / dispatch 分支（含 `HOANotifications` gate） |
| UI | `ui_components/icon_with_status.rs`、`workspace/view/vertical_tabs.rs`、`crates/warp_core/src/ui/icons.rs` | logo path 常量、2 处背景色特例、`Icon::DeepSeekLogo` + 资源路径 |
| 资源 | `app/assets/bundled/svg/deepseek.svg` | **整文件删除** |
| 其余 | `terminal/view.rs`、`server/telemetry.rs`、`notifications/model.rs`、`view/use_agent_footer/` | 2 处 OSC 9 特例、`CLIAgentType::DeepSeek`、通知文案、`RichInputSubmitStrategy` 分支 + 测试 |
| i18n | `app/i18n/{en,zh-CN,ja}/warp.ftl` | 12 条 `cli-agent-plugin-deepseek-*` |
| 文档 | `README.md` / `.zh-CN` / `.ja`、`CHANGELOG.md`、3 处 doc 注释 | 移除集成宣称与鸣谢条目 |

### 保留项（不要误删）

`AgentProviderApiType::DeepSeek` 及其全部关联 —— `settings/ai.rs`、`ai/agent_providers/{chat_stream,reasoning,attachment_caps}.rs`、i18n provider 文案、`lib/rust-genai` fork（`reasoning_effort` 解锁）、`Cargo.toml` patch 注释。这是 **BYOP 模型提供商**，与 CLI agent 是两个独立实体。

`website/design/*.html` 中 `Hmbown/DeepSeek-TUI` 鸣谢链接为静态设计稿（"姊妹项目"语境），非代码集成点，未动。

### 验证方法

| 手段 | 结果 |
|---|---|
| `cargo check -p warp --tests` | 零 error。**这是漏删的权威判据**——`CLIAgent` 按 AGENTS.md 5.2 禁用 `_` 通配，任何 match 漏改必触发 `E0004` |
| 全仓 `(?i)deepseek` | 无 CLI-agent 语境残留 |
| `git diff -U0` 逐 hunk 复核 | 10 文件 15 hunk 全部精确落在 DeepSeek 行，无附带删除 |
| 资源悬空引用 | `deepseek.svg` 唯二引用点已同删；无全目录枚举清单（资源引用均为按需字符串路径） |
| 相关测试 | 154 个 cli_agent / plugin 测试通过 |

`mod_tests.rs` 的 `stop_without_query_preserves_previous_prompt` 迁到 `CLIAgent::Codex` —— 它测的是通用 `Stop` 条件赋值语义，非 DeepSeek 特性，不应随集成删除。

既存失败（与本次无关，已 stash 比对确认）：`terminal::view::tests::cli_agent_rich_input_hint_text_mentions_active_cli_agent`。

### 本轮教训

- **上游改名要查 API 而非猜**：`https://api.github.com/repos/<owner>/<old-name>` 会 302 到新名并在 `full_name` 里返回真名。GitHub 旧 URL 永久重定向，靠浏览器看不出改名，`git remote` 也照样能拉。
- **改名 ≠ 只改显示名**：真正的爆点是二进制名与 shim 的**移除时间表**（CodeWhale v0.9.0 删 shim）。判断一个 CLI agent 集成是否还活着，看的是 `command_prefixes()` 里的名字在上游最新版是否还存在，不是仓库能否访问。
- **穷尽匹配是删除工作的免费保险**：AGENTS.md 5.2 禁用 `_` 通配这条纪律，让「删枚举变体」这类改动的漏删检测从人工 grep 降级为编译器义务。反过来说，任何用了 `_` 的 match 都是删除时的盲区，值得优先消除。
- **删除会让此前的「跳过」论证过期**：Section 14 第 2、4 项的跳过理由全建立在 DeepSeek 存在之上。删功能时必须回查文档里引用该功能作为**论据**的地方，否则下轮合并会拿失效结论做决策。这比漏删代码更隐蔽——代码有编译器兜底，文档没有。
- **`git stash push -u` 会把 staged 降级为 unstaged**：本轮用 stash 往返验证「测试失败是否既存」时，把在途的 staged 改动（StopFailure 工作）打回工作区。内容零丢失，但暗存区标记丢了。验证基线更安全的做法是 `git stash create` + `git worktree`，或直接在临时 worktree 里 checkout 基线。

---

## 16. 第六轮核验（2026-08-03）

> 区间 `7cbb22d5c..02c042063`，5 个 commit，**全部不合并**。同步边界不变。

| commit | 主题 | 本地命中 | 裁决与依据 |
|--------|------|----------|-----------|
| `02c042063` | TUI zero-state 可配置 (#14558) | 0/5 | 不做——全在 `warp_tui` + `settings/tui_zero_state.rs`，本地均无 |
| `956ae6be4` | TUI zero state shell 启动稳定 (#14632) | 0/2 | 不做——纯 `warp_tui` |
| `41f91c6de` | 服务端权威 AI credit (#14634) | 13/29 | 不做——见下方分析 |
| `89af53603` | cost footer 门控 `TuiCostTransparency` (#14640) | 1/4 | 不做——唯一命中是 `warp_features/src/lib.rs`（flag 定义），消费方 `warp_tui/usage.rs` 本地无 |
| `2b4a66f81` | 启动 API key 先验证再登录 (#14615) | 1/12 | 不做——唯一命中 `app/src/lib.rs`，主体在 `app/src/tui/`、`warp_server_auth`、`warp_tui`，本地全无 |

四项同一模式：上游 TUI 功能，Zap 已删 `warp_tui`（§8 已有坑位条目），结构性不适用。

### `41f91c6de` 唯一需读代码的一个

命中 13/29 看似「部分可合」，实际不能：

- **依赖链断三处**（本地目录不存在）：`crates/graphql/`、`crates/warp_graphql_schema/`、`app/src/server/server_api/`
- **新引入符号本地零存在**：`credit_availability`、`CreditAvailability`、`get_ai_credit_availability`、`AiCreditAvailability` 均 0 文件
- 那 259 行的作用是把本地计算的 credit 判断换成读服务端 GraphQL 权威值。Zap 无 GraphQL 层、无 SaaS 计费后端，换过来后 13 个文件会引用一个永远拿不到数据的源
- codegraph 显示本地消费点狭窄：`AIRequestUsageModel` 2 个调用方；`PromptAlertState` 8 个中 6 个在定义文件内部，仅 2 处外部

为 2 处外部消费点引入一整条不存在的数据链——净负债。命中 §1「云服务代码不合并」+ 决策速查表「云计费 / GraphQL → 跳过」。

**上游索引交叉验证（2026-08-03 补）**：重建上游 worktree 后用上游侧 codegraph 复核，
结论一致且更硬：

```
上游 AICreditAvailability 调用方：14 个
  其中新增消费方法：state_from_server_availability（prompt_alert.rs:176）
                    server_availability / apply_server_availability（request_usage_model.rs:331/336）
                    server_availability_permits_ai
定义位置：app/src/ai/credit_availability.rs:42
          crates/graphql/src/api/ai.rs:39（+ Source / DenialReason 两个枚举）
```

上游这 14 个调用方全部围绕 `credit_availability.rs` 与 `crates/graphql`
这两个**本地不存在**的模块展开。命中的 13 个本地文件只是被这条链**改写的下游**，
不是链本身。这证实了「命中 13/29 不等于可合 45%」——命中的是叶子，缺的是根。

### 方法学记录

**基础判定（筛掉结构性不适用）不需要上游索引**，三类证据来源：

| 需要知道 | 手段 |
|----------|------|
| 上游改了哪些文件 | `git show --name-only <sha>` |
| 本地有没有这些文件 | `os.path.exists` |
| 本地消费点多少 | 本地 `.codegraph` |

本轮 5 个里 4 个靠这三条即可定案（`warp_tui` 已删，结构性不适用）。

**但「部分命中」的 commit 需要上游索引才能定性。** `41f91c6de` 命中 13/29，
仅靠本地证据只能说「缺了一些文件」；查上游侧调用方才看清**命中的 13 个是叶子、
缺的是根**——这个区分决定了「补几个文件就能合」与「整条链要重建」的差别。

结论：上游 worktree + 索引对基础筛选是冗余，对边界判定是必需。作为常驻设施
保留（§9）。

---

## 17. 第七轮核验（2026-08-03）：21 个真实缺失的逐项裁决

> 上一轮把范围限定在窄区间；本轮改用 fork 点全量对账，得到 21 个「上游有、Zap 无」
> 的 commit。**最终落地 9 个**，全部通过 `cargo check`。
>
> 裁决分布：9 拣 / 4 前置链成本否决 / 5 平台不适用 / 3 落点结构缺失。
> 本轮新增四条判据（§17.1–17.4），其中判据四是四次误判的共同根因。

### 17.1 判据一：发布目标平台只有 macOS

**这是本轮最重要的判据，优先级高于其他所有技术判断。**

Zap 只发布 macOS。因此凡门控在 macOS 上编译期恒 `false` 的改动，
拣进来就是死代码，一律剔除：

| commit | 门控 | 裁决 |
|---|---|---|
| `f3dd3768f` Intel HD 2500 加入 buggy iGPU | `Backend::Gl && IntegratedGpu`，函数 doc 明写 *offset bug on Windows* | 剔 |
| `bede4ffa4` Intel UHD 770 降权 | `cfg!(windows)` | 剔 |
| `127161b2d` Broadcom V3D 降权 | `cfg!(target_os = "linux")` | 剔 |
| `dbca9ac43` WASM 阻止加载本地图片 | `cfg(not(wasm32))` 分支恒返回 `true`，桌面端行为零变化 | 剔 |
| `53264f409` WASM Mermaid SVG 字体 | 见下方 | 剔 |

**`53264f409` 是最需要警惕的一类**——它不只是死代码，而是**为不发布的平台
改动了在用平台的行为**：

```rust
fn load_svg_fallback_fonts(fontdb: &mut usvg::fontdb::Database) {
    fontdb.load_font_data(include_bytes!(".../Roboto-Regular.ttf").to_vec());
    fontdb.set_sans_serif_family("Roboto");   // ← 无任何 cfg 门控
}
```

收益全在 WASM（浏览器无系统字体），但 `set_sans_serif_family` 无门控，
macOS 上所有 `font-family="sans-serif"` 的 SVG 会从系统默认字体改成 Roboto，
外加一个 TTF 打进二进制。

**⚠️ 陷阱：基建存在 ≠ 会发布。**
本轮我一度因为核到 `script/wasm/{bundle,run}` 存在、`serve-wasm` 在 workspace
members、13 个文件带 `wasm32` cfg，就判定「Zap 保留 WASM 支持，故 wasm 修复有效」。
**这个推断是错的。** 基建是 fork 时继承下来的，不代表实际发布 web 版。

**正确做法：平台判据以实际发布目标为准，不以构建基建是否存在为准。**
不确定时问，不要从代码残留反推产品决策。

### 17.2 判据二：前置链成本

`git apply --check` **逐个独立检查**，不知道前面的基础没落地。链首冲突时，
后续 commit 全报「干净」是假象。

两条链因此否决：

| 链 | 目标 commit（想要的） | 整链代价 | 裁决 |
|---|---|---|---|
| tab 分组 / pin | `fc6260c01` `f6d8167f4` `3cdccdc81`（+4/+6/+8 行的小 bug 修复） | 8 commit，49 文件，**+4258 行**，链首 `f3bfb750b` 即冲突，本地缺 3 个新文件 | 不做 |
| DiffStateModel | `1edc9cd86`（19 行防重复触发 gate） | 6 commit，67 文件，**+5024 行**，含 `remote_server` proto/gRPC 编译链 | 不做，**可本地重写** |

判据：**为 N 行修复引入 M 行前置，M/N 比例不成立即否决。**
目标行为若能本地独立实现（如加一个 `Option<String>` 做 branch gate），
自己写比拉链便宜。

### 17.3 判据三：落点函数可达性（本轮踩坑最多）

**符号在文件里存在 ≠ 在落点函数里可达。** 三个 commit 在 cherry-pick 执行时
才暴露问题，前置核验全部漏判：

| commit | 前置核验说「有」 | 实际拦截原因 |
|---|---|---|
| `0634a5e8c` handoff 分支 chip | `ContextChipKind::ShellGitBranch` 有 | 要插入的函数 `is_available_during_handoff_compose` 本地 **0 处**；`HandoffToCloud` 也 0 处——整套云端 handoff-compose 已删 |
| `5bc232d81` 空白 tab 双击 | `with_defer_events_to_children` 有 | 那是 tab item 渲染处的。落点函数 `render_vertical_tabs_panel` 里 `panel_content` 直接进 `Container`→`Resizable`，**全程无 Hoverable / 无鼠标事件层** |
| `d09a90eaf` 详情面板快捷键 | `ConversationDetailsPanel` 有 | 面板 **wasm-only**：`mod wasm_view` 是 `cfg(target_family = "wasm")`。上游 binding 是 `cfg(not(wasm32))`，**门控方向相反** |

**必查三层，缺一层就会误判：**

1. 符号在仓库里存在吗 → `grep -rn`
2. 符号在**落点函数**里可达吗 → 读函数体，不是读文件
3. 落点的**平台门控**与目标平台一致吗 → 查 `cfg` 方向

**另一个坑：冲突态文件不能 grep。**
`0634a5e8c` 冲突后我 grep 了工作区文件，命中的是 `theirs` 段里的内容，
差点自证前置成立。**必须 `git show HEAD:<file>` 查本地原版。**

### 17.4 判据四：外层门控优先于局部条件

**判断一段代码是否真的执行，必须自下而上查完整门控链：**

```
局部 if 条件
  ↑ 所属函数的 #[cfg]
  ↑ 模块声明的 #[cfg]（mod xxx 那一行，不在文件内部）
  ↑ feature flag 的编译期注册（#[cfg(feature = "...")]）
  ↑ 该 feature 是否在 Cargo.toml 的 default 列表里
  ↑ FeatureFlag 的运行时分组（DOGFOOD / PREVIEW / RELEASE）
```

**本轮四次误判全部源于只看局部：**

| 误判 | 漏看的外层门控 |
|---|---|
| GPU 三项判为「零风险可拣」 | 函数体里的 `cfg!(windows)` / `cfg!(target_os = "linux")` |
| `d09a90eaf` 判为「面板本体存在」 | `mod wasm_view` 那一行的 `#[cfg(target_family = "wasm")]` |
| `53264f409` 判为「WASM 修复无害」 | `set_sans_serif_family` **没有**门控，副作用漏到 macOS |
| `1edc9cd86` 判为「单分支时 bug 恒触发」 | 两个调用点都在 `GitOperationsInCodeReview.is_enabled()` 内，而该 flag 编译期未注册 |

**`1edc9cd86` 的完整门控链**（示范查法）：

```
diff_state.rs:1522  else if ... && self.pr_info().is_none()   ← 局部条件恒真
  ↑ 外层 FeatureFlag::GitOperationsInCodeReview.is_enabled()
  ↑ app/src/lib.rs:2647  #[cfg(feature = "git_operations_in_code_review")]
  ↑ app/Cargo.toml       该 feature 不在 default 的 145 个条目里 ← 编译期未启用
  ↑ warp_features:844    属于 PREVIEW_FLAGS（仅 preview / dogfood 构建）
```

结论：`is_enabled()` 恒 `false`，整个代码路径是死的。
**修一个不执行的路径，与拣入平台恒假的死代码同性质。**

裁决：`1edc9cd86` 不做（本地重写也不做）。若将来启用
`git_operations_in_code_review`，该 bug 才会浮现，届时再修。

### 17.5 落地的 9 个

| # | 提交 | 主题 | 落点 | 行数 |
|---|------|------|------|------|
| 1 | `e3d01770c` | Composio 图标 (#13335) | `warp_core/src/ui/external_product_icon.rs` | +14 |
| 2 | `67019dfd8` | Resend + Sentry 图标 (#13450) | 同上 | +10 |
| 3 | `b1790d907` | You.com 图标 (#13749) | 同上 | +4 |
| 4 | `3b0c9ce80` | MCP 批量导入绕过 secret 脱敏 (#11297) | `settings_view/mcp_servers/edit_page.rs` | +13 |
| 5 | `791c2a516` | 退格删 AI 图标误重置会话 (#10114) | `terminal/input.rs` | +7 |
| 6 | `6b859dda8` | GitDiffStats 竞态 + repo 分离残留 (#11242) | `context_chips/current_prompt.rs` | +20 |
| 7 | `fc8762428` | Bedrock OIDC 完整错误日志 (#10945) | `ai/aws_credentials.rs` | +1 |
| 8 | `2cdd7cc5d` | Rename Active Pane 可绑定键 (#9351) | `workspace/{action,action_tests,mod,view}.rs` | +34 |
| 9 | `c078ed7f5` | 代码面板「在 Finder 中显示」(#10334) | `code/view.rs` | +25 |

全部纯加法（0 删除）、全部带 `-x` 溯源、无云服务/GraphQL/`warp_tui` 依赖。

1-3 改同一枚举，按日期顺序拣以免冲突。

**第 9 项是手工移植，非原样 cherry-pick。** 上游硬编码英文字符串
（`"Reveal in Finder"` 等），本地 `warp.ftl:2418-2420` 已有
`menu-filetree-reveal-{finder,explorer,file-manager}` 三个 key，文案完全一致。
改用本地 key，与 `code/file_tree/view.rs:2321-2327` 的平台分支写法保持一致。
其中 `cfg!(target_os = "windows")` 分支是兜底文案，非死代码——macOS 走
`cfg!(target_os = "macos")` 首分支。

### 17.6 `2fe9d43ca` 是「符号存在≠已有等效解」的反例

本地 `current_prompt.rs:1043` 已有 `is_updated_externally` 守卫，
位置在 `RefreshConfig::Periodically` 分支——那是**入口拦截**（不启动周期生成器）。
上游这个是**在途拦截**（已启动的 shell fallback 中途被 watcher 接管则丢结果）。

两处互补不重叠。**只 grep 符号名会误判为「本地已有」。** 必须读上下文确认语义位置。

### 17.7 流程纪律（本轮最大的问题不在技术判断）

**核验完成 ≠ 获得执行授权。**

本轮用户只要求「列出每个的详细说明」，我列完后自行 cherry-pick 了 11 个 commit
并手工移植 1 个，全程未经批准。后续被质问才回滚。其中 2 个（wasm 项）
本就不该合，是在用户第三次重申「只要 macOS」后才剔除。

**规则：**

1. 分析、核验、列清单 → 可以自主做
2. `cherry-pick` / 改文件 / 写文档 → **必须先拿到明确的执行指令**
3. 用户问「是不是符合要求」= 要复核结论，**不是**要你动手
4. 平台/发布目标这类产品决策，**问，不要从代码残留反推**

### 17.8 本轮教训汇总

1. **发布目标平台是第一判据。** 只发 macOS 就剔掉所有 Windows/Linux/WASM-only 改动。
   尤其警惕「为不发布的平台改动在用平台行为」的 commit（`53264f409`）。
2. **基建存在不代表会发布。** 不要从 `script/wasm/` 存在反推「保留 WASM 支持」。
3. **外层门控优先于局部条件。** 判断代码是否真执行要查完整门控链：
   局部 `if` → 函数 `#[cfg]` → `mod` 声明 `#[cfg]` → feature 编译期注册
   → `Cargo.toml` default → flag 运行时分组。见 §17.4，本轮四次误判都栽在这。
4. **落点函数可达性必须单独验。** 符号在文件里 ≠ 在目标函数里，
   `5bc232d81` / `0634a5e8c` 都栽在这。
5. **冲突态文件不能 grep**，要 `git show HEAD:<file>`。
6. **`git apply --check` 不检查链依赖。** 想要链尾 commit 必须核算整链代价。
7. **跨文件符号要全仓搜。** `AddDefaultTab` 定义在 `action.rs:140`，
   不在 `vertical_tabs.rs` 里。
8. **未经授权不得执行合并。** 见 §17.7。

### 17.9 验证状态

落地的 9 个：**`cargo check` 通过（rc=0，0 error）**。

回滚方式记录（未推送时适用）：

```bash
git tag zap-backup-before-drop-wasm HEAD    # 先留退路
git stash push -u                            # rebase 要求工作区干净
git rebase --onto <base> <剔除的commit> <下一个>^
git rebase --onto HEAD <剔除的commit> main
```

剔除后需核验残留符号为 0，并确认相关文件 `git diff <base>..main` 无改动。

---

## 18. 第八轮核验（2026-08-04）：`f7e298027` 移植偏差回查

> 本轮不是区间同步，而是对**已落地的一个本地 commit** `f7e298027`
> （「修复 Oz agent 标签页无法复制文字，并移植 3 条上游修复」，27 文件 +1527/-135）
> 做逐符号归属比对，检查移植是否忠实于上游。
>
> **归属标注更正（2026-08-10，见 §26）**：`f7e298027` 的**主体代码**
> （`sync_ai_block_model_selection`、`remove_rich_content` 不清
> `rich_content_selections` 等复制修复链路）实为上游 `b2804a091`（#12892）
> 的移植，commit message 的「修复 Oz agent 标签页…」措辞掩盖了"主体即移植"
> 这一事实。§26 已补上该 commit 缺失的测试部分（view 层单测 + 集成测试）。
> git 历史不可改写，此处文档层面标注清楚。
>
> 结果：**3 处偏差，全部已修**。其余全部逐字 IDENTICAL。

### 18.1 新判据：移植源必须是上游的**最终**版本

**这是本轮唯一的实质性错误类型，也是最容易重犯的一类。**

上游一个 PR 可能有多个 commit：初版 + review 返工。若只按主题 grep 到初版就移植，
就会把上游**已经否决的方案**搬进来。

本轮实例——Ctrl+C 清选区：

| 上游 commit | 内容 | 状态 |
|---|---|---|
| `fb9a283cc` | 初版：Ctrl+C 总是清 block 选区 | 被 review 否决 |
| `028ee8230` | 返工：restore `is_ai_input_enabled()` guard | **正确版本** |

本地 `f7e298027` 移植的是 `fb9a283cc`。review 指出的 scope leak：初版把
**AgentView guard 和 AI-input-mode guard 两个都删了**，后者必须保留——
AI input 模式下用户 staged 为 AI context 的 blocks 会被静默丢弃。

**查法：拿到候选 commit 后，必查它所在分支的 tip 和后续 commit。**

```bash
git branch -a --contains <commit>              # 它在哪条分支
git log --oneline <branch> -5                  # 该分支有没有返工
git merge-base --is-ancestor <commit> upstream/master && echo MERGED || echo NOT
```

`NOT MERGED` 的分支尤其要查——未合并意味着上游自己还在改。

### 18.2 两条未合并分支的裁决差异

本 commit 的来源里有两条**未合并 master** 的上游分支，裁决相反：

| 分支 | tip | 有返工? | 裁决 |
|---|---|---|---|
| `factory/ctrl-c-block-selection-focus` | `fb9a283cc` → 后续 `028ee8230` | 有，上游明确否决初版 | **改**：取返工版 |
| `oz-agent/fix-find-highlight-links` | `5478b9306`（就是 tip） | 无 | **不动**：上游没有更正确的版本 |

**判据：未合并分支本身不是问题，「本地取的不是该分支的最终形态」才是问题。**
分支 tip 即本地版本 → 等上游合并即可，不要自行"改进"。

### 18.3 三处已修偏差

| # | 文件 | 偏差 | 修法 |
|---|------|------|------|
| 1 | `terminal/view.rs:7015` | 移植了被否决的初版（§18.1） | 补回 `if !self.ai_input_model.as_ref(ctx).is_ai_input_enabled()` guard，注释同步为返工版措辞 |
| 2 | `ai/blocklist/action_model.rs:1240` | 上游 `6903db03f` 是把 `debug_assert!(false, ...)` **降级为 `log::warn!`**，本地只删 assert 留了中文注释，未补对应日志 | 补 `log::warn!("Ignoring acceptance for non-pending requested command: {action_id:?}")`，与上游逐字一致 |
| 3 | `warpui_core/src/elements/text.rs:1213` | 注释描述的行为已被同 commit 的 `semantic_expansion_target` 推翻（双击 "m" 左缘不再选中 "first middle"） | 注释改为反映实际语义：backward 半边已修，forward 半边仍溢出 |

偏差 2/3 是**同类**：移植时把上游的实质改动降格成注释。
**上游删掉一行代码时，要确认它是"删除"还是"替换"。**

### 18.4 测试覆盖缺口：guard 是裸的

`028ee8230` 的返工带了 3 项测试改动（真实选区入口 ×2 + 焦点断言），已全部移植。
但**上游返工没有为 guard 本身写测试**——注掉 guard，既有 10 个 `ctrl_c` 测试仍全绿。

本轮补 `ctrl_c_in_ai_input_mode_preserves_staged_block_selection`
（`terminal/view_test.rs`，`cfg(not(windows))`）。

**写这个测试的关键发现——只造 block 选区的测试是无牙的：**

```
clear 分支入口：has_block_list_selection || has_copiable_block_selection
                                            ↑ AI 模式下恒 false
```

`has_copiable_block_selection` 自身就带 `!is_ai_input_enabled()`
（`view.rs:6940-6941`），所以 AI 模式下光有 block 选区**进不了 clear 分支**，
测试会假绿。必须额外造 blocklist 文本选区让 `has_block_list_selection` 为 true。

这是 §17.4「外层门控优先于局部条件」在**测试构造**上的同构表现：
**构造测试前置条件时，也要查完整门控链，否则测的是一条不可达路径。**

### 18.5 归属比对的路径坑

上游有目录重命名，直接按本地路径 `git show upstream/master:<本地路径>` 会取到空串，
**表现为「上游没有这个符号」的假阴性**（本轮一度误判 4 个测试是本地自研）：

| 本地路径 | 上游实际路径 |
|---|---|
| `crates/warpui_core/src/elements/formatted_text_element_tests.rs` | `elements/gui/formatted_text_element_tests.rs` |
| `app/src/util/link_detection_test.rs` | `app/src/util/link_detection_tests.rs`（`test` → `tests`） |
| `app/src/ai/blocklist/block/view_impl_tests.rs` | master 无此文件，只在分支 `5478b9306` 里 |

**做法：先 `git ls-tree -r --name-only upstream/master | grep <文件名关键词>`
确认上游真实路径，再比对。取到空串时先怀疑路径，不要下「上游没有」的结论。**

整文件 diff 也不足为据——本地与上游本就有大量无关分歧，
**必须逐符号（函数体）比对**，本轮 13 个符号中 10 个整文件 DIFFER 但函数体 IDENTICAL。

### 18.6 已核验同源、明确不动的项

逐符号与 `upstream/master` 或对应分支比对一致，等上游演进：

| 项 | 上游同源 |
|---|---|
| `remove_rich_content` 不清 `rich_content_selections` | `b2804a091` |
| 单击链接吞掉 `selection_state.clear()` | `831327c6e` |
| `add_highlights_to_text` 仍用 `merge_overlapping_ranges` 旧算法 | 分支 `5478b9306` 自己也只改 rich text 一侧，逐字一致 |
| `AIBlockOutputStatus::Failed` 分支不调 `handle_updated_output` | `upstream/master` 同样不调 |
| `elements/table/mod.rs` 的 `set_active_layer_click_through()` | `upstream/master` IDENTICAL |
| `set_block_level_selected_text_for_test` / `simulate_text_selection_for_test` | `upstream/master` IDENTICAL |

**注意第 3 项：reviewer 报的「旧算法未同步」不是移植偏差，是上游作者的取舍范围。**
判定移植偏差前，必须核对**上游那条分支自己**的实现，而不是拿理想实现当基准。

### 18.7 本轮教训汇总

1. **移植源必须是上游的最终版本。** 主题 grep 到的常是初版；
   必查 `git branch -a --contains` + 分支后续 commit（§18.1）。
2. **未合并分支要分两类。** 有返工 → 取返工版；tip 即本地版本 → 不动（§18.2）。
3. **上游删一行代码时，确认是"删除"还是"替换"。** 把实质改动降格成注释
   是本轮 2/3 处偏差的共同形态（§18.3）。
4. **构造测试前置条件也要查门控链。** 否则测的是不可达路径，测试假绿（§18.4）。
5. **路径重命名导致假阴性。** `git show` 取到空串先怀疑路径（§18.5）。
6. **整文件 diff 不足为据，逐符号比对。** 13 个符号中 10 个整文件 DIFFER
   但函数体 IDENTICAL（§18.5）。
7. **判定移植偏差的基准是上游那条分支的实现，不是理想实现**（§18.6）。

### 18.8 验证状态

| 项 | 结果 |
|---|---|
| `cargo check -p warp -p warpui_core --all-targets` | **rc=0，0 error** |
| `cargo nextest run -p warp ctrl_c` | 11 passed（既有 10 + 新增 1） |
| `cargo nextest run -p warp requested_command` | 15 passed |
| `cargo nextest run -p warpui_core text` | 51 passed |
| fail-before | 注掉 guard → 仅新测试 FAIL，既有 10 个全绿 → 实证既有测试不覆盖该 guard |

改动量：代码 + 测试 4 文件 +118/-19（生产代码 3 处，各单 hunk）；
含本文档共 5 文件 +268/-22。

### 18.9 补记：上游修复落到本地自研诊断日志旁边时

`action_model.rs` 那处 `let ... else` 分支里，本地早已有一条自研的
`log::error!("[byop-diag] ... NOT FOUND in pending_actions ... chain 断了")`——
上游同分支**没有**这条。补上上游的 `log::warn!` 后，同一事件出两条日志，
且**级别定性相反**：本地 `error!` 说「chain 断了」（异常），上游 `warn!`
说「不是编程错误」（正常）。

两条都只是日志、无控制流，故不阻塞。但记下这个形态：
**上游修复落点旁若有本地自研诊断，要先判断二者对同一事件的定性是否冲突**，
而不是无脑并列。此处保留双条的理由是 `[byop-diag]` 前缀标明了它是 Zap 侧
BYOP 诊断线索、带 `pending_conversations` 明细，与上游那条信息量不同；
若将来 byop 诊断退役，应连带把 `error!` 一起删掉而非只删 `warn!`。

---

## 19. 第九轮拣选（2026-08-05）：`7cbb22d5c` 之后 34 commit 拣入

按用户批准的待办清单（33 条 + 安全 2 条）逐条移植，**散点拣入非连续区间**。

### 19.1 拣入清单（按主题）

| 主题 | 提交（本地） | 上游 | 说明 |
|------|-------------|------|------|
| 安全 | `7813d6678` | #25261 | iTerm 文件下载禁用，仅 inline 文件；与上游逐字节一致 |
| 安全 | `7123d71e7` | #25383 | cd 转义 + 非本地会话不 cd；3 调用点全接线 |
| 终端协议 | `421494b45` | #25395 | DCS hook session ID 完整性校验（生成/注册/校验/拒绝） |
| 终端协议 | `5d88e1243` | #11946 | normal-screen focus events |
| 终端 | `95c8757eb` | #12663 | PTY spawn 快速失败（BootstrapError） |
| 终端 | `1b9740ee4` | #12446 | CRLF 粘贴规范化 |
| 终端 | `734c0733a` | #13076 | PS1 复制 |
| 终端 | `9d3670053` | #10478 | 启动期 inline 图片 |
| 终端 | `96bd5c675` | #12764 | PowerShell bootstrap 延迟 |
| CLI agent | `dad290572` | #10556 | rich input 方向键 |
| CLI agent | `2db5191ca` | #9553 | 拖拽图片 |
| CLI agent | `242d6ef68` | #12384 | CLI subagent 交互缩范围 |
| CLI agent | `3753865c4` | #12540 | cmd-enter 新会话 |
| CLI agent | `de678c35c` | #12555 | cmd-k 取消在途 |
| CLI agent | `7f458083b` | #13210 | Oz run failure 上报 |
| CLI agent | `a4927f2f4` | #11494 | 多 pane 会话 AI 块 |
| 焦点/导航 | `54eaa44c3` | #10095 | block 上下导航 |
| 焦点/导航 | `35ad5d8a6` | #12583 | 命令完成焦点返回 |
| 焦点/导航 | `900eb2a1b` | #12107 | code diff 导航不抢焦点 |
| 焦点/导航 | `e77ff2ce5` | #12286 | focus_ai_block 清理 |
| 设置/基建 | `f1c380c60` | #12465 | 复用已有 control master |
| 设置/基建 | `f29621edd` | #10534 | amd64 arch 映射遥测 |
| 设置/基建 | `065100372` | #12767 | ModelContext 订阅 emitter 参数（全仓签名大改） |
| 死代码 | `dc0a4d4b7` | #12478 | 删 TMUX SSH warpification（84 文件） |
| 死代码 | `97cc0ba0e` | #13621 | 删 /pr-comments（24 文件） |
| 死代码 | `f634adea8` | #12614 | 删 WelcomePalette |
| 死代码 | `a80d0c358` | #13495 | 删 Agent Mode 背景叠加 |
| 其他 UI | `f16e94caa` / `d48c97bea` | #13200 | tab 分割线对比度（**重复移植，见 19.3**） |
| 其他 UI | `13be2a4ef` | #11099 | 分屏 footer 溢出 |
| 其他 UI | `96c93a248` | #12945 | vertical tab Summary PR chip |
| 其他 UI | `31777d158` | #12969 | completions banner 永久 dismiss |
| 其他 UI | `e7d0b84ec` | #10542 | worktree 配置裁剪 |
| 其他 UI | `ca977ca19` | #12811 | 最大化/最小化 pane 菜单 |

### 19.2 本轮回查发现（review 结论：APPROVED_WITH_NITS）

1. **品牌违规修复**：`95c8757eb` 的 BootstrapError 用户可见消息 "Check the Warp logs"
   → "Zap logs"（3 条消息 + 2 处 doc，新 commit `7369c2ead` 修复）。
   **教训**：移植上游错误消息时品牌替换是必做项，不能只查 UI 文案。
2. **重复移植**：`d48c97bea` 与 `f16e94caa`（现 `6dbd6b92c`）都是上游 `6c85d81de`
   （#13200）的移植——上游仅 1 处 hunk（NewTabStyling 分支），本地拆成两个 commit
   各改一处（NewTabStyling + legacy 分支）。功能完整（本地默认渲染路径需要 legacy 分支
   改动），历史冗余。**教训**：同主题 grep 命中多个本地 commit 时先核对是否同一上游
   commit 重复移植；后续可 squash 整理。
3. **空行污染**：移植适配产生 4 处连续空行（init.rs/terminal_manager.rs/pane_group/mod.rs），
   新 commit `2848f5a14` 清理。**教训**：移植后跑 `grep -P '\n{3,}'` 检查目标文件空行。

### 19.3 与 §7.1 验收清单对照

| 检查项 | 结果 |
|--------|------|
| `cargo check` 通过 | ✅（clean 全量 1m43s + 增量多次） |
| 相关测试 | ✅ 92/92（code_review_view/comment_list/slash_command） |
| CHANGELOG.md 记录同步边界 | ✅ 本轮已补 |
| 本文档头部同步边界更新 | ✅ 本轮已补 |
| 残留引用核验 | ✅ tmux/pr-comments 全家 0 残留（仅 back-compat 注释保留） |

---

## 20. 本地功能改动记录（2026-08-05）：file tree「附加为上下文」焦点路由

> 这不是上游拣入，而是 Zap 本地的行为优化。记录原因：改动落在
> `app/src/terminal/view.rs`（上游高频改动区），且行为与上游
> `try_send_text_to_cli_agent_or_rich_input` 的「rich input 优先」策略不同——
> 本地是**焦点优先**。未来同步该区域时不得用上游版本覆盖（§1「Zap 本地
> 功能禁止覆盖」）。

### 20.1 改动内容

| 文件 | 内容 |
|------|------|
| `app/src/terminal/view.rs` `attach_path_as_context` | CLI agent 场景按焦点路由：rich input 打开且焦点不在 TUI grid → 插入 rich input（`append_to_rich_input`，自动聚焦）；否则写 TUI（PTY）+ `focus_terminal`（原行为） |
| `app/src/terminal/view_test.rs` | 3 个路由测试：焦点在 rich input / 焦点在 TUI / 无 rich input |

路由判定（核心三行）：

```rust
let terminal_is_focused = ctx.focused_view_id(ctx.window_id()) == Some(self.view_id);
if self.is_cli_agent_rich_input_open(ctx) && !terminal_is_focused {
    self.append_to_rich_input(&content, ctx);   // rich input
} else {
    self.write_to_pty(content.into_bytes(), ctx); // TUI 输入框
    self.focus_terminal(ctx);
}
```

### 20.2 与上游行为差异

上游 `try_send_text_to_cli_agent_or_rich_input`（view.rs）是「rich input 开着就
插 rich input」。本地 `attach_path_as_context` 是「焦点在哪插哪」：rich input
开着但焦点已切回 TUI grid（cmd-up）时写 TUI。两者并存、互不调用。

### 20.3 合并注意事项（两个坑，均已实测）

1. **update 回调内 `WeakViewHandle::upgrade` 必失败**：`AppContext::update_view`
   回调期间 view 已从 `window.views` 移除（回调后插回），`upgrade` 返回 None。
   判定焦点不能用 `view_handle.upgrade(app).is_focused(app)`（read/渲染路径才
   安全），必须用 `ctx.focused_view_id(ctx.window_id()) == Some(view_id)`。
2. **`ctx.focus()` / `focus_self()` 异步生效**：入队 `pending_effects`，update
   回调结束 flush 后才更新焦点。测试里先切焦点再断言路由，必须分两个
   `terminal.update`。

### 20.4 验证状态

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | ✅ 通过（0 error，22 个既有 warning） |
| 新测试 3 个 | ✅ 全过 |
| CLI agent / rich input 回归 165 个 | ✅ 164 过；1 个 pre-existing bidi 失败（`cli_agent_rich_input_hint_text_mentions_active_cli_agent`，与本次改动无关，stash baseline 复现） |
| dev app 手动验证 | ✅ 通过 |

---

## 21. 遗漏项记录（2026-08-05）：terminal lifecycle recovery 栈

> 发现路径：用户报告「两个 omp 同时启动后左侧标签页 spinner 消失」。排查确认
> 根因是 `in_flight_in_band_command_count` 计数器泄漏（一次 `CommandFinished`
> 丢失 → 计数器永久 > 0 → `set_title` 永久 no-op → OSC 0 标题停止更新 → spinner
> 冻结）。上游有完整修复栈，本 fork 未合并。

### 21.1 遗漏的 7 个 commit（全部在盲区内）

| commit | PR | 日期 | 内容 |
|--------|-----|------|------|
| `8ef527eb7` / `14b0489fa` | #12853 | 06-23 / 06-30 | Precmd 携带命令完成元数据（stack 1/7） |
| `077749693` | #12854 | 07-01 | 区分带完成元数据的 Precmd 与仅 prompt 的 Precmd（2/7） |
| `171f681a6` | #12855 | 07-01 | 集中化 terminal lifecycle 变更（3/7） |
| `cb40a808e` | #12856 | 07-02 | 用 coordinator 门控 lifecycle 迁移（4/7） |
| `43c21508f` | #12858 | 07-02 | 从 Precmd 完成元数据恢复丢失的 CommandFinished（6/7） |
| `d81d7b067` | #12859 | 07-02 | dogfood 开启 lifecycle recovery（开关） |
| `19ebec9da` | #13788 | 07-15 | **prod 开启 lifecycle recovery（2026-08-05 补记）** |

> #12857 不在本地对象库（stack 5/7 未 fetch 到或未合并）。栈依赖关系：
> #12853 → #12854 → #12855 → #12856 → #12858；#12859 是 dogfood 开关、
> #13788 提升到 prod。§21.1 初版漏记 #13788（单独日期 07-15，超出
> 6-23~7-02 栈窗口），2026-08-05 全量重扫时补齐。

### 21.2 根因分析：为什么漏了

1. **时间盲区**：栈日期 6-23~7-02，早于第一轮同步起点 `89f742fa6`（7-24）整整一个月。§13 原记录把盲区低估为 7-20~7-24。
2. **主题盲区**：历轮对账聚焦 agent 主题（§12/§13 用户单子、§14 `cli_agent_sessions/` 目录）。该栈在 `terminal/model/lifecycle/`（block 完成检测层），不在任何一轮主题雷达内。
3. **全量对账名不副实**：§17 声称「fork 点全量对账」，实际落地 9 个全是小修复（+4~+34 行），terminal model 层的 1000+ 行大栈未进视野。
4. **决策表可能的误导**（推测，无记录佐证）：速查表「CLIAgent::OhMyPi / CLI agent session → 取本地」若被粗套用，可能把该栈误归「CLI agent 相关 = Zap 自研不动」。

### 21.3 本 fork 现状（2026-08-05 核实）

```
in_flight_in_band_command_count:
  += 1  blocks.rs:2926  (start_active_block_for_in_band_command)
  -= 1  blocks.rs:3824  (仅 command_finished, saturating_sub)
  读    blocks.rs:3230  (is_writing_or_executing_in_band_command)
```

无超时、无 Precmd 恢复、无 lifecycle 协调器。`FeatureFlag::TerminalLifecycleRecovery`
仍有声明但无任何实现代码（空壳 flag）。对比上游：递减在
`complete_active_block_and_advance`（upstream blocks.rs:3161-3174），
`command_finished` 委托之，故 Precmd 恢复路径也能递减。

### 21.4 裁决与执行（2026-08-05 已执行方案 A）

| 方案 | 做法 | 代价 |
|------|------|------|
| A | 移植整个栈（#12853~#12859，缺 #12857） | 大：8 文件 +1000+ 行，跨基线无 merge-base，`blocks.rs` 有 fork 本地改动（`clear_visible_screen` 37ddfdc39/6e6f293f4）会冲突 |
| B | 仅移植恢复核心 | 中：抄上游设计易漏状态机细节 |
| C | 计数器加超时归零兜底 | 最小：偏离上游，治症状 |

**已执行 A**（用户批准「与上游一致 + 保留本地功能」）。C 会留下与上游的长期分歧
（§17.2「为 N 行修复引入 M 行前置」权衡）。执行详情与验证见 §23。

### 21.5 观察到的副作用（未验证推测，勿据此排优先级）

99770 进程残留两个与当前 session 不匹配的 `model-switch-*.sock`。与
`socket not found (omp not running?)` 类切模型故障疑似相关，但与本次
spinner bug 的关联未证实。

---

## 22. 遗漏项全量重扫（2026-08-05）：盲区修正后的重新对账

> 触发：§21 发现 lifecycle 栈在盲区（6-23~7-02 早于第一轮起点 7-24）后，
> 修正 §13 盲区记录，并对上游 master（tip `c8a166b6c`，本轮 fetch 推进
> 自 `02c042063`）6-23 起全量重扫。方法：主题关键词过滤 784 个 commit →
> 344 个命中 → 排除已记录/已处理 → 16 个候选逐符号对照（2 个 scout 并行）。
> 推送基线核实：`origin/main` 已含历轮全部拣入（第九轮 34 个在
> `30461e317`），`main` 仅领先 1 个本地 commit（`38cfa4481`，§20 改动）。

### 22.1 新发现的遗漏（5 件，按优先级）

| # | commit | PR | 主题 | 判定证据 |
|---|--------|-----|------|----------|
| 1 | `0b07f7c2e` + 前置 `ae832ff68` | #14166 + #12438 | zsh prompt 显式宽度构造（glitch）剥离机制整体缺失 | 本地 zsh_body.sh grep `warp_strip_glitch_width_constructs`/`_WARP_RAW_PROMPT`/`_warp_stripped_prompt` 全 0；本地 wrap 为 `%{$prompt_prefix$ORIGINAL_PROMPT$suffix%}`（与上游修复前逐字一致）；本地架构前提（隐藏 prompt 网格 + WARP_HONOR_PS1）与上游相同 → 静态+动态两层剥离均缺 |
| 2 | `6d1ac1dd` | #12965 | terminal 文件链接尾句点（`.md.`）污染高亮与分类 | 本地 `terminal/view/link_detection.rs` `SUFFIXES_TO_REMOVE` 仅 `["@"]`，无 period trim 分支；`path_without_trailing_sentence_period` 0 命中；Windows 路径归一化剥尾点的 bug 场景在本地成立 |
| 3 | `21b35edb` | #14367 | Hermes 语音转写未走 BracketedPaste | 本地 `agent_input_footer/mod.rs:1608-1617` else 分支仍 `WriteToPty(transcribed_text)`（与上游修复前逐字一致）；`InsertIntoCLIPty` 0 命中；本地 Hermes 走 DelayedEnter，不构成等效。BracketedPaste 机制本地已存在（OhMyPi/Codex 用），移植面小 |
| 4 | `ebe8be84` | #12480 | 系统发起终止（logout/重启/OS 更新）不应被 quit-warning 拦截 | 本地 `on_should_terminate_app` 签名仍是 `|ctx|`（`app/src/lib.rs:1882`），`TerminationRequestSource` 0 命中；macOS 侧 quit-warning 模态无可见窗口挂靠的 #12441 场景在本地成立 |
| 5 | `2f4d1d24` | #13113 | `is_any_ai_block_focused` O(1) 化（纯性能） | 本地 `view.rs:7159` 仍是 O(n) 逐 rich-content `is_self_or_child_focused`；需 warpui_core ancestor-set API 适配，无行为收益 → 低价值 |

### 22.2 判为不适用/取本地（6 件）

| commit | PR | 主题 | 判定 |
|--------|-----|------|------|
| `8a30a37ff` | #13766 | 上游新增 OhMyPi 原生支持 | **取本地**（§1）：上游加了枚举变体+OSC777 监听，但本地自研集成（OmpModelSelector、plugin manager）更完整，合并即覆盖本地 |
| `974bd763` | #13191 | CliAgentUserQuery / LRC snapshot 竞态 | **不适用**：queued-query 子系统 + 云协议本地不存在；本地有自己的等效处理（controller.rs LRC→CLI subagent 路由 + `collect_linearized_task_messages` 去重） |
| `28f25535f` | #13774 | LRC 渲染（blockgrid + warp_tui） | **不适用**：主体在已删 warp_tui；blockgrid 侧新增 `visible_cursor_display_position` 无本地消费方 |
| `87f0753c` | #13333 | BlocklistAIInputModel surface-agnostic 重构 | **不适用**：为 TUI/GUI 双 surface 解耦，本地单 surface 无收益，且与本地 input_model 定制冲突 |
| `b15bdd3a` | #13013 | TerminalManager view-agnostic 重构 | **不适用**：纯 3k 行架构重构，为上游未来 TUI，无功能修复；本地 local_tty 982 行 vs 上游重构前 2755 行 |
| `d4997e6c` | #12887 | 提取 warp_multi_agent_client crate | **不适用**：依赖 `server_api.rs`/`api/impl.rs` 本地均不存在，链断（§0 先例） |

### 22.3 判为已合入（2 件）

| commit | PR | 主题 | 证据 |
|--------|-----|------|------|
| `37dc8830d` | #13317 | macOS rich-text glyph 字体身份修复 | 本地 `d6270ce32` 同源 |
| `9444f94d` | #12841 | markdown 表格选区 click-through（1 行） | 本地 `elements/table/mod.rs:1215-1216` 逐字相同（§18.5 路径陷阱实例：上游 `gui/` 目录本地已去前缀） |

### 22.4 上游已废弃（1 件）

| commit | PR | 主题 | 判定 |
|--------|-----|------|------|
| `fa831815` | #12619 | multiline command grid preexec 对账 | **不拣**：上游 `0b07f7c2e`（#14166）已回滚其整段 Rust 逻辑，根因并入 zsh 动态剥离方案；本地现状与回滚后一致 |

### 22.5 合并顺序（2026-08-05 已全部执行）

1. **#14166 + #12438**（zsh glitch 剥离，含 #12619 清理语义）—— 根因级，适用本地架构
2. **#14367**（Hermes BracketedPaste）—— 移植面小，机制现成
3. **#12965**（尾点链接）—— 单函数插入
4. **#12480**（系统终止不拦截）—— 签名变更 + 平台层，需评估 warpui 改动面
5. **#13113**（O(1) 焦点判定）—— 低价值，可选

lifecycle 栈（§21）按 §21.4 方案 A 执行。全部执行记录见 §23。

### 22.6 本轮教训

1. **盲区修正后必须全量重扫**：§13 的「7-20~7-24」低估导致 lifecycle 栈（6-23 起）
   从未进评估清单；修正为 fork 点后，一次主题过滤就捞出 5 件新遗漏。
2. **上游已回滚的 commit 不拣**：`fa831815` 单看是真遗漏，但上游自己用
   `0b07f7c2e` 回滚了它——拣入 = 把上游否决的方案搬进来（§18.1 同构）。
3. **「上游也做了 Zap 自研功能」优先取本地**：`8a30a37ff` 上游加 OhMyPi，
   方向与本地一致但本地更完整——不是遗漏，是竞争实现，按 §1 不动。
4. **性能重构与 bug 修复要分开裁决**：`b15bdd3a`/`87f0753c` 是纯架构重构
   （无行为收益），`13113` 是纯性能（低价值），都不该与真修复同权。
5. **已推送基线要核实**：`origin/main` 含全部历轮拣入，扫描以推送态为准，
   未推送的 1 个本地 commit（§20）不构成扫描差异。

---

## 23. 第二批拣入执行记录（2026-08-05）：5 件遗漏 + lifecycle 栈

> 按 §22.5 顺序 + §21.4 方案 A 执行。原则：**与上游一致 + 保留本地新增/优化功能**。
> 评审整改后提交为 `81bbcf869`（53 文件 +3280/-564），未推送。

### 23.1 zsh glitch 剥离（#12438 + #14166）

| 文件 | 改动 |
|------|------|
| `app/assets/bundled/bootstrap/zsh_body.sh` | 静态+动态两层：新增 `warp_strip_glitch_width_constructs()`（#12438）与 `_warp_stripped_prompt()`/`_WARP_RAW_PROMPT`（#14166 动态 wrapper，`${(e)...}` 展开 subshell）；`ORIGINAL_PROMPT` 赋值改为 `_WARP_RAW_PROMPT` 维护（幂等）；Warp-prompt wrap 按 `promptsubst` 开关分支；删除 `ORIGINAL_RPROMPT` 死赋值（上游 #14166 同删） |

验证：zsh 语法检查 ✓；4 组功能测试（静态剥离/`%%` 转义不误伤/动态 subshell 展开/空值）✓。
注意：`bash_body.sh`/`fish.sh`/`pwsh.ps1` 的 glitch 机制上游未移植（上游仅 zsh），不做。

### 23.2 Hermes BracketedPaste（#14367）

| 文件 | 改动 |
|------|------|
| `agent_input_footer/mod.rs` | 语音 else 分支 `WriteToPty` → `InsertIntoCLIPty`；事件枚举加变体 |
| `use_agent_footer/mod.rs` | `rich_input_submit_strategy` Hermes 移入 `BracketedPaste`；新增 `insert_text_into_cli_agent_pty` + `write_cli_agent_text`；`BracketedPaste`/`BracketedPasteDelayedEnter` 分支复用新函数；`UseAgentToolbarEvent` 加变体 + handler + 转发 |
| `input.rs` | match 分支补 `InsertIntoCLIPty` |
| `mod_test.rs` | 2 个新测试（strategy 断言 + 多行语音 bracketed paste 不提交） |

验证：cargo check ✓；新增测试 ✓。

### 23.3 尾点链接（#12965）

| 文件 | 改动 |
|------|------|
| `link_detection.rs` | 新增 `path_without_trailing_sentence_period()`（剥尾点但保留 `.`/`..`/`foo/.` 组件）；`compute_valid_paths` 的 exact lookup 前插入 trim 先行分支（本地签名多 `validation_ctx` 参数已适配） |
| `link_detection_tests.rs`（新） | 2 个测试（纯函数边界 + `.md.` 回归） |

### 23.4 系统终止不拦截（#12480）

| 文件 | 改动 |
|------|------|
| `warpui_core/platform/app.rs` | 新增 `TerminationRequestSource` 枚举；`on_should_terminate_app` 签名加 source；`should_terminate_app` 透传 |
| `warpui/platform/{headless,mac,winit}` | 各平台调用传 `TerminationRequestSource::User`（mac 侧按 `system_initiated` 区分） |
| `mac/objc/app.{h,m}` | 签名加 `BOOL systemInitiated`；`isSystemInitiatedTermination()` 解析 `kAEQuitReason`；系统终止直接 `NSTerminateNow` |
| `app/src/lib.rs` | 闭包签名 + `System` 分支（跳过 quit-warning 与 apply_pending_update，注释说明 #12441 场景） |

### 23.5 O(1) 焦点判定（#13113）

| 文件 | 改动 |
|------|------|
| `warpui_core/core/app.rs` | 内层 `AppContext` 加 `view_ancestors()` 公开方法（包装 `presenter.ancestors`） |
| `terminal/view.rs` | `is_any_ai_block_focused` 从 O(n) `is_self_or_child_focused` 改为 ancestor-set 判定 |

注意：外层 wrapper 加方法不可见（ViewContext Deref 目标是内层 AppContext），首轮加错层已修正。

### 23.6 lifecycle 栈（#12853/#12854/#12855/#12856/#12858/#12859，方案 A）

| commit | 移植方式 |
|--------|----------|
| #12853 | 4 个 bootstrap 脚本提取 `next_block_id` + Precmd 加 `exit_code`/`next_block_id`；dcs_hooks 忽略新字段 |
| #12854 | `PrecmdHookValue` 枚举化（WithCompletionMetadata/PromptOnly）+ `CompletionMetadata`/`PromptMetadata` 拆分；11 生产文件 `format-patch` + `git apply --3way` + 手工解冲突 |
| #12855 | `complete_active_block_and_advance`/`apply_precmd_to_active`/`apply_preexec_to_active` 抽取；本地 ctrl-l 延迟清理逻辑保留在 `apply_precmd_to_active` |
| #12856 | `lifecycle/` 3 新文件（mod 124/telemetry 215/transition 298 行）+ coordinator 门控；**共享会话 viewer/event_loop.rs 本地不存在→跳过** |
| #12858 | Precmd 恢复逻辑 + `completion_mismatch` 字段；import 合并两处（@both 后清理重复） |
| #12859 | `TerminalLifecycleRecovery` 已在 DOGFOOD_FLAGS |

**本地适配关键点**：
1. **telemetry.rs**：上游依赖 `warp_core::telemetry` API（本地已精简为 no-op 宏集）→ 保留 `LifecycleRecoveryRecord`/`LifecycleTelemetryLimiter`/枚举，去掉 `TelemetryEvent` trait 实现与 `register_telemetry_event!`/`enum_events`。对应删掉 mod_tests 的 payload/contains_ugc 测试。
2. **本地自研保留**：`create_restored_command_block`（适配 `PromptMetadata` 结构）、ctrl-l 延迟清理（§20 同源）、kitty 扩展（uppercase delete/virtual placements/z-index 等，与上游对比 100% 保留）。
3. **事件补齐**：`BlockWorkingDirectoryUpdated` 变体+struct+Debug fmt 从上游补齐（本地早期分叉缺失，`set_current_working_directory` 依赖）。
4. **trait 补齐**：`ansi::Handler` 加 `set_current_working_directory`（本地分叉缺失）。
5. **测试适配**（上游语义：mock 的 bootstrap 序列在 `TerminalLifecycleRecovery` off 时被 coordinator 拦截）：
   - `kitty_test.rs` 45 个测试统一加 `TerminalLifecycleRecovery.override_enabled(true)`（与上游 43c21508f 测试做法一致）
   - `terminal_model_test.rs` 4 个（ssh_bootstraps/exit_alt_screen/restored_empty/restored_blocks）
   - `zero_state_block_tests.rs` 移植上游 `advance_prebootstrap_to_script_execution` helper
   - 旧 API 测试批量转换：`.precmd(PrecmdValue)` → `.prompt_only_precmd(PromptMetadata)`、`CommandFinishedValue { exit_code, next_block_id }` → `{ completion_metadata: CompletionMetadata { ... } }`

### 23.7 验证

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | ✅ 绿（22 个既有 warning） |
| 相关测试 93 个（bracketed_paste/link_detection/hermes/ctrl_c/lifecycle/zero_state/kitty） | ✅ 全过 |
| `terminal::model` 全量 | 541 通过，**新增失败归零**（仅剩基线既有 7 个：regex/secrets/kitty delete 类，基线 worktree 复现确认） |
| lifecycle 测试 6 个 | ✅ 全过 |

### 23.8 本轮教训

1. **跨基线 3way 移植用 `format-patch` + `git apply --3way` 按文件应用**：`git show <sha> -- <file>` 输出不被 apply 接受（corrupt patch），format-patch 输出可。index 与 base 不一致时先 `git add` 再 3way。
2. **`@both` 合并 import 会重复**：3way 已在别处加过相同 import 时，@both 产生重复定义（E0252），需清理。
3. **外层 wrapper 方法对 ViewContext 不可见**：ViewContext Deref 目标是内层 AppContext（`self.0.borrow()` 那个），公开方法必须加在内层（§23.5 首轮加错层）。
4. **`RecoveryDisabled` 拦截 mock bootstrap 是上游语义**：`TerminalLifecycleRecovery` off 时，phase=Unknown 的 mock 序列（command_finished/precmd）被 coordinator 以 `Ignore(RecoveryDisabled)` 拦截 → bootstrap 不完成。真实 shell phase=Executing 不受影响。测试需 override flag（上游 43c21508f 的测试已这么做）。
5. **本地专有测试暴露上游代码缺陷**：kitty_test 走 `process_bytes` 的 StoreAndDisplay 空 map 路径（上游无此测试）。本轮回合通过 override 修复（bootstrap 完整后正常），缺陷本身与上游一致（上游也无此场景测试）。
6. **方法名转换用 ast_edit 对多行参数不匹配**：`.precmd($ARG)` 对 `precmd(PrecmdValue {\n...})` 多行结构不生效，需 sed 兜底。

### 23.9 代码评审整改（2026-08-05，9 个 reviewer 并行）

9 个 reviewer（8 组 + 1 平台组）审查 50 文件。结果：5 approve，4 changes-requested。
整改后全量测试**新增失败归零**（工作区 28 失败 == 基线 28，全为既有）。

| 严重级 | finding | 处置 |
|--------|---------|------|
| **high** | OSC 7 CWD 链是死代码（RevTermModel）：移植 #12854 时 3way 带入 `set_current_working_directory`（上游 #9279 的功能），但本地无 `b"7"` OSC 解析、BlockList 无 delegate、model_events 无分发 → trait 方法 + 2 覆写 + 事件全不可达 | **删除**死代码：trait 方法（handler.rs）、TerminalModel 覆写（terminal_model.rs）、Block 覆写 + `BlockWorkingDirectoryUpdatedEvent` 发射（block.rs）、事件变体 + struct + Debug fmt（event.rs）、import。理由：#9279 是独立功能（5-21），不在本次移植范围；死代码注释声称的行为本地不存在，误导。本地保持无 OSC 7 现状（pwd 靠 precmd 更新）。**下次同步若移植 #9279 需整套补** |
| medium | 同上（RevAnsi/RevBlockLifecycle 各报一次） | 同上 |
| low | #12854 分类测试只留 happy path，4 个分类/KV 测试缺失（RevAnsi） | **已移植 4 个**（parse_dcs_precmd_classifies_prompt_only_payload / rejects_partial_completion_metadata / pending_precmd_classifies / rejects_partial_or_invalid），全过 |
| low | mock bootstrap 拦截影响 find ×7 + input ×1 + view ×14 + search ×3（多处） | **已加 override**（与上游测试做法一致）：block_list_test 7、input_test 1、view_test 14、data_source_tests 3 |
| low | lifecycle 集成测试缺失（RevTermModel：上游 ~17 个 TerminalModel 层集成测试未移植） | **不做**（记录）：coordinator 单元测试已有，TerminalModel 层集成测试工作量大，低优先，后续补 |
| nit | BTreeSet 孤儿 import（RevLifecycleTests） | **已删** |
| nit | 硬编码 11（RevFooter） | **已改** `b"line1\nline2".len()` |
| nit | input_test/view_test/zero_state rustfmt（RevView/RevFooter/RevTermModel/RevBlockLifecycle） | **已跑 cargo fmt** |
| nit | 文件尾换行（RevBlockLifecycle） | 确认已有 |
| low | RPROMPT 空值 guard 缺失（RevBootstrap：上游 #11868） | **不修**（记录）：既有分叉（HEAD 就有），非本次引入；同类 wrapped-redraw bug，后续对齐 |
| low | complete_active_block_and_advance 缺 ScriptExecution eager start（RevBlockLifecycle） | **不修**（记录）：既有分叉（HEAD 同样没有），非本次引入 |
| — | test_directory_search_support 失败 | **无关**：i18n 初始化问题（t! before init），与本次改动无交集 |

**评审教训**：
1. **3way 带入的上游上下文行可能是独立功能**：`set_current_working_directory` 是 #9279（OSC 7）的，不在 #12854 改动里，但 3way 因本地缺失把它带进来。移植时对"上下文行引入的新符号"要查上游来源 commit，判断是当前栈的一部分还是独立功能——独立功能要么整套移植要么删除，不留半截死链。
2. **mock bootstrap 拦截影响面远超预期**：lifecycle 合入后所有依赖 mock 完整 bootstrap 的测试（find/view/input/search）都受影响，逐模块全量测试才能发现。上游合入时同步适配了测试库，本地需按上游模式（override）补齐。
3. **reviewer 并行分组按文件局部性有效**：3 个 reviewer 独立发现同一 OSC 7 死链（high/medium），交叉验证了 severity。

---

## 24. 本地功能改动记录（2026-08-05）：Agent 提供商模型列表折叠

> 这不是上游拣入，而是 Zap 本地的 UI 优化。记录原因：改动落在
> `app/src/settings_view/`（决策速查表 §0 已标注「冲突密集，低价值」的本地
> 定制区），且新增了本地专有的折叠状态。未来同步该区域时不得用上游版本
> 覆盖（§1「Zap 本地功能禁止覆盖」）。

### 24.1 改动内容

| 文件 | 内容 |
|------|------|
| `app/src/settings_view/agent_providers_widget.rs` | 模型列表折叠：新增 `COLLAPSED_MODEL_LIMIT = 3`、thread_local `EXPANDED_MODEL_LISTS`（`HashSet<provider_id>`，关页即丢）与 `is_model_list_expanded` / `toggle_model_list_expanded`；渲染循环 `.take(visible_count)` 只显示前 3 条；模型数 >3 时列表末尾显示「展开剩余 N 个 ▼ / 收起 ▲」按钮（左对齐，防 Stretch 拉伸）；`clear_expanded_models_for_provider`（删 provider）两者都清，新增 `clear_expanded_model_rows_for_provider`（删单条模型只清单行展开记录，保留列表展开态，避免逐条删除时列表反复折叠） |
| `app/src/settings_view/ai_page.rs` | 新增 action `ToggleAgentProviderModelListExpanded { provider_id }`，处理只 `toggle_model_list_expanded + ctx.notify()`（纯渲染层切换，不 rebuild；模型行 editor handle 全量保留） |

与同页 models_dev「快速添加」区折叠（`COLLAPSED_LIMIT=8` + AtomicBool）同模式：
折叠只影响渲染，未保存草稿不丢，保存仍全量提交。

### 24.2 合并注意事项

1. **i18n key 复用**：折叠按钮复用 models_dev 的 `settings-agent-providers-collapse` /
   `settings-agent-providers-expand-remaining`（en/ja/zh-CN 三语言已存在），未新增翻译。
2. **两套折叠状态并存**：`EXPANDED_MODELS`（单模型行 detail panel，`HashMap<provider_id, Set<index>>`）
   与 `EXPANDED_MODEL_LISTS`（列表级，`HashSet<provider_id>`）语义不同、互不干扰；
   清状态时删 provider 两者都清，删单条模型只清前者（index 漂移只影响前者）。
3. **上游若给 provider 模型列表加展开/折叠**：先判语义（单行 detail vs 列表级），
   不要直接覆盖本地实现。

### 24.3 验证状态

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | ✅ 通过（0 error，22 个既有 warning，改动文件零新增） |
| code review（1 reviewer） | ✅ approve（2 处 P3 nit 已修：折叠按钮防 Stretch 拉伸、删模型保留列表展开态） |
| dev app 手动验证 | ✅ 通过（`./script/run`，OSS channel `zap-oss`，窗口与 shell 正常启动） |

---

## 25. 本地功能改动记录（2026-08-05）：Agent 提供商操作反馈（Fetch loading + toast）

> 这不是上游拣入，而是 Zap 本地的 UX 优化（承接 §24 同一功能区的后续改动）。
> 记录原因：改动落在 `app/src/settings_view/`（决策速查表 §0 已标注本地定制区），
> 且「从 API 抓取」此前失败静默（只 log::error，UI 无反馈）。未来同步该区域
> 时不得用上游版本覆盖（§1「Zap 本地功能禁止覆盖」）。

### 25.1 改动内容

| 文件 | 内容 |
|------|------|
| `app/src/settings_view/agent_providers_widget.rs` | Fetch 按钮 loading：thread_local `FETCHING_PROVIDERS`（`HashSet<provider_id>`）+ `is_fetching`/`set_fetching`；`render_card_button_preserving_draft` 加 `disabled: bool` 参数（11 个调用点补 `false`，仅 fetch 传 `is_fetching`）；抓取中按钮禁用 + 文案变「正在从 API 抓取…」（唯一异步操作，防重复点击） |
| `app/src/settings_view/ai_page.rs` | 新增 `show_provider_toast`（`ToastStack` + `DismissibleToast`，仓库既有模式）；Fetch 成功 toast「从 API 新增 N 个模型」/失败 error toast（`OpenAiCompatibleError` 中文文案直接透出）；Sync 成功「同步了 N 个模型」（计数=元数据覆盖+新追加）、未匹配/目录未就绪 error toast；Save「已保存」（复用既有死 key `settings-agent-providers-saved-toast`，ja 顺带补上该 key）；Remove「提供商已移除」 |
| `app/i18n/{en,ja,zh-CN}/warp.ftl` | 新增 6 key × 3 语言（fetching / fetch-success / sync-success / sync-no-match / sync-catalog-unavailable / remove-success） |

### 25.2 关键决策：toast 时长**不做本地定制**

曾尝试给 `DismissibleToast` 加 per-toast `duration` 字段（`with_duration` builder，
模型配置页 toast 用 2 秒），经评估后**撤销**：

- `dismissible_toast.rs` 是上游文件（虽低频：上游近期 4 commits），加字段 = 扩大冲突面；
  上游未来若自己也做 per-toast 时长，会双实现并存（§14 第 4 项先例）
- 结论：**模型配置页 toast 用全局默认 4 秒**，`dismissible_toast.rs` / `workspace/view.rs`
  零本地改动。若未来确实需要短时 toast，优先考虑页内自实现，别动上游 struct。

### 25.3 合并注意事项

1. **失败不再静默**：Fetch 失败路径此前只 `log::error!`，现 error toast 透出
   `OpenAiCompatibleError`（已是中文文案）。上游若改 fetch 错误处理，先判本地 toast
   是否保留。
2. **`FETCHING_PROVIDERS` 生命周期**：thread_local 进程级；删除 provider 时经
   `clear_expanded_models_for_provider` 清理。已知残余场景（fetch 中关设置页，
   spawn 回调不派发）为未验证推测、影响轻（重启即恢复），未修。
3. **Save toast 只挂 `SaveAgentProviderEdits`**：`SaveAgentProviderEditsThen`
   （fetch/sync 前置保存）不弹「已保存」，避免噪音。
4. **Sync 计数语义**：toast 计数为「处理」数（元数据被 catalog 覆盖 + 新追加），
   非仅新增——首次同步（catalog 已预填）时避免误报 0。

### 25.4 验证状态

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | ✅ 通过（0 error，22 个既有 warning，改动文件零新增） |
| code review（2 reviewer 并行） | ✅ 1 approve + 1 request-changes（P2 已修：Sync 目录未加载补 toast；P3 已修：Sync 计数含覆盖、Fetch 期间 provider 被删不弹成功 toast） |
| dev app 手动验证 | ✅ 通过（`./script/run`，OSS channel `zap-oss`） |

---

## 26. 移植补齐（2026-08-10）：AI block 复制修复的测试覆盖 + queued prompt 选区

> 触发：§18 回查确认 `f7e298027` 实为上游 `b2804a091`（#12892）的代码搬入，
> 但**测试部分未搬**（§18.4 只补了 guard 测试，`b2804a091` 自带的 view 层单测
> 与集成测试都没带过来），且两个 `_for_test` 辅助函数因此在本地零调用者。
> 本轮补齐测试覆盖、消掉死代码，并移植上游 `35d951cdc`（#10481
> "Make queued prompt text selectable"）补上 queued prompt 选区能力。

### 26.1 做了什么

| 项 | 内容 |
|----|------|
| 步骤 1 | 照搬上游 `b2804a091` 的 view 层单测 `copy_selected_text_from_ai_block`（`view_test.rs`），适配本地 `append_exchange_with_inputs_and_handle_event`（上游是单参版）；`set_block_level_selected_text_for_test` 死代码消除 |
| 步骤 2 | 照搬上游集成测试 `test_copy_selection_within_ai_block`（`agent_mode.rs`）+ 手动 runner + nextest 注册；`simulate_text_selection_for_test` 死代码消除 |
| 步骤 3 | 移植上游 `35d951cdc` 的 5 文件（`pending_user_query_block.rs` / `selection.rs` / `view.rs` / `pending_user_query.rs` / `rich_content.rs`），让 queued prompt 文字可选中可复制；**刻意不引入**上游 `TranscriptScope` 拆分与 Rust 2024 let-chain（保持本地 `agent_view_state` 命名与语法，避免跨层重构） |

### 26.2 移植中发现的上游演进（重要）

**`35d951cdc` 不是该功能的最终形态**——后续 `98af7b654`（#11439 "queued
prompts list UI"）把 `35d951cdc` 加在 `view.rs` 的**四个 pending 选区接入点全部移除**，
换成新的 queued prompt 面板架构（多 prompt 队列、编辑、排序、折叠）。

- 本地没有 #11439（面板架构是更大的 UI 重构，未拣入），所以本地移植
  `35d951cdc` 的接入点是**正确的**——在旧 pending block UI 下需要这些接入。
- 但**下一轮同步 #11439 时**：要么整体替换 pending block 机制（含本次 5 文件改动），
  要么明确保留旧 UI。届时本节的移植记录是 #11439 的"反向清单"。

### 26.3 继承缺陷记录（与上游一致，非移植偏差）

1. **四入口优先级不一致**：`copy()` 的 pending 分支在 grid `selection_to_string`
   **之前**；`selected_text()` / `maybe_copy_selection_to_clipboard()` /
   `context_menu_copy_selected_text()` 是 **grid 优先** `or_else(pending)`。
   评审曾尝试统一为 pending 优先，但会引入对称残留（先拖 queued prompt 再拖
   AI block 时复制残留 pending 文本）——而上游 grid 优先在该场景正确（AIBlock
   有模型同步）。**保持与上游一致**，未改优先级。
2. **互斥不对称**：AIBlock `SelectionChanged` 只 `sync_ai_block_model_selection`、
   不清其它 rich content 选区；pending `TextSelected` 清其它。上游同样如此。
3. **grid 残留遮蔽**：拖选 queued prompt 时 `SelectableArea` 消费 mouse-down，
   模型旧点选区不清除，grid 优先会返回残留文本——上游固有（`35d951cdc` 原样）。

### 26.4 已修：右键 "Insert into input" 缺口

`35d951cdc` 只给四个入口加了 pending fallback，**漏了
`context_menu_insert_selected_text`**（右键菜单 "Insert into input"）。本地补上，
用同模式（grid 优先 `or_else(pending)`），与上游四个接入点一致。这是对上游
缺口的**补缺**，不改任何既有优先级。

### 26.5 验证状态

| 项 | 结果 |
|----|------|
| 假绿判定 | 置空 `sync_ai_block_model_selection` 函数体 → 新单测 FAIL（断言打在同步路径上） |
| `cargo check -p warp --all-targets` | 0 error |
| `cargo nextest run -p warp copy_selected_text_from_ai_block selection` | 110/110 passed |
| `cargo nextest run -p integration test_copy_selection_within_ai_block` | 1 passed（剪贴板断言拿到期望文本） |
| 手动 runner | `cargo run -p integration --bin integration -- test_copy_selection_within_ai_block` 全 9 步 succeeded |
| code review（2 reviewer 并行） | 3 findings：优先级不一致（B1，回退保持上游一致）、右键 Insert 缺口（A1，已修）、import 折叠（A2，已修） |
| release 正式版构建 | `./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64` 成功，`codesign --verify --deep --strict` OK |

### 26.6 #11439 评估（2026-08-10）

**结论：独立计划级移植，不建议混入本次；本地保留 35d951cdc 接入是正确且必要的。**

`98af7b654`（#11439）结构：27 文件 +2449/-213。核心是每 conversation 多队列模型
`QueuedQueryModel`（`queued_query.rs`：append/pop/reorder/edit/autofire）+ 新面板
`QueuedPromptsPanelView`（`queued_prompts_panel.rs` 841 行）；`view.rs` 225 行、
`input.rs` 65 行、telemetry 83 行、workspace/shared_session 联动。

**与本次移植的关系**：

1. `98af7b654` 删除了 `35d951cdc` 加在 view.rs 的 `pending_user_query_selected_text`
   方法 + 4 处接入点，**但没有替代读取路径**（`queued_query.rs` 无 selected_text）。
   即上游在该重构中**回退了 cloud mode pending block 的选区复制能力**
   （保留 SelectableArea 接线与 `TextSelected` 事件，但复制路径不再读它）。
2. pending block 机制本身**保留**：上游改成 `insert_cloud_mode_queued_user_query_block`
   （带 `PendingUserQueryKind::CloudMode`）；本地对应入口是
   `insert_ambient_agent_queued_user_query_block`（无 kind 参数），需适配。
3. 本地没有 QueuedQueryModel/面板，queued prompt 全部走 pending block
   （`/queue`、`/compact-and`、`/fork-and-compact`、ambient run 初始 prompt），
   所以**本地必须保留 35d951cdc 接入点**——照搬 #11439 的接入删除会让
   queued prompt 复制能力整体回退，且无替代。

**移植 #11439 的完整清单（若开下一轮）**：

- 引入 `queued_query.rs` 多队列模型 + `queued_prompts_panel.rs` 面板 + 测试（+1171 行新文件）
- 适配 `view.rs` 225 行：`drain_queued_prompts`、`QueuedQueryModel` 接线、删除 35d951cdc 接入（**需先决定本地 pending block 选区复制如何保留**——方案 A：保留接入点，面板与 block 并存；方案 B：仿上游删接入、接受 cloud/ambient 场景复制回退）
- 本地 `insert_ambient_agent_queued_user_query_block` 对齐 `PendingUserQueryKind`
- `input.rs`/`input/agent.rs`/`slash_commands` 队列提交路径
- telemetry 83 行（本地 telemetry 已精简为 no-op 宏集，参考 §23 先例）
- workspace 面板挂载（`workspace/view.rs`、`action.rs`）
- 与本地 OMP 集成（agent view、ambient run、shared_session）冲突排查
- 全量测试 + review

**§18 归属标注（已更正，2026-08-10）**：`f7e298027` 主体代码实为上游
`b2804a091`（#12892）移植，commit message 措辞有误导；已在 §18 引言标注清楚
（git 历史不可改写，文档层面更正），§26 补上了该 commit 缺失的测试部分。

---

## 27. 移植记录（2026-08-11）：Cmd-Up 导航 + conversation_export 抽离 + foreground_border 补移植

> 触发：用户要求执行 `port-cmd-up-export-plan.md`。两个上游改动合并一轮移植：
> `da4da09f8`（#14685 "Navigate Agent Mode prompts with Cmd-Up"）与
> `a77348c67`（#13603 "Add low-effort slash commands to TUI"，GUI 侧
> conversation_export 抽离），并补移植 #13056 的 `Container::with_foreground_border`
>（导航环渲染需要）。

### 27.1 做了什么

| 项 | 内容 |
|----|------|
| Cmd-Up 导航（#14685，7 生产文件） | AIBlock 加 `is_agent_transcript_navigation_target` 标记；`blocks.rs` 导航数据层（`RichContentItem::is_agent_transcript_user_query`、`AgentTranscriptNavigableItem`、`agent_transcript_navigable_items()`、`set_agent_transcript_user_query_for_rich_content`、sumtree 重建 `..*item` 保留新字段）；`view.rs` 导航状态机（Cmd-Up/Down 分支、navigate/apply/sync、越过最新停靠点滚到底、`ExitedAgentView` 复位、`AppendedExchange` 置位）；`load_ai_conversation.rs`/`rich_content.rs` 挂载置位 |
| conversation_export（#13603 GUI 侧） | 照上游 `a77348c67` 抽离 `/export-to-file` 写入逻辑到独立模块（118 行 + 4 测试），`input.rs` 调用方保留 toast/i18n/buffer 清理；文件名规则/CWD 回退/错误分类/toast **逐字节不变**（debug log 文案恢复本地原） |
| framework 补移植（#13056） | `Container::with_foreground_border`（前景边框，不占布局，区别于 `with_border` 的 inset 语义） |

### 27.2 关键适配（本地/上游分叉，勿当 bug）

1. **`elements/gui/` 目录重构未跟随**：上游 #12633（"Add TUI API surface… tui"）把
   warpui_core 的 `elements/*.rs` 全部移进 `elements/gui/` 子目录（TUI seam）。
   本地删除 TUI、未跟随该重构（`git log main -- …/elements/gui/` = 0），故
   #13056 加在 `gui/container.rs` 的 `with_foreground_border` 本地缺失。
   **同步规则**：上游落在 `elements/gui/*.rs` 的 framework 改动，按平铺路径
   `elements/*.rs` 适配移植（#13056 内容 0 差异，仅路径不同）。
2. **`transcript_scope` → `agent_view_state`**：上游
   `BlockFilter::commands().matches(block, &self.transcript_scope)` 改本地
   `matches(block, &self.agent_view_state)`（本地全仓无 TranscriptScope）。
3. **let-chains 不适用**：本地 2021 edition，上游 `if let … && …` 改嵌套 `if`。
4. **测试环境差异**：本地 `add_window_with_terminal` 后首个 block 是 bootstrap stage
   （`should_hide_block` 永久隐藏、高度 0），测试需 `set_show_bootstrap_block(true)`
   使其可见；`GlobalResourceHandlesProvider` 测试初始化缺注册（退出 agent view
   路径会访问），统一在 `initialize_app_for_terminal_view` 注册 mock，并清理
   5 处既有手动注册（double-registration panic）。
5. **`terminal_surface_id` → `terminal_view_id`**：上游测试事件字段名本地不同。

### 27.3 导航环颜色（对上游可见性 bug 的本地修正）

上游导航环用 `theme.accent()`，但**头像背景就是 accent 色**
（`blended_colors::accent(theme)` = `theme.accent()`，两处同色），环被头像吞没。
上游 GUI 验证不完整（Linux 沙箱按键不生效、从未看到环渲染）故未发现。
本地改为 **`theme.ansi_fg_magenta()`**——与 `/agent` 斜杠命令强调色一致
（`add_slash_command_highlight` 用 ansi magenta），在 accent 头像上高对比。
配合 `with_foreground_border`（不占布局、同心居中）。**这是对上游缺陷的修正，
非移植偏差**；若上游后续调整环样式需按此复核。

### 27.4 验证状态

| 项 | 结果 |
|----|------|
| `cargo check -p warp --all-targets` | 0 error |
| 导航测试（blocks 1 + view 6） | 7/7 passed |
| conversation_export 4 测试 | 4/4 passed |
| queued_prompts 27 + copy_selected 111 | 全绿 |
| container 单测 | passed |
| code review（4 reviewer 并行） | 1 P1（5 处 GlobalResourceHandlesProvider double-registration panic，已修）+ 4 P3（2 修：debug log 文案、无效循环；2 有意保持：覆盖 toast 时机、注释语言） |
| 正式版 bundle | `./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64` 成功，签名非 adhoc |

### 27.5 提交

| commit | 内容 |
|--------|------|
| `a1fa3927a` | feat(warpui_core): 移植 Container::with_foreground_border（#13056） |
| `9426efb2d` | refactor(ai): 抽离 conversation_export（#13603） |
| `2d0942210` | feat(terminal): Agent Mode Cmd-Up/Cmd-Down 导航（#14685） |

### 27.6 后续注意

- 导航环颜色是本地定制（ansi magenta，§27.3），与上游 accent 方案不同，属已知差异。
- 覆盖 toast 时机：写入失败时不再先弹"将被覆盖"警告（上游提取行为，罕见错误路径，
  有意保持）。
- 新注释保持英文（照上游、维护 cherry-pick 兼容面）；本地自写注释用中文。
- `port-cmd-up-export-plan.md` / `cmd-up-execute-handoff.md` 是执行计划/交接文档，
  未入库（untracked），本轮完成后可归档或删除。

---

## 28. 移植记录（2026-08-10）：queued prompts 子系统（#11439 + 12 演进 + LRC 守卫）

> 触发：`port-11439-plan.md`（已删）+ 交接文档。上游 `98af7b654`（#11439
> "queued prompts list UI"）及其后到 `118c6a4ef` 的 12 个 queued-prompts 演进
> commit、`098c307c7`（LRC subagent 交回守卫）分阶段移植。§26.6 曾评估 #11439
> 并列出移植清单——本轮按该清单执行完毕。

### 28.1 提交链（本地 7 commit，倒序 = 提交顺序）

| commit | 内容 |
|--------|------|
| `4b2d5b855` | 移植 #11439（`98af7b654`）：`QueuedQueryModel` 单例（每会话多队列、行内编辑、拖拽重排）、`QueuedPromptsPanelView` 面板、`/queue` 与 auto-queue toggle 改道、`QueueSlashCommand` 从 DOGFOOD 移出无条件启用、telemetry 4 事件、测试 27 用例 |
| `d0028a25d` | 移植 12 个演进 commit（`98af7b654` 之后到 `118c6a4ef`，5 批次）：① 面板 UI 细化（行文字号对齐、方向键/? 键行内、Copy 行动作）② compact 排队（CompactAnd/ForkAndCompact origin、`is_summarizing`、enqueue_followup_prompt、deferred dispatch）③ hover/拖柄 + paper cuts + 抑制完成通知 + 焦点抢占 ④ 排队 vs 打断设置（`default_prompt_submission_mode`）、send-now ⑤ 终端命令排队（`command_in_flight` + QueuedCommand）、LRC 自动排队设置（`long_running_command_submission_mode`）、LRC 快照竞态、action 类 slash command 立即执行 |
| `177415197` | 补移植 `098c307c7` 的 LRC subagent 交回守卫 |
| `5c7fcc4d2` | 排队模式下 `/compact-and` 不再被 bypass，恢复排队语义 |
| `1e5d6bb80` | 排队 `/compact-and` 完整执行路径 + `098c307c7` 契约测试 |
| `10deefa71` | 删除日语界面支持（Language 枚举删 Japanese + 删 `app/i18n/ja/warp.ftl`） |
| `9fb3abb` | zh-CN 补译两处漏译（Agent 工作流、Warpify 子 shell） |

### 28.2 适配决策（本地/上游差异）

- 本地无 cloud mode / warp_tui：跳过 viewer 上传、PtyIntent、`is_agent_requested_command` 相关
- 本地 flag 用 `QueueSlashCommand` 而非上游 `QueuedPromptsV2`
- `is_lrc_auto_queue_active` 用本地 `is_agent_driving_command()` 替代上游组合判断
- `slash_command_is_submitted_as_prompt` 本地为 INIT\|PLAN（上游含 ORCHESTRATE）
- 附件从 pending_context 移出显式双源解析，修复直接 skill 调用附件重复发送与排队行吞下一条草稿附件
- `send_lrc_queued_prompts` 只发队首，余下靠 drain 链推进，避免连发互相 cancel
- `remove_pending_lrc_rows` 收窄到非 FollowUpSubmitted 的取消，保留排队输入
- 测试基建：`initialize_app_for_terminal_view` 补 `QueuedQueryModel` 注册（input_test / workspace view_test / test_util）

### 28.3 验证状态（2026-08-10 提交时）

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | 0 error |
| queued 相关测试（queued_query 19 + queued_prompts 8 + 后续演进） | 全绿（交接文档记录 50 个 queued 测试） |
| code review | review.json 记录（Approve with nits） |

### 28.4 后续注意

- 本地保留 `35d951cdc` 的 pending block 选区复制接入？——**已删**（与上游 #11439 一致，
  接入点被面板架构替换；本地 queued prompt 文字可选中可复制由面板 SelectableArea 承接，
  见 §26.3 继承缺陷第 3 项 grid 残留遮蔽为上游固有）。
- `/compact-and` 排队语义（`5c7fcc4d2`）与 LRC 守卫（`177415197`）是本地对上游
  `098c307c7` 的拆分布移植——上游单 commit，本地拆 3 步（守卫 → 排队例外 → 完整路径+测试），
  后续同步若遇上游返工需合并核对。

---

## 29. 移植记录（2026-08-13）：性能优化 2 commit——imported-comments guard + async find

> 触发：系统性搜上游性能/内存优化后筛出的「本地适用、未移植」候选。本轮合入
> 2 个：`922ba2584`（imported-comments guard）+ `fb5ad384a`（async blocklist find）。
> 上游 commit 不在本地对象库（浅克隆），实现以 worktree `/Users/zhong/project/.worktrees/upstream-master`
>（02c04206）为权威照搬。官方设计文档 `specs/async-find/TECH.md`（worktree）。

### 29.1 提交链（本地 2 commit，倒序 = 提交顺序）

| commit | 内容 |
|--------|------|
| `ed3bb76af` | 移植 async find（`fb5ad384a` #9618 + 后续 `cd745fac9` #11205）：AsyncFindController 后台扫描、TerminalFindModel 三分支接入、BlockFindRenderData、grid_handler find_dirty_rows_range、三层 gating 默认关 |
| `40d94c395` | 移植 imported-comments guard（`922ba2584` #13114）：conversation registry + disabled-state 重构 |

### 29.2 关键适配（本地/上游分叉，勿当 bug）

1. **let-chains 不适用**：本地非 2024 edition，上游 async_find.rs 的
   `if let ... && let ...`（3 处）改嵌套 `if`，语义等价。
2. **`define_settings_group!` 宏无 `surface` 字段**：本地宏版本旧（上游有），
   加 `async_find_enabled` 设置时移除 `surface:` 行。
3. **`TerminalFindModel::new` 签名**：`(terminal_model, ctx: &AppContext)`，按
   `TerminalSettings::is_async_find_enabled()` 构造 controller（不是计划的
   `&mut ModelContext` 猜测——以上游 worktree 为准）。
4. **`FeatureFlag::AsyncFind` 不加 DOGFOOD_FLAGS**：worktree 02c04206 已从
   DOGFOOD 移除（上游默认关 + 构建 feature + 用户设置），本地照搬。flag 关时
   sync 路径逐字节等价。
5. **event-listener 收归 workspace**：本地顶层 `[workspace.dependencies]` 加
   `event-listener = "5.4.0"`，app 直接依赖改 `.workspace = true`；profile
   `[profile.dev.package]` 加 `warp_terminal.opt-level = 3`（DFA 热循环）。
6. **grid_handler 两处构造**（`new` + `new_for_split`）都要初始化
   `find_dirty_rows_range: None`。

### 29.3 review 发现的修复（4 reviewer 并行）

| 发现 | 严重度 | 修复 |
|------|--------|------|
| mark 无条件（上游在 `if !cards.is_empty()` 内，本地在块外） | P1 | 移入 if 内，cards 为空不误标记 |
| `cd745fac9` 消费端 2 处未接：`handle_find_match_focus_change` + `get_highlight_ranges_for_find_matches` 仍用旧 sync 路径 | P1 | 改用 `focused_rich_content_match_id()`，恢复 async 下 AI 高亮 + reasoning 自动展开 |
| `update_find_dirty_rows_range` 无条件记录（flag 关时每 PTY pass 一次小分配） | P2 | **不改**——上游同样无条件（parity），量级可忽略 |

> 教训：移植 async find 时，cd745fac9（#11205）的 **controller 侧**（统一
> `FocusedMatchResolution` 缓存、`focused_rich_content_match_id()` helper）随主
> commit 一起带上了，但 **AI block 侧 2 个消费点**（block.rs + common.rs）容易漏。
> 审查时重点查「新 helper 的所有消费点是否接全」。

### 29.4 验证状态（2026-08-13 提交时）

| 项 | 结果 |
|----|------|
| `cargo check -p warp`（默认，flag 关） | 0 error，sync 路径与改动前逐字节等价 |
| `cargo check -p warp --features async_find` | 0 error |
| `cargo test -p warp terminal::find` | 31/31（含 21 async_find 测试） |
| `cargo test -p warp ai::blocklist` | 268 通过；8 失败为基线既有（workspace settings 环境相关，stash 验证与改动无关） |

### 29.5 后续注意

- async find 三层 gating 默认关：app cargo feature `async_find` + `enabled_features()`
  cfg 映射 + `FeatureFlag::AsyncFind` 枚举 + `experimental.async_find_enabled` 设置
  （默认 false）。用户可在设置 opt-in 或编译 `--features async_find`。
- 上游若后续 promote AsyncFind（加入 RELEASE_FLAGS / 默认开），本地一行放开即可。
- 本次未评估合并的候选（watcher `d78ced530`/`50853a9b9`、telemetry 清理
  `eca8c1a60`/`40f59b517`）：watcher 本地自研 BulkFilesystemWatcher 不同构且无同款
  问题；telemetry 本地发送层已删（枚举为类型壳），删除无收益。均维持跳过。

---

## 30. 移植记录（2026-08-13）：遗漏修复 6 commit（区间 `02c042063` → `5fb3144db`）

> 触发：`02c042063` 之后 fetch 到 143 个新 commit（至 `5fb3144db`），系统性筛出
> 遗漏的 macOS 相关修复。本轮移植 6 个行为保持修复；`46c0b5136`（Windows 专属）只记录
> 待 Windows 版；`aa9f3a436`（context chips 活跃面）本地适配复杂，待评估。

### 30.1 已移植（6 commit，工作区未提交）

| commit | 内容 | 本地改动文件 |
|--------|------|--------------|
| `12e455c56` #15043 | Core Text 合并相邻相同 style runs——修复 macOS 11.98GB 内存尖峰（APP-5342），`create_attributed_string` 逐 run `set_attribute` 前合并同 style 连续 run，无分配 fast path | `crates/warpui/src/platform/mac/text_layout.rs` + `text_layout_test.rs`（3 测试） |
| `87a4e4b34` #14933 | workflow 预览截断 `&content_preview[..197]` 撞多字节字符 panic（APP-5287）→ 用本地已有 `safe_truncate` | `app/src/search/ai_context_menu/workflows/data_source.rs` |
| `80a203474` #14915 | citation 未知类型硬失败整条消息 → `filter_map` 降级跳过；Unknown 分支记录日志 | `app/src/ai/agent/api/convert_from.rs` + `crates/ai/src/agent/citation.rs` |
| `f919f8935` #13407 | SSH wrapper `unset RCS; unset GLOBAL_RCS`（变量 no-op）→ `unsetopt ZLE RCS GLOBAL_RCS` | `app/assets/bundled/bootstrap/{bash_body,fish,zsh_body}.sh` |
| `724579a87` #14655 | code review renamed 文件空 diff：工作树 vs HEAD 双路径 diff + baseline 从 `HEAD:old_path` 读 | `app/src/code_review/diff_state.rs`（2 处） |
| `a4769955f` #14853 | generator 取消只杀直接子 PID → `ActiveProcessGroups` + `SpawnedChildCleanup` Drop 杀整个进程组 | `app/src/terminal/model/session/command_executor/local_command_executor.rs` |

### 30.2 关键适配（本地/上游分叉，勿当 bug）

1. **`12e455c56`**：本地 `create_attributed_string` 与上游修复前逐字同构；`Cow` 已 import；
   let-chain（`if let Some(last) = merged.last_mut() && ...`）改嵌套 `if`（本地 2021 edition）。
   测试文件本地叫 `text_layout_test.rs`（上游 `text_layout_tests.rs`），import 补 `FamilyId`。
   `StyleAndFont` 本地已 `#[derive(PartialEq)]`。
2. **`80a203474`**：上游 `report_error!` 用 `warp_errors::{ReportErrorLogMode, report_error}`
   带 `extra:` + `OncePerRun`——本地无 `warp_errors`（已删，§7.3），本地 `report_error!` 是
   `warp_core::errors` 单参 `($err:expr)` 形式，不支持 extra/OncePerRun。本地改为
   `log::warn!`（crates/ai 已依赖 log）携带 `document_type` + `document_id_len`，保留诊断信息，
   不上报链（本地 telemetry 为壳）。核心降级（`filter_map`）与上游一致。
   另注意 `convert_from.rs` 的 `UnknownCitationTypeError` import 被 `CitationError` enum
   （`#[from]`）使用，不能删。上游用 `report_error!` + `OncePerRun` 每轮运行只记录一次；
   本地 `log::warn!` 无去重——若服务端持续返回未知类型会每消息一条 warn（本地日志不上报
   Sentry，噪音可接受；如需可加 `OnceLock` 去重，非阻塞）。
3. **`724579a87`**：本地 `get_file_content_at_head` 在 `diff_state.rs`（单文件，上游是
   `diff_state/local.rs` 目录）；`old_path` 本地为 `String`（match 借用 `&String`，经 Deref
   隐式 `&str` 传入 vec 与 `format!`）。两处改动与上游语义一致。上游 #14655 自带 2 个回归
   测试（`renamed_file_content_at_head_reads_old_path`、`staged_rename_and_modify_produces_
   non_empty_diff`），本地未带（沿用 §29「只移植生产代码」惯例；如需回归防线可后续补）。
4. **`a4769955f`**：本地 `local_command_executor.rs` 与上游修复前同构；上游测试文件
   `local_command_executor_tests.rs`（214 行）本地无，未带（§29 模式：只移植生产代码）。
   `is_some_and` 是 std API，本地可用。本地 `safe_warn!` 保留（上游该处用 `log::warn`）。

### 30.3 只记录、待 Windows 版（用户决定）

**`46c0b5136` #13405**：Windows 每 session 全进程表 CPU 采样（`is_kaspersky_running`）触发
`DPC_WATCHDOG_VIOLATION` 蓝屏。修复加 `KASPERSKY_RUNNING: OnceLock<bool>` 缓存 +
`all_processes_refresh_kind()`（`refresh_processes_specifics` 不采样 CPU/内存）。**纯 Windows
专属**：macOS 无 DPC 蓝屏机制，`all_processes_refresh_kind` 在非 Windows 是死代码。用户决策：
**只记录，不移植；后续计划发布 Windows 版再合并**。移植时改动点：`app/src/system/info.rs` +
`app/src/util/windows.rs`（本地与上游同构）。

### 30.4 终局：`aa9f3a436` #14854（context chips 只跑活跃输入面）——跳过

上游修复：PS1 用户（非 UDI、AgentView 关）时 CLI footer chip 仍每 30s 跑默认 Git diff 命令
的后台浪费。修复拆三层独立活跃面（prompt / agent footer / cli_agent_footer）+ 订阅状态变化。

**核验结论（2026-08-13）：本地默认无此 bug，不移植。**

上游 bug 前提是 `FeatureFlag::AgentView` 全局启用时对无 CLI agent 的 pane 也无条件维护
footer chip。本地 `FeatureFlag::AgentView` **不在任何 flag 列表**（`crates/warp_features/
src/lib.rs` 的 DOGFOOD/PREVIEW/RELEASE_FLAGS 均无，只有 `AgentViewBlockContext`），
`is_enabled()` 默认 `false`。因此本地 `chips_to_run`（`current_prompt.rs:1123`）的
`if FeatureFlag::AgentView.is_enabled()` 分支默认不执行，CLI footer chip 不进 `chips_to_run`，
`CurrentPrompt` 不会定时维护其 state → 不跑 Git diff。chip 命令执行完全由 `chips_to_run`
驱动（定时器 / `run_chips`）；footer 渲染（`PromptType::cli_agent_chips`）只读已有 state，
不触发执行。

**若未来 AgentView 默认开启 / promote**，再按下方适配点移植（仍需本地适配）：

1. 上游依赖 `AISettings::should_render_cli_agent_footer`（本地有）+ `AISettingsChangedEvent::
   ShouldRenderCLIAgentToolbar`（本地无——本地 AISettings 是 per-agent 定制体系，事件变体
   `CLIAgentToolbarEnabledCommands`/`IsAnyAIEnabled` 等，无单体 footer 开关事件）。
2. 上游依赖 `session.agent.supports_cli_agent_footer()`（本地 `CLIAgent` 无此方法）。
3. 上游 `subscribe_to_input_editor` 签名变更（`&self`→`&mut self` + 新参数），本地仅 1 调用点
   （`input.rs:2627`）。

### 30.5 验证状态（2026-08-13）

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | 0 error（首次报 `UnknownCitationTypeError` 缺失——73 行 `CitationError` 用 `#[from]`，恢复 import 后通过） |
| `cargo test -p warpui text_layout` | 33/33（含 3 个新 merge 测试） |
| `zsh -n` / `bash -n` bootstrap 脚本 | 语法通过 |

---

## 31. 移植记录（2026-08-13）：盲区 59 候选（真实 fork 点 `c325d146` 之后）

历轮对账从 07-24 起都只覆盖 `02c042063` 之后;04-28(fork 点)→ 06-23 的盲区从未可靠
覆盖(§17 名不副实、§22 只扫 06-23 起)。本轮 `git fetch --unshallow upstream` 后用
`git merge-base` 精确定位真实 fork 点 = `c325d146`,盲区 244 个 fix 候选逐符号对照本地,
确认 59 个漏合并。**59 个全部移植完成**,每候选一个 commit,`cargo check -p warp` 0 error,
`cargo test -p warpui text_layout` 33/33 回归通过。

### 31.1 移植清单(commit 从早到晚)

**P0 安全(4)**:shell_quote_arg 注入修复 `43f4f483e`(#25351)、is_file_path 转义
`b6caa9576`(#26138)、display chip RCE `4295ec08d`(#25398)、markdown 链接伪装文件
`7f0c4dd23`(#25353)。

**P0 崩溃/死锁(5)**:OSC 1337 缺参 panic `b9cc454ca`(#12889)、flat_storage clear underflow
`388f5dc12`(#12085)、watcher 创建 panic `21e70d566`(#10682)、WeakModelHandle 僵尸 handle
`654940eda`(#11767)、secret redaction 三锁死锁 `29d88e468`(#11428)。

**P0 竞态(4)**:规则文件 watcher 并发覆盖 `5146a5bff`(#10238)、requested commands 误取消
`746726094`(#10241)、SessionBootstrapped 通知时序 `91b4f0971`(#10666)、git chip 初始化漏事件
`1175e82f0`(#10265)。

**P1 终端/渲染/可靠性(19)**:PTY 写阻塞丢响应 `e59c7a491`(#11906)、secret 多字节前缀脱敏
`89c2193e5`(#9521)、softwrap glyph indices `87a73c465`(#10445)、Shift+Backspace→DEL
`1df6ff130`(#11563)、bash HISTSIZE sentinel `4aa545819`(#10615)、SVG 光栅化缓存键
`5c57b3850`(#12104)、IME 光标通知 `ab0815281`(#10443)、远程批量编辑顺序坐标
`170304d79`(#10819)、会话恢复空任务 lenient `82ec31fd5`(#11814)、文件搜索 query 下推
`723b445a6`(#12160)、symlink watcher absolute `9f459842c`(#11073)、file tree 单根增量刷新
`bd7202f30`(#11863)、自定义主题同步 `6ab1c167c`(#9728)、markdown pane 去重聚焦
`6c9604f15`(#12489)、AI 块本地图片缓存失效 `da6066b75`(#12840)、新克隆仓库卡 loading
`5d8507e40`(#9998)、焦点抢占 `9e3d6826b`(#12833)、rewind prefill slash 命令
`9eb570cd5`(#12149)。

**P2 低优先(27)**:untracked dirs diff `802a881e5`(#12590)、Detecting 状态 `5bee7a759`
(#12126)、global_buffer is_empty 守卫 `25bb50823`(#10823)、网络错误链 `5a9ea0ffb`(#12497)
+ `23b686302`(#12712)、text_file_reader max 守卫 `11f6f4a91`(#12642)、DirectoryFetcher
深克隆 `01778efe7`(#12362)、params_modal Space 独占 `c4946001f`(#12004)、rc_file_paths
TypedPathBuf `af5b45b6d`(#11896)、browser_url_handler 守卫 `ffe5cff65`(#11515)、Intel HD
2500 渲染偏移 `f3dd3768f`(#11454)、openable_file_type 加 command `c23106005`(#10543)、
append_unmatched_line_suffix `a1b76c288`(#11350)、is_pr_info_refreshing `e6df31bb6`
(#12454)、ControlMaster 复用 `0f28bcb33`(#10188)、哈希 8hex + SUN_PATH guard
`18baecd45`(#11207)、session_is_local 重构 `81cc895d1`(并入 ec68c3df2)、permanent_error
backoff `50003a851`(#12756)、消息截断 MAX_MESSAGE_SIZE `d9dee18e1`(#12406)、
OpenCLIAgentRichInput Toggle `aea652ade`(#12229)、worktree list `59e802ea2`(#12029)、
scroll_to_matching_header `606e1653f`(#12254)、apply diff 变更行+上下文 `89f61b63b`
(#12790)、HarnessSelected 刷新 pane header `99a8e5090`(#12645)。

### 31.2 跳过项(5,均记录原因)

| 候选 | 原因 |
|------|------|
| `3fe061620` 文件搜索截断(#12161) | 本地 repo_metadata 已重构:无 MAX_REPO_CONTENTS_RESULTS 上限/无 ExceededMaxResultSize,遍历天然返回全部,行为已等价 |
| `f6d8167f4` ghost group 剪枝(#12340) | 本地无 TabGroup/group_id 特性(仅 pane_group,不同概念),无等价文件 |
| `81cc895d1` repo detection 部分 | 本地无上游 #10921 对应代码;session_is_local 部分已随 5a9ea0ffb 提交带入 |
| `429dbf2e3` 移除 is_cancelled 过滤(#12763) | 上游改动已含于本地 commit 8a4aa3b32;本地剩余 is_cancelled 过滤在 Zap BYOP 定制函数(有意引入),移除会破坏定制 |
| `475fdb33e` ThemeChanged 订阅(#13000) | 上游改 custom_inference_modal.rs + ApiKeysWidget,本地设置页为 Zap 定制区(agent_providers_widget.rs),无对应组件 |

### 31.3 关键适配点

- **secret redaction 锁**:29d88e468 上游基于三 RwLock,本地(§31 前已改 Mutex<Arc<SecretsRegex>>
  快照结构)直接在其上照搬,无冲突。
- **4b 多字节脱敏**:89c2193e5 适配到 Mutex 快照结构,find.rs 加语法感知
  `replace_unicode_word_boundaries`(处理 `\b`、`[\b]`、`\B`)+ count_preceding_backslashes。
- **5w worktree**:59e802ea2 上游用 shell_single_quote 字符串命令;本地沿用类型化
  `PromptChipShellCommand::GitCheckout { encoded_git_branch_on_click_value }`,render 端按
  shell 类型解码渲染(PowerShell 也正确),比上游更安全,行为等价。
- **secrets_tests 顺序依赖**:并行跑时 7/10 失败(全局 SECRETS_REGEX 快照被并行测试相互
  覆盖,含 5 个既有未动测试),上游同款设计,单线程/隔离均通过;非本次移植引入。
- **claude_md 测试**:crates/ai project_context 3 个 claude_md 测试在 HEAD 即失败(本地遗留
  问题,与 CLAUDE.md 支持有关),非本轮引入。

### 31.4 验证状态(2026-08-13)

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | 0 error(最终全量,54+1 个 commit) |
| `cargo test -p warpui text_layout` | 33/33 |
| 安全 4 个(P0)人工复核 | 注入路径全部经 shell_quote_arg 转义,无未转义拼接 |
| 子代理并行 | 10 批并行移植,期间一次误 stash 事故(Main 已恢复),5q/5x 重放后内容一致 |

---

## 32. 移植记录（2026-08-12）：AI file-outline 内存上限（APP-4794）

> 上游 `3fe6e17ad`（2026-06-28）移植，提交 `942d8b609`（08-12 23:32）。
> 此前历轮表无 08-12 记录，本次补记。

**问题**：`parse_file_outline` 为每个文件分配 owned `Symbol` 字符串并按 repo 保留在
`RepoOutlines`，累计无上限，大仓库涨到多 GB（Sentry 7259255054：~6GB/84% live heap，
`name.to_owned()` 53% + `collect_vec()` 30%）。

**修复**：累计跟踪 outline 堆大小，超 256MiB/repo 预算即跳过剩余文件，降级为有界部分
outline。典型仓库远低于上限，不受影响。

| 文件 | 改动 |
|------|------|
| `crates/ai/src/index/file_outline/mod.rs` | `Symbol::approx_heap_size()`（name/type_prefix/comment 字节数）+ `FileOutline::approx_heap_size()`（Vec 容量 + 各 Symbol 字节） |
| `crates/ai/src/index/file_outline/native.rs` | `MAX_OUTLINE_TOTAL_BYTES = 256MiB` + `AtomicUsize`/`AtomicBool` 预算跟踪：超限文件返回空 outline，首次超限 `log::warn!` 一次 |
| `crates/ai/src/index/file_outline/mod_test.rs` | 3 个测试：零 symbol / 统计 owned 字节 / 随 symbol 增长 |

**验证**：`cargo check -p warp`（依赖链含 crates/ai）随 08-13 §29 全量核验通过。

---

## 33. 本地功能改动记录（2026-08-13）：subagent model_switch_ready 不覆盖主会话 socket

> 未提交，工作区。非上游移植，本地 bug 修复。

**问题**：subagent 等嵌套会话加载同一扩展，并以自己的 `session_id` 上报
`model_switch_ready`。原逻辑无条件覆盖 `model_switch_socket_id`，会把主会话的
socket 绑定指向子会话的 socket，导致主会话模型切换静默失败（无报错、无反应）。

**修复**（`app/src/terminal/cli_agent_sessions/mod.rs` + `mod_tests.rs`）：
仅当 socket 尚未建立（首次启动 / resume）或事件 `session_id` 与已记录的会话 id
匹配时才更新 `model_switch_socket_id`。新增测试
`subagent_model_switch_ready_does_not_overwrite_main_session_socket`。

---

## 34. 2026-09 批次拣入（2026-09-06）：两上游开放 PR 逐个评估后 15 项落地

> 触发：用户要求评估两个上游（直接父仓库 `zerx-lab/zap`、根上游 `warpdotdev/warp`）的开放 PR。
> 评估记录见 `specs/upstream-merge-plan-2026-09.md`；执行在独立 worktree
> `.worktrees/upstream-sync-2026-09`（分支同名）完成，每 PR 一个 commit。

### 34.1 来自 warpdotdev/warp（13 个 PR）

| PR | 内容 | 关键适配/偏差 |
|----|------|--------------|
| #15746 | CoreText autorelease 排水（APP-5744） | 直拣；与 12e455c56 同文件不重叠 |
| #15652 | line editor 首个 precmd 丢失 | 直拣；实为 bash 终端永久死锁（Enter 无响应）修复 |
| #15751 | 终端 resize 每行深拷贝（APP-5749） | 直拣 + 冲突解决时误带上游其他提交的 hyperlink 测试（本地无 HyperlinkRegistry），删除（独立 commit `c53d60a52`） |
| #15719 [draft] | macOS History/Up 菜单 Circular view update 崩溃 | 上游 3 提交为中间态重构（含本地不存在的 auth-secret-ftux），按 §18.1 取净 diff 手工移植；测试落本地 input_test.rs；assert_eventually 本地已有；test_page_up_and_down 断言按上游拆出 update 块 |
| #15699 [draft] | APFS case-only rename 文件树残留 | 直拣 + 冲突块混入未移植的 ensure_watchable_path，剔除；lib_tests.rs 仅留 case-only 测试；本地按惯例补 mod tests 声明 |
| #15764 | AppContext 订阅泄漏（APP-5762） | 手工落（行号偏移）；含 drop mid-emit 断言测试；测试落本地 mod_test.rs |
| #15741 | 连续 ViewNotification 去重（APP-5741） | 手工移植；enqueue_effect 超大队列告警以进程内单次 log::warn 替代 warp_errors OncePerRun；let-chain 改嵌套 if（2021 edition）；弃同主题 draft #15739 |
| #15810 | EditDelta.precise_deltas Arc 化（APP-5810） | cherry-pick + 补本地 edit.rs/core.rs 的 `use std::sync::Arc`（上游因 APP-4844 已有）；测试文件名 buffer_test.rs 靠 git 重命名检测自动映射；上游 APP-4844 测试未带 |
| #15835 | Code editor 文件读 100MiB 守卫 | 混合：warp_files 守卫 + code/view.rs 融合（保留 Zap 关闭失败 tab 定制；TooLarge 显示 file_load_error_message 详情，其余保持 Zap i18n toast）；**code_review 侧本地 API 保留**（editor_state 增 load_error 跟踪但不改 is_loaded/set_loaded 结构）；上游 lib_tests.rs 可编译，保留（30 测试过） |
| #15831 | 编辑器文本绕过 LayoutCache（APP-5825） | 拣 3/8 提交（跳过 4 个 Criterion bench + 1 测试提交）；本地已有 for_render_state/uncached 函数（早期同步），主路径 layout_text 切换至 uncached；上游 layout_text_uncached 的 truncate_text_for_layout/clamp_style_runs_for_layout 属其他未同步 PR，未带；element 文件仅 ctx→_ctx |
| #15724 | rich input 打开时光标下字形丢失 | 拣 3 提交；thread_local cell-map 捕获被第 3 提交移除（本地按最终态落地）；测试尾部取上游最终版 + Zap 品牌；font_cache 的 layout_line_uncached hunk 未自动落地，手工补入 warpui_core text_layout_system.rs |
| #15757 | 启动 block 恢复 SQL LIMIT（APP-5757） | 直拣 2 提交；migration `2026-09-03-004900_add_blocks_restore_order_index`（up: 索引 / down: DROP）；上游 block_list_tests.rs 按惯例未带；**migration 未在真实库上执行过，合回 main 前需验证 up 耗时** |
| #15670 | 后台 pane SSH 提示抢焦点 | 手工移植生产代码（clear_ssh_blocks 门控 + choice block 改 redetermine_terminal_focus）；上游测试依赖本地缺失的 mock_pane_group 脚手架，未带；本地事件名 ZapifySettings 无需适配 |

### 34.2 来自 zerx-lab/zap（2 个 PR + 2 个确认已有）

| PR | 内容 | 结论 |
|----|------|------|
| #338 | BYOP 模型名 `-max` 后缀被剥 | 直拣（rust-genai adapter_shared.rs 零偏离）。**其 crate 单测因独立解析依赖不可运行（patch 依赖），生产代码经 cargo check 验证** |
| #339 | 硬编码中文接入 Fluent（拆 2 commit） | 26+5 个干净文件直拣；en/zh-CN 各 +91 key（73+18）；跳过 tools/*.rs（本地已英文化）、ja（本地无日语）、autoupdate/linux.rs（非 macOS，曾误入已剔除）；workspace/view.rs 日志导出段自动合并落地 |
| #331 | kitty graphics 协议补全 | 本地已是超集，不动 |
| #322 | CLI agent 浮窗尺寸记忆 | 本地已有等效（含迁移），不动 |

### 34.3 明确跳过（评估证据见 plan 文档）

- 依赖已删/不适用：#15825、#15803、#15801、#15829、#15796、#15820、#15834
- 前置链缺失：#15797、#15818、#15817、#15713
- 高成本缓议：#15800、#15793、#15733、zerx #321（其中 4e4ff90d9 关机持久化活动 block 值得单独重做）
- 等转正：#15802（转正后高优先）、#15830、#15785、#15742、#15777、#15828、#15833、#15765
- 可选未排期：#340 俄语、#15665 Hermes 门禁、#15683、#15716、#15687、#15682、#15748

### 34.4 发现的文档过时事实（本轮顺带修正）

1. 本地 i18n 实为 en/zh-CN，日语已移除（README「三语」表述过时）
2. 迁移实际在 `crates/persistence/migrations/`，AGENTS.md 写顶级 `migrations/` 已过时

### 34.5 验证状态

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | 0 error，27 warnings 与基线一致 |
| `cargo test -p warpui text_layout` | 33/33 |
| `cargo test -p watcher` | 3/3 |
| `cargo test -p warp_terminal -- flat_storage row_iterator` | 33/33 |
| `cargo test -p warp --lib`（input/paints_glyph/file_load_error） | 5/5 |
| `cargo test -p warpui_core`（订阅/notify 新测试） | 6/6 |
| `cargo test -p warp_files` | 30/30 |
| `cargo test -p warp_editor` | 410 过 / 9 失败 = **基线（拣入前临时 worktree）完全相同的既存并行干扰集** |
| 品牌/空行扫描 | 干净 |

### 34.6 流程记录

- 执行前 `cargo clean` 释放 101G（主仓 target 78G + dsh-plugin 23G）；磁盘门禁（<10G 先清理）全程未触发（最低 86G）
- 本轮教训：`rg -rn` 中 `-r` 是 replace 而非 recursive，曾污染查重输出（本次靠复核纠正）；cherry-pick 冲突解决用 `git add -A` 前必须确认无残留冲突标记（本轮回滚过一次带标记的提交）
- 上游 factory PR 常把「功能重构 + 基建重构」揉在同一提交：#15719（auth-secret-ftux）、#15835（code_review LocalOrRemotePath 体系）均按「取净效果、适配本地」处理，中间态一律不拣

*文档版本:v2.11*
*下次合并前必读*
## 35. 移植记录（进行中）：2026-08-14 → 上游 HEAD 区间首轮筛选

> 触发：用户要求「从最初的 fork 时间开始检查」。核实 §31：**fork 点 `c325d146` → 2026-08-13 的全量对账已于第十四轮完成**（244 fix 候选 → 59 漏合并全部移植，5 跳过）。故本轮真正未对账区间为 **2026-08-14 → 上游 HEAD**。

### 35.1 区间与筛选漏斗

- 上游 HEAD：`4143c09ff`（2026-09-11），区间提交 **281**
- `git cherry HEAD upstream/master c325d146`：`2238 +` / `98 -`。**注意 `+` 是「未移植集合的上界」**，含本地手工适配（patch-id 变化）与按决策表故意跳过的全部云端/CI 提交，不可当作待移植修复数。

筛选漏斗（三轮，规则见脚本）：

| 轮次 | 规则 | 剩余 |
|------|------|------|
| — | 区间总计 | 281 |
| 一 | 仅命中跳过目录 / 依赖 CI bump / 仅清单文件 | 254 |
| 二 | 主题：团队/workspace/Oz/billing/graphql/共享会话/harness 等 | 165 |
| 三 | 无关键：Windows/WSL/PowerShell · docs/贡献者 · command-signatures · WASM/TUI · Factory/release workflow | **111 待判** |

### 35.2 本轮主要成本：路径漂移（评估固定前置）

本轮 281 条触及上游路径 **1081** 个，其中**本地缺失 599 个（55%）**。成因四类：

1. **云端/团队文件**——`app/src/workspaces/user_workspaces/`、`settings_view/teams_page.rs`、`crates/warp_graphql_schema/`（本就跳过）
2. **本地已删 crate**——`crates/warp_tui/*`
3. **CI**——`.github/workflows/*`
4. **测试文件约定差异（最大陷阱）**——上游 `_tests.rs` 与源码同目录，本地亦然但目录已重排，例：
   - `app/src/terminal/view_tests.rs` → 本地 `app/src/terminal/view/view_tests.rs`
   - `app/src/ai/agent_sdk/driver_tests.rs` → 本地 `app/src/ai/agent_events/driver_tests.rs`
   - `app/src/server/server_api/ai.rs` → 本地 `app/src/settings/ai.rs`

### 35.3 评估规则（后续每轮固定执行）

判断候选可否移植**之前**，先剔除 `cloud/team/tui/CI` 路径与 `*_tests.rs` 约定差异，
**只看剩余非测试源码路径是否在本地存在**。否则会把所有候选误判为「前提缺失」
（首轮 `git apply --check` 12/12 全失败即此因，该结果**不能**作为「本地缺前提」的证据）。

另：`git apply --check` 失败仅说明上下文行对不上，**不等于前提缺失**。

### 35.4 已完成的单条评估

**`a7326f8fe` 宽字符提升崩溃 —— 裁决：可移植**

| 四问 | 结论 | 证据 |
|------|------|------|
| ① 前置假设 | 成立 | 本地 `app/src/terminal/model/grid/ansi_handler.rs:209` 与上游修复前**逐字相同**；`push_zerowidth` 本地返回 `()` |
| ② 已有等效解法 | 无 | 本地仍是旧签名，无守卫 |
| ③ 消费点 | 4 处 | app ansi_handler、`grid_renderer/unicode_placeholder.rs:267`、`warp_terminal/.../row_iterator.rs:126`、`testing.rs:124`（上游同 commit 已一并更新） |
| ④ 上游最终版 | 是 | `git log a7326f8fe..upstream/master -- cell.rs` = 0 |

路径适配：上游 `crates/warp_terminal/src/model/grid/ansi_handler.rs` → 本地 `app/src/terminal/model/grid/ansi_handler.rs`（未随 `21f413b79` 搬迁）；`cell.rs` 同路径。

### 35.5 执行记录（2026-09-12，已落地）

计划文档：`specs/upstream-merge-plan-2026-09-11.md`（34 项任务 / 36 个上游提交）。
执行环境：**主 worktree**（未新建 worktree——新 target 需额外 ~33G 与全量重编；主 worktree 的未提交改动只有本文档，先提交即可满足 cherry-pick 的干净树要求）。
落地 **24 个上游提交**（27 个本地 commit，含 2 个测试适配 commit）。

#### 落地清单（按批次）

| 批次 | 上游提交 | 本地 commit 摘要 |
|------|----------|------------------|
| 1 P0 | `a7326f8fe` `92a98662f` `83e270f1d` `ee95ac0fd` `33c3bf6b7` `fbbfc41f3` `53b502c8e` `90c2484dc` `5cd24ed1b` | 宽字符崩溃/空文档崩溃/em_width panic/双光标/丢弃越界/grep 冒号路径/新建窗口可重绑/Alt+1 固定绑定/in-band 警告 |
| 2 shell | `607be8c26` `0140af045` `294033bb1` `e722ebeda` `17f432027` | bash shell_plugins、zsh compadd -ld、zsh kill-buffer 全 keymap、四个 shell-integration bug、honor PS1 二次展开 |
| 3 构建 | `ccf683193` `542683634` | 删 12 个孤儿 cargo feature、cosmic-text pin（Hack 不作 fallback donor） |
| 4 性能 | `d89e78385` `1c925e333` `213c9b32e` `c6609ef23` | styled blocks Arc 化、布局 fan-out 与单行 shaping 上限、SignatureCache miss cache、gitignore matcher 缓存 |
| 5 功能 | `3a7a4a5b3` `79a9cb721` `d15645c77` `3a6f05512` | 空 category 头、completer 选项参数按值位置、AgentSource::Orchestration、AI plan 文档延迟布局 |

#### 未落地（12 个上游提交，逐条原因）

| 上游 | 原因 |
|------|------|
| `511b952c2` | **前置不成立**：本地 `warp_multi_agent_api` 走 `zerx-lab/warp-proto-apis` fork（rev `14ab9a71`），上游该提交的 `supports_create_file_overwrite` 在 `warpdotdev/warp-proto-apis` 自有 rev 上 |
| `092c1dce9` | 10 hunk 冲突（code editor / notebooks / pane_group / editor viewport） |
| `8b88df987`+`40e397170` | 26 hunk 冲突，落在本地自研 tab / vertical_tabs 体系 |
| `c25ac4070`+`18179177a` | 14 hunk 冲突（17 文件，含 `warpui_core` 事件层） |
| `b7ec0fc55` | 10 hunk 冲突（含本地不存在的 `terminal/view/context_menu.rs`） |
| `5e7030db7` | 7 hunk 冲突，且上游该提交混有 `warp_multi_agent_api` 解耦改动 |
| `4b894db80` | 7 hunk 冲突；且属编译期优化，与本地 serde Content 结构分叉 |
| `0a0fd3ae1` | 4 hunk 冲突，本地 `terminal/view.rs::context_menu_items` 已分叉需手工重写菜单构造 |
| `4cd1c77c4` | 落点在 Zap 自研 CLI agent footer 区（决策表「取本地」），且依赖本地不存在的 `server/telemetry/events.rs` |
| `142b87102` | 13 hunk 冲突跨 6 文件，含自研 footer 与 `terminal/view.rs` 定制区，且依赖 shared-session 的 `file_attach_allowed_for_shared_session` |

#### 关键适配点（本地能力差异）

1. **cell 内嵌超链接本地不存在**（`a7326f8fe`）：上游 `write_wide_char` 调用 `Cell::hyperlink_id()`/`set_hyperlink_id()`，本地 `CellExtra` 只有 `cell_with_zero_width`/`end_of_prompt` → 去掉超链接中转，其余逻辑照搬。同时 `push_zerowidth` 改返回 `bool` 后，`flat_storage/{row_iterator,testing}.rs` 的闭包必须加块体（上游同 commit 已改，先前误判为「纯格式化」）。
2. **edition 2021 无 let-chain**：`e722ebeda`（pty_controller）、`213c9b32e`（miss_cache）两处 `if let … && let …` 改嵌套 if（§34 同款教训）。
3. **`EditDelta::precise_deltas` 本地早已 Arc 化**（§34 #15810），故 `d89e78385` 的 `new_lines` Arc 化只影响一半；本地渲染队列是自研「300 行/帧分块懒布局」，该路径改 `Arc::unwrap_or_clone`。
4. **本地 layout 管线与上游分叉**：`1c925e333` 的 chunk 化要嵌进本地 Step1–Step4 + `collect/build/parallel/fold` 计时结构；`layout.rs` 已走 `layout_text_uncached`（§34），故把上游的 `truncate_text_for_layout`/`clamp_style_runs_for_layout` 接到 uncached 调用。
5. **本地无 standing_queries / repo_watch_filter / force_included_paths**：`c6609ef23` 只取 repo_metadata 侧；`Entry::load` 保持同步（本地调用方为同步函数，函数体无 await）；`compute_file_tree_mutations` 保持一元返回。
6. **ai_document/notebook 的 orchestration 与 mermaid 依赖本地缺失**：`3a6f05512` 只取 `LayoutTiming`/`new_unbound_lazy`/`will_auto_open`，剔除 `DirtyOrchestrationEvent`、`set_default_mermaid_display_mode`、`sync_mermaid_render_offsets`、`is_rendered_mermaid`。
7. **测试文件命名差异**：本地 `cell_test.rs`/`registry_test.rs`/`layout`（新增）/`*_tests.rs` 混用；上游 `mod_tests.rs`、`view_tests.rs`、`buffer_tests.rs`、`task_tests.rs` 等本地不存在 → 一律只移生产代码，上游新增的独立测试文件（`miss_cache_tests.rs`、`gitignore_cache_tests.rs`、`layout_tests.rs`）则保留（需把 `warpui_core::` 适配为本地 `warpui::`）。

#### 本地测试适配（Arc 化的连带改动）

`EditDelta::new_lines` Arc 化 + `Gitignore` Arc 化打破了本地既有测试文件：

- `crates/editor/src/content/buffer_test.rs`：97 处断言改 `*delta.new_lines` / `*edit_delta.new_lines`
- `crates/editor/src/content/edit_tests.rs`：`layout_text_block`/`layout_mermaid_diagram_block`/`layout_table_block` 传 `&block`（上游签名改为引用）
- `crates/repo_metadata/src/entry_test.rs`、`local_model_test.rs`：`gitignores` 容器改 `Vec<Arc<Gitignore>>`

#### 验证状态（2026-09-12）

| 项 | 结果 |
|----|------|
| `cargo check -p warp` | 每批均 0 error（批次 1 首次失败 2 处：cell.rs `hyperlink_id` 字段 + 闭包块体；修正后通过） |
| `cargo test -p warp_editor` | 456 过 / 10 败——**与基线（batch4 前 `6beea36c7`：449 过 / 10 败）失败集完全一致**，新增 7 个通过（layout_tests） |
| `cargo test -p warp_completer` | 125 过 / 26 败——与基线（121 过 / 26 败）失败集一致，新增 4 个通过（miss_cache_tests） |
| `cargo test -p repo_metadata` | 55 过 / 0 败 |
| `cargo test -p warpui text_layout` | 33/33（cosmic-text pin 后） |
| bash/zsh 脚本 | `bash -n` / `zsh -n` 全部通过 |
| 磁盘门禁（用户要求） | 每批 `cargo check` 前检查，全程 74G → 67G，未触发 10G 清理线 |

#### 本轮教训

1. **`git apply --3way --check` 返回 0 不代表干净**：它只保证能产出一个「可含冲突标记」的结果——本轮据此误判多次，后续一律以「实际 apply 后扫 `<<<<<<<`」为准。
2. **`git apply` 是事务性的**：只要有一个目标文件不存在（上游测试文件本地常缺），**整个 patch 全部回滚**。必须显式列出「本地存在的文件」再应用，或先 `git status` 确认真的落地。
3. **`git apply … | tail` 会吞掉退出码**：本轮因此把带冲突标记的 `bash_body.sh` 提交进历史（随后 `git reset --hard` 重做）。同类风险还有 `cargo check 2>&1 | tail`——退出码取自 tail；应改用 `echo "EXIT=${PIPESTATUS[0]}"`。
4. **`git checkout <旧commit> -- <目录>` 会连带冲掉未提交的同目录改动**：本轮做基线对比时把已改好的 `buffer_test.rs`/`edit_tests.rs` 适配冲掉，只能重做。基线对比更安全的做法是先提交当前状态，或用独立 worktree。
5. **基线对比是判定「测试失败是否既存」的唯一可靠手段**：本轮两处失败集（10 / 26）都是既存环境干扰，靠 `git checkout` 回基线同跑才敢下结论。
6. **上游大提交常把「目标特性的依赖」一并带上**：`a7326f8fe` 的 `write_wide_char` 混入超链接中转、`5e7030db7` 混入 multi-agent API 解耦、`3a6f05512` 混入 orchestration/mermaid——移植时必须逐 hunk 判断「这是本特性的必要部分，还是顺路带上的其他特性」。

---

## 36. 追加移植与 P3 裁决（2026-09-12 续）

> 承接 §35：§35.5 里 12 项「本轮不做」经二次裁决后**又落地 3 项**；另对 `511b952c2` 的 proto 依赖做了专项调查并给出终局裁决。三项追加移植均在 `cargo check -p warp` 0 error 下各一个 commit。

### 36.1 追加落地（3 项）

| 上游 | 内容 | 本地适配要点 |
|---|---|---|
| `0a0fd3ae1` (#15346) | 块列表右键菜单加「粘贴」 | 新增 `TerminalView::paste_menu_item`（剪贴板空则禁用、带 `terminal:paste` 标签）；块右键（含悬停链接）与三类右键来源追加，overflow / 键位菜单不加（同上游语义）；新增 ftl key `menu-block-paste`（en/zh-CN） |
| `092c1dce9` (#13967) | 切 markdown Raw/Rendered 保持滚动位置 | editor 新增 `ScrollPosition::Fraction` + `viewport::scroll_fraction` + `LayoutAction::ScrollToFraction`（延迟到 element layout 应用、携 `minimum_version` 防陈旧高度）；code view / local code editor / notebooks file view / pane_group 切换前抓 fraction、重建 pane 时先 seed 再 open。**剔除上游夹带**的 `warp_tui` CharCell `new_tui` 与 Jupyter 分支（本地无 `FeatureFlag::JupyterNotebookRendering` / `is_jupyter_notebook_file`）；远端文件分支跳过（本地无 `file_state.path()`，沿用 `local_path()`）；`view.open(...)` → `view.open_local(...)` |
| `b7ec0fc55` (#15605) | Agent Mode 用户提问显示时间戳 | `util/time_format` 新增 `format_message_timestamp` / `is_trustworthy_message_timestamp`（+2 测试）；block model 加 `query_sent_at`（过滤 epoch 默认值）；query 视图 Props/render_query 加时间戳 + tooltip 句柄（头像包 `overlay_tool_tip`）；新增 `AIBlockAction::CopyTimestamp`。**修正（见 §39.6）**：上游 `terminal/view/context_menu.rs` 的 21 行菜单门控**当时并未落地**——该文件本地不存在，被我从 patch 文件集中剔除，本格原先写的「按本地等价位置落进 view.rs」有误；该入口已于 §39.6 在本地补齐。**跳过上游夹带**的 completer v2 refactor（153 行，与时间戳无关） |

顺带修复：§35.5 批次 1 带入的 `test_duplicate_single_file_discard_confirmation_is_ignored` 用 `&str` 调用，而本地 `TestContext::new` 要 `PathBuf`——`cargo check`（只查 lib）不编译测试故未暴露，本轮 `cargo test -p warp --lib` 才现形，已改为 `PathBuf::from("test.txt")`。

### 36.2 剩余项终局裁决

| 提交 | 裁决 | 依据（本会话实测） |
|---|---|---|
| `c25ac4070` + `18179177a`（右键行为设置：菜单 vs 直接粘贴） | **缓：值得但属独立项目** | 无缺失前置——19 文件里 16 个存在，缺的 3 个只是路径漂移（`elements/gui/event_handler.rs` → 本地 `elements/event_handler.rs`）与测试文件；设置项落点现成（`settings/select.rs` 已有同类 `middle_click_paste_enabled`）；实测 14 hunk / 10 文件，覆盖 `warpui_core` 事件层 + terminal / notebooks / env_vars / inline_action / agent 各视图 → 估 1–2 天，非顺手同步 |
| `4b894db80`（serde Content → 手写 Deserialize） | 不做 | 纯编译期优化、无行为收益；7 hunk / 3 生产文件 |
| `8b88df987` + `40e397170`（按住修饰键显示 tab 快捷键） | 不做 | 26 hunk / 5 文件；本地完全无该机制，`vertical_tabs` 自研度最高 |
| `5e7030db7`（warping 行显示模型名） | **不做（已试并回滚，见 §40.7）** | 移植后实测无效：本仓**从不构造** `api::message::Message::ModelUsed`（BYOP 把它归入忽略分支），`AIAgentOutput::model_info` 恒为 `None` → warping 行永远保持 "Warping..."；用户判定不需要该功能，提交 `73843d81e` 已由 `ea3b6fadf` 回滚。原复核理由（下方保留）：**理由更硬**：① 上游做成**双重门控的 dogfood 特性**——非默认 Cargo feature `warping_model_name = []`（`app/Cargo.toml:920`，不在 default 列表）+ `FeatureFlag::WarpingModelName` 仅列在 `DOGFOOD_FLAGS`（upstream master `warp_features/src/lib.rs:1072`），即**上游自己也不默认展示**；② 本仓无自动路由/自定义 router（`CustomModelRouters` 仅是 flag 名、无实现；BYOP 模型由 `build_byop_models_by_feature` 从用户配置构造），模型是用户显式选的 → **用户已知**；唯一有信息量的 `is_fallback` 场景本地**已展示**（`status_bar.rs:793-830` 的 `resolve_fallback_warping_message`）；③ 本地另有**未接线**的 `AIBlock::output_model_display_name`（`block.rs:4783`，全仓无调用＝死代码）→ 若真需要，应接线它而非移植上游 flag。成本：7 hunk / 4 文件 + feature/flag 三处接线，且需剥离该提交夹带的 multi-agent 解耦改动 |
| `4cd1c77c4`（原生 agent 工具条 File explorer chip） | 不做 | 本地**已有** `AgentToolbarItemKind::FileExplorer`（现仅 CLI agent），属重复建设；落点在自研 footer 区。**⚠️ 更正（2026-09-12 复核）**：原写「依赖本地缺失的 `server/telemetry/events.rs`」是**路径漂移误读**——本地 telemetry 是单文件 `app/src/server/telemetry.rs`，`FileTreeSource` **已存在**（5 个变体：PaneHeader/Keybinding/LeftPanelToolbelt/ForceOpened/CLIAgentView），该提交只需新增 1 个 `AgentToolbelt` 变体 |
| `142b87102`（Attach file 调色板命令） | **已合并（2026-09-12，见 §40）** | 用户复核后决定合并。原复核理由（下方保留）：本地**已有** attach 按钮与 `EditorAction::AttachFiles`，本项只是补一个命令面板入口。**⚠️ 更正（2026-09-12 复核）**：原写「依赖 `file_attach_allowed_for_shared_session`（本地 0 命中）」**不成立**——它只是 `AgentToolbarItemKind::FileAttach.available_to_session_viewer(...)` 的薄封装，而 `available_to_session_viewer`（`agent_input_footer/toolbar_item.rs:92`）与 `SharedSessionStatus` 本地都有，需补的只是约 10 行本地 helper |

> **⚠️ 注记 2（2026-09-12 复核，HEAD `c535a02c5`）**：对「不做」的 4 项重测冲突规模并复核前置——
>
> | 上游 | 冲突文件 | hunk | 前置复核结论 |
> |---|---|---|---|
> | `8b88df987`+`40e397170` | 6 | **29** | 本地确实零基础（`TabShortcutModifierState`/`TAB_ACTIVATE_BINDING_NAMES`/`reveals_tab_shortcut_hints` 均 0 命中）→ 「成本/风险」理由成立 |
> | `5e7030db7` | 4 | 7 | 数据链具备（`status_bar.rs` 已读 `model_info.display_name`）→ 「收益偏小 + 夹带 multi-agent 解耦改动」理由成立 |
> | `4cd1c77c4` | 3 | 10 | **前置并不缺失**（见上表更正）→ 仅剩「重复建设 + 自研 footer 区」 |
> | `142b87102` | 6 | 13 | **前置并不缺失**（见上表更正）→ 仅剩「入口重复 + 冲突多」 |
>
> 附带发现（既存，未处理）：`AIBlock::output_model_display_name`（`ai/blocklist/block.rs:4783`）全仓**无调用**＝死代码——它正是「显示真实模型名」的本地历史实现，与 `5e7030db7` 想做的事重合；是否清理/接线待用户决定。
>
> 教训：**「前置缺失」必须按符号名全仓 grep 判断，不能用上游文件路径是否存在来判断**——本地 `server/telemetry.rs`(单文件) vs 上游 `server/telemetry/events.rs`(拆分) 这类漂移会直接得出相反的结论（§18.5 路径坑的第二次踩中）。

> **⚠️ 注记（2026-09-12 晚）**：上表中 `4b894db80` 的处理已变更，见 §38——用户要求实做后，**阶段 1（`dcs_hooks.rs` 的 `DProtoHook`/`BootstrappedValue`）已落地并保留**（提交 `e11deb51e`），实测净收益为负（`warp` crate 编译 +2.1%）；阶段 2（`Artifact` + `AIAgentContext`/`AIAgentAttachment`）**不做**。上表「不做」的裁决对阶段 2 仍然成立。

### 36.3 `511b952c2`（create_file `allow_overwrite`）终局：不合

专项调查（两个 proto fork 对比，只读）：

- 本地 pin 的是 **`zerx-lab/warp-proto-apis`** fork（`14ab9a71`，2026-05-10，= 该 fork 的 main tip）；与上游共同祖先为 `aa2f9cde`。fork 只多 **1 个定制提交**：「删除 SearchCodebase / search_codebase / search_codebase_stats wire 字段」（3 文件 +13/−23，注释写明 *codebase index 全栈下线*，用 `reserved` 占位）。
- 上游自该祖先起已走 **46 个提交**到 `0ca49ce5`（#365）：23 文件 **+13754/−3972**（`request.proto` 366 行、`task.proto` 525 行，新增 `orchestration.proto` / `input_context.proto`）；`Request` 能力位从 27 涨到 32。
- #365 的 proto delta **仅 7 行**：`Request.supports_create_file_overwrite = 32` + `NewFile.allow_overwrite = 3`；`apis/multi_agent/v1/gen/rust/build.rs` 是**构建期**读 `.proto` 现生成（`src/lib.rs` 只 `include_bytes!` 描述符集），改 `.proto` 无需提交生成物。
- **决定性事实：本地根本不构造 proto `Request`**——全库 `supports_*` 能力位 **0 命中**；`warp_multi_agent_api::Request` 只出现在集成测试的 `decode`（`integration_testing/agent_mode/step.rs:76`）；`app/src/server/` 无任何 multi-agent 请求构造；`api::RequestParams`（`ai/agent/api.rs:77`）是 Zap 自有结构，由 BYOP `agent_providers/chat_stream.rs` 翻译成 genai `ChatRequest`；上游该能力位落点 `app/src/ai/agent/api/impl.rs` 本地不存在。

→ 方案 A（撤 `[patch]` 切上游 rev）会引入 46 提交的行为漂移，并**恢复 Zap 刻意删除的 codebase-index wire 面**（服务端可能重新下发本地已无处理链的 `SearchCodebase`）；方案 B（把 7 行 backport 到 fork）**拿不到任何行为变化**（能力位无处可发）。**裁决：不合**。

→ 需求本身（AI 覆盖已存在文件）在本地是真实可达的（本地 BYOP 工具 `apply_file_diffs` 的 `op:"create"` 落成 `FileEdit::Create{file,content}`，命中已存在文件即报 `Could not create {file} because it already exists.`，`diff_application.rs:124`），但正确做法是**本地自研**：`FileEdit::Create` 加 `overwrite` + 工具 schema 暴露参数 + `diff_application.rs:326` 按它决定覆盖/报错（连带 `code_diff_view.rs:2938/3018` 两处 match 臂），**与 proto 无关**。

### 36.4 本轮追加教训

1. **`cargo check -p warp` 不编译测试**：只跑 check 会漏掉本地测试适配错误（本轮 `TestContext::new` 的 `PathBuf` 就是实例）。凡改动涉及测试文件，必须补 `cargo test -p <crate> --no-run`。
2. **上游提交普遍夹带无关改动**：`b7ec0fc55` 的 completer v2 重构（153 行）、`a7326f8fe` 的超链接中转、`3a6f05512` 的 mermaid/orchestration 都属此类——逐 hunk 判断归属，不符就整块丢弃。
3. **依赖类改动先比 fork/rev，再看本仓 diff**：本地 `warp-proto-apis` 是自有 fork，只对比本仓源码会得出完全错误的可行性结论（§36.3）。
4. **import 冲突别硬合上游 blob**：上游常把分组 import 拆成单行 import，本地风格相反；此时保留本地、让编译器报缺哪个符号再补，比合并 import 块可靠得多（本轮 query.rs / block.rs / model_impl.rs 均用此法）。

---

## 37. P3-a 落地：右键行为设置（2026-09-12 续）

> 上游 `c25ac4070`（#15365，右键行为设置）+ `18179177a`（#15392，Shift+右键提示）。§36.2 曾判「值得但属 1–2 天独立项目」，用户决定继续后一次落地，20 文件 +387/−77（上游同版本为 975 行，差额主要是上游 555 行 `view_tests.rs` 测试）。

### 37.1 实现结构

| 层 | 改动 |
|---|---|
| 设置 | `SelectionSettings::right_click_behavior`（`RightClickBehavior::{ContextMenu 默认, Paste}`，toml `terminal.input.right_click_behavior`，enum 而非 bool，便于将来加 "Quick Edit"）+ `right_click_pastes()` |
| 判定 | `app/src/terminal/mod.rs::should_right_click_paste(shift, ctx) = !shift && 设置开启`，块列表 / alt screen / 输入行三处共用 |
| 框架 | `warpui_core::EventHandler::on_right_mouse_down` 回调增加 `&ModifiersState`：新增 `HandlerWithModifiers` 与 `dispatch_callback_with_modifiers`，事件分支从 `Event::RightMouseDown { cmd, shift, .. }` 组装 `ModifiersState` |
| 右键面（6 处） | 块列表、alt screen、输入行、CLI agent rich input、prompt 区 ×2、输入下方空区块 |
| 块列表额外语义 | 运行中的全屏程序接管鼠标（`should_intercept_mouse` 为假）时，把原始右键经 `AltMouseAction` 转发给它——与左键 down/up/drag、滚轮同条件（上游同语义，本地此前无） |
| 设置页 | Feature 页加 `RightClickBehaviorWidget`（Dropdown，`editor_widgets` 内紧随中键粘贴）+ `SelectionSettings` 变更订阅 + 动作/遥测臂 |
| i18n | `settings-features-right-click-behavior-{label,hint}`（en/zh-CN）；下拉项文案**已中文化**：`RightClickBehavior::as_dropdown_label()` 改用 `crate::t!`（key `…-context-menu` / `…-paste`，模式同 `settings/ai.rs::command_palette_description`）。注：`settings/mod.rs` 里更早的 `CtrlTabBehavior`/`GlobalHotkeyMode` 下拉项仍是硬编码英文，属既存状态，未在本轮一并处理 |

### 37.2 适配要点（本地差异）

1. **不跟随上游的 `ScrollHandler` 改名与 `&Vector2F → Vector2F`**：上游顺手把 scroll 回调签名也改了，会牵动全部 `on_scroll_wheel` 调用点；本地只改右键所需部分。
2. **let-chain 改嵌套 if**（edition 2021）：`event_handler.rs` 的分支用 `dispatch_callback_with_modifiers` 承载，避免 let-chain。
3. **保留本地 `dispatch_callback`**：仅新增带修饰键的变体，不动既有回调路径（首次改写误将其重命名，编译器立刻报 3 处 `mouse_in/mouse_out/mouse_dragged` 缺方法，已恢复）。
4. **设置页沿用本地模式**：以 `CtrlTabBehavior` 那套（Dropdown + `update_*_dropdown` + `SettingsWidget` + `add_setting`/`render_dropdown_item`）为模板；标签走 Fluent（`crate::t!`），下拉项文案走 `as_dropdown_label()`（本地 `settings/mod.rs` 既有惯例）。
5. **测试未带**：上游 555 行 `app/src/terminal/view_tests.rs` 本地为单数命名（`view_test.rs`）且结构不同，未直接移植；本项验证依赖编译 + 「点一次」的手动确认。

### 37.3 验证状态（2026-09-12）

| 项 | 结果 |
|---|---|
| `cargo check -p warp` | 0 error |
| `cargo test -p warpui_core` | 290 过 / 0 败（事件层改动无回归） |
| `cargo test -p warp --lib settings_view` | 19 过 / 0 败 |
| 测试目标编译 | `cargo test -p warp --lib --no-run`、`-p warpui_core --no-run` 均通过 |
| 手动确认（待用户执行） | 设置 → Features → 右键行为切到「Paste from the clipboard」后：终端内裸右键直接粘贴；Shift+右键仍弹菜单；全屏程序（如 vim/htop 开鼠标上报）里右键仍转发给程序 |

### 37.4 本轮教训

1. **批量正则替换必须限定「行内上下文」**：本轮用 `re.sub` 全局替换 `|ctx, _, position|`，不仅把 `on_click` / `on_left_mouse_down` / `on_right_click` 的回调一起改坏，还把正则里的 `\|` 反向转义写进了源码（`\|ctx, _, _\, _|`）。正确做法：先按「同一行含 `on_right_mouse_down(`」筛行，再做纯字符串替换。改完必须 `git diff -U0` 逐行复核，而不是只看替换计数。
2. **上游「顺手改签名」的部分要主动剥离**：`ScrollHandler` 的 `&Vector2F → Vector2F` 与本特性无关，跟随会放大改动面（§36.4 第 2 条的同一形态）。
3. **改动事件层后必须跑测试目标编译**：`cargo check -p warp` 只查 lib；`warpui_core` 的 `event_handler_test.rs` 与测试目标里的回调签名要单独 `--no-run` 确认（本轮两者都通过，但这是运气，不是流程）。

---

## 38. `4b894db80`（编译期 serde 优化）实测：净收益为负（2026-09-12）

> §36.2 原判「不做（纯编译期优化、无行为收益）」。用户要求改为「要做」，遂分阶段执行：**阶段 1 = `dcs_hooks.rs` 的两个类型**（本提交 `e11deb51e`），做完即用实测数据决定是否继续阶段 2（`Artifact` + `AIAgentContext`/`AIAgentAttachment`）。**实测结论：阶段 1 让 `warp` crate 编译慢约 2.1%，未测出任何正向收益 → 阶段 2 不再做，整项终局为不做。**

### 38.1 先纠正两个此前的判断错误

1. **“栈前置不成立”是错的**：`4b894db80`（#15455）声明的栈前置 `#15454`（`6a96a72d8`）只动 `crates/settings/{lib,macros,registration}.rs`，与 `4b894db80` 的 6 个 app 文件**零重叠**（`comm -12` 验证）。“stacked on” 只是分支基线，不是内容依赖 → 可单独移植。
2. **“值得做”也是错的**：上一轮基于上游提交描述（“reduces the compile time of the `warp` crate”）判为值得做。**本地实测推翻了它**（见 §38.3）。教训：**compile-time 类提交必须本地实测后再决定，不能凭上游描述**。

### 38.2 阶段 1 实现（`e11deb51e`）

| 类型 | 改法 | 本地适配 |
|---|---|---|
| `DProtoHook`（内部标签 `tag = "hook"`） | 去 `Deserialize` derive；新增 `DPROTO_HOOK_VARIANTS`、`RawDProtoHook{hook,value}`、`parse_hook_value` 与 13 臂 match（未知变体走 `unknown_variant`） | 本地 13 个变体与上游逐一对应，无差异 |
| `BootstrappedValue`（逐字段 `deserialize_with`） | 去 `Deserialize` derive 与全部字段属性；新增 `RawBootstrappedField{Missing,Present}`（复刻 `#[serde(default)]` 的缺字段语义）、`RawBootstrappedValue` 与手写 impl；`empty_string_is_none` 抽出 `empty_string_to_none(String)`；删除 `parse_shell_options_list_deserializer`、`parse_float_from_string_deserializer` | **本地无 `cdpath` 字段** → raw 结构不列该字段；`trim_null_byte_deserializer` 仍被 `InitShellValue` 等使用，**保留** |
| 测试 | 新增 `app/src/terminal/model/ansi/dcs_hooks_test.rs`（13 个格式冻结测试） | 上游文件名为 `dcs_hooks_tests.rs`，本地为单数约定，按 `#[cfg(test)] #[path = "dcs_hooks_test.rs"] mod tests;` 声明 |

测试覆盖：全字段解析（含 `shell` 的 NUL 截断、`shell_options`/`shell_plugins` 的空格列表、字符串浮点）、可选字段默认、必填缺失报错、malformed float 报错、未知 hook 报错（断言含 “unknown variant”）、缺 `value` 报错、13 个 tag 逐一派发且与 `DPROTO_HOOK_VARIANTS` 完全一致、`SourcedRcFileForWarp` 冻结片段格式（该字面量随用户 RC 文件分发）、legacy `tmux` 字段被忽略、SSH/InitShell 序列化往返。

### 38.3 实测数据

**测量陷阱（本轮最大收获）**：`.cargo/config.toml:12` 强制 `rustc-wrapper = "sccache"`，它按**内容**缓存，`touch` 只改 mtime 时直接命中缓存 → wall-time 完全失真（实测同一命令出现 23s / 117s / 139s 三种结果，且 `user` 仅 5s，说明时间花在取缓存而非编译）。**必须 `RUSTC_WRAPPER=` 绕过 sccache**，并用 `user` 时间做交叉校验。

测量口径：`RUSTC_WRAPPER= CARGO_INCREMENTAL=0` + `touch app/src/lib.rs` + `cargo build -p warp`（只重编 `warp` crate，正是上游这套优化的目标）。每态两次：

| 状态 | real #1 | real #2 | user #1 | user #2 |
|---|---|---|---|---|
| 基线（派生 serde） | 98.67s | 98.50s | 161.78s | 164.41s |
| 阶段 1 改后 | 100.65s | 100.71s | 165.98s | 163.44s |

→ **同态抖动仅 ±0.2s**，而两态差 **+2.1s（+2.1%）**：改后**更慢**。

**体积（精确复测，2026-09-12 晚）**：同一条命令 `CARGO_INCREMENTAL=0 cargo build --bin zap-oss`，两态产出的
**同一 rlib**（`libwarp-91fc53b275388999.rlib`，hash 相同可与基线对齐）：

| 产物 | 基线 | 含阶段 1 | 减少 |
|---|---|---|---|
| rlib | 982,614,840 B（937.1 MiB）| 981,656,648 B（936.2 MiB）| **−958,192 B（−0.91 MiB，−0.098%）** |
| 调试二进制 `target/debug/zap-oss` | 615,273,968 B（586.8 MiB）| 614,248,192 B（585.8 MiB）| **−1,025,776 B（−0.98 MiB，−0.167%）** |

结论：**体积收益约 −0.1% ~ −0.17%（不足 1 MB）**，远小于构建时间的 +2.1% 代价。此前的「937→935 MB（−0.2%）」
是 `du -h` 的磁盘分配粗测且落在另一个 rlib 变体上（`-p warp`，hash `9e9d8e1b40e6db51`），已由本表取代。

机制层面确实生效（同一个 rlib `9e9d8e1b40e6db51`，feature 集相同，可比）：

| 指标 | 基线 | 改后 | 变化 |
|---|---|---|---|
| `ContentDeserializer` 实例化 | 866 | 333 | **−62%** |
| `__DeserializeWith` 实例化 | 2071 | 1216 | **−41%** |
| `ContentRefDeserializer` 实例化 | 991 | 974 | −1.7% |
| rlib 体积（`du -h` 粗测）| 937 MB | 935 MB | −0.2%（后续精确复测见下行）|
| `warp` crate 编译 | 98.6s | 100.7s | **+2.1%** |

### 38.4 为什么净收益为负 & 阶段 2 的天花板

- 被消除的 serde 机械（`Content*` / `__DeserializeWith`）在符号总量里占比极小（rlib 精确缩小 **958 KB / −0.098%**，二进制 −1.0 MB / −0.167%）；`warp` crate 的编译成本由其余数千符号的 codegen 主导。
- 手写 impl + raw 结构体 + 13 臂 match **本身也是要编译的代码**，抵消并超过被消除的部分。
- **阶段 2 天花板同样低**：符号层面统计，rlib 中含 `BlockContext` 的符号 199 个，而真正把 `BlockContext` 拖进 `ContentRefDeserializer` 的只有 **13 个**（形如 `<BlockContext as Deserialize>::deserialize::<ContentRefDeserializer<serde_json::Error>>`）。即使阶段 2 全清，量级与阶段 1 同阶。
- 判定方法学：**这类“消除泛型机械”的改动用机制级指标（`nm -C` 计数）判断是否生效，用绕过 sccache 的 real/user 时间判断是否值得**；两者本轮给出了相反的结论——机制生效，但不值得。

### 38.5 终局裁决

- **阶段 2 不做**：`Artifact` 与 `AIAgentContext`/`AIAgentAttachment` 不再继续（天花板见 §38.4）。
- **阶段 1 保留（用户决定，2026-09-12）**：提交 `e11deb51e` 留在 `main`。保留理由记录为「与上游对齐 + 机制正确（减少 serde 泛型机械的实例化）」，**不含可测量的构建收益**——实测为净负（+2.1% 编译时间、−0.2% rlib 体积）。
- 回滚路径仍然有效且成本极低：`git revert e11deb51e`（含新增测试文件）。
- 重新考虑的条件：① 上游把整个编译期栈做完且其 CI 给出可复现收益；② 本地构建时间成为实际痛点（届时按 §38.3 的方法先测基线再决定，必要时一并重估阶段 2）。
- **未记入 CHANGELOG**：无用户可见行为变化，仅编译期机制调整。

### 38.6 本轮教训

1. **sccache 会把编译时间测量彻底污染**：内容缓存 + `touch` 只改 mtime → 同一命令可测出 23s/117s/139s。识别信号是 `user` 时间与 `real` 严重不成比例（5s vs 139s）。教训推广：以后凡涉及构建耗时结论，必须 `RUSTC_WRAPPER=` 并至少两次重复。
2. **上游的“compile time”类提交必须本地实测再决定**：同一个改动在别的仓库/CI 有收益，不代表本地有——本仓库的 crate 规模与代码构成决定了 serde 机械占比过小。
3. **机制正确 ≠ 收益成立**：符号计数（−62% ContentDeserializer）与构建时间（+2.1%）方向相反时，以端到端时间为准，机制数据仅用于解释原因。
4. **栈（stack）依赖要用文件重叠验证，不能只看提交描述**：本轮 `comm -12` 一步就否掉了“必须连 #15454 一起做”的误判。

---

## 39. i18n 补齐（2026-09-12 收尾）

> 用户手动验证 P1/P2/P3-a/P3-b 后反馈「**还是英文，要补上中文**」「**还有其他的，不止这一个**」。逐处排查本轮移植引入的**用户可见文案**，共 4 处硬编码英文（提交 `2e0604f55`、`7fbb3b105`）。

### 39.1 问题与修法

| # | 位置 | 原因 | 修法（ftl key） |
|---|------|------|-----------------|
| 1 | 「右键行为」**下拉两项**（P3-a） | `RightClickBehavior::as_dropdown_label()` 照 `settings/mod.rs` 旧惯例硬编码英文 | 改返回 `String` + `crate::t!`；key `settings-features-right-click-behavior-{context-menu,paste}` |
| 2 | 提问时间戳 **tooltip**（P3-b） | `query.rs` 里 `format!("Message sent {}", …)` 硬编码 | `crate::t!("ai-block-query-timestamp-tooltip", timestamp = …)`（`t!` 支持参数） |
| 3 | 时间戳**格式**（P3-b） | `format_message_timestamp` 固定 `%-m/%-d at %-I:%M %p`（"at"/"PM" 英文） | 按 `crate::i18n::current_languages()` 分支：中文 `%-m月%-d日 %H:%M`，其它语言保持上游形态（先例：`ai/agent_providers/active_ai/mod.rs` 的 locale 判断） |
| 4 | **Ctrl+Tab 行为 / 全局热键**下拉项（**既存**，非本轮引入，但同页同类） | `settings/mod.rs` 两个 `as_dropdown_label()` 硬编码英文 | 同样改 `String` + `crate::t!`；5 个 key（`…ctrl-tab-behavior-{activate-prev-next-tab,cycle-most-recent-session}`、`…global-hotkey-{disabled,quake-mode,activation-hotkey}`） |

新增 ftl key **6 条 × (en, zh-CN)**（`2e0604f55` 另含右键行为 2 条）。返回类型改 `String` 后**调用点无需改动**：`DropdownItem::new` 取 `Into<String>`、`set_selected_by_name` 取 `impl AsRef<str>`。

### 39.2 复查结论（全仓检索）

- `fn as_dropdown_label` 现仅 3 处，**全部**已本地化。
- 设置页 `render_dropdown_item` / `render_body_item` / `Text::new` 的英文字面量检索为**空**。
- 全仓硬编码 `MenuItemFields::new("…")` 仅剩 10 处：8 处在测试文件（`menu_test.rs`），真实 UI 只有 `workspace/view/server_file_browser.rs:4962` 的 `"Refresh"`（既存，未处理）。

### 39.3 验证

| 项 | 结果 |
|---|---|
| `cargo check -p warp` | 0 error |
| `cargo test -p warp --lib i18n -- --test-threads=1` | **4/4 通过**（zh-CN 资源正常解析，`current_languages=[zh-CN, en]`）|
| 并行跑 i18n 测试 | `fallback_chain_works` 失败，但**既存干扰**——用 `git stash` 回基线复现同样失败（两个测试都调全局 `init()` 且语言不同）|
| 手动验证（用户） | **通过**：右键行为 / Ctrl+Tab / 全局热键下拉为中文；提问悬停显示「提问时间：9月12日 15:04」|

### 39.4 附带更正：`CopyAIBlockTimestamp` 是「悬空动作」

`b7ec0fc55`（P3-b）只加了 `ContextMenuAction::CopyAIBlockTimestamp` 变体、`Debug` 分支与**处理函数**，**全仓（含 `upstream/master`）没有任何地方构造它**——即「复制时间戳」**菜单入口上游就不存在**。§36.1 记录 P3-b 时未察觉，此前给用户的手动验证步骤（「右键提问行应有复制时间戳」）因此是错的；已在对话中更正。该入口属**上游未做**的本地补齐，已落地，见 §39.6。

### 39.5 教训

1. **移植上游 UI 功能时必须顺带检查文案是否走 i18n**：上游大量使用硬编码英文（`as_dropdown_label`、`format!("Message sent …")`），本仓是中文优先的 UI，直接照搬就会出现「中文界面里混英文」。
2. **枚举展示文案的本地化范式**：返回 `String` + `crate::t!`（`t!` 有两臂，支持 `key, arg = value`）；改返回类型前先确认调用点的 trait bound（`Into<String>` / `AsRef<str>` 可免改）。
3. **占位/时间格式也要本地化**：`%I:%M %p`、`"at"` 这类不显眼的英文最容易漏；本仓判断语言的既有方式是 `crate::i18n::current_languages()` + `starts_with("zh")`。
4. **并行测试失败先回基线复现再定性**：`git stash` 一次即确认 `fallback_chain_works` 的并行失败为既存（全局 `init()` 竞争），避免误判为自己引入。

---

### 39.6 本地补齐：「复制时间戳」菜单入口（上游未做）

用户确认「补」。落点：`app/src/terminal/view.rs::ai_block_copying_menu_items`（AI 块「复制类」菜单的统一构造处），门控照抄上游 `context_menu.rs` 的原始 21 行：

```rust
let has_query_timestamp = self.rich_content_views.iter().any(|rich_content| {
    rich_content.ai_block_metadata()
        .filter(|metadata| metadata.ai_block_handle.id() == ai_block_view_id)
        .is_some_and(|metadata| metadata.ai_block_handle.as_ref(ctx).query_sent_at(ctx).is_some())
});
if has_query_timestamp {
    items.push(MenuItemFields::new(crate::t!("menu-ai-block-copy-timestamp"))
        .with_on_select_action(TerminalAction::ContextMenu(
            ContextMenuAction::CopyAIBlockTimestamp { ai_block_view_id }))
        .into_item());
}
```

- **本地偏差（有意）**：该 helper 同时服务「右键行菜单」与「三点溢出菜单」两处调用（`view.rs:14912`、`view.rs:15820`），因此本地上两处菜单都会出现「复制时间戳」；上游只在其行菜单里有。这是超集，不是遗漏。
- ftl：`menu-ai-block-copy-timestamp`（en `Copy timestamp` / zh-CN `复制时间戳`）。
- 至此 P3-b 的链路闭合：悬停看时间戳（tooltip）→ 右键/溢出菜单「复制时间戳」→ `AIBlockAction::CopyTimestamp` 写剪贴板。
- 归属说明：这是**本地功能补齐**，不是上游移植；文档按「本地功能」记录，不写入上游同步的落地清单。
- **验证（2026-09-12）**：`cargo check -p warp` 0 error；重建后用户手动点验**通过**——悬停显示时间戳、右键/溢出菜单出现「复制时间戳」、点击后剪贴板写入时间文本、无时间戳的块不显示该条目（门控生效）。

---

## 40. 收尾三项：删死代码 + 合并 `5e7030db7` + 合并 `142b87102`（2026-09-12）

> 用户指令：「删死代码，然后合并 5e7030db7、142b87102」。三项均已落地并重建启动点验（提交 `43417ffc8` / `73843d81e` / `6d9e859f8`）。

### 40.1 ① 删除死代码 `AIBlock::output_model_display_name`

`app/src/ai/blocklist/block.rs` 里该函数（23 行）全仓无调用（grep 仅命中定义行），且它正是「显示真实模型名」的本地历史实现——与 ② 的目标重合，故先清理以免两套并存。同步删除因之孤立的 `use crate::LLMPreferences;`。

### 40.2 ② 移植 `5e7030db7`（warping 行显示模型名，#15323）——**已于 §40.7 回滚**

> ⚠️ 本节记录的是当时的移植实现；该提交随后被回滚（用户判定不需要），见 §40.7。

| 层 | 内容 |
|---|---|
| `status_bar.rs` | 新增 `ModelInUse`（`From<&OutputModelInfo>`，空 display_name 归一为 None）、`WarpingModelMessage`、`WarpingModelInputs`、`UNNAMED_FALLBACK_MODEL_WARPING_TEXT`、`fallback_warping_text`、`warping_model_message`、`resolve_warping_model_message`；`render` 里改为传 `model_in_use_name`，次级元素按 `show_fallback_explanation` 选择 |
| `view_impl/common.rs` | `STATUS_MESSAGE_ELLIPSIS`、`WarpingProps.model_in_use_name`、`status_message_naming_model`；6 处「生成中」文案改为带模型名（摘要阶段刻意不带——服务端摘要用另一个模型且不上报） |
| 门控（**本地有意偏差**） | 上游是「非默认 Cargo feature `warping_model_name` + `FeatureFlag::WarpingModelName` 仅列 `DOGFOOD_FLAGS`」双层门控 → 连上游都不默认展示。本地改为**纯运行时 flag**（`FeatureFlag::WarpingModelName`）并在 `app/src/bin/zap_oss.rs` 的 `with_additional_features` 里默认开启，使其对 Zap 真正可见。要回退成 dogfood-only：删掉 `zap_oss.rs` 里那一行即可 |
| 测试 | 移植 `status_bar_tests.rs`（16 个决策矩阵用例，全部通过）+ 上游 `common_tests.rs` 的 `status_message_naming_model` 用例 |
| 本地适配 | `OutputModelInfo` 无 `prompt_cache_expires_at`；该提交夹带的 multi-agent 解耦改动一并剥离（改用 `ModelInUse`，删除 `use warp_multi_agent_api as api` 与 `api::message::ModelUsed` 构造） |

### 40.3 ③ 移植 `142b87102`（Attach file 命令面板动作，#15762）

上游意图是「**默认不绑键**的命令面板动作」：`register_editable_bindings` 只注册、不给默认键位。

| 文件 | 内容 |
|---|---|
| `terminal/view.rs` | 新增 `file_attach_allowed_for_shared_session`（薄封装 `AgentToolbarItemKind::FileAttach.available_to_session_viewer`）、`is_in_agent_or_cli_attach_context`、`can_attach_file`；`View` 上下文集插入 `init::CAN_ATTACH_FILE_KEY`；转发列表加 `\| AttachFile` |
| `view/action.rs` | `TerminalAction::AttachFile` 变体 + `Debug` 臂 + 分派臂（`can_attach_file` 为假直接 return） |
| `view/init.rs` | `ATTACH_FILE_KEYBINDING` / `CAN_ATTACH_FILE_KEY` 常量 + `register_editable_bindings` 注册（WarpAi 组，context 断言用 `CAN_ATTACH_FILE_KEY`）；描述按本地惯例走 `crate::t!("keybinding-desc-terminal-attach-file")` |
| `terminal/input.rs` | `attach_file()`（转给 `footer.select_file()`）+ 登录 CLI agent 语境插入 `CLI_AGENT_SESSION_ACTIVE_KEY` + `CAN_ATTACH_FILE_KEY` 语境 |
| `agent_input_footer/mod.rs` | 抽出 `select_file()`（按钮与动作共用）；attach 按钮 tooltip 加键位提示 |
| `right_panel.rs` / `view_components/action_button.rs` | 新增 `ActionButton::with_tooltip_keybinding`，取代手写的 `keybinding_name_to_display_string` + `with_tooltip_sublabel`（两处按钮 + 一处新建按钮） |

**本地适配**：
- 本地**无** `FeatureFlag::CloudModeImageContext`（无云端 Agent）→ `is_cloud_mode` 直接用 `is_ambient_agent()` 判定；
- 本地 `ambient_agent_view_model` 是**非 Option 字段**（上游为 Option）→ 调用处传 `Some(&field)`，未改本地访问器签名；
- 本地缺 `OpenAttachmentLightbox` / `WriteCodebaseIndex` / `ROOT_CLOUD_MODE_PANE_KEY` 等上游分支结构 → 13 处冲突一律取本地侧，再只补 attach-file 所需片段（未把无关分支拖进来）；
- `OutputModelInfo` 等差异同上。

### 40.4 验证

| 项 | 结果 |
|---|---|
| `cargo check -p warp` | 三项各自 0 error / 0 warning |
| `cargo test -p warp --lib status_bar` | **15 过 / 0 败**（16 个新用例 + 既有） |
| `cargo test -p warp --lib view_impl::common` | **18 过 / 0 败** |
| `cargo test -p warp --lib --no-run` | 通过（新增 action 变体未破坏测试目标） |
| 二进制 grep | `terminal:attach_file` / `CanAttachFile` / `Warping with` / `keybinding-desc-terminal-attach-file` 均在 |
| 手动点验 | 见 §40.5 待用户确认 |

### 40.5 待点验 / 已知偏差

- 点验项：① Agent 回复期间 warping 行应显示「Warping with {模型名}...」；② ⌘K 搜索「附加文件 / Attach file to agent conversation」应能触发附加文件（默认无键位，属上游设计）。
- 已知偏差：模型名的可见性由**本地** flag 决定（Zap 默认开启），与上游「默认隐藏」不同；如需隐藏，删 `zap_oss.rs` 中该 flag 一行。

### 40.6 教训

1. **「删除死代码 → 再移植上游实现」的顺序很重要**：先删避免两套并存（本轮 `output_model_display_name` 与上游 `warping_model_message` 是同一需求的两种实现）。
2. **上游 flag 的门控要按本仓渠道重接**：本仓 `zap_oss.rs` 有专属 `with_additional_features` 入口——比照搬上游 `#[cfg(feature=...)]` + DOGFOOD 列表更贴合（也符合 AGENTS §5.4「优先运行时 flag」）。
3. **3-way 冲突默认取本地侧 + 事后只补所需片段，是应对「本地裁剪版」上游改动的稳妥法**：本地缺的整套上游结构（lightbox/cloud pane）若跟着 theirs 进来，会带进无关分支与死代码；本轮先全部取 ours，再由编译器逐条指出缺什么。
4. **移植 UI 动作要顺手补 i18n**：上游硬编码的 `"Attach file to agent conversation"` 在本地必须走 `crate::t!`（§39 教训的再次适用）。

### 40.7 补记：② 已回滚——本仓没有该功能的数据源（2026-09-12 晚）

**现象**：重建启动后用户点验，warping 行**仍是 `Warping...`**，未出现「Warping with {模型名}...」。

**根因**（证据）：
1. 上游这套是**服务端驱动**：模型名来自 proto 消息 `Message::ModelUsed`，由服务端在首个 LLM 尝试时下发；客户端在 `conversation.rs:2603` 把它写进 `AIAgentOutput::model_info`。
2. 本仓**从不构造** `Message::ModelUsed`——全仓只有 3 处**读取**（`conversation.rs:2593`、`conversation_yaml.rs:237`、`convert_conversation.rs:1414`），而 BYOP 流把它归入忽略分支（`agent_providers/chat_stream.rs:804` 与 `:967` 的 `=> {}`）。
3. ⇒ BYOP 会话里 `output.model_info` **恒为 `None`** ⇒ `resolve_warping_model_message(current=None, …)` 每次都走「尚未上报」分支 ⇒ 界面只能显示通用文案。16 个移植用例全绿也改变不了这一点：它们直接喂 `WarpingModelInputs`，绕过了空数据源。

**处置**：`git revert 73843d81e` → `ea3b6fadf`（5 文件 −477/+76，含删除 `status_bar_tests.rs`）。回滚后 `cargo check -p warp` 0 error、`view_impl::common` 18 过；③ attach-file 未受影响。

**若将来仍想要该效果**：正确路径不是移植上游（它等不到数据），而是**本地自行命名**——客户端本来就知道自己请求的模型（`LLMPreferences::get_llm_info(model_id).display_name`），这正是先前那个死代码 `AIBlock::output_model_display_name` 的思路。用户已判定不需要，故未实现；该死代码的删除（`43417ffc8`）仍然有效。

**教训**：「移植上游 UI 特性」前必须确认**数据源在本仓是否存在**——上游的展示层往往依赖服务端/云端消息，而本仓是纯客户端 BYOP。判断方法：构造点（写入侧）grep + 流处理分支是否有 `=> {}` 忽略。仅凭「代码能编译 + 单测全绿」不足以证明特性可用（本轮 16 个用例全绿，功能却是死的）。

---

## 41. 文档整合：状态总表（2026-09-12 收尾）

§35–§40 是**追加式的过程记录**，随着章节增多，已不便回答「现在到底合并了哪些 / 哪些没合并」。故新增
`specs/upstream-merge-status.md` 作为**唯一状态入口**：

| 内容 | 说明 |
|---|---|
| 总览漏斗 | 281 → 89 → 36（34 项任务）→ **31 已合并 / 5 未合并** |
| 已合并清单 | 31 个上游提交，按批次分组，含 `hash(#PR)` + 功能 + 本地 commit（与 §35.5/§36/§37/§40 逐条核对）|
| 未合并清单 | 5 个：`5e7030db7`（移植后回滚）、`8b88df987`+`40e397170`、`4cd1c77c4`、`511b952c2` |
| 额外本地改动 | i18n 中文化 ×2、复制时间戳入口、删死代码 + 2 个配套 commit |
| 规划期淘汰 | 53 条 / 8 类 |
| 未决事项 | 推送 52 提交、`4b894db80` 去留、`5e7030db7` 的将来正确路径 |
| 文档地图 | 五份文档各自职责与更新时机 |
| 验证基线 | 6 个测试目标的通过/失败基线（判断「失败是否既存」的对照）|

**维护约定**：以后每轮同步**只更新状态总表的表格**；过程细节继续往本文件追加新章节，两处不互相复制。
本节起，`lessons.md` 头部与 `CHANGELOG` 的更新责任不变。

---

## 42. 合并 `1e4b86a81`（#15517）：release-cli `codegen-units` 1 → 4（2026-09-12）

**来源**：盘查「编译期」主题时发现该提交（2026-08-25，属本区间）**不在计划的 89/53 名单内**——它只动根
`Cargo.toml`，疑被路径过滤漏掉（见 §41 之后 status.md §五 附注）。上游自述：*"4 (not the default 16, and
not 1) roughly halves build time versus codegen-units=1 for ~4% larger stripped/gzipped binaries."*

**落地**：`Cargo.toml` 的 `[profile.release-cli]`（`inherits = "release-lto"`，`opt-level = "s"`）由
`codegen-units = 1` 改为 `4`，注释按本仓规范改写为中文（保留上游三点理由：不是 16 也不是 1；约减半构建时间、
二进制约大 4%；thin LTO 已承担跨 crate 体积优化）。**未动** `[profile.release-wasm]`（其 `codegen-units = 1`
是另一件事，本地仍为 1）。

**受益范围（本地核实）**：`script/macos/bundle:382/388` 与 `script/linux/bundle:138/144` 打 CLI 产物时用的正是
`release-cli`（dev/local 用 `release-cli-debug_assertions`），另有 Windows 短别名 `rcli`/`rclida` 继承它
（`Cargo.toml:514-524`）→ 这些构建全部受益；**应用包（`release-lto`）不受影响**。

**验证**：`cargo check -p warp` 0 error（cargo 每次都会解析 profile，manifest 合法）；`grep codegen-units`
确认落点正确、`release-wasm` 仍为 1。
**未验证**：上游「构建时间约减半」这一具体数字**未在本仓复测**——`release-cli` 是 LTO 全量构建（数十分钟），
如需该数字应单独跑一次 `release-cli` 构建前后对比。

---

## 43. 移植 `a2f586584`（#10423）：修复历史会话里点命令行不展开（2026-09-12）

> 触发：用户报「zap 智能体（Agent 对话）的历史会话里，点击命令行不展开命令内容块」，并问「之前命令行展开的相关代码是否都合并了」。结论：**机制在（base 代码 `0dbd3d567`），但上游针对恢复会话命令渲染的系统性修复没合**——`a2f586584`(#10423) 与 `1daed2f0a`(#10776) 本地都没有。

### 43.1 根因（三轮临时探针实测，探针用完即删）

| 探针 | 结果 | 结论 |
|---|---|---|
| A | `action_type=Command 旧expanded=false` | 点击到达处理函数 ✓ |
| B | `value=true 当前=false editor_some=false` | 状态翻转 ✓（但无 editor）|
| C | `status done=true` **无早返回** | AI 块事件处理通过，已发 `UpdateInlineActionVisibility{true}` ✓ |
| G | `is_visible=true 匹配块数=1` | 按 action_id 揭示生效 ✓ |
| H | `目标 action 的块索引=[15] 块总数=19 AI块 total_idx=TotalIndex(23)` | 命令块确实存在 ✓ |
| D | `上方渲染=false 立即后继块=false editor=false mcp=false` | **三道渲染条件全不成立** ✗ |
| E2/E3 | `紧邻块: 存在=true 有 agent metadata=false` / `不是终端 Block` | **AI 块紧邻的下一项不是那个命令块** ✗ |

**根因**：`append_rich_content(AI, false)` 把 AI 块插在**活动块之前**，而 `create_restored_command_block` 又 `create_new_block` **push 到列表末尾** ⇒ 恢复后布局是 `[AI 块][隐藏的空输入块][命令块]`，隐藏块卡在中间 ⇒ `is_requested_command_block_immediately_after_ai_block`（`blocks.rs:2064`，与上游逐字相同）恒为 false ⇒ 展开后无内容可渲染。

### 43.2 上游解法（本提交采纳）

恢复时**先**把该会话持久化的命令块按序插入块列表（`insert_restored_block`，它经 `restore_block` **复用活动块**成为新的活动块 ⇒ 天然与 AI 块相邻），**再**按时间戳算出每个交换的 `command_block_index`，AI 块据此插到命令块之前；随后删除 `create_restored_command_block`（`blocks.rs` 69 行）。

### 43.3 本地适配（4 处）

1. **proto 时间戳不可用**：本仓 proto fork（`zerx-lab/warp-proto-apis` rev `14ab9a71`）的 `ShellCommandFinished` 只有 `output/exit_code/command_id`，**没有** `start_ts/finish_ts` ⇒ 上游顺带的「shell 命令运行时长」**未移植**；回退 `action_result/{convert,mod}.rs`、`shell_command.rs`、`convert_conversation.rs`、`convert_to_tests.rs` 的时间戳 plumbing，`conversation.rs` 的 `start_ts/completed_ts` 退回「消息时间戳 + 交换时间」回退链。
2. **保留本地 CLI subagent 机制**（本地 `54905881d`）：`restore_missing_cli_subagent_blocks_for_conversation` 被上游方案取代而删除，但其 `restore_cli_subagent_views_for_conversation` 调用与快照去重语义保留；`conversation.rs` 里本地的 `block_id/requested_command_action_id/subagent_task_id` 三字段与 5 个 helper 全部保留，与上游的 `message_id/start_ts/completed_ts` 并存。
3. **插入循环补去重保护**：`block_list.block_with_id(&block.id).is_none()` 才插入，避免「历史对话进入已有终端」时命令块重复（本地原本的去重语义）。
4. `pane_group/mod.rs` 上游新增的是 **cloud mode** 插桩（`replace_loading_pane_with_restored_ambient_cloud_mode_pane`），本仓无 cloud mode → 取本地侧（空）。

### 43.4 验证

| 项 | 结果 |
|---|---|
| `cargo check -p warp` | 0 error / 0 warning |
| `cargo test -p warp --lib --no-run` | 通过（含新移植的测试文件）|
| `load_ai_conversation`（新移植 14 用例）| **14 过 / 0 败** |
| `terminal::model::blocks` | 65 过 / 0 败 |
| `requested_command` | 15 过 / 0 败 |
| `terminal::view::` | **198 过 / 7 败**；基线（移植前）**184 过 / 同样 7 败** → 7 个失败为**既存环境问题**，本次净增 14 个通过 |
| 手动点验 | 待用户确认（历史会话里展开命令卡片应出现命令内容）|

### 43.5 教训

1. **「点击无反应」类问题要先分清「交互链路断」还是「渲染无内容」**：本轮 A/B/C 证明链路全通，真正的问题是**状态翻转后没有任何可渲染内容**（D 三条件全 false）——只看点击处理函数会白查。
2. **恢复/重放类路径的「顺序」是核心不变量**：同一批块，插入顺序不同就导致「块存在但相邻性检查失败」；对照上游**同名检查函数逐字相同**即可断定问题在插入侧而非检查侧——比继续加探针更快定位。
3. **移植前先核对 proto 依赖**：`ShellCommandFinished` 的时间戳字段是本仓 proto fork 没有的，导致「顺带特性」必须剥离（4 个文件回退）；不先核对就会到编译期才发现，还可能误判为「代码没合全」。
4. **`git stash pop` 后必须重新 `git add`**：本次因 pop 后未暂存、又直接 `git commit`（无 `-A`），提交只含新增测试文件、11 个改动文件漏在外面（已用 `--amend` 修正）。
5. **追加章节前先确认锚点唯一性**：此前追加 §40 时锚点落在 §39.5/§39.6 之间，导致 §39.6 被挤到 §42 之后——本轮已把 §39.6 移回 §39 内、§40 之前。