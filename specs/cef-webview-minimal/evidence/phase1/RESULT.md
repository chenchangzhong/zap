# 阶段 1 第一生死项:CEF 子视图 × zap 挖洞分层窗口 —— 验证结果

> 2026-09-21,arm64 macOS 27.0,CEF 152.0.8 minimal。探针 = `tools/cef-spike`(bin `hole-probe`,
> ObjC 宿主 `probes/hole_probe.m`,页面 `probes/hole_page.html`,回环接收端 `probes/loopback_receiver.py`)。
>
> **判定:有条件通过**——三要素中「落点/层级/命中」与「输入可达」成立;「渲染正确」的像素级证据
> 受环境(屏幕锁定)阻塞、脚本已就绪;「尺寸跟随」**不成立**,已转为必做集成要求 #2。

## 0 探针复刻了什么(以及有意不复刻什么)

| 探针 | zap 真实对应 |
|------|-------------|
| `ProbeHostView`(contentView,非 flipped) | `WarpHostView`(host_view.h:35,窗口 contentView) |
| `ProbeContainerView`(透明,承载嵌入视图) | `WebViewContainerView`(host_view.m:654) |
| `ProbeBackgroundView` | `MetalBackgroundView`(铺满、最底层) |
| CEF 子视图 `CefBrowserHostView` | wry 的 WKWebView 子视图(同一挂载点与坐标约定) |
| `ProbeOverlayView`(洞内 `NSCompositingOperationClear` + `hitTest`→nil) | `MetalRenderView`(洞外绘制 overlay UI;洞内透明 + `warp_overlay_hit_test` 穿透) |
| `ProbeWindow::sendEvent:` | `WarpWindow::sendEvent:` 的 macOS 27 分支(window.m:464-557) |

**有意不复刻(结论边界)**:窗口按钮/缩放边缘处理(`standardWindowButtonAtEvent:`/`eventIsOverResizeEdge:`);
`RightMouseDown` 的同类特判(window.m:545);`MetalRenderView` 是 `isFlipped=YES` 且用 Metal 清像素,本探针用
`Clear` 混合模式自绘;**洞外 `hitTest` 在 zap 返回 host view 以驱动 overlay UI,本探针返回 overlay 自身**。
故本探针证伪/证实的是"事件分流分支"这一层,不覆盖右键菜单与 Metal 像素合成路径。

## 1 逐要素结论

| 要素 | 判定 | 证据 |
|------|------|------|
| 落点(CEF 视图 frame = 洞 rect) | ✅ | `A_plain_probe.log`:`CefBrowserHostView frame={{100, 100}, {600, 400}}` |
| 层级(背景→CEF→覆盖层) | ✅ | 同 dump 的子视图顺序与 frame |
| 命中可达(洞内→Chromium;洞外→覆盖层) | ✅ | `hitTest(hole center) -> RenderWidgetHostViewCocoa`;`hitTest(outside) -> ProbeOverlayView` |
| 页面在洞内运行 + 回环通道可用 | ✅ | 各日志均有 `probe.loaded` |
| **输入可达(页面级完整点击链)** | ✅(key window 下) | 黄金运行 `resize_server.log`:`loaded → mousedown → mouseup → click`;同轮 `resize_probe.log`:`isKeyWindow=1 appActive=1 firstResponder=RenderWidgetHostViewCocoa` |
| **渲染正确(像素级)** | ⚠️ **未取证(环境阻塞)** | 屏幕锁定/休眠时全屏截图纯黑、窗口定向报 `could not create image from window`;采样链路已独立验证(§4),补跑见 §5 |
| **尺寸跟随** | ❌ → 集成要求 #2 | `resize_probe.log`:窗口 frame 960×672 → 1200×832(content 960×640 → 1200×800),CEF 视图 600×400 → **840×560**,洞仍 600×400 |

## 2 事件分流:缺陷与修复(分支级证据,确定性)

`ProbeWindow::sendEvent:` 支持三态路由;分支日志是**主证据**(不依赖窗口是否 key,也不依赖 Chromium 是否接受合成事件):

```
A_plain_probe.log    (routing=plain,平台默认)      : 无分支日志(直接 [super sendEvent:])
B_zap_probe.log      (routing=zap,旧 WKWebView 特判) : mouseUp branch=contentView(改投!) target=RenderWidgetHostViewCocoa
C_embedded_probe.log (routing=embedded,泛化判定)     : mouseUp branch=super(交给命中视图) target=RenderWidgetHostViewCocoa
```

