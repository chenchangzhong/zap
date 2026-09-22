# OSR T4:主仓 `cef_backend` 增加 OSR 渲染模式(通过)

> 目的:把 T1/T2 在 `tools/cef-spike` 验证过的 windowless 手法落进主仓,以**开关**形式
> 与 windowed 并存(默认 windowed,可回滚)。见 [OSR-PLAN.md](../../OSR-PLAN.md) T4。
> 结论:**通过** —— 真机 3 轮实跑(OSR 加速路径 / windowed 回归 / OSR CPU 兜底)全部正常,
> 并挖出两个"只有真机才暴露"的硬坑。

## 1 实现落点

| 文件 | 改动 |
|------|------|
| [app/src/browser/cef_backend.rs](../../../../app/src/browser/cef_backend.rs) | `RenderMode{Windowed,Osr}`(env `ZAP_CEF_OSR`,默认 windowed)+ `OsrState` 共享单元;`initialize_inner` 按模式置**进程级** `windowless_rendering_enabled`;`spawn_browser` 按模式分支(OSR:三开关 + DIP bounds + `windowless_frame_rate=60` + `background_color` alpha=0);新增 `wrap_render_handler!`(`view_rect`/`screen_info`/`on_accelerated_paint`/`on_paint`);几何/可见性/销毁/关机各路径按模式分支;`pump` 支持外部 begin-frame。 |
| [app/src/platform/mac/objc/cef_support.m](../../../../app/src/platform/mac/objc/cef_support.m) | 新增 `WarpCefOsrView`(非 ARC,显式 retain/release):`CALayer.contents = IOSurface`(零拷贝,每帧重贴 + `CATransaction` 禁隐式动画)、`on_paint` 的 BGRA→CGImage 兜底、frame/隐藏/scale/surface 尺寸读回。 |
| [app/build.rs](../../../../app/build.rs) | cef feature 下显式链接 QuartzCore / CoreGraphics / IOSurface(不依赖他 crate 的传递链接指令)。 |
| [app/src/browser/cef_backend_tests.rs](../../../../app/src/browser/cef_backend_tests.rs) | 新增 3 个纯逻辑单测:模式开关解析、OSR 视图尺寸为 DIP 且夹到 ≥1、默认 windowed。 |

开关(均为环境变量,设置项与 UI 归 T8):

| 变量 | 作用 |
|------|------|
| `ZAP_CEF_OSR=1` | 启用 OSR(windowless)。**不设=windowed,与改动前逐字节等价**。 |
| `ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME=1` | 由 zap 的 60Hz 泵驱动 `send_external_begin_frame`。默认关:T1 实测内部 60fps 节奏已够,外部驱动需严格按刷新率喂帧(OSR-PLAN.md §5 性能风险)。 |
| `ZAP_CEF_OSR_CPU_PAINT=1` | 关掉共享纹理,强制走 `on_paint` 位图兜底(验证/GPU 异常时的退路)。 |

## 2 真机暴露的两个硬坑(必须记住)

### 2.1 `was_resized()` 会**同步**回调 render handler ⇒ RefCell 重入 panic(首次真机跑必崩)

**现象**:打开 dsh pane 立即闪退。崩溃日志:

```
core::cell::panic_already_mutably_borrowed
  11: borrow<HashMap<u64, CefWebview>>            app/src/browser/cef_backend.rs:1161
  13: view_size_dip                                app/src/browser/cef_backend.rs:1160
  17: get_screen_info<WebviewRenderHandler, …>    cef-152.4.0/…/aarch64_apple_darwin.rs:23364
```

**根因**:`with_browser()` 持着 `WEBVIEWS.borrow_mut()` 调 `host.was_resized()`,而 CEF 在该调用内
**同步**回调 `GetScreenInfo`/`GetViewRect`;回调再 `WEBVIEWS.borrow()` ⇒ 重入 panic。
windowed 模式不会触发(那时 CEF 不调 render handler),所以**只有 OSR 真机才会暴露**。

**修复**:几何/视图指针改走独立的 `OsrState`(`Rc` + `Cell`),render handler **完全不碰 `WEBVIEWS`**;
每帧 `set_bounds` 在通知 CEF **之前**写入新尺寸,CEF 同步回调读到的就是最新值。
⇒ 不变量:**任何 CEF 调用都可能同步回调 render handler,故 render handler 不得借用 `WEBVIEWS`。**

### 2.2 windowless 下 `CefBrowserHost::GetWindowHandle()` 返回的是**父容器**

