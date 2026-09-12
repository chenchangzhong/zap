# 上游合并计划 · 2026-08-14 → 2026-09-11 区间（§35 轮）

> 状态：**待批准**。本文档只做计划，**未执行任何拣入、未改任何生产代码**。
> 评估结论来源：2026-09-12 对区间内 281 个上游提交的逐条核验（只读；证据均为 `文件:行号` 或「符号全库 0 命中」级）。
> 执行纪律：每条结论**在拣入前必须重读落点复核**（历轮教训：评估时的行号在执行时必然漂移，一律用符号锚点定位）。

---

## 0. 计划要件

| 项 | 内容 |
|---|---|
| Goal | 把 2026-08-14 → 2026-09-11 区间内**本地确实缺失且适用**的上游修复并入 `main`，崩溃/真实 bug 优先 |
| Architecture | 沿用历轮方式：独立 worktree + 上游 `cherry-pick -x` 逐条移植，冲突时按本地形态手工适配（非 git merge） |
| Tech Stack | Rust / Cargo workspace；验证门槛 `cargo check -p warp`（AGENTS.md §5.1） |
| Baseline refs | `specs/upstream-merge-lessons.md`（决策速查表 / 移植前四问 / 核心原则）、`specs/upstream-merge-history.md` §17.3 §17.4 §18.1 §35、`AGENTS.md` §5 |
| Requirement Ready Check | ready —— 候选已逐条核验，落点为 文件/符号 级证据；**执行授权仍待用户批准**（§17.7） |
| Compatibility Boundary | 不改任何公开契约/持久化 schema；`FeatureFlag`/Cargo feature 只删「全库 0 引用」的孤儿项；不引入新 crate |
| Change Necessity | code-change —— 36 个提交均为本地存在对应落点的行为修复（崩溃/竞态/渲染/脚本正确性），无 docs-only 替代路径 |
| TDD Route | Mode: `off`；Decision: `skipped`（用户未要求 strict，本地既定门槛是 `cargo check` + 定向回归）；Test posture: post-change regression |
| Verification | 每批 `cargo check -p warp` + 该批定向测试/脚本语法校验（见 §5） |
| Existence Check | 仅两处新增文件，均**照抄上游既有文件**、非本地新造 owner：`crates/warp_completer/src/signatures/legacy/miss_cache.rs`（4.3）、`crates/repo_metadata/src/gitignore_cache.rs`（4.4）。判定 `add-with-proof`；无新 crate、无新契约、无新持久化面 |
| 代码内容来源 | 本计划不粘贴上游代码全文——**权威来源是 `git show <sha>` 本身**；计划只固定「落点锚点 + 适配要点 + 验证命令」，与本仓历轮计划（`upstream-merge-plan-2026-09.md`）惯例一致。执行时以 `cherry-pick -x` 取原 diff 为准，禁止凭记忆重写 |
| 本轮不做 | 云/团队 scoping、GraphQL、Drive/共享会话、Factory、`warp_tui`、Windows/WASM 专属、纯重构与编译期优化 |

### 0.1 基线

| 项 | 值 |
|---|---|
| 本地 | `main` = `9006d3645`（**工作区有 1 个未提交文件 `specs/upstream-merge-history.md`**） |
| 上游 | `warpdotdev/warp` master = `4143c09ff`（2026-09-11） |
| 真实 fork 点 | `git merge-base HEAD upstream/master` = `c325d146` |
| 区间 | 2026-08-14 → `4143c09ff`，**281** 提交；三轮筛选后**候选 89**，逐条核验后**可做 36**（34 项任务，其中 2 项各含 1 个后续提交）|
| 上一轮 | §34（2026-09-06，两上游开放 PR 共 15 项）；其 worktree 分支 `upstream-sync-2026-09` 已全部并入 `main`（0 独有提交） |

### 0.2 逐条裁决汇总（完整表见执行后写入 `history.md §35`）

| 裁决 | 数量 | 去向 |
|---|---|---|
| 可做 | **36** | 本计划 §3 批次 1–6（34 项任务；`40e397170`、`18179177a` 各随 6.2 / 6.3 同批）|
| 已含等效 | 3 | §4 不并入 |
| promote 无对象 | 4 | §4 不并入 |
| 结构性跳过 | 46 | §4 不并入 |
| 合计 | 89 | — |

---

## 1. 全局适配常识（本区间多处用到）

