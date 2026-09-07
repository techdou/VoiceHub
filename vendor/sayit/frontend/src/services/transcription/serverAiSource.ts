import { getSetting } from '../store'

export type ServerAiSource = 'managed' | 'custom'
export const SERVER_AI_SOURCE_KEY = 'server.aiSource'

let runtimeSource: ServerAiSource = 'managed'

function normalizeServerAiSource(value: unknown): ServerAiSource {
  return value === 'custom' ? 'custom' : 'managed'
}

/** 从持久化设置刷新运行时值。Provider connect 时调用，保证冷启动与导入配置后正确。 */
export async function loadServerAiSource(): Promise<ServerAiSource> {
  runtimeSource = normalizeServerAiSource(await getSetting(SERVER_AI_SOURCE_KEY, 'managed'))
  return runtimeSource
}

/** start() 必须同步读取这一份，不能依赖上一次 connect 留下的快照。 */
export function getRuntimeServerAiSource(): ServerAiSource {
  return runtimeSource
}

/** 设置页保存时同步更新，避免同一个 ServerProvider 实例继续按旧来源发送下一次请求。 */
export function setRuntimeServerAiSource(value: ServerAiSource): void {
  runtimeSource = value
}
