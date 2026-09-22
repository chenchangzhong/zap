# OSR T7:输入法(IME)落地主仓(通过)

> 目的:把 T2 spike 验证过的 `NSTextInputClient` → `ime_set_composition`/`ime_commit_text`
> 链路移进主仓,让 dsh pane 能真正打中文(见 [OSR-PLAN.md](../../OSR-PLAN.md) T7)。
> 结论:**通过** —— 真机拼音→候选→中文上屏 + 候选框跟随光标,用户实机确认;
> 英文输入与 Cmd 快捷键无回归。

## 1 实现落点

| 文件 | 改动 |
|------|------|
| [app/src/platform/mac/objc/cef_support.m](../../../../app/src/platform/mac/objc/cef_support.m) | `WarpCefOsrView` 实现 `<NSTextInputClient>`:`setMarkedText:`/`insertText:`/`unmarkText`/`hasMarkedText`/`markedRange`/`selectedRange`/`attributedSubstringForProposedRange:`/`validAttributesForMarkedText`/`firstRectForCharacterRange:`/`characterIndexForPoint:`/`doCommandBySelector:`;`keyDown:` 改为 **deferred 模型**;新增 `performKeyEquivalent:`(只认 Cmd+A/C/V/X/Z);`warp_cef_osr_view_set_ime_bounds()` 接收候选框几何。 |
| [app/src/browser/cef_backend.rs](../../../../app/src/browser/cef_backend.rs) | `WarpCefOsrKeyInput`(按键 + 输入法累积状态)与 `WarpCefOsrImeCommand` 两个结构体 + 两个新回调;`osr_key_trampoline` 执行 [`deferred_key_plan`](../../../../app/src/browser/cef_backend.rs) 的决策;`osr_ime_trampoline` 处理按键外直接调用;render handler 新增 `on_ime_composition_range_changed`;`ime_replacement_range`/`ime_underline` 两个辅助。 |
| [app/src/browser/cef_backend_tests.rs](../../../../app/src/browser/cef_backend_tests.rs) | 新增 2 个单测:deferred 决策(普通键/组合/上屏/finish-vs-cancel)、`ime_replacement_range` 永不传 NULL。 |

**为什么必须用 deferred 模型**(参考实现 CefSwift / cefclient 的
`HandleKeyEventBefore/AfterTextInputClient` 同款):`keyDown:` 里先 `interpretKeyEvents:`
让输入法回调,但回调**只累积状态**;`keyDown:` 末尾再一次性决定:
普通按键 → KEYDOWN+CHAR(保住页面 JS `keydown` 与光标移动)、组合中 → `ime_set_composition`、
上屏 → `ime_commit_text`。若像 T2 spike 那样"`insertText:` 直接转发",**普通字母也会走
`ime_commit_text`**,页面收不到 `keydown` —— 等于把 T5 做好的两段式弄丢。

## 2 交付前审核发现的两个真问题(先验证再修)

### 2.1 T7 改写 `keyDown` 后,T5 的 CapsLock 兜底失效(我自己引入的回归)

`keyDown` 改走 deferred 路径后,旧路径(`OSR_EVENT_KEY` 且 `key_type==0`)的
`edit_command_for_key` 拦截不再被调用 —— 已成死代码。用户实测复现:
**CapsLock 开着时 Cmd+A/C/V/X/Z 只有 A 生效**。

### 2.2 根因:`NSEventModifierFlags` 的低 16 位是设备相关位

`edit_command_for_key` 里"除 Cmd/Shift/CapsLock 外没有别的修饰键"的判断用了**原始**
`event.modifierFlags`,而实测**每次按键都带低位噪声**(`0x100` =
`kCGEventFlagMaskNonCoalesced`、`0x8` = 左 Command 的设备相关位、`0x2` 左 Shift),
于是 `others != 0` 永远为真 ⇒ 拦截形同虚设。

日志判据(CapsLock 开着时):

```
[cef] osr 1: send_key_event(KEYDOWN+CHAR) code=8 chars=Some("c")   ← Cmd+C 被当普通键发给页面
[cef] osr 1: send_key_event(KEYDOWN+CHAR) code=7 chars=Some("x")
[cef] osr 1: send_key_event(KEYDOWN+CHAR) code=9 chars=Some("v")
(没有任何 edit command 日志)
```