1. **路径漂移**（本轮最大成本，281 条触及上游路径 1081 个，本地缺失 599）：
   - 上游已把终端 grid 代码搬到 `crates/warp_terminal/src/model/…`，**本地大部分仍在 `app/src/terminal/model/…`**（本地未跟随上游 `21f413b79`）。
   - 上游 `app/src/terminal/view_tests.rs` → 本地 `app/src/terminal/view/view_tests.rs`；上游 `app/src/ai/agent_sdk/driver_tests.rs` → 本地 `app/src/ai/agent_events/driver_tests.rs`。
   - 上游 `crates/warpui_core/src/elements/gui/*` → 本地 `crates/warpui_core/src/elements/*`（无 `gui/` 前缀）。
2. **本地无 `crates/warp_errors`、无 Sentry 上报链**：凡上游改动只是「typed error / `extra:` / `OncePerRun` / PII 脱敏」的，一律不并入（§7.3 / §30.2）。
3. **`git apply --check` 失败 ≠ 前提缺失**：只说明上下文行对不上；前提是否成立必须读本地函数体。
4. **清单 bump（`Cargo.lock` + `Cargo.toml` pin 同改）默认跳过**，除非该 pin 携带的是本地真实缺的修复（本区间仅 `542683634`、`511b952c2` 两条例外，见 §3 批次 3）。
5. **品牌**：拣入 diff 中的用户可见 `Warp` → `Zap`、`Oz` → `Zap`（§19.2 教训）。
6. **空行**：移植后跑 `grep -P '\n{3,}' <改动文件>`（§19.2 教训）。
7. **测试惯例**：优先只移生产代码；上游测试若能直接编译则一并带上（本计划在每条标注是否有测试可带）。不跑 `cargo nextest`（未安装），用 `cargo test -p <crate> <filter>`。

---

## 2. 执行环境（一次性准备）

```bash
# 主 worktree 有未提交改动（specs/upstream-merge-history.md），cherry-pick 要求干净工作区 → 独立 worktree
# 复用上一轮已并入 main 的分支名会歧义，故新建：
git worktree add .worktrees/upstream-sync-2026-09-11 -b upstream-sync-2026-09-11 main
cd .worktrees/upstream-sync-2026-09-11

# 基线对照（记录既有 warning 数）
cargo check -p warp
```

规约：

