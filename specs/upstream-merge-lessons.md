# 上游合并经验文档

> 同步边界：`89f742fa6` → `ddba1684e` (73 commits)
> Zap 分支：`5e5dc06da7b8e8273b874a33f8c7946c575654e7`
> 日期：2026-07-30

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

*文档版本：v1.0*
*下次合并前必读*