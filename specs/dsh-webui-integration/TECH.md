# 计划:dsh WebUI 融入 Zap(第一步)

## 背景与目标

[deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)(dsh)是 DeepSeek 开源的插件化 agent harness(MIT,一切皆插件,Cordis)。Zap 已有嵌入式 WebView 基建(`app/src/browser/`)。本计划:将 dsh Web UI 作为 Zap 一等功能融入 —— UI = dsh 官方 Web UI(渲染在 Zap WebView),引擎 = dsh runtime(Node 子进程,随面板启停)。后续功能(目录同步、项目文件等)以插件方式增量集成,本计划仅预留通道。

## 范围

| 属于本计划 | 不属于本计划(路线图) |
|---|---|
| dsh runtime 自动安装/启动/停止/崩溃重启 | workspace 目录同步插件 |
| WebView 渲染 dsh Web UI | 项目文件/文件树注入插件 |
| 命令面板/快捷键入口 | 终端上下文注入插件 |
| 会话持久(跨面板关闭/重启) | 事件映射/自研 UI 渲染 |
| 首次引导(模型 API key 配置) | webview JS 注入自定义 UI |

## 现状(已核实)

- `app/src/browser/browser_web_view.rs`:BrowserWebViewManager 单例,create/navigate/set_bounds/destroy,platform-view 每帧上报 rect,IPC 处理 focusin/URL 变更(双光标、地址栏同步已修复)。macOS 先行,非 macOS 空壳。
- `app/src/browser/browser_pane_view.rs`:BrowserPaneView(地址栏 + PlatformViewElement),BrowserPane( PaneContent)。
- `crates/node_runtime`:`install_npm` / `find_working_node_binary` / `node_installation_dir`,已入 workspace **零使用点**(天然落点)。
- dsh 官方分发:`npx @deepseek-ai/dsh web`(需 Node ≥22);另有单文件 exe 打包先例(Python SDK 用,无需 Node)。

## 调研结论(步骤 1 落地,2026-08-15 实测)

| 项 | 结论 | 来源 |
|---|---|---|
| `dsh web --port 0` | **支持**,OS 分配空闲端口(实测 63815/61085 等) | `dsh web --help` + 实测 |
| `--host` | 可绑,默认 127.0.0.1(无鉴权,仅本机) | `dsh web --help` |
| DSH_HOME 环境变量 | **生效**:`profiles/`(配置)+ `storages/`(数据,600 权限) | 实测 |
| 就绪判据 | **无 stdout banner**(输出缓冲),用**端口监听 + HTTP 200** | 实测 |
| 优雅退出 | SIGTERM 进程树干净退出,无残留 | 实测 |
| npm 包入口 | `@deepseek-ai/dsh` 的 CLI 入口是 `node_modules/@deepseek-ai/dsh/lib/bin.js`(非 apps/cli/dist) | 实测 |
| npm install 落盘 | npm 11 下 `--prefix` 不落盘 node_modules,需**在目标目录内执行** | 实测 |
| Node 要求 | engines `^22.19 \|\| >=24`(实测系统 v24.13.0 可用) | 包 engines |
| 版本锁定 | `0.1.0-rc.6`(npx 最新,源码 rc.5) | npx 实测 |


## 验收标准

1. 冷启动(无 Node、无 dsh):命令触发 → 自动装 Node + dsh → 面板可交互,总耗时 ≤60s(含安装;已安装 ≤5s)。
2. 渲染:dsh UI 完整可用(对话、工具卡片、审批、设置),无白屏。
3. 生命周期:关面板 → dsh 退出;重开 → 新进程、会话还在;Zap 退出无残留进程。
4. 崩溃恢复:kill dsh 进程 → 面板 3s 内自动重启恢复。
5. 端口:动态选择,无冲突,关闭即释放。
6. 数据隔离:DSH_HOME 在 Zap 数据目录下,不污染用户 `~/.dsh`。

## 技术方案

### 新模块 `app/src/dsh/`

