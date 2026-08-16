# 阶段 0 桥通道设计:TECH-bridge.md

> 前置:specs/dsh-plugin-integration/TECH.md(总览)、research.md(调研结论,传输层已定 WS)。
> 本文件:阶段 0(桥通道)的详细技术设计,供评审与实现。范围仅限阶段 0;
> 方法/事件/数据契约在后续阶段填充,但**协议框架现在定对,后续不返工**。

## 1. 目标与原则

建立 Zap ⇄ dsh 的一条可信双向通道,承载后续所有插件能力。以"完美集成"终态为
准绳:通道要能演进到 workspace 跟随、文件工具、终端上下文、UI 动作,不因阶段 0
的最小实现而在阶段 1+ 返工。

**原则**:
- 传输:WebSocket 单长连接(已调研定案),双向、实时、可重连
- 协议:JSON-RPC 2.0 over WS,方法命名空间 `zap.*`,版本化 + 能力协商 + token 校验
- 生命周期:桥随 `DshRuntime` 启停;连接状态进面板 UI
- 配置:Zap 为单一事实源,cordis.yml 由 runtime 生成

## 2. 架构

```
┌───────────────────── Zap 进程(Rust) ─────────────────────┐
│  DshRuntime(单例,已有)                                    │
│    ├─ 生成 cordis.yml(bridgeAddress + token + 插件绝对路径)│
│    ├─ 启动 dsh web --patch cordis.yml --port 0            │
│    └─ BridgeServer(新,async_tungstenite)                  │
│         bind 127.0.0.1:0(随机端口)                        │
│         accept → 每连接 token 校验 → 握手 → 请求/事件分发   │
│         BridgeEvent::Connected/Disconnected/… → 面板 UI    │
└──────────────────────────┬───────────────────────────────┘
                           │ WebSocket(JSON-RPC 2.0)
┌──────────────────────────┴───────────────────────────────┐
│  dsh 进程(Node) — cordis.yml 注入 zap-bridge 插件          │
│    apply(ctx) → new WebSocket(bridgeAddress)              │
│    → bridge/hello(token, dshVersion, capabilities)        │
│    → 断线重连 + 重新握手                                   │
└──────────────────────────────────────────────────────────┘
```

## 3. 传输与协议规范

### 3.1 WS 端点与生命周期

- Zap 侧 `BridgeServer`:`async_tungstenite::accept_async` 起 server,`bind 127.0.0.1:0`(随机端口,关闭即释放)
- 生命周期:随 `DshRuntime`。启动 dsh 前起 server(先得端口+token 才能写 cordis.yml);dsh 停止时关闭
- dsh 侧:插件内 Node 原生 `WebSocket(bridgeAddress)`

### 3.2 消息格式(JSON-RPC 2.0)

所有消息 JSON 文本帧。沿用 JSON-RPC 2.0 语义:

```json
// 请求(dsh→Zap 调用)
{"jsonrpc":"2.0","id":1,"method":"zap.ping","params":{"seq":42}}
// 响应
{"jsonrpc":"2.0","id":1,"result":{...}}
// 错误
{"jsonrpc":"2.0","id":1,"error":{"code":-32601,"message":"method not found"}}
// 通知(dsh→Zap 无响应,Zap→dsh 推送)
{"jsonrpc":"2.0","method":"workspace.changed","params":{...}}
```

- 请求/响应用 `id` 关联;通知无 `id`、无响应
- `params` 一律对象(位置参数禁止,可演进)

### 3.3 方法清单(命名空间 `zap.*`)

| 方法 | 方向 | 阶段 | 说明 |
|---|---|---|---|
| `bridge/hello` | dsh→Zap(请求) | 0 | 握手:token 校验 + 能力交换(见 3.4) |
| `zap.ping` | dsh→Zap(请求) | 0 | 探活,返回 `{dshPing:true}`;验收往返 <100ms |
| `zap.get_workspace` | dsh→Zap | 1 | 返回当前 Zap 项目目录(预留) |
| `zap.list_files` / `zap.read_file` / `zap.search` / `zap.open_in_editor` | dsh→Zap | 2 | 文件能力(预留) |
| `zap.terminal_context` | dsh→Zap | 3 | 当前终端上下文(预留) |
| `workspace.changed` | Zap→dsh(通知) | 1 | 项目/终端 cwd 变化推送(预留) |
| `files.changed` | Zap→dsh(通知) | 2 | 文件树刷新(预留) |
| `terminal.context` | Zap→dsh(通知) | 3 | 终端上下文更新(预留) |

> 阶段 0 只实现 `bridge/hello` + `zap.ping`;其余方法/事件在对应阶段填充,协议框架(JSON-RPC + 命名空间 + 双向通知)现在定死。

