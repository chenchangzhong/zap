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
