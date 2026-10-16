import { describe, expect, it } from 'vitest'
import { createComponentFromSelection, insertComponentInstance, resolveDesignTokens, resolveWidgetStyle, setComponentOverride, setComponentVariant, updateComponent, validateDesignSystem } from './design-system'
import { deleteDesignComponent, deleteDesignLibrary, exportDesignLibrary, importDesignLibrary, parseDesignLibrary, serializeDesignLibrary } from './design-library'
import { createWidget, emptyProject, type DesignComponent } from './model'
import { useEditor } from './editor'

function projectWithTokens() {
  const project = emptyProject()
  project.designSystem = {
    version: 1,
    tokenSets: {
      base: { id: 'base', name: 'Base', tokens: { blue: { $type: 'color', $value: '#123456' }, spacing: { $type: 'dimension', $value: { value: 8, unit: 'px' } }, alias: { $type: 'color', $value: '{blue}' }, 'color.primary': { $type: 'color', $value: '#123456' }, dottedAlias: { $type: 'color', $value: '{color.primary}' }, body: { $type: 'typography', $value: { fontFamily: 'Arial', fontSize: { value: 20, unit: 'px' }, fontWeight: 600 } }, small: { $type: 'dimension', $value: '12px' } } },
      dark: { id: 'dark', name: 'Dark', tokens: { blue: { $type: 'color', $value: '#abcdef' } } },
    },
    themes: { darkTheme: { id: 'darkTheme', name: 'Dark', tokenSets: ['base', 'dark'] } }, components: {}, libraries: {}, activeThemeId: 'darkTheme',
  }
  return project
}

