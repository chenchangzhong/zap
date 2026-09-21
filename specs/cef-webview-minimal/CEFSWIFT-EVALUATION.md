# 集成 CefSwift vs 直接集成 CEF/Chromium 评估

> 结论:**不建议整体集成 CefSwift**。它和 zap 抢同一批"进程级独占资源"(CEF 初始化、消息泵、
> helper 布局、bundle 组装、CEF 版本),整体集成等于**替换**掉我们已验证的 Rust 侧 CEF 层,
> 而且它把 Swift/SwiftUI 引入构建链。
> **建议把它当 OSR 的参考实现**,把它的手法 port 到我们现有的 Rust `cef_backend`。
> 另外:它的"透明"同样只在 **OSR(windowless)** 模式下成立,windowed 模式它也和我们一样只能填色。

## 1 取证(读它的源码/文档,非推测)

来源:`Rajaniraiyn/CefSwift`(BSD-3,Swift/SwiftPM,macOS 14+,CEF 148+ 由插件下载)。

### 1.1 三种宿主模式(README/architecture.md)
| 模式 | 底层 | 说明 |
|------|------|------|
| `CefWebView` | 原生 NSView + **Alloy** + `parent_view` | 与我们现在的实现同构;文档明说"CEF 绘制该区域,**无法**在同一视图内把原生 UI 叠在页面上" |
| `CefChromeWindow` | CEF 自建顶层窗口(Chrome runtime) | 需要 CEF 拥有窗口;与 zap 的窗口模型冲突 |
| `CefMetalWebView` | **OSR**:`on_accelerated_paint` → `IOSurface` → `CALayer.contents` | "indistinguishable embedded web view";原生 UI 可叠在网页之上 |

### 1.2 windowed 模式的背景处理 = 与我们完全相同(`CefBrowser.createBrowser`)
```swift
var browserSettings = cef_browser_settings_t()
if let color = options.backgroundColor?.usingColorSpace(.sRGB) {
    browserSettings.background_color = cefColorFromNSColor(color)   // 填色,不是透明
}
```
文档对该选项的定位:"Paint color before the page renders (**kills the white flash** in dark UIs)"
—— 与我们的 `appearance::window_surface_color` 填色方案是同一个思路。**windowed 下它也没有透明**。

### 1.3 它的"透明"来自 OSR(`CefBrowser.createOSRBrowser`)
```swift
windowInfo.windowless_rendering_enabled = 1
windowInfo.shared_texture_enabled    = 1
windowInfo.external_begin_frame_enabled = 1      // 由 CADisplayLink 驱动,vsync 节奏
browserSettings.windowless_frame_rate = 60
windowInfo.runtime_style = CEF_RUNTIME_STYLE_ALLOY
browserSettings.background_color = …(可传 alpha=0)
```
⇒ 与 CEF 头文件一致:**windowless + 透明 alpha 才启用透明绘制**(`cef_types.h:701-708`),
IOSurface 带 alpha,`CALayer` 合成后自然透出下层。这正是我们评估里 **方案 D 的"路径 1"**。

### 1.4 它为 OSR 手工接的全部原生能力(即方案 D 的风险清单,已被它逐项验证)
鼠标 move/click/wheel、键盘(含 mac→Windows VK 映射、KEYDOWN+CHAR 两段式)、焦点、
`was_resized`/`notify_screen_info_changed`/`was_hidden`/`invalidate`/`send_external_begin_frame`、
**IME(`NSTextInputClient` ↔ `ime_set_composition`/`ime_commit_text`/`ime_finish_composing`/`ime_cancel_composition`)**、
编辑命令(copy/cut/paste/selectAll/undo/redo 走 focused frame)、右键菜单(**异步 NSMenu**,
注释明确"回调里跑 modal loop 会崩 CEF")、手势(zoom/swipe)、拖放双向、无障碍
(`set_accessibility_state` + `cef_accessibility_handler_t` 桥成 `NSAccessibilityElement`)、
`<select>` 弹层用独立 `popupLayer`。

### 1.5 进程/打包形态
`cef-helper` 单一二进制 × 5 个 helper.app;SwiftPM 命令插件下载 CEF 并"inside-out"签名;
`windowlessRenderingEnabled` 是**进程级**且必须在 `CefRuntime.initialize` 前设置。

## 2 维度对比

