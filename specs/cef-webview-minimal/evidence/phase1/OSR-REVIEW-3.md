# OSR 计划全量代码审核(第三轮,2026-09-22)

> 范围:`433bdb206..HEAD`(整个 OSR 计划的 18 个提交)。
> 方式:**两位独立审查者**(ObjC 侧完成;Rust 侧第一次运行失败退出,已收窄范围重派),
> 加上我的机械审计。逐条**先验证再改**。

## 1 已修

### I1【Important·ObjC】`performKeyEquivalent:` 放行 Shift ⇒ `Cmd+Shift+A/C` 被本视图吞掉

- 判据(我自证):`app/src/util/bindings.rs:362` `CustomAction::CopyBlockCommand => "cmd-shift-C"`;
  `app/src/workspace/mod.rs:805` `.with_mac_key_binding("cmd-shift-A")` → `ToggleConversationListView`
  (带 `MAC_MENUS_CONTEXT`,即菜单项)。而 `WarpWindow` 对嵌入视图要求
  `mods == NSEventModifierFlagCommand` **精确相等** ⇒ Shift 形态落到 `[super performKeyEquivalent:]`;
  审查者用探针实测 NSWindow 派发顺序是"内容视图层级先于主菜单" ⇒ 我们 `return YES` 后菜单永不命中。
- 修法:`a/c/v/x` 必须不带 Shift(`z` 保留 Shift 作 redo)。改后 `Cmd+Shift+A/C` 回到 zap 既有链路。

### U6【收口】`destroy()` 先异步关闭再释放视图,release 内的同步 focus 回调可能对"已请求关闭"的
browser 调 `set_focus`。改为先从状态里 `take` 掉句柄再关闭/释放,与 `on_before_close`/`shutdown` 一致。

### 机械审计:最后一处"持借调外部"

`drive_external_begin_frame` 在 `map.borrow()` 内调 `host.send_external_begin_frame()`
(仅默认关闭的 `ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME=1` 可达)⇒ 改为借用内收集 `BrowserHost`、
借用外调用。全文件 `WEBVIEWS.with` 的 7 处含外部调用的块,其余 6 处经核对安全(只取 clone/指针,
调用在借用外)。

### M1【Minor】`webview_init.js` 的 hasFocus 重试窗口回退

我上一轮为修"元素未挂载"把两个重试合并成 3s,连带把 hasFocus 窗口从 ~600ms 拉长 —— 会推迟
"聚焦 + 光标折叠到末尾",期间可能覆盖用户自己的光标操作(且该脚本 **wry 共用**)。已拆成两个计数:
元素缺失保持 ~3s(SPA 首渲染晚于 load,这是必需的),页面未就绪恢复 20×30ms。

### M2【Minor】注释与证据的自相矛盾

`cef_support.m` 原写"Cmd+Z 走 warp 菜单、故不拦截",实际本视图已拦截并 `return YES`
(`undo:`/`redo:` 只在菜单路径可达)—— 已写清两条入口的关系;`OSR-T5-INPUT.md` 里"没有实现
`performKeyEquivalent:`"的断言与 T7 实现相反(上一轮已在 T7/PLAN 更正),本次在代码注释里补齐说明。

## 2 复核确认无问题(审查者独立验证,非照抄)

- **MRC 配对**(**运行期探针实证**):设 `self.layer = root` 后 NSView 确实持有 layer 并在视图
  dealloc 时释放(探针打印 `[ProbeLayer dealloc]`);`_contentLayer`/`_popupLayer`/
  `_textToInsert`/`_markedText`/`_trackingArea`/菜单项/CGImage+CGDataProvider+CGColorSpace
  全部配平(含 `image == NULL` 分支)。
- **释放幂等无悬垂**:`take_and_release_osr_view` 先置空共享指针再释放;销毁/on_before_close/
  shutdown 三条路径都先 `Option::take` 或收集到借用外。
