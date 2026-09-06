// zap-bridge:Zap ⇄ dsh 桥插件。
//
// 零 npm 依赖:不用 `import`(插件文件位于 DSH_HOME,Node 无法解析外部包),
// 仅用 Node 原生 `WebSocket` 与 `ctx.tools.register`(运行时注入)。工具定义
// 以 JSON Schema 格式手动构造(等价 `defineTool` 转换结果)。
//
// 职责:
// - 握手(`bridge/hello`,token + 协议版本 + 能力)→ 保持连接 → 断线指数退避重连
// - 注册 `zap_*` 工具,`execute` 经桥(WS JSON-RPC)调用 Zap 的文件能力
// - `rpc` 请求-响应映射(id → resolve)
//
// 配置经 cordis.yml `config:` 注入(`bridgeAddress` + `token`),由 Zap 侧
// runtime 启动 dsh 时生成。

export const name = 'zap-bridge'
/// 声明依赖 tools 服务(框架保证就绪后才加载本插件)。
export const inject = ['tools']

const PROTOCOL_VERSION = 1
const DSH_VERSION = '0.1.0-rc.6'
const INITIAL_BACKOFF_MS = 1000
const MAX_BACKOFF_MS = 30000
/// 工具 rpc 起始 id(握手占用 id=1,避免冲突)。
const RPC_ID_START = 100

let ws: WebSocket | null = null
let nextRpcId = RPC_ID_START
const pending = new Map<number, (v: unknown) => void>()
/// 定时器句柄类型(浏览器与 Node 的 setTimeout 返回类型不同)。
type TimerHandle = ReturnType<typeof setTimeout>
/// 重连定时器句柄(卸载时清理)。
let reconnectTimer: TimerHandle | null = null
/// 连续握手失败次数(超限后放弃自动重连,避免 token/协议永久不匹配时
/// 无限循环刷日志)。
let handshakeFailures = 0
/// 已卸载(dispose/`ctx.effect` 清理)标记:onclose 据此不再调度重连。
let disposed = false
const MAX_HANDSHAKE_FAILURES = 5

/// 工具注册所需的最小 tools 接口(避免依赖完整 ctx 类型推断)。
interface ToolsLike {
  register(d: unknown): () => void
}

export function apply(ctx: unknown, config: unknown): void {
  const c = config as { bridgeAddress?: string; token?: string }
  const address = c.bridgeAddress
  const token = c.token
  if (!address || !token) {
    console.error('[zap-bridge] missing bridgeAddress/token in config:', JSON.stringify(config))
    return
  }
  // 卸载/HMR 时清理:关 socket + 清重连定时器,避免旧实例残留连接与僵尸
  // 重连链(配置变更触发 HMR 时旧实例会继续反复握手)。
  const effect = (ctx as { effect?: (fn: () => void) => void }).effect
  effect?.(() => cleanup())
  // 新配置实例 = 重新尝试的意图:重置握手失败计数(HMR 复用模块状态时,
  // 否则 token 修复后 handshakeFailures 卡在 MAX 永不重连)。
  handshakeFailures = 0
  connect(address, token)
  registerTools(ctx)
}

/// 从 ctx 提取 tools 服务(运行时守卫,避免内联断言)。
function extractTools(ctx: unknown): ToolsLike | undefined {
  if (ctx && typeof ctx === 'object' && 'tools' in ctx) {
    // 'tools' in ctx 已守卫存在性;cast 到最小接口。
    return (ctx as { tools: ToolsLike }).tools
  }
  return undefined
}

/// 手动构造一个工具定义(JSON Schema 格式,等价 defineTool 的转换结果)。
function zapTool(
  name: string,
  description: string,
  method: string,
  parameters: unknown,
): unknown {
  return {
    name,
    description,
    parameters,
    output: {
      schema: { type: 'string' },
      render: (_args: unknown, value: unknown) => [
        { type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value) },
      ],
    },
    async execute(args: unknown) {
      // output.schema 要求 string;桥返回对象,JSON 序列化后返回。
      return JSON.stringify(await rpc(method, args))
    },
  }
}

/// 手动构造一个无参数工具定义(JSON Schema 格式)。
function zapToolNoArgs(name: string, description: string, method: string): unknown {
  return {
    name,
    description,
    parameters: { type: 'object', properties: {} },
    output: {
      schema: { type: 'string' },
      render: (_args: unknown, value: unknown) => [
        { type: 'text', text: typeof value === 'string' ? value : JSON.stringify(value) },
      ],
    },
    async execute(args: unknown) {
      return JSON.stringify(await rpc(method, args))
    },
  }
}