describe('design system engine', () => {
  it('resolves aliases, theme overrides, DTCG dimensions and rejects dangling aliases/cycles/type mismatches', () => {
    const project = projectWithTokens()
    const tokens = resolveDesignTokens(project)
    expect(tokens['base.blue']).toBe('#123456')
    expect(tokens.blue).toBe('#abcdef')
    expect(tokens['base.alias']).toBe('#123456')
    expect(tokens['base.spacing']).toEqual({ value: 8, unit: 'px' })
    expect(tokens['base.dottedAlias']).toBe('#123456')
    project.designSystem!.tokenSets.base!.tokens.alias!.$value = '{missing}'
    expect(() => resolveDesignTokens(project)).toThrow(/Unresolved/)
    project.designSystem!.tokenSets.base!.tokens.alias!.$value = '{spacing}'
    expect(() => resolveDesignTokens(project)).toThrow(/type mismatch/)
    project.designSystem!.tokenSets.base!.tokens.alias!.$type = 'dimension'
    project.designSystem!.tokenSets.base!.tokens.alias!.$value = '{cycle}'
    project.designSystem!.tokenSets.base!.tokens.cycle = { $type: 'dimension', $value: '{alias}' }
    expect(() => resolveDesignTokens(project)).toThrow(/cycle/)
  })

  it('applies visual tokens while leaving bound visual properties in control', () => {
    const project = projectWithTokens(), widget = createWidget('text')
    widget.styleTokens = { color: 'blue', fontSize: 'base.spacing' }
    widget.bindings.color = { source: 'variable', key: 'color' }
    expect(resolveWidgetStyle(project, widget)).toEqual({ fontSize: 8 })
  })

  it('resolves dotted unqualified token types and pins them to the source library theme', () => {
    const project = projectWithTokens(), widget = createWidget('text')
    project.designSystem!.tokenSets.base!.tokens['typography.body'] = { $type: 'typography', $value: { fontSize: '1.5rem', fontFamily: 'Arial' } }
    widget.styleTokens = { color: 'color.primary', typography: 'typography.body' }
    expect(resolveWidgetStyle(project, widget)).toEqual({ color: '#123456', fontSize: 24, fontFamily: 'Arial' })
    const component = createComponentFromSelection({ widgets: [widget] }, [widget.id], { id: 'dotted' })
    project.designSystem!.components[component.id] = component
    const target = projectWithTokens()
    target.designSystem!.tokenSets.base!.tokens['color.primary']!.$value = '#ff0000'
    const result = importDesignLibrary(target, exportDesignLibrary(project), { namespace: 'dotted-library' })
    const imported = target.designSystem!.components[result.componentIds.dotted!]!.widgets[0]!
    expect(imported.styleTokens).toEqual({ color: 'dotted-library:base.color.primary', typography: 'dotted-library:base.typography.body' })
    expect(resolveWidgetStyle(target, imported)).toEqual({ color: '#123456', fontSize: 24, fontFamily: 'Arial' })
  })

  it('uses sorted token sets without a theme and clears external slot ownership from a nested component root', () => {
    const project = projectWithTokens()
    delete project.designSystem!.activeThemeId
    delete project.designSystem!.themes.darkTheme
    project.designSystem!.tokenSets = {
      zeta: { id: 'zeta', name: 'Zeta', tokens: { shared: { $type: 'dimension', $value: '18px' } } },
      alpha: { id: 'alpha', name: 'Alpha', tokens: { shared: { $type: 'dimension', $value: '8px' } } },
    }
    expect(resolveDesignTokens(project).shared).toBe('18px')
    const parent = createWidget('frame'), nested = createWidget('button')
    nested.parentId = parent.id; nested.slotId = 'content'
    const component = createComponentFromSelection({ widgets: [parent, nested] }, [nested.id])
    expect(component.widgets[0]!.parentId).toBeUndefined()
    expect(component.widgets[0]!.slotId).toBeUndefined()
  })

  it('keeps qualified tokens available while a theme exposes only its own sets as unqualified values', () => {
    const project = projectWithTokens()
    project.designSystem!.tokenSets.brand = { id: 'brand', name: 'Brand', tokens: { blue: { $type: 'color', $value: '#ff0000' } } }
    project.designSystem!.themes.brandTheme = { id: 'brandTheme', name: 'Brand', tokenSets: ['brand'] }
    const tokens = resolveDesignTokens(project, 'brandTheme')
    expect(tokens['base.blue']).toBe('#123456')
    expect(tokens['brand.blue']).toBe('#ff0000')
    expect(tokens.blue).toBe('#ff0000')
    const widget = createWidget('text')
    widget.styleTokens = { color: 'base.blue' }
    expect(resolveWidgetStyle(project, widget, 'brandTheme')).toEqual({ color: '#123456' })
  })

  it('validates widgets against only the active theme and rejects switching to a theme missing their token', () => {
    const project = projectWithTokens(), widget = createWidget('text')
    widget.styleTokens = { color: 'blue' }
    project.pages[0]!.widgets.push(widget)
    project.designSystem!.tokenSets.other = { id: 'other', name: 'Other', tokens: { accent: { $type: 'color', $value: '#ff0000' } } }
    project.designSystem!.themes.otherTheme = { id: 'otherTheme', name: 'Other', tokenSets: ['other'] }
    expect(() => validateDesignSystem(project)).not.toThrow()
    project.designSystem!.activeThemeId = 'otherTheme'
    expect(() => validateDesignSystem(project)).toThrow(/Unknown style token/)
  })

  it('rejects component parent cycles and dangling or cyclic nested component references', () => {
    const project = projectWithTokens(), first = createWidget('frame'), second = createWidget('frame')
    first.parentId = second.id; second.parentId = first.id
    project.designSystem!.components.loop = { id: 'loop', name: 'Loop', rootWidgetId: first.id, widgets: [first, second] }
    expect(() => validateDesignSystem(project)).toThrow(/parent cycle/)
    first.parentId = undefined; second.parentId = first.id
    first.componentInstance = { componentId: 'missing', instanceId: 'nested', rootWidgetId: first.id, widgetIdMap: {} }
    expect(() => validateDesignSystem(project)).toThrow(/missing nested component/)
    first.componentInstance.componentId = 'loop'
    expect(() => validateDesignSystem(project)).toThrow(/Nested component reference cycle/)
  })

  it('applies typography before explicit scalar tokens regardless of object key order', () => {
    const project = projectWithTokens(), widget = createWidget('text')
    widget.styleTokens = { fontSize: 'base.small', typography: 'base.body' }
    expect(resolveWidgetStyle(project, widget)).toEqual({ fontFamily: 'Arial', fontSize: 12, fontWeight: 600 })
  })

  it('inserts and updates instances with stable IDs, overrides, variants and bindings/events preserved', () => {
    const project = projectWithTokens(), page = project.pages[0]!
    const root = createWidget('button'), child = createWidget('text'), grandchild = createWidget('text'); child.parentId = root.id; grandchild.parentId = child.id
    child.x = 13; child.y = 19; grandchild.x = 4; grandchild.y = 6
    const nestedRoot = createWidget('text'), nestedChild = createWidget('text'); nestedChild.parentId = nestedRoot.id
    const nestedComponent: DesignComponent = { id: 'nested', name: 'Nested', rootWidgetId: nestedRoot.id, widgets: [nestedRoot, nestedChild] }
    project.designSystem!.components.nested = nestedComponent
    child.componentInstance = { componentId: nestedComponent.id, instanceId: 'nested-template-instance', rootWidgetId: child.id, widgetIdMap: { [nestedRoot.id]: child.id, [nestedChild.id]: grandchild.id } }
    root.bindings.disabled = { source: 'variable', key: 'busy' }; root.events.click = { kind: 'flow', flowId: 'flow-a' }
    page.widgets.push(root, child, grandchild)
    const component = createComponentFromSelection(page, [root.id], { id: 'button-component' })
    component.variants = { axes: { size: ['small', 'large'], tone: ['primary', 'quiet'] }, combinations: { small: { size: 'small', tone: 'primary' }, large: { size: 'large', tone: 'primary' } }, values: { small: { [root.id]: { fontSize: 10 } }, large: { [root.id]: { fontSize: 20 } } } }
    project.designSystem!.components[component.id] = component
    const instance = insertComponentInstance(page, component, { x: 300, y: 200, variant: { size: 'large', tone: 'primary' }, overrides: { [root.id]: { text: 'Local' } } })
    const mappedChild = instance.componentInstance!.widgetIdMap[child.id]!
    const mappedGrandchild = instance.componentInstance!.widgetIdMap[grandchild.id]!
    const instanceChild = page.widgets.find(widget => widget.id === mappedChild)!
    expect(instance.x).toBe(300); expect(instance.y).toBe(200)
    expect(instanceChild.x).toBe(13); expect(instanceChild.y).toBe(19)
    expect(instanceChild.componentInstance?.componentId).toBe('nested')
    expect(instanceChild.componentInstance?.widgetIdMap[nestedRoot.id]).toBe(mappedChild)
    expect(() => deleteDesignComponent(project, nestedComponent.id)).toThrow(/instances reference/)
    instanceChild.bindings.value = { source: 'variable', key: 'local-binding' }
    instanceChild.events.click = { kind: 'flow', flowId: 'local-event' }
    expect(instance.props.fontSize).toBe(20)
    expect(instance.props.text).toBe('Local')
    expect(instance.bindings.disabled).toEqual({ source: 'variable', key: 'busy' })
    expect(instance.events.click).toEqual({ kind: 'flow', flowId: 'flow-a' })
    setComponentOverride(page, instance.id, root.id, 'text', 'Override')
    const updated: DesignComponent = structuredClone(component)
    updated.widgets[0]!.props.title = 'Updated'
    updated.widgets[0]!.props.text = 'Main'
    updated.widgets[0]!.bindings.value = { source: 'variable', key: 'newValue' }
    updated.widgets[1]!.x = 23
    updated.widgets[1]!.bindings.value = { source: 'variable', key: 'newDefinitionBinding' }
    updated.widgets[1]!.events.click = { kind: 'flow', flowId: 'definition-event' }
    updateComponent(project, updated)
    expect(page.widgets.find(widget => widget.id === instance.id)!.props.text).toBe('Override')
    expect(page.widgets.find(widget => widget.id === instance.id)!.props.title).toBe('Updated')
    expect(page.widgets.find(widget => widget.id === instance.id)!.bindings.disabled).toEqual({ source: 'variable', key: 'busy' })
    expect(page.widgets.find(widget => widget.id === instance.id)!.events.click).toEqual({ kind: 'flow', flowId: 'flow-a' })
    expect(page.widgets.find(widget => widget.id === mappedChild)!.x).toBe(23)
    expect(page.widgets.find(widget => widget.id === mappedChild)!.bindings.value).toEqual({ source: 'variable', key: 'local-binding' })
    expect(page.widgets.find(widget => widget.id === mappedChild)!.events.click).toEqual({ kind: 'flow', flowId: 'local-event' })
    expect(page.widgets.find(widget => widget.id === mappedGrandchild)).toBeTruthy()
    expect(page.widgets.find(widget => widget.id === mappedGrandchild)!.x).toBe(64)
    expect(() => setComponentVariant(project, instance.id, { size: 'large', tone: 'quiet' })).toThrow(/Unsupported/)
    setComponentVariant(project, instance.id, { size: 'small', tone: 'primary' })
    expect(page.widgets.find(widget => widget.id === instance.id)!.props.fontSize).toBe(10)
  })

  it('rolls back failed instance normalization atomically', () => {
    const project = projectWithTokens(), root = createWidget('button')
    project.pages[0]!.widgets.push(root)
    const component = createComponentFromSelection(project.pages[0]!, [root.id], { id: 'rollback' })
    project.designSystem!.components[component.id] = component
    const instance = insertComponentInstance(project.pages[0]!, component)
    const editor = useEditor(project), before = JSON.stringify(editor.project.value)
    expect(() => editor.mutate(value => { value.designSystem!.components.rollback!.widgets = [] })).toThrow(/root widget/)
    expect(JSON.stringify(editor.project.value)).toBe(before)
    expect(editor.past.value).toHaveLength(0)
  })

  it('refuses to remove referenced instance children and rolls back the component edit', () => {
    const project = projectWithTokens(), page = project.pages[0]!, root = createWidget('frame'), child = createWidget('text')
    child.parentId = root.id; page.widgets.push(root, child)
    const component = createComponentFromSelection(page, [root.id], { id: 'referenced-child' })
    project.designSystem!.components[component.id] = component
    const instance = insertComponentInstance(page, component), mappedChild = instance.componentInstance!.widgetIdMap[child.id]!
    const reader = createWidget('text'); reader.bindings.value = { source: 'property', key: mappedChild, property: 'text' }; page.widgets.push(reader)
    const editor = useEditor(project), before = JSON.stringify(editor.project.value)
    const updated = structuredClone(component); updated.widgets = updated.widgets.filter(widget => widget.id !== child.id)
    expect(() => editor.mutate(value => updateComponent(value, updated))).toThrow(/still referenced/)
    expect(JSON.stringify(editor.project.value)).toBe(before)
    expect(editor.project.value.designSystem!.components[component.id]!.widgets.some(widget => widget.id === child.id)).toBe(true)
  })

  it('round trips an offline library with embedded asset bytes and remaps imported IDs', () => {
    const project = projectWithTokens(), widget = createWidget('button')
    widget.props.assetId = 'logo'
    widget.styleTokens = { color: 'blue' }
    const nestedWidget = createWidget('image')
    project.designSystem!.components.nested = { id: 'nested', name: 'Nested', rootWidgetId: nestedWidget.id, widgets: [nestedWidget] }
    widget.componentInstance = {
      componentId: 'nested', instanceId: 'nested-instance', rootWidgetId: widget.id,
      widgetIdMap: { [nestedWidget.id]: widget.id }, overrides: { [nestedWidget.id]: { assetId: 'logo' } },
    }
    const component = createComponentFromSelection({ widgets: [widget] }, [widget.id], { id: 'primary', name: 'Primary' })
    component.variants = {
      axes: { appearance: ['asset', 'plain'] },
      combinations: { asset: { appearance: 'asset' }, plain: { appearance: 'plain' } },
      values: { asset: { [widget.id]: { assetId: 'logo' } } },
    }
    project.designSystem!.components[component.id] = component
    const exported = exportDesignLibrary(project, { namespace: 'brand', assets: [{ id: 'logo', name: 'logo.png', mimeType: 'image/png', bytesBase64: 'AQID', metadata: { width: 1, height: 1 } }] })
    const parsed = parseDesignLibrary(serializeDesignLibrary(exported))
    const target = emptyProject()
    const first = importDesignLibrary(target, parsed, { namespace: 'brand-v2' })
    const result = importDesignLibrary(target, parsed, { namespace: 'brand-v2' })
    expect(first.namespace).toBe('brand-v2')
    expect(result.namespace).toBe('brand-v2-2')
    expect(result.assets[0]!.bytesBase64).toBe('AQID')
    expect(result.assets[0]!.id).toBe('brand-v2-2:logo')
    expect(target.designSystem!.components[result.componentIds.primary!]!.name).toContain('Primary')
    expect(target.designSystem!.components[result.componentIds.primary!]!.widgets[0]!.props.assetId).toBe('brand-v2-2:logo')
    expect(target.designSystem!.components[result.componentIds.primary!]!.widgets[0]!.styleTokens!.color).toBe('brand-v2-2:dark.blue')
    expect(target.designSystem!.components[result.componentIds.primary!]!.widgets[0]!.componentInstance!.componentId).toBe(result.componentIds.nested)
    expect(target.designSystem!.components[result.componentIds.primary!]!.widgets[0]!.componentInstance!.overrides![nestedWidget.id]!.assetId).toBe('brand-v2-2:logo')
    expect(target.designSystem!.components[result.componentIds.primary!]!.variants!.values!.asset![component.rootWidgetId]!.assetId).toBe('brand-v2-2:logo')
    const variantInstance = insertComponentInstance(target.pages[0]!, target.designSystem!.components[result.componentIds.primary!]!, { variant: { appearance: 'asset' } })
    expect(variantInstance.props.assetId).toBe('brand-v2-2:logo')
    target.pages[0]!.widgets = []
    expect(target.designSystem!.tokenSets[result.tokenSetIds.base!]!.tokens.blue!.$value).toBe('#123456')
    target.pages[0]!.widgets.push({ ...createWidget('button'), componentInstance: { componentId: result.componentIds.primary!, instanceId: 'used', rootWidgetId: 'instance', widgetIdMap: { [component.rootWidgetId]: 'instance' } } })
    expect(() => deleteDesignComponent(target, result.componentIds.primary!)).toThrow(/instances reference/)
    expect(() => deleteDesignLibrary(target, result.namespace)).toThrow(/in use/)
    target.pages[0]!.widgets = []
    const image = createWidget('image'); image.props.assetId = 'brand-v2-2:logo'; target.pages[0]!.widgets.push(image)
    expect(() => deleteDesignLibrary(target, result.namespace)).toThrow(/in use/)
    target.pages[0]!.widgets = []
    const externalWidget = createWidget('image')
    target.designSystem!.components.externalVariant = {
      id: 'externalVariant', name: 'External variant', rootWidgetId: externalWidget.id,
      widgets: [externalWidget],
      variants: {
        axes: { appearance: ['asset'] }, combinations: { asset: { appearance: 'asset' } },
        values: { asset: { [externalWidget.id]: { assetId: 'brand-v2-2:logo' } } },
      },
    }
    expect(() => deleteDesignLibrary(target, result.namespace)).toThrow(/in use/)
    delete target.designSystem!.components.externalVariant
    expect(deleteDesignLibrary(target, result.namespace)).toEqual(['brand-v2-2:logo'])
    expect(target.designSystem!.components[result.componentIds.primary!]).toBeUndefined()
  })

  it('persists direct Inspector prop edits as instance overrides and remaps metadata when copied', () => {
    const project = projectWithTokens(), page = project.pages[0]!, root = createWidget('button'), child = createWidget('text')
    child.parentId = root.id
    root.bindings.value = { source: 'property', key: child.id, property: 'text' }
    page.widgets.push(root, child)
    const component = createComponentFromSelection(page, [root.id], { id: 'copyable' })
    project.designSystem!.components[component.id] = component
    const instance = insertComponentInstance(page, component)
    const editor = useEditor(project), instanceRoot = editor.page.value.widgets.find(widget => widget.id === instance.id)!
    editor.mutate(() => { instanceRoot.props.text = 'Inspector value' })
    expect(instanceRoot.componentInstance!.overrides?.[root.id]?.text).toBe('Inspector value')
    const copy = editor.cloneWidgets([instanceRoot, ...editor.page.value.widgets.filter(widget => widget.parentId === instanceRoot.id)])
    const copiedRoot = copy.find(widget => !widget.parentId)!
    expect(copiedRoot.componentInstance!.instanceId).not.toBe(instanceRoot.componentInstance!.instanceId)
    expect(copiedRoot.componentInstance!.rootWidgetId).toBe(copiedRoot.id)
    expect(Object.values(copiedRoot.componentInstance!.widgetIdMap)).not.toContain(instanceRoot.id)
    const sourceChild = component.widgets.find(widget => widget.id !== component.rootWidgetId)!
    expect(copiedRoot.bindings.value).toEqual({ source: 'property', key: copiedRoot.componentInstance!.widgetIdMap[sourceChild.id], property: 'text' })
    const updated = structuredClone(component); updated.widgets[0]!.props.text = 'New component value'; updateComponent(editor.project.value, updated)
    expect(editor.page.value.widgets.find(widget => widget.id === instance.id)!.props.text).toBe('Inspector value')
  })
})