- **windowed 纯净**:`install_osr_input_callbacks` 唯一调用点在 Osr 分支;`WarpCefOsrView`
  在 windowed 下永不实例化;所有导出函数 NULL 早返回。我另核对:`set_bounds` 两分支都调
  `was_resized`;`spawn_browser` windowed 臂的 `set_as_child`/`background_color` 与基线逐行一致;
  全文件 59 行删除全是有意重构。
- **NSEvent 字段纪律**:`clickCount` 只在 CLICK、`characters`/`isARepeat` 只在 keyDown/keyUp
  —— T5 的两类必崩场景都被挡住。
- **层语义经实验证实自洽**:非 flipped layer-hosting 下 `geometryFlipped = 0`;AppKit 随视图
  resize 同步宿主 layer bounds;`view.hidden` 会同步到 `root.hidden` ⇒ 不会残留旧画面。
- **坐标**:`screen_point` 与 `firstRectForCharacterRange:` 都做 view DIP 左上 → AppKit 屏幕 DIP
  左下,与 CEF mac 契约一致;`popUpMenuPositioningItem:...inView:self` 用视图坐标不重复换算。
- **静态检查**:`clang -fsyntax-only -fno-objc-arc -Wall -Wextra -Wdeprecated-declarations`
  对 `cef_support.m` **0 warning**。

## 3 记录项(有判据、当前无可见影响,不本轮修)

| 项 | 判据 | 何时需要处理 |
|----|------|--------------|
| `_contentLayer`/`_popupLayer` 的 `contentsScale` 只在创建时设置 | SDK `NSView.h`:`layer:shouldInheritContentsScale:fromWindow:` 明说非 backing layer 不自动继承;今天因 `contentsGravity = kCAGravityResize` 无可见影响 | 若将来改用非 resize 重力 |
| `otherMouse*` 忽略 `buttonNumber`,一律当中键 | 参考实现按 buttonNumber 把 3/4 映射为 back/forward | 若产品要用五键鼠标侧键 |
| `on_paint`(CPU 兜底)缓冲区未拷贝 | CEF 的两个头都未声明软件缓冲区生命周期;T4 的 `ZAP_CEF_OSR_CPU_PAINT=1` 真机渲染正常 | 若 CPU 兜底路径出现撕裂/随机花屏,改为 memcpy 进自有 NSData |
| IOSurface 直接做 `layer.contents` | CEF 头说句柄"回调返回后归还池";参考实现同款、T4/T6 真机渲染正确 | 若高速滚动出现撕裂,改 Metal blit |
| 光标离开视图后无显式恢复路径 | `mouseExited:` 只转发;T5 只验证了悬停手型 | 若实测从链接移出到终端后残留手型 |
| `selectedRange` 只喂 composition 选区 | 未接 `on_text_selection_changed`(代码注释已自认) | 若需要 emoji/输入面板的精确插入位置 |
| 弹层(popup layer)不可执行 | mac 上 `<select>` 不产生 PET_POPUP(T6 实测 + 用户撤销验收) | 若将来支持其他平台或 CEF 补上 mac 菜单路径 |

## 4 验证

```bash
SDKROOT=$(xcrun --sdk macosx --show-sdk-path) CARGO_INCREMENTAL=0 cargo check -p warp   # 0 warning
CEF_PATH=... cargo check -p warp --features cef_webview                                # 0 warning
CEF_PATH=... cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend)+test(loopback)+test(webview_init)'                             # 24/24
```

**本轮按用户要求不做 GUI 测试**;上述修复涉及的行为(Shift 形态快捷键回到 zap 链路、重试窗口)
待下次实机时顺带确认。

## 5 Rust 侧审查(第一位审查者运行失败后收窄范围重派)

结论:**无 Critical**;1 个 Important(结构性)、6 个 Minor。逐条处置:

### 已修

