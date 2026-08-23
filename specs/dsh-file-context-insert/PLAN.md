# dsh 文件浏览器「附加为上下文」插入 dsh 输入框 — 执行计划

## Context
用户在文件浏览器右键某文件选「附加为上下文」时，当前若为 dsh pane（DeepSeek Harness Web UI），需把路径像终端那样插入 dsh 输入框，而非终端 input。现有链路：`FileTreeView::attach_as_context` → `FileTreeEvent::AttachAsContext(relative_path)` → `LeftPanelView → pane_group::Event::AttachPathAsContext` → `Workspace::attach_path_as_context` → `active_session_view → TerminalView::attach_path_as_context`（append_to_buffer / rich input / PTY）。dsh pane 场景下该链路直接 `log::warn`“No active terminal”或误插终端，无 dsh 分支。目标：复用同一右键菜单入口，按焦点路由：dsh pane 聚焦时走 dsh 输入框，否则走终端原逻辑。

## Approach
### 步骤 1 — 工作区路由分流（唯一行为分叉点）
**目标**：`Workspace::attach_path_as_context` 内先判 dsh 是否为插入目标，命中则走 dsh， miss 则保持终端原逻辑。

- **位置**：`app/src/workspace/view.rs` 的 `fn attach_path_as_context(&mut self, path: PathBuf, ctx: &mut ViewContext<Self>)`（现 7237 行附近）。
- **判定**：复用 `self.active_tab_pane_group().as_ref(ctx).dsh_panes()` 与焦点判断。已有 `PaneGroup::dsh_panes()`（`app/src/pane_group/mod.rs:2065`），`DshPane::is_loading()` 与 `DshPaneView::get_browser_view() → Option<ViewHandle<BrowserPaneView>>`（`app/src/dsh/pane.rs:180`）以及 `BrowserWebViewManager::evaluate_script_on` 存在。焦点判定复用终端同款：`ctx.focused_view_id(ctx.window_id())` 对比 pane view id，或 `PaneGroup::focused_pane_id(ctx)` 判 panes 包含 DshPane 之一为 focused。优先用 `focused_view_id`，与 `TerminalView::attach_path_as_context` 焦点路由一致，避免 update 回调内 upgrade 失效问题。
- **逻辑**：
  ```
  if FeatureFlag::DshPane.is_enabled() && dsh_pane_focused_or_active(ctx) && dsh_ready {
    self.insert_path_into_dsh_input(&path, ctx); return;
  }
  // 原终端分支不变
  ```
- **新辅助**：同文件新增 `fn insert_path_into_dsh_input(&self, path: &Path, ctx: &mut ViewContext<Self>)` 或 `&mut self`（视是否需 `ctx.notify`），内部取 `BrowserPaneView` 的 `model().platform_view_id`，调 `BrowserWebViewManager::handle(ctx).as_ref(ctx).evaluate_script_on(id, &js)`。无 ready webview 时 `log::warn` 并 return，不回退终端（避免误插）。
- **无 ready/无 pane**：直接回退原终端分支，保持现有 `active_session_view`  warn 行为。

### 步骤 2 — dsh 输入框 JS 注入（React 受控兼容）
**目标**：经 `evaluate_script_on` 在 dsh webview 内把 `path.to_string_lossy()` 文本插入当前会话的输入框并触发受控更新。

- **位置**：JS 字符串在 `workspace/view.rs` 内 `insert_path_into_dsh_input` 中构造，或抽 `app/src/dsh/pane.rs` 常量复用。执行入口 `BrowserWebViewManager::evaluate_script_on`（`app/src/browser/browser_web_view.rs:421`）已存在，无需新增 Rust API。
- **选择器策略（按优先级，unverified — 需首帧运行时用浏览器 devtools 确认）**：
  1) `textarea[data-testid="dsh-input"]` / `textarea[placeholder]`（dsh 主输入）
  2) `div[contenteditable="true"][role="textbox"]`（若为 Lexical/CodeMirror）
  3) `div.ProseMirror, .cm-content[contenteditable]` 回退
- **插入手法**：命中 `HTMLTextAreaElement | HTMLInputElement` 时 `focus(); setRangeText/插入 at selectionStart; dispatchEvent(new InputEvent('input', {bubbles:true}))`; 命中 `contenteditable` 时 `document.execCommand('insertText', false, text)` 或 `Selection + Range + dispatch InputEvent`。末尾 `focus_webview` 已在 `DshPaneView` 管理，无需额外 focus 调用，但插入后补 `BrowserWebViewManager::focus_webview(id)` 确保光标归位。
- **转义**：`shell-escape` 不适用，路径原样插入；`JS` 侧用 `JSON.stringify(path)` 方式安全转义引号与换行，避免拼接 XSS/截断。Rust 侧 `format!("({})({})", js_fn, serde_json::to_string(&content).unwrap())`。
- **空/异常**：`path` 为空不注入；`evaluate_script_on` 失败仅 `log::warn`。

