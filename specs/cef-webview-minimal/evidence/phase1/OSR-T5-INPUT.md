# OSR T5:输入与编辑命令(通过)

> 目的:OSR 宿主视图转发鼠标/滚轮/键盘/焦点,编辑命令走 focused frame,并让
> Cmd+A/C/V/X 在页面里生效(见 [OSR-PLAN.md](../../OSR-PLAN.md) T5)。
> 结论:**通过** —— 真机逐项验过(用户确认 + 日志证据),并挖出 3 个"只有真机才暴露"的问题。

## 1 实现落点

| 文件 | 改动 |
|------|------|
| [app/src/platform/mac/objc/cef_support.m](../../../../app/src/platform/mac/objc/cef_support.m) | `WarpCefOsrView` 增加:鼠标(move/down/up/drag/right/other/enter/exit)、滚轮(CGEvent point delta + 亚像素余量)、键盘(keyDown/keyUp/flagsChanged)、`acceptsFirstResponder`/`becomeFirstResponder`/`resignFirstResponder`、tracking area、光标(`cursorUpdate:`/`resetCursorRects`)、响应者链编辑动作(`copy:`/`cut:`/`paste:`/`selectAll:`/`undo:`/`redo:`)。事件只做"原始采集",经一个回调结构体交给 Rust。 |
| [app/src/browser/cef_backend.rs](../../../../app/src/browser/cef_backend.rs) | `WarpCefOsrInputEvent`(与 ObjC 逐字段对齐)+ 3 个回调(mouse/key、edit command、focus);修饰键位映射 `cef_modifiers`、mac→Windows 键码表 `windows_key_code`、小键盘/修饰键状态判定、光标语义归一化;KEYDOWN+CHAR 两段式;编辑命令 → `focused_frame`(缺省回退主 frame);新增 `CefDisplayHandler::on_cursor_change` 与 `CefKeyboardHandler::on_key_event`。 |
| [app/src/browser/cef_backend_tests.rs](../../../../app/src/browser/cef_backend_tests.rs) | 新增 6 个纯逻辑单测(修饰键位、键码表、修饰键状态、小键盘、DIP 坐标截断、光标语义、编辑快捷键判定)。 |

## 2 真机暴露的三个问题(都值得记住)

### 2.1 `-[NSEvent clickCount]` 对 enter/exit 事件会抛 ObjC 异常 ⇒ 进程直接崩

`mouseEntered:` 里读 `event.clickCount` 抛 `NSInternalInconsistencyException`,AppKit
`reportException:` → SIGTRAP。同类:`characters`/`isARepeat` 只对 keyDown/keyUp 有效,
`flagsChanged` 里读同样危险。**修法**:按事件类型取字段(cefclient 的 `getKeyEvent` 也是这么做的)。

### 2.2 CEF 会把"页面未处理的按键"交给**应用主菜单** ⇒ 在页面里打 `f` 会把 zap 窗口切全屏

源码链路(逐段核对过 CEF 7977 分支):

1. 页面没有 `preventDefault()` 的按键,渲染器回报"未处理";
2. `AlloyBrowserHostImpl::HandleKeyboardEvent` → 客户端 `CefKeyboardHandler::OnKeyEvent`(我们当时没实现)
   → 返回 false → `platform_delegate_->HandleKeyboardEvent(event)`;
3. `CefBrowserPlatformDelegateNativeMac::HandleKeyboardEvent`:
   `[[NSApp mainMenu] performKeyEquivalent:合成的 NSEvent]`
   (合成事件来自 `TranslateWebKeyEvent`,带我们的字符/键码);
4. zap 的窗口菜单经 `NSApplication::setWindowsMenu:` 让 AppKit 自动补了 **"Enter Full Screen"**,
   它的 `keyEquivalent` 就是**裸 `f`**(Fn+F,修饰键掩码 = `NSEventModifierFlagFunction`)
   —— 菜单匹配命中 ⇒ **全屏**。

判据(实测):`f` 的 keyDown 先到我们的 view(有日志),紧接着 `window resized`;
而我们的 `performKeyEquivalent:` **从未被调用**(探针证明)——所以触发点不在视图的
快捷键路径,而在 CEF 主动查菜单这一步。

**修法(CEF 官方钩子)**:实现 `CefKeyboardHandler::on_key_event` 返回 1(已认领)。
这些键我们本来就已转发给渲染器,页面才是它们的主人;zap 自己的快捷键(Cmd+Ctrl+F 全屏、
Cmd+T 新标签等)在**窗口的 key equivalent 阶段**就被处理,根本到不了这里,不受影响。

### 2.3 开着 CapsLock 时 Cmd+A/C/V/X 失效(zap 窗口判定的精确比较)

`WarpWindow::performKeyEquivalent:` 对嵌入视图用的是
`mods == NSEventModifierFlagCommand` **精确相等**,而 `mods` 里还会带上
`NSEventModifierFlagCapsLock`(实测 `ns_flags=0x110108`)⇒ 这组编辑快捷键被跳过、
当成普通按键转发给页面 ⇒ 复制/全选/剪切全部失效(用户实机反馈"5 不通过")。

