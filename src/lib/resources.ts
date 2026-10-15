import { ref } from 'vue'
import { convertFileSrc } from '@tauri-apps/api/core'
import { call, desktop } from './backend'
import type { PluginManifest, Project } from './model'
export const pluginCatalog = ref<PluginManifest[]>([{ id: 'demo-temperature', name: '示例温度协议', version: '1.0.0', commands: [{ name: 'read', label: '读取温度', params: { type: 'object', properties: {} } }, { name: 'setTarget', label: '设置目标温度', params: { type: 'object', properties: { target: { type: 'number', title: '目标温度' } }, required: ['target'] } }], points: [{ key: 'temperature', label: '温度', type: 'number' }, { key: 'rawTemperature', label: '原始温度', type: 'number' }, { key: 'target', label: '目标温度', type: 'number' }] }])
export const assetUrls = ref<Record<string, string>>({})
export const assetFonts = ref<Record<string, string>>({})
const loadedFonts = new Map<string, FontFace>()
export async function refreshAssets(project: Project) {
  if (!desktop) return
  const paths = await call<Record<string, string>>('resolve_assets', { project })
  assetUrls.value = Object.fromEntries(Object.entries(paths).map(([id,path]) => [id, convertFileSrc(path)]))
  assetFonts.value = {}
  for (const asset of project.assets ?? []) {
    if (!/\.(ttf|otf|woff2?)$/i.test(asset.path) || !assetUrls.value[asset.id]) continue
    const family = `HMI-${asset.id}`
    const prior = loadedFonts.get(asset.id)
    if (prior) document.fonts.delete(prior)
    const font = new FontFace(family, `url("${assetUrls.value[asset.id]}")`)
    await font.load(); document.fonts.add(font); loadedFonts.set(asset.id, font); assetFonts.value[asset.id] = family
  }
}
export function dataPoints(project: Project) {
  return project.connections.flatMap(connection => {
    const points = pluginCatalog.value.find(plugin => plugin.id === connection.protocolId)?.points
    return (Array.isArray(points) ? points : Object.entries(points ?? {}).map(([key, value]) => ({ key, ...(value as object) }))).map(point => ({ key: `${connection.id}.${point.key}`, label: `${connection.name} / ${point.label || point.key}` }))
  })
}
