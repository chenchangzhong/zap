# OSR Spike B:windowless 下自建 NSView 的 IME 链路(通过)

> 目的:验证 `OSR-PLAN.md` T2 —— OSR 宿主 NSView 实现 `NSTextInputClient` 并转发到
> `CefBrowserHost::ime_set_composition`/`ime_commit_text` 后,中文输入法能否真正打进去。
> 结论:**通过**。并挖出一个"传 NULL replacement_range ⇒ 整条 composition 被静默丢弃"的
> 硬坑(不是可选项,**T4/T7 必须照此实现**)。

## 1 实现落点

| 文件 | 改动 |
|------|------|
| [probes/osr_host.m](../../../tools/cef-spike/probes/osr_host.m) | 宿主视图由 `NSView` 改为 `OsrHostView : NSView <NSTextInputClient>`:实现 `setMarkedText:selectedRange:replacementRange:`/`insertText:replacementRange:`/`unmarkText`/`hasMarkedText`/`markedRange`/`selectedRange`/`attributedSubstringForProposedRange:`/`validAttributesForMarkedText`/`firstRectForCharacterRange:actualRange:`/`characterIndexForPoint:`/`doCommandBySelector:`/`keyDown:`(`interpretKeyEvents:`);经 4 个 C 函数指针回调 Rust(不直接链接 Rust 符号)。候选框锚点用 CEF 回报的 composition 几何缓存后换算成屏幕坐标。 |
| [src/bin/osr-probe.rs](../../../tools/cef-spike/src/bin/osr-probe.rs) | 回调表 `OsrImeCallbacks` + 4 个 trampoline → `CefBrowserHost::ime_set_composition`/`ime_commit_text`/`ime_finish_composing_text`/`ime_cancel_composition`;`on_ime_composition_range_changed` 回传选区与逐字位置;`LoadHandler::on_load_end` 补焦点;`on_after_created` 让宿主视图成为 first responder 并 `set_focus(1)`。 |
| [probes/ime_page.html](../../../tools/cef-spike/probes/ime_page.html) | 新增:`<textarea>` + composition/input 事件与焦点状态 POST 到 `probes/loopback_receiver.py`(127.0.0.1:9911)。 |

参考映射:`CEFSWIFT-EVALUATION.md` §1.4 / CEF 自带 mac OSR 客户端
`tests/cefclient/browser/text_input_client_osr_mac.mm`(映射逐条对齐;`relative_cursor_pos`
取 0 = 光标落在提交文本末尾,经 Chromium `InputMethodController::ComputeAbsoluteCaretPosition`
= `起点 + 长度 + relative` 核对)。

## 2 关键发现:NULL `replacement_range` 会废掉整条 IME(否决级陷阱)

`CefBrowserHost::ImeSetComposition` 的 `replacement_range` 是**可选参数**,但 cef-rs 的
`Option::None` 会传 **NULL 指针**,而 CEF 的 capi 对空指针退化成 `CefRange()` = **(0,0)**:

```
gfx::Range range(0, 0) → IsValid() 为真 → WebRange(0,0)
→ WebInputMethodControllerImpl::SetComposition 里执行 web_frame_->SelectRange(...)
→ <textarea> 没有文档级编辑器,SelectRange 打断焦点 → composition 被静默丢弃
```

现象:ObjC 侧 `setMarkedText`/`insertText` 全部照常回调,Rust 侧 `ime_set_composition`
也确认已调用到 `CefBrowserHost`,但页面**没有任何** composition 事件、`ime_commit_text`
也不上屏、`on_ime_composition_range_changed` 一次都不回调 —— 全程无报错。

定位过程(可复现):
1. `send_key_event(CHAR 'X')` 能进 textarea ⇒ 输入通道本身是通的,只有 IME 通道不通;
2. CDP `Input.imeSetComposition` + `Input.insertText` 能打出 `nihao`→`你好` ⇒ **渲染器 IME 正常**,
   问题在 CEF 浏览器进程侧的参数;
