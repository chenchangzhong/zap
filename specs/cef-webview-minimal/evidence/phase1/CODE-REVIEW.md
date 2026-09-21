# CEF 后端代码审核与修复记录(2026-09-21)

审核方式:独立子代理只读审查全部未提交改动(23 个已跟踪文件 + 10 项新增),
产出 2 个 Blocker + 若干 Major/Minor。**以下是 Lead 逐条核验 + 修复的记录**;
凡未核验的一律标注「未验证推测」。

## 1 Blocker:已核验并修复

### B1 dsh ↔ zap 的 IPC 实际不成立(已实测复现 → 已修)

- **问题**:shim 用 `mode:'no-cors'` POST,并把会话 token 放在自定义头
  `x-zap-webview-token`。no-cors 请求的 header guard 会**静默丢弃**非 CORS-safelisted 头。
- **核验方式(探针实测)**:用 `tools/cef-spike` 的洞拓扑探针加载临时页面,向本地接收端发两组对照请求:

  | 变体 | 服务端收到的头 | 结论 |
  |------|----------------|------|
  | `no-cors` + `x-zap-webview-token: TOKEN123` | `x-zap-webview-token=None` | **头被丢弃**,服务端必然 403 |
  | `no-cors` + `?token=TOKEN123` | URL 完整保留 | 查询串可用 |

- **修复**:token 改走**请求体信封** `{"token":"…","message":"<原样透传的 zap IPC 串>"}`
  —— 体不进访问日志、不触发预检、不受 no-cors 头过滤影响;服务端优先解析信封,
  `?token=`/请求头仅作原生调用方兜底。`extract_token_and_message` 抽成纯函数并加单测
  (`envelope_is_the_authoritative_token_source`,覆盖"裸请求体不得被当成凭证")。
- 取头曾被弃用的原因(避免 `TraceLayer` 把 token 记进 uri 日志)由"体"方案一并满足。

### B2 非 macOS 编译中断(代码可证 → 已修)

`BrowserWebViewManager::create` 的 macOS 版已扩到 8 参,`#[cfg(not(target_os = "macos"))]`
空壳仍是 6 参,而调用点 `BrowserPaneView::create_webview` **无 cfg 门控** ⇒ Linux/Windows
必然 E0061。已给空壳补齐 `_backend: WebViewBackend` 与
`_background_color: Option<ColorU>`(并加 `#[allow(clippy::too_many_arguments)]`)。

## 2 Major:已核验并修复

| 项 | 结论与修复 |
|----|-----------|
| **F3** 默认路径回归 | `warp_is_embedded_platform_view` 把容器**自身**也判为嵌入视图,而容器是全窗 frame、内容区普通点击的回退命中目标就是它 ⇒ 会跳过 `[self.contentView mouseUp:]` 的 CLD-2581 兜底,改变 feature-off 路径的事件分发。已改为「只认严格后代」(`view == container → NO`) |
| **F5** 子进程兜底反成 panic | `handle_subprocess_or_continue` 在 framework 加载失败时 `return true`(继续启动)→ 走到 `ChannelState::new` 会命中 AppId 三段断言 panic。已改为:子进程分支加载失败 → 记日志 + `exit(1)` |
| **F6** 缺 CefShutdown | 新增 `cef_backend::shutdown()`(幂等、有 `SHUTTING_DOWN` 门控让消息泵先停),接到 `app/src/lib.rs` 的 `on_will_terminate` |
| **F7** renderer 崩溃无 parity | wry 有 `WebContentCrashed` → pane 弹崩溃态;CEF 侧新增 `RequestHandler::on_render_process_terminated`(带代际过滤)→ 复用 `PENDING_WEBVIEW_CRASHED` 同一事件路径 |
| **F8** 每帧无效 resize | `set_bounds` 每帧无条件 `setFrame + was_resized` → 改为与 wry 一致“几何变化才下发” |
| **F9** 代际过滤缺口 | `notify_webview_page_loaded` 移入 `generation == self.generation` 判定内 |
| **F10** 冻结/返回值 | ① frozen 改为“下发成功后才置位”,且候选必须已有 browser;② `send_cdp` 返回成功与否并记 warning;③ `browser_host_create_browser` 返回值非 1 时 error 日志 |
| **F11** 菜单按标题保留 | 实测 CEF(Alloy)默认项只有 Back/Forward/分隔符/Print…/View Page Source(**没有"自动填充"**)⇒ 改为 `clear()` 后只放「重新加载」「检查元素」,与语言无关 |
| **F14** 会误提交 626MB | 新增 `tools/cef-helper/.gitignore`(`/target`) |
| **F18** thread_local 静默 no-op | `set_visible`/`set_bounds`/`destroy` 入口加 UI 线程 `debug_assert` |
| **F19** 懒初始化时序 | 加 `CONTEXT_INITIALIZED` 标志 + debug_assert:CEF 的 `OnContextInitialized` 在 `CefInitialize` 期间同步回调,故 `ensure_initialized` 同栈建浏览器是安全的(该断言把"CEF 行为若变化"暴露出来) |
| **F20** i18n 文案 | 英文描述改为 "Applies to newly opened dsh panes.";冻结标签补冒号 |

