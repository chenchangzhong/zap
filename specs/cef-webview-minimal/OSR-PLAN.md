# OSR 真透明实施计划（dsh pane · CEF windowless）

> **给新会话的执行者**:本文件是自包含计划,不需要之前的对话上下文。开始前先读:
> `AGENTS.md`(仓库纪律)、本文件、[TRANSPARENCY.md](TRANSPARENCY.md)(为何 windowed 不行 + 实测证据)、
> [CEFSWIFT-EVALUATION.md](CEFSWIFT-EVALUATION.md)(参考实现与 API 清单)、
> [evidence/phase1/CODE-REVIEW.md](evidence/phase1/CODE-REVIEW.md)(既有实现已修过什么)。
>
> 基线:分支 `develop-CEF/Chromium`,HEAD 已含 windowed CEF 阶段 1 与两轮审核修复
> (`5d3a497dc`、`1c167e7e9`、`a1356541c`)。工作树应为干净。

## 0 目标与验收

**目标**:让 dsh pane 的 webview **真透明** —— 窗口不透明度 < 100% 时透出桌面,主题带背景图时显示背景图对应区域。

**为什么必须走 OSR**:windowed 模式已被**实测排除**(透明设置 + layer 置非不透明 + 页面全透明 ⇒ 洞内仍 `#FFFFFF`),
CEF 头文件也明确只有 windowless 才启用透明绘制(`cef_types.h:701-708`)。详见 TRANSPARENCY.md §1.3。

**非目标(明确不做)**:
- 不整体集成 CefSwift(理由见 CEFSWIFT-EVALUATION.md §2/§3)。
- 不改 dsh 前端(第三方产品,归属约束见 AGENTS/MEMORY)。
- 不删 windowed 代码路径 —— OSR 以**开关**形式并存,便于对照与回滚。
- 默认(wry)路径必须逐字节不变;非 macOS/未开 feature 的构建必须照常编译。

**验收标准(可测量,缺一不可)**:
1. **透明**:窗口不透明度设 80% 时,用 `screencapture` + 采样同一水平线上"pane 内"与"pane 外"的像素,
   与桌面透出程度一致(RGB 差 ≤ 2/255);且与 wry 路径同场景对照。
2. **背景图**:主题带背景图时,pane 区域显示背景图对应区域(而非纯色)。
3. **功能等价**(逐项与 windowed 现状对照):中文输入法(拼音→候选→上屏)、英文输入、Cmd+C/V/X/A、
   选中与拖拽、滚轮、鼠标进出、右键菜单(重新加载/检查元素)、DevTools 可打开、
   隐藏(pane 后台)与冻结、切回、懒初始化(设置里开开关后新开 pane 生效)、renderer 崩溃后 pane 进崩溃态。
4. **构建**:`cargo check -p warp`(feature off)与 `--features cef_webview` 均 **0 warning**;
   `tools/cef-helper` release 构建 0 warning;打包仍走 `script/macos/bundle`(AGENTS §5.9)。
5. **性能**:`windowless_frame_rate=60` + 外部 begin-frame 下滚动 dsh 页面无明显掉帧(用 Instruments 粗采样 CPU/GPU)。

## 1 关键设计决策(执行者按此实现,除非有证据推翻)

| 决策 | 选择 | 理由 |
|------|------|------|
| 显示路径 | **自建 NSView(仍挂 `WebViewContainerView` 下)+ `CALayer.contents = IOSurface`** | 保持 zap 现有分层(Metal 背景 → webview 容器 → 带洞的 Metal 内容层)与命中逻辑;零拷贝、retina 正确;不触碰 warpui 渲染管线 |
| 备选(不选) | warpui 场景内 Metal 采样外部纹理 | 改动面大(新增 element + 纹理导入),且要重做洞/hit-test 关系 |
| 与现状共存 | 同一 CEF 后端内加**渲染模式**:`Windowed`(默认) / `Osr`;由环境变量 + 设置项控制 | 可对照、可回滚;默认保持 windowed 直到验收通过 |
| OSR 三开关 | `WindowInfo{ windowless_rendering_enabled=1, shared_texture_enabled=1, external_begin_frame_enabled=1 }`、`BrowserSettings.windowless_frame_rate=60`、`runtime_style=ALLOY` | 参考 CefSwift `createOSRBrowser`;外部 begin-frame 让滚动/动画按真实刷新率走 |
| 透明实现 | `BrowserSettings.background_color` 传 **alpha=0**(windowless 下即启用透明绘制) | 与 CEF 文档一致;IOSurface 带 alpha,CALayer 合成后自然透出 |

## 2 参考实现(照抄手法,不抄代码)

CefSwift(BSD-3,`Rajaniraiyn/CefSwift`)已把 OSR 的全部原生affordance跑通,按文件对照:

