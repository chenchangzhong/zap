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
| **冻结 → 解冻**完整路径 | ✅ **已实机验证** | 见 §3.1 的日志闭环:冻结与解冻**都成功**,且**未 panic**(上轮 Critical 所在路径第一次真正跑通) |
| 设置页切换开关 + 重启 | ✅ **已实机验证** | 见 §3.3:设置页打开开关后,用**不带 `ZAP_CEF_OSR`** 的命令启动 ⇒ `render mode = Osr (env ZAP_CEF_OSR="(unset)", setting use_osr_rendering=true)` + `OSR surface 3836x1908px` |
| renderer 崩溃 → pane 进崩溃态 | ✅ **已实机验证** | pane 打开时杀掉自建实例的 renderer helper,日志立刻出现 `[ERROR] [warp::dsh::pane] [dsh] webview crashed, showing reload prompt (webview_id=Some(1))`,pane 停在崩溃态(带重新加载提示),renderer 未被自动重建 |
| 懒初始化(启动期不初始化 CEF,由第一个 pane 触发) | ✅ **已实机验证** | 见 §3.3:不带 `ZAP_CEF_WEBVIEW`/`ZAP_CEF_OSR` 启动 ⇒ 启动后无模式日志,开 pane 时才出 `render mode = Osr (…setting=true)` + `OSR surface …`,用户确认 pane 正常可用。**这条同时验证了 `BrowserPaneView::new_dsh` 里"初始化前补推设置"的修复** |

### 3.1 冻结/解冻实机日志(闭环)

```
06:23:19 [warpui_core::platform::app] active window changed: None            ← pane 被隐藏/应用失活
06:23:22 [warp::browser::cef_backend] [cef] webview 1: 隐藏超时,已冻结页面(CDP setWebLifecycleState=frozen)
06:44:39 [warpui_core::platform::app] application did become active           ← 切回
06:44:41 [warpui_core::core::app] dispatching typed action: …WorkspaceAction::FocusPane(PaneId{ pane_type: DeepSeek … })
06:44:41 [warp::browser::cef_backend] [cef] webview 1: 重新可见,解冻页面
```

- 运行参数:`ZAP_CEF_WEBVIEW=1 ZAP_CEF_OSR=1 ZAP_CEF_FREEZE_AFTER_SECS=5`(把阈值调小便于观察);
  实例全程存活(pid 71932),**无新增崩溃报告**。
- 由此确认两件事:① `send_cdp`(CDP `Page.setWebLifecycleState`)在 OSR 下**确实下发成功**
  ——冻结成功即证明,不是此前担心的"静默失败";② "隐藏 → 冻结 → 切回 → 解冻"整条等价性路径成立,
  且**不再触发**上轮修掉的借着重入 abort(`set_visible` 已改为借用外调外部)。
### 3.2 renderer 崩溃态(实机)

```
06:48:45 [ERROR] [warp::dsh::pane] [dsh] webview crashed, showing reload prompt (webview_id=Some(1))
```

杀 renderer 后:pane 立刻进崩溃态并提供重新加载入口;主进程存活、无崩溃报告。
**教训(操作层面)**:杀进程必须用**只匹配自建实例**的模式(例如
`.../target/cef-smoke/ZapCEF.app/Contents/Frameworks/zap-oss Helper (Renderer).app`),
不要用 `Helper \(Renderer\)` 这类会命中所有 Chromium 应用的宽模式 —— 本轮误用过一次,
把用户机器的微信/Arc/Lark/VS Code 的 renderer 一起杀了(它们均已自动重建,无持久影响)。

- 备注:冻结只在 pane 隐藏且超过阈值时发生(`freeze_after_secs`,默认 300s;0 = 不冻结),
  与 dsh 的 Node 服务端/agent 任务无关(只停页面 JS 与渲染)。

### 3.3 设置项单独生效 + 懒初始化(实机日志)

把设置页的「使用无窗口(OSR)渲染」打开后,分别用两种启动方式验证:

```
# A. 只有 ZAP_CEF_WEBVIEW=1(启动期 eager init;不带 ZAP_CEF_OSR)
06:54:44 [INFO] [cef] render mode = Osr (env ZAP_CEF_OSR="(unset)", setting use_osr_rendering=true)

# B. 两者都不带(启动期不初始化 CEF,由第一个 pane 懒初始化)
06:55:58 [INFO] [cef] render mode = Osr (env ZAP_CEF_OSR="(unset)", setting use_osr_rendering=true)
06:56:00 [INFO] [cef] OSR surface 3836x1908px (view_rect=(1918, 954) DIP, scale=2.0)
```

- 两次的模式决策都**只来自设置项**(env 为 unset)⇒ C1 的顺序问题已修实。
- B 是**懒初始化**路径(启动后先无模式日志,开 pane 才初始化)⇒ 同时验证了
  `BrowserPaneView::new_dsh` 在 `ensure_initialized()` 之前补推设置的修复。
- 新增**永久观测点**:`[cef] render mode = … (env …, setting …)` —— 进程级模式只在此刻定一次,
  以后排查"到底听了谁"直接看这行(评审建议)。

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

## 6 交付前审核(两位独立审查者)与修复

### C1【Critical·已修】设置项被静默忽略:`render_mode()` 的 `OnceLock` 抢在"每帧推入"之前

- 判据(审查者给的行号 + 我复核):启动期 `lib.rs` 的 eager init(`is_requested()` 为真时)
  → `initialize_runtime()` → `initialize_inner()` → `render_mode()`(**首次读取**,`OnceLock` 定值);
  而唯一的推入点在**首帧**的 `on_frame_drawn` 闭包里,且还嵌在 `DshPane` flag 分支内。
  ⇒ 在 `ZAP_CEF_WEBVIEW=1`(或 dogfood flag)路径上,设置永远读不到。
- 影响:那条路径下"渲染模式设置项"是**空操作**,无任何告警(只有 `create_webview` 的
  `mode=Windowed` 日志能看出来)。
- 修法:**eager init 推迟到设置推入之后** —— 启动期只 `request_eager_init()` 登记请求,
  首帧把两项设置推到帧回调**最前面**(不再挂 flag 分支)后,再 `take_eager_init_request()`
  并做初始化 + 协议桥 + 泵(顺序与原来一致;若本会话已有 pane 走懒初始化则跳过)。
- 另外两条推入路径也已就位:`BrowserPaneView::new_dsh` 在 `ensure_initialized()` **之前**推一次
  (覆盖"恢复会话时 pane 先于首帧创建"的顺序)。注意这里踩过一个坑:把语句插在
  `#[cfg(...)]` 与它修饰的 `let` 之间会让 cfg 挂错对象 ⇒ 无 cef feature 的构建直接编译失败;
  已改成独立 `#[cfg] { ... }` 块(两套 cfg 都验过)。

### I1【Important·已收口】`set_visible` 曾跨对外调用持 `WEBVIEWS` 可变借用

崩溃根因所在(见 §2)。`try_borrow` 只是把 abort 降级为"跳过",根因是"持借调用外部"。
已改为:**借用内只取 `browser` 句柄与 OSR 视图指针,借用外再调 `setHidden`/`invalidate`/`was_hidden`**;
解冻写回用独立的短借用。与 `focus()`/`on_before_close`/`take_and_release_osr_view` 同一模式。

### D1【收口】`shutdown()` 在借用内 `close_and_detach`

CEF 文档对 `close_browser` 只说 "may complete either synchronously or asynchronously";
若同步回调 `on_before_close`(那里会 `borrow_mut`)即同类 abort。已与其他资源一样
**收集到借用外**再关闭/释放。

### M1【已补注释】渲染模式的优先级

环境变量**非空即覆盖**(含 `ZAP_CEF_OSR=0`/`=yes` 这类非真值 ⇒ 覆盖设置、回落 windowed),
只有空/空白才回落设置项;单测固化的正是这个语义,已在 `parse_render_mode` 上写明。

### 审查者确认无问题的部分

`try_borrow` 热修非重入语义与改前逐分支等价;6 个 `extern "C"` trampoline 内无 panic 源
(`CefString::from` 不 panic、`notify_*` 只入队+请求重绘);render/display/keyboard handler
完全不碰 `WEBVIEWS`;设置/UI/i18n 三层闭合(`SupportedPlatforms::MAC` 是运行时判定,
故未加 cfg 的 action/遥测臂在非 mac 也能编译);默认 false + 无 env ⇒ Windowed 基线不变。
