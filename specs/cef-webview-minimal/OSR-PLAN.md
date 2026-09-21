# OSR 真透明实施计划（dsh pane · CEF windowless）

> **给新会话的执行者**:本文件是自包含计划,不需要之前的对话上下文。开始前先读:
> `AGENTS.md`(仓库纪律)、本文件、[TRANSPARENCY.md](TRANSPARENCY.md)(为何 windowed 不行 + 实测证据)、
> [CEFSWIFT-EVALUATION.md](CEFSWIFT-EVALUATION.md)(参考实现与 API 清单)、
> [evidence/phase1/CODE-REVIEW.md](evidence/phase1/CODE-REVIEW.md)(既有实现已修过什么)。
>
> 基线:分支 `develop-CEF/Chromium`,HEAD 已含 windowed CEF 阶段 1 与两轮审核修复
> (`5d3a497dc`、`1c167e7e9`、`a1356541c`)。工作树应为干净。

## 0 目标与验收

**目标**:让 dsh pane 的 webview **真透明** —— 窗口不透明度 < 100% 时透出桌面,主题带背景图时显示背景图对应区域。

**为什么必须走 OSR**:windowed 模式已被**实测排除**(透明设置 + layer 置非不透明 + 页面全透明 ⇒ 洞内仍 `#FFFFFF`),
CEF 头文件也明确只有 windowless 才启用透明绘制(`cef_types.h:701-708`)。详见 TRANSPARENCY.md §1.3。

**非目标(明确不做)**:
- 不整体集成 CefSwift(理由见 CEFSWIFT-EVALUATION.md §2/§3)。
- 不改 dsh 前端(第三方产品,归属约束见 AGENTS/MEMORY)。
- 不删 windowed 代码路径 —— OSR 以**开关**形式并存,便于对照与回滚。
- 默认(wry)路径必须逐字节不变;非 macOS/未开 feature 的构建必须照常编译。

**验收标准(可测量,缺一不可)**:
1. **透明**:窗口不透明度设 80% 时,用 `screencapture` + 采样同一水平线上"pane 内"与"pane 外"的像素,
   与桌面透出程度一致(RGB 差 ≤ 2/255);且与 wry 路径同场景对照。
2. **背景图**:主题带背景图时,pane 区域显示背景图对应区域(而非纯色)。
3. **功能等价**(逐项与 windowed 现状对照):中文输入法(拼音→候选→上屏)、英文输入、Cmd+C/V/X/A、
   选中与拖拽、滚轮、鼠标进出、右键菜单(重新加载/检查元素)、DevTools 可打开、
   隐藏(pane 后台)与冻结、切回、懒初始化(设置里开开关后新开 pane 生效)、renderer 崩溃后 pane 进崩溃态。
4. **构建**:`cargo check -p warp`(feature off)与 `--features cef_webview` 均 **0 warning**;
   `tools/cef-helper` release 构建 0 warning;打包仍走 `script/macos/bundle`(AGENTS §5.9)。
5. **性能**:`windowless_frame_rate=60` + 外部 begin-frame 下滚动 dsh 页面无明显掉帧(用 Instruments 粗采样 CPU/GPU)。

## 1 关键设计决策(执行者按此实现,除非有证据推翻)

| 决策 | 选择 | 理由 |
|------|------|------|
| 显示路径 | **自建 NSView(仍挂 `WebViewContainerView` 下)+ `CALayer.contents = IOSurface`** | 保持 zap 现有分层(Metal 背景 → webview 容器 → 带洞的 Metal 内容层)与命中逻辑;零拷贝、retina 正确;不触碰 warpui 渲染管线 |
| 备选(不选) | warpui 场景内 Metal 采样外部纹理 | 改动面大(新增 element + 纹理导入),且要重做洞/hit-test 关系 |
| 与现状共存 | 同一 CEF 后端内加**渲染模式**:`Windowed`(默认) / `Osr`;由环境变量 + 设置项控制 | 可对照、可回滚;默认保持 windowed 直到验收通过 |
| OSR 三开关 | `WindowInfo{ windowless_rendering_enabled=1, shared_texture_enabled=1, external_begin_frame_enabled=1 }`、`BrowserSettings.windowless_frame_rate=60`、`runtime_style=ALLOY` | 参考 CefSwift `createOSRBrowser`;外部 begin-frame 让滚动/动画按真实刷新率走 |
| 透明实现 | `BrowserSettings.background_color` 传 **alpha=0**(windowless 下即启用透明绘制) | 与 CEF 文档一致;IOSurface 带 alpha,CALayer 合成后自然透出 |

