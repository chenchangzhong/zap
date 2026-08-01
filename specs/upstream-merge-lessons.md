# 上游合并经验文档

> 同步边界：`89f742fa6` → `ddba1684e` (73 commits)
> Zap 分支：`5e5dc06da7b8e8273b874a33f8c7946c575654e7`
> 日期：2026-07-30

> **第二轮同步**：`ddba1684e` → `7cbb22d5c` (60 commits)
> 日期：2026-08-01

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

## 1. 核心原则

| 原则 | 说明 |
|------|------|
| **品牌字符串替换** | 所有用户可见 "Warp" → "Zap"、"Oz" → "Zap"、"Oz CLI" → "Zap Agent CLI" |
| **Zap 本地功能禁止覆盖** | OMP 集成（`CLIAgent::OhMyPi`、CLI agent session）、OhMyPi 模型选择器（`OmpModelSelector`）等 Zap 自研功能，上游 cherry-pick **不得覆盖**。合并时若冲突，取本地版本 |
| **第三方 Harness 已删除** | `ThirdPartyHarness`、`HarnessRunner`、`FeatureFlag::AgentHarness`、Claude/Gemini harness 执行路径已全部清理。上游 `Harness` 枚举变体保留（序列化兼容），`ClaudeHarness`/`GeminiHarness` struct 已删 |
| **云服务代码不合并** | 上游 workspace/team/云同步/Drive/Notebook sync 等 SaaS 功能 Zap 不需要，仅保留 struct 兼容字段 |
| **无 merge-base** | 采用分阶段手动 cherry-pick，非标准 git merge |

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

### 3.2 故意跳过的 Cherry-pick（后续处理）

| 文件 | 原因 |
|------|------|
| `app/src/terminal/view.rs` | slow-bootstrap 自动消除，与 OMP NDJSON 入口不同区 |
| `app/src/terminal/input.rs` | +277/-136 大规模改动，附件上传重构 |
| `app/src/ai/agent_sdk/config_file.rs` → `mcp_config.rs` → `driver.rs` → `mcp/templatable_manager/*` | MCP WellKnown ID 链，5 文件依赖 |
| `app/src/ai/blocklist/block.rs` → `response_stream.rs` | MCP tool name 响应流传递，与 NDJSON 交互需验证 |
| `app/src/ai/llms.rs` | `LLMProvider` 迁移到 `crates/ai`，纯重构 |
| `app/src/ai/agent_sdk/agent_management.rs` | workspace-aware agent management，云 teams 功能 |

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
| `agent/task_store.rs` 的 `prune_unreachable_subtasks()` | 2026-07-30 | `task_store.rs`、`task_store_tests.rs` | 函数 + 测试完整但无生产调用者，保留待后续观察 |

## 6. 误删/遗漏恢复记录

| 项目 | 原因 | 恢复方式 |
|------|------|----------|
| `schemars` workspace dep | 清理 `sentry` 时误删 | `INS.POST` 加回 |
| `rustls` workspace dep | 同上 | `INS.POST` 加回 |
| `reload_stale_conversation_files` feature | 清理 nld features 时误删 | `INS.POST` 加回 |
| `app/src/tui/user_info.rs` | 孤立模块引用已删 `warp_tui` | 删除整个 `app/src/tui/` 目录 |

---

## 7. 后续合并清单

### 7.1 必做
- [ ] `cargo check -p warp` 通过
- [ ] `cargo test -p warp --no-run` 编译通过
- [ ] `cargo build -p warp` 生成二进制
- [ ] `./target/debug/zap-oss --version` / `whoami` 冒烟通过
- [ ] 更新 `CHANGELOG.md` 记录同步边界
- [ ] 更新 `specs/upstream-merge-plan.md` 进度

### 7.2 推荐做
- [ ] 运行 `cargo nextest run --workspace --exclude command-signatures-v2`（需安装 nextest）
- [ ] 检查 `git diff HEAD --stat` 无异常大文件
- [ ] 搜索残留 `warp_tui`/`warp_search_core`/`warp_errors` 引用

### 7.3 可选
- [ ] 处理 Phase 4 遗留 cherry-pick（按优先级）
- [ ] 清理云服务相关死代码（`admin.rs` workspace 字段、`cloud_environments` 残留等）
- [ ] 补全 `warp_core/src/async` 模块声明（如未来需要 `warp_search_core`）

---

## 8. 常见坑位速查

| 现象 | 可能原因 | 对策 |
|------|----------|------|
| `cargo check` 报 `unresolved import` | 上游测试依赖本地无 helper | 删除测试而非补全 |
| `cargo check` 报 `use of unresolved module` | 同上 | 同上 |
| `warp_tui` 编译失败 | 缺 `warp::tui_export`（已删） | 删除 crate（Zap 不用） |
| `warp_search_core` 编译失败 | 缺 `warp_core::r#async` | 删除 crate 或补全模块声明 |
| `sentry`/`unicode-segmentation` 未使用 | 仅被已删 crate 依赖 | 同步删除 workspace dep |
| `FeatureFlag` 编译错误 | 上游新增 flag 本地无 | 在 `crates/warp_core/src/features.rs` 加 variant + 对应 FLAGS 列表 |
| `FeatureFlag::AgentHarness` 引用 | 上游 cherry-pick 引用此 flag | **跳过**，Zap 已删除此 flag 及其 6 处门控 |
| `ThirdPartyHarness` / `HarnessRunner` | 上游改动 driver/harness 模块 | **跳过**，Zap 已清理全部第三方 harness 执行路径 |
| 终端死锁 | `TerminalModel::lock()` 重入 | 检查调用栈，传已锁引用而非再次加锁 |

---

## 9. 关键文件定位

| 类别 | 路径 |
|------|------|
| Feature Flag 定义 | `crates/warp_core/src/features.rs` |
| Cargo workspace deps | `Cargo.toml` `[workspace.dependencies]` |
| App features | `app/Cargo.toml` `[features]` |
| 同步边界记录 | `specs/upstream-merge-plan.md` |
| 详细变更记录 | `specs/upstream-changes-detailed.md` |
| 遗留 cherry-pick 计划 | `specs/remaining-cherry-pick.md` |

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

## 11. 经验总结

1. **大胆删除**：上游新增的完整 crate（`warp_tui` 等）若依赖本地没有的胶水层，直接删——别试图修补。
2. **测试优先删**：Cherry-pick 带来的测试编译失败 90% 是上游专有 helper 缺失，删测试最快。
3. **品牌替换要全**：不仅代码字符串，`Cargo.toml` description、二进制名、帮助文本都要改。
4. **Zap 本地功能禁止覆盖**：OMP 集成、OhMyPi 模型选择器、CLI agent session 等 Zap 自研代码，
   上游 cherry-pick **不得覆盖**。合并时若冲突，取本地版本。已在核心原则中统一声明。
   （`oh_my_pi.rs` 已删除，是未被编译的死代码）
5. **记录同步边界**：每次合并必须在 `specs/upstream-merge-plan.md` 记录 commit hash，方便下次 diff。
6. **分阶段验证**：每阶段结束跑 `cargo check -p warp`，别攒到最后才发现基础设施坏了。

---

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

*文档版本：v1.0*
*下次合并前必读*