| # | 问题 | 修法 |
|---|------|------|
| #2 | `on_before_close` 在借用内 `state.browser = None`(drop = CEF `release`,持借期 drop 即"借用内调外部") | 改成 `take` 出来、借用外 `drop`(与 `destroy`/`shutdown` 一致) |
| #3 | `create_webview` 直接 `insert` 覆盖同 id 旧条目 ⇒ 借用内 drop `Browser`,且旧 `Rc<OsrState>` 无 `Drop` ⇒ 旧 NSView 永久留在容器(幽灵页)、旧 browser 无人关 | insert 前先 `remove` 取旧条目:借用外 `close_and_detach` + `take_and_release_osr_view`,并告警(防御路径) |
| #4 | `on_after_created` 借用内 `browser.clone()`/覆盖旧值(add_ref/release) | 同样 take 旧值、借用外 drop |
| #6 | `set_visible`/`set_bounds`/`apply_geometry` 用 `osr.is_some()` 选分支:若 `osr=None` 而进程是 OSR,会落到 windowed 臂去操作 **warpui 父容器**(`host.window_handle()` 在 OSR 下就是容器) | 三处都补 `None if render_mode() == RenderMode::Osr => {}`(什么都不做),与 `detach_view` 口径一致 |
| #1(部分) | 四个 handler 入口用**会 panic** 的 `borrow_mut()`(从借用内被同步触达 = `extern "C"` 里 panic = abort) | 新增 `try_with_webviews()`:借不到就 `log::warn!` + 跳过,回调入口不再可能 abort |

### 有意不改(附判据)

- **#5「`deferred_key_plan` 缺 `!textInserted` 守卫」**:**不改**。cefclient 里 `BOOL textInserted = NO;`
  在提交分支**从未置 YES**(我逐行读过 `text_input_client_osr_mac.mm:289-337`)⇒ 该守卫在 cefclient 里
  **恒真(形同虚设)** ⇒ 它的**实际行为**与"commit + cancel 同发"一致,而这正是我们的实现;
  主参考 CefSwift 同样没有该守卫。**更强的判据是我们的实机证据**:T7 里 `ime_commit_text "敬他是发"`
  紧接 `ime_cancel_composition`,中文上屏正确、光标正常(详见 OSR-T7-IME.md)。
  即:参考实现的注释描述了意图,但代码没实现;我们跟随的是**两者共同的实际行为**。
