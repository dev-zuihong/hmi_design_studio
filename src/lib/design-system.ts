import { clone, newId, type DesignComponent, type DesignToken, type DesignTokenType, type Project, type Widget } from './model'

const visualProperties = new Set(['color', 'background', 'backgroundColor', 'borderColor', 'fontFamily', 'fontSize', 'fontWeight', 'lineHeight', 'letterSpacing', 'opacity', 'borderRadius', 'radius', 'boxShadow', 'shadow', 'typography', 'padding', 'gap', 'width', 'height'])
const tokenTypeChecks: Record<DesignTokenType, (value: unknown) => boolean> = {
  color: v => typeof v === 'string' || !!v && typeof v === 'object' && Array.isArray((v as any).components) && typeof (v as any).colorSpace === 'string', dimension: v => typeof v === 'number' || typeof v === 'string' || !!v && typeof v === 'object' && typeof (v as any).value === 'number' && typeof (v as any).unit === 'string', number: v => typeof v === 'number' && Number.isFinite(v), boolean: v => typeof v === 'boolean',
  typography: v => !!v && typeof v === 'object', shadow: v => typeof v === 'string' || Array.isArray(v) || !!v && typeof v === 'object', fontFamily: v => typeof v === 'string' || Array.isArray(v), opacity: v => typeof v === 'number' && Number.isFinite(v) && v >= 0 && v <= 1, radius: v => typeof v === 'number' || typeof v === 'string',
}
function flattenTokens(project: Project, themeId?: string): Map<string, DesignToken> {
  const system = project.designSystem
  const result = new Map<string, DesignToken>()
  if (!system) return result
  const theme = themeId ? system.themes[themeId] : system.themes[system.activeThemeId ?? '']
  if (themeId && !theme) throw new Error(`Unknown design theme: ${themeId}`)
  if (!themeId && system.activeThemeId && !theme) throw new Error(`Unknown active design theme: ${system.activeThemeId}`)
  const sets = Object.keys(system.tokenSets).sort()
  for (const setId of sets) {
    const set = system.tokenSets[setId]
    if (!set) throw new Error(`Unknown token set: ${setId}`)
    for (const [name, token] of Object.entries(set.tokens)) result.set(`${setId}.${name}`, token)
  }
  return result
}
export function resolveDesignTokens(project: Project, themeId?: string): Record<string, unknown> {
  if (!project.designSystem) return {}
  const source = flattenTokens(project, themeId), values = new Map<string, unknown>(), active = new Set<string>()
  const resolve = (key: string): unknown => {
    if (values.has(key)) return values.get(key)
    const token = source.get(key)
    if (!token) throw new Error(`Unresolved design token alias: ${key}`)
    if (active.has(key)) throw new Error(`Design token alias cycle: ${[...active, key].join(' -> ')}`)
    active.add(key)
    let value = token.$value
    if (typeof value === 'string') {
      const match = value.match(/^\{([^{}]+)\}$/)
      if (match) {
        let reference = match[1]!
        const setId = key.slice(0, key.indexOf('.'))
        if (source.has(`${setId}.${reference}`)) reference = `${setId}.${reference}`
        const target = source.get(reference)
        if (!target) throw new Error(`Unresolved design token alias: ${key} -> ${reference}`)
        if (target.$type !== token.$type) throw new Error(`Design token alias type mismatch: ${key} -> ${reference}`)
        value = resolve(reference)
      }
    }
    if (!tokenTypeChecks[token.$type](value)) throw new Error(`Invalid ${token.$type} token value: ${key}`)
    active.delete(key); values.set(key, value); return value
  }
  const output: Record<string, unknown> = {}
  for (const key of source.keys()) output[key] = resolve(key)
  const system = project.designSystem!
  const theme = themeId ? system.themes[themeId] : system.themes[system.activeThemeId ?? '']
  const visibleSets = theme?.tokenSets ?? Object.keys(system.tokenSets).sort()
  for (const setId of visibleSets) for (const name of Object.keys(system.tokenSets[setId]!.tokens)) output[name] = resolve(`${setId}.${name}`)
  return output
}
export function validateDesignSystem(project: Project): void {
  const system = project.designSystem
  if (!system) return
  if (system.version !== 1 || !system.tokenSets || !system.themes || !system.components || !system.libraries) throw new Error('Invalid design system structure')
  for (const theme of Object.values(system.themes)) for (const setId of theme.tokenSets) if (!system.tokenSets[setId]) throw new Error(`Theme ${theme.id} references missing token set ${setId}`)
  for (const themeId of Object.keys(system.themes)) resolveDesignTokens(project, themeId)
  if (!Object.keys(system.themes).length) resolveDesignTokens(project)
  for (const component of Object.values(system.components)) {
    if (!component.id || !component.widgets.some(widget => widget.id === component.rootWidgetId)) throw new Error(`Invalid component ${component.id}: missing root widget`)
    const ids = new Set(component.widgets.map(widget => widget.id))
    if (ids.size !== component.widgets.length || component.widgets.some(widget => widget.parentId && !ids.has(widget.parentId))) throw new Error(`Invalid widget tree in component ${component.id}`)
    for (const widget of component.widgets) {
      const seen = new Set<string>([widget.id]); let parentId = widget.parentId
      while (parentId) {
        if (seen.has(parentId)) throw new Error(`Invalid widget tree in component ${component.id}: parent cycle`)
        seen.add(parentId); parentId = component.widgets.find(item => item.id === parentId)?.parentId
      }
      const nestedId = widget.componentInstance?.componentId
      if (nestedId && !system.components[nestedId]) throw new Error(`Component ${component.id} references missing nested component ${nestedId}`)
    }
  }
  const visiting = new Set<string>(), visited = new Set<string>()
  const visitComponent = (id: string) => {
    if (visiting.has(id)) throw new Error(`Nested component reference cycle: ${[...visiting, id].join(' -> ')}`)
    if (visited.has(id)) return
    visiting.add(id)
    for (const widget of system.components[id]!.widgets) if (widget.componentInstance?.componentId) visitComponent(widget.componentInstance.componentId)
    visiting.delete(id); visited.add(id)
  }
  for (const id of Object.keys(system.components)) visitComponent(id)
  for (const page of project.pages) for (const widget of page.widgets) if (widget.componentInstance) {
    const metadata = widget.componentInstance, component = system.components[metadata.componentId]
    if (!component || metadata.widgetIdMap[component.rootWidgetId] !== widget.id || component.widgets.some(source => !metadata.widgetIdMap[source.id] || !page.widgets.some(item => item.id === metadata.widgetIdMap[source.id]))) throw new Error(`Invalid component instance metadata on ${widget.id}`)
    assertVariant(component, metadata.variant)
  }
  const activeThemeId = system.activeThemeId
  for (const page of project.pages) for (const widget of page.widgets) resolveWidgetStyle(project, widget, activeThemeId)
}
export function resolveWidgetStyle(project: Project, widget: Widget, themeId?: string): Record<string, unknown> {
  const tokens = resolveDesignTokens(project, themeId), result: Record<string, unknown> = {}, entries = Object.entries(widget.styleTokens ?? {})
  const resolved = entries.map(([property, reference]) => {
    const key = reference
    if (!(key in tokens)) throw new Error(`Unknown style token: ${reference}`)
    return { property, value: tokens[key], type: tokenTypeForReference(project, reference, themeId) }
  })
  for (const entry of resolved.filter(item => item.type === 'typography')) {
    if (!visualProperties.has(entry.property)) continue
    if (!entry.value || typeof entry.value !== 'object') throw new Error(`Invalid typography token: ${entry.property}`)
    const spec = entry.value as Record<string, unknown>
    for (const [field, output] of [['fontFamily', 'fontFamily'], ['fontSize', 'fontSize'], ['fontWeight', 'fontWeight'], ['lineHeight', 'lineHeight'], ['letterSpacing', 'letterSpacing']] as const) {
      if (spec[field] !== undefined && visualProperties.has(output) && !Object.hasOwn(widget.bindings, output)) result[output] = normalizeStyleValue(spec[field], field === 'fontSize' || field === 'letterSpacing' ? 'dimension' : undefined)
  }
  }
  for (const { property, value, type } of resolved.filter(item => item.type !== 'typography')) if (visualProperties.has(property) && !Object.hasOwn(widget.bindings, property)) result[property] = normalizeStyleValue(value, type)
  return result
}
function tokenTypeForReference(project: Project, ref: string, themeId?: string): DesignTokenType | undefined {
  const entries = Object.entries(project.designSystem?.tokenSets ?? {})
  const qualified = entries.filter(([id, set]) => ref.startsWith(`${id}.`) && set.tokens[ref.slice(id.length + 1)]).sort(([a], [b]) => b.length - a.length)[0]
  if (qualified) return qualified[1].tokens[ref.slice(qualified[0].length + 1)]?.$type
  const theme = project.designSystem?.themes[themeId ?? project.designSystem?.activeThemeId ?? '']
  const fallbackSetIds = theme?.tokenSets ?? entries.map(([id]) => id).sort()
  const set = fallbackSetIds.map(id => project.designSystem?.tokenSets[id]).filter((value): value is NonNullable<typeof value> => !!value).reverse().find(value => value.tokens[ref])
  return set?.tokens[ref]?.$type
}
function normalizeStyleValue(value: unknown, type?: DesignTokenType): unknown {
  if (typeof value === 'string' && (type === 'dimension' || type === 'radius')) {
    const match = value.match(/^(-?(?:\d+(?:\.\d+)?|\.\d+))(px|rem)$/)
    if (match) return Number(match[1]) * (match[2] === 'rem' ? 16 : 1)
  }
  if (!value || typeof value !== 'object') return value
  const item = value as Record<string, any>
  if (typeof item.value === 'number' && typeof item.unit === 'string') {
    if (item.unit === 'px') return item.value
    if (item.unit === 'rem') return item.value * 16
    throw new Error(`Unsupported dimension unit for visual style: ${item.unit}`)
  }
  if (Array.isArray(item.components) && typeof item.colorSpace === 'string') {
    if (item.colorSpace !== 'srgb') throw new Error(`Unsupported DTCG color space for CSS visual style: ${item.colorSpace}`)
    const channels = item.components.map((channel: number) => Math.round(Math.max(0, Math.min(1, channel)) * 255))
    return item.alpha == null || item.alpha === 1 ? `rgb(${channels.join(' ')})` : `rgb(${channels.join(' ')} / ${item.alpha})`
  }
  if (type === 'shadow' && Array.isArray(value)) return itemToShadow(value)
  if (type === 'shadow' && ('offsetX' in item || 'offsetY' in item)) return itemToShadow([value])
  if (type === 'dimension' || type === 'radius') throw new Error('Unsupported visual dimension token value')
  return value
}
function itemToShadow(shadows: Record<string, any>[]): string {
  return shadows.map(shadow => {
    const dim = (entry: any) => { if (typeof entry === 'number') return `${entry}px`; if (typeof entry?.value === 'number' && ['px', 'rem'].includes(entry.unit)) return `${entry.value * (entry.unit === 'rem' ? 16 : 1)}px`; throw new Error('Unsupported DTCG shadow dimension') }
    const color = normalizeStyleValue(shadow.color, 'color')
    return `${dim(shadow.offsetX)} ${dim(shadow.offsetY)} ${dim(shadow.blur)}${shadow.spread === undefined ? '' : ` ${dim(shadow.spread)}`} ${color}`
  }).join(', ')
}
function descendants(widgets: Widget[], ids: string[]): Widget[] {
  const included = new Set(ids); let changed = true
  while (changed) { changed = false; for (const widget of widgets) if (widget.parentId && included.has(widget.parentId) && !included.has(widget.id)) { included.add(widget.id); changed = true } }
  return widgets.filter(widget => included.has(widget.id))
}
export function createComponentFromSelection(page: { widgets: Widget[] }, ids: string[], options: { id?: string; name?: string } = {}): DesignComponent {
  const selected = page.widgets.filter(widget => ids.includes(widget.id))
  if (!selected.length) throw new Error('Select at least one widget')
  const roots = selected.filter(widget => !widget.parentId || !ids.includes(widget.parentId))
  if (roots.length !== 1) throw new Error('A component must have exactly one selected root widget')
  const widgets = descendants(page.widgets, [roots[0]!.id]).map(clone)
  const componentRoot = widgets.find(widget => widget.id === roots[0]!.id)!
  delete componentRoot.parentId
  delete componentRoot.slotId
  return { id: options.id ?? newId('component'), name: options.name ?? roots[0]!.name, rootWidgetId: roots[0]!.id, widgets }
}
function assertVariant(component: DesignComponent, variant?: Record<string, string>) {
  if (!component.variants || !variant || !Object.keys(variant).length) return
  const axes = component.variants.axes
  for (const [axis, value] of Object.entries(variant)) if (!axes[axis]?.includes(value)) throw new Error(`Unknown component variant ${axis}=${value}`)
  const valid = Object.values(component.variants.combinations).some(combo => Object.keys(axes).every(axis => combo[axis] === variant[axis]))
  if (!valid) throw new Error('Unsupported component variant combination')
}
function applyVariantValues(component: DesignComponent, variant: Record<string, string> | undefined, sourceId: string, target: Widget) {
  if (!variant || !component.variants) return
  const comboId = Object.keys(component.variants.combinations).find(id => Object.keys(component.variants!.axes).every(axis => component.variants!.combinations[id]?.[axis] === variant[axis]))
  for (const [property, value] of Object.entries(component.variants.values?.[comboId ?? '']?.[sourceId] ?? {})) if (Object.hasOwn(target.props, property)) target.props[property] = clone(value)
}
function remapWidgetReferences(widget: Widget, ids: Record<string, string>) {
  for (const ref of Object.values(widget.bindings)) if ('key' in ref && ids[ref.key]) ref.key = ids[ref.key]
  for (const property of Object.values(widget.customProperties ?? {})) if (property.binding && 'key' in property.binding && ids[property.binding.key]) property.binding.key = ids[property.binding.key]
  if (widget.componentInstance) {
    widget.componentInstance.rootWidgetId = ids[widget.componentInstance.rootWidgetId] ?? widget.componentInstance.rootWidgetId
    widget.componentInstance.widgetIdMap = Object.fromEntries(Object.entries(widget.componentInstance.widgetIdMap).map(([key, value]) => [key, ids[value] ?? value]))
    widget.componentInstance.instanceId = newId('instance')
  }
}
/** Remap a cloned expanded widget while keeping component source IDs intact. */
export function remapClonedWidget(widget: Widget, ids: Record<string, string>): void {
  widget.id = ids[widget.id] ?? widget.id
  if (widget.parentId) widget.parentId = ids[widget.parentId] ?? widget.parentId
  remapWidgetReferences(widget, ids)
}
/** Capture property edits made directly to expanded instance widgets as persistent overrides. */
export function captureComponentInstanceOverrides(before: Project, after: Project): void {
  const previousWidgets = new Map(before.pages.flatMap(page => page.widgets).map(widget => [widget.id, widget]))
  for (const page of after.pages) for (const root of page.widgets.filter(widget => widget.componentInstance)) {
    const metadata = root.componentInstance!, component = after.designSystem?.components[metadata.componentId]
    if (!component) continue
    for (const [sourceId, instanceId] of Object.entries(metadata.widgetIdMap)) {
      const current = page.widgets.find(widget => widget.id === instanceId), previous = previousWidgets.get(instanceId), source = component.widgets.find(widget => widget.id === sourceId)
      if (!current || !previous || !source) continue
      for (const property of new Set([...Object.keys(previous.props), ...Object.keys(current.props)])) {
        if (JSON.stringify(previous.props[property]) === JSON.stringify(current.props[property])) continue
        if (!Object.hasOwn(current.props, property) || JSON.stringify(current.props[property]) === JSON.stringify(source.props[property])) delete metadata.overrides?.[sourceId]?.[property]
        else { metadata.overrides ??= {}; metadata.overrides[sourceId] ??= {}; metadata.overrides[sourceId]![property] = clone(current.props[property]) }
      }
    }
  }
}
export function insertComponentInstance(page: { widgets: Widget[] }, component: DesignComponent, options: { x?: number; y?: number; overrides?: Record<string, Record<string, unknown>>; variant?: Record<string, string> } = {}): Widget {
  assertVariant(component, options.variant)
  const map = Object.fromEntries(component.widgets.map(widget => [widget.id, newId('widget')]))
  const sourceRoot = component.widgets.find(widget => widget.id === component.rootWidgetId)
  if (!sourceRoot) throw new Error(`Component ${component.id} has no root widget`)
  const dx = (options.x ?? sourceRoot.x) - sourceRoot.x, dy = (options.y ?? sourceRoot.y) - sourceRoot.y
  const instanceId = newId('instance'), rootId = map[component.rootWidgetId]!
  const copies = component.widgets.map(source => {
    const widget = clone(source); widget.id = map[source.id]!; if (widget.parentId && map[widget.parentId]) widget.parentId = map[widget.parentId]
    if (source.id === component.rootWidgetId) { widget.x += dx; widget.y += dy }
    remapWidgetReferences(widget, map)
    if (source.id === component.rootWidgetId) widget.componentInstance = { componentId: component.id, instanceId, rootWidgetId: rootId, widgetIdMap: map, variant: clone(options.variant ?? {}), overrides: clone(options.overrides ?? {}) }
    applyVariantValues(component, options.variant, source.id, widget)
    for (const [property, value] of Object.entries(options.overrides?.[source.id] ?? {})) if (Object.hasOwn(widget.props, property)) widget.props[property] = clone(value)
    return widget
  })
  page.widgets.push(...copies); return copies.find(widget => widget.id === rootId)!
}
export function updateComponent(project: Project, component: DesignComponent): void {
  if (!project.designSystem?.components[component.id]) throw new Error(`Unknown component: ${component.id}`)
  project.designSystem.components[component.id] = clone(component)
  syncComponentInstances(project)
}
export function syncComponentInstances(project: Project): void {
  const components = project.designSystem?.components ?? {}
  for (const page of project.pages) for (const root of page.widgets.filter(widget => widget.componentInstance)) {
    const metadata = root.componentInstance!, component = components[metadata.componentId]
    if (!component) throw new Error(`Missing component ${metadata.componentId}`)
    if (!component.widgets.some(widget => widget.id === component.rootWidgetId)) throw new Error(`Component ${component.id} has no root widget`)
    if (metadata.widgetIdMap[component.rootWidgetId] !== root.id) throw new Error(`Component ${component.id} root widget ID cannot change after instances exist`)
    assertVariant(component, metadata.variant)
    const sourceIds = new Set(component.widgets.map(widget => widget.id))
    const removedSourceIds = Object.keys(metadata.widgetIdMap).filter(sourceId => !sourceIds.has(sourceId))
    const removedInstanceIds = removedSourceIds.map(sourceId => metadata.widgetIdMap[sourceId]!).filter(Boolean)
    if (removedInstanceIds.some(id => isWidgetReferenced(project, id))) throw new Error(`Cannot remove component widgets still referenced by bindings or flows: ${removedInstanceIds.join(', ')}`)
    for (const sourceId of removedSourceIds) { const id = metadata.widgetIdMap[sourceId]; delete metadata.widgetIdMap[sourceId]; page.widgets = page.widgets.filter(widget => widget.id !== id) }
    for (const source of component.widgets) {
      metadata.widgetIdMap[source.id] ??= newId('widget')
      const id = metadata.widgetIdMap[source.id]
      let target = page.widgets.find(widget => widget.id === id)
      if (!target) { target = clone(source); target.id = id; page.widgets.push(target) }
      const bindings = target.bindings, events = target.events, customProperties = target.customProperties, existingNestedInstance = target.componentInstance
      let sourceNestedInstance: Widget['componentInstance']
      if (source.componentInstance) { const holder = clone(source); remapWidgetReferences(holder, metadata.widgetIdMap); sourceNestedInstance = holder.componentInstance }
      Object.assign(target, clone(source), { id, parentId: source.parentId ? metadata.widgetIdMap[source.parentId] : root.parentId, x: source.id === component.rootWidgetId ? root.x : source.x, y: source.id === component.rootWidgetId ? root.y : source.y, bindings, events, customProperties })
      if (source.id === component.rootWidgetId) target.componentInstance = metadata
      else if (sourceNestedInstance) target.componentInstance = existingNestedInstance ?? sourceNestedInstance
      else delete target.componentInstance
      applyVariantValues(component, metadata.variant, source.id, target)
      for (const [property, value] of Object.entries(metadata.overrides?.[source.id] ?? {})) if (Object.hasOwn(target.props, property)) target.props[property] = clone(value)
    }
  }
}
function isWidgetReferenced(project: Project, widgetId: string): boolean {
  const contains = (value: unknown): boolean => {
    if (!value || typeof value !== 'object') return false
    if (Array.isArray(value)) return value.some(contains)
    return Object.entries(value as Record<string, unknown>).some(([key, child]) => (['widgetId', 'key'].includes(key) && child === widgetId) || contains(child))
  }
  return project.pages.some(page => page.widgets.some(widget => Object.values(widget.bindings).some(ref => 'key' in ref && ref.key === widgetId) || Object.values(widget.customProperties ?? {}).some(prop => prop.binding && 'key' in prop.binding && prop.binding.key === widgetId))) || project.flows.some(flow => flow.nodes.some(node => contains(node.config)))
}
export function setComponentOverride(page: { widgets: Widget[] }, rootId: string, sourceWidgetId: string, property: string, value: unknown): void {
  const root = page.widgets.find(widget => widget.id === rootId), metadata = root?.componentInstance
  if (!metadata) throw new Error('Widget is not a component instance root')
  const id = metadata.widgetIdMap[sourceWidgetId], target = page.widgets.find(widget => widget.id === id)
  if (!target || !Object.hasOwn(target.props, property)) throw new Error(`Unknown component property ${sourceWidgetId}.${property}`)
  metadata.overrides ??= {}; metadata.overrides[sourceWidgetId] ??= {}; metadata.overrides[sourceWidgetId]![property] = clone(value); target.props[property] = clone(value)
}
export function setComponentVariant(project: Project, rootId: string, variant: Record<string, string>): void {
  let root: Widget | undefined
  for (const page of project.pages) { root = page.widgets.find(widget => widget.id === rootId); if (root) break }
  const metadata = root?.componentInstance, component = metadata && project.designSystem?.components[metadata.componentId]
  if (!metadata || !component) throw new Error('Widget is not a valid component instance root')
  assertVariant(component, variant)
  metadata.variant = clone(variant)
  syncComponentInstances(project)
}
export function resetComponentOverrides(page: { widgets: Widget[] }, rootId: string, sourceWidgetId?: string, property?: string): void {
  const root = page.widgets.find(widget => widget.id === rootId), metadata = root?.componentInstance
  if (!metadata) throw new Error('Widget is not a component instance root')
  if (sourceWidgetId && property) delete metadata.overrides?.[sourceWidgetId]?.[property]
  else if (sourceWidgetId) delete metadata.overrides?.[sourceWidgetId]
  else metadata.overrides = {}
}
export function detachComponentInstance(page: { widgets: Widget[] }, rootId: string): void {
  const root = page.widgets.find(widget => widget.id === rootId), metadata = root?.componentInstance
  if (!metadata) throw new Error('Widget is not a component instance root')
  for (const id of Object.values(metadata.widgetIdMap)) { const widget = page.widgets.find(item => item.id === id); if (widget) delete widget.componentInstance }
}
