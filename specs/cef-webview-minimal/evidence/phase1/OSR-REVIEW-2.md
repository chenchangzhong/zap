# OSR 交付前独立审核 + 修复轮(第二轮)

> 范围:`433bdb206..e23a47bfe`(T2 spike / T4+T5 / T7)加审核期间的工作树改动。
> 方式:**两位独立审查者**(第一位做了缺陷清单,第二位按 canonical 评审模板给出分级结论),
> 每条结论都由我先复核证据、再决定是否修;修完再审、再实机验证。

## 1 审核发现的**真实缺陷**与修复

### 1.1 【Critical·既有】解冻路径嵌套 `borrow_mut` ⇒ 必 panic

`set_visible()` 在 `with_browser` 闭包内(已持有 `WEBVIEWS.borrow_mut()`)又调
`WEBVIEWS.with(|map| map.borrow_mut()...)` 去清 `frozen` —— 解冻成功时必然
`BorrowMutError` panic。触发:**隐藏 CEF pane 超过 `freeze_after_secs` 后切回**
(即 OSR 验收项 3 的"隐藏/冻结/切回")。判据:该行是本轮改动之外的既有代码,
全仓只有这一处这种嵌套(脚本扫描确认)。修法:直接用闭包已给出的 `state.frozen = false`。

### 1.2 【高】`webview_init.js` 在 CEF 模式**从未注入**(引导脚本缺席)

该文件的文件头写着"CEF 在 render 进程于 OnContextCreated 注入",但 **CEF 后端从来没有实现
`on_context_created`** ⇒ 引导脚本缺席,连带:

- `window.__restoreFocused` 不存在(切回/重载后无法恢复输入框 DOM 焦点);
- 早期 IPC 队列 `window.__zapIpcQueue` 不存在(文档开始 → shim 就位之间的 zapRpc 永久丢失);
- `warp:webview-focusin`/`warp:webview-mousedown` 不上报(地址栏与页面双光标问题);
- `window.open` 外链拦截失效。

修法:实现 `wrap_render_process_handler!` + `App::render_process_handler()`,在
`on_context_created` 里 `frame.execute_java_script(webview_init.js)`(与 wry 一致:
注入**所有** frame)。**实机判据**(临时探针 `document.title` 回传,已删):
`PROBE restore=function ... queue=1 ipc=1` —— 修复前这些都不存在。

### 1.3 【高】OSR 下 CEF 的 `SetFocus` 不会把自建视图设为 first responder

`CefBrowserPlatformDelegateNativeMac::SetFocus` 只对 **content native view** 调
`makeFirstResponder`,windowless 下它是 null ⇒ 只调 `host.set_focus(1)` 的话,键盘事件
仍发给 WarpHostView,页面收不到输入(要先用鼠标点一下页面才行)。这与 wry 的
`WebView::focus()`(内部就是 `makeFirstResponder`)不一致,且 spike 里本来有
`osr_host_view_focus`,移植时丢了。
修法:新增 `warp_cef_osr_view_focus()`(带 `_syncingFocus` 可重入守卫,避免
`makeFirstResponder` → `becomeFirstResponder` → `focus()` 递归),在 `focus(id, true)` 里调用。

### 1.4 【高】新开 pane 后"不点页面直接打字没有任何反应"(pane 层)

链条(实测 + 代码):`DshPaneView::focus_contents` 在 `webview_visible()` 为假(加载中)
时**早返回并丢弃**这次聚焦请求 → 加载完成后的补焦点分支用
`ctx.is_self_or_child_focused()` 判断 ⇒ 恒假 ⇒ 页面永远拿不到键盘焦点。**wry 后端同样如此**。
修法:加载完成时补上 pane 级权威判据 `PaneFocusHandle::is_focused(app)`;
真正切走时它也为假,不会抢焦点。

### 1.5 【中】CEF 导航后丢焦点(计划硬约束 2)在主仓未兑现

`on_load_end` 只注入了 shim + 通知 page_loaded,没有 spike 实测通过的
`was_hidden(0)+set_focus(0)+set_focus(1)`。修法:`on_load_end` 在"本视图已是 first responder"
时补一次(重载场景);`focus(id, true)` 在"焦点**首次**落到页面上"时补一次(初次打开场景,
且避免页面内每次点击都产生 blur/focus 抖动 —— 页面菜单 onBlur 会误收起)。

### 1.6 【中】跨屏 backing scale 变化永不检测

scale 比较写在 `if ns_rect == state.rect { return; }` **之后** ⇒ 窗口在 1x/2x 屏之间拖动
(逻辑几何不变)时永远不 `notify_screen_info_changed`,一直按旧 DPI 出图。
修法:把 scale 检测移到早返回之前(只读窗口 backing scale,不产生 CEF 调用),变化时
在几何落地后补 `notify_screen_info_changed()`。

### 1.7 【中】`__restoreFocused` 在输入框尚未挂载时不重试

load 事件早于 SPA 首次渲染,此刻查不到输入框,原实现直接 `return` ⇒ DOM 焦点留在 body。
**实机判据**(临时探针):`t0 active=BODY` → 修复后 `t2 active=DIV`。
修法:元素缺失时按 50ms 重试(≈3s 上限,与"页面尚未 hasFocus"的既有重试合并计数)。

### 1.8 其它(小)

- **I5**:小键盘 Clear(mac 键码 71)cefclient 只发 KEYDOWN、**跳过 CHAR**(它的 characters
  是 ESC,当文本发出去会插控制字符)⇒ 照做。