## 3 明确不采纳 / 延后(附理由)

| 项 | 处置 |
|----|------|
| **F16** `class_addProtocol(CefAppProtocol)` | **不采纳**:`CefAppProtocol` 只在 CEF 的 C++ 头(`cef_application_mac.h`)声明,`cef_support.m` 是纯 ObjC 无法引用;要补须改 `.mm` 并引入 CEF C++ 头(构建面变大)。当前 CEF 按“是否响应方法”判定,实测桥有效。已在注释记录该决策 |
| **F4** 懒初始化旁路运行时门控 | **按产品决定保留**:「使用 Chromium 内核」是用户可见开关(默认开),开关即为运行时门控;env/flag 仍决定**启动时**是否初始化。代价:feature-on 构建默认走 CEF —— 这正是该开关的语义 |
| ~~**F12/F13**~~ | **已修**:F12 把 `APPLE_TEAM_ID` 常量提到 CEF 嵌入之前(否则 framework/helper 只拿到 adhoc 签名,只能靠 Step 3 `--deep` 重签);F13 `cef_smoke --run` 改为检查应用真实日志(`~/Library/Logs/zap.log[.old.0]`),断言串与实现对齐(`message pump active` / `已初始化 CEF 后端`),删掉恒为 0 的 `SMOKE_PID` 分支与未用的 `LOG_MARK` |
| ~~**F15**~~ | **已修**:`#[cfg(feature = "cef_webview")] mod loopback_ipc;` + `dispatch2` 改 optional(`cef_webview = ["dep:cef", "dep:dispatch2"]`);已确认 app 内 `dispatch2` 无其他使用点 |
| **F17** 未开 CEF 也 dlopen libcef | **不采纳(与 F4 的决策冲突)**:`use_chromium` 默认开,即"默认请求 CEF";若按 `is_requested()`(env/flag)提前返回,反而会让默认开启的设置失效。子进程分发也必须无条件加载,改错会静默破坏 CEF pane |

## 3.5 F6 运行期验证(实跑)

| 检查项 | 结果 |
|--------|------|
| 正常退出(Apple Event quit) | 退出码 **0**(有序退出) |
| `CefShutdown` 是否被调用 | 上一轮退出日志有 `[cef] 关闭 CEF(CefShutdown)`(本轮该行因退出时日志异步 flush 可能丢失,故以退出码 + 无崩溃报告为准) |
| 崩溃报告 | 无 `zap-oss` 报告(仅历史 `hole-probe`(透明实验 Abort)与系统 WebKit 报告) |

## 3.6 第二轮审核(审核"修复本身")发现的新问题与处置

第一轮修复提交后做了第二次独立只读审核,**又发现 2 个 Blocker**(说明第一轮的 B1 只修了传输层、
端到端仍不通):