**1. `runtime.rs` — DshRuntime 单例(model,Entity)**

- 安装:优先 `node_runtime::find_working_node_binary`;无则 `node_runtime::install_npm`(已实现)。
- 定位 dsh:先查 Zap 数据目录缓存(`~/.warp/dsh/node_modules`),无则 `npm install @deepseek-ai/dsh`(或 `npx`),版本锁 pin。
- 启动:`dsh web --port 0`(若支持)或动态选空闲端口;DSH_HOME 指向 `~/.warp/dsh`;注入 `ZAP_BRIDGE_ADDRESS`(预留,当前可为空)。**子进程一律走 `crates/command`**。
- 就绪检测:HTTP probe `GET /` 或 log 匹配,超时上报失败。
- 停止:面板关闭/SIGTERM → 优雅退出 → 超时 kill;Drop 兜底。
- 崩溃:进程 exit 监听 → 事件 `DshRuntimeEvent::Exited(restart: bool)`。

**2. `pane.rs` — DshPane / DshPaneView**

- 复用 `BrowserWebViewManager` 渲染 webview,URL 锁定 `http://127.0.0.1:<port>`,隐藏地址栏(内部 URL,无导航需求),显示连接状态(启动中/已断开/就绪)。
- 复用既有 focusin/URL 同步 IPC 机制(双光标修复直接适用)。

**3. 入口**

- 命令面板命令 "Open DeepSeek Harness"(复用现有 pane 打开机制)+ 快捷键(与 browser pane 同类)。

**4. 首次引导**

- 检测 dsh 无凭据配置(HTTP API 或文件探测)→ 面板内提示 + 直接打开 dsh 设置页(web UI 自带)。

### 复用与不重复造轮子

- WebView 渲染、焦点、IPC:**全复用**,不新写平台代码。
- Node 安装:**全复用** node_runtime。
- dsh 安装/版本管理:新写(少量,npm 命令封装 + 目录探测)。
- 进程生命周期:新写(基于 crates/command + 就绪探测 + exit 监听)。

### 风险与对策

| 风险 | 对策 |
|---|---|
| dsh developer preview,API/CLI 漂移 | 锁版本 pin;`dsh web` 启动参数封装在单文件(runtime.rs),升级只改一处 |
| `dsh web` 无 `--port 0` 支持 | 动态选端口(绑定 0 探测),封装在 runtime.rs |
| 安装耗时(首次) | 面板加载态 + 进度提示;后台预安装(可选) |
| dsh web 无鉴权 | 仅绑 127.0.0.1(默认),webview 只访问 localhost;不暴露网络 |
| 退出残留 | SIGTERM → 超时 kill → 退出钩子兜底;验收标准 3 强制验证 |

## 实施步骤

1. 调研确认:`dsh web` 实际 CLI 参数(端口、DSH_HOME 环境变量名)、`~/.warp/dsh` 布局;验证 `node_runtime` 可用性。→ 验收:本地手动跑通 `dsh web`。
2. `runtime.rs`:安装/定位/启动/就绪/停止/崩溃监听。→ 验收:单元测试 + 手动起停。
3. `pane.rs` + 入口:面板打开 → webview 加载 → 状态显示。→ 验收:验收标准 2。
4. 生命周期收尾:关闭/退出/崩溃重启。→ 验收:验收标准 3、4。
5. 首次引导 + 数据隔离收尾。→ 验收:验收标准 5、6。
6. 总验收:全部 6 条验收标准。

## 路线图(后续,另行立项)

1. bridge 通道:注入 `ZAP_BRIDGE_ADDRESS`,dsh 侧 `zap-bridge` 插件握手(每功能 = Zap 服务 + dsh 插件 + 通道)。
2. workspace 同步:Zap 活动终端/项目目录 → dsh workspace。
3. 项目文件注入:Zap 文件树/repo 元数据 → dsh 工具(`zap_*`)。
4. 终端上下文注入;按需 webview JS 轻量 UI 动作(评估 DOM 脆弱性后)。
