import { clone, newId, type DesignSystem, type Project } from './model'

export interface EmbeddedDesignAsset { id: string; name: string; mimeType: string; bytesBase64: string; metadata?: Record<string, unknown> }
export interface DesignLibraryDocument { format: 'hmi-design-library'; version: 1; libraryVersion: string; namespace: string; designSystem: Pick<DesignSystem, 'tokenSets' | 'themes' | 'components'> & { activeThemeId?: string }; assets: EmbeddedDesignAsset[] }

export function exportDesignLibrary(project: Project, options: { namespace?: string; libraryVersion?: string; assets?: EmbeddedDesignAsset[] } = {}): DesignLibraryDocument {
  const system = project.designSystem
  if (!system) throw new Error('Project has no design system')
  return { format: 'hmi-design-library', version: 1, libraryVersion: options.libraryVersion ?? '1.0.0', namespace: options.namespace ?? project.id, designSystem: clone({ tokenSets: system.tokenSets, themes: system.themes, components: system.components, activeThemeId: system.activeThemeId }), assets: clone(options.assets ?? []) }
}
export function parseDesignLibrary(json: string): DesignLibraryDocument {
  const value = JSON.parse(json) as DesignLibraryDocument
  if (value?.format !== 'hmi-design-library' || value.version !== 1 || typeof value.namespace !== 'string' || !value.designSystem || !value.designSystem.tokenSets || !value.designSystem.themes || !value.designSystem.components || !Array.isArray(value.assets)) throw new Error('Invalid design library document')
  value.libraryVersion ??= '1.0.0'
  if (typeof value.libraryVersion !== 'string' || !/^\d+\.\d+\.\d+(?:[-+][\w.-]+)?$/.test(value.libraryVersion)) throw new Error('Invalid design library version')
  for (const asset of value.assets) if (!asset.id || typeof asset.name !== 'string' || typeof asset.mimeType !== 'string' || typeof asset.bytesBase64 !== 'string' || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(asset.bytesBase64)) throw new Error(`Invalid embedded library asset: ${asset.id}`)
  return value
}
export function importDesignLibrary(project: Project, document: DesignLibraryDocument, options: { namespace?: string } = {}): { namespace: string; tokenSetIds: Record<string, string>; componentIds: Record<string, string>; themeIds: Record<string, string>; assetIds: Record<string, string>; assets: EmbeddedDesignAsset[] } {
  const system: DesignSystem = project.designSystem ?? (project.designSystem = { version: 1, tokenSets: {}, themes: {}, components: {}, libraries: {} })
  const preferred = options.namespace ?? document.namespace
  const occupied = (candidate: string) => system.libraries[candidate] || Object.keys(system.tokenSets).some(id => id.startsWith(`${candidate}:`)) || Object.keys(system.themes).some(id => id.startsWith(`${candidate}:`)) || Object.keys(system.components).some(id => id.startsWith(`${candidate}:`))
  let namespace = preferred, suffix = 2
  while (occupied(namespace)) namespace = `${preferred}-${suffix++}`
  const sets: Record<string, string> = {}, components: Record<string, string> = {}, themes: Record<string, string> = {}
  for (const id of Object.keys(document.designSystem.tokenSets)) sets[id] = `${namespace}:${id}`
  for (const id of Object.keys(document.designSystem.components)) components[id] = `${namespace}:${id}`
  for (const id of Object.keys(document.designSystem.themes)) themes[id] = `${namespace}:${id}`
  for (const [oldId, oldSet] of Object.entries(document.designSystem.tokenSets)) {
    const set = clone(oldSet); set.id = sets[oldId]!; set.name = `${namespace} / ${set.name}`
    for (const token of Object.values(set.tokens)) if (typeof token.$value === 'string') token.$value = token.$value.replace(/^\{([^{}.]+)\.([^{}]+)\}$/, (_m, setId: string, tokenName: string) => `{${sets[setId] ?? setId}.${tokenName}}`)
    system.tokenSets[set.id] = set
  }
  for (const [oldId, oldTheme] of Object.entries(document.designSystem.themes)) { const theme = clone(oldTheme); theme.id = themes[oldId]!; theme.name = `${namespace} / ${theme.name}`; theme.tokenSets = theme.tokenSets.map(id => sets[id] ?? id); system.themes[theme.id] = theme }
  const assetIds = Object.fromEntries(document.assets.map(asset => [asset.id, `${namespace}:${asset.id}`]))
  const rewriteAssetIds = (value: unknown): unknown => {
    if (typeof value === 'string') return assetIds[value] ?? value
    if (Array.isArray(value)) return value.map(rewriteAssetIds)
    if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, child]) => [key, rewriteAssetIds(child)]))
    return value
  }
  for (const [oldId, oldComponent] of Object.entries(document.designSystem.components)) {
    const component = clone(oldComponent); component.id = components[oldId]!; component.name = `${namespace} / ${component.name}`
    for (const widget of component.widgets) {
      widget.props = rewriteAssetIds(widget.props) as Record<string, any>
      if (widget.customProperties) widget.customProperties = rewriteAssetIds(widget.customProperties) as typeof widget.customProperties
      if (widget.componentInstance?.overrides) widget.componentInstance.overrides = rewriteAssetIds(widget.componentInstance.overrides) as typeof widget.componentInstance.overrides
      if (widget.styleTokens) widget.styleTokens = Object.fromEntries(Object.entries(widget.styleTokens).map(([property, ref]) => {
        const qualifiedSetId = Object.keys(sets).filter(id => ref.startsWith(`${id}.`) && document.designSystem.tokenSets[id]?.tokens[ref.slice(id.length + 1)]).sort((a, b) => b.length - a.length)[0]
        if (qualifiedSetId) return [property, `${sets[qualifiedSetId]}.${ref.slice(qualifiedSetId.length + 1)}`]
        const sourceTheme = document.designSystem.themes[document.designSystem.activeThemeId ?? '']
        const availableSetIds = sourceTheme?.tokenSets ?? Object.keys(document.designSystem.tokenSets).sort()
        const sourceSetId = [...availableSetIds].reverse().find(setId => document.designSystem.tokenSets[setId]?.tokens[ref])
        return [property, sourceSetId ? `${sets[sourceSetId] ?? sourceSetId}.${ref}` : ref]
      }))
      if (widget.componentInstance) widget.componentInstance.componentId = components[widget.componentInstance.componentId] ?? widget.componentInstance.componentId
    }
    if (component.variants) component.variants = rewriteAssetIds(component.variants) as typeof component.variants
    system.components[component.id] = component
  }
  system.libraries[namespace] = { id: namespace, name: namespace, version: document.libraryVersion, namespace, assetIds: Object.values(assetIds) }
  return { tokenSetIds: sets, componentIds: components, themeIds: themes, assetIds, namespace, assets: document.assets.map(asset => ({ ...clone(asset), id: assetIds[asset.id]! })) }
}
export function deleteDesignComponent(project: Project, componentId: string): void {
  const system = project.designSystem, component = system?.components[componentId]
  if (!system || !component) throw new Error(`Unknown design component: ${componentId}`)
  const pageReference = project.pages.some(page => page.widgets.some(widget => widget.componentInstance?.componentId === componentId))
  const componentReference = Object.values(system.components).some(item => item.id !== componentId && item.widgets.some(widget => widget.componentInstance?.componentId === componentId))
  if (pageReference || componentReference) throw new Error(`Cannot delete component ${componentId} while instances reference it`)
  delete system.components[componentId]
}
export function deleteDesignLibrary(project: Project, namespace: string): string[] {
  const system = project.designSystem, library = system?.libraries[namespace]
  if (!system || !library) throw new Error(`Unknown design library: ${namespace}`)
  const componentIds = Object.keys(system.components).filter(id => id.startsWith(`${namespace}:`))
  const componentIdSet = new Set(componentIds), assetIds = new Set(library.assetIds ?? [])
  const tokenSetIds = new Set(Object.keys(system.tokenSets).filter(id => id.startsWith(`${namespace}:`)))
  const referencesTokenSet = (tokens: Record<string, { $value: unknown }>) => Object.values(tokens).some(token => {
    if (typeof token.$value !== 'string') return false
    const match = token.$value.match(/^\{([^{}]+)\}$/)
    if (!match || !match[1]!.includes('.')) return false
    const referencedSet = match[1]!.slice(0, match[1]!.lastIndexOf('.'))
    return tokenSetIds.has(referencedSet)
  })
  const referenced = project.pages.some(page => page.widgets.some(widget => componentIdSet.has(widget.componentInstance?.componentId ?? '') || containsAsset(widget, assetIds) || Object.values(widget.styleTokens ?? {}).some(ref => [...tokenSetIds].some(id => ref.startsWith(`${id}.`))))) || Object.values(system.components).some(component => !componentIdSet.has(component.id) && (containsAsset(component.variants, assetIds) || component.widgets.some(widget => componentIdSet.has(widget.componentInstance?.componentId ?? '') || containsAsset(widget, assetIds) || Object.values(widget.styleTokens ?? {}).some(ref => [...tokenSetIds].some(id => ref.startsWith(`${id}.`)))))) || Object.values(system.themes).some(theme => !theme.id.startsWith(`${namespace}:`) && theme.tokenSets.some(id => tokenSetIds.has(id))) || Object.entries(system.tokenSets).some(([id, set]) => !tokenSetIds.has(id) && referencesTokenSet(set.tokens))
  if (referenced) throw new Error(`Cannot delete design library ${namespace} while components or assets are in use`)
  for (const id of componentIds) delete system.components[id]
  for (const id of Object.keys(system.tokenSets)) if (id.startsWith(`${namespace}:`)) delete system.tokenSets[id]
  for (const id of Object.keys(system.themes)) if (id.startsWith(`${namespace}:`)) delete system.themes[id]
  if (system.activeThemeId?.startsWith(`${namespace}:`)) delete system.activeThemeId
  delete system.libraries[namespace]
  return [...assetIds]
}
function containsAsset(value: unknown, ids: Set<string>): boolean {
  if (typeof value === 'string') return ids.has(value)
  if (Array.isArray(value)) return value.some(item => containsAsset(item, ids))
  return !!value && typeof value === 'object' && Object.values(value).some(item => containsAsset(item, ids))
}
export function serializeDesignLibrary(document: DesignLibraryDocument): string { return JSON.stringify(document, null, 2) }