- **缺陷确认**:`[_leftMouseDownTarget isKindOfClass:WKWebView]` 对 CEF 视图为假 ⇒ mouseUp 走
  `[self.contentView mouseUp:event]`,**不进 CEF 视图**,点击链断在 mouseUp。
  (B 组页面日志只有 `probe.loaded`,与"mouseUp 未达页面"一致;A 组在 key window 下则收到完整链。)
- **修复确认(正向对照)**:把判定泛化为"命中目标是否属于嵌入平台视图"(探针用"是否为
  `WebViewContainerView` 的后代")后,mouseUp 交回命中视图。

**集成要求 #1(必须做)**:zap 侧同类类名特判共 **4 处**,全部需要泛化:
`window.m:505`(LeftMouseUp)、`window.m:525`(LeftMouseDragged)、`window.m:545`(RightMouseDown,
不改则 CEF 下右键菜单继续失效)、`window.m:607-608`(`performKeyEquivalent` 定义在 window.m:593,
负责 Cmd+C/V/X/A)。
另:`host_view.m:281` 是 tracking-area 的注释行,**该文件内没有任何类名判定**(唯一 `isKindOfClass` 是
NSAttributedString);"CEF 下 hover 是否失效"本轮**未验证**,按 AGENTS.md §5.6.1 记为「未验证推测」。

## 3 尺寸跟随与坐标空间(集成要求 #2)

CEF `set_as_child` 后 CefBrowserHostView 的 autoresizing 表现是"随父视图增量扩张"
(窗口 +240/+160 后视图 600×400 → 840×560),**不跟洞 rect**;洞中心仍命中(视图变大后覆盖洞中心),
但视图与洞错位、且未通知 Chromium 视口更新。
⇒ CefBackend 必须实现与 `BrowserWebViewManager::set_bounds`(browser_web_view.rs:634)等价的每帧布局,
并调用宿主 `was_resized()`。

**坐标空间(未实测,代码可溯)**:探针的洞与传入 bounds 同在 AppKit frame 空间(非 flipped、底部原点),
故"逐像素一致"只在该空间成立。zap 的逻辑 rect 是左上原点的场景坐标,wry 在
`window_position`(wry-0.56.1/src/wkwebview/mod.rs:1446)做 `parentHeight - y - h` 翻转;
CEF `set_as_child` 取 AppKit frame 坐标 ⇒ CefBackend **必须复刻同一翻转**,否则垂直错位。

## 4 像素采样链路(已独立验证,待屏幕可捕获时补跑)

- `probes/sample_pixel.swift`:`sample_pixel app/assets/resources/mac/warp_install_image.png 100,100 400,300`
  → `#0B0B0B` / `#1E1E1E`,越界点报 `out-of-range`(证明采样器可读真实像素)。
- `probes/compute_sample_points.py`:由探针日志(`shotmap`/`window frame`/`content`)算出洞内/洞外采样点;
  在 `A_pixel_probe.log` 上得 `IN_HOLE=(1120,1240) OUTSIDE=(380,600)`。
- 当前会话截图全黑(屏幕锁定),`*_pixels.txt` 里的 `#000000` **不是结论**,判读时须忽略。

## 5 复现命令(自包含;脚本已内置重签与重试)

```bash
cd tools/cef-spike
cargo run -q --bin make-bundle -- hole-probe -o target/hole
probes/run_hole_probe.sh plain    A_plain                  # 平台默认路由
probes/run_hole_probe.sh zap      B_zap                    # 复刻 zap 现状(缺陷)
probes/run_hole_probe.sh embedded C_embedded               # 泛化判定(修复对照)
probes/run_hole_probe.sh plain    resize --resize-test     # 尺寸跟随
```

**环境前提(本轮踩到)**:页面级事件与像素证据都要求**屏幕可用**(应用能被 LaunchServices 激活、
`screencapture` 能取到画面)。屏幕锁定/休眠时窗口不成 key ⇒ Chromium 丢弃合成事件(仅分支日志可信),
截图输出纯黑。探针打印 `isKeyWindow/appActive` 与 `CLICK_PROCEEDING` 供判读;`--log` 每次以 `"w"` 截断。
