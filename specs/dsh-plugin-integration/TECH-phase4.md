# 立项:阶段 4 — dsh UI 动作 + zap_open_in_editor

> 前置:阶段 0-3 桥功能已完成(6 commits,33/33 测试)。本文件为阶段 4 专项立项,
> 含前置调研结论、实施计划、风险与验收。需**专门预算 + 真实 Zap 运行环境**。

## 1. 背景与目标

阶段 0-3 已打通 Zap ⇄ dsh 桥:WS 连接、workspace 跟随、`zap_*` 文件/终端工具。
阶段 4 目标:让 dsh UI 内出现 **Zap 动作**(如"在 Zap 中打开此文件"),并补齐
`zap_open_in_editor` 工具(agent 可请求 Zap 打开文件)。

## 2. 前置调研结论(2026 已核实)

### 2.1 zap_open_in_editor — editor 深度集成(高复杂度)

两次 codegraph 调研 + 内置 grep,**未找到**"打开文件到编辑器"的简单程序化 API:

- `root_view::OpenPath { path }` 是**参数结构体**,无 dispatchable 打开文件 action
  (root_view 全局 action 仅 add_file_pane 等)
- 唯一入口:`util::file::open_file_path_with_editor(line, path, editor, ctx: &mut AppContext)`
  → 平台层打开。**需主线程 `AppContext`**
- 打开到编辑器本身是 Zap 核心 UI 流程:`CodeEditorModel` 创建 + buffer + 视图导航
  (`app/src/code/` 深层),非单点调用

**结论**:`zap_open_in_editor` 需:
1. 桥方法(独立 runtime)→ 全局暂存(复用 PENDING 模式)
2. 主线程消费点(有 `AppContext`)→ 调 `open_file_path_with_editor`
3. 工具注册(复用 zap-bridge `zapTool`)

关键难点在 **2**(AppContext 获取 + editor 集成),需专项深入。

### 2.2 webview UI 动作 — 需真实环境评估

TECH.md 前置条件:实际打开 dsh webview,检查 DOM 是否有稳定 hook 注入按钮。
**需真实 Zap 运行时**(启动 dsh pane 渲染 webview),无法自动化验证。

## 3. 范围

| 属于本专项 | 不属于 |
|---|---|
| `zap_open_in_editor` 工具(agent → Zap 打开文件) | 自研 dsh UI 渲染 |
| webview JS 注入 Zap 动作(若 DOM 评估通过) | 事件映射/深层 UI 重写 |
| 桥协议 `zap.open_in_editor` 方法 | — |

## 4. 子项拆分与实施顺序

### 子项 A:zap_open_in_editor(先做,独立可交付)

**实现**(复用阶段 0-3 模式):
1. `bridge.rs`:`zap.open_in_editor { path }` 方法 → `resolve_in_root` 校验 →
   全局暂存 `PENDING_OPEN_PATHS`(复用 `PENDING_EVENTS` 模式)→ 返回 `{ok:true}`
2. 主线程消费:在某 `AppContext` 可达处(如 `on_frame_drawn` 外层或 BridgeServer
   drain 时)读 `PENDING_OPEN_PATHS` → `util::file::open_file_path_with_editor(
   None, abs_path, None, ctx)`
3. `zap-bridge.ts`:注册 `zap_open_in_editor` 工具(复用 `zapTool`,参数 `path`)
4. 单测:路径校验、暂存、方法分发(不依赖真实 editor)

**前置调研(专项内)**:确认主线程消费点能否获得 `&mut AppContext`
(warpui `ModelContext` → `AppContext` 转换)。

**验收**:
1. agent 调 `zap_open_in_editor("src/main.rs")` → Zap 打开该文件到编辑器
2. 路径穿越(`..`/绝对)被拒(复用 `resolve_in_root`)
3. 无活动窗口/无 AppContext 时优雅降级(不崩)
4. 单测覆盖校验/分发

### 子项 B:webview UI 动作(依赖子项 A + DOM 评估)

**前置评估**(需真实环境):
1. 启动 Zap debug 版 → 开 dsh pane → 检查 webview DOM(工具结果卡片是否有稳定 hook)
2. 若 dsh UI 无稳定注入点 → **放弃 webview 注入**,降级方案:由 `zap_*` 工具
   结果承载动作(如 `zap_open_in_editor` 返回"已打开" + 结果卡片带跳转)

**实现**(若评估通过):
- webview JS 注入 + IPC(复用双光标修复的 JS→IPC 链路)
- dsh UI 挂"在 Zap 中打开"动作

**验收**(仅当评估通过):按钮出现、点击后 Zap 打开文件、焦点正确。

## 5. 风险与对策

| 风险 | 对策 |
|---|---|
| 主线程 `AppContext` 获取不直接 | 专项内先 spike 确认 warpui 上下文转换 |
| editor 打开流程深(CodeEditorModel/导航) | 复用 `open_file_path_with_editor`(平台层),不触碰 editor 内部 |
| dsh UI DOM 无稳定 hook | 评估先行;失败则降级为工具结果承载动作(子项 A 已覆盖) |
| dsh 版本漂移(UI/API) | 动作经桥协议,独立于 dsh 内部 API |

## 6. 资源需求

- **真实 Zap debug 版运行环境**(验证端到端 + DOM 评估)
- 专门 token/会话预算(editor/AppContext 集成调研 + 实现)

## 7. 待确认(专项启动时)

- 主线程消费点选型:on_frame_drawn 外层 vs BridgeServer drain(依 AppContext 可达性)
- 打开文件的 editor 选择:默认(open_file_layout 设置)vs 固定
- 若 UI 动作评估失败:确认降级方案接受