T5 那次"修好了"其实是 **CapsLock 关着时走 zap 窗口的响应者动作**(`mods == Command`
精确比较成立),掩盖了这个 bug。

**修法**:
1. 加 `NS_MOD_DEVICE_INDEPENDENT_MASK` + `device_independent_modifiers()`,
   判修饰键组合前先过滤(单测覆盖"带 0x100/0x8 噪声仍能命中");
2. 补上计划里要求的视图级 `performKeyEquivalent:`(排在 AppKit 菜单**之前**),
   只认 Cmd+A/C/V/X/Z,其余原样交回 —— zap 自己的 Cmd+T/Cmd+1..9 不受影响;
   它里面同样用设备无关掩码。

修复后实测(stderr + 应用日志):

```
[osr] performKeyEquivalent 拦下 'a' → edit 3     ← AppKit 确实会走到视图这一层
[osr] performKeyEquivalent 拦下 'x' → edit 1
[osr] performKeyEquivalent 拦下 'c' → edit 0
[osr] performKeyEquivalent 拦下 'z' → edit 4
(应用日志同期:edit command 3/1/2/0/4 齐全)
(同期 ActivateTabByNumber(1/2/3) —— Cmd+1..9 仍归 zap,Cmd+T 正常开新标签)
```

## 3 验证

### 3.1 静态检查与单测

```bash
cargo check -p warp                                     # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo check -p warp --features cef_webview   # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend)'                                # 15/15 passed
```

### 3.2 真机(自建 ZapCEF 实例,OSR 模式;用户实机确认 + 日志)

| 项 | 结果 | 证据 |
|----|------|------|
| 拼音 → 候选 → 中文上屏 | ✅ | `ime_set_composition "jing'ta" → "jing'ta's'fa"`(微信输入法的分词符)→ `ime_commit_text "敬他是发"` → `ime_cancel_composition`;第二次 `"a's'fa's'j"` → `"啊沙发睡觉"` |
| 候选框跟随光标 | ✅ | `on_ime_composition_range_changed sel=Some((3,15)) bounds=12 Rect{x:538,y:739,w:3,h:20}`(随组合增长而变化),视图 `firstRectForCharacterRange:` 换算成屏幕坐标交给输入法 |
| 英文输入无回归 | ✅ | 用户确认;普通键走 `send_key_event(KEYDOWN+CHAR)` 分支 |
| Cmd+C/V/A/X/Z(CapsLock 关) | ✅ | 用户确认;`edit command 3/1/2/0/4` |
| Cmd+A/C/V/X/Z(**CapsLock 开**) | ✅ | 用户确认;`performKeyEquivalent 拦下 …` + `edit command` |
| Cmd+T / Cmd+1..9 仍归 zap | ✅ | `ActivateTabByNumber(1/2/3)` 日志,新标签正常 |
| 无崩溃 | ✅ | 无新增 `.ips` |

## 4 已知边界(不算本次缺陷)

1. 非 composition 的**文档选区**未接(`selectedRange()` 只在 composition 期间有值):
   需要 `on_text_selection_changed`,当前不影响输入法组合与候选框。
2. `<select>` 等页面内弹层仍是 T6 范围(`on_accelerated_paint` 的 POPUP 分支直接忽略)。
3. **Cmd+Z 在 OSR 下归页面**(计划要求,已验证);windowed 路径此键走 zap 的
   Edit▸Undo(未逐项对照验证是否为同一行为)——若要求两模式完全一致,需要改
   `crates/warpui` 的快捷键策略,不在本任务范围。
4. 兜底保留:按键按下路径里仍有一次编辑快捷键拦截(设备无关掩码),用于
   "AppKit 没走 key equivalent 阶段"的情形 —— 与视图级 `performKeyEquivalent:` 互斥,
   不会重复执行。

## 5 复跑

```bash
cd /path/to/zap
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 日志:tail -f ~/Library/Logs/zap.log | grep -a 'ime_\|edit command'
```
