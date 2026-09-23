# 与参考实现(CefSwift)的 OSR 对齐比对(2026-09-22)

> 方式:独立审查者读 `/tmp/CefSwift-main`(12285 行 Swift,含其 OSR 输入/逻辑/菜单/Handler 测试断言)
> 与我们的 `app/src/platform/mac/objc/cef_support.m` + `app/src/browser/cef_backend.rs` 逐层比对;
> 头号结论由我方**自证**(见 §0)。
> 说明:本计划的 T4–T9 范围为**渲染 / 输入 / IME / 菜单 / 弹层 / 开关**,下列多数项(下载、拖放、
> 手势、对话框、权限…)从未进入范围 ⇒ 这是**后续路线图**,不是"本轮实现的 bug 清单"。

## 0 自我更正:上下文菜单的归因错了(已自证)

- **我此前的记录**(`OSR-T6-POPUP-MENU.md` / `cef_support.m:479-486`):"OSR 下 CEF 原生菜单
  结构性不可用(windowless 无 window handle)⇒ 宿主必须自己弹 NSMenu"。
- **更正**:"不可用"只对 **CEF 的默认 menu runner** 成立 ✓;但它**不能推出"菜单模型路径不可用"** ✗。
  参考实现证明后者可用:它的 OSR 浏览器**同样不设 `parent_view`**(`CefBrowser.swift:133` 注释:
  "parent_view stays nil"),却实现了 `run_context_menu`(`BrowserClient.swift:581-588` →
  `host.osrRunContextMenu(menu, at:, callback:)`)并消费 CEF 菜单模型。
- **我们的实际缺口**:`cef_backend.rs:2495-2516` 清空模型再塞 2 项,但**从未实现 `run_context_menu`**
  ⇒ 模型无人消费;`on_context_menu_command`(`:2520-2548`)与 `WebviewContextMenu` 是**死代码**;
  实际菜单是 `cef_support.m:487-515` 硬编码的 2 项(重新加载/检查元素)⇒ **页面自己的菜单项
  (选中文本的复制、输入框的粘贴、链接/图片动作、拼写检查等)全部拿不到**。

## 1 未对齐清单(按用户可见影响排序)

### P0
1. **上下文菜单**(见 §0):缺 `run_context_menu`;模型被清空;死代码 + 记录归因错误。
2. **下载 handler 完全缺失**(**已解决,2026-09-22**):`cef_backend.rs` 无 `download_handler`;而 `assets/webview_init.js:95-101`
   明确对 `<a download>` 不拦截、依赖"原生下载管道" ⇒ CEF 下 dsh 的 Session 日志导出等**无保存面板/无进度**。
   (wry 侧有:`browser_web_view.rs:336-360,402-420`。)参考:`BrowserClient.swift:294-341` + `CefDownloads.swift`。
   ⇒ 已接 `download_handler`(弹共用保存面板 + 可取消 + 终态日志),实机验证与踩坑见
   [OSR-T10-DOWNLOAD.md](OSR-T10-DOWNLOAD.md)。进度 UI 仍未做。
3. **拖放双向缺失**:无 `registerForDraggedTypes`、无 `start_dragging`/`update_drag_cursor`
   ⇒ 不能拖文件进 pane、不能把内容拖出去(Finder/终端)。参考:`CefMetalHostView+DragDrop.swift:34-127`。
4. **编辑快捷键"怎么发"不同**:参考在 `performKeyEquivalent` 里**把按键事件本身转发**给页面
   (KEYDOWN+CHAR ⇒ 页面 JS `keydown` 会触发、`preventDefault` 生效),且覆盖**任意 Cmd/Ctrl 组合**;
   我们只认 `Cmd+A/C/V/X/Z` 且调 **CEF API**(`frame.copy()` 等)⇒ Monaco/CodeMirror/xterm.js 等
   页面自定义快捷键失效。

### P1
5. 手势/缩放全缺(magnify/smartMagnify/swipe/touch、`zoomLevel`)。参考 `CefMetalHostView.swift:417-513`。
6. JS 对话框无 handler(参考用 NSAlert)⇒ `confirm` 静默、`beforeunload` 无提示。
7. 焦点不跟随窗口 key 状态(参考观察 `didBecomeKey/didResignKey`)⇒ 切后台页面仍显示 caret。
8. `selectedRange` 只喂 composition,未接 `on_text_selection_changed`(我们代码注释已自认)。
9. `screen_point` 原点疑似相反(**未实证**):参考左上,我们左下 AppKit。若我们错 ⇒ JS `event.screenX/Y` 垂直镜像。
10. 权限请求无 handler(getUserMedia/geolocation 一律失败)。参考 `BrowserClient.swift:613-655`。
11. 帧节奏:参考 `external_begin_frame_enabled=1` + CADisplayLink + 实现 `on_schedule_message_pump_work`;
    我们 begin-frame 默认关、固定 1/60s 泵、不实现该 pump 调度(计划 §5 记了取舍,**"忽略 pump 调度"未记**)。