| 维度 | 直接集成 CEF(现状:cef-rs + Rust) | 整体集成 CefSwift |
|------|-----------------------------------|-------------------|
| 语言/运行时 | 全 Rust,零新增语言 | 引入 **Swift + SwiftUI + SwiftPM**;需 Rust↔Swift ABI 桥(`@_cdecl`)、Swift 并发/@MainActor 与 warpui 主线程模型共存 |
| UI 宿主 | WarpUI(Metal)内嵌 NSView,已有分层/洞/命中方案 | SwiftUI/`NSViewRepresentable` 生命周期;与 zap 的窗口/场景模型不同源 |
| **进程级独占资源** | 我们已占:CEF 初始化、60Hz 消息泵、helper 命名与 `--type=` 分流、bundle 组装与签名、CEF 152 版本 | 它也要独占同样这些 ⇒ **二者只能选一**,不是叠加 |
| 构建链 | `cargo` + `script/macos/bundle`(AGENTS §5.9 强制) | 追加 SwiftPM 插件与它自己的 bundle 组装 ⇒ 与强制流程冲突 |
| 已验证资产 | windowed 渲染/几何/隐藏与冻结/右键菜单/崩溃对齐/设置门控/回环 IPC+文档开始注入(两轮审核修复) | 全部作废(需按它的模型重写) |
| 可复用资产 | — | **OSR 全套手法**(§1.4)是目前最有价值的参考 |
| 维护风险 | 我们控制:CEF 版本、升级节奏、构建脚本 | 第三方 0.1.0、单人维护、CEF 版本被它 pin、helper/bundle 假设与 zap 不一致;每次 CEF 升级要等它跟进 |
| 许可证 | CEF(BSD) | BSD-3(兼容),但引入 Swift 生态依赖 |

## 3 若"整体集成",必须付出的代价(具体清单)

1. 移除/停用 `app/src/browser/cef_backend.rs`(927 行,含两轮审核修好的 IPC、冻结、菜单、崩溃路径)。
2. 移除 `tools/cef-helper` 与 `script/macos/cef_embed`,改为它的 5 helper 与 SwiftPM 插件;
   `script/macos/bundle --cef` 与它争抢 `Contents/Frameworks` 与签名顺序(AGENTS §5.9 明令不得绕开)。
3. CEF 初始化权交给 `CefRuntime`;我们的 `ZAP_CEF_WEBVIEW` / `FeatureFlag::CefWebview` / 设置开关
   都要改成驱动它 —— 而它的 API 是 Swift 的,需再包一层 Rust FFI。
4. 消息泵改由它的 `on_schedule_message_pump_work` 驱动(我们的 60Hz 定时器与它不能共存)。
5. 回环 IPC + `__ZAP_BRIDGE__` + `zap:` 前缀归一(N1/N2 修复)**全部重做**(它的 bridge 是 Swift 闭包,
   与 dsh 的 `webkit.messageHandlers.ipc` 协议不兼容)。
6. 冻结/隐藏策略、右键菜单内容(用户已确认的三项)、设置页两个开关:全部按其模型重写。
7. CEF 版本从 152 降到它 pin 的 148+ 或等它跟进。

## 4 建议路径(推荐)

**保留 Rust 侧集成,把 CefSwift 当 OSR 参考实现,按需 port**。它的代码量小、注释清楚,
且正好把我评估里列为"否决级风险"的 IME/弹层/菜单都跑通了,能显著降低方案 D 的不确定性:

| port 项 | 参考点 | 我们侧落点 |
|---|---|---|
| OSR 三开关 + 帧率 + 外部 begin-frame | `createOSRBrowser` | `cef_backend` 新增 OSR 分支 |
| IOSurface → `CALayer.contents`(+popupLayer) | `CefMetalHostView*` | 自建 NSView(仍挂 `WebViewContainerView` 下,保持现有分层) |
| IME 映射 | `imeSetComposition`/`imeCommitText`/… | ObjC 侧 `NSTextInputClient` → Rust 转发 |
| 输入/编辑命令/焦点 | `sendMouse*`/`sendKeyEvent`/`setFocus`/`copy:…` | 同上 |
| 右键菜单异步化 | 注释"modal loop 会崩" | 我们 OSR 化后必须照做 |
| 无障碍 | `set_accessibility_state` + AX 桥 | 可选,后置 |
| 顺带可捡的小改进 | `notify_screen_info_changed`、`find`、`zoom`、`audio mute`、DevTools 给真实窗口尺寸 | 现有 windowed 路径也能受益 |

**工作量估计**:OSR 核心(渲染 + 输入 + IME + 弹层)2~4 天(有参考实现的前提下);
无障碍/拖放/手势等可后置。**前提**:先做 2 天 spike 验证"IOSurface → CALayer 在 zap 分层下正确透出"。

## 5 透明问题的新结论(不变)

- windowed CEF(我们现在的模式)**仍然无法透明**:CefSwift 在 windowed 下同样只填色;
  它的透明来自 OSR(windowless + alpha=0 → 透明绘制)。
- 因此"集成 CefSwift"并不能绕过方案 D;它只是把 D 的实现难度降低(有现成参考)。
- 与我们已有实测一致:探针里 windowed + 透明背景实测为 `#FFFFFF`(见 TRANSPARENCY.md §1.3)。
