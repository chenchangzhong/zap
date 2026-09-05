# 上游合并计划 · 2026-09 批次

> 状态：**待批准**。本文档只做计划，未执行任何拣入。
> 评估结论来源：2026-09-06 对两个上游开放 PR 的逐 PR 落点核验（3 个并行评估，证据均为 文件:行号 级）。
> 执行时每条结论需在拣入前重读落点复核（历轮教训：评估时的行号在执行时会漂移，以符号锚点定位）。

---

## 0. 背景与边界

| 项 | 值 |
|---|---|
| 本地基线 | `main` = `e2ae7e450`（主 worktree 在 `feat/dsh-webui-integration`，有未提交改动，**本批次不在主 worktree 执行**） |
| 上游 1（根） | `warpdotdev/warp`，master = `a48ff8014`；本地同步边界仍为 2026-08-13 `5fb3144db` |
| 上游 2（直接父仓库） | `zerx-lab/zap`，main = `5d874456a`；本地领先 241 提交、落后 0（其开放 PR 均未合并，需拣 PR 分支） |
| PR 引用 | 已批量 fetch 到本地：`upstream-pr/<N>`（warp）、`zerx-pr/<N>`（zerx），可直接 diff/cherry-pick |
| 平台判据 | 只发布 macOS；Windows/Linux/WASM 专属剔除 |
| 本轮不做 | 云服务/GraphQL/Drive/Factory/team scoping、`warp_tui`、依赖已删的第三方 harness 执行路径 |

### 全局适配常识（本轮多处用到）

- 本地**无 `warp_errors` crate**：上游凡用 `report_error!(..., ReportErrorLogMode::OncePerRun)` 的，本地一律降级为 `log::warn!`（携关键字段）或本地 `warp_core::errors::report_error!`（单参签名）。
- 上游目录差异：上游 `elements/gui/` → 本地 `elements/`；上游 `diff_state/local.rs` → 本地单文件 `diff_state.rs`；上游 `crates/warp_terminal/src/runtime.rs` → 本地并入 `app/src/terminal/mod.rs`。
- 迁移目录：实际在 `crates/persistence/migrations/`（AGENTS.md 写的顶级 `migrations/` 已过时）。
- 品牌检查：拣入 diff 中用户可见文案 `Warp` → `Zap`（§19.2 教训）。
- 拣入后跑 `grep -P '\n{3,}' <改动文件>` 查连续空行（§19.2 教训）。

---

## 1. 执行环境（一次性准备）

```bash
# 主 worktree 有未提交改动，且 cherry-pick 要求干净工作区 —— 独立 worktree 执行
git worktree add .worktrees/upstream-sync-2026-09 -b upstream-sync-2026-09 main
cd .worktrees/upstream-sync-2026-09

# 基线验证（记录既有警告数作为对照）
cargo check -p warp
cargo test -p warpui text_layout     # 基线 33/33
```

规约：
- **每个 PR 一个独立 commit**；上游拣入用 `git cherry-pick -x <sha>` 保留溯源；手工移植的在 commit message 里写明「移植自 upstream-pr/<N>」。
- draft PR 的拣入（仅 #15719、#15699 两个例外项）commit message 注明 `[draft 转正前拣入]`，上游转正后须 diff 复核（§18.1：取上游最终版）。
- 每批完成 `cargo check -p warp` 全绿才进下一批。
- 回滚：合回 main 前直接弃分支；合入后逐 commit `git revert`；涉及 migration 的（#15757）靠配套 `down.sql`。

---

## 2. 第一批：零冲突直拣（预计半天内）

彼此独立、落点与本地零/近零偏离，全部可 `cherry-pick -x` 或接近直拣。顺序无关，按下面顺序只是方便逐个验证。

### 2.1 warp #15746 — CoreText autorelease 排水（首选，5 行）

| 项 | 内容 |
|---|---|
| 落点 | `crates/warpui/src/platform/mac/text_layout.rs`：`layout_line_with_offset` else 分支（≈:381）与 `layout_text`（≈:630）包 `AutoreleasePoolGuard` |
| 本地前提 | 两处无 pool guard，与上游修复前逐字一致；`AutoreleasePoolGuard` 已在 `crates/warpui/src/platform/mac/mod.rs:56` 且 pub |
| 与本地定制关系 | 与 12e455c56（同 style runs 合并，≈:447-498）不重叠 |
| 验证 | `cargo test -p warpui text_layout` 33/33 |

