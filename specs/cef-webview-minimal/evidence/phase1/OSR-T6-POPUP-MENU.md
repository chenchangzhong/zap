# OSR T6:弹层与右键菜单(部分验证)

> 目标见 [OSR-PLAN.md](../../OSR-PLAN.md) T6:页面内 `<select>` 能展开;右键菜单可用。
> 结论:**右键菜单 ✅ 通过**(实机 + 日志);**弹层 ⚠️ 已实现但未验证**(当前 dsh 页面里
> 找不到可触发的 `<select>`,日志中 `on_popup_show`/`on_popup_size` 一次都没出现)。

## 1 弹层(已实现,缺实机证据)

实现照参考实现 CefSwift 的层树结构:

| 层 | 作用 |
|----|------|
| root(`self.layer`) | 透明、`masksToBounds = NO`(弹层可越出视图边界) |
| `_contentLayer` | 页面内容(原实现直接写 `self.layer.contents`,现拆为独立子层) |
| `_popupLayer` | 弹层:显隐来自 `on_popup_show`,尺寸来自 `on_popup_size`,绘制走 POPUP 类型的 paint 回调 |

- Rust 侧:`on_popup_show`/`on_popup_size` 接到宿主视图;`on_accelerated_paint`/`on_paint`
  的 `PET_POPUP` 分支分别落到 `set_popup_surface`/`set_popup_bitmap`(原来直接丢弃)。
- **坐标**:`on_popup_size` 给的是 **DIP、左上原点**(与 `view_rect` 同口径);本视图非 flipped,
  故 y 翻成底部原点(`warp_cef_osr_view_set_popup_rect`)。
- 子层不继承父层 `contentsScale`,内容层/弹层各自设(否则 IOSurface 按错倍率贴)。

**未验证风险**:上述 y 翻转与层结构只做过代码级核对(与 CefSwift 逐行对齐),没有实机触发过
popup。等 dsh 页面出现 `<select>`(或设置面板)时按本文末尾命令复验。

## 2 右键菜单(✅ 已通过)

### 为什么不用 CEF 原生菜单

CEF 的 `CefMenuManager::CreateContextMenu` 是 `on_before_context_menu` 的**唯一**调用点
(CEF 源码 `libcef/browser/menu_manager.cc`),而实测在 OSR 下该回调**一次都没被调用** ——
渲染器的右键请求到不了 CEF 的菜单路径,菜单自然不出现(`screen_point` 已实现、鼠标
RIGHT 事件也确实到达 CEF,均排除)。

这正是计划里写"右键菜单改**异步 NSMenu**"的原因。

### 实现

- `cef_support.m` 的 `rightMouseDown:` **先照常把右键转发给页面**(页面的 `contextmenu`
  JS 事件与自定义菜单不受影响),再弹宿主自己的 `NSMenu`(「重新加载」「检查元素」),
  位置用视图坐标 `popUpMenuPositioningItem:atLocation:inView:self`(免手工换算屏幕坐标)。
- 选中项经新回调 `handle_menu_command(id, command, x, y)` 回 Rust:
  重新加载 → `browser.reload()`;检查元素 → `host.show_dev_tools(..., Some(point))`,
  point 是**右键点**的 DIP 坐标(DevTools 定位到点中的元素)。
- CEF 的 `ContextMenuHandler` 保留:windowed 模式下它照旧走原生菜单路径,不受影响。

### 实机证据(自建 ZapCEF,OSR;用户确认"菜单出来了、两项都能用")

```
[INFO] [warp::browser::cef_backend] [cef] webview 1 右键重新加载
[INFO] [warp::browser::cef_backend] [cef] webview 1 右键检查元素 (669.55859375,203.25)
```

## 3 顺带修掉的一项(与 T6 同源)

**补上缺失的 `CefRenderHandler::GetScreenPoint`**(view DIP → 屏幕坐标)。CEF 头文件原文:
"Windows/Linux should provide screen device (pixel) coordinates and **MacOS should provide
screen DIP coordinates**. Return true if the requested coordinates were provided" —— 默认实现
返回 false ⇒ 不实现的话右键菜单/DevTools/拖拽这些原生 UI 拿不到屏幕坐标(实测菜单完全不出现)。
已实现并经日志验证换算正确:`screen_point (714,216) → (715,798)`。

同一份头文件还写明:`GetScreenInfo` 的矩形**留空会回退到 `GetViewRect`**,所以"填视图矩形"
是合规实现(上一轮审查里那条 minor 可以结案)。

