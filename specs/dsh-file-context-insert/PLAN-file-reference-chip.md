# dsh「附加为上下文」复用 `@文件` 芯片样式 — 升级计划

> 演进自同目录 `PLAN.md`(初版:从零搭建"附加为上下文"链路,采用 textarea 裸路径文本注入)。
> 本计划解决初版遗留问题:**插入的是裸完整路径文本,不显示 dsh 手动 `@` 选文件时的"文件名"芯片样式**。
> 目标:让 zap 插入的文件在 dsh 输入框呈现为与手动 `@` 选文件完全一致的"文件名"芯片。
>
> 本次修订(v2):
> 1. **逐行研读了 [dsh-workspace-explorer](https://github.com/Jiyr0119/dsh-workspace-explorer) v0.6.0** 的插入实现(dock 槽位桥 + `setDraft`),完成方案对比 —— 结论:它的路线**做不出芯片**,不采纳;但其"相对路径引用"与"桥捕获"思想被吸收(见下文对比节)。
> 2. **全部契约改按本机实装的 dsh 0.1.2-rc.1 重新核实**。旧计划引用的 `~/.dsh/dsh-install/node_modules/.pnpm` 路径与 0.1.1-rc.2 行号已全部作废;当前包位于 `~/.dsh/profiles/node_modules/@deepseek-ai/`(符号链接指向 `/Applications/DSH Desktop.app/Contents/Resources/app.asar.unpacked/node_modules/@deepseek-ai/`)。

## Context(问题根因,已逐行核实)

- dsh 0.1.2-rc.1 的输入框是 **Lexical 编辑器**(`SessionInputShell` 构造:`editor = ys({namespace: "dsh-composer", nodes: [ReferenceChipNode, TextRefNode], ...})`),**不是 textarea/ProseMirror/CodeMirror**。初版的 DOM 选择器回退(`textarea[data-testid="dsh-input"]` / `div.ProseMirror` / `.cm-content`)基本落空,只剩通用 `div[contenteditable="true"][role="textbox"]` 可能撞中 Lexical 根节点。
- 手动 `@` 选文件时,`dsh-client-ui-reference` 的 `onPick` 产出结构化引用对象
  `{ source:"reference", ref:"@<mention>", label:"<basename>", appearance:"file", clipboardText:"@<mention>" }`
  → 经 scoped 事件 `slash/input-insert-reference` → `SessionInputShell.insertReference(ref, span)`(client.js:11844)
  → Lexical 事务 `$replaceDetectSpanWithNodes(span, [$createReferenceChipNode(ref), ...])` 渲染成芯片
  (`span[data-composer-chip="reference"]`,非内联、不可编辑,显示 `label`;芯片的 `clipboardText` 携带完整 `@path`,发送时按 clipboardText 投影序列化)。
  **芯片是 Lexical 节点状态;纯文本插入永远变不成芯片**(`insertText` 的注释明说:"no chip node; the chip look is a scan-derived decoration, never state")。
- zap 当前 `Workspace::insert_path_into_dsh_input`(`app/src/workspace/view.rs:7308`)用
  `evaluate_script_on` 把**裸路径字符串**塞进 DOM 控件 → 落在 Lexical 里只是纯文本,无芯片、且选择器命中不可靠。
- 正确入口:插件侧拿到会话的 `SessionInputShell`,调 `shell.insertReference(ref, span)`。该 shell 可从根服务
  `conversation`(ConversationController,`extends Service(ctx, "conversation")`,client.js:1884)的 `.input`(InputHub)用
  `shell(sessionId)` 直达(client.js:12335),**无需 session scope ctx**(比旧计划的 `scopeCtx.get("conversation.input")` 更少一跳且已核实)。

## 参考项目对比:dsh-workspace-explorer 是怎么做的

来源:`dynamic/client.js`(动态免构建版,与 `src/client/index.tsx` 逻辑一致),以下行号以 v0.6.0 的 `dynamic/client.js` 为准。

| 环节 | 它的实现 | zap 计划的实现 |
|------|---------|---------------|
| 捕获输入能力 | `slots.inject('conversation.input.dock', ...)` 注册 `DockBridge` React 组件(1105-1108),从槽位 props 拿 `inputActions` + `useInput`,存进模块级 `bridge`(430-433、562-579) | 插件已有 cordis ctx,直接 `ctx.get("conversation").input.shell(sessionId)` 拿 shell(更强能力,见下) |
| 插入动作 | `actions.setDraft(draft + sep + text)`(568-576)—— **整草稿替换** | `shell.insertReference(reference, span)` —— Lexical 事务内替换 span 为芯片节点 |
| 插入物 | `[file: 相对路径]` **纯文本标记**(704:`markerFor`),靠模型自己解析;目录插 ASCII 树(445);还有"插入内容"把 ≤32KB 文件全文内联(727-735) | `@mention` 结构化引用 → **真芯片**,发送语义与手动 `@` 完全一致 |
| 定位 | 只会追加到草稿末尾(sep 为 `\n`) | `shell.caretSpan()`:有选区插在选区,无选区追加文末(检测坐标) |
| 路径形态 | 相对 workspace root(`entry.rel`),可配置绝对 | 同样相对化(见改动 1),与 dsh 官方 `@` 语法对齐 |
| 触发来源 | 插件自己 UI 里的点击/拖拽/多选 | zap 原生侧文件浏览器按钮 → Rust `evaluate_script_on` |

**为什么不采纳它的路线(三个硬伤)**:

1. **`inputActions` 没有芯片能力**。dock 槽位 props 由 `uiSession.provide` 注入(client.js:16041-16056),
   `props: { inputActions: shell.actions }`,而 `shell.actions` 只有
   `setDraft / addImages / removeImage / pruneImages / submit` 五个方法(client.js:11494-11509)。
   参考项目拿到的最高权限就是 `setDraft` —— 它不是"选择了不做芯片",是**做不了**芯片。
2. **`setDraft` 会摧毁已有芯片**。`setDraft` 的实现是 `root.clear()` 后按行重插纯文本(client.js:11627-11644),
   还会剥离引用占位符(`REFERENCE_PLACEHOLDER_RE = /[\uE100-\uE11D\uFFFC]/gu`,client.js:11479)。
   即:用户草稿里已有的 `@` 芯片,被参考项目插入一次后就降级成纯文本。zap 若走这条路,反而是功能退化。
3. **dock 桥依赖 React 渲染时机**。bridge 只有在 dsh 渲染出输入框 dock 槽位后才被捕获;
   而 zap 的插入由原生按钮异步触发,多一层"桥是否已就绪"的不确定态。

**吸收的两个点**:

- **相对路径**:它的引用全部是 workspace-root 相对路径。已核实 dsh 官方语义一致:
  `FILE_REFERENCE_PROMPT`(`@deepseek-ai/dsh-file-reference/lib/index.js:52`)明说
  "Tokens prefixed with @ are workspace paths the user explicitly referenced, **relative to the workspace root**";
  `@` 菜单候选由 `LocalFileReferenceService.scanWorkspace` 相对扫描产出,且 `resolveDisplayDirectory`
  **拒绝绝对路径钻取**(dsh-file-reference-local/lib/index.js:204-211)。zap 应把绝对路径相对化到当前会话 cwd(改动 1)。
- **返回值驱动的回退**:它的插入全部经 `getBridge()` 判空。zap 的 Rust 侧脚本同样按
  `window.__zapInsertFileReference(p, isDir)` 的**布尔返回值**决定是否回退到旧 DOM 注入(改动 2),
  修掉旧计划"函数内部吞错后既无芯片也无文本"的空洞。

## 方案(已与用户确认:整合进现有插件,不新建,插件列表保持 1 行)

只改 **2 个文件**,无新增插件 / 无新增 loader entry。

### 改动 1 — `app/assets/bundled/dsh/zap-bridge-client.js`(browser 插件)

在 `apply(ctx)` 内、`sessionsRef = sessions;` 之后新增一段:暴露 `window.__zapInsertFileReference(fullPath, isDir)`,
返回 `true` 表示芯片已插入(或重试中),`false` 表示能力不可用(调用方回退旧文本注入)。

```js
function installFileReferenceInjection(rootCtx, sessions) {
  if (window.__zapInsertFileReference) return;
  // dsh-file-reference formatFileMention 的等价移植(@deepseek-ai/dsh-file-reference/lib/index.js:39):
  // 含引号/控制字符 → 无法表示;含空白 → @"path";否则 @path。
  function mentionFor(clean, isDir) {
    const path = isDir ? clean + "/" : clean;
    if (/[\u0000-\u001f\u007f-\u009f"]/.test(path)) return null;
    if (!/\s/.test(path)) return "@" + path;
    return isDir ? '@"' + path : '@"' + path + '"';
  }
  function relativeUnder(dir, fullPath) {
    if (!dir) return undefined;
    const norm = (s) => String(s).replace(/\\/g, "/").replace(/\/+$/, "");
    const d = norm(dir);
    const f = norm(fullPath);
    return f === d ? undefined : (f.startsWith(d + "/") ? f.slice(d.length + 1) : undefined);
  }
  window.__zapInsertFileReference = function (fullPath, isDir) {
    try {
      const snap = sessions.list.getSnapshot();
      if (snap.phase !== "ready" || !snap.current) return false;
      const sessionId = snap.current;
      const conversation = rootCtx.get("conversation");
      const input = conversation && conversation.input;
      if (!input) return false;
      const shell = input.shell(sessionId); // binding 缺失时 throw → 落入 catch
      // 相对路径优先:@ 引用的规范形态即 workspace-root 相对(dsh FILE_REFERENCE_PROMPT);
      // 文件不在会话 cwd 下时保留绝对路径(模型仍可 read)。
      const cwd = snap.byId[sessionId] && snap.byId[sessionId].cwd;
      const clean = relativeUnder(cwd, fullPath) || String(fullPath).replace(/[\\/]+$/, "");
      const mention = mentionFor(clean, isDir === true);
      if (!mention) return false;
      const label = (clean.split(/[\\/]/).pop() || clean) + (isDir === true ? "/" : "");
      const reference = {
        source: "reference",
        ref: mention,
        label: label,
        appearance: isDir === true ? "folder" : "file",
        clipboardText: mention
      };
      // insertReference 要求 phase ∈ {plain, claimed} 且 span.draftRev === shell.rev。
      // span 必须是 detect 坐标(shell.caretSpan 的产物),不能拿 compose().draft(clipboard 坐标)当 span。
      // false ⇒ 提交窗口期(phase busy),逐帧重试若干次;放弃仅 console.warn。
      let tries = 0;
      const attempt = () => {
        try {
          const st = shell.compose();
          const span = shell.caretSpan();
          span.draftRev = st.draftRev;
          if (shell.insertReference(reference, span)) return true;
        } catch (err) {
          console.error("[zap-bridge-client] insertFileReference failed:", err);
          return false;
        }
        if (++tries > 8) {
          console.warn("[zap-bridge-client] insertFileReference: composer busy, giving up");
          return false;
        }
        requestAnimationFrame(attempt);
        return true; // 重试中:对外仍视为已受理,不触发 Rust 回退
      };
      return attempt();
    } catch (err) {
      console.error("[zap-bridge-client] insertFileReference failed:", err);
      return false;
    }
  };
}
installFileReferenceInjection(ctx, sessions);
```

并在 `ctx.effect` 的清理回调中补一行 `delete window.__zapInsertFileReference;`
(与既有 `delete window.__onZapResponse` 对称)。

**已核实的契约依据**(全部来自本机实装 dsh **0.1.2-rc.1**,`~/.dsh/profiles/node_modules/@deepseek-ai/`):

- `ConversationController extends Service(ctx, "conversation")`(dsh-client-ui-conversation/lib/client.js:1884),
  由 `ctx.plugin(ConversationController, { input: inputHub, blocks: composerBlocks })` 注册(client.js:16266-16269)。
  → 根 ctx 上 `get("conversation")` 得服务实例,`.input` 即 InputHub。懒解析(调用时 get)规避插件加载顺序问题;
    插件 ctx 的懒 `get` 是 dsh 自家模式(InputHub 构造即持 rootCtx 按需 `get("sessions")`)。
- `InputHub.shell(id)`(client.js:12335):按 id 直取/现建 shell;`sessions().binding(id)` 缺失时 throw
  (12339-12341)。会话 ready 时 binding 必在。
- `SessionInputShell.insertReference(ref, span)`(client.js:11844-11856):
  - phase 守卫:`phase !== "plain" && phase !== "claimed"` → 返回 false;
  - CAS:`span.draftRev !== this.rev` → 返回 false;`compose().draftRev` 即 `this.rev`(12235-12247),同步调用无竞态;
  - span 为 **detect 坐标**(11848 用 `this.projection.detectText.slice` 做尾空格判断);
  - 应用:`$replaceDetectSpanWithNodes(span, [$createReferenceChipNode(ref)] + 尾随空格)`;
  - 返回 applied 布尔 → 驱动重试。
- `shell.caretSpan()`(client.js:11787-11796):有选区返回选区 span,否则返回文末 collapsed span
  `{start: detectText.length, end: detectText.length}` —— **span 的唯一正确来源**。
  旧计划用 `compose().draft.length` 当 span 是错的:`draft` 是 clipboard 投影,含芯片时比 detect 坐标长
  (芯片在 detect 里占 1 字符,在 clipboard 里展开为完整 `@path`,见 `$composerLayout` 11240-11267 与
  `detectOffsetOfClipboardOffset` 11271-11286)。
- 芯片节点 `ReferenceChipNode`:构造吃 `{source, ref, label, appearance, clipboardText}`(client.js:10800-10810);
  DOM 为 `<span data-composer-chip="reference" contenteditable="false">`(10836-10842);
  `getTextContent()` 即 clipboardText(10854-10856)。
- 手动 `@` 的 onPick 构造(dsh-client-ui-reference/lib/client.js:126-152):
  `{source:"reference", ref: value.mention, label, appearance: "file"|"folder", clipboardText: value.mention}`;
  目录 label 为 `basename/`(138),appearance 为 `"folder"`。
- mention 语法(dsh-file-reference/lib/index.js:39-55 `formatFileMention`):
  含 `"` 或控制字符(`\u0000-\u001f`、`\u007f-\u009f`)→ 不可表示;含空白 → `@"path"`;
  否则 `@path`;目录带尾斜杠(目录 + 空格时 dsh 自身产出不闭合的 `@"dir/`,为保持"与手动 @ 一致"照抄)。
- 会话 cwd:`sessions.list.getSnapshot().byId[id].cwd`(zap-bridge-client.js 现有 `currentWorkspacePath` 已在用,
  即 dsh workspace root,相对化的基准)。
- `setInvalid`(10861)在全仓 client 包内无调用方 → 芯片不会因路径形态渲染成 invalid,绝对路径兜底无视觉风险。
- 事件等价路径 `slash/input-insert-reference`(12315)与 session scope 入口 `actx.get("conversation").input.for(actx)`
  (queue dock 用法,13908-13912)存在但不再采用:根 ctx 直取更少一跳。

### 改动 2 — `app/src/workspace/view.rs` 的 `insert_path_into_dsh_input`(约 7308 行)

把当前"往 DOM 控件塞裸文本"的 JS,改为**优先调用 `window.__zapInsertFileReference`,按布尔返回值回退旧文本注入**;
顺带把 `path.is_dir()` 传给浏览器端以产出 folder 芯片:

```rust
let is_dir = if path.is_dir() { "true" } else { "false" };

// JS 策略:优先走插件的结构化芯片插入;返回 false(插件未就绪/不可表示)时回退纯文本注入
let js = format!(
    r#"
    (function() {{
        var p = {escaped};
        var isDir = {is_dir};
        if (window.__zapInsertFileReference && window.__zapInsertFileReference(p, isDir)) {{
            return;
        }}
        // 回退:插件能力未就绪时,插入 @路径 纯文本(保持旧行为)
        var el = document.querySelector('textarea[data-testid="dsh-input"]')
                 || document.querySelector('textarea[placeholder]')
                 || document.querySelector('div[contenteditable="true"][role="textbox"]')
                 || document.querySelector('div.ProseMirror')
                 || document.querySelector('.cm-content[contenteditable]');
        if (!el) {{ console.warn('[dsh] No input element found for attach_as_context'); return; }}
        if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {{
            el.focus();
            var start = el.selectionStart || el.value.length;
            var end = el.selectionEnd || el.value.length;
            el.setRangeText('@' + p, start, end, 'end');
            el.dispatchEvent(new InputEvent('input', {{ bubbles: true, cancelable: true }}));
        }} else {{
            el.focus();
            document.execCommand('insertText', false, '@' + p);
        }}
    }})()
    "#
);
```

> Rust 注意:`@` 在格式化/原始字符串中敏感,必须用 `'@' + p` 字面拼接,不得写 `@{escaped}`;
> 回退分支的 JS 保持原文不动(它就是今天的线上行为,只作为插件未就绪时的兜底)。

## 不在范围内 / 明确排除

- **不新建 dsh 插件**:避免插件列表从 1 行变 2 行
  (`host-plugin-inventory` 的 `list()` 遍历 `loader.entries()`,新增 loader entry 即多一行)。
- **不采用 dsh-workspace-explorer 的 dock 槽位桥方案**:其 `inputActions` 无芯片能力,`setDraft` 重建会丢已有芯片
  (见对比节)。其"点按钮插入文件"的入口形态与 zap 原生文件浏览器重复,不需要移植。
- **不改 dsh 核心包**(不动 `~/.dsh/profiles` 下 npm 包)。
- **不改「附加为上下文」的触发入口**(`pane_group::Event::AttachPathAsContext`、`left_panel.rs`、
  文件浏览器按钮逻辑均不动);只改"插入 dsh 输入框"这一末端。
- **不改变语义**:插入仍是 dsh `@文件` 路径引用,模型按需 `read`,与手动 `@` 完全一致
  (非把文件内容内联 —— 参考项目"插入内容"整文件内联的模式**不**采纳)。

## 验证

1. **构建/类型**:`cargo check`(仓库 AGENTS.md 约定,PR 前只需通过 `cargo check`)。
2. **功能手测**(需 dsh 0.1.2-rc.1 运行时;芯片可用 DOM 断言 `document.querySelectorAll('span[data-composer-chip="reference"]')`):
   - 在 zap 侧边栏项目行点文件浏览器 → 选文件「附加为上下文」→ dsh 输入框出现**文件名芯片**
     (图标 + `code_review_view.rs`),与手动输入 `@` 选文件视觉一致;existing 芯片不被破坏(混插两个文件验证)。
   - **相对路径**:文件在当前会话 cwd 下 → 芯片 clipboardText 为 `@app/src/...`(无前导 `/`);
     发送后模型收到的 mention 与手动 `@` 选同一文件逐字节一致。
   - **跨项目**:文件不在会话 cwd 下 → 芯片仍出现,clipboardText 为绝对路径,模型可 `read`。
   - 含空格路径(如 `my file.rs`):clipboardText 为 `@"…/my file.rs"`;目录:appearance 为 folder、label 带 `/`。
   - dsh 未就绪(极早启动期,`window.__zapInsertFileReference` 不存在或返回 false):回退为旧「插 `@路径` 文本」行为,不丢功能。
   - composer 忙碌期(正在提交)插入:逐帧重试后芯片落地,console 无 give-up 警告。
   - 插件设置列表仍只有 `zap-bridge-client` 一行,无新增条目。
3. **回归**:确认现有 `switch_project` / `open_file_explorer` / 终态 `notify` / 文件浏览器按钮行为不受影响
   (仅新增一个 `window` 全局函数 + 调用,无副作用;清理回调同步删除该全局)。

## 风险与未决

- `ctx.get("conversation")` / `input.shell(id)` / `insertReference` / `caretSpan` 是 dsh 内部 cordis API(非公开类型)。
  已逐行从 0.1.2-rc.1 源码确认;若 dsh 升级改名需同步(本计划所有锚点都给了函数名,便于 grep 重定位)。
  现有 `zap-bridge-client` 已在用 dsh 内部 `sessions`/`workspaces` 快照结构,性质一致。
- 任一环节失败(服务未就绪、binding 缺失、路径不可表示、重试耗尽)最终都收敛为:返回 false → Rust 回退旧文本注入,
  或 console 告警;**zap 不崩、dsh 输入框不脏**。
- 重试耗尽(8 帧 ≈ 130ms)只在"提交窗口期恰好覆盖全部重试"这种极端时序下发生,后果是本次不插入(与旧版在该时序下的
  行为一致或更好);如手测发现实际可复现,再评估加长重试窗。
- Windows 路径(反斜杠、盘符)未验证:相对化工具已把 `\` 归一为 `/`,但 dsh 在 Windows 上的 mention 语法未经手测;
  首个 Windows 打包前补测。
- `caretSpan` 在用户先前于输入框内留有选区时会**替换该选区**(与粘贴语义一致,手动 `@` 同样如此),属预期行为。

## Critical files & anchors

- `app/assets/bundled/dsh/zap-bridge-client.js` — browser 插件,新增 `window.__zapInsertFileReference(fullPath, isDir)`
  (整合进现有 `apply`,清理回调同步删除全局)。
- `app/src/workspace/view.rs:7308` — `insert_path_into_dsh_input`,改 JS 注入逻辑(优先调全局函数并按返回值回退,
  传 `is_dir`)。
- 参考项目源码:<https://github.com/Jiyr0119/dsh-workspace-explorer> v0.6.0
  - `dynamic/client.js:562-579` DockBridge(捕获 inputActions)、`:568-576` `setDraft(draft + sep + text)`;
    `:1105-1108` dock 槽位注册;`:704-709` `markerFor`/`insertMarker`(`[file: …]` 纯文本);`:445-469` 目录树文本。
  - `src/client/index.tsx` — 同逻辑 TS 源。
- dsh 源码(本机实装 0.1.2-rc.1,`~/.dsh/profiles/node_modules/@deepseek-ai/`,符号链接指向 DSH Desktop.app):
  - `dsh-client-ui-conversation/lib/client.js` — `ConversationController`/"conversation" 服务(1884)、
    `REFERENCE_PLACEHOLDER_RE`(11479)、`SessionInputShell.actions` 五方法(11494-11509)、`setDraft`(11627)、
    `caretSpan`(11787)、`insertReference`(11844)、`ReferenceChipNode`(10760-10870)、`$projectComposer`/
    detect-vs-clipboard 投影(11240-11360)、`InputHub.shell(id)`(12335)、`slash/input-insert-reference`(12315)、
    `compose()`(12235)、dock 槽位 provide 与配置(16041-16056、16077-16081)、
    `ctx.plugin(ConversationController, {input})`(16266-16269)。
  - `dsh-client-ui-reference/lib/client.js` — 手动 `@` 的 `onPick` 引用对象构造(126-152)。
  - `dsh-file-reference/lib/index.js` — `formatFileMention` 引号语法(39-55)、`FILE_REFERENCE_PROMPT`
    相对路径语义(52)。
  - `dsh-file-reference-local/lib/index.js` — `@` 候选为 root 相对扫描(148-171)、`resolveDisplayDirectory`
    拒绝绝对路径(204-211)。