### 2.2 warp #15652 — line editor 丢首个 precmd（本批最高价值密度）

| 项 | 内容 |
|---|---|
| 落点 | `app/src/terminal/line_editor_status.rs:70-79`：`let Some(active_session_id) = ... else { return; }` 早退，首个 precmd 早于 session 注册时被丢弃 |
| 规模 | 单文件 +14/-9 |
| 验证 | `cargo check -p warp` + `cargo test -p warp line_editor`（按上游附带测试） |

### 2.3 warp #15751 — 终端 resize 每行深拷贝（APP-5749）

| 项 | 内容 |
|---|---|
| 落点 | `crates/warp_terminal/src/model/grid/flat_storage/mod.rs`（`pop_rows` ≈:127-141 的 `Rc::unwrap_or_clone`）+ `row_iterator.rs`（`Iterator::next` 末尾 `Some(self.row.clone())` 使 strong_count=2，unwrap 必深拷贝） |
| 规模 | 4 文件 +127/-5，含新增 `row_iterator_tests.rs`（需确认 mod 声明符合本地 `${file}_tests.rs` 约定，cherry-pick 会带上） |
| 注意 | 调用点 resize 在 `app/src/terminal/model/grid/`（本地已把 grid 移到 app/），PR 不触碰该文件，无需适配 |
| 验证 | `cargo test -p warp flat_storage row_iterator` |

### 2.4 zerx #338 — BYOP 模型名 `-max` 后缀被剥（zerx 侧首选）

| 项 | 内容 |
|---|---|
| 落点 | `lib/rust-genai/src/adapter/adapters/openai/adapter_shared.rs`（+70/-9，含 2 单测） |
| 本地前提 | 该文件与 zerx base `5d874456a` **零偏离**，cherry-pick 应干净；测试依赖 `util_to_web_request_data`（adapter_shared.rs:62）与 `ChatOptionsSet::with_chat_options`（chat_options.rs:488）本地均在 |
| bug 链 | BYOP OpenAI 兼容 + 模型名带 `-max`/`-high` + effort 默认 `Auto`（`app/src/settings/ai.rs:828-829` 不注入）→ `ReasoningEffort::from_model_name` 剥后缀注入 `reasoning_effort` → 网关 503 model_not_found。受害名：`qwen-max`/`qwen3-max`/`gpt-5.1-codex-max` |
| 执行 | `git log --oneline 5d874456a..zerx-pr/338` 取提交，`git cherry-pick -x` |
| 验证 | `cargo test -p warp` 中 rust-genai 相关（patch 自带测试） |

### 2.5 warp #15719 [draft 例外] — macOS History/Up 菜单 Circular view update 崩溃

| 项 | 内容 |
|---|---|
| 落点 | `app/src/terminal/input.rs`（≈:13890 `InputAction::Up => self.editor_up(ctx)` 直调改 `dispatch_typed_action_deferred`） |
| 依赖 | `dispatch_typed_action_deferred` 已在 `crates/warpui_core/src/core/view/context.rs:416`，app 内 5 处已用 |
| 规模 | 2 文件 +136/-1（核心 5 行 + 测试） |
| 例外理由 | crash 修复、自包含、依赖齐备——符合「draft 等转正」的例外条款 |
| 验证 | `cargo test -p warp input` 相关；手动从 macOS 菜单触发 Up |

### 2.6 warp #15699 [draft 例外] — case-only rename 后文件树残留（APFS）

| 项 | 内容 |
|---|---|
| 落点 | `crates/watcher/src/lib.rs:325`：`let path_exists = is_rename && path.exists();` → 大小写敏感存在性判断（APFS 大小写不敏感，case-only rename 后旧路径误判存在） |
| 规模 | 4 文件 +94/-1；**新增 dev-dependency `tempfile`（仅测试用）**——执行时确认 `crates/watcher/Cargo.toml` 与 workspace deps 版本复用 |
| 例外理由 | macOS 主平台真 bug、改动极小自包含 |
| 验证 | `cargo test -p watcher` |

**第一批完成门禁**：`cargo check -p warp` 0 error；上述测试全绿；逐 commit `git show` 复查无顺手改动。

---

## 3. 第二批：框架/内存修复，需小适配（预计半天）

