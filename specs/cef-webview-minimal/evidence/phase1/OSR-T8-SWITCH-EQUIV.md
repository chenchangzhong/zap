# OSR T8:开关、回滚与等价性(进行中)

> 目标见 [OSR-PLAN.md](../../OSR-PLAN.md) T8。状态:**渲染模式设置项已完成;等价性验证部分完成**,
> 过程中发现并修掉一个**用户实机命中的崩溃**(下节)。

## 1 设置项(已完成)

| 层 | 改动 |
|----|------|
| `app/src/settings/cef_webview.rs` | 新增 `use_osr_rendering`(bool,默认 **false**,toml `general.webview.use_osr_rendering`)。注释写明它是**进程级**开关、**下次启动生效**,以及为什么需要它(windowed 的 CEF 子视图在半透明窗口里做不到真透明) |
| `app/src/lib.rs` | 每帧推入设置(与 `freeze_after_secs` 同一模式) |
| `app/src/browser/cef_backend.rs` | `render_mode()` → 纯函数 `resolve_render_mode(env, setting)`:环境变量 `ZAP_CEF_OSR` **显式非空**则覆盖(dev 排查/灰度),否则跟随设置项;结果由 `OnceLock` 缓存 |
| `app/src/settings_view/features_page.rs` + `app/i18n/{en,zh-CN}/warp.ftl` | 设置页新增开关(放在「使用 Chromium 内核」与冻结超时之间,与既有 `UseChromiumWebviewWidget` 同构) |
| `app/src/browser/cef_backend_tests.rs` | 新增 `resolve_render_mode_env_wins_then_setting`:默认/设置项/env 覆盖(两个方向)/空值回落 共 6 组断言 |

**回滚基线**:`use_osr_rendering = false`(默认)且不设 `ZAP_CEF_OSR` ⇒ `RenderMode::Windowed`
⇒ 与改动前逐字节等价(T4 已证)。

## 2 【Critical·实机命中并已修】切 tab 崩溃:借着重入 + `extern "C"` 里的 panic = abort

用户实机复现:按快捷键切 tab **必闪退**。崩溃报告(`EXC_CRASH/SIGABRT`)栈自证:

```
set_visible(false) → with_browser【已持 WEBVIEWS.borrow_mut()】
  → warp_cef_osr_view_set_hidden(1) → [NSView _setHidden:] → [NSWindow _realMakeFirstResponder:]
    → [WarpCefOsrView resignFirstResponder] → osr_focus_trampoline（Rust 回调）
      → core::panicking::panic_cannot_unwind → abort
```

`setHidden:` 会让 first responder 让位 ⇒ 我们自己的 `resignFirstResponder` 同步回调进 Rust
⇒ `focus(id, false)` 再借 `WEBVIEWS` ⇒ 重入 panic;而 panic 发生在 `extern "C"` 回调里会
**直接 abort**(Rust 不能跨 C 边界 unwind)。

**这条正是 T7 审核时被我标为"残余风险"却没动手的项**(当时判断"AppKit 隐藏视图大概不会让位"——错了)。

修法(系统性,不是补症状):

1. `with_browser()` 与 `browser_snapshot()` 改用 `try_borrow[_mut]`:借不到(说明是重入)
   **跳过本次操作**而不是 panic —— 这两个函数是所有回调入口的公共路径;
2. `focus()` 里读视图指针的那次借用同样改 `try_borrow`;
3. 复核其余借用点:render/display/keyboard handler 按 T4 的设计**完全不碰 `WEBVIEWS`**
   (只读 `Rc<OsrState>`),`spawn_browser` 的那次借用不跨对外调用 —— 均无同类风险。

实机验证:同一实例切 tab **不再闪退**,且无新增崩溃报告。

## 3 等价性验证状态

| 项 | 状态 | 说明 |
|----|------|------|
| 默认 windowed(windowed 回归) | ✅ | T4 证据 + 本次默认值 false 的解析单测 |
| OSR 下渲染/输入/IME/弹层/菜单 | ✅ | T4/T5/T7/T6 各轮实机 |
| **切 tab(隐藏)不崩溃** | ✅ | 本节 §2;实机确认 |
| **冻结 → 解冻**完整路径 | ⏳ **未跑通** | 启动时给了 `ZAP_CEF_FREEZE_AFTER_SECS=5`,但日志里没有"隐藏超时,已冻结页面"/"重新可见,解冻页面"(用户切回约 <5s)。需要**切走后停 20s 以上**再回来;若仍无日志,则要查 `send_cdp` 在 OSR 下是否真的下发成功(失败分支目前是静默重试) |
| 设置页切换开关 + 重启 | ⏳ 未验证 | 需先在设置页打开「使用无窗口(OSR)渲染」,再用**不带 `ZAP_CEF_OSR`** 的命令启动,确认走 OSR(可看日志 `mode=Osr` 或 `OSR surface …`) |
| renderer 崩溃 → pane 进崩溃态 | ⏳ 未验证 | 需 pane 打开时 kill 掉 `--type=renderer` 的 helper(自建实例的 helper 路径含 `target/cef-smoke/ZapCEF.app`) |
| 懒初始化(先 windowed 后开 Chromium 内核) | ⏳ 未验证 | CEF 按需初始化路径未在 OSR 开关下复测 |

## 4 复验命令

```bash
export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)     # 本机 CLT/Xcode SD 解析问题,见 §5
export CARGO_INCREMENTAL=0                               # 增量缓存坏掉时的规避(见 §5)
CEF_PATH="$HOME/.local/share/cef" script/macos/cef_smoke

# A. 冻结/解冻(阈值调小,便于观察):
ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 ZAP_CEF_FREEZE_AFTER_SECS=5 \
  RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 日志:grep -a "已冻结\|解冻" ~/Library/Logs/zap.log

# B. 只靠设置项走 OSR(不带 ZAP_CEF_OSR):
ZAP_CEF_WEBVIEW=1 RUST_LOG="warp::browser::cef_backend=debug" \
  target/cef-smoke/ZapCEF.app/Contents/MacOS/zap-oss
# 预期日志:webview … mode=Osr / OSR surface …
```

## 5 本机环境问题(与代码无关,记录备查)

1. `DockTilePlugin` 链接偶发失败(`tapi error: … unknown architecture arm64e.x1-macos`):
   用 `export SDKROOT=$(xcrun --sdk macosx --show-sdk-path)` 可绕过。
2. `cargo` 增量缓存偶发损坏(`unable to copy … .o: No such file or directory`):
   用 `CARGO_INCREMENTAL=0` 构建即可(或删 `target/debug/incremental`)。