### 步骤 3 — 保持文件树侧零改动
- `app/src/code/file_tree/view.rs:2450 attach_as_context` 保持发射 `relative_path`（`relative_path_for_item` 2084 已 strip repo root），不改菜单可见性 `has_terminal_session` 门控。dsh 场景下 `has_terminal_session==false` 会导致菜单项不显示——需同步放宽。
- **菜单门控**：`app/src/code/file_tree/view.rs:2392 has_terminal_session` 守卫使无终端时「附加为上下文」不渲染。dsh 单独打开时应仍可见。改判定为 `has_terminal_session || has_dsh_pane`。获取 `has_dsh_pane` 经 `FeatureFlag::DshPane.is_enabled()` 且 `ctx` 可访问的 `Workspace`/`PaneGroup` 状态；最简不跨层：直接放宽为恒显示（终端/dsh 任一存在即可，而右键菜单已在 Workspace 聚合，去掉门控副作用最小）。决策：**移除该门控或改为 `if true` 展示**，路由失败时仅 warn，原终端无会话场景本就 warn。

### 步骤 4 — 规避 rival 模式
- 不新增 `BridgeEvent` / `zap.insert_context` RPC：`evaluate_script_on` 已满足单向插入，无需改 `app/src/dsh/bridge.rs`、`app/assets/bundled/dsh/zap-bridge-client.js` 或 `~/.dsh/profiles/web/node_modules/dsh-at-file`。避免新增往返与 client 插件发布。
- 不引入 `DshRuntime` 中间态：复用 `PaneView<DshPaneView>` → `BrowserPaneView` 现有链路，不经 `DshRuntime::WORKSPACE_DIR`。

## Critical files & anchors
- `app/src/workspace/view.rs:7237` — `attach_path_as_context` 路由分流与新增 `insert_path_into_dsh_input`，唯一行为分叉点
- `app/src/terminal/view.rs:4963` — `TerminalView::attach_path_as_context` 参考实现（焦点/RichInput 分流、append_to_buffer）
- `app/src/code/file_tree/view.rs:2450` 与 `2392` — `attach_as_context` 发射点与菜单 `has_terminal_session` 门控
- `app/src/dsh/pane.rs:180` — `get_browser_view()` 与 `DshPaneView::is_loading` 判 ready
- `app/src/browser/browser_web_view.rs:421` — `evaluate_script_on(id, script)` 执行入口与 `focus_webview` 辅助

## Verification
- **cargo check**：`cargo check`（AGENTS.md 约定，提 PR 前唯一必需）必须绿；关注 `attach_path_as_context` 分支新增未破坏终端原路径。
- **手工端到端（需 `cargo run --bin zap-oss` 启动）**：
  1) 单 tab 仅 dsh pane：在文件浏览器右键文件 → 附加为上下文 → dsh 输入框末尾出现相对路径（`relative_path_for_item`），光标在路径后，无终端时不 warn 误插。
  2) dsh + 终端共存，焦点在终端：同操作应插入终端 input（`TerminalView` 原行为），焦点不在 rich input 时走 input buffer。
  3) dsh + 终端共存，焦点在 dsh（点击 dsh webview 输入框）：同操作应插入 dsh 输入框，而非终端；dsh 输入框内容受控更新（键入可继续）。
  4) 无 dsh pane：行为与现状一致，插入终端或 warn。
- **边界**：路径含空格/引号/中文/非 UTF-8（`to_string_lossy`）插入后在 dsh 输入框显示正确；连续两次插入叠加无覆盖。
- **回归**：原有 `terminal/view_test.rs:7170 attach_path_as_context_routes_*` 三用例仍绿（仅 Workspace 路由改，不动 TerminalView）。

## Assumptions & contingencies
- **假设**：dsh Web UI 输入框为 `textarea` 或 `contenteditable` 且可经 `evaluate_script_on` 访问；若实际为 Shadow DOM/Canvas 输入，`querySelector` 命中失败则插入无效果。Contingency：首验失败时改走 `zap-bridge-client.js` 新增 `zap.insert_context` IPC，由插件在 React 侧用 dsh 官方 `dsh-client-ui-input-trigger` API（`@deepseek-ai/dsh-client-ui-input-trigger`）插入，届时再补 `bridge.rs` 分支与 client.js `inject`。
- **假设**：聚焦 dsh 时 `focused_view_id` 可区分。Contingency：若 wkwebview 内焦点不映射到 Warp `focused_view_id`，改用 `PaneGroup::focused_pane_id` + `panes_of::<DshPane>` 比对，仍无则退化为“有且仅有 dsh pane 时即走 dsh”。
- **假设**：`relative_path` 为期望插入内容。若 dsh 需绝对路径或 `@path` 触发 `@file` 引用，Contingency：改 `path` 为 `item.path().to_local_path_lossy()` 绝对路径，或前缀 `@`，由 QA 定。