### 3.4 握手时序(核心)

```
dsh 插件                        BridgeServer(Zap)
  │  new WebSocket(url)            │  accept → 记录连接(未握手)
  │────────────────────────────────▶
  │  bridge/hello                  │
  │  { token, dshVersion,          │
  │    capabilities:[...] }        │
  │                                │  校验 token
  │                                │  - 无效 → 返回错误 + 关闭连接
  │                                │  记录 dsh 版本/能力 → BridgeEvent::Connected
  │◀──────────────────────────────── 响应 result:
  │  { zapVersion, protocolVersion,│
  │    capabilities:[...] }        │
  │  握手完成,进入就绪             │
```

- **协议版本**:`protocolVersion` 整数递增;不匹配 → 返回明确错误(可协商降级),不静默
- **能力协商**:dsh 上报它要消费的 `zap.*` 能力;Zap 回它提供的。阶段 2 据此注册工具,阶段 1+ 据此定推送
- **token**:Zap 启动 dsh 前生成随机 token(≥32 字节,`rand`),写进 cordis.yml `config:`;握手携带,校验失败即断。防其他本地进程越权
- **握手超时**:连接后 5s 内未完成握手 → 关闭(防半开连接)

### 3.5 错误码

| 码 | 含义 |
|---|---|
| -32700 | 解析错误 |
| -32600 | 无效请求 |
| -32601 | 方法不存在 |
| -32602 | 无效参数 |
| -32603 | 内部错误 |
| 1001 | token 无效 / 未认证 |
| 1002 | 协议版本不匹配 |
| 1003 | 能力不支持 |
| 1000x | zap.* 业务错误(路径/权限等,阶段 2+ 扩展) |

### 3.6 心跳 / 超时 / 重连

- **心跳**:应用层 `zap.ping`(dsh→Zap)兜底;Zap 侧超时(如 30s 无消息)视为断线 → `BridgeEvent::Disconnected`
- **重连**:dsh 插件检测断线(close/error)→ 指数退避重连(1s,2s,4s… 上限 30s)→ 重新握手。Zap 重启后新端口 → dsh 重连时从新 cordis.yml 拿新地址(runtime 重启 dsh 场景)或插件内可配置
- **Zap 侧**:连接断开 → 状态 `Disconnected`;dsh 崩溃重启(runtime 自动重启)→ 插件重新加载 → 自动握手(计划一已有重启机制,桥状态随之刷新)

### 3.7 数据契约(提前定路径语义)

- **路径一律 Zap 项目根相对路径**,由 dsh 侧 `zap.*` 工具返回时携带项目根上下文;Zap 侧内部转换绝对路径。避免绝对路径跨机器/转义问题(阶段 1/2 一致)
- 文件树语义复用 `repo_metadata`(含 gitignore,阶段 2)

## 4. Zap 侧模块设计(`app/src/dsh/bridge.rs`)

`DshRuntime` 内新增桥子模块(或独立文件)。职责:

```
DshBridge(主线程持有状态)
  ├─ 状态:token, port, conn_state(Down|Connecting|Ready|Error)
  ├─ listener 任务(async_tungstenite accept 循环)
  ├─ 每连接:握手校验 → 分发 JSON-RPC 请求到 handler 注册表
  ├─ 事件分发:Zap→dsh 通知(经已握手连接 Sink 发出)
  └─ 对外事件:BridgeEvent::{Connected{dshVersion,capabilities},
                Disconnected, HandshakeFailed{reason}}
```

- **单例/挂载**:桥服务作为 `DshRuntime` 的一部分;或独立 `BridgeServer` Entity,由 DshPane 消费 `BridgeEvent` 显示连接状态
- **并发模型**:沿用 runtime 的 `'static` 异步任务 + `ModelContext::spawn` 回调回主线程;连接句柄(握手后的 Sink/Stream)由主线程持有,推送经 channel 发往 Sink
- **随机端口**:`TcpListener::bind(127.0.0.1:0)` → `local_addr()`.port()
- **token**:`rand` 生成(确认 `rand` 已在 workspace;否则加依赖),≥32 字节

**启动时序(runtime.rs 集成)**:
1. `start()`:起 BridgeServer → 得 `{port, token}`
2. 生成 cordis.yml(见 §5.2)→ 写 DSH_HOME 下临时目录
3. 注入 env + 启动 `dsh web --patch <cordis.yml> --port 0`
4. 就绪探测(已有,HTTP GET /)+ 等待握手(桥 Ready)

**停止时序**:关闭 listener → 关闭连接 → dsh 停止(已有)。无残留。

## 5. dsh 侧插件设计(`zap-bridge`)

### 5.1 插件文件

随 Zap 分发(绝对路径由 runtime 写入 cordis.yml)。结构:

```ts
import type { Context } from '@deepseek-ai/cordis'
export const name = 'zap-bridge'
export interface Config {
  bridgeAddress: string
  token: string
}
export const Config = /* Schemastery schema: 两者 required */
export function apply(ctx: Context, config: Config) {
  // new WebSocket(config.bridgeAddress) → 握手 → 断线重连
  // ctx.effect(清理: close socket, clear timers)
}
```

- 依赖:Node 原生 `WebSocket` + `fetch`,**零 npm 依赖**(避免 loader 解析路径问题)
- 阶段 2:`export const inject = ['tools']` + `ctx.tools.register(defineTool(zap_*))`,经桥调用
- 清理:`ctx.effect(() => () => { ws.close(); clearInterval(timer) })`

### 5.2 cordis.yml(runtime.rs 生成)

```yaml
- insert:
    - id: zap-bridge
      name: '/Users/…/zap/resources/dsh/zap-bridge.ts'   # 绝对路径
      config:
        bridgeAddress: 'ws://127.0.0.1:5xxxx'
        token: '<随机 token>'
```

- 每次启动重新生成(端口/token 每次变化);插件路径固定(随安装)
- 插件文件放置:随 Zap 安装目录(如 `resources/dsh/zap-bridge.ts` 或打包后 assets),`bundled!` 宏(已有 asset 基建)或直接文件路径

### 5.3 握手 / 重连(TS)

```
apply: connect → send bridge/hello → 等 result → 置 ready
on close/error: 指数退避重连(1s..30s) → 重新握手
fetch 封装: 后续 zap.* 调用复用同一连接(请求 id 关联)
```

## 6. 验收标准(阶段 0)

1. 面板打开 → dsh 加载 zap-bridge → 桥握手成功,Zap 侧可见 `Connected`(含 dsh 版本/能力)
2. dsh 侧调 `zap.ping`,往返 <100ms
3. 假 token 握手被拒,连接关闭,状态 `HandshakeFailed`
4. kill dsh 进程 → runtime 自动重启 → 插件重载 → 自动重新握手(无需重启 Zap)
5. 关面板 → listener 关闭、连接关闭、无残留端口/进程
6. 随机端口无冲突;token 非空且每次启动不同
7. 单测:JSON-RPC 解析、错误码映射、握手校验、token 生成(不依赖真实 dsh)

## 7. 风险与对策

| 风险 | 对策 |
|---|---|
| `dsh web --patch` 在 npm 分发版行为未实测 | 用户已确认支持;实现首步在 worktree 手动跑通 `npx dsh web --patch`,输出样本再开发 |
| cordis.yml 里 TS 插件绝对路径 / loader 解析 | 阶段 0 spike 验证:插件能否被 `--patch` 加载;若需打包为 npm 包则调整 §5.2 |
| 随机 token 需 `rand` 依赖 | 确认 workspace 依赖;无则加(微小) |
| WS server 无现成先例,glue 新写 | 复用 async_tungstenite;桥独立模块,不影响现有 websocket crate(客户端) |
| 断线重连 dsh 侧实现复杂 | 阶段 0 只做"重连 + 重新握手";推送/订阅阶段 1+ |

## 8. 实施步骤(阶段 0)

1. **spike**:worktree 内手动 `npx dsh web --patch` + 极简 TS 插件(`console.log` + `new WebSocket`),确认插件加载与 WS 连通 → 输出样本
2. Zap 侧 `bridge.rs`:BridgeServer(accept/握手/token/分发/事件)→ 单测(解析/错误码/握手)
3. runtime.rs 集成:起桥 → 生成 cordis.yml → 启动 dsh 注入 → 就绪+握手
4. dsh 侧 `zap-bridge.ts`:握手 + ping + 重连
5. 验收:§6 全过

## 9. 决策记录(已定)

| # | 项 | 决定 |
|---|---|---|
| 1 | 桥挂载 | **独立 `BridgeServer` Entity**(非 DshRuntime 内嵌),状态/事件清晰,DshPane 消费 `BridgeEvent` |
| 2 | 插件文件放置 | **磁盘文件路径为运行时形态**;dev 指向源码,发布用 bundled 嵌入 + 首次运行提取到 DSH_HOME 数据目录;runtime 统一读磁盘文件写绝对路径进 cordis.yml |
| 3 | `rand` 依赖 | workspace 已有 `rand = "0.8.6"`(`Cargo.toml:234`),直接复用,无需新增 |
| 4 | 插件打包形态 | **裸 TS 文件**,零 npm 依赖(`--patch` 直指源 ts);`zap-bridge.ts` 用 Node 原生 `WebSocket`/`fetch`,不依赖 loader 解析 npm 包路径 |