3. 改用 `CefRange::InvalidRange()`(两个 `0xFFFFFFFF`,CEF 自带 mac 客户端同款)⇒ 立刻全通。

**T4/T7 结论**:转发 IME 时 `replacement_range` 永远传显式 `(0xFFFFFFFF, 0xFFFFFFFF)`,
不能传 NULL/`None`。

## 3 证据 A(确定性,可机器复跑):直接调用 `NSTextInputClient`

```bash
cd tools/cef-spike
CEF_PATH="$HOME/.local/share/cef" cargo run --bin make-bundle -- osr-probe -o target/osr
codesign --force --deep -s - target/osr/osr-probe.app
python3 probes/loopback_receiver.py > /tmp/ime_t2_server.log 2>&1 &
PROBE_TRANSPARENT=1 PROBE_IME_SELFTEST=1 target/osr/osr-probe.app/Contents/MacOS/osr-probe \
  --url="file://$PWD/probes/ime_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=0.5 --exit-after=14 --log=/tmp/osr_ime_A.log
```

探针日志(`/tmp/osr_ime_A.log`,去掉帧率/像素噪声):

```
[osr] NSTextInputClient setMarkedText "nihao" sel={5,0} rep={9223372036854775807,0}
[osr] ime_set_composition text="nihao" sel=5..5 rep=-1..-1 utf16_len=5
[osr] on_ime_composition_range_changed sel=Some((0, 5)) bounds=5 Some(Rect { x: 31, y: 104, width: 12, height: 25 })
[osr] NSTextInputClient setMarkedText "ni hao" sel={3,3} rep={9223372036854775807,0}
[osr] ime_set_composition text="ni hao" sel=3..6 rep=-1..-1 utf16_len=6
[osr] on_ime_composition_range_changed sel=Some((0, 6)) bounds=6 Some(Rect { x: 31, y: 104, width: 12, height: 25 })
[osr] NSTextInputClient insertText "你好" rep={9223372036854775807,0}
[osr] ime_commit_text text="你好" rep=-1..-1
```

回环接收端(页面内 `<textarea>` 的真实事件与内容):

```
[zap-ipc-received] ime.compositionstart value=""
[zap-ipc-received] ime.compositionupdate value="" data="nihao"
[zap-ipc-received] ime.input value="nihao" isComposing=true
[zap-ipc-received] ime.compositionupdate value="nihao" data="ni hao"
[zap-ipc-received] ime.input value="ni hao" isComposing=true
[zap-ipc-received] ime.compositionupdate value="ni hao" data="你好"
[zap-ipc-received] ime.input value="你好" isComposing=true
[zap-ipc-received] ime.compositionend value="你好" data="你好"
```

⇒ ObjC `NSTextInputClient` → Rust → CEF → 渲染器 → 页面 textarea 全链路成立,且
`on_ime_composition_range_changed` 有回调(候选框位置数据源可用)。

## 4 证据 B(人工,真实输入法):微信输入法拼音→候选→上屏

```bash
PROBE_TRANSPARENT=1 target/osr/osr-probe.app/Contents/MacOS/osr-probe \
  --url="file://$PWD/probes/ime_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=0.5 --exit-after=240 --log=/tmp/osr_ime_manual.log
# 人工:点输入框 → 打 nihao → 空格选第一个候选
```

探针日志(真实输入法的 composition 串带分词符,可见确为输入法而非直接键入):

```
[osr] keyDown keyCode=45 chars=n
[osr] NSTextInputClient setMarkedText "n" sel={1,0} rep={9223372036854775807,0}
[osr] ime_set_composition text="n" sel=1..1 rep=-1..-1 utf16_len=1
[osr] on_ime_composition_range_changed sel=Some((0, 1)) bounds=1 Some(Rect { x: 31, y: 104, width: 12, height: 25 })
[osr] firstRectForCharacterRange → screen=(291.0,531.0) 12.0x25.0(cef DIP=(31.0,104.0) 12.0x25.0 cached=1)
... (n → ni → ni'h → ni'ha → ni'hao,每次都有 setMarkedText + composition range 回调)
[osr] keyDown keyCode=49 chars= 
[osr] NSTextInputClient insertText "你好" rep={9223372036854775807,0}
[osr] ime_commit_text text="你好" rep=-1..-1
```

