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

**实机结论(2026-09-22,已实测):mac 上 `<select>` 不产生 PET_POPUP ⇒ 弹层链路无触发路径**

- 做法:临时探针往页面注入一个可见 `<select>`(左上角),让用户点击。日志显示点击确实到达
  CEF(坐标 (53,19) 正是注入位置,`send_mouse_click` 连续多次),但
  **`on_popup_show`/`on_popup_size` 一次都没出现**,下拉也不展开。
- 判据:CEF 的 mac 侧没有实现 `<select>` 所需的嵌入方菜单路径 —— 在 CEF 源码里搜
  `PopupMenu`/`WebMenuRunner`/`ShowPopupMenu` 无命中(`libcef/browser/native/menu_runner_mac.mm`
  等),而 CEF 的 `OnPopupShow` 只在 `CefRenderWidgetHostViewOSR::InitAsPopup`
  (`render_widget_host_view_osr.cc:643-670`)里触发 —— 即**渲染器必须创建 popup widget**。
  mac 上 `<select>` 走的是另一条路(嵌入方菜单),CEF 未提供 ⇒ 链路不可达。
- 影响与结论:
  1. 本节的弹层实现(层树 + `on_popup_show/size` + POPUP 绘制路由)按参考实现写就、代码级核对通过,
     但**在 mac 上没有触发路径**,属"为将来/其他平台保留"的能力(不删,保持与 CefSwift 同构);
  2. **产品行为**:mac 上 CEF pane 里的 `<select>` 下拉**打不开**(windowed 模式同理,属 CEF-mac 限制,
     非 OSR 特有);需要 `<select>` 的页面请用 **wry 内核**(WKWebView 原生支持)。
- 因此 T6 的验收项"页面内 `<select>` 能展开"在 mac 上**无法达成**,原因不在本实现。

## 2 右键菜单(✅ 已通过)

### 为什么不用 CEF 原生菜单

CEF 的 `CefMenuManager::CreateContextMenu` 是 `on_before_context_menu` 的**唯一**调用点
(CEF 源码 `libcef/browser/menu_manager.cc`),而实测在 OSR 下该回调**一次都没被调用**。
真正原因是 **CEF 的 mac 菜单 runner 在 windowless 下结构性拒绝**:

```cpp
// libcef/browser/native/menu_runner_mac.mm
if (browser->IsWindowless()) {
  if (!browser->GetWindowHandle()) return false;   // ← 拿不到 handle 就任何菜单都不弹
  ...
```

而 windowless 的 host window handle 取自 `WindowInfo.parent_view`(`window_info().parent_view`
→ `host_window_handle_`)—— 本项目的 OSR 路径**从不设** `parent_view`(只有 windowed 路径
`set_as_child`),`GetWindowHandle()` 因此是 0 ⇒ 原生菜单永不出现。

**更正(初版归因错误)**:初版把原因写成"未实现 `GetScreenPoint`",这是错的 ——
`GetScreenPoint` 与菜单是否弹出**无关**;它的真实价值是 CEF 每次鼠标事件翻译
(`TranslateWebMouseEvent`)都要用它填 `screenX/screenY`,以及拖动/DevTools 等原生 UI。
本仓库已实现它(见 §3),但那是**另一件事**。

这正是计划里写"右键菜单改**异步 NSMenu**"的原因。

### 实现

- `cef_support.m` 的 `rightMouseDown:` **先照常把右键转发给页面**(页面的 `contextmenu`
  JS 事件与自定义菜单不受影响),再弹宿主自己的 `NSMenu`(「重新加载」「检查元素」),
  位置用视图坐标 `popUpMenuPositioningItem:atLocation:inView:self`(免手工换算屏幕坐标)。
- 选中项经新回调 `handle_menu_command(id, command, x, y)` 回 Rust:
  重新加载 → `browser.reload()`;检查元素 → `host.show_dev_tools(..., Some(point))`,
  point 是**右键点**的 DIP 坐标(DevTools 定位到点中的元素)。
- CEF 的 `ContextMenuHandler` 保留:windowed 模式下它照旧走原生菜单路径,不受影响。

### 实机证据(自建 ZapCEF,OSR)

事件对称性(修复后,成对):

```
[DEBUG] osr 1: send_mouse_click button=1 up=0 count=1 (838,241)
[DEBUG] osr 1: send_mouse_click button=1 up=1 count=1 (838,241)
```

菜单命令(用户确认可用):

```
[INFO] [warp::browser::cef_backend] [cef] webview 1 右键重新加载
[INFO] [warp::browser::cef_backend] [cef] webview 1 右键检查元素 (669.55859375,203.25)
```

## 3 顺带修掉的一项(与 T6 同源)

