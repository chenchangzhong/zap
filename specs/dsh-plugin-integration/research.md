# 调研:dsh 插件 API(阶段 0/2 依据)

> 来源:https://deepseek-harness.github.io/deepseek-harness/develop/basic/(第一个插件)、
> /develop/basic/tool(开发一个工具)、/develop/basic/config(插件配置)
> 日期:2026-08-16。状态:已确认。

## 1. 插件是什么

插件 = 导出 `apply` 函数的 TypeScript 模块。框架加载时调用 `apply`,传入 `ctx`(Cordis Context),通过 `ctx` 注册能力。

```ts
import type { Context } from '@deepseek-ai/cordis'

export const name = 'zap-bridge'

export function apply(ctx: Context) {
  // Register capabilities here.
}
```

三种形态:函数形式(多数情况够用)、对象形式(`{ name, inject, apply }`)、类形式(继承 `Service`,向其他插件提供服务时用)。

## 2. 注册到 cordis.yml(patch 机制)

本地插件通过 cordis.yml 覆盖层注入,启动参数:

```sh
dsh web --patch <cordis.yml>
```

```yaml
- insert:
    - id: zap-bridge
      name: '/absolute/path/to/zap-bridge.ts'   # 必须绝对路径
      config:
        bridgeAddress: 'http://127.0.0.1:12345'
```

- **插件路径必须绝对路径**;patch 只贡献配置,不改变 loader 解析模块路径用的 profile 目录。
- `config:` 注入插件配置,经导出的 Schema 校验并填充默认值。
- 配置变更触发 HMR:卸载旧实例、加载新实例;注册均为 effect,替换后无残留。

## 3. 依赖注入

```ts
export const inject = ['tools']   // 等 tools 服务就绪后才加载本插件
```

`apply` 运行时,声明的依赖必已就绪。

## 4. 工具注册 DSL(阶段 2 用)

```ts
import { defineTool } from '@deepseek-ai/dsh-tools'

ctx.tools.register(defineTool({
  name: 'zap_list_files',
  description: '...',
  parameters: {
    path: { type: 'string', required: true, description: '...' },
  },
  output: {
    schema: { type: 'string' },
    render: (_args, value) => [{ type: 'text', text: value }],
  },
  async execute(args) {
    return /* 调用 Zap bridge */ ''
  },
}))
```

- `parameters` 用 Schemastery schema:推导 args 类型 + 运行时校验。
- `execute` 返回 `output.schema` 声明的规范值;`output.render` 转面向模型的内容。
- 更多能力(嵌套 schema、后台工作、策略钩子、UI 卡片):`/reference/cookbook/adding-a-tool`(阶段 2 细读)。

## 5. 自动清理

ctx 注册的一切(事件、工具、定时器)卸载时自动清理;手动资源用 `ctx.effect()`。

## 6. 对阶段 0 设计的影响

| 设计点 | 结论 |
|---|---|
| zap-bridge 插件形态 | 函数形式;阶段 2 加 `inject: ['tools']` |
| 桥地址传递 | 走 `cordis.yml` 的 `config:`(比 env `ZAP_BRIDGE_ADDRESS` 正规,Schema 校验+默认值);runtime.rs 启动时生成 cordis.yml |
| 插件文件位置 | 绝对路径指向 Zap 安装目录内(随 Zap 分发);runtime.rs 生成 patch 时写绝对路径 |
| 握手 | `apply` 内 `fetch(config.bridgeAddress + '/bridge/hello')`;插件加载成功 = dsh 侧就绪 |
| 网络 | 插件跑在 dsh 的 Node 进程内,`fetch` 可用,无 CORS;127.0.0.1 仅本机 |
| 断线重连 | apply 内周期探测 + dsh 重启自动重新握手(Cordis 生命周期) |
| 配置变更 | HMR 热替换天然支持;Zap 重启改端口后,若 runtime 重启 dsh 进程则无需 HMR |

## 7. 传输层选型(阶段 0 调研结论,2026-08-16 已定 WebSocket)

**目标**:桥双向可靠——dsh→Zap 调用, Zap→dsh 推送(workspace/终端上下文/UI 消息),实时、可演进、无返工。

**候选实测**:

| 方案 | dsh 侧(Node 24) | Zap 侧 | 双向 | 结论 |
|---|---|---|---|---|
| JSON-RPC over HTTP + 轮询 | `fetch` ✅ | axum 现成(http_server) | 推送靠轮询,秒级延迟 | 单向,阶段 1 返工 |
| HTTP + SSE | `EventSource` **undefined** ❌ | axum | 推送实时 | SSE 在 Node 24 不可靠,否决 |
| **WebSocket 单连接** | `WebSocket` 原生全局 ✅ | `async_tungstenite` 已依赖,自写 bind/accept/分发 glue | ✅ 真双向实时 | **采用** |

**已定方案(WebSocket)**:
- 传输:WS 单长连接,`127.0.0.1:<随机端口>`;Zap 侧 `async_tungstenite` 起 server(复用依赖树,随机端口),dsh 插件 Node 原生 `WebSocket` 客户端
- 协议:JSON-RPC 2.0 over WS,方法命名空间 `zap.*`
- 握手:首消息 `bridge/hello` 交换协议版本 + 能力清单 + token 校验
- 双向:请求-响应(dsh→Zap 调用)+ Zap→dsh 事件推送(复用同连接)
- 安全:127.0.0.1 + 随机 token(runtime 生成,注入 cordis.yml `config:`,握手校验)
- 保活/重连:WS ping/pong + 断线自动重连 + 重新握手;Zap 侧记录连接状态供面板 UI 显示
- Zap 侧 WS server 无现成先例(无 axum ws / accept_async 用法),需新写桥服务模块;axum 0.8.4 在 workspace,`async_tungstenite` 已在依赖树

## 8. 风险/待验证

- `dsh web --patch` 在 npm 分发版(`npx @deepseek-ai/dsh web`)的支持:用户确认支持(源码 `pnpm dsh web --patch` 已证实);实现时若发现差异,备选方案为 DSH_HOME profiles 下写配置。
- cordis.yml 生成时机:dsh 每次启动前生成(端口、桥地址每次可能变化)。
- 插件包依赖(`@deepseek-ai/cordis`、`@deepseek-ai/dsh-tools`、`@deepseek-ai/schemastery`):随 dsh 安装的 node_modules 或随 zap-bridge 打包,需在实现阶段确认 loader 解析路径。