## 2 参考实现(照抄手法,不抄代码)

CefSwift(BSD-3,`Rajaniraiyn/CefSwift`)已把 OSR 的全部原生affordance跑通,按文件对照:

| 需求 | 参考位置 | 要点 |
|------|----------|------|
| OSR 创建参数 | `Sources/CefKit/CefBrowser.swift` → `createOSRBrowser` | 三个 windowInfo 开关 + `windowless_frame_rate` + 异步创建(`on_after_created` 回调后才拿到 browser) |
| IOSurface → CALayer | `Sources/CefKit/CefMetalHostView*.swift` | `on_accelerated_paint` 里 `CALayer.contents = IOSurface`,包 `CATransaction`;`<select>` 弹层用独立 `popupLayer`(由 `on_popup_size` 定尺寸) |
| CPU 兜底 | 同上 | `on_paint` → `CGImage`(仅作降级) |
| IME | `CefBrowser` 的 OSR Input 扩展 | `NSTextInputClient.setMarkedText/insertText` → `ime_set_composition`/`ime_commit_text`;候选框位置用 `on_ime_composition_range_changed` |
| 鼠标/滚轮/光标 | 同上 | `send_mouse_move_event`(含 mouse_leave)/`send_mouse_click_event`/`send_mouse_wheel_event`;`on_cursor_change` |
| 键盘/快捷键 | 同上 + host view | mac→Windows VK 映射;**KEYDOWN+CHAR 两段式**(否则 JS keydown 不触发、光标不动);`performKeyEquivalent` 转发 Cmd+A/C/V/X/Z;应用级 Cmd+Q/W/M/H 放行 |
| 编辑命令 | `CefBrowser` 的 Editing 扩展 | `copy:`/`paste:`/`selectAll:`/`undo:`… → focused frame 的 `copy/paste/select_all/undo` |
| 右键菜单 | `docs/configuration.md` + host view | **必须异步**:回调里跑 modal NSMenu 会崩 CEF;先快照 model,再在回调外弹菜单 |
| 几何/DPI | `CefBrowser` 的 Geometry 扩展 | `was_resized` / `notify_screen_info_changed`(retina) / `was_hidden` / `invalidate` / `send_external_begin_frame` |
| 无障碍(可选,后置) | 同上 | `set_accessibility_state(STATE_ENABLED)` + `cef_accessibility_handler_t` 桥成 `NSAccessibilityElement` |
| 顺带可捡 | 同上 | `find`(页内查找)、`zoom_level`、`set_audio_muted`、DevTools 传真实窗口尺寸(900×700) |

## 3 任务分解(每步都有验证;spike 先行,不通过就停)

> `TDD Route`: mode=`off`(无用户/项目显式 TDD 要求), decision=`skipped`(FFI/GUI 路径不适合先写失败测试)。
> 替代做法:**每个任务都给可执行验证命令或可采样证据**;纯逻辑(坐标翻转、DPI 换算、键映射表)仍补单测。

### T1(spike A)—— IOSurface → CALayer 在 zap 分层下能否正确透出 ✅ **已完成(通过)**
> 骨架已就绪:`tools/cef-spike/src/bin/osr-probe.rs` + `probes/osr_host.m`;
> 结果与证据见 [evidence/phase1/OSR-SPIKE-A.md](evidence/phase1/OSR-SPIKE-A.md)。
> 结论:windowless + alpha=0 背景 + 透明页面 ⇒ 帧带 alpha(实测 82.7% 像素 alpha=0),
> 洞内透出下层;不透明内容(红块/渐变)同样正确渲染;≈60fps。
> **T4 必须遵守**:`view_rect`/`ScreenInfo.rect` 用 **DIP**,否则会重复缩放(实测 2400x1600 vs 1200x800)。