### 3.1 warp #15764 — AppContext 订阅泄漏（APP-5762）

| 项 | 内容 |
|---|---|
| 落点 | `crates/warpui_core/src/core/app.rs` `remove_dropped_items`（本地 ≈:3106-3165，上游 ≈:3402）：只清理被 drop 实体作为 emitter 的记录，不清理其作为 subscriber 的残留 |
| 规模 | 2 文件 +226/-1（~40 行实现 + 187 行测试） |
| 适配 | 行号偏移，hunk 手工落；上游第二个 commit（drop mid-emit 断言）**可舍弃** |
| 验证 | `cargo test -p warpui_core`（上游附带订阅生命周期测试） |

### 3.2 warp #15741 — 连续 ViewNotification 去重（APP-5741）【同主题 draft #15739 弃】

| 项 | 内容 |
|---|---|
| 落点 | `crates/warpui_core/src/core/view/context.rs`（`notify()` ≈:452-459 无去重）+ 全部 effect 入队点收敛（本地 8-9 处 `pending_effects.push_back` 散点：view/context.rs + model/context.rs） |
| 规模 | 4 文件 +196/-20 |
| 适配 | 上游 `enqueue_effect` 带 `report_error!(..., OncePerRun)` 超大队列告警——本地无 `warp_errors`，改为纯封装或 `log::warn!`（~10 行手工）；#15739 是其严格子集，**不拣** |
| 语义 | 只去重同一 view 的连续重复通知，安全 |
| 验证 | `cargo test -p warpui_core notify`；UI 冒烟（AI 输出流式渲染期间 CPU/内存） |

### 3.3 warp #15810 — EditDelta.precise_deltas Arc 化（APP-5810）

| 项 | 内容 |
|---|---|
| 落点 | `crates/editor/src/content/edit.rs:167` `Vec<PreciseDelta>` → `Arc<Vec<PreciseDelta>>`；buffer.rs 本地构造点 :935/:2538/:4802/:4943/:5025/:5151/:5200；消费端 `app/src/code/editor/model.rs:1335` |
| 适配（3 处机械） | ① 本地测试文件叫 `buffer_test.rs`（上游 `buffer_tests.rs`）；② **本地自有构造点** `crates/editor/src/render/model/mod.rs:2677/:2719` 的 `precise_deltas: Vec::new()` 需包 `Arc::new`（上游无此二点）；③ 行号偏移 55-110 |
| 顺序 | 必须先于 #15831（同碰 edit.rs，先小后大） |
| 验证 | `cargo test -p warp_editor buffer` |

**第二批完成门禁**：`cargo check -p warp` + 各 crate 测试；UI 冒烟一次（编辑器流式输出、面板开关）。

---

## 4. 第三批：手工移植 / 拆分拣入（预计 1 天，逐项可独立批准）

### 4.1 zerx #339 — 硬编码中文接入 Fluent（拆 3 个 commit）

| 子项 | 内容 |
|---|---|
| (a) 干净拣入 | 26 个与 base 零偏离文件：`zap_sftp`、`zap_sync`、`warp_ssh_manager`、`sftp_manager/*`、`cloud_sync_page`、`openai_compatible.rs`、`agent_providers/mod.rs` 等；crate 内英文 + UI 边界 `localize_sync_error` 方案与本地 `t!` 体系兼容 |
| (b) ftl 手工并入 | 91 个新 key 手工并入本地 `en/`、`zh-CN/`（本地 ftl 已偏离 base 180/302 行；**本地已无 ja，跳过 ja/warp.ftl**） |
| (c) 手工移植 | `app/src/workspace/view.rs:5928` 日志导出段 |
| 明确跳过 | 10 个 `tools/*.rs`（本地已自行英文化、措辞不同，拣入必冲突且无价值）；`autoupdate/linux.rs`（非 macOS） |
| 验证 | `cargo check -p warp`；切换英文 UI 冒烟 SFTP 错误提示 |

### 4.2 warp #15670 — SSH 后台提示抢焦点