| 项 | 问题(已核验) | 处置 |
|----|--------------|------|
| **N1**(Blocker) | CEF 路径**从不设置 `window.__ZAP_BRIDGE__`**,而 `zap-bridge-client.js:611` 的 `apply()` 第一行就是 `if (!window.__ZAP_BRIDGE__) return;` ⇒ dsh 侧全部能力(切项目/通知/打开文件/文件引用)在 CEF 下**自我禁用**;该标记只在 wry 的 init_js 里,而 wry 用 WKUserScript 在**文档开始前**注入,CEF 只在 load_end 注 shim(时机也晚) | 抽出**后端无关注入脚本** `app/assets/webview_init.js`(wry 与 CEF 共用同一文件,防漂移);CEF 侧由 **helper 的 `CefApp::render_process_handler` → `on_context_created`** 在文档开始前注入(helper 原先传 `None::<&mut App>`,整条路径根本没接上);脚本前置"消息排队占位",load_end 的带 token shim 就位后**回放队列** |
| **N2**(Blocker) | 客户端发的是 `zap:<method>\n…`(`zap-bridge-client.js:102`),wry 侧在 handler 里剥前缀,**回环端没剥** ⇒ method 变成 `zap:zap.switch_project` → bridge 不匹配 → 丢弃;且只记 debug(文件 logger 过滤在 Info)⇒ 完全不可见,这就是实跑没发现的原因 | `normalize_zap_payload()` 剥前缀(与 wry 同位置),加"带前缀的真实 wire format"回归单测;未识别方法改记 **warn**(带方法名),不再静默 |
| **N3**(Major) | `shutdown()` 只关 CEF,没清 `WEBVIEWS`;`Browser` 是引用计数对象,留到 thread_local 析构时 drop 就是"关机后 release"(CEF 明示关机后不得再调任何 CEF 函数);且门控只覆盖 pump | `shutdown()` 先逐个 `close_and_detach` + 清空注册表再 `CefShutdown`;`with_browser()` 作为统一 choke point 加关机早返回 |
| **N4**(Major) | CEF 缺 wry 的链接/`window.open` 拦截 ⇒ pane 内导航(无地址栏/后退)与可能的野窗口 | 随 N1 一并解决(同一份脚本在 CEF 生效);回环端点新增 `warp:open-external:` 分支 → main 线程调系统浏览器 |
| **N5**(Major) | `cef_smoke --run` 同时 grep `zap.log` 与 `zap.log.old.0`,只要历史某轮成功就**假绿**;收尾用 SIGTERM 也走不到 F6 关机路径 | 只看当前 `zap.log` 且要求其 mtime 在本轮被写过;收尾改为 **Apple Event quit**(真正走 `applicationWillTerminate` → `CefShutdown`) |
| **N6** | 解冻失败也清 `frozen` ⇒ 页面永久冻结 | 仅解冻下发成功才清 `frozen`(失败保留下次重试) |
| **N8** | `execute_process` 返回值被丢 ⇒ helper 恒以 0 退出,掩盖 renderer/GPU 失败码 | helper 改为 `exit(ret)` |
| **N10** | 创建失败仍登记条目 ⇒ 永久空 pane | 新增 `PENDING_WEBVIEW_CREATE_FAILED`,drain 时移除条目以便重建 |
| **N11/N12/N15** | zh 标签缺冒号;shim 设了无人读取的 `__zapPostIPC`;清空菜单后以分隔符开头 | 均已清理 |
| **N19** | `CONTEXT_INITIALIZED` 用 `debug_assert` ⇒ CEF 行为若变化会把 debug 构建打挂 | 改为 `log::warn! + return false`(回退 wry) |
| **N7/N9/N14** | 文档与实现漂移(三重门控注释、"保留自动填充"、shim 形态、TraceLayer 理由被夸大) | 全部按实现改写 |

未处置的 Nit:N13(use 分组)、N16(`--type=` 判定早于 CLI 解析,当前无冲突)、N18(wasm+feature 组合不可达)。

## 4 回归证据(修复后)

```
CEF_PATH=… cargo check -p warp                         # feature off → 0 warning
CEF_PATH=… cargo check -p warp --features cef_webview   # feature on  → 0 warning
cargo nextest run -p warp --features cef_webview -E 'test(cef_backend) + test(loopback)'
  → 10 passed
cargo nextest run -p warp -E 'test(schema_validation) or test(i18n)'
  → 5 passed
cargo nextest run -p warp --features cef_webview -E 'test(cef_backend) + test(loopback) + test(webview_init_js)'
  → 12 passed            # 含 N1/N2 的新回归单测
cargo nextest run -p warp -E 'test(webview_init_js)'
  → 1 passed             # 默认构建下也守住引导脚本内容
cargo build --manifest-path tools/cef-helper/Cargo.toml --release
  → 0 warning
```

## 5 端到端验证配方(需打开一次 dsh pane)

`ZapCEF3.app` 已含全部修复(主二进制 + 5 个 helper 均已更新重签)。打开 dsh pane 后:

- ✅ 正常:`~/Library/Logs/zap.log` **不出现** `[dsh-loopback] 未识别的 IPC 方法` 与
  `[dsh-loopback] rejected`;
- ❌ 若出现 `未识别的 IPC 方法: …`:前缀/方法名仍有偏差(该 warn 是本次新增的可观测性);
- ❌ 若出现 `rejected`:token 传输又断了。