- **每条上游修复一个独立 commit**；直拣用 `git cherry-pick -x <sha>` 保留溯源，手工适配的在 commit message 写明「移植自 upstream `<sha>`(#PR)」。
- 每条拣入后**先跑该条验证**，整批结束再跑一次 `cargo check -p warp`；批次间不攒。
- 落点冲突时**取本地**（§1「Zap 本地功能禁止覆盖」）。
- 任一条在当前基线无法干净适配 → 记为「本轮不做」并写下原因，**不硬塞**。
- 回滚：合回 `main` 之前直接弃分支；合入后逐 commit `git revert`。

---

## 3. 批次计划（34 项任务 / 36 个上游提交）

### 批次 1 — P0 崩溃 / 真实 bug（9 条，先做）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 验证 |
|---|---|---|---|---|---|---|
| 1.1 | `a7326f8fe` (#15763) | 提升行尾 cell 为宽字符时崩溃 | `app/src/terminal/model/grid/ansi_handler.rs` `push_zerowidth`；`crates/warp_terminal/src/model/grid/cell.rs` | §35.4 已核：与上游修复前逐字相同，`push_zerowidth` 本地返回 `()` | 4 个消费点同步改签名：app ansi_handler、`grid_renderer/unicode_placeholder.rs`、`.../flat_storage/row_iterator.rs`、`.../flat_storage/testing.rs` | `cargo test -p warp_terminal` + `cargo check -p warp` |
| 1.2 | `92a98662f` (#15720) | 空流式 Agent 文档更新崩溃 | `crates/editor/src/content/core.rs:1072` | 仍是 `if range.end >= self.max_charoffset() && …`（无 `buffer_end == zero` 守卫） | 引入 `let buffer_end = self.max_charoffset();` + 短路；上游测试在 `content/markdown_tests.rs`，可带 | `cargo test -p warp_editor` |
| 1.3 | `83e270f1d` (#15705) | glyph 无 bbox 时 `em_width` panic | `crates/warpui_core/src/fonts.rs:537-545` | 仍 `.expect("we verify in Config::new that we can measure the typographic bounds of the 'm' glyph")` | 改为 `match`：bounds → advance → `font_size.max(1.0)` + `log::warn!`；上游测试 `fonts_tests.rs` 依赖 mock FontDB，本地同构则可带 | `cargo test -p warpui_core fonts` |
| 1.4 | `ee95ac0fd` (#15322) | 已结束后台 block 双光标 | `app/src/terminal/block_list_element.rs:2595 / 2677-2680 / 2772-2774`；`app/src/terminal/model/block.rs` | 三处判定与上游修复前一致；第 75 行仍 `use …grid_handler::{Link, TermMode}` | 新增 `Block::is_command_cursor_visible()` / `is_output_cursor_visible()`；渲染侧抽 `command_grid_visible_cursor_shape()` / `output_grid_visible_cursor_shape()`；**确认删掉 `TermMode` 后本地其它使用点不受影响** | `cargo check -p warp` + `cargo test -p warp terminal` |
| 1.5 | `33c3bf6b7` (#15884) | 重复确认「丢弃文件」时 `[0]` 越界 panic | `app/src/code_review/code_review_view.rs:9519` | 仍 `let file_path = self.discard_dialog_state.discard_file_paths[0].clone();` | 本地形态与上游不同（本地未调 `to_standardized_path`）：改为 `.first()` + `if let Some(...)`；**另核 `:7287` 处同类 `[0]` 是否需一并加守卫** | `cargo test -p warp code_review` |
| 1.6 | `fbbfc41f3` (#15422) | grep 工具在含冒号路径上 `ParseIntError` | `app/src/ai/blocklist/action_model/execute/grep.rs:649-680` | `parse_grep_output` 仍是 `line.split(":")` + `line_number.parse::<usize>()` 旧实现 | 按上游重构为 `GrepCommandOutcome{NoMatches,Matches}` + `execute_grep_command`；本地 `run_git_grep_command` 与上游修复前同构；确认 `build_git_grep_command` 保持 | `cargo test -p warp grep` |
| 1.7 | `53b502c8e` (#15771) | 「新建窗口」快捷键不可重绑 | `app/src/resource_center/utils.rs:106-119`；`app/src/app_menus.rs`；`app/src/workspace/mod.rs` | 本地仍是 `CommandBinding::new("workspace:new_window", "Open New Window", Some(cmd-n))` | 删除固定绑定；菜单项改为 `move |_props, ctx| { … find(trigger == CustomAction::AddWindow …) }`；`MenuItemPropertyChanges`/`bindings::trigger_to_keystroke` 本地已存在 | `cargo check -p warp` + 手动验证菜单快捷键随绑定变化 |
| 1.8 | `90c2484dc` (#15310) | 隐藏且不可重绑的 Alt+1 Project Explorer | `app/src/workspace/mod.rs:435-448` | 本地仍有 `app.register_fixed_bindings([FixedBinding::custom(CustomAction::ToggleProjectExplorer, …)])`（文案已 Fluent 化） | 删该 `FixedBinding` 块；**确认 `register_editable_bindings` 中仍有同 action 的可编辑绑定**，否则不能删 | `cargo check -p warp` + 手动验证 Project Explorer 快捷键可改 |
| 1.9 | `5cd24ed1b` (#15779) | in-band command reset 误报 warn（仅日志） | `app/src/terminal/model/terminal_model.rs:3312-3313` | 仍是 `IsReceivingInBandCommandOutput::No => log::warn!("Received 'end_in_band_command_output' …")` | 改为 `No if from_osc_sequence => warn!(新文案)` + `No => {}`；价值低，可最后做 | `cargo check -p warp` |

### 批次 2 — shell / bootstrap 脚本（5 条）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 验证 |
|---|---|---|---|---|---|---|
| 2.1 | `e722ebeda` (#15428) | 四个 shell-integration 潜在 bug | `bash_body.sh:734`；`fish.sh:166 / 171` | `echo "$1" \| command -p od …`；`test (! string match -q … $argv[1])`；`kill -9 $pids`（`$pids` 是不存在的拼写错误） | 四处小改：`printf '%s'`、`not string match -q … -- (string trim -- …)`、`kill -9 $pid`；纯脚本改动 | `bash -n app/assets/bundled/bootstrap/bash_body.sh`、`fish -n …/fish.sh`（若无 fish 则目视） |
| 2.2 | `607be8c26` (#15518) | bash bootstrap 丢 `shell_plugins` | `bash_body.sh:1362 / 1384` | 仍用未转义的 `"$shell_plugins"`；`escaped_json` 里缺 `shell_plugins` 字段 | +3 行：先 `shell_plugins_list="$(printf '%s\n' "${shell_plugins[@]}")"`，再 `warp_send_hook_kv_pair_escaped`，并在 `escaped_json` 补回字段 | `bash -n` + 启动 dev app 看 Bootstrapped 载荷含 `shell_plugins` |
| 2.3 | `0140af045` (#15313) | zsh 丢 `_describe` 的 `-ld` 描述 | `zsh_body.sh:1331-1333` | 仍是 `if (( $@[(I)-d] )); then … __tmp=${@[$[${@[(i)-d]}+1]]}` | 换成按 flags 前缀搜索 `-[a-zA-Z]#d`（`setopt localoptions extendedglob` + `__flags=(${@[1,(i)(-|--)]})`）；注意与本地 zsh 定制（glitch 剥离区）不重叠 | `zsh -n app/assets/bundled/bootstrap/zsh_body.sh` |
| 2.4 | `294033bb1` (#15118) | 只绑 main keymap 导致 bootstrap 残留回显 | `zsh_body.sh:347-351` | 仍是 `bindkey -r '^P'` + `bindkey '^P' kill-buffer` | 新增 `warp_kill_buffer_and_reset_insert_mode` widget + 对 `main emacs viins vicmd` 四个 keymap 逐个 `bindkey -M`（`2>/dev/null \|\| :`） | `zsh -n` + 手动：`bindkey -v` 后再触发清行 |
| 2.5 | `17f432027` (#15792) | "honor PS1" 模式下 Bash PS1 二次展开 | `bash_body.sh:497-521`（deref 区）与 `:645-662`（`local honor_ps1` 区） | 两段都与上游修复前一致；上游把 `honor_ps1` 判定前移并与 `deref_ps1`/`escaped_ps1` 合并 | **结构重组，必须手工**：`WARP_HONOR_PS1==1` 时不再二次 deref（`deref_ps1=""`、`escaped_ps1=""`），否则保持原逻辑；`WARP_IN_MSYS2` 分支保留 | `bash -n` + 三种组合手动冒烟（honor PS1 开/关 × MSYS2 开/关） |

### 批次 3 — 构建 / 依赖（3 条，逐条独立 commit）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 验证 |
|---|---|---|---|---|---|---|
| 3.1 | `ccf683193` (#15694) | 删孤儿 cargo feature | `app/Cargo.toml` | 上游删的 18 条中本地存在 12 条（`command_predictor`、`ssh_enable_host_denylist_in_settings`、`grab_the_baton_editing`、`inline_ssh_banner`、`notebook_parameter`、`quake_mode`、`rich_history`、`system_theme`、`toggle_bootstrap_block`、`permanent_autosuggestion_hint`、`agent_management_popup`、`conversation_filter`），**全库 `feature = "<名>"` 引用数均为 0** | 只删这 12 条；上游另 6 条（`cloud_object_initial_load` 等）本地本就没有。删前**再跑一次全库引用检查**（含其它 `Cargo.toml`） | `cargo check -p warp` |
| 3.2 | `542683634` (#15569) | cosmic-text pin 到「禁止 Hack 作 fallback donor」 | `crates/warpui/Cargo.toml:147` | rev 仍是 `15198beba692162201c0ea8b15222cf5643ea068`（上游改 `a7c7b71497542758e08f77390b8efce543b3181f`） | 1 行 rev + `Cargo.lock`；**需能 fetch 该 rev**（同 fork `warpdotdev/cosmic-text`）；lock 只应变动 cosmic-text 相关条目，若出现大面积变动则放弃 | `cargo check -p warp` + `cargo test -p warpui text_layout`（基线 33/33） |
| 3.3 | `511b952c2` (#15380) | `create_file` 工具 `allow_overwrite` | `crates/ai/src/agent/action/{mod,convert}.rs`、`request_file_edits/diff_application.rs` | `allow_overwrite` 符号全库 0 命中；但该提交同时 **bump `warp-proto-apis` rev**（`app/src/ai/agent/api/impl.rs` 本地本就缺失） | **成本最高的一条**：需要先确认本地能否升 `warp-proto-apis` 到该 rev（`supports_create_file_overwrite` 由 proto 提供）。若 pin 不可升或 lock 变动过大 → 记「本轮不做」 | `cargo check -p warp` + API 侧手动验证 |

### 批次 4 — 性能（4 条）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 验证 |
|---|---|---|---|---|---|---|
| 4.1 | `d89e78385` (#13508, APP-4844) | 加载大文件时整份 styled blocks 克隆 | `crates/editor/src/content/buffer.rs` 6 处 `new_lines: self.styled_blocks_in_range(…)` | 6 处均无 `Arc::new`，字段仍是裸 `Vec`（本地未含 APP-4844） | `BufferToFormattedText::new_lines` 改 `Arc<…>`，6 处包 `Arc::new`；`content_update.new_lines.len()` 等消费点确认无需改 | `cargo test -p warp_editor` + `cargo check -p warp` |
| 4.2 | `1c925e333` (#15128, APP-5392) | 限制 `EditDelta::layout_delta` 的 rayon fan-out | `crates/editor/src/render/layout.rs`、`crates/editor/src/content/edit.rs` | `chunk_layout_tasks`、`MAX_LAYOUT_TASKS_PER_PARALLEL_CHUNK`、`MAX_LAYOUT_CONTENT_CHARS_PER_PARALLEL_CHUNK` 全库 0 命中 | 新增 `chunk_layout_tasks` 分块（64 任务 / 64KiB 内容上限）后并行；确认本地 `LayoutTask` 类型同构 | `cargo test -p warp_editor` |
| 4.3 | `213c9b32e` (#15181, APP-5431) | SignatureCache 无界增长 | `crates/warp_completer/src/signatures/legacy/{mod,registry}.rs` | 本地有 `SignatureCache`（4 处命中），但无 `miss_cache.rs`、无 key 长度上限 | 新增 `crates/warp_completer/src/signatures/legacy/miss_cache.rs`（`MAX_CACHED_MISSES = 256` + `VecDeque` FIFO + `RwLock`）并在 registry 接线；按本地 `${file}_tests.rs` 约定加 mod 声明 | `cargo test -p warp_completer` |
| 4.4 | `c6609ef23` (#15240, APP-4828) | file-tree 遍历重复构造 gitignore matcher | `crates/repo_metadata/src/{entry,file_tree_store,local_model,repository,watcher}.rs`、`crates/ai/src/index/file_outline/*` | `GitignoreCache` 全库 0 命中，需新建 `crates/repo_metadata/src/gitignore_cache.rs` | 引入共享缓存并按仓库隔离；上游同时改 `crates/ai/src/index/full_source_code_embedding/*`（本地无）→ **只取 repo_metadata 侧** | `cargo test -p repo_metadata` + `cargo test -p watcher` |

### 批次 5 — 功能（小成本，7 条）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 验证 |
|---|---|---|---|---|---|---|
| 5.1 | `3a7a4a5b3` (#15376, PR0 of APP-5559) | 搜索前不画空 category 头 | `app/src/settings_view/settings_page.rs`（`Category<V>` 在 :1843） | `categories_with_visible_content` 0 命中；`Category`/`PageType`/`FilteredPageType` 本地齐 | 加 `categories_with_visible_content` helper + 渲染处改用；上游带 `NeverRendersWidget` 测试可带 | `cargo test -p warp settings` |
| 5.2 | `79a9cb721` (#15475) | completer 选项参数按「值位置」解析 | `crates/warp_completer/src/completer/engine/argument/legacy.rs`、`src/parsers/hir/mod.rs` | `option_value_index` 0 命中 | 新增 `option_value_index()` 并把两处解析改用；需 `FlagType`/`NamedArgument`（本地已有） | `cargo test -p warp_completer` |
| 5.3 | `0a0fd3ae1` (#15346) | 块列表右键菜单加 Paste | `app/src/terminal/view.rs`（`context_menu_items` ≈:14486） | `paste_menu_item` 0 命中；`BlockListMenuSource::*RightClick` 本地齐 | 按上游把 items 构造成 `let mut items = …` 后追加分隔符 + `self.paste_menu_item(ctx)`；注意本地 view.rs 定制，取最小 hunk | `cargo check -p warp` + 手动右键验证 |
| 5.4 | `4cd1c77c4` (#15007) | agent 输入工具条加 File explorer chip | `app/src/ai/blocklist/agent_view/agent_input_footer/{mod,toolbar_item}.rs`、`app/src/terminal/view/use_agent_footer/mod.rs` | `CodeSettingsChangedEvent::ShowProjectExplorer` 本地已存在（`app/src/settings/code.rs:25`） | 新增 toolbar item + 订阅 `CodeSettings`，`ShowProjectExplorer` 变化时 `ctx.notify()` | `cargo check -p warp` + 手动验证 chip 显隐跟随设置 |
| 5.5 | `3a6f05512` (#15579, APP-5344) | AI plan 文档编辑器延迟布局 | `app/src/ai/document/ai_document_model.rs`、`app/src/notebooks/editor/model.rs` | 落点齐；`LayoutTiming`/`will_auto_open` 由该提交引入 | 引入 `LayoutTiming::{Eager,Lazy}` 并在 `will_auto_open` 时走 Lazy；上游集成测试依赖本地缺失的 `integration_testing/ai_document.rs` → **不带测试** | `cargo check -p warp` |
| 5.6 | `142b87102` (#15762) | 「Attach file」调色板命令 | `app/src/terminal/view.rs`、`view/action.rs`、`view/init.rs`、`input.rs`、`view_components/action_button.rs` | `ATTACH_FILE_KEYBINDING` 0 命中；本地**无** `file_attach_allowed_for_shared_session`（shared session 不适用） | 去掉 shared-session 门控（直接允许），其余按上游：`select_file()` 抽取、`attach_file()`、`ATTACH_FILE_KEYBINDING` 注册、action button 加 tooltip 快捷键 | `cargo check -p warp` + 手动触发调色板命令 |
| 5.7 | `d15645c77` (#15164, APP-5412) | `AgentSource::Orchestration` 变体 | `app/src/ai/ambient_agents/task.rs:148`（本地枚举 8 变体） | `AgentSource::Orchestration` 0 命中；纯加法 | 加变体 + `as_str()`/显示名/`from_str` 映射（本地枚举与上游不同，按本地现有分支穷尽补齐，禁用 `_` 通配）。价值低：仅服务端来源标记兼容 | `cargo check -p warp` |

### 批次 6 — 功能（中大成本，6 条，逐条独立评估是否值得做）

| # | 上游 | 内容 | 落点锚点（本地） | 本地前提证据 | 适配要点 | 风险 |
|---|---|---|---|---|---|---|
| 6.1 | `092c1dce9` (#13967) | 切换 markdown Rendered/Raw 时保持滚动位置 | `app/src/code/editor/{view,scroll}.rs`、`local_code_editor.rs`、`notebooks/file/mod.rs`、`pane_group/*`、`crates/editor/src/render/model/{viewport,mod}.rs` | `ScrollPosition::Fraction`、`scroll_to_fraction` 0 命中 | 新增 `Fraction(f32)` 变体 + `scroll_fraction()` / `scroll_to_fraction()` 并在 viewport 接线；9 文件 | 中：跨 code/notebook 两套视图 |
| 6.2 | `8b88df987` (#15221) + `40e397170` (#15496) | 按住修饰键显示 tab 切换快捷键提示（含随后的一处优先级修复） | `app/src/tab.rs`、`workspace/view/vertical_tabs.rs`、`workspace/action.rs`、`lib.rs` | `TabShortcutModifierState`、`TAB_ACTIVATE_BINDING_NAMES`、`TAB_SHORTCUT_HINT_REVEAL_DELAY` 全库 0 命中 → 两条必须**成对**做（单独揀 `40e397170` 是前置缺失） | 先揀 `8b88df987`（+366 行，8 文件，含 `TabShortcutModifierState` 单例 + 750ms 延迟），再揀 `40e397170`（`TAB_ACTIVATE_LAST_BINDING_NAME` + `tab_activate_binding_name()` 优先级） | 中高：落在本地自研 vertical tabs / tab 体系，冲突面大 |
| 6.3 | `c25ac4070` (#15365) + `18179177a` (#15392) | 右键行为设置（上下文菜单 / 粘贴）+ Shift+右键提示 | 17 文件：`settings/select.rs`、`settings_view/features_page.rs`、`terminal/{input,view,block_list_element}.rs`、`notebooks/*`、`env_vars/*`、`crates/warpui_core/src/elements/event_handler.rs`（本地无 `gui/` 前缀） | `right_click_pastes` / `RightClickBehavior` 全库 0 命中（两条成对，单独揀 `18179177a` 是前置缺失） | 新增 `RightClickBehavior` 设置 + 各视图右键分派 + `right_click_pastes()`；再揀提示文案 | 中高：跨 warpui_core 事件层 + 多视图，但**无云依赖**，属纯 GUI 偏好 |
| 6.4 | `b7ec0fc55` (#15605) | Agent Mode 用户查询加时间戳 | `app/src/ai/blocklist/block/{model,model/model_impl,view_impl,view_impl/query}.rs`、`util/time_format.rs` | `query_sent_at` 0 命中；`format_message_timestamp` 需先确认本地 `time_format.rs` 是否已有（**拣入前先 grep**） | model 记 `query_sent_at`，UI 加 `query_timestamp_tooltip_handle` + `CopyTimestamp` action；上游 `terminal/view/context_menu.rs` 本地无 → 该 hunk 改为适配本地菜单构造 | 中 |
| 6.5 | `5e7030db7` (#15323, APP-5532) | warping 行显示当前模型名 | `app/src/ai/blocklist/block/status_bar.rs`、`view_impl/common.rs` | `status_message_naming_model` 0 命中 | **必须剔除与 `warp_multi_agent_api` 解耦相关的 hunk**（本地无该 crate），只取 `status_message_naming_model` + `OutputModelInfo` 展示部分 | 中：上游该提交混了解耦改动 |
| 6.6 | `4b894db80` (#15455) | serde Content 缓冲改为 JSON-value 反序列化 | `app/src/ai/agent/mod.rs`、`app/src/ai/artifacts/mod.rs`、`terminal/model/ansi/dcs_hooks.rs` | 落点齐；改动含新增 `AIAgentContextTagged` 反序列化枚举 | **先确认语义**：是否为服务端载荷兼容性修复（可能是）；若是纯编译期优化则**不做** | 中：需先定性再决定 |

---

## 4. 明确不并入（53 条，依据见执行时写入的 `history.md §35`）

| 类别 | 条数 | 代表 / 依据 |
|---|---|---|
| 已含等效 | 3 | `3529ae637`（本地已有 `impl TypedActionView`）、`742ca57b5`（本地已走 `layout_text_uncached`）、`ff16a0b2a`（本地已有 FxHashMap 用法） |
| promote 无对象 | 4 | `77105899b` `0d57d4bb2` `33b59410b` `9c2879ab6` —— 本地无对应 Cargo feature（grep 0）/ 只在枚举定义没有实现 |
| Sentry / `warp_errors` 链 | 10 | `d2cb17abb` `27f8ee6c1` `04a7f8342` `d68a638ef` `d019ddfe9` `8ba01aa1a` `e0d01fff4` `78d1eabdf` `fb594d2c8` `86bcf038f` |
| `agent_sdk/driver/` 新模块本地不存在 | 11 | `b2bcc408d` `44357a02f` `d58f555a3` `db6ab7305` `1b4b13964` `4ab7ef99c` `391dd76ad` `e4857bd60` `60d602df6` `73bd01431` `f8aa4b98e` |
| native shell completions / widget handoff 整条未跟 | 4 | `bf2364bc9` `fc4d563b8` `7c360f772` `d5d12d90f` |
| 设置页 / onboarding 定制区 | 6 | `e1bcf5d07` `a18026275` `94daf47f3` `e054075b8` `c5e4a02e3` `7feb88b5e` |
| 依赖缺失符号（本地 0 命中） | 9 | `b9c21aa01` `4fa1a3c66` `76cfd2c17` `216d0efe7` `36dd2cc2e` `2a183d552` `efbf553ed` `6a96a72d8` `a9c0a1ebd`（本地 `project_context/model.rs` 已分叉，需按本地重写）|
| 纯重构 / 编译期 / 高风险 / pin bump | 6 | `21f413b79`（114 文件 +16909 行搬运）、`9d3f3e1ec`（`.clone()` 消除）、`dc1077845`（仅编译期）、`83b4c101e`（release 期 schema 生成）、`f42c4ab6c`（`TerminalModel` → `Arc<FairMutex>` 大改）、`83ddbefff`（command-signatures pin bump） |
| 合计 | **53** | = 89 − 36 |

> 表中「代表/依据」只列骨架；执行完成后按 §7 把**带证据的完整 89 行表**写入 `history.md §35`。

---

## 5. 验证与验收清单

### 5.1 每批必做

```bash
cargo check -p warp                       # 门槛（AGENTS.md §5.1），0 error
```

批 1 追加：`cargo test -p warp_editor` / `-p warpui_core` / `-p warp_terminal`（按 §3 表逐条）
批 2 追加：`bash -n app/assets/bundled/bootstrap/bash_body.sh`、`zsh -n app/assets/bundled/bootstrap/zsh_body.sh`
批 3 追加：`cargo tree -p warp -i cosmic-text`（确认 lock 变动范围）
批 4 追加：`cargo test -p warp_completer` / `-p repo_metadata` / `-p watcher`

### 5.2 全轮收尾

| 检查 | 期望 |
|---|---|
| `cargo check -p warp` | 0 error；warning 数与基线一致 |
| 品牌扫描 | 拣入 diff 中无用户可见 `Warp`/`Oz` 残留 |
| 空行扫描 | `grep -P '\n{3,}'` 改动文件中无新增连续空行 |
| 孤儿 feature 核验 | 批 3.1 删除后 `cargo check` 无 unused-feature 报错 |
| `CHANGELOG.md` | 记录本轮同步边界 |
| `specs/upstream-merge-lessons.md` 头部 | 更新同步边界为 `4143c09ff`（2026-09-11） |
| `specs/upstream-merge-history.md` | 新增 §35 完整记录（含 36 个提交落地 + 53 条不并入依据 + 教训） |

---

## 6. 风险与回滚

| 风险 | 触发条件 | 处置 |
|---|---|---|
| 依赖 lock 大面积变动 | 批 3.2 / 3.3 升级 pin 后 `Cargo.lock` 出现非目标 crate 变动 | 该条单独 commit 并可 `git revert`；变动过大则放弃该条 |
| 本地自研区被覆盖 | 批 6.2 / 6.3 落在 tab / vertical tabs / 右键分派 | 冲突一律取本地；若无法干净适配则该条记「本轮不做」 |
| 终端模型锁纪律 | 批 1.4 / 批 6.x 涉及 `TerminalModel` | 不新增 `model.lock()`；改动前确认调用栈无上层持锁（AGENTS §5.3） |
| bash/zsh 脚本行为回退 | 批 2.5 结构重组 | 保留本地 MSYS2 分支与 `WARP_HONOR_PS1` 组合矩阵，手工冒烟后再继续 |
| 删 feature 后编译失败 | 批 3.1 | 逐条删（12 条可分 2 个 commit），失败即 revert 单条 |
| 回滚整体 | 合回 main 前 | 直接弃 `upstream-sync-2026-09-11` 分支；合入后逐 commit `git revert`（本轮无 migration，无 `down.sql` 需求） |
| Retirement | — | 本轮不新增 owner / 契约 / schema；批 3.1 属删除孤儿声明（无兼容面），无退役遗留 |

---

## 7. 记录要求（执行完成后）

1. `specs/upstream-merge-history.md` 新增 **§35**：区间与漏斗（281 → 89 → 36）、逐条裁决表（89 行，带证据）、关键适配点、本轮教训。
   - 需写明**漏斗口径差异**：§35 原文记「111 待判」，其筛选脚本未落盘；本次重建得更严的筛选得 89，两者是包含关系。
2. `specs/upstream-merge-lessons.md` 头部：同步边界更新为 `4143c09ff`，并加本轮 ✅ 行。
3. `CHANGELOG.md`：同步边界记录。
4. 每条 commit message 保留 `-x` 溯源或「移植自 upstream `<sha>`(#PR)」字样。

---

## 8. 执行路线

```text
Execution Route:
- Decision: inline
- Evidence: 本会话 subagent / team_spawn 工具不可用（`cannot get property "agent" without inject`），无并行下发能力；
            且 34 项任务集中改同一批文件（Cargo.toml / Cargo.lock / bootstrap 脚本 / view.rs），写入域高度重叠，本就不适合并行
- Fallback: 若后续子代理能力恢复，批次 4（性能 4 条）与批次 5（功能 7 条）写入域互不重叠，可改 subagent-driven
- User confirmation required: yes —— 执行（cherry-pick / 改文件）尚需明确指令（history §17.7）
```

---

*文档版本：v1.0 · 状态：待批准 · 未执行任何拣入*