## 3.1 `screen_point` 的原点约定(由消费方源码坐实)

CEF 的 mac 菜单 runner(`libcef/browser/native/menu_runner_mac.mm`)是这么用客户端给的点:

```cpp
const gfx::Point& screen_point = browser->GetScreenPoint(...);
NSPoint screen_position = NSPointFromCGPoint(screen_point.ToCGPoint());
[[menu_controller_ menu] popUpMenuPositioningItem:nil
                                       atLocation:screen_position
                                         inView:nil];       // inView:nil ⇒ AppKit 屏幕坐标(左下原点)
```

⇒ 客户端应当返回 **AppKit 的屏幕坐标(左下原点,单位点/DIP)**,我们的实现
(`[v convertPoint:inView toView:nil]` → `[window convertPointToScreen:]`,**不翻转**)正确。
(CefSwift 未实现 `GetScreenPoint`,故它没有可对照的实现;这里以 CEF 消费方为准。)

## 3.2 与参考实现的差异(刻意)

参考实现 CefSwift 的右键菜单走 **CEF 的 `RunContextMenu` 回调**,其注释还写明"CEF 禁止在回调里
跑 OS 模态循环(只有 start_dragging 除外),所以同步快照菜单项、下一拍再弹 NSMenu"。
问题是:这条链的前提是 CEF 会先调 `on_before_context_menu` —— 而实测在 OSR 下**它从未被调用**
(见 §2),所以那条路在我们这里会得到"没有任何菜单"。

我们的做法是**在 `rightMouseDown:` 里由宿主直接弹菜单**。`rightMouseDown:` 是 AppKit 回调、
不是 CEF 回调,因此不受"CEF 回调里禁跑模态循环"那条限制,可以直接弹。

代价(已知):宿主菜单不再受页面 `contextmenu` 事件影响 —— 页面即使 `preventDefault()` 也会看到
我们的菜单(CEF 不告诉我们它是否处理了)。当前 dsh 页面没有自定义右键菜单,故可接受;
若将来要精确对齐浏览器语义,需要另找通路(CEF 侧触发链或页面侧约定)。

## 3.3 交付前自查(本轮,逐项有判据)

| 项 | 结论 | 判据 |
|----|------|------|
| `WarpCefOsrInputCallbacks` 新增字段的两侧一致 | ✅ | 脚本逐字段比对:ObjC 与 Rust 都是 `handle_event/handle_edit_command/handle_focus/handle_key/handle_ime/handle_menu_command`,顺序类型一致 |
| 新 ivar(`_contentLayer`/`_popupLayer`/`_lastContextMenuPointDIP`)在 `@public` 块内 | ✅ | C 函数需直接访问,已确认在 `@public` 之后 |
| `setFrameSize:` 在 `initWithFrame:` 早期(ivar 仍为 nil)是否安全 | ✅ | 对 nil 发消息是 no-op;init 里随后显式设 `_contentLayer.frame = bounds` |
| 菜单 `NSMenu`/`NSMenuItem` 在 `popUpMenuPositioningItem:` 返回后释放 | ✅ | 该方法跑完菜单事件循环才返回(期间 action 已执行完);`target` 是 assign,不会形成 retain 环 |
| 弹层 y 翻转 | ✅(公式) | `bounds.height - (y + h)` 与已验证过的 `flip_rect_to_appkit`(`parentHeight - y - h`,有单测)同形 |
| 层树重构不破坏原有渲染 | ✅(间接) | 本轮实机里页面渲染/点击/输入全部正常;证据函数 `surface_size` 也已改为读内容层 |
| `screen_point` 原点 | ✅(源码) | 见 §3.1,CEF 消费方直接交给 AppKit |
| `contentsScale` 跨屏后是否过期 | 无需处理 | `contentsGravity = resize` 下 contents 会被拉伸到层边界,`contentsScale` 不参与显示尺寸;跨屏时 CEF 已收到 `notify_screen_info_changed` 按新 DPI 出图 |
| 菜单弹出期间 CEF pump 暂停 | 已知 | 模态菜单跑在 `NSEventTrackingRunLoopMode`,默认模式的定时器不触发 ⇒ 期间页面不刷新(标准菜单行为),关闭后继续 |

## 4 复验命令

```bash
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 弹层:`grep -a 'on_popup_show\|on_popup_size' ~/Library/Logs/zap.log`
# 菜单:`grep -a '右键' ~/Library/Logs/zap.log`
```
