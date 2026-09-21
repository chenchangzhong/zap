# 无需 GUI 的静态/半静态取证(阶段 1)

> 2026-09-21。本文件归档"不需要屏幕/交互也能做"的验证,用于把评审里若干
> 【未验证推测】转成有证据的结论。所有命令可复跑。

## 1 Chromium 宿主视图是否实现标准编辑 selector(评审 Important 4)

**背景**:`window.m` 的嵌入视图支路会把 Cmd+V/X/A 直接发给 first responder;
若该视图未实现 `paste:`/`cut:`/`selectAll:`,会 unrecognized selector 崩溃。

**取证**:`tools/cef-spike/probes/cef_selector_probe.m` —— dlopen CEF framework 后
用 ObjC runtime 查类方法(不需要浏览器实例、不需要窗口)。

```bash
FW="$HOME/.local/share/cef/Chromium Embedded Framework.framework/Chromium Embedded Framework"
clang -fobjc-arc tools/cef-spike/probes/cef_selector_probe.m -o /tmp/cef_selector_probe -framework Foundation
/tmp/cef_selector_probe "$FW"
```

输出:
```
class RenderWidgetHostViewCocoa: found
  paste:      YES
  cut:        YES
  selectAll:  YES
  copy:       YES
class CefBrowserHostView: found
  paste:      no
  cut:        no
  selectAll:  no
  copy:       no
```

**结论**:
- CEF 下真正承接键盘的 first responder 是 `RenderWidgetHostViewCocoa`(探针实测:
  hole-probe 的 `firstResponder=RenderWidgetHostViewCocoa`),它实现了全部四个 selector
  ⇒ Cmd+V/X/A 在 CEF 上可用。
- 外层 `CefBrowserHostView` **没有**实现 ⇒ 若焦点落在它上面,旧代码(无守卫)会崩溃。
  因此 `window.m` 新增的 `respondsToSelector` 守卫是必要兜底而非冗余。

## 2 回环 shim 的传输与幂等(有单测锁定)

`cargo nextest run -p warp --features cef_webview -E 'test(loopback)'`(5/5)锁定:
- token 走请求头 `x-zap-webview-token`,**不得**出现在 URL(`?token=` 会被
  `http_server` 的 `TraceLayer` 把 uri 记进日志);
- 常量时间比较,长度/单字符不同均拒绝;
- 体限 64KiB;
- shim 幂等守卫 `__ZAP_LOOPBACK_IPC__`,且**每次主 frame 加载都要注入**(否则第二次
  文档加载后 `webkit.messageHandlers.ipc` 不存在 ⇒ `zap.*` 静默失效)。

## 3 坐标翻转公式(有单测锁定)

`flip_rect_to_appkit` 单测锁定 `y_appkit = parentHeight - y - h`(与 wry 的
`window_position` 一致):顶部 rect → 200、底部 rect → 0、父高 0 时公式稳定无 panic。

## 4 默认(feature-off)路径等价性(静态核对)

- CEF 的 manager 分支全部在 `#[cfg(all(target_os = "macos", feature = "cef_webview"))]` 下,
  wry 分支未被改写;CEF 元数据另表(`cef_entries`)。
- 回环端点注册:`#[cfg(feature = "cef_webview")]` + 运行时 flag + 环境开关 `ZAP_CEF_WEBVIEW`
  三重门控 ⇒ feature-off 构建不会注册新监听面。
- 唯一非门控改动:`window.m` 的嵌入视图判定泛化(对 WKWebView 等价,见 RESULT.md §2 与
  评审核实)+ Cmd+C 对非 `WKWebView` responder 改走 `copy:`(旧代码在该分支是强转风险)、
  Cmd+V/X/A 加 selector 守卫。

## 5 为什么没跑完整 release-lto 打包(资源约束,非技术阻塞)

`./script/macos/bundle --channel oss --selfsign --nouniversal --arch aarch64 --cef` 需要一次
release-lto 构建。实测环境:`target/` 已 **127GB**,所在分区**仅剩 13GB**;LTO 重链
(含新增 cef 依赖与 cmake wrapper)有写满磁盘的实际风险,而磁盘写满会影响整机
(不止本仓库)。因此**不主动执行**,留给用户在确认磁盘余量后自行跑或授权执行。

前置依赖已确认就位:`create-dmg`(/opt/homebrew/bin)与 `cargo-about 0.9.2`。

## 6 仍未验证(必须实跑)

1. zap 本体 + CEF 的 GUI 行为:dsh pane 渲染、IPC 4 方法、reload 后 shim 仍在、
   切 tab/关 pane 后主窗存活与无残留视图、内存回落基线;
2. `do_close` 在真实 CEF 上是否确实阻止"关闭转发给顶层窗口"(仅按 CEF 头文件语义实现);
3. `script/macos/bundle --cef` 的完整 release-lto 打包与最终签名/公证;
4. 像素级渲染证据(屏幕锁定期间无法截图;`probes/run_hole_probe.sh` 可在屏幕可用时补跑)。