| 需求 | 参考位置 | 要点 |
|------|----------|------|
| OSR 创建参数 | `Sources/CefKit/CefBrowser.swift` → `createOSRBrowser` | 三个 windowInfo 开关 + `windowless_frame_rate` + 异步创建(`on_after_created` 回调后才拿到 browser) |
| IOSurface → CALayer | `Sources/CefKit/CefMetalHostView*.swift` | `on_accelerated_paint` 里 `CALayer.contents = IOSurface`,包 `CATransaction`;`<select>` 弹层用独立 `popupLayer`(由 `on_popup_size` 定尺寸) |
| CPU 兜底 | 同上 | `on_paint` → `CGImage`(仅作降级) |
| IME | `CefBrowser` 的 OSR Input 扩展 | `NSTextInputClient.setMarkedText/insertText` → `ime_set_composition`/`ime_commit_text`;候选框位置用 `on_ime_composition_range_changed` |
| 鼠标/滚轮/光标 | 同上 | `send_mouse_move_event`(含 mouse_leave)/`send_mouse_click_event`/`send_mouse_wheel_event`;`on_cursor_change` |
| 键盘/快捷键 | 同上 + host view | mac→Windows VK 映射;**KEYDOWN+CHAR 两段式**(否则 JS keydown 不触发、光标不动);`performKeyEquivalent` 转发 Cmd+A/C/V/X/Z;应用级 Cmd+Q/W/M/H 放行 |
| 编辑命令 | `CefBrowser` 的 Editing 扩展 | `copy:`/`paste:`/`selectAll:`/`undo:`… → focused frame 的 `copy/paste/select_all/undo` |
| 右键菜单 | `docs/configuration.md` + host view | **必须异步**:回调里跑 modal NSMenu 会崩 CEF;先快照 model,再在回调外弹菜单 |
| 几何/DPI | `CefBrowser` 的 Geometry 扩展 | `was_resized` / `notify_screen_info_changed`(retina) / `was_hidden` / `invalidate` / `send_external_begin_frame` |
| 无障碍(可选,后置) | 同上 | `set_accessibility_state(STATE_ENABLED)` + `cef_accessibility_handler_t` 桥成 `NSAccessibilityElement` |
| 顺带可捡 | 同上 | `find`(页内查找)、`zoom_level`、`set_audio_muted`、DevTools 传真实窗口尺寸(900×700) |

## 3 任务分解(每步都有验证;spike 先行,不通过就停)

> `TDD Route`: mode=`off`(无用户/项目显式 TDD 要求), decision=`skipped`(FFI/GUI 路径不适合先写失败测试)。
> 替代做法:**每个任务都给可执行验证命令或可采样证据**;纯逻辑(坐标翻转、DPI 换算、键映射表)仍补单测。

### T1(spike A)—— IOSurface → CALayer 在 zap 分层下能否正确透出 ✅ **已完成(通过)**
> 骨架已就绪:`tools/cef-spike/src/bin/osr-probe.rs` + `probes/osr_host.m`;
> 结果与证据见 [evidence/phase1/OSR-SPIKE-A.md](evidence/phase1/OSR-SPIKE-A.md)。
> 结论:windowless + alpha=0 背景 + 透明页面 ⇒ 帧带 alpha(实测 82.7% 像素 alpha=0),
> 洞内透出下层;不透明内容(红块/渐变)同样正确渲染;≈60fps。
> **T4 必须遵守**:`view_rect`/`ScreenInfo.rect` 用 **DIP**,否则会重复缩放(实测 2400x1600 vs 1200x800)。

