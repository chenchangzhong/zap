# dsh pane 透明背景评估（CEF/Chromium 后端）

> 结论先行:**默认配置下不存在可见差异**;只有"窗口不透明度 < 100%"或"主题带背景图"时才会看出 CEF 区域是不透光的。
> 若必须做到像 wry 路径那样的真透明,**只能走 OSR(windowless)渲染**;建议先做一次 2 天 spike 验证 IME 与 IOSurface 合成再决定。

## 1 现状与证据

### 1.1 当前实现(参考方案)
CEF 用 **windowed(子视图)** 模式:把浏览器视图挂到 `WebViewContainerView` 下,并给浏览器背景填上 zap 的工作区底色
(`BrowserSettings.background_color` ← `appearance::window_surface_color`,与 workspace 的窗口背景是**同一个函数**,见 [appearance.rs](../../../app/src/appearance.rs))。

### 1.2 为什么不能像 wry 那样直接设透明
- wry 的做法是 WebKit **私有 KVC 键**:`config.setValue_forKey(NO, "drawsBackground")`(+ 运行时对实例再设一次),
  见 `wry-0.56.1/src/wkwebview/mod.rs:371` 与 `:989`。**这是 WKWebView 专有**,CEF 的视图是 Chromium 的
  `RenderWidgetHostViewCocoa`,没有对应键。
- CEF 官方头文件 `~/.local/share/cef/include/internal/cef_types.h:701-708` 明确:
  > Background color used for the browser ... If the alpha component is fully transparent for a **windowed** browser
  > then the **CefSettings.background_color** value will be used. If the alpha component is fully transparent for a
  > **windowless (off-screen)** browser then transparent painting will be enabled.

  而 `CefSettings.background_color` 那段又写:windowed 下透明 → 回退**不透明白色**。
  ⇒ 已验证:windowed 模式不可能透明,透明尝试会退化成白底(比填色更糟),故已回退。

### 1.3 实测证据(2026-09-21,探针实验)

用 `tools/cef-spike` 的洞拓扑探针做决定性实验:页面背景全透明(`html,body{background:transparent}` +
一块不透明红色标记证明页面确实渲染),并把 `BrowserSettings.background_color` 与
`CefSettings.background_color` 都设为 `0`(透明):

| 采样点(图像坐标,scale=2) | 对应屏幕坐标 | 位置 | 实测颜色 |
|---|---|---|---|
| 1120,920 | 560,460 | **洞内** | **#FFFFFF** |
| 1400,1000 | 700,500 | **洞内** | **#FFFFFF** |
| 1800,1000 | 900,500 | 洞外(覆盖层) | #2C3D54 |

