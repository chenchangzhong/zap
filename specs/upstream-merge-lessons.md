# 上游合并经验文档

> **同步边界**：`7cbb22d5c` 之后拣入 34 commit（散点，非连续区间；详见 §19）
> **✅ 遗漏修复（2026-08-05）**：terminal lifecycle recovery 栈 6 commit（#12853/#12854/#12855/#12856/#12858/#12859）已按方案 A 完整移植（详见 §21.4/§23）；§22 重扫发现的 5 件遗漏已全部拣入（zsh glitch 剥离 #14166/#12438、尾点链接 #12965、Hermes BracketedPaste #14367、系统终止 #12480、O(1) 焦点 #13113，详见 §23）
> Zap 分支：`5e5dc06da7b8e8273b874a33f8c7946c575654e7`
> 最后核验：2026-08-05，`cargo check -p warp` 通过（0 error）；全量 `cargo test -p warp --lib` 3802 通过 / 28 失败（与基线 worktree 复现的既有失败完全一致，新增归零）；9 个 reviewer 并行评审整改后提交 `81bbcf869`
>
> 历轮边界：
>
> | 轮次 | 区间 | commits | 日期 | 记录章节 |
> |------|------|---------|------|----------|
> | 一 | `89f742fa6` → `ddba1684e` | 73 | 2026-07-30 | §2–§11 |
> | 二 | `ddba1684e` → `7cbb22d5c` | 60 | 2026-08-01 | §0 |
> | 三 | 21 个 agent commit 核对 | — | 2026-08-01 | §12 |
> | 四 | 遗漏候选核对 | — | 2026-08-01 | §13 |
> | 五 | CLI agent 事件链路 5 项 | — | 2026-08-03 | §14 |
> | — | DeepSeek 集成移除 | — | 2026-08-03 | §15 |
> | 六 | `7cbb22d5c` → `02c042063` | 5 | 2026-08-03 | §16（全不合并） |
> | 七 | fork 点全量对账 21 个缺失 | — | 2026-08-03 | §17（落地 9 个） |
> | 八 | `f7e298027` 移植偏差回查 | — | 2026-08-04 | §18（修 3 处偏差） |
> | 九 | `7cbb22d5c` 后 34 个拣入 | 34 | 2026-08-05 | §19 |
> | 十 | lifecycle 栈 6 commit + 5 件遗漏 | 11 | 2026-08-05 | §21–§23（提交 `81bbcf869`） |
>
> **⚠️ 算待评估区间只能用上面的边界 hash**：本 fork 与上游无 merge-base
> （浅克隆，历史断开）。`git rev-list HEAD..upstream/master` 会把全部历史
> 算进去（曾得出 1795，真实待评估只有 4 个）。正确命令：
> `git log <边界hash>..upstream/master`。

---

## 决策速查表

每轮同步先查这张表；命中即按裁决执行，未命中再读详细章节。

| 上游改动主题 | 裁决 | 依据 |
|--------------|------|------|
| `crates/warp_tui` / `app/src/tui/` 任何改动 | 跳过 | §2 阶段③（crate 已删） |
| 云计费 / GraphQL credit / workspace teams | 跳过 | §1 核心原则 |
| 共享会话 / Drive / Notebook sync | 跳过 | §1 核心原则 |
| `ThirdPartyHarness` / `HarnessRunner` / `FeatureFlag::AgentHarness` | 跳过 | §10 |
| `CLIAgent::DeepSeek` 相关（注意区分 BYOP 的 `AgentProviderApiType::DeepSeek`） | 跳过 | §15 |
| `OmpModelSelector` / `CLIAgent::OhMyPi` / CLI agent session | 取本地 | §1（Zap 自研禁止覆盖） |
| `Stop` 分支字段清理（`clear_permission_scoped_state` 类） | 跳过 | §14 第 2 项 |
| rich status 判定机制（latch vs `session_id`） | 保持本地 | §14 第 4 项 |
| Codex OSC777/OSC9 双通道去重 | 跳过 | §14 第 3 项 |
| `settings_view/` 定制区 | 逐项判 | §0（冲突密集，低价值） |
| `FeatureFlag` promote 类（加 variant + 列表） | 可拣 | §0 教训（需手动补 Cargo feature 定义） |
| 纯加法的事件/枚举变体 | 可拣 | §14 第 1 项（`StopFailure` 先例） |
| 来自**未合并分支**的 commit | 查分支 tip | §18.1（初版可能已被 review 否决） |