`browser_platform_delegate_osr_mac.mm:GetHostWindowHandle()` 返回 `window_info.parent_view` ——
即 warpui 的 `WebViewContainerView`。windowed 那套"拿 `window_handle()` 当浏览器子视图"
(`setHidden:`/`setFrame:`/`removeFromSuperview`)在 OSR 下会**把整个容器藏掉/摘掉**。
修复:OSR 分支一律操作自建宿主视图(`detach_view` 在 OSR 下直接早返回)。

## 3 验证

### 3.1 静态检查与单测

```bash
cargo check -p warp                                     # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo check -p warp --features cef_webview   # 0 warning
CEF_PATH="$HOME/.local/share/cef" cargo nextest run -p warp --features cef_webview \
  -E 'test(cef_backend)'                                # 6/6 passed
```

### 3.2 真机(debug 构建 + `script/macos/cef_smoke` 组装的自建 .app,bundle id `dev.zap.cef-smoke`)

> 全程只启动/管理自建实例,未触碰 `/Applications/Zap.app`;验证完已结束自建实例。
> 运行方式:`ZAP_CEF_WEBVIEW=1 [ZAP_CEF_OSR=1] ZapCEF.app/Contents/MacOS/zap-oss`,
> 再在窗口里「+」→「DeepSeek Harness」打开 dsh pane。

**A. OSR 加速路径(默认共享纹理)** —— 用户实机确认:页面渲染正常、**方向正确、不模糊**:

```
[browser] create webview 1 url=http://127.0.0.1:60926/?token=… backend=Cef
[cef] webview 1 parent=0x… in_window=true parent_frame=…1280x800… mode=Osr
[cef] webview 1 browser created
[cef] webview 1 loaded (shim 注入)
[cef] OSR surface 2x2px (view_rect=(1278, 763) DIP, scale=2.0)        ← 首帧(几何未落位)
[cef] OSR surface 2556x1526px (view_rect=(1278, 763) DIP, scale=2.0)  ← 2556 = 1278 × 2 ✓
```

窗口被拖大后继续正确跟随(证明 `was_resized` 链路):

```
[cef] OSR surface 2558x1526px (view_rect=(1510, 853) DIP, scale=2.0)
[cef] OSR surface 3020x1706px (view_rect=(1510, 853) DIP, scale=2.0)  ← 3020 = 1510 × 2 ✓
```

⇒ **surface = view_rect(DIP) × device_scale_factor**,retina 无重复缩放(即 T1 记下的
"view_rect 必须用 DIP"陷阱在主仓侧也守住了)。

**B. windowed 回归(不设 `ZAP_CEF_OSR`)** —— 用户实机确认:页面正常**且可正常操作**:

```
[cef] webview 1 … mode=Windowed
[cef] webview 1 browser created
[cef] webview 1 loaded (shim 注入)
```

**C. OSR + CPU 位图兜底(`ZAP_CEF_OSR_CPU_PAINT=1`)** —— 用户实机确认:页面正常渲染
(此时 `layer.contents` 是 CGImage 而非 IOSurface,故**没有** surface 尺寸日志,符合预期)。

三轮均无崩溃报告新增(唯一一份 `zap-oss-*.ips` 是 §2.1 那次修复前的崩溃)。

## 4 明确未做(按计划留给后续任务)

1. **输入**(鼠标/键盘/滚轮/焦点/编辑命令):T5。故 OSR 下 pane 目前"看得到、点不动"——
   用户实跑时如实反馈"无法任何操作",属预期。
2. **页面内弹层**(`<select>` 等):T6(popup layer);当前 `POPUP` 类型的 paint 直接忽略。
3. **透明验收**(窗口不透明度 80% 的像素比对):OSR 已按 `background_color=0` 开启透明绘制,
   但端到端像素验收属验收标准 1 / T8。
4. **设置项开关**:T8(`ZAP_CEF_OSR` 目前是环境变量;进程级开关在 `CefInitialize` 前读,故设置项
   需在初始化前可读)。
5. `ZAP_CEF_OSR_EXTERNAL_BEGIN_FRAME` 路径已实现但**未实跑**(默认关;T1 未启用该路径,
   性能验收属验收标准 5)。

## 5 复跑命令

```bash
cd /path/to/zap
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke      # 构建 + 组装 + 签名自建 .app

# A. OSR 加速路径
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# B. windowed 回归(默认路径)
ZAP_CEF_WEBVIEW=1 target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# C. CPU 位图兜底
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 ZAP_CEF_OSR_CPU_PAINT=1 target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 日志
tail -f ~/Library/Logs/zap.log | grep -a '\[cef\]'
```