回环接收端:

```
[zap-ipc-received] ime.compositionupdate value="" data="n"
[zap-ipc-received] ime.input value="n" isComposing=true
[zap-ipc-received] ime.compositionupdate value="ni'hao" data="ni'hao"
[zap-ipc-received] ime.input value="ni'hao" isComposing=true
[zap-ipc-received] ime.compositionupdate value="ni'hao" data="你好"
[zap-ipc-received] ime.input value="你好" isComposing=true
[zap-ipc-received] ime.compositionend value="你好" data="你好"
```

⇒ **拼音 → 候选 → 中文上屏**在 OSR 下成立(人工确认:输入框里出现「你好」);
候选框锚点链路也被真实输入法实际调用(`firstRectForCharacterRange` 有日志,DIP→屏幕坐标换算正确)。

## 5 边界与已知限制(给 T5/T7)

1. **合成 NSEvent 驱动不了 IMK 输入法**:进程内 `keyEventWithType:` + `[window sendEvent:]`
   能让 AppKit 走完 `interpretKeyEvents:`(ASCII 会正常上屏),但第三方输入法(微信输入法)会把
   这类事件当普通按键透传、不进入 composition ⇒ **IME 只能人工验证**,自动化只能覆盖 §3 那一段。
2. T2 只做了 IME 与焦点,**普通按键未转发给 CEF**(`keyDown:` 只喂 `interpretKeyEvents:`,
   不做 KEYDOWN+CHAR 两段式)⇒ ASCII 输入目前靠 `insertText → ime_commit_text` 生效,
   JS `keydown` 事件不触发;T5 补。
3. `selectedRange()` 只在 composition 期间有值(来自 `on_ime_composition_range_changed`),
   非 composition 的文档选区需要 `on_text_selection_changed`,T5 补。
4. 焦点必须在**导航完成后**补一次(CEF 导航后会静默丢焦点,`chromiumembedded/cef#3870`);
   本次实测通过的是 `was_hidden(0)` + `set_focus(0)` + `set_focus(1)`(对齐 cefclient `Show()`)。
5. 探针的 CEF 缓存根改到系统临时目录(`--cache-dir`,默认 `$TMPDIR/cef-spike-osr-cache`):
   默认的 `~/Library/Application Support/CEF/User Data` 在受限环境下不可写,会让浏览器进程
   直接起不来(`Failed to create SingletonLock`)。

## 6 复跑命令速查

```bash
cd tools/cef-spike
CEF_PATH="$HOME/.local/share/cef" cargo run --bin make-bundle -- osr-probe -o target/osr
codesign --force --deep -s - target/osr/osr-probe.app
python3 probes/loopback_receiver.py > /tmp/ime_server.log 2>&1 &   # 端口 9911

# 确定性自检(直接调用 NSTextInputClient)
PROBE_TRANSPARENT=1 PROBE_IME_SELFTEST=1 target/osr/osr-probe.app/Contents/MacOS/osr-probe \
  --url="file://$PWD/probes/ime_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=0.5 --exit-after=14 --log=/tmp/osr_ime_A.log

# 合成按键(只用于确认输入通道;输入法不会进入 composition)
PROBE_TRANSPARENT=1 PROBE_IME_KEYTEST=1 target/osr/osr-probe.app/Contents/MacOS/osr-probe \
  --url="file://$PWD/probes/ime_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=0.5 --exit-after=18 --log=/tmp/osr_ime_B.log

# 人工验证(真实输入法):去掉两个自检开关,自己点输入框打拼音
```