### 移植前四问（§14 / §18 教训提炼）

1. **前置假设成立吗**——上游修复常依赖上游自己的实现前提。Zap 改过那个前提，
   同步修复反而引入退化（§14 第 2 项是典型）。
2. **本地已有等效解法吗**——同一问题两套解法不要合并。同形化看似便于未来
   cherry-pick，实际是删除已验证的本地功能换假想便利（§14 第 4 项）。
3. **字段有消费点吗**——零消费点字段改动无风险；在回落链上的字段改动高风险。
   同一提交里不同字段的风险可以完全不同，`grep` 消费点是必要步骤。
4. **这是上游的最终版本吗**——主题 grep 命中的常是初版。必查
   `git branch -a --contains <commit>` 与该分支后续 commit；未合并 master 的
   分支尤其要查有无返工（§18.1）。

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
| **DeepSeek CLI agent 已移除** | `CLIAgent::DeepSeek` 整条集成于 2026-08-03 删除（上游改名 CodeWhale 且 v0.9.0 移除 `deepseek`/`deepseek-tui` shim）。上游若新增 DeepSeek CLI agent 相关改动**一律跳过**，详见 Section 15。BYOP 侧 `AgentProviderApiType::DeepSeek`（模型提供商）**保留**，两者是独立实体 |
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

## 7. 每轮同步验收清单

这是**每轮同步都要重跑**的模板，不是一次性待办。下方「上次核验」列记录
2026-08-03 第五轮结束时的实测结果。

### 7.1 必做

| 检查项 | 上次核验（2026-08-03） |
|--------|------------------------|
| `cargo check -p warp` 通过 | ✅ 通过 |
| `cargo build -p warp` 生成二进制 | ✅ `target/debug` 已产出 |
| 二进制冒烟（`--version` / `whoami`） | ✅ 通过 |
| `CHANGELOG.md` 记录同步边界 | ✅ 已记录 |
| 本文档头部「同步边界（最新）」已更新 | ✅ `7cbb22d5c` |

### 7.2 残留引用核验

已删功能不得有残留；BYOP 同名项不得误删。

| 检查项 | 期望 | 上次核验（2026-08-03） |
|--------|------|------------------------|
| `warp_tui` / `warp_search_core` / `warp_errors` | 0 引用 | ✅ 均 0 |
| `CLIAgent::DeepSeek` / `DeepSeekLogo` / `deepseek.svg` | 0 引用 | ✅ 均 0 |
| `AgentProviderApiType::DeepSeek`（BYOP，**保留项**） | >0 引用 | ✅ 3 处 |

核验命令：

```bash
for s in warp_tui warp_search_core warp_errors \
         'CLIAgent::DeepSeek' DeepSeekLogo deepseek.svg; do
  echo "$s: $(grep -rl "$s" app crates script Cargo.toml 2>/dev/null | wc -l)"
done
grep -rl 'AgentProviderApiType::DeepSeek' app crates | wc -l   # 应 > 0
```

### 7.3 已知未做项（明确不做，非待办）

| 项 | 结论 |
|----|------|
| `cargo nextest run` 全量测试 | **未跑**——nextest 未安装。`cargo check` 是本仓交付门槛（见 AGENTS.md §5.1），全量测试非必需 |
| 云服务死代码清理（`admin.rs` workspace 字段、`cloud_environments` 残留） | **不做**——保留 struct 兼容字段是 §1 既定原则，删除会破坏服务端反序列化 |
| 补全 `warp_core/src/async` 模块声明 | **不做**——仅 `warp_search_core` 需要，该 crate 已删且不会回来 |
| Phase 4 遗留 cherry-pick（§3.2 六项） | **不做**——见 §3.2 各项已更新为终局结论 |