- 位置:`tools/cef-spike`(独立 crate,不碰主仓)。
- 做:新增 OSR 探针(可复用 `hole-probe` 的洞拓扑:底层 `ProbeBackgroundView` #12171F、容器、覆盖层带洞),
  实现 `wrap_render_handler!`(至少 `view_rect`/`screen_info`/`on_accelerated_paint`),把 IOSurface 贴进自建 NSView 的 CALayer;
  加载**透明页面**(`probes/transparent_page.html`)与**不透明页面**(现有 `hole_page.html`)各跑一次。
- **验证(硬证据)**:`screencapture -x` + `probes/sample_pixel` 采样洞内中心:
  - 透明页面 ⇒ 期望 `#12171F`(下层背景色),**不是** `#FFFFFF`;
  - 不透明页面 ⇒ 期望页面渐变/标记色(证明内容确实渲染)。
- 失败判据:仍为白/黑/无内容 ⇒ **停止**,把证据写入 TRANSPARENCY.md,回到方案 B/C。

### T2(spike B)—— 自建 NSView 的 IME 链路能否打出中文 ✅ **已完成(通过)**
> 结果与证据见 [evidence/phase1/OSR-SPIKE-B.md](evidence/phase1/OSR-SPIKE-B.md)。
> 结论:宿主视图实现 `NSTextInputClient` → `ime_set_composition`/`ime_commit_text` 后,
> 微信输入法**拼音→候选→中文上屏**跑通(人工确认),候选框锚点(`firstRectForCharacterRange`
> ← `on_ime_composition_range_changed`)也被真实输入法实际调用。
> **T4/T7 必须遵守的两条硬约束**:
> 1. 转发 IME 时 `replacement_range` **必须传显式 `(0xFFFFFFFF,0xFFFFFFFF)`**,不能传 NULL/`None`
>    —— CEF 的 capi 会把 NULL 退化成 `(0,0)`,渲染器据此 `SelectRange` 打断 `<textarea>` 焦点,
>    整条 composition 被**静默丢弃**(无任何报错,排查花了整轮)。
> 2. 焦点必须在**导航完成后**补一次(CEF 导航后静默丢焦点,`chromiumembedded/cef#3870`),
>    否则 IME 与光标都失效。
> 另:合成 NSEvent 驱动不了第三方输入法进程 ⇒ IME 的自动化只能覆盖"直接调用 `NSTextInputClient`"
> 那一段,真实输入法必须人工验证。

- 位置:`tools/cef-spike/probes/osr_host.m`(宿主视图)+ `src/bin/osr-probe.rs`(转发)+
  `probes/ime_page.html`(textarea + 回环上报);`probes/loopback_receiver.py` 直接复用。

### T3 —— 决策点 ✅ **已裁决:T1/T2 均通过 ⇒ 进 T4**
- T1、T2 都通过 ⇒ 进 T4;任一失败 ⇒ 记录结论、停止,不进入主仓改动。

### T4 —— 主仓:cef_backend 增加 OSR 渲染模式 ✅ **已完成(通过)**
> 结果与证据见 [evidence/phase1/OSR-T4-MAIN-REPO.md](evidence/phase1/OSR-T4-MAIN-REPO.md)。
> 开关:`ZAP_CEF_OSR=1`(默认 windowed,逐字节等价);另有
> `ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME`(默认关)、`ZAP_CEF_OSR_CPU_PAINT`(验证 on_paint 兜底)。
> **真机暴露的两个硬坑(必须记住)**:
> 1. `host.was_resized()` / `invalidate()` 会**同步**回调 render handler(GetViewRect/GetScreenInfo),
>    而调用点通常正持有 `WEBVIEWS.borrow_mut()` ⇒ 回调里再借用就 `already mutably borrowed`
>    **直接崩**(首次真机跑必崩)。修法:render handler 只读独立的 `OsrState`(`Rc`+`Cell`),
>    **不得借用 `WEBVIEWS`**;几何要在通知 CEF **之前**写入。
> 2. windowless 下 `CefBrowserHost::GetWindowHandle()` 返回的是**父容器**
>    (`browser_platform_delegate_osr_mac.mm`),windowed 那套 `setHidden:`/`setFrame:`/
>    `removeFromSuperview` 会把 warpui 的 `WebViewContainerView` 整个藏掉/摘掉。
> 证据要点:真机 3 轮(OSR 加速 / windowed 回归 / OSR+CPU 兜底)全部正常;
> `surface 2556x1526px = view_rect(1278,763) DIP × scale 2.0`,retina 无重复缩放。

- 文件:`app/src/browser/cef_backend.rs`、`app/src/platform/mac/objc/cef_support.m`、
  `app/build.rs`(QuartzCore/CoreGraphics/IOSurface 显式链接)、`app/src/browser/cef_backend_tests.rs`。
- 验证:`cargo check` 两种 cfg **0 warning**;`nextest -E 'test(cef_backend)'` 6/6;
  真机打开 pane(含窗口缩放跟随)渲染与尺寸正确。

### T5 —— 输入与编辑命令 ✅ **已完成(通过)**
> 结果与证据见 [evidence/phase1/OSR-T5-INPUT.md](evidence/phase1/OSR-T5-INPUT.md)。
> 做:宿主视图转发鼠标/滚轮/键盘/焦点 + 光标 + 响应者链编辑动作;Rust 侧构造 CEF 事件
> (修饰键位映射、mac→Windows 键码表、KEYDOWN+CHAR 两段式),编辑命令走 focused frame。
> **真机暴露的三个坑(必须记住)**:
> 1. `-[NSEvent clickCount]` 对 mouseEntered/Exited 会抛 ObjC 异常直接崩进程;
>    `characters`/`isARepeat` 同理只对 keyDown/keyUp 有效 —— 按事件类型取字段。
> 2. **CEF 会把"页面未处理的按键"交给应用主菜单**
>    (`CefBrowserPlatformDelegateNativeMac::HandleKeyboardEvent` →
>    `[[NSApp mainMenu] performKeyEquivalent:]`),而 zap 的窗口菜单经 `setWindowsMenu:`
>    带了 AppKit 自动加的 "Enter Full Screen"(keyEquivalent 就是**裸 `f`**)
>    ⇒ 页面里打 `f` 会把窗口切全屏。修法:`CefKeyboardHandler::on_key_event` 返回 1 认领。
> 3. 开着 **CapsLock** 时 `WarpWindow::performKeyEquivalent:` 的
>    `mods == NSEventModifierFlagCommand` 精确比较会漏掉 Cmd+A/C/V/X ⇒ 宿主侧补
>    `edit_command_for_key`(纯函数 + 单测)。**没有**在视图里实现 `performKeyEquivalent:`
>    —— 那会抢在菜单前吞掉 zap 自己的 Cmd+T/Cmd+1..9(参考实现 CefSwift 的策略与 zap 不兼容)。
> 验证:两种 cfg `cargo check` 0 warning;`nextest -E 'test(cef_backend)'` 13/13;
> 真机逐项(点击/拖选/滚轮/进出/光标/英文/`f` 不全屏/Cmd+C·V·A·X)全部通过,无崩溃。
> 中文输入不在本任务(主仓宿主视图尚未实现 `NSTextInputClient`)⇒ T7。

### T6 —— 弹层与右键菜单 ✅ **已关闭**(右键菜单通过;`<select>` 弹层经用户决定不做 —— 没用到)
> 结果与证据见 [evidence/phase1/OSR-T6-POPUP-MENU.md](evidence/phase1/OSR-T6-POPUP-MENU.md)。
> 做:层树拆成 root + 内容层 + **popup 层**(照 CefSwift),`on_popup_show`/`on_popup_size`
> 与 POPUP 类型的 paint 回调路由到弹层(原来直接丢弃);**右键菜单按计划改宿主异步 NSMenu**
> (「重新加载」「检查元素」),命令回 Rust 走 CEF API。
> **必须记住的坑**:
> 1. **OSR 下 CEF 原生菜单触发链不通**:`CefMenuManager::CreateContextMenu` 是
>    `on_before_context_menu` 的唯一调用点(CEF 源码 menu_manager.cc),而实测该回调
>    **一次都没被调用** ⇒ 必须由宿主自己弹菜单(计划的"异步 NSMenu"就是这个原因)。
> 2. **CEF 原生菜单在 OSR 下结构性不可用**:`menu_runner_mac.mm` 的 windowless 分支第一句是
>    `if (!browser->GetWindowHandle()) return false;`,而 windowless 的 host window handle 取自
>    `WindowInfo.parent_view` —— 本项目的 OSR 路径**从不设**它(只有 windowed 路径 `set_as_child`)
>    ⇒ handle=0 ⇒ 原生菜单永不出现(实测 `on_before_context_menu` 一次都没被调用)。
>    **若将来给 OSR 设了 `parent_view`,必须给宿主菜单加互斥,否则双弹。**
> 3. **`CefRenderHandler::GetScreenPoint` 必须实现**(默认 false):CEF 每次鼠标事件翻译
>    (`TranslateWebMouseEvent`)都要用它填 `screenX/screenY`,拖动/DevTools 等原生 UI 也需要。
>    mac 上要返回 **AppKit 屏幕坐标(左下原点、单位 DIP)**:`menu_runner_mac.mm` 把这个点直接
>    交给 `popUpMenuPositioningItem:…inView:nil`。**它跟右键菜单出不出现无关**(原因见上一条)。
>    另:`GetScreenInfo` 的矩形留空会回退 `GetViewRect`,故"填视图矩形"是合规实现。
> 3. 弹层坐标是 **DIP、左上原点**,而我们的视图**非 flipped** ⇒ y 必须翻转(未实机验证)。
> **弹层实测结论(2026-09-22)**:注入可见 `<select>` 并点击,点击确实到达 CEF(坐标吻合),
> 但 **`on_popup_show`/`on_popup_size` 一次都没触发**、下拉不展开 ⇒ mac 上 `<select>` 走嵌入方
> 菜单路径,而 CEF mac 侧没有实现它(`PopupMenu`/`WebMenuRunner` 在 CEF 源码里无命中),
> `OnPopupShow` 只在 `InitAsPopup`(渲染器创建 popup widget)时触发 ⇒ **链路在 mac 不可达**。
> 实现保留(与 CefSwift 同构,供其他平台/将来);**产品行为**:mac 上 CEF pane 里 `<select>` 打不开
> (windowed 同理,非 OSR 特有)⇒ 需要 `<select>` 的页面用 **wry 内核**。
> **2026-09-22 用户决定:`<select>` 场景不用到,该验收项撤销、T6 关闭**(右键菜单与
> `screen_point` 两项能力仍为已验证交付)。

### T7 —— IME 落地主仓 ✅ **已完成(通过)**
> 结果与证据见 [evidence/phase1/OSR-T7-IME.md](evidence/phase1/OSR-T7-IME.md)。
> 做:`WarpCefOsrView` 实现 `NSTextInputClient`(端口自 T2 spike);
> `keyDown:` 改 **deferred 模型**(`interpretKeyEvents:` 只累积状态,末尾一次决定 →
> KEYDOWN+CHAR / `ime_set_composition` / `ime_commit_text`);render handler 接
> `on_ime_composition_range_changed` → 候选框锚点;决策逻辑在 Rust(纯函数 + 单测)。
> **两个必须记住的坑**:
> 1. **不能用 T2 spike 的"直接转发 `insertText:`"**:那会让普通字母也走
>    `ime_commit_text`,页面收不到 JS `keydown`(T5 的两段式白做)。必须 deferred。
> 2. **`NSEventModifierFlags` 低 16 位是设备相关位**(实测每次按键都带 `0x100`
>    `kCGEventFlagMaskNonCoalesced`、`0x8` 左 Command),**比较修饰键组合前必须先按
>    `NSEventModifierFlagDeviceIndependentFlagsMask` 过滤**;否则"CapsLock 开着时的
>    Cmd+A/C/V/X/Z"这类兜底判断永远不成立(T5 的兜底就因此失效,直到 T7 才发现)。
>    另:视图级 `performKeyEquivalent:`(排在菜单之前,只认这 5 个组合)才能盖住
>    "zap 窗口 `mods == Command` 精确比较 + CapsLock"的组合盲区。
> 验证:两种 cfg 0 warning;`nextest -E 'test(cef_backend)'` 15/15;真机拼音→候选→
> 中文上屏 + 候选框跟随 + 英文/Cmd 快捷键无回归(Cmd+T/Cmd+1..9 仍归 zap)。

- 文件:`app/src/platform/mac/objc/cef_support.m`、`app/src/browser/cef_backend.rs`、
  `app/src/browser/cef_backend_tests.rs`。

### T5.5 —— 交付前独立审核与修复轮 ✅ **已完成**
> 详细结论见 [evidence/phase1/OSR-REVIEW-2.md](evidence/phase1/OSR-REVIEW-2.md)。
> 两位独立审查者(缺陷清单 + canonical 模板分级)提出 5 条确认缺陷与 8 条次要项;每条都
> 先复核证据再决策,共修复 8 处、明确不改 5 处(含"cefclient 的 `textInserted` 是死变量,
> 不该加上那条守卫")。
> **新增/固化的硬约束(后续任务必须遵守)**:
> 1. `app/assets/webview_init.js` **必须由渲染进程 `on_context_created` 注入**(CEF 后端此前
>    从未实现 ⇒ `__restoreFocused`、早期 IPC 队列、focusin 上报、`window.open` 拦截全部缺席)。
> 2. OSR 下 CEF 的 `SetFocus` **不会**把自建视图设为 first responder,必须由宿主
>    `makeFirstResponder`(带可重入守卫)。
> 3. 焦点交接要认 pane 级权威判据 `PaneFocusHandle::is_focused`:新开 pane 时
>    `focus_contents` 会因 webview 未可见而早返回,`is_self_or_child_focused` 因此恒假。
> 4. **解冻路径不得嵌套借用 `WEBVIEWS`**(既有 panic,已修);新增"借用内调 CEF/ObjC"前
>    先按本文件的借用纪律核对。
> 5. `__restoreFocused` 对"输入框尚未挂载"必须重试(load 早于 SPA 首渲染)。
> 6. mac OSR 下 `windows_key_code` **不被 CEF 采用**(合成 NSEvent 后由 Chromium 反推),
>    不要把它写成"已在 mac 生效"的行为。

### T8 —— 开关、回滚与既有策略的等价性 ✅ **已完成(通过)**
> 见 [evidence/phase1/OSR-T8-SWITCH-EQUIV.md](evidence/phase1/OSR-T8-SWITCH-EQUIV.md)。
> **已完成**:渲染模式升级为设置项 `general.webview.use_osr_rendering`(默认 false)+ 设置页开关
> + `resolve_render_mode(env, setting)` 纯函数与单测(env 显式非空才覆盖设置);`ZAP_CEF_OSR` 仍可用。
> **实机命中并已修一个 Critical**:切 tab 必闪退 —— `setHidden:` 让 first responder 让位 ⇒
> `resignFirstResponder` 同步回调进 Rust ⇒ `focus()` 在调用方已持 `WEBVIEWS.borrow_mut()` 时重入
> 借用 ⇒ panic;而 `extern "C"` 回调里的 panic 会**直接 abort**。修法:所有回调入口的公共路径
> (`with_browser`/`browser_snapshot`)改 `try_borrow[_mut]`,借不到就跳过;`focus()` 同理。
> **这是 T7 审核时标为"残余风险"却没修的那条,教训:标为残余风险的借用重入要当场修。**
> **已全部验证**:冻结→解冻完整路径(日志闭环、未 panic,顺带证明 OSR 下 `send_cdp` 生效)、
> 切 tab(隐藏)不崩、renderer 崩溃→pane 进崩溃态、设置页开关单独生效(env 不设也是 OSR)、
> 懒初始化路径(启动不初始化,由第一个 pane 触发且取到设置项)。
> **新增永久观测点**:`[cef] render mode = … (env …, setting …)`。
> **操作教训(记录)**:杀进程必须用只匹配自建实例的模式 —— 本轮误用过
> `pkill -f 'Helper \(Renderer\)'`,把用户机器的微信/Arc/Lark/VS Code 的 renderer 一起杀了
> (均已自动重建,无持久影响)。

### T9.5 —— 计划全量代码审核(第三轮)✅ **已完成**
> 见 [evidence/phase1/OSR-REVIEW-3.md](evidence/phase1/OSR-REVIEW-3.md)。范围:`433bdb206..HEAD` 全部 18 个提交。
> - 修:`performKeyEquivalent:` 放行 Shift 导致 `Cmd+Shift+A/C` 被吞(自证绑定存在)、
>   `destroy()` 的关闭/释放顺序、`drive_external_begin_frame` 的"持借调外部"(最后一处)、
>   `webview_init.js` 的 hasFocus 重试窗口回退、两处与实现相反的注释。
> - 复核确认无问题:MRC 配对(运行期探针实证)、释放幂等、windowed 纯净、NSEvent 字段纪律、
>   层语义、坐标换算、`clang -Wall -Wextra` 0 warning。
> - 记录项(有判据、当前无可见影响):子层 contentsScale、`otherMouse` 的 buttonNumber、
>   CPU 兜底缓冲区未拷贝、IOSurface 生命周期、光标离开视图、`selectedRange` 未接选区回调、
>   弹层在 mac 不可执行 —— 逐条写在证据文档 §3 表格里,**将来触发条件明确**。
> - Rust 侧第一位审查者运行失败,已收窄范围重派;其结论到达后按同样纪律处理。

### T9 —— 证据与文档归档 ✅ **已完成**
> - `TRANSPARENCY.md` 追加 §7「落地结果」:方案 D 已实现、默认 windowed、开关与硬约束清单;
>   并明确 **仍未做的验收 1/2(真实 pane 里的像素级透明比对)**。
> - `RUNTIME-VERIFICATION.md` 追加 §8「OSR 阶段实跑(T4–T8)」:一张汇总表 + 各任务证据文档索引。
> - `TECH.md` 追加「渲染模式与开关(定稿)」:两种模式、两级开关与优先级、生效时机、
>   以及"设置不会被静默忽略"的两条保证与永久观测点日志。
> - **临时探针已清理**:主仓代码内无 `printf/NSLog/dbg!/eprintln`、无探针残留(已核对);
>   `tools/cef-spike`(阶段 0/T2 的独立探针 crate,372 KB,无任何构建引用)**已按计划删除**
>   (经用户确认,删除记录在 git 历史里可恢复);`tools/cef-helper` 是构建必需资产,保留。

## 3.5 T10 —— 对齐补齐(后续任务,新会话执行)

> 来源:与参考实现(CefSwift)的逐层比对,清单与判据见
> [evidence/phase1/OSR-ALIGNMENT-VS-REFSWIFT.md](evidence/phase1/OSR-ALIGNMENT-VS-REFSWIFT.md)。
> **先读该文档 §0–§5**:§0 是我对 T6 归因的自我更正,§5 是用户对各项的决定(右键菜单保持两项、
> P0-4/P1-5 只记录)。下面三项是**用户指定要做的**,每项做完都要:两种 cfg `cargo check` 0 warning、
> `cargo nextest` 窄过滤、`script/macos/cef_smoke` 构建,并**由用户实机验证**(单元测试覆盖不到)。
>
> **本轮进度(2026-09-22)**:T10.3 的**保存面板 + 取消**部分已完成并通过实机验证(**进度 UI 未做**);
> T10.1(拖放)/T10.2(焦点随窗口 key)的实现已在工作区(未提交),**实机验收尚未做** —— 判据见各自小节。

### T10.1(P0-3)拖放双向 —— 优先
- **缺口**:既不能从 Finder 拖文件/文本进 pane,也不能从页面往 Finder/终端拖(实测两个方向都无反应)。
- **参考**:`/tmp/CefSwift-main/Sources/CefSwiftUI/CefMetalHostView+DragDrop.swift:34-127`、
  `/tmp/CefSwift-main/Sources/CefKit/CefRenderHandler.swift:188-209`(拖放部分)、
  其测试断言 `Tests/CefKitTests/OSRInputPassthroughTests.swift:17-36`(拖放掩码)。
- **改动点**:`app/src/platform/mac/objc/cef_support.m`(OSR 宿主视图 `WarpCefOsrView`,**非 ARC**;
  `registerForDraggedTypes` + `draggingEntered/draggingUpdated/draggingExited/prepareForDragOperation/
  performDragOperation/draggingEnded`,并正确返回 `NSDragOperation`)、
  `app/src/browser/cef_backend.rs`(handler + `extern "C"` trampoline;系统→页面方向由
  **`BrowserHost`** 的 `drag_target_drag_enter/over/leave/drop` 送入,页面→系统方向由
  `RenderHandler` 的 `start_dragging` + `update_drag_cursor` 送出 —— 该 API 归属在本计划初稿里
  写错过两次:既不是 `CefDragHandler::on_drag_enter`,`drag_target_*` 也不在 `RenderHandler` 上)。
- **若 cef-rs 缺必需 API**:先报缺失点与最小替代方案,**不要编造 API**。
- **验收(用户实机)**:①从 Finder 拖一个文件到 pane 上 → 页面收到 drag/drop(例如输入框插入路径、
  或页面 dragover 高亮);②在页面里选中内容往终端/Finder 拖 → 产生拖拽会话。
  **状态(2026-09-23,OSR 实例,用户确认"测试通过")**:①② **均通过**;日志证据(drag enter → 页面回报
  `0x1`=Copy → drop;start_dragging → drag session ended op=0x7)见
  [evidence/phase1/OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md) §1–§2。页面可见效果(是否插入路径、
  高亮)与"源文件是否被移动"**日志不覆盖**。

### T10.2(P1-7)焦点跟随窗口 key 状态
- **缺口**:点一下页面后用 Cmd+Tab 切走,页面里光标**仍一直闪烁**(实测)。
- **参考**:`/tmp/CefSwift-main/Sources/CefSwiftUI/CefMetalHostView.swift:143-169`(观察
  `didBecomeKey/didResignKey`)。
- **改动点**:OSR 视图观察 `NSWindowDidBecomeKeyNotification`/`NSWindowDidResignKeyNotification`
  (视图进出窗口/窗口重建时正确注册与注销 observer;注意 selector 版 `addObserver:selector:name:object:`
  **不 retain observer**,故**不应**对 observer 做 release,`dealloc` 里摘除只是兜底;不得在 dealloc 后还收通知)
  → 新增一条 ObjC→Rust 回调 → `browser.host().set_focus(0/1)`。
  **不要破坏**现有 `becomeFirstResponder/resignFirstResponder` 逻辑与 `_syncingFocus` 重入守卫;
  回调路径遵守"不得在持有 `WEBVIEWS` 借用时调外部"。
- **验收(用户实机)**:点页面 → Cmd+Tab 切走 → 页面光标停止闪烁;切回来 → 恢复闪烁且可继续输入/上屏中文。
  **状态(2026-09-23,OSR 实例)**:机制层证据可见(日志里 `window key=1/0` 成对出现,
  [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md) §3);**视觉判据**(光标停闪/恢复、中文上屏)日志
  不覆盖,由用户在本轮整体确认"测试通过"。

### T10.3(P0-2)下载保存面板/进度 UI —— ✅ **保存面板 + 取消已完成并实机验证(2026-09-22);进度 UI 未做**
> 落地结果、证据与两条踩坑见 [evidence/phase1/OSR-T10-DOWNLOAD.md](evidence/phase1/OSR-T10-DOWNLOAD.md)。
> 要点:接 `download_handler` → 与 wry 共用 `NSSavePanel`;取消**不能**靠"返回 1 但不执行 callback"
> (实测下载卡在 target-pending、数据落到隐藏临时文件、页面一直"下载中"),改用
> `CefDownloadItemCallback::Cancel()`;面板期间用 `DOWNLOAD_PANEL_OPEN` 抑制消息泵避免重入。
- **现状(实测)**:没有保存面板,**但下载直接完成** ⇒ CEF 默认下载管道可用,缺的是 UI。
  (`assets/webview_init.js:95-101` 明确对 `<a download>` 不拦截,依赖"原生下载管道"。)
- **参考**:`/tmp/CefSwift-main/Sources/CefKit/BrowserClient.swift:294-341` + `CefDownloads.swift:53-72`。
- **可对齐的交互**:wry 侧的保存面板实现在 `app/src/browser/browser_web_view.rs:336-360,402-420`
  (两条后端最好表现一致)。
- **改动点**:加 CEF 的 download handler(`on_before_download` 决定走默认还是自己接管)+ Rust 侧
  会话/进度状态 + 保存面板与进度 UI(`app/src/browser/` 内,复用现有 webview 相关 UI 风格)。
- **验收(用户实机)**:触发一次下载(dsh 的 Session/日志导出,或页面里任意 `<a download>`)
  → 出现保存面板(或按产品决定直接落到 `~/Downloads` 并给出进度/完成提示)。

### T10 收尾:已修项与待验证项(2026-09-22 独立审查后)

> **验证状态(2026-09-23 更新)**:2026-09-22 曾停止 GUI 测试;2026-09-23 在 OSR 实例上恢复了 GUI 测试,
> **拖放双向 ①② 通过**(日志证据见 [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md))。
> 同日完成 **CEF 升级(`152.0.8` → `154.0.23`,见 [CEF-UPGRADE.md](CEF-UPGRADE.md) §8),升级后再次实机回归**:
> 用户确认**除"mac `<select>` 弹层"与"保存面板后焦点恢复"两项外,其余项通过**(逐项结果表见
> CEF-UPGRADE.md §8.3)。这两项**同属用户明确决定"暂不处理、仅记录"**。
> **仍未单独验证**:`external begin-frame` 内部时序、`start_dragging` 坐标口径的专门取证、
> windowed 模式下的下载面板与焦点、成功 drop 后缺 `drag leave`(未验证推测)。
> 第二轮独立复审(下载 + 拖放)的完整发现、处置与"无 GUI 无法确认"清单见
> [evidence/phase1/OSR-T10-REVIEW.md](evidence/phase1/OSR-T10-REVIEW.md)。

**已修(审查确认项)**:

| 项 | 证据/结论 | 修法 |
|---|---|---|
| 拖放:`start_dragging` 的 `x`/`y` 被当视图 DIP 用 | **确认是契约违反**:CEF 152 `cef_render_handler.h` 明写 screen coordinates;Blink `web_frame_widget_impl.cc:3319-3321` 用 `event.PositionInScreen()` 填,CEF `browser_platform_delegate_osr.cc:539-542` 原样透传 | ObjC **不再用 `x`/`y` 定位**,改用 AppKit 当前鼠标位置(窗口→视图坐标)——两种坐标假设下都正确,不引入新的未验证换算;`x`/`y` 仅留日志 |
| 拖放掩码测试名不副实 | 原测试只断言 CEF 侧常量,看不到 AppKit 一侧(而"直接透传、不做映射表"正建立在该等价关系上) | 给 `objc2-app-kit` 启用 `NSDragging` feature,测试改为**两侧逐项比对**(`app/src/browser/cef_backend_tests.rs`) |
| `_dragAllowedOps` 会话结束未复位 | 无功能影响(`start_drag` 每次覆盖),属确认的状态不一致 | `draggingSession:endedAtPoint:operation:` 里复位为 `NSDragOperationNone` |
| **保存面板后焦点丢失**(用户实机报告:取消后页面收不到键盘) | 面板是 **app-modal**,关闭时 AppKit 不保证把 key 状态还给原窗口;前两版修复(视图焦点 / DOM 焦点)都**未解决** | 三层:① 共用面板函数里归还 key window(在 `activate()` **之前**取 `keyWindow()`、仅 `isActive()` 时 `makeKeyAndOrderFront`)② 视图/CEF 焦点(OSR 走 `focus()` 含 0→1 补同步;windowed 走 `makeFirstResponder` 原生视图)③ DOM `__restoreFocused`;**仅当弹出前焦点在页面上才做**(不抢终端焦点),立即 + 主队列下一拍各一次(DOM 只一次)。**至今无运行时证据**;⏸ **2026-09-23 用户决定:暂不追、仅记录**(见 [evidence/phase1/OSR-T10-DOWNLOAD.md](evidence/phase1/OSR-T10-DOWNLOAD.md) §5.3) |
| 系统→页面拖放的应答掩码 | destination 返回值决定来源执行哪种操作;原实现返回"来源全掩码" = 接受 Move ⇒ 同卷文件从 Finder 拖入时 Finder 默认走 Move,**页面即使拒收也可能删掉源文件**(数据丢失) | `draggingEntered/Updated` 收敛为 `Copy|Link|Generic`(来源只给别的操作时兜底 Copy)。依据:网页不消费 Move 语义,且 windowed 下 Chromium 原生视图给的就是 Copy(实机光标为绿色 +)。**✅ 2026-09-23 用户实机:拖放双向通过**(证据 [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md):页面回报 `0x1`=Copy、会话正常结束) |
| 页面→系统拖拽无拖拽图像 | `setDraggingFrame:contents:nil` 按 `NSDraggingItem` 契约 = **隐藏该项** | 记为**有意接受**(与参考实现同款;注意参考实现该处注释写成 "placeholder image",与头文件相反,勿照抄) |

**待验证(未确认,不据此改动)**:

| 项 | 现象/判据 | 触发条件 | 怎么验 |
|---|---|---|---|
| ⏸ 保存面板后焦点恢复(**用户决定暂不追,仅记录**) | 三层修复是否真的把键盘焦点还回页面;**至今无运行时证据**(升级后的实机回归也未覆盖该路径,见 [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md) §4) | 导出 → 取消 → 不点页面直接打字 | 将来要动时:看日志有无 `[cef] 面板关闭后恢复页面焦点 …`;仍不行则查"app 是否被弄成非激活"。详见 [OSR-T10-DOWNLOAD.md](evidence/phase1/OSR-T10-DOWNLOAD.md) §5.3 |
| 页面拒收时的操作语义 | `draggingUpdated:` 恒返回来源掩码、`performDragOperation:` 恒返回 YES ⇒ 来源(Finder)可能认为"已接受" | 页面在 dragOver 里判定不收 | T10.1 实机:从 Finder 拖文件到 pane,按 Option/Cmd 观察**源文件是否被移动/删除**(windowed 下已实测**未被移走**) |
| 拖入瞬间无 `drag_target_drag_over` | `draggingEntered:` 只发 enter(AppKit 随后会发 `draggingUpdated:`,落点前也补 over) | 页面高亮依赖首次 over | T10.1 实机:拖入时页面是否**立刻**高亮;不亮再补 |
| `update_drag_cursor` 把 0 当"无变化" | 页面无法在拖拽中途把允许操作收窄为 None | 拖拽光标语义不对 | 实机观察拖拽途中的光标 |
| 下载在 windowed 模式 | handler 与渲染模式无关,但只验了 OSR 下的下载行为 | 用 `ZAP_CEF_OSR=0`(或关设置项)启动 | 冒烟一次导出 |
| 取消依赖后续 IN_PROGRESS 回调 | 该回调不来 ⇒ 下载停在 target-pending(无终态日志) | CEF 行为变化 | 若出现"取消后仍显示下载中",先看日志有无 `cancel requested` |
| 重入防护只覆盖 `pump()` | 嵌套 run loop 期间 `set_bounds`/`focus`/`navigate`/`close` 等入口未 gate | **无已知触发输入** | 未验证推测,仅记录 |
| 文件名截断 | 首轮存出 `e7145-…zip`(建议名尾部),次轮为完整名 | 建议名超长时 | 再出现时查 `NSSavePanel::setNameFieldStringValue` 与长名的交互 |
| CEF "未处理下载"的成因 | 现象已确认(默认目录静默落盘);与上游 master 的 Alloy 取消分支不符 | —— | 需要时做判别实验:取消分支临时 `return 0`,看是 `state=2 CANCELLED` 还是仍落 `~/Downloads` |
| **T10.1/T10.2 实机验收** | 拖放双向 ①② **已通过**(2026-09-23 OSR,日志证据见 [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md));T10.2 的视觉判据与"页面可见效果"日志不覆盖,由用户整体确认 | —— | 剩余未覆盖项:保存面板路径(windowed 下的下载/焦点)、成功 drop 后缺 `drag leave` |
| ✅ 掩码收敛后的拖放表现 | 收敛为 Copy 后光标/落下是否仍正常 | —— | **已完成**:2026-09-23 用户实机拖放双向通过(证据 [OSR-T10-DRAG.md](evidence/phase1/OSR-T10-DRAG.md)) |
| ✅ OSR 视图遮蔽终端拖放路径 | OSR 宿主视图 `frame` = webview 矩形、无 hitTest 覆盖 ⇒ **只**在 webview 矩形内遮蔽 `WarpHostView` 的"拖文件进终端";webview 隐藏时不参与命中 | —— | **已完成**:用户实机确认"拖到终端本身"正常(2026-09-23,"三项都正常") |
| `draggingEntered:` 未立即补 `drag_target_drag_over` | AppKit 随后会周期性发 `draggingUpdated:`(我们未实现 `wantsPeriodicDraggingUpdates`,默认行为属推断) | 页面高亮依赖首次 over | 最坏只是高亮晚一个周期;实机看"拖入瞬间是否立刻高亮" |
| `drag over` 无日志通道 | `drag over` 完全不打日志(非 debug 级),故"高亮是否延迟"没有证据通道 | —— | 需要时加 debug + 限流 |

### 不在本轮范围(仅记录,用户已定)
P0-4(编辑快捷键改为转发按键事件,让页面 JS `keydown`/`preventDefault` 生效)、
P1-5(手势与缩放:magnify/smartMagnify/swipe/touch、`zoomLevel`)、
以及比对文档 §1 的 P1-6/8/9/10/11 与全部 P2 项 —— 需要时按同一格式扩写。

## 4 验证命令速查

```bash
# 构建(两种 cfg 都必须 0 warning)
cargo check -p warp
CEF_PATH="$HOME/.local/share/cef" cargo check -p warp --features cef_webview

# 测试(仓库规矩:一律 nextest)
CEF_PATH="$HOME/.local/share/cef" cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend) + test(loopback) + test(webview_init_js)'
cargo nextest run -p warp -E 'test(schema_validation) or test(i18n) or test(webview_init_js)'

# helper(改动过必须重打;它承载 render 进程的文档开始前注入)
CEF_PATH="$HOME/.local/share/cef" cargo build --manifest-path tools/cef-helper/Cargo.toml --release

# 探针(阶段 0/1 既有)
cd tools/cef-spike
CEF_PATH="$HOME/.local/share/cef" cargo run --bin make-bundle -- hole-probe -o target/hole
PROBE_TRANSPARENT=1 target/hole/hole-probe.app/Contents/MacOS/hole-probe \
  --url="file://$PWD/probes/transparent_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=25 --exit-after=40 --log=/tmp/osr.log
screencapture -x /tmp/osr.png && probes/sample_pixel /tmp/osr.png 1120,920 1800,1000

# 打包(必须走它;禁止 --skip-build / 替代命令)
./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef
./script/macos/cef_smoke            # 打包 + 冒烟(--run 走优雅退出)
```

## 5 风险与缓解

| 风险 | 级别 | 缓解 |
|------|------|------|
| IME(中文输入) | **否决级** | T2 先 spike 验证;失败即停止(参考 CefSwift 的 `NSTextInputClient` 映射) |
| `<select>` 等页面内弹层 | 高 | T6 用独立 popup layer;先确认 dsh 页面是否用到原生 select |
| 输入保真(修饰键/双击/拖选/滚轮惯性) | 中 | T5 逐项用例;参考 CefSwift 的 KEYDOWN+CHAR 两段式与 VK 映射 |
| 右键菜单崩 CEF | 中 | 必须异步 NSMenu(回调内 modal loop 会崩,参考实现已踩过) |
| 性能(60fps) | 中 | shared texture + external begin-frame;失败则降帧率(30)并记录 |
| 无障碍/拖放/手势 | 低(可后置) | 明确列为后置项,不阻塞验收 |
| 半透明窗口下桌面透出仍不对 | 中 | 验收标准 1 的像素比对即判据;必要时对照 wry 路径 |

## 6 边界与纪律(违反即返工)

- **禁止**终止/关闭用户正在运行的任何进程,尤其 `/Applications/Zap.app`;只启动/管理自建实例。
- 默认 wry 路径逐字节不变;OSR 代码全部 `#[cfg(all(target_os="macos", feature="cef_webview"))]` 门控。
- 打包必须走 `script/macos/bundle`;helper 必须走 `tools/cef-helper`(不得用主二进制当 helper)。
- 注释与回复用简体中文;改动每一行都能溯源到本计划的任务。
- 每个任务完成后归档证据;临时探针/日志用完即删。
- 不做计划外"顺手重构";发现无关问题只记录。

## 7 回滚

OSR 全程在**开关**之后,默认 windowed:
- 若 T1/T2 spike 失败:主仓零改动,仅归档结论。
- 若 T4+ 中途失败:关掉开关即回到 windowed 现状(该路径不删),并把失败点写入 TRANSPARENCY.md。