**修法**:在宿主视图侧兜住 Cmd+A/C/V/X/Z(`edit_command_for_key`,纯函数 + 单测),
命中就走 focused frame 的编辑命令、不再转发按键。这条也顺带满足计划里
"`performKeyEquivalent` 处理 Cmd+A/C/V/X/Z"的要求 —— 但**没有**在视图里实现
`performKeyEquivalent:`:那会抢在菜单之前吞掉 zap 的 Cmd+T/Cmd+1..9 等快捷键
(参考实现 CefSwift 的策略是"除少数应用级键外全部转发并消费",与 zap 的快捷键体系不兼容)。
`window.m` 那处精确比较是**既有隐患**(wry 路径靠 WKWebView 自己的快捷键处理掩盖了),
本次只记录、不改动共享代码。

## 3 验证

### 3.1 静态检查与单测

```bash
cargo check -p warp                                     # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo check -p warp --features cef_webview   # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend)'                                # 13/13 passed
```

### 3.2 真机逐项(自建 ZapCEF 实例,OSR 模式;用户实机确认 + 日志)

| 项 | 结果 | 证据 |
|----|------|------|
| 鼠标点击 → 页面获得焦点、能打英文 | ✅ | `send_mouse_click button=0 up=0/1 count=1 (750,758)` + `send_key_event ... chars=Some("a")` |
| 拖选文字 | ✅ | 用户确认;`mouseDragged` 走 `send_mouse_move` |
| 滚轮 | ✅ | `send_mouse_wheel (15,-41) at (824,631)` 连续事件 |
| 鼠标移入/移出 | ✅ | 用户确认;tracking area + `mouse_leave` 分支 |
| 光标形状(链接手型) | ✅ | 用户确认;`on_cursor_change` → `warp_cef_osr_view_set_cursor` |
| 英文输入 | ✅ | 用户确认 + 上面的 key 日志 |
| `f` 不再切全屏 | ✅ | 修复后按 `f`:keyDown+**keyUp** 都转发、日志里**没有** `window resized` |
| Cmd+C / Cmd+V / Cmd+A / Cmd+X | ✅ | 用户确认;日志 `edit command 0/2/3/1` 四种命令齐全 |
| 无崩溃 | ✅ | 全程无新增 `.ips`(唯一一份是 §2.1 修复前那次) |

> 中文输入(拼音→候选→上屏)**不在 T5 范围**:主仓宿主视图尚未实现 `NSTextInputClient`,
> 属 T7(T2 spike 已验证机制可行,证据见 [OSR-SPIKE-B.md](OSR-SPIKE-B.md))。

## 4 交付前代码审核(2026-09-22)发现并修掉的问题

审核逐行看了 T4/T5 的改动,并**先验证再修**;三处都是真问题(附触发条件),另加一处与参考实现的保真补齐:

| # | 问题 | 触发条件 | 修法 |
|---|------|----------|------|
| 1 | **use-after-free**:`destroy()` 里 `close_browser(1)` 是**异步**的(CEF 文档明确),但我们紧接着就 `warp_cef_osr_view_release` 释放了 NSView;render handler 持有同一个 `OsrState`,`view` 指针不清空 ⇒ 关闭完成前的任何一帧绘制(`on_accelerated_paint`/`on_paint`)都会对已释放对象发消息。 | 关 pane/关窗口后、CEF 真正销毁浏览器之前的绘制(页面有动画时几乎必现窗口) | `take_and_release_osr_view()`:**先置空共享指针,再释放**;所有权置空后所有回调走 null 早返回,渲染/光标/几何安全降级 |
| 2 | **视图泄漏**:浏览器创建失败时,管理端只 `cef_entries.remove(id)`,`cef_backend` 侧的 WEBVIEWS 条目(含**先于浏览器创建**的 OSR 宿主视图)无人清理 ⇒ NSView 永久留在容器里,同 id 重建还会再叠一层。 | `browser_host_create_browser` 返回 != 1 | `browser_web_view.rs` 的失败清理改为调 `cef_backend::destroy(id)`(幂等,顺带释放视图与状态) |
| 3 | **残留最后一帧**:`on_before_close` 只清 browser 句柄、不处理自建视图 ⇒ 页面自行关闭(`window.close()`)后洞内继续显示旧画面(windowed 路径会摘掉 CEF 视图,两种模式不一致) | CEF 主动销毁浏览器(页面自关等) | `on_before_close` 里按**代际**取出 `state.osr` 并在借用外释放(迟到旧回调不得释放新视图) |
| 4 | 保真补齐:窗口未激活时的第一次点击只用于激活窗口,页面收不到 | 从别的 app 切回来点输入框 | 加 `acceptsFirstMouse:` 返回 YES(与 WKWebView / 参考实现一致) |

**修完再审**并做了真机确认(关键点:这三处都走"关闭"路径,前几轮测试没覆盖):

```
2026-09-22T02:06:01Z [INFO] [warp::browser::cef_backend] [cef] 关闭 CEF(CefShutdown)
2026-09-22T02:06:01Z [INFO] [warp::browser::cef_backend] [cef] webview 1 closed (gen 1)
```

优雅退出全程无新增 `.ips`、无残留进程(用户实机确认"关闭后正常,无异常");
两种 cfg `cargo check` 0 warning、`nextest -E 'test(cef_backend)'` 13/13 复跑通过。

## 5 复跑

```bash
cd /path/to/zap
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 打开 dsh pane 后:tail -f ~/Library/Logs/zap.log | grep -a 'osr 1:'
```