| 项 | 内容 |
|---|---|
| 落点 | `app/src/terminal/view.rs:8418-8427`（`clear_ssh_blocks` 无条件 `redetermine_global_focus`）+ SSH choice block 插入尾部（≈:11131）→ 改为仅当自身/子视图持焦点时重判定 |
| 依赖 | `redetermine_terminal_focus`（≈:9436）、`is_self_or_child_focused` 本地齐备 |
| 适配 | 本地事件名 `ZapifySettings`（上游 `OpenWarpifySettings`），hunk 微调；代码 ~15 行 + 测试 |
| 验证 | 双 pane：后台 pane SSH 密码提示弹出时前台焦点不被抢（手动） |

### 4.3 warp #15724 与 #15684 二选一 — rich input 打开时光标下字形丢失

> **决策点（默认取 #15724）**：同一 bug、同一落点（`app/src/terminal/grid_renderer.rs:672/:1222` 逐字命中）、互斥。
> - **#15724**：+239/-35，删 skip 逻辑，199 行测试（含 ligature 路径）+ warpui_core 测试 FontDB delegate（本地落点 `crates/warpui_core/src/platform/test/delegate.rs` 存在）→ **回归保护最好，推荐**。
> - #15684：+74/-46，额外修 marked-text 对比色路径漏判 `hide_cursor_cell`（本地 ：676 起同款）→ 语义更全但测试少。
> 取 #15724，marked-text 对比色细节等上游两个 PR 定稿后跟随终版补（届时另一 PR 若被否决，其独有修复按 §18.1 原则评估单独移植）。

| 项 | 内容 |
|---|---|
| 验证 | `cargo test -p warp grid_renderer` + 手动：OMP/Codex 会话开 rich input，确认光标下字符可见 |

### 4.4 warp #15757 — 启动 block 恢复 SQL LIMIT

| 项 | 内容 |
|---|---|
| 落点 | `app/src/persistence/block_list.rs:129-159` `get_all_restored_blocks`（现全量 load 后内存 drain 到每 pane 100 条）→ SQL 窗口函数；`crates/persistence/src/model.rs` 加 `QueryableByName`（blocks.pane_leaf_uuid/start_ts 列本地已存在 :688/:699） |
| migration | 新目录 `crates/persistence/migrations/2026-09-03-004900_*` up/down.sql——执行时确认与本地现有迁移无时间戳冲突 |
| 注意 | migration 涉及建索引，先在本地大库（真实 ~/.zap）上验证 up 耗时可接受再合入 |
| 验证 | `cargo test -p warp block_list`；手动重启验证启动恢复行为与耗时 |

### 4.5 warp #15835 — Code editor 文件读 100MiB 守卫（手工移植 ~5 文件）

| 项 | 内容 |
|---|---|
| 落点 | `crates/warp_files/src/lib.rs:422/:469/:1084` 三处无界 `read_to_string`（打开/read_content/watcher reload）；`warp_util` 的 `FileLoadError` 加 `TooLarge` variant |
| 依赖已满足 | `BinaryFileReadResult::TooLarge` 本地已存在（`app/src/ai/blocklist/action_model/execute.rs:1071`）；与本地 11f6f4a91（text_file_reader 分段守卫）、3fe6e17ad（file-outline 上限）不重叠 |
| 手工面 | 上游 11 文件 +567/-91，实际手工 ~5 个：warp_files/lib.rs、warp_util file error、`app/src/code/mod.rs`（**warp_errors → warp_core 适配**）、`app/src/code/view.rs`（load 失败展示 16/38 行）、`app/src/code_review/code_review_view.rs`（本地高度分歧，只取守卫与降级展示） |
| 验证 | `dd` 造 >100MiB 文本文件手动打开（应降级提示不卡死）；watcher 触发 reload 同验 |

### 4.6 warp #15831 — 编辑器文本绕过 LayoutCache（APP-5825）

