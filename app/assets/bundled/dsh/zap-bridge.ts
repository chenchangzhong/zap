// zap-bridge:Zap ⇄ dsh 桥插件(阶段 0)。
//
// 零 npm 依赖:仅用 Node 原生 `WebSocket` + `fetch`(Node >= 22.19)。
// 流程:握手(`bridge/hello`,携带 token + 协议版本 + 能力)→ 保持连接 →
// 断线指数退避重连 + 重新握手。
//
// 配置经 cordis.yml 的 `config:` 注入(`bridgeAddress` + `token`),由
// Zap 侧 runtime 启动 dsh 时生成。发布形态:随 Zap 打包(bundled asset),
// 运行时提取到 DSH_HOME,`--patch` 指向该文件。

export const name = 'zap-bridge'

const PROTOCOL_VERSION = 1
const DSH_VERSION = '0.1.0-rc.6'
/// 重连初始退避(ms)与上限。
const INITIAL_BACKOFF_MS = 1000
const MAX_BACKOFF_MS = 30000

export function apply(ctx: unknown, config: unknown): void {
  const c = config as { bridgeAddress?: string; token?: string }
  const address = c.bridgeAddress
  const token = c.token
  if (!address || !token) {
    console.error('[zap-bridge] missing bridgeAddress/token in config:', JSON.stringify(config))
    return
  }
  connect(address, token)
}

/// 建立一条 WS 连接;断线自动指数退避重连。
function connect(address: string, token: string): void {
  const ws = new WebSocket(address)
  let handshaken = false
  let backoff = INITIAL_BACKOFF_MS

  ws.onopen = () => {
    backoff = INITIAL_BACKOFF_MS
    // 握手:token + 协议版本 + 能力清单。
    ws.send(
      JSON.stringify({
        jsonrpc: '2.0',
        id: 1,
        method: 'bridge/hello',
        params: {
          token,
          protocolVersion: PROTOCOL_VERSION,
          dshVersion: DSH_VERSION,
          capabilities: [],
        },
      }),
    )
  }

  ws.onmessage = (event) => {
    const text = typeof event.data === 'string' ? event.data : ''
    if (!text) return
    try {
      const msg = JSON.parse(text) as {
        id?: number
        result?: { protocolVersion?: number; zapVersion?: string }
        error?: unknown
      }
      if (msg.id === 1 && msg.result) {
        handshaken = true
        console.log(
          '[zap-bridge] handshake ok (zap ' +
            (msg.result.zapVersion ?? '?') +
            ', protocol ' +
            (msg.result.protocolVersion ?? '?') +
            ')',
        )
        // 握手成功:发一次 zap.ping 验证往返。
        ws.send(JSON.stringify({ jsonrpc: '2.0', id: 2, method: 'zap.ping', params: {} }))
      } else if (msg.error) {
        console.error('[zap-bridge] rpc error:', JSON.stringify(msg.error))
        // 握手失败(token 错 / 版本不匹配):关闭触发重连。
        ws.close()
      }
    } catch {
      // 非 JSON 帧:忽略。
    }
  }

  ws.onclose = () => {
    console.log('[zap-bridge] connection closed, reconnecting in ' + backoff + 'ms')
    setTimeout(() => connect(address, token), backoff)
    backoff = Math.min(backoff * 2, MAX_BACKOFF_MS)
  }

  // onerror 后必随 onclose,统一走重连。
  ws.onerror = () => {}
}
