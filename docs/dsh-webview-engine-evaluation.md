# dsh webview 内核体验评估:留在 WebKit 还是换 Chromium

> 2026-09-20 评估。回答一个问题:**macOS 自带 WKWebView 承载 dsh Web UI 体验差,值不值得换内核、换哪个、成本多大。**
> 结论均附证据;标注「未验证」的条目不得作为决策依据(仓库纪律 §5.6.1)。

---

## 1. 结论先行

1. **原始性能差距比桌面跑分显示的更大**:桌面 Speedometer 上 Safari 与 Chrome 打平(M2 MBA 42.7 vs 41.9,JetStream Safari 反超;[Magic Lasso 2026-01](https://supasidebar.com/blog/fastest-browser-mac-2026)),但**差距主要被桌面双引擎的长期优化掩盖**——iOS 26.5 上 Edge 的 Blink 原型实测 Speedometer 3.1 领先 Safari **+28.6%**、JetStream +13.1%,差距集中在 JS 响应性(JSC vs V8)而非图形(MotionMark 仅差 2.1%)([Open Web Advocacy 实测](https://open-web-advocacy.org/blog/28-percent-faster--the-blink-prototype-that-shows-why-apples-ios-browser-engine-ban-must-end/))。dsh 的"流式输出+长对话高频 setState"正是 JSC 相对 V8 的劣势场景。
   ⇒ 换 CEF 的收益 = **响应性差距收敛 + 一致性/兼容性**(bug 群消失、CDP、Chromium 行为对齐)。
2. **稳定性有可归因的已知爆点**:macOS 26(Tahoe)存在被多方独立复现的"高频 UI 更新触发 WKWebView 崩溃白屏"([Wails #4592](https://github.com/wailsapp/wails/issues/4592):同应用浏览器打开不崩、仅内嵌 WKWebView 崩、降频后停止)——dsh 流式长列表正中要害;且 wry 截至 0.56.x **没有针对性修复**(changelog 全查)。另有 WKWebView+React 受控组件与中文 IME 组合的长期已知问题(ChatGPT 桌面端、Obsidian 均有复现帖)。
3. **真正的痛点是 WKWebView 作为嵌入式组件的"共存税"**:焦点/输入法/事件分发/崩溃恢复的 bug 群,且每一个都要在 zap 侧用 ObjC hack 或注入 JS 绕(见 §3 清单)。这些 hack 是持续维护成本,而且部分无解(只能等 Apple 修)。
4. **换 CEF 工程上可行,但代价比直觉高**:包体 +100-400MB、内存常驻 +80MB 以上(比 WKWebView 的系统共享进程池高一个数量级)、签名/打包链路重做、IPC 桥重接,且**永久堵死 Mac App Store 渠道**(CEF 与 App Sandbox 不兼容)。生态上唯一成熟绑定是 tauri-apps/cef-rs(Chromium 152 分支,阶段 0 实装 152.4.0,持续活跃),但属单一社区维护、无知名产品背书。zap 的架构恰好把 wry 依赖收敛在一个 789 行的门面文件里,改造面比典型 Tauri 应用小;与 Metal 合成还有 OSR+IOSurface 共享纹理这条路可走。
5. **推荐路线:两段式。** 短期(天级)做方案 A(外部浏览器逃生门 + 有限的注入式止血;dsh 前端调优项归属上游,只记反馈清单);CEF 作为中期(周级)专项,按 §6 的验证步骤先做 spike 再立项——spike 必须先回答"放弃 MAS 渠道是否可接受"。**归属约束:dsh 是第三方产品,zap 只拥有承载层(wry/门面/IPC 桥),前端根因修复都在 dsh 上游手里——这使 zap 侧唯一能独立根治的路径是换 CEF。**

---

## 2. 现状架构(证据)

链路:`app/src/dsh/runtime.rs` 启动 `dsh --profile zap` 本地 Node 服务(随机端口+token)→ `app/src/browser/browser_web_view.rs:298` 经 **wry 0.56.1**(WKWebView 封装)建子 webview → `DshPane` 承载 dsh Web UI。

关键基建面(换内核时都要过一遍):

| 基建点 | 位置 | 与内核的耦合度 |
|---|---|---|
| Metal 挖洞(原生视图洞) | `crates/warpui_core/src/scene.rs:24`、`crates/warpui/src/platform/mac/rendering/metal/renderer.rs:396` | 低——只关心"洞的 rect",内核换成 CEF 的 NSView 一样适用 |
| 平台视图每帧上报 | `crates/warpui/src/platform/mac/window.rs:496`(`PlatformViewHandler`) | 低,同上 |
| webview 生命周期门面 | `app/src/browser/browser_web_view.rs`(约 25 个 pub fn:create/navigate/focus/set_bounds/destroy…) | **高**——wry 类型全部集中在此,是天然适配层 |
| ObjC 事件分发特判 | `crates/warpui/src/platform/mac/objc/window.m`(WKWebView 特判 12 处)、`host_view.m:281,428` | 高——但换 CEF 后大部分特判可删(CEF 是标准 NSView 行为,不需要 webview 分支) |
| 注入 JS(init_js,约 150 行) | `browser_web_view.rs:139-286` | 高——一半是 WebKit 专属绕虫(见 §3) |
| dsh IPC 桥 | `app/src/dsh/bridge.rs`(`zap.switch_project` 等 4 个方法) | 中——走 `webkit.messageHandlers.ipc`,CEF 需换 CDP/`executeJavaScript` 回传通道 |
| 输入/文本注入 | `insert_text_into_dsh_input`、`evaluate_script_on` | 低——纯 `evaluate_script`,任何内核都支持 |
| 下载管道 | `browser_web_view.rs:382`(WKDownloadDelegate + NSSavePanel) | 中——CEF 有自己的 `OnBeforeDownload` |

**有利因素**:wry 调用被门面封死在一个文件;`zap:` IPC 消息格式是自定义字符串协议,不绑死 WebKit;dsh 本身是标准 web 应用,在 Chrome 里本来就正常跑。

---

## 3. WebKit 共存税:实测踩过的 bug 清单(全部有提交/文档佐证)

近两个月 webview/dsh 相关提交 66 个,其中**兼容性/焦点/崩溃类 15 个**(2026-08-01 之后 `git log -- app/src/browser app/src/dsh crates/warpui/src/platform/mac/objc/window.m`)。代表性案例:

| # | 问题 | 本质 | zap 侧现状 |
|---|------|------|-----------|
| 1 | WebContent 渲染进程崩溃(含 macOS beta JSC JIT bug)→ 死页 | WebKit 多进程架构 + JIT 稳定性 | 崩溃检测 + 覆盖层 + 重载重走 token 认证(cb3f6f911、ae9a10ccf)——**只能兜底,不能根治** |
| 2 | 页面点击抢焦:重复 makeFirstResponder 让 WebKit 把聚焦元素 blur 到 body,弹层 onBlur 即关(切模型失败) | WebKit 焦点模型与 AppKit 交互的特有行为 | init_js 里 focusin/mousedown 只在 `!document.hasFocus()` 时上报 + 菜单 mousedown preventDefault(49533b402) |
| 3 | contenteditable 内方向键经 WebKit 编辑兜底路径插入 U+001D 方块 | WebKit 专属 bug,Chromium 无此问题(代码注释实测) | keydown preventDefault + `Selection.modify()`(WebKit 扩展 API)绕行(1ceb160ed) |
| 4 | webview 持久 cookie 在 127.0.0.1 上跨实例累积 → HTTP 431,bundle 全部加载失败 | WKWebView cookie 按 host 不按端口存储、且不淘汰 | incognito 数据存储根治(4f9efa15e);完整根因链见受管文档 aeb33a5c |
| 5 | Cmd+C/V/X/R 与快捷键分发:wry child webview performKeyEquivalent 返回 NO,编辑命令被丢弃 | AppKit responder chain 与 wry 子视图的组合缺陷 | window.m 手动转发 paste:/cut:/selectAll:/evaluateJavaScript(window.m:581-630);Cmd+R 在 init_js 手动处理 |
| 6 | webview 拿 first responder 后 Warp 收不到 mouseMoved,hover 全灭 | AppKit 按 responder chain 投递 mouseMoved | host_view.m 挂全区域 tracking area 兜底(host_view.m:281) |
| 7 | 中文输入法:closeIME 误清 webview 的 marked text | 输入上下文归属 | host_view.m:428 按 firstResponder 归属守卫 |
| 8 | 鼠标事件序列:webview 收不到 mouseUp,点击/拖选失效 | NSWindow sendEvent 重定向 | window.m:505 按命中目标分流 `[super sendEvent:]` |
| 9 | 嵌入 webview 上方浮层点击失效 | hitTest 坐标系 | dd2cdc47f(y 翻转入 scene 坐标) |
| 10 | 下载静默失败(wry 不配 handler 就无人应答) | wry 的 WKDownloadDelegate 挂载条件 | f618e1989 原生管道 + 保存面板;认证 cookie 依赖见受管文档 2e22d365 |

**定性**:1/3/5/9 这类属于"WebKit/Apple 侧 bug,zap 无法修只能绕",2/6/7/8 属于"嵌入式共存的固有摩擦,CEF 下大多自动消失"(CEF 的 NSView 行为与 Electron 一致,是业界验证过的嵌入形态)。

**「未验证推测」**:backdrop-filter/磨砂在 WKWebView 下的表现差异(此前在 DSH 桌面端 Electron 有 backdrop-filter bug 先例,但 zap 内嵌 WKWebView 下 dsh web 是否踩同类问题未逐项排查)。

---

## 4. 方案评估

### A. 留在 WebKit,调优 + 逃生门(短期,天级)

**归属约束(重要)**:dsh 是第三方产品(`@deepseek-ai/dsh`,npm 全局安装),其 web 前端不是 zap 仓库代码,zap **不能直接修改**。因此所有"前端调优"只有三条合法路径:zap 侧注入(init_js / dsh 客户端插件)、向上游反馈等修复、或绕开。按可操作性重排:

**zap 侧可直接做**:
1. **外部浏览器逃生门(第一杠杆)**:dsh URL 一键在外部 Chrome/Edge `--app=` 窗口打开(dsh 已是本地 HTTP 服务,外部浏览器需走一次 `GET /?token=` 自举——机制上可行,`GET /` 受理 token,见受管文档 2e22d365 §2)。Chrome 下 JSC 响应性差距、macOS 26 崩溃、焦点/IME 问题全部消失。代价:`zap:` IPC 桥的 4 个通知(switch_project/open_file/notify/…)在外部窗口失效或需另走通道。
2. **zap: IPC 桥的浏览器通道补齐**(若逃生门要常用):给 dsh 的客户端插件补一条不依赖 webview IPC 的回传通道(如 zap 内嵌 HTTP server 的本地回环端点,`crates/http_server` 已有基建),让外部浏览器窗口也能发 `zap.*` 消息。中等工作量,按需做。
3. **注入式降频补丁(受限)**:zap 已有 dsh 客户端插件注入机制(`zap-bridge-client` 经 profile patch),理论上可注入"节流/虚拟滚动"类补丁;但 dsh 是第三方且持续升级,**注入补丁每次上游更新都可能失效**(oh-my-dsh-ui 0.1.6 升级即断的先例),只适合作为临时止血,不做长期方案。
4. **升级 wry 到 0.56.x**(已是 0.56.1,#1719 自定义协议零拷贝已拿到);继续跟踪后续版本,但 wry 对 macOS 26 崩溃/焦点/IME 均无专项修复([changelog 全查](https://v2.tauri.app/release/wry/)),[wry Discussion #1014](https://github.com/tauri-apps/wry/discussions/1014) 确认 macOS 换内核没有路线图。
5. §3 里 zap 侧的绕行 hack 已基本做完(崩溃兜底、焦点交接、cookie 隔离等均已落地)。

**只能靠 dsh 上游(记录为反馈清单,不排期)**:
- React 侧降频 + 虚拟滚动(第一有效杠杆,同时压卡顿与 macOS 26 崩溃,[Wails #4592](https://github.com/wailsapp/wails/issues/4592) 的实证修法)
- backdrop-filter 层数审计(shadcn #327 实证的 CSS 绘制级卡顿)
- IME 受控组件 composition 分支(ChatGPT/Obsidian 桌面端同款问题)
- macOS 26 高频更新崩溃的官方修复

**收益/成本**:zap 侧能做的主要是逃生门,治标;前端根子在 dsh 上游手里。⇒ 方案 A 的天花板比初估更低,这**提高了 CEF spike 的相对价值**:换内核是 zap 侧唯一能独立完成、且能同时消掉性能差距+崩溃+共存税的路径。

### B. 换 CEF(Chromium)(中期,周级)

**可行性证据**(深度调研,2026-09-20):
- [tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs)(crates.io 名 `cef`,最新 `152.4.0+152.0.8`(阶段 0 实装版),对应 Chromium 152 分支):Tauri 组织下社区维护,macOS ARM64 支持,134 个版本持续迭代(2021 起),近期仍在更新。Hacker News 评价"可用、体验完整,只是用的人少"([HN](https://news.ycombinator.com/item?id=48627307))。但**未成为 wry/tauri 官方 backend**([wry Discussion #1014](https://github.com/tauri-apps/wry/discussions/1014) 维护者明确"尚未定论"),旧社区绑定已被判定 unmaintained 风险。⇒ zap 要直接基于 cef-rs 自建嵌入层,且承担**单一社区维护者的生态集中风险**。
- **同形态先例**:atrium(基于 Tauri 的 AI 浏览器)已把浏览器 pane 从 WKWebView 换成 CEF:CEF 视图放 wrapper NSView、主 webview `drawsBackground=NO` 挖洞、延迟初始化保冷启动——与 zap 的 Metal 挖洞架构同构([atrium 工程博客](https://getatrium.dev/blog/embedding-real-browser-tauri),该站本环境抓取受限,要点经搜索摘要交叉确认)。但**没有找到"Rust 自绘 UI + CEF"的知名 macOS 产品先例**,cef-rs 目前只有 demo 级实例(「未验证推测」:无大规模产品背书)。
- **对齐 Metal 的另一条路(比挖洞更彻底)**:CEF 离屏渲染(OSR)的 `OnAcceleratedPaint` 在 macOS 上以 **IOSurface** 抛出渲染结果,宿主可直接包成 MTLTexture 参与自绘合成([Apple MTLTexture 文档](https://developer.apple.com/documentation/metal/mtltexture))——这能消灭"挖洞/分层透明"整类问题。已知坑:CEF 内部纹理池缺同步机制,须在回调内等待完成、不能长期持有共享 texture([QCefView 作者实务讨论](https://scalibq.wordpress.com/2024/08/06/cef-developers))。「未验证推测」:cef-rs 上 OSR+sandbox 联用状态需 spike 实测。
- **zap 侧改造面**:§2 表格中"高耦合"三项(门面文件重写为 cef-rs 实现、ObjC 特判大幅删减、init_js 中 WebKit 绕虫删除 + IPC 通道重接)。dsh 桥的 `zap:` 字符串协议可原样保留,只换传输层。
- **代价(调研实测区间,替换此前的粗估)**:
  - 磁盘:CEF 分发解压后约 100-400MB(社区 CefDetector 汇总 ~100MB 是下限,多数应用 150-400MB;有裁剪到 ~29MB 的激进案例,[实测视频](https://www.youtube.com/watch?v=WuxaTPch12m)、[CefDetectorX](https://github.com/ShirasawaSama/CefDetectorX));精确数字以 spike 实测为准。
  - 内存:单浏览器进程基线 ~80MB,子进程各 30-60MB([IronPdf CEF 实测](https://ironpdf.com/troubleshooting/cef-chromium-memory-usage);子进程数字为社区经验值,「未验证」)。对比 WKWebView 系统共享进程池,**常驻成本高一个数量级**。
  - 打包:`.app` 内新增 framework + 至少 1 个 Helper.app bundle,全部签名+公证;放置位置与签名顺序敏感([Unreal 案例实测](https://pgaleone.eu/unrealengine/macos/2024/07/06/codesigning-notarization-issues))。
  - **硬约束:CEF 与 macOS App Sandbox 不兼容**(CEF 全局 Mach port IPC 与 sandbox 冲突,[Apple 论坛案例](https://developer.apple.com/forums/thread/807949))⇒ **Mac App Store 渠道不可行**,只能 Developer ID 直发。zap 当前走 selfsign Developer ID 形态(`script/macos/bundle --selfsign`),不受影响,但意味着未来上 MAS 的路被堵死——需产品侧确认可接受。
- **收益**:§3 全表 10 项 bug 群大概率消失(1/2/3/5/6/7/8/9 属 Chromium 无此类问题或行为不同);获得 CDP(调试/自动化能力升级);dsh web 与官方 DSH Desktop(Electron/Chromium)渲染一致,前端不再需要 WebKit 分支。

**风险**:cef-rs 单一社区维护、无完善测试套件(自述);CEF 版本升级节奏自己背;签名/分发链路回归(当前 bundle 流程强制签名校验,AGENTS.md §5.9);MAS 渠道永久放弃(见上)。

### C. 不内嵌,外部浏览器承载 dsh(快速缓解,天级)

dsh 是本地 web 服务,直接给 Chrome `--app=` 窗口。最快见效,但 `zap:` IPC 桥失效(项目切换、终端上下文、通知点击定位会话都断),窗口脱离 zap 的 tab/分屏管理。**适合作为 A 的逃生门,不适合做终态**——除非 dsh 侧提供原生通道(如 WebSocket 桥,历史上 zap 曾用过 WS 桥后迁回 webview IPC,见 bridge.rs 头注释)。

### D. Servo/Verso

**调研确认此路已断**:Verso(versotile-org/verso)2025-10-08 归档,versoview-release 同期归档,tauri-runtime-verso 从未上 crates.io;Servo 本体虽有 WebView API 且已上 crates.io,但官方自述"API 不稳定、尚不成熟",无生产级 Rust 应用以其为主 webview 的公开案例。Tauri 维护者称 Servo backend 实验"seems discontinued"([wry Discussion #1014](https://github.com/tauri-apps/wry/discussions/1014))。**不推荐,仅作远期观察项**。

---

## 5. 一个必须先做的对照实验(30 分钟)

在动任何方案之前,先量化"体验差"的归属:

1. 在 zap 内打开 dsh pane,记录主观卡顿场景(流式输出/长对话/滚动)。
2. 同一 dsh 实例的 URL(含 token)在 Chrome 里打开,重复同样操作。
3. 若 Chrome 里明显更顺 → 问题大头在 WebKit(JSC 响应性 + 渲染管线 + macOS 26 崩溃),CEF/逃生门收益大;若差不多 → 问题在 dsh web 前端本身,换内核收益有限,应向上游反馈前端问题(降频/虚拟滚动/backdrop-filter/IME)。
4. **检查系统版本**:macOS 26 上重点验证"高频更新崩溃白屏"是否复现(Wails #4592 同款);非 26 系统上白屏多属 WebContent 崩溃(已有兜底)。

同时可用 Speedometer 3 在 zap 内嵌 webview 的 devtools 里跑一次,与系统 Safari 分数对照(「未验证」,做完补数据)。

---

## 6. 建议决策路径

```
第 0 步(30 分钟):§5 对照实验 → 确认问题归属
第 1 步(1-2 天):方案 A + C 逃生门 → 压掉可绕项
第 2 步(2-3 天 spike):基于 tauri-apps/cef-rs(CEF 152 分支,tools/cef-spike 独立 crate)建最小 CEF 宿主,
  在 zap 窗口里以挖洞方式(或 OSR+OnAcceleratedPaint→IOSurface→MTLTexture)显示 dsh URL,验证:
  a) 包体增量实测数字  b) 签名后 bundle 可正常分发启动(selfsign 链路)
  c) Metal 挖洞/背景层与 CEF 共存(或 OSR 纹理合成路径跑通)
  d) zap: IPC 走通一条(实测为回环 HTTP shim 通道,见 §6.1)
  e) 内存对比(同场景 WKWebView vs CEF)
  f) 确认放弃 MAS 渠道对产品分发策略可接受(CEF 与 App Sandbox 不兼容)
第 3 步:spike 数据齐 → 决定是否立项全面迁移(预算:2-4 周,含回归)
```

 spike 期间正式版不受影响(用户日常使用的 /Applications/Zap.app 不动,遵循进程管理约定)。

### §6.1 阶段 0 spike 实测结果(2026-09-21,arm64 macOS 27.0,CEF 152.0.8 minimal)

实现:`tools/cef-spike/`(移植自 cef-rs dev 的 cefsimple 现成示例,独立 crate,不进 zap workspace)。

| 项 | 结果 | 实测数据 |
|---|------|---------|
| a 包体 | **超限** | bundle 322MB→**275MB**(locale 全量裁到中/英文系,84MB→34MB);引擎 dylib 独占 224MB 不可减。无路径接近 spec 目标 <150MB;现实下限 ≈ 260-280MB |
| b 签名 | ✅ | `codesign --force --deep -s -` + `--verify --deep --strict` 全绿,5 个 helper 子 app 结构完整 |
| c 裸窗共存 | ✅(部分) | CEF 窗体正常启动+渲染本地页(9 进程);**Metal 挖洞合并未测**(待阶段 1) |
| d IPC 通道 | ✅ | zap: 协议经本地回环 HTTP(spec 的 shim 设计)字节级往返:`zap.switch_project\n1\n{"path":"/tmp"}` 完整到达 |
| e 内存 | **超限** | 同一高频更新页面(私有足迹合计):CEF **~634MB**(browser 73.6 + renderer 365.5 + 其余 ~195)vs WKWebView **~456MB**(WebContent 306 + GPU 121 + 应用 29)→ CEF **+180MB(~+40%)**;dsh 式单页固定开销差 ~+120-180MB,页面渲染内存两者接近。注意口径:RSS 总和(跨进程重复计入 224MB libcef)会虚报 ~870MB,私有足迹才是真账面 |
| f 高频更新稳定性 | ✅ | 60Hz DOM 追加+scrollTo 连续 150s:**无崩溃**;renderer 私有内存被 GC 回落到 47.1MB(WKWebView 同场景峰值 367MB) |
| 弃用确认 | ✅ | single_process 全程未启用;site isolation 保持 CEF 默认(关) |

**spike 结论(评审 #2 校准口径):4/6 绿(b/d/f + 挖洞外 c),a/e 超限但幅度可入产品账;c(挖洞合并)按评审决策降级为阶段 1 第一项生死 criterion,未跑通不计入**:e 的净差只在"dsh pane 前台使用"窗口发生,不在常驻路径;代价 = 磁盘 +275MB + 前台内存 +180MB。是否放宽这两条目标继续阶段 1,待产品侧决策。

---

## 附:证据源

- 仓库内:`app/src/browser/browser_web_view.rs`(init_js 全文 §3-#2/#3/#5/下载)、`crates/warpui/src/platform/mac/objc/window.m:505-630`、`crates/warpui/src/platform/mac/objc/host_view.m:281,428`、`app/src/dsh/bridge.rs`、`app/src/dsh/pane.rs`
- 受管文档:`2e22d365`(下载管道/cookie 认证)、`aeb33a5c`(431 根因与排除链)
- 外部:[tauri-apps/cef-rs](https://github.com/tauri-apps/cef-rs) · [crates.io/crates/cef](https://crates.io/crates/cef) · [wry Discussion #1014](https://github.com/tauri-apps/wry/discussions/1014) · [wry #703 CEF fallback](https://github.com/tauri-apps/wry/issues/703) · [atrium 博客](https://getatrium.dev/blog/embedding-real-browser-tauri) · [Magic Lasso 跑分](https://supasidebar.com/blog/fastest-browser-mac-2026) · [BrowseRating](https://browserating.com) · [Tauri/CEF 播客](https://www.youtube.com/watch?v=6SO_hRDDGXs) · [HN: Tauri CEF 讨论](https://news.ycombinator.com/item?id=48627307) · [CEF 官方 usage 文档](https://chromiumembedded.github.io/cef/general_usage.html) · [CEF 纹理池同步坑](https://scalibq.wordpress.com/2024/08/06/cef-developers) · [IronPdf CEF 内存实测](https://ironpdf.com/troubleshooting/cef-chromium-memory-usage) · [CefDetectorX](https://github.com/ShirasawaSama/CefDetectorX) · [CEF 打包签名案例](https://pgaleone.eu/unrealengine/macos/2024/07/06/codesigning-notarization-issues) · [Apple 论坛:CEF×Sandbox 冲突](https://developer.apple.com/forums/thread/807949) · [Verso 归档生态纪要](https://github.com/GoldStrikeArch/rust-gui-desktop-ecosystem-state/blob/main/report/data/stack-rows.md) · [Blink on iOS +28.6% 实测](https://open-web-advocacy.org/blog/28-percent-faster--the-blink-prototype-that-shows-why-apples-ios-browser-engine-ban-must-end/) · [Edge Top Developer Needs](https://microsoftedge.github.io/TopDeveloperNeeds/) · [Wails #4592:macOS 26 高频更新崩溃](https://github.com/wailsapp/wails/issues/4592) · [shadcn-ui #327:backdrop-blur 卡顿](https://github.com/shadcn-ui/ui/issues/327) · [WebKit backdrop-filter 官方博客](https://webkit.org/blog/3632/introducing-backdrop-filters) · [ChatGPT 桌面端中文 IME 问题](https://community.openai.com/t/bug-in-macos-chatgpt-client-with-chinese-japanese-input-method/1039258) · [Craft.do WKWebView focus 实战](https://www.craft.do/blog/thinking-outside-of-the-wkwebview) · [wry changelog](https://v2.tauri.app/release/wry/)