---

## 8. 常见坑位速查

| 现象 | 可能原因 | 对策 |
|------|----------|------|
| `cargo check` 报 `unresolved import` | 上游测试依赖本地无 helper | 删除测试而非补全 |
| `cargo check` 报 `use of unresolved module` | 同上 | 同上 |
| `warp_tui` 编译失败 | 缺 `warp::tui_export`（已删） | 删除 crate（Zap 不用） |
| `warp_search_core` 编译失败 | 缺 `warp_core::r#async` | 删除 crate——该 crate 已于第一轮删除且不再引入 |
| `sentry`/`unicode-segmentation` 未使用 | 仅被已删 crate 依赖 | 同步删除 workspace dep |
| `FeatureFlag` 编译错误 | 上游新增 flag 本地无 | 在 `crates/warp_core/src/features.rs` 加 variant + 对应 FLAGS 列表 |
| `FeatureFlag::AgentHarness` 引用 | 上游 cherry-pick 引用此 flag | **跳过**，Zap 已删除此 flag 及其 6 处门控 |
| `ThirdPartyHarness` / `HarnessRunner` | 上游改动 driver/harness 模块 | **跳过**，Zap 已清理全部第三方 harness 执行路径 |
| `CLIAgent::DeepSeek` 引用 | 上游改动 DeepSeek CLI agent 路径 | **跳过**，Zap 已删除整条集成（枚举变体、handler、plugin manager、logo）。注意区分 BYOP 的 `AgentProviderApiType::DeepSeek`（保留） |
| 终端死锁 | `TerminalModel::lock()` 重入 | 检查调用栈，传已锁引用而非再次加锁 |

---

## 9. 关键文件定位

| 类别 | 路径 |
|------|------|
| Feature Flag 定义 | `crates/warp_core/src/features.rs` |
| Cargo workspace deps | `Cargo.toml` `[workspace.dependencies]` |
| App features | `app/Cargo.toml` `[features]` |
| 同步边界记录 | **本文档头部**（原 `specs/upstream-merge-plan.md` 已删） |
| 详细变更记录 | **本文档 §0 / §12–§15**（原 `specs/upstream-changes-detailed.md` 已删） |
| 跳过项终局结论 | **本文档 §3.2 / 决策速查表**（原 `specs/remaining-cherry-pick.md` 已删） |
| 验证纪律 | `AGENTS.md` §5.6.1 |
| 上游源码对照（**常驻**） | `/Users/zhong/project/.worktrees/upstream-master` |

> **上游 worktree 是常驻设施，不要删除。** 它挂在 `upstream/master` 的 detached HEAD 上，
> 并有**独立**的 codegraph 索引（主仓 299 MB / 上游 427 MB，互不覆盖），
> 用于对照上游实现与查上游侧调用方。
>
> ```bash
> # 每轮同步前更新
> git fetch upstream master
> cd /Users/zhong/project/.worktrees/upstream-master
> git checkout --detach upstream/master
> codegraph sync
> ```
>
> 若意外删除，完整重建（**两步，缺一不可**）：
>
> ```bash
> git worktree add /Users/zhong/project/.worktrees/upstream-master \
>   upstream/master --detach
> cd /Users/zhong/project/.worktrees/upstream-master
> codegraph init          # 不是 sync——见下方坑位
> ```
>
> **坑位一：新 worktree 必须先 `init`。** 直接跑 `codegraph sync` 会正常输出
> 「Indexed 4,101 files / 133,484 nodes」然后报 `CodeGraph not initialized`——
> 索引算完了但没落盘，`.codegraph/` 不会创建。先 `init` 才写库。
>
> **坑位二：`.codegraph/` 需写进 `.git/info/exclude`。** 上游 `.gitignore` 不含
> 该条目（那是 Zap 本地加的），且 `.codegraph/.gitignore` 只忽略目录内部文件、
> 不忽略目录本身，所以会污染 worktree 的 `git status`。已写入
> `.git/info/exclude`（worktree 与主仓共享该文件），重建后无需重做。

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

*文档版本：v2.4*
*下次合并前必读*