- **#7** 共享 scale 会被"视图无窗口时兜底 2.0"写入(≥1x 屏、窗口拆除中):影响 ≤1 帧(下一帧纠正),记录。
- **#1 的结构**(`with_browser` 闭包在借用内执行):今天不可达 —— 审查者已逐个核对 render/display/
  keyboard handler 完全不碰 `WEBVIEWS`(T4 的设计),ObjC 的 `setFrameSize:` 只碰 layer;
  但这是"安全依赖别处不变量"的位置,**记录并把 handler 入口全部改成 try**(见上表 #1),
  将来若有人给 handler 加一次 `WEBVIEWS` 读取,症状会是"几何/焦点被静默跳过 + 一条 warn",而不是 abort。

### 记录项(未验证疑点,触发未证实)

| 项 | 判据 | 触发条件 |
|----|------|----------|
| `focus()` 的焦点三联不检查 `visible`(而 `on_load_end` 同一组有守卫) | 代码;调用侧只找到 `handle_attach` | 隐藏 pane 收到 `focus(id,true)` 且此前不是 first responder ⇒ CEF 被改成 visible(60fps 出图)直到冻结 |
| 关机后仍可能调 CEF:`create_webview`/`spawn_browser`/`set_bounds`/`destroy` 无 `is_shutting_down` 门控(`shutdown` 也不复位 `INITIALIZED`) | 代码 | shutdown 之后再有 create/destroy;缓解事实:`on_window_will_close` 在 Terminating 阶段直接 return,故窗口清理不会在 shutdown 后跑 |
| `set_bounds` windowed 臂第二次 `window_handle()` 无 null 检查 | 两次取用之间无 CEF 调用,当前被第一次检查覆盖 | 未来在该区间插入 CEF 调用即可能解引用 null |
| `has_marked`/`marked_text` 语义与参考不同(我们传持久 composition 文本,参考是 per-key 累积量) | `cef_support.m` 的 producer + `text_input_client_osr_mac.mm` | 组合期间按下不经 `setMarkedText` 的键(如 F5)⇒ 多发一次同文本的 `ime_set_composition`(参考不发);影响未证实 |
| `on_accelerated_paint`/`on_paint` 不做 null 检查 | 但 ObjC 四个 setter 都有 NULL 早返回 ⇒ **无 UAF**(审查者确认) | — |

审查者同时确认:释放路径三条幂等、`drive_external_begin_frame`/6 个 trampoline 借用纪律正确、
"先写共享几何再通知 CEF"的不变量成立、`resolve_render_mode` 与三处推入的顺序正确。

## 6 「审核修复」轮(2026-09-22,第三位审查者,只审 §5 那批修复)

结论:**可合入(advisory only)**;8 条修复 6 条成立、2 条部分成立,**无 Critical、无新引入缺陷**;
问题都落在"今天不可达"的防御分支。据此做的处理:

| 审查意见 | 处理 |
|----------|------|
| `on_after_created` 的跳过会留下"浏览器已创建但句柄未登记"的僵尸(页面在画,但输入/JS/shim 全失效,且 manager 不知道失败) | **已修**:跳过分支改为 `close_and_detach(&browser)` + `notify_webview_create_failed(id)` + warn,让 manager 走失败重建 |
| `try_with_webviews` 只覆盖 2/4 回调入口,"回调入口不再可能 abort"属**过度声明** | **已收窄声明**:注释里写明覆盖范围,并指出 `on_load_end`(3 处)/`on_render_process_terminated`(1 处)仍是裸借用、今天不可达,属纵深防御下一批候选 |
| Shift 守卫实际把 `cmd-shift-V/X` 也放行了(提交只声明 A/C);且注释"覆盖 CapsLock/Shift 形态"与实现不符 | **已改注释**写明:`cmd-shift-V/X` 本仓无绑定,放行后不是"什么都不做"—— super 走完菜单事件仍回到本视图 `keyDown` 照常转发给页面(只是不再走 `paste:`/`cut:` 响应者动作);同时注明"内容视图先于主菜单"的派发顺序来自探针、未在本项目复现 |
| `create_webview` 的"幽灵页"在证据里被写成活 bug(实际当前不可达) | 该缺陷的**可达性**以 §5 表为准:属防御性缺口(调用方有 `has_webview` 守卫、三条移除路径都与 `destroy` 配对) |
| 三处模式守卫的因果缺可达路径 | 同上:属**纵深防御**(`browser=Some && osr=None` 只在 `release_osr_view`/`on_before_close` 两条清空路径之后出现,而那两处都先 take 了句柄) |
| `on_after_created` 里 `browser.clone()` 仍在借用内 | 已核对 cef-rs:`RefGuard::Clone = add_ref`(纯引用计数、不触发回调),`Drop = release`(可能销毁+跑回调)——**危险的是 release,已移出**;`add_ref` 留在借用内无害 |
| `_old_browser`(下划线绑定)当时就已正确,`f38a98242` 只是改显式 | 采纳:非冗余,保留显式 drop 写法 |
| **重要验证缺口 ①**:"24/24" 不能作为这两轮修复的验证证据(那些用例是纯逻辑,不碰 `create_webview`/`destroy`/`on_before_close`/`on_after_created`/`try_with_webviews`/`drive_external_begin_frame`;ObjC/JS 改动亦无自动化覆盖) | 如实记录:**这两轮修复的验证 = 编译 + 代码级核对 + 审查者复核,没有自动化覆盖** |
| **重要验证缺口 ②**:M1 把 hasFocus 窗口收回 600ms 缺实机复验(此前 `5cf44c7dc` 的实机验收是"两个窗口都 3s"的组合,从未单独证伪 600ms 足够) | 列入待办:**下次实机时单独复验**"打开 pane 不点页面直接打字"与 Shift 组合键路由 |
