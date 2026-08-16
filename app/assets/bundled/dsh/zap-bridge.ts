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

/// 建立一条 WS 连接;断线自动指数退避重连。
function connect(address: string, token: string): void {
  const sock = new WebSocket(address)
  ws = sock
  let handshaken = false
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
        handshaken = true
        console.log('[zap-bridge] handshake ok')
      } else if (msg.error) {
        console.error('[zap-bridge] rpc error:', JSON.stringify(msg.error))
        // 握手失败(token 错 / 版本不匹配):关闭触发重连。
        if (msg.id === 1) {
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
    console.log('[zap-bridge] connection closed, reconnecting in ' + backoff + 'ms')
    setTimeout(() => connect(address, token), backoff)
    backoff = Math.min(backoff * 2, MAX_BACKOFF_MS)
  }

  // onerror 后必随 onclose,统一走重连。
  sock.onerror = () => {}
}