- **I3**:`special_windows_key_code` 这张表在 **mac OSR 上不生效** ——
  `TranslateWebKeyEvent` 会用我们给的字符/键码**合成 NSEvent**,由 Chromium 反推
  windowsKeyCode(CEF 注释直言这是唯一无法直接翻译的成员)。保留表是为与 cefclient/CefSwift
  的跨平台构造保持一致,但已在代码里写明"本平台不采用",并更正了证据文档里把它当交付项的说法。
- **I6**:删除 T5 加在 `key_trampoline` 里的 Cmd+A/C/V/X/Z 兜底 —— 视图级
  `performKeyEquivalent:`(排在 AppKit 菜单之前)必先消费,该分支不可达(重复 owner)。
  连带删掉只服务于它的 `edit_command_for_key`/`device_independent_modifiers` 与单测。
- 环境开关口径统一(`ZAP_CEF_OSR_CPU_PAINT=0` 不再被当成开启)。
- 陈旧注释/死分支清理(`key_type==0` 的不可达防御分支、对已删函数的引用)。

## 2 审查者提出但**经复核不改动**的项

| 项 | 复核对结论 |
|----|-----------|
| I4「上屏后再发 cancel/finish 应加 `!textInserted` 守卫」 | **不改**。读 cefclient 原文:`BOOL textInserted = NO;` 后**从未置 YES**(提交分支只 clear 文本)⇒ `!textInserted` 恒真 ⇒ cefclient 同样是 commit + cancel 同发;CefSwift 亦然。加了反而偏离两份参考实现。 |
| I1「`focus()` 的 makeFirstResponder 会从地址栏抢焦点」 | 方向与 wry 的 `focus()`(makeFirstResponder)一致 ⇒ 属"向 windowed/wry 看齐"的修正;且 `focus(id,false)`(点击地址栏后 WarpHostView 抢回 first responder 会触发 resign)会清掉请求,不产生持续抢占。已实机回归点击/地址栏无异常。 |
| I2「`on_key_event` 恒返回 1 关掉了应用菜单回退」 | 未逐项实测 Cmd+Q/W/M/H,但路由上它们在 AppKit key-equivalent 阶段就被菜单消费(与已验证的 Cmd+T/Cmd+1..9 同类),不会到 CEF;Cmd+Ctrl+F 在 T5 轮已实测正常。**标注为"部分未验证"**。 |
| D5/Q3「候选框只取 `character_bounds[0]` / composition 下划线不分段」 | 与主参考实现 CefSwift 完全一致(`.first` 与整串实线),且实机候选框跟随光标正常 ⇒ 保持。 |
| Minor:`screen_info.rect/available_rect` 填视图尺寸、`CURSOR_DISAPPEAR` 不可达、`create_webview` 覆盖同 id 旧条目靠调用方守卫、`drive_external_begin_frame` 在借用内调 CEF | 影响面小/当前无触发路径,**记录待 T6/T8 处理**(弹层定位依赖 `screen_info`)。 |

## 3 复核意见(审查者确认无问题的部分)

- **借用纪律**:新加的回调都不在 `WEBVIEWS` 借用内二次借用(`on_before_close`/
  `take_and_release_osr_view` 都是"借用外释放");本轮修的 1.1 是唯一例外(已修)。
- **MRC**:tracking area(alloc +1 与 addTrackingArea 的 retain 用 remove+release 配平)、
  `_markedText` copy/release、`_textToInsert`、view 的 +1 与 release 配平;三条释放路径幂等。
- **ABI**:三个 `repr(C)` 结构体与 ObjC 侧逐字段一致(脚本核对),CEF 实参顺序与绑定签名一致,
  量纲统一 UTF-16。
- **windowed 等价性**:逐分支核对成立(三个新 handler 只在 OSR 注册;`scale_changed` 对
  windowed 恒 false)。**一处例外**:创建失败清理现在也清理 windowed 的残留条目(修泄漏,
  无用户可见变化)。

## 4 验证

```bash
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) cargo check -p warp                      # 0 warning
CEF_PATH=... cargo check -p warp --features cef_webview                                # 0 warning
CEF_PATH=... cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend)+test(loopback)+test(webview_init)'                             # 23/23
```

实机(自建 ZapCEF,OSR;用户逐项确认):

| 项 | 结果 |
|----|------|
| 新开 pane **不点页面**直接打字 | ✅(1.3+1.4+1.5+1.7 修复后) |
| 刷新后不点页面直接打字 | ✅ |
| 点页面选字 / 中文上屏 / Cmd+C·V·A·X·Z / Cmd+T·Cmd+1..9 | ✅ 无回归 |
| 导航/重载后页面焦点与 IME | ✅(1.5) |
| 隐藏→解冻(1.1 的触发路径) | ⏳ **待 T8 验证**(隐藏超时默认 300s;本轮只做了代码级修复 + 扫描确认无同类嵌套借用) |

## 5 环境问题(与代码无关,记录备查)

`script/macos/cef_smoke` 偶发在 `DockTilePlugin` 链接阶段失败:
`tapi error: malformed file ... unknown architecture arm64e.x1-macos`(CommandLineTools SDK
与所选 Xcode 的 SDK 解析不一致)。临时办法:构建前 `export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)`
(本轮验证过:加了就通过)。未改动任何构建配置(需要用户决定是否长期修)。