- 位置:`tools/cef-spike`(独立 crate,不碰主仓)。
- 做:新增 OSR 探针(可复用 `hole-probe` 的洞拓扑:底层 `ProbeBackgroundView` #12171F、容器、覆盖层带洞),
  实现 `wrap_render_handler!`(至少 `view_rect`/`screen_info`/`on_accelerated_paint`),把 IOSurface 贴进自建 NSView 的 CALayer;
  加载**透明页面**(`probes/transparent_page.html`)与**不透明页面**(现有 `hole_page.html`)各跑一次。
- **验证(硬证据)**:`screencapture -x` + `probes/sample_pixel` 采样洞内中心:
  - 透明页面 ⇒ 期望 `#12171F`(下层背景色),**不是** `#FFFFFF`;
  - 不透明页面 ⇒ 期望页面渐变/标记色(证明内容确实渲染)。
- 失败判据:仍为白/黑/无内容 ⇒ **停止**,把证据写入 TRANSPARENCY.md,回到方案 B/C。

### T2(spike B)—— 自建 NSView 的 IME 链路能否打出中文 ← **下一步从这里开始**
- 做:探针的宿主 NSView 实现 `NSTextInputClient`(`setMarkedText`/`insertText`/`firstRectForCharacterRange`),
  转发到 `ime_set_composition`/`ime_commit_text`;页面放一个 `<textarea>` 并把内容 POST 到现有 `probes/loopback_receiver.py`。
- **验证**:手动输入拼音(如 "nihao")→ 候选 → 上屏;接收端日志里出现期望中文;
  并验证候选框位置(`on_ime_composition_range_changed` 有回调,日志可证)。
- 失败判据:打不出中文 ⇒ **停止**(IME 是否决级风险,见 TRANSPARENCY.md §3.3)。

### T3 —— 决策点
- T1、T2 都通过 ⇒ 进 T4;任一失败 ⇒ 记录结论、停止,不进入主仓改动。

### T4 —— 主仓:cef_backend 增加 OSR 渲染模式
- 文件:`app/src/browser/cef_backend.rs`(现 900+ 行)、`app/src/browser/browser_web_view.rs`。
- 做:`WindowInfo`/`BrowserSettings` 按模式分支;`wrap_render_handler!` 实现 `view_rect`/`screen_info`/`on_accelerated_paint`(+`on_paint` 兜底);
  IOSurface → CALayer;几何/DPI(`was_resized`/`notify_screen_info_changed`);`was_hidden`/`invalidate`/`send_external_begin_frame`。
- **验证**:`cargo check` 两种 cfg 0 warning;`nextest -E 'test(cef_backend)'` 全绿(坐标/DPI 纯逻辑补单测);
  真机打开 pane,DevTools 里确认页面渲染与尺寸正确(retina 无模糊)。

### T5 —— 输入与编辑命令
- 做:自建宿主视图转发 mouse/key/wheel/focus;编辑命令走 focused frame;`performKeyEquivalent` 处理 Cmd+A/C/V/X/Z。
- **验证**:逐项手工用例(选中、复制粘贴、滚轮、鼠标进出、光标形状变化),与 windowed 现状对照;日志可证 `send_mouse_*`/`send_key_event` 被调用。

### T6 —— 弹层与右键菜单
- 做:`on_popup_show`/`on_popup_size` → 独立 popup layer;右键菜单改**异步 NSMenu**(沿用现有三项:重新加载/检查元素,清空默认项)。
- **验证**:页面内 `<select>` 能展开;右键菜单三项可用且不崩。

### T7 —— IME 落地主仓
- 做:把 T2 的宿主视图实现移入 `app/src/platform/mac/objc/`(新增 `.m` 或扩展 `cef_support.m`),Rust 侧转发。
- **验证**:在 dsh pane 里实际输入中文(拼音→候选→上屏),候选框跟随光标。

### T8 —— 开关、回滚与既有策略的等价性
- 做:渲染模式开关(环境变量如 `ZAP_CEF_OSR=1` + 设置项),默认 windowed;
  确认隐藏/冻结(CDP `Page.setWebLifecycleState`)/懒初始化/renderer 崩溃在 OSR 下同样工作。
- **验证**:开关来回切换;pane 隐藏→切回;设置里开关 Chromium 内核后新开 pane;kill renderer 后 pane 进崩溃态。

### T9 —— 证据与文档归档
- 更新 `TRANSPARENCY.md`(实测像素证据 + 结论)、`RUNTIME-VERIFICATION.md`(OSR 实跑)、`TECH.md`(模式与开关)。
- 探针用完即删(临时探针规则);证据文件放 `specs/cef-webview-minimal/evidence/phase1/`(文本为主,截图仅必要几张)。

## 4 验证命令速查

```bash
# 构建(两种 cfg 都必须 0 warning)
cargo check -p warp
CEF_PATH="$HOME/.local/share/cef" cargo check -p warp --features cef_webview

# 测试(仓库规矩:一律 nextest)
CEF_PATH="$HOME/.local/share/cef" cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend) + test(loopback) + test(webview_init_js)'
cargo nextest run -p warp -E 'test(schema_validation) or test(i18n) or test(webview_init_js)'

# helper(改动过必须重打;它承载 render 进程的文档开始前注入)
CEF_PATH="$HOME/.local/share/cef" cargo build --manifest-path tools/cef-helper/Cargo.toml --release

# 探针(阶段 0/1 既有)
cd tools/cef-spike
CEF_PATH="$HOME/.local/share/cef" cargo run --bin make-bundle -- hole-probe -o target/hole
PROBE_TRANSPARENT=1 target/hole/hole-probe.app/Contents/MacOS/hole-probe \
  --url="file://$PWD/probes/transparent_page.html" --routing=plain --hole=100,100,600,400 \
  --click-after=25 --exit-after=40 --log=/tmp/osr.log
screencapture -x /tmp/osr.png && probes/sample_pixel /tmp/osr.png 1120,920 1800,1000

# 打包(必须走它;禁止 --skip-build / 替代命令)
./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef
./script/macos/cef_smoke            # 打包 + 冒烟(--run 走优雅退出)
```

## 5 风险与缓解

| 风险 | 级别 | 缓解 |
|------|------|------|
| IME(中文输入) | **否决级** | T2 先 spike 验证;失败即停止(参考 CefSwift 的 `NSTextInputClient` 映射) |
| `<select>` 等页面内弹层 | 高 | T6 用独立 popup layer;先确认 dsh 页面是否用到原生 select |
| 输入保真(修饰键/双击/拖选/滚轮惯性) | 中 | T5 逐项用例;参考 CefSwift 的 KEYDOWN+CHAR 两段式与 VK 映射 |
| 右键菜单崩 CEF | 中 | 必须异步 NSMenu(回调内 modal loop 会崩,参考实现已踩过) |
| 性能(60fps) | 中 | shared texture + external begin-frame;失败则降帧率(30)并记录 |
| 无障碍/拖放/手势 | 低(可后置) | 明确列为后置项,不阻塞验收 |
| 半透明窗口下桌面透出仍不对 | 中 | 验收标准 1 的像素比对即判据;必要时对照 wry 路径 |

## 6 边界与纪律(违反即返工)

- **禁止**终止/关闭用户正在运行的任何进程,尤其 `/Applications/Zap.app`;只启动/管理自建实例。
- 默认 wry 路径逐字节不变;OSR 代码全部 `#[cfg(all(target_os="macos", feature="cef_webview"))]` 门控。
- 打包必须走 `script/macos/bundle`;helper 必须走 `tools/cef-helper`(不得用主二进制当 helper)。
- 注释与回复用简体中文;改动每一行都能溯源到本计划的任务。
- 每个任务完成后归档证据;临时探针/日志用完即删。
- 不做计划外"顺手重构";发现无关问题只记录。

## 7 回滚

OSR 全程在**开关**之后,默认 windowed:
- 若 T1/T2 spike 失败:主仓零改动,仅归档结论。
- 若 T4+ 中途失败:关掉开关即回到 windowed 现状(该路径不删),并把失败点写入 TRANSPARENCY.md。