**补上缺失的 `CefRenderHandler::GetScreenPoint`**(view DIP → 屏幕坐标)。CEF 头文件原文:
"Windows/Linux should provide screen device (pixel) coordinates and **MacOS should provide
screen DIP coordinates**. Return true if the requested coordinates were provided" —— 默认实现
返回 false。它的消费方是**每一次鼠标事件翻译**(`TranslateWebMouseEvent` 填
`screenX/screenY`,见 `bpd_native_mac.mm`)以及拖动/DevTools 等原生 UI;
**它不是**右键菜单不出现的原因(见 §2 的更正)。
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
更强的同类判据:CEF 自家 mac 的 windowed 实现(`bpd_native_mac.mm`)、OSR 实现
(`browser_platform_delegate_osr` / `tic.mm` 把结果直接当 AppKit `NSRect` 用)都是同一口径。

**更正(初版陈述错误)**:初版写"CefSwift 未实现 `GetScreenPoint`"是**错的** ——
它在 `Sources/CefKit/CefRenderHandler.swift` 里实现了,并且返回**左上原点**的屏幕坐标
(原点在 `CefMetalHostView.swift` 里显式翻转得到,因为它的视图是 flipped)。
也就是说:参考实现**有**对照,而且与 zap 刻意采用的口径**不同**。
我们以 **CEF 自家 mac 实现/消费方**为准(理由见上),这是有意选择,不要照 CefSwift 改回去。

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
| 层树重构不破坏原有渲染 | ✅(日志) | T6 实例里 `OSR surface 3836x1908px (view_rect=(1918,954) DIP, scale=2.0)` —— 1918×954 DIP × 2 = 3836×1908,说明内容层确实拿到了 IOSurface(证据函数已改为读内容层,故这行同时证明贴图路由落在内容层) |
| `screen_point` 原点 | ✅(源码) | 见 §3.1,CEF 消费方直接交给 AppKit |
| `contentsScale` 跨屏后是否过期 | **记录项**(当前无显示影响) | `_contentLayer`/`_popupLayer` 的 `contentsScale` 只在创建时设一次,没有 `viewDidChangeBackingProperties`;结论"无影响"依赖"永远用 `contentsGravity = resize`"这一前提(此时 contents 被拉到层边界、`contentsScale` 不参与显示尺寸)。参考实现的 `updateScale()` 会更新三层,若将来改用别的重力,必须补上 |
| 菜单弹出期间 CEF pump 是否暂停 | **不会暂停**(初版写错已更正) | pump 定时器注册在 `NSRunLoopCommonModes`(见 `warp_cef_start_periodic_main_timer` 的注释:"滚动/拖拽等 tracking 期间也要继续推进 CEF"),common modes 含 event tracking ⇒ 菜单期间 pump 照常跑 |
| 右键手势事件是否对称 | ✅(已修 + 实测确认) | 初版在 `rightMouseDown:` 里弹模态菜单,会吃掉随后的 mouse-up —— **实测日志只有 `up=0` 没有 `up=1`**。仅改成"延到下一拍异步弹"**不够**(下一拍 ~1ms 后菜单已进入跟踪,仍吃掉 UP)。最终改为**在 `rightMouseUp:` 里先转 UP、再弹菜单**,日志验证成对:`up=0/up=1` ×3 组;菜单两项仍可用。同时把视图跨菜单 `retain` 起来(防跟踪中视图被销毁后菜单项 target 悬垂) |

### 弹层首次实测的观察清单(尚未执行)

1. y 是否颠倒(翻转公式与有单测的 `flip_rect_to_appkit` 同形,但未实跑);
2. 下拉越过 pane 边界是否被裁(root 层 `masksToBounds = NO`,但 warpui 侧未查裁剪设置);
3. `screen_info.rect` 目前填的是**视图矩形**而非真实屏幕矩形 —— CEF 头文件明写
   "矩形为空/非法时 popup 可能画不对",参考实现填的是真实屏幕 frame ⇒ 弹层真出问题时
   第一嫌疑在这里。

## 4 复验命令

```bash
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 弹层:`grep -a 'on_popup_show\|on_popup_size' ~/Library/Logs/zap.log`
# 菜单:`grep -a '右键' ~/Library/Logs/zap.log`
```

## 5 收尾(2026-09-22,用户决定)

- **`<select>` 弹层不做**:用户明确"没用到"。本文件的弹层实现与实测结论保留作为记录,
  验收项"页面内 `<select>` 能展开"撤销;T6 以"右键菜单 + `screen_point`"两项已验证能力关闭。
- **真透明已生效**:用户在实机确认"现在已经可以透明了"(OSR 模式下 dsh pane 的透明背景生效)。
  配合阶段 0 探针的像素证据(透明页 82.7% 像素 alpha=0、中央红块 alpha=255,见
  `OSR-SPIKE-A.md`)与主仓实跑的 `OSR surface = DIP×2`,透明这条目标的证据链闭合。
