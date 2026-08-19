// zap-bridge-client — 宿主半(空实现)。
// 浏览器端逻辑在 client.js(经 dsh.client 声明 + exports["./client"] 加载)。
// 宿主半仅需存在(loader 要 import 包入口 index.js)。
export const name = 'zap-bridge-client'

export function apply() {
  // 无宿主端逻辑。浏览器端插件经 dsh.client 由 client-modules 加载。
}