/// 注册 Zap 文件能力工具(execute 经桥调用 Zap)。
function registerTools(ctx: unknown): void {
  const tools = extractTools(ctx)
  if (!tools) {
    console.error('[zap-bridge] tools service unavailable')
    return
  }
  const pathParam = {
    type: 'object',
    properties: {
      path: { type: 'string', description: 'Path relative to the project root.' },
    },
    required: ['path'],
  }
  const patternParam = {
    type: 'object',
    properties: {
      pattern: { type: 'string', description: 'Filename substring to search.' },
    },
    required: ['pattern'],
  }

  tools.register(
    zapTool(
      'zap_list_files',
      'List entries (files and directories) in the current Zap project. ' +
        'Paths are relative to the project root; use empty string for the root.',
      'zap.list_files',
      pathParam,
    ),
  )
  tools.register(
    zapTool(
      'zap_read_file',
      'Read a file in the current Zap project as UTF-8 text.',
      'zap.read_file',
      pathParam,
    ),
  )
  tools.register(
    zapTool(
      'zap_search',
      'Search for files in the current Zap project by filename substring.',
      'zap.search',
      patternParam,
    ),
  )
  tools.register(
    zapToolNoArgs(
      'zap_terminal_context',
      'Return recent commands from the current Zap terminal. ' +
        'Only available when the Zap "terminal context" privacy setting is enabled.',
      'zap.terminal_context',
    ),
  )
  console.log(
    '[zap-bridge] registered zap_list_files, zap_read_file, zap_search, zap_terminal_context',
  )
}

/// 经桥发起一次 JSON-RPC 请求并等待响应。
function rpc(method: string, params: unknown): Promise<unknown> {
  const { promise, resolve } = Promise.withResolvers<unknown>()
  const sock = ws
  if (!sock || sock.readyState !== WebSocket.OPEN) {
    resolve({ error: 'zap bridge not connected' })
    return promise
  }
  const id = nextRpcId++
  pending.set(id, resolve)
  sock.send(JSON.stringify({ jsonrpc: '2.0', id, method, params }))
  return promise
}

/// 清理连接与重连定时器(插件卸载/`ctx.effect` 时调用)。
function cleanup(): void {
  disposed = true
  if (reconnectTimer) {
    clearTimeout(reconnectTimer)
    reconnectTimer = null
  }
  if (ws) {
    // 先置空 onclose,避免 close() 再触发重连。
    ws.onclose = null
    ws.close()
    ws = null
  }
  for (const resolve of pending.values()) {
    resolve({ error: 'zap bridge unloaded' })
  }
  pending.clear()
}

/// 调度一次重连(记录句柄供卸载时清理)。
function scheduleReconnect(address: string, token: string, backoff: number): void {
  reconnectTimer = setTimeout(() => {
    reconnectTimer = null
    connect(address, token)
  }, backoff)
}

/// 建立一条 WS 连接;断线自动指数退避重连(握手永久失败或已卸载则停止)。
function connect(address: string, token: string): void {
  // 新连接尝试:清除卸载标记(保留 handshakeFailures,使失败计数在未成功
  // 前跨重连累积;握手成功时归零)。
  disposed = false
  const sock = new WebSocket(address)
  ws = sock
  let backoff = INITIAL_BACKOFF_MS

  sock.onopen = () => {
    backoff = INITIAL_BACKOFF_MS
    sock.send(
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'bridge/hello',
        params: {
          token,
          protocolVersion: PROTOCOL_VERSION,
          dshVersion: DSH_VERSION,
          capabilities: ['files.list', 'files.read'],
        },
      }),
    )
  }

  sock.onmessage = (event) => {
    const text = typeof event.data === 'string' ? event.data : ''
    if (!text) return
    try {
      const msg = JSON.parse(text) as { id?: number; result?: unknown; error?: unknown }
      if (msg.id === 1 && msg.result) {
        // 握手成功:重置失败计数,之后可再次重试。
        handshakeFailures = 0
        console.log('[zap-bridge] handshake ok')
      } else if (msg.error) {
        console.error('[zap-bridge] rpc error:', JSON.stringify(msg.error))
        // 握手失败(token 错 / 版本不匹配):计次;超限后不再重连,避免
        // token/协议永久不匹配时无限循环刷日志。
        if (msg.id === 1) {
          handshakeFailures++
          if (handshakeFailures >= MAX_HANDSHAKE_FAILURES) {
            console.error(
              '[zap-bridge] handshake failed ' + handshakeFailures +
                ' times; giving up auto-reconnect (permanent failure)',
            )
          }
          sock.close()
        } else if (msg.id !== undefined && pending.has(msg.id)) {
          const resolve = pending.get(msg.id)
          pending.delete(msg.id)
          resolve?.({ error: msg.error })
        }
      } else if (msg.id !== undefined && pending.has(msg.id)) {
        const resolve = pending.get(msg.id)
        pending.delete(msg.id)
        resolve?.(msg.result)
      }
    } catch {
      // 非 JSON 帧:忽略。
    }
  }

  sock.onclose = () => {
    ws = null
    // 拒绝挂起的请求。
    for (const resolve of pending.values()) {
      resolve({ error: 'zap bridge disconnected' })
    }
    pending.clear()
    // 已卸载或握手永久失败:不再自动重连。
    if (disposed || handshakeFailures >= MAX_HANDSHAKE_FAILURES) {
      console.log('[zap-bridge] connection closed (disposed or permanent failure; no reconnect)')
      return
    }
    console.log('[zap-bridge] connection closed, reconnecting in ' + backoff + 'ms')
    scheduleReconnect(address, token, backoff)
    backoff = Math.min(backoff * 2, MAX_BACKOFF_MS)
  }

  // onerror 后必随 onclose,统一走重连。
  sock.onerror = () => {}
}