### P2
12. `screen_info.rect` 用 view rect 而非屏幕 frame;13. 无障碍未启用;14. `depth=32` vs 参考 24;
15. `on_before_popup`/`on_open_urlfrom_tab` 未实现(靠 init JS 覆写 `window.open`;绕过时会弹原生窗口);
16. 侧键 3/4 未映射 back/forward;17. `cursor:none` 显示为箭头;18. 鼠标事件缺 `pressedMouseButtons` 位;
19. 零碎:`contentsScale` 只在创建时设(§3 已记)、popup 隐藏不清 rect、`viewDidEndLiveResize` 无收尾 invalidate。

## 2 我们做了但参考没有(建议保留)

真透明(windowless `background_color` alpha=0)、`validAttributesForMarkedText` 返回三个属性(参考为空,**我们更对**)、
CapsLock 设备无关掩码、隐藏超时 CDP 冻结、实例代际号、借用纪律。

## 3 未验证疑点(有代码依据、无实证)

① `screen_point` 原点;② `on_before_context_menu` 是否真被调用过(`cef_support.m:483` 断言"一次都没被调用",
与参考证据冲突 —— 可用现成的 debug 日志验证);③ ~~无 download handler 时 CEF 默认是静默落盘还是失败~~
→ **现象已确认、成因未定(2026-09-22)**:本 build 下"未处理"⇒ **默认目录静默落盘**(`~/Downloads`,
三条 Chromium 下载记录);头文件写的 "cancel with Alloy style" 与现象不一致,最可能是 CEF 152 尚无该
分支(**未验证** —— 我们的浏览器是 Alloy,与上游 master 的代码路径存在张力);见
[OSR-T10-DOWNLOAD.md](OSR-T10-DOWNLOAD.md) §1;
④ `ime_commit_text` 传 nil(参考)vs 我们传 `u32::MAX`(我们的注释称 capi 会把 null 退化成 (0,0) ——
若成立则参考会踩 T2 记录的问题,说明该注释可能不准);⑤ OSR 下 `show_dev_tools` 是否真有可见窗口。

## 4 参考测试断言到、我们零覆盖

`shouldForwardKeyEquivalent` 策略(`OSRInputConformanceTests.swift:20-38`)、`otherMouseAction` 3/4(`:88-95`)、
拖放掩码(`OSRInputPassthroughTests.swift:17-36`)、`CefOSRViewInfo` 的 screenOrigin/screenRect(`OSRLogicTests.swift:29-38`)、
JSDialog/权限映射(`HandlerLogicTests.swift:12-70`)。
我们的 `cef_backend_tests.rs` 只覆盖:cef_modifiers / windows_key_code / is_modifier_pressed / is_key_pad /
deferred_key_plan / ime_replacement_range / cursor_semantic。

## 5 实机验证与用户决定(2026-09-22,实例 pid 83540 / OSR 模式)

| 项 | 用户实测 | 决定 |
|----|----------|------|
| P0-1 右键菜单 | — | **保持现在的两项(重新加载/检查元素)就够** ⇒ **不实现 `run_context_menu`**。于是 `on_before_context_menu`(填模型)、`on_context_menu_command`、`WebviewContextMenu`、`handle_menu_command` 属**已确认不用**的死代码 —— 建议单独一轮清理(改动虽小但跨 Rust/ObjC/回调声明,需重建验证) |
| P0-2 下载 | **没有保存面板,但下载直接完成**(CEF 默认行为可用,只是无 UI) | **已按"补齐面板"落地(2026-09-22,当日实机验证通过)**:接 `download_handler` → 共用 `NSSavePanel`、可取消、终态日志。取消**不能**靠"不执行 callback"(实测会卡在 target-pending),改用 `CefDownloadItemCallback::Cancel()`。证据与踩坑见 [OSR-T10-DOWNLOAD.md](OSR-T10-DOWNLOAD.md)。**进度 UI 未做**(待产品决定) |
| P0-3 拖放 | **两个方向都没反应**(与预期一致) | 记录为已确认缺口(计划外,后续路线图) |
| P0-4 编辑快捷键转发 | 未测 | **仅记录**(页面 JS 收不到按键;我们走 CEF API) |
| P1-5 手势/缩放 | 未测 | **仅记录** |
| P1-7 焦点随窗口 key | **确实一直有光标**(窗口失焦后页面仍显示光标,缺口成立) | 记录为已确认缺口 |

**验证方式说明**:上表为**用户实机观察**;"P0-1 死代码""P0-3 无响应"与代码判据一致(见 §0/§2);
P0-2 的"下载直接完成"说明 CEF 默认下载管道可用 ⇒ 缺的只是保存面板/进度 UI(原判断"完全缺失"应细化为
"缺 handler,但默认行为会静默落盘")。**P0-2 已于 2026-09-22 补齐并通过实机验证**(见 §1 第 2 条与
[OSR-T10-DOWNLOAD.md](OSR-T10-DOWNLOAD.md))。