⇒ **windowed CEF 的透明背景确实是"不透明白"**(与 §1.2 的头文件描述一致),不是"没生效"而是
**CEF 的既定行为**。洞内背后本应是探针的 `ProbeBackgroundView`(#12171F),实际却是纯白,
说明浏览器视图在不透明绘制、不给下层任何透出机会。

复现:`PROBE_TRANSPARENT=1 <hole-probe> --url=file://…/probes/transparent_page.html --routing=plain --hole=100,100,600,400`,
随后 `screencapture -x` + `probes/sample_pixel`(探针里的"递归清 layer"试验代码会 panic,已确认无必要 —— 白底来自 CEF 自身,不是 layer 不透明标记)。

### 1.4 影响面量化(决定"值不值得做")
| 场景 | 是否可见差异 |
|------|--------------|
| 窗口不透明度 = 100%(**默认**:`BackgroundOpacity.default = 100`,你的 `~/.zap/settings.toml` 未覆盖) | **无差异** —— 我们填的就是 workspace 用的同一个颜色 |
| 窗口不透明度 < 100%(设置里可调,macOS 可配置;仅 Windows 原生装饰下禁用) | 有:整个窗口透出桌面,而 CEF 区域是不透光色块 |
| 主题带背景图(`theme.background_image()`,叠在底色之上) | 有:webview 区域显示纯色而非背景图 |

## 2 方案对比

| 方案 | 做法 | 成本 | 效果 | 风险 |
|------|------|------|------|------|
| **A 保持现状** | windowed + 填工作区底色 | 0 | 默认无差异;半透明/背景图场景不一致 | 无 |
| **B 页面注入底图** | 把当前窗口的底色/**背景图**经回环端点注入页面 CSS(`body{background:url(...)}`),让页面自己画出同款背景 | 小(约 0.5 天) | 背景图场景基本一致;半透明仍无法让桌面透出(页面只能画出静态图) | 依赖注入(改 dsh 前端归属之外的地方需谨慎);页面自身若画不透明底会盖住 |
| **C 强制窗口不透明** | 有 CEF pane 时禁用该窗口的不透明度(等价于把 `effective_opacity` 钉在 100) | 极小 | 彻底消除不一致(代价:放弃半透明特性) | 产品取舍 |
| **D OSR + 共享纹理(推荐的真透明路线)** | windowless 渲染,`OnAcceleratedPaint` 拿到 macOS **IOSurface**,贴到子视图的 `CALayer.contents` 或 warpui 纹理 | **大(5~10 天)** | 真透明:alpha 逐像素透出下层 Metal 背景 | IME/弹层/输入保真,见 §4 |
| **E OSR + 软件位图** | `OnPaint` 拿 CPU 位图再上传 | 中 | 真透明但性能差 | Retina 下每帧 2560×1600×4B≈16MB 拷贝,不可接受(仅作降级兜底) |

## 3 方案 D 技术设计(要点)

### 3.1 渲染
- `Settings { windowless_rendering_enabled: 1, shared_texture_enabled: 1, .. }`;
  `WindowInfo { windowless_rendering_enabled: 1, shared_texture_enabled: 1, .. }`(不再 `set_as_child`)。
- 实现 `RenderHandler`(`cef-rs` 已提供 `wrap_render_handler!` 与所需回调):
  `view_rect`(视图像素尺寸,含 DPI)、`screen_info`/`screen_point`、`on_paint`(兜底)、
  **`on_accelerated_paint`**(macOS 给 `shared_texture_io_surface: *mut c_void` → 用 Metal 打开为纹理;注意头文件明确
  "handle 每帧可能不同、不可缓存、回调返回即回收",必须每次重开并把内容拷进自有纹理)、
  `on_cursor_change`、`on_scroll_offset_changed`、`on_ime_composition_range_changed`、
  `on_popup_show`/`on_popup_size`、`start_dragging`/`update_drag_cursor`。
- 合成落地(两条路,**推荐第一条**,不动 warpui 场景):
  1. **NSView + CALayer**:我们自建子视图(仍挂在 `WebViewContainerView` 下,保持现有分层),
     把 IOSurface 作为 `layer.contents`;alpha 由 CA 与下层 `MetalBackgroundView` 合成 ⇒ 真透明且不改 warpui 渲染管线。
  2. warpui 场景内画四边形:需要新增"外部纹理 element + Metal 采样",改动面大、还要处理与现有 hole/hit-test 的关系。

### 3.2 输入(`cef-rs` 已确认可用的方法名)
- 鼠标:`send_mouse_click_event` / `send_mouse_move_event`(含 `mouse_leave`) / `send_mouse_wheel_event`
- 键盘/焦点:`send_key_event`、`set_focus`、`send_capture_lost_event`
- 帧率:`set_windowless_frame_rate`(默认 30,建议 60);尺寸变化 `was_resized`;可见性 `was_hidden`
- **坐标**:OSR 的事件坐标是视图像素坐标,需要把 zap 逻辑坐标 → AppKit 翻转 → 像素(乘 DPI)三段换算(现有 `flip_rect_to_appkit` 可复用前半段)。
- **事件从哪来**:windowed 下事件由 CEF 自己的子视图原生接收;OSR 后没有浏览器视图,**必须自建 NSView 并把
  `mouseDown/Dragged/Up/Moved/scrollWheel/keyDown/keyUp/flagsChanged` 转发到 CEF**(ObjC 侧 `cef_support.m` 扩写)。

### 3.3 高风险项
| 风险 | 说明 | 影响 |
|------|------|------|
| **中文输入法(IME)** | windowed 下浏览器视图自己实现 `NSTextInputClient`,中日韩输入开箱可用;OSR 下**必须由我们的 NSView 实现该协议**,再翻译成 `ime_set_composition`/`ime_commit_text`/`ime_finish_composing_text` + `on_ime_composition_range_changed` 回显候选框位置 | 做不好=用户打不了中文,是**否决级**风险 |
| `<select>` 等页面内弹层 | OSR 下弹层要自己渲染(`on_popup_show/on_popup_size` + popup 帧) | dsh 的 Web UI 若用到原生下拉,会出现"点开无内容" |
| 输入保真 | 修饰键、双击/三击、拖选、滚轮惯性、鼠标离开 | 体感退化 |
| 性能 | 共享纹理路径 60fps 可行;软件路径不可用 | 需实测 |
| 无障碍/朗读、拖放、触摸板手势 | OSR 需逐项补 | 功能缺口 |
| DevTools | 仍是独立 windowed 浏览器窗口,不受影响 | 无 |

## 4 改动清单与工作量(方案 D)

| 文件 | 改动 |
|------|------|
| `app/src/browser/cef_backend.rs`(现 927 行,17 处 windowed 专有点) | 新增 RenderHandler + OSR 生命周期;删/替换 `set_as_child`、`window_handle` 取视图、`setFrame`/`setHidden`/`removeFromSuperview` 等 windowed 路径;输入转发入口;popup 缓存 |
| `app/src/platform/mac/objc/cef_support.m` | 自建承载视图(NSTextInputClient + 事件转发 + CALayer contents 更新) |
| `app/src/browser/browser_web_view.rs` / `browser_pane_view.rs` | 几何/可见性接口语义不变,内部实现换 OSR |
| `tools/cef-spike` | 增加 OSR 探针(先验证 ISO 面→CALayer 合成与 IME,再动主仓) |
| 估计 | **5~10 人日**(含 spike 2 天);IME 与弹层是主要不确定性 |

## 5 验收标准(方案 D 若实施)

1. **透明**:窗口不透明度设 80% 时,CEF 区域与周围像素一致透出桌面 —— 用 `screencapture` + 采样同一行的像素比对(RGB 差值 ≤ 2/255),并与 wry 路径同场景对照。
2. **背景图**:主题带背景图时,CEF 区域显示背景图对应区域(而非纯色)。
3. **输入**:中文输入(拼音→候选→上屏)、英文、`Cmd+C/V`、选中、右键菜单、滚轮、鼠标进出 全部与 wry 路径一致。
4. **弹层**:页面内 `<select>` / 右键菜单可正常展开。
5. **性能**:滚动 dsh 页面时 `windowless_frame_rate=60` 下无明显掉帧(可用 Instruments 采样 CPU/GPU)。
6. **回归**:windowed 相关能力不受影响(DevTools、冻结/解冻、隐藏即隐藏、懒初始化)。

## 6 建议

1. **若你的痛点是半透明窗口/背景图下的色块** → 先做 **B(注入底图)** 或 **C(窗口钉不透明)**,一天内可验证,不动渲染架构。
2. **若明确要"真透明"** → 按 D 走,但**先花 2 天做 spike**(`tools/cef-spike` 内):只验证两件事 ——
   ① `on_accelerated_paint` 的 IOSurface 贴 `CALayer.contents` 能否在 zap 分层下正确透出;
   ② 我们自建 NSView 的 `NSTextInputClient` → CEF IME 链路能否打出中文。
   两件都通过再动主仓,否则成本会失控。
3. 无论选哪条,**当前的填色方案应保留**(它是 OSR 未启用时的兜底,也保证默认配置视觉一致)。