| 项 | 内容 |
|---|---|
| 落点 | `crates/editor/src/render/layout.rs:118`（主文本路径仍 `layout_cache.layout_text`）；`crates/warpui_core/src/fonts/external_fallback.rs:33-37` 加 `UncachedText` variant；7 个 element 文件本地零分歧；`strip_leading_unicode_bom` 改 pub(crate) |
| 适配 | `render/layout.rs` 本地 12+/64- 分歧；`render/model/mod.rs` 分歧 117+/1426-；`warpui_core/src/core/app.rs` match arm 补一行；Criterion bench 可不带 |
| 顺序约束 | 已在 4.3(#15810) 之后；**若将来取 #15733 必须先合本 PR**（同批 element/layout 文件） |
| 验证 | `cargo test -p warp_editor` + `cargo test -p warpui text_layout`；编辑器手动冒烟（中文输入、undo、长行折叠） |

**第三批完成门禁**：`cargo check -p warp`；完整 UI 冒烟（终端、编辑器、code review、SFTP、SSH）；CHANGELOG 草拟。

---

## 5. 明确不在本轮（记录在案）

| 组 | 项 | 原因 |
|---|---|---|
| zerx 可选 | #340 俄语翻译 | 维护成本低但用户群中英为主，~31 个本地新 key 回退英文；要合需手工适配本地无日语的 3 处 match。**默认不做**，用户点名再做 |
| zerx 跳过 | #331 kitty / #322 浮窗尺寸 | 本地已是超集/已有等效 |
| zerx 跳过 | #332 pwsh 7.6 | Windows 专属，记入未来 Windows 版清单 |
| zerx 跳过 | #321 workspace 大改 | 107 提交产品方向与本地自有方案冲突；其中 4e4ff90d9（关机持久化活动 block，385 行）值得将来单独立项重做 |
| warp 跳过 | #15825 / #15803 / #15801 / #15829 / #15796 / #15752 | 依赖已删 / 云 / WASM / macOS 不编译 / 本地已免疫 |
| warp 跳过 | #15797 / #15818 / #15817 / #15713 | 前置链缺失（gitignore_cache、rank.rs、CommandView、wait_for_mcp） |
| warp 可选未排期 | #15665 Hermes 门禁（需先验证 Hermes 真发 OSC 777）、#15683（手工取 reload 清树 + /tmp 规范化两条）、#15716（拆守卫、剥离诊断与 team scoping）、#15687 编辑器快捷键、#15682 footer 隐藏设置（需重接 ai_page）、#15748 run 脚本 --format osx | 收益小或需额外验证，用户点名再排 |
| warp 高成本缓议 | #15800 / #15793 / #15733 | 真实内存收益但落在本地重度分歧文件（code_review_view、code/view、render/model），属手工重写非拣入；建议合完 #15831 后单独立项 |
| 等转正追踪 | #15802（**转正后高优先**：SizeInfo 无 MAX 钳制已确认）、#15830 Mermaid 卡死、#15785 搜索 line_text 上限、#15742 超长行（需与 11f6f4a91 手工合并）、#15777 SSH 复用、#15828 垂直标签重命名提交、#15833、#15765 | draft 未定稿，等上游 merge 后按本轮判据重评 |

---

## 6. 收尾清单（全部批次完成后）

1. `cargo check -p warp` 全量 0 error（AGENTS.md 唯一硬门禁）。
2. 全量 UI 冒烟：终端输入/resize、OMP/Codex 会话 + rich input、编辑器（中文/undo/大文件）、code review、SFTP/SSH、设置页、启动恢复。
3. 品牌与残留检查：对本轮所有拣入 diff `rg -n "Warp"`（用户可见文案）；`grep -P '\n{3,}'` 空行。
4. 文档同步：
   - `specs/upstream-merge-history.md` 追加 §34（逐 PR：判定、适配点、验证结果、偏差）；
   - `specs/upstream-merge-lessons.md` 头部「同步边界」更新为本轮拣入的最高上游 commit；
   - `CHANGELOG.md` 追加条目；
   - 顺带修正两处文档过时事实：AGENTS.md 的顶级 `migrations/` 路径、README 三语表述（现仅 en/zh-CN）。
5. PR 分支合回 `main`（或先推 `upstream-sync-2026-09` 远端供 review）。
6. `git worktree remove .worktrees/upstream-sync-2026-09`。

---

## 7. 需要用户确认的决策点

| # | 决策 | 计划默认 |
|---|------|----------|
| 1 | #15724 vs #15684（rich input 光标字形，互斥） | 取 **#15724**（测试保护优先） |
| 2 | #15719、#15699 为 draft，是否接受例外提前 | **接受**（crash/APFS 真 bug、自包含），转正后复核 |
| 3 | zerx #339 拆分范围（跳过 tools/*.rs、ja、linux） | 按上表 |
| 4 | #340 俄语是否纳入 | 不纳入 |
| 5 | 第三批体量是否全做，还是先做 4.1-4.3 | 全做，但各子项独立成 commit 可单独取舍 |
