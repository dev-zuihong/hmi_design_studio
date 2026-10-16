import { computed, ref, type Ref } from 'vue'
import { clone, createWidget, emptyProject, newId, type Project, type Widget, type FlowEdge, type Flow } from './model'

export function checkConnection(flow: Flow, edge: Omit<FlowEdge, 'id'>): string | undefined {
  if (edge.source === edge.target) return '不能将节点连接到自身'
  const source = flow.nodes.find(n => n.id === edge.source), target = flow.nodes.find(n => n.id === edge.target)
  if (!source || !target) return '连接的节点不存在'
  if (target.kind.startsWith('trigger.')) return '触发节点不能接收输入'
  const ports = source.kind === 'condition' ? ['true', 'false', 'error'] : ['out', ...(source.kind.startsWith('trigger.') ? [] : ['error'])]
  if (!ports.includes(edge.sourcePort) || edge.targetPort !== 'in') return '端口方向无效'
  if (flow.edges.some(e => e.source === edge.source && e.target === edge.target && e.sourcePort === edge.sourcePort)) return '该连线已经存在'
  const visited = new Set<string>()
  function reaches(id: string): boolean { if (id === edge.source) return true; if (visited.has(id)) return false; visited.add(id); return flow.edges.filter(e => e.source === id).some(e => reaches(e.target)) }
  if (reaches(edge.target)) return '首版流程不允许回环，请使用定时触发重复执行'
}

export function parseProject(text: string): Project {
  const value = JSON.parse(text)
  if (!value || ![1, 2, 3].includes(value.schemaVersion)) throw new Error('不支持的项目格式版本；当前支持版本 1、2 和 3')
  if (typeof value.id !== 'string' || typeof value.name !== 'string' || !Number.isFinite(value.width) || !Number.isFinite(value.height) || value.width < 100 || value.height < 100) throw new Error('项目名称或画布尺寸无效')
  for (const key of ['pages', 'connections', 'variables', 'flows']) if (!Array.isArray(value[key])) throw new Error(`项目缺少 ${key}`)
  if (!value.pages.length || value.pages.some((page: any) => !page.id || !Array.isArray(page.widgets))) throw new Error('项目至少需要一个有效页面')
  for (const page of value.pages) for (const widget of page.widgets) {
    if (!widget.id || !widget.kind || !widget.props || !widget.bindings || !widget.events || ['x','y','width','height','zIndex'].some(key => !Number.isFinite(widget[key]))) throw new Error('控件属性不完整')
  }
  if (value.flows.some((flow: any) => !flow.id || !Array.isArray(flow.nodes) || !Array.isArray(flow.edges))) throw new Error('流程格式无效')
  const legacy = value.schemaVersion === 1
  value.schemaVersion = Math.max(2, value.schemaVersion)
  for (const page of value.pages) { page.layout ??= { type: 'absolute' }; page.responsive ??= !legacy; page.guides ??= [] }
  value.i18n ??= { defaultLocale: 'zh-CN', locales: ['zh-CN', 'en-US'], messages: { 'zh-CN': {}, 'en-US': {} } }
  value.actions ??= []
  return value as Project
}

export function useEditor(initial = emptyProject(), readOnlyState?: Ref<boolean>) {
  const readOnly = readOnlyState ?? ref(false)
  const project = ref<Project>(clone(initial)), selected = ref<string[]>([]), pageId = ref(initial.pages[0]!.id), flowId = ref(initial.flows[0]?.id ?? '')
  const activeSlots = ref<Record<string,string>>({})
  const past = ref<string[]>([]), future = ref<string[]>([]), saved = ref(JSON.stringify(initial))
  const remoteDirty = ref<boolean>(), remoteCanUndo = ref(false), remoteCanRedo = ref(false), gesturing = ref(false)
  let remoteUndo: (() => Promise<void>) | undefined, remoteRedo: (() => Promise<void>) | undefined
  const page = computed(() => project.value.pages.find(p => p.id === pageId.value) ?? project.value.pages[0]!)
  const flow = computed(() => project.value.flows.find(f => f.id === flowId.value))
  const selectedWidgets = computed(() => page.value.widgets.filter(w => selected.value.includes(w.id)))
  const dirty = computed(() => remoteDirty.value ?? JSON.stringify(project.value) !== saved.value)
  const canUndo = computed(() => remoteUndo ? remoteCanUndo.value : past.value.length > 0), canRedo = computed(() => remoteRedo ? remoteCanRedo.value : future.value.length > 0)
  function checkpoint() { if (readOnly.value) return false; const state = JSON.stringify(project.value); if (past.value.at(-1) !== state) { past.value.push(state); if (past.value.length > 100) past.value.shift() }; future.value = []; return true }
  function mutate(change: (p: Project) => void) { if (readOnly.value) return false; checkpoint(); change(project.value); return true }
  function ensureSelection() { selected.value = selected.value.filter(id => page.value.widgets.some(w => w.id === id)); if (!project.value.pages.some(p => p.id === pageId.value)) pageId.value = project.value.pages[0]!.id; if (!project.value.flows.some(flow => flow.id === flowId.value)) flowId.value = project.value.flows[0]?.id ?? '' }
  function undo() { if (readOnly.value) return false; if (remoteUndo) { void remoteUndo(); return true }; const state = past.value.pop(); if (!state) return false; future.value.push(JSON.stringify(project.value)); project.value = JSON.parse(state); ensureSelection(); return true }
  function redo() { if (readOnly.value) return false; if (remoteRedo) { void remoteRedo(); return true }; const state = future.value.pop(); if (!state) return false; past.value.push(JSON.stringify(project.value)); project.value = JSON.parse(state); ensureSelection(); return true }
  function replace(value: Project) { if (readOnly.value) return false; project.value = clone(value); saved.value = JSON.stringify(value); past.value = []; future.value = []; pageId.value = value.pages[0]!.id; flowId.value = value.flows[0]?.id ?? ''; selected.value = []; return true }
  function addWidget(kind: string, x?: number, y?: number, parentId?: string, slotId?: string, layoutItem?: Widget['layoutItem']) { if (readOnly.value) return; const widget = createWidget(kind, x, y); if (parentId) { widget.parentId = parentId; widget.slotId = slotId }; if (layoutItem) widget.layoutItem = layoutItem; if (!mutate(() => page.value.widgets.push(widget))) return; selected.value = [widget.id] }
  function updateWidget(id: string, patch: Partial<Widget>) { mutate(() => { const widget = page.value.widgets.find(w => w.id === id); if (widget) Object.assign(widget, patch) }) }
  function descendants(ids = selected.value): Widget[] { const included = new Set(ids); let changed = true; while (changed) { changed = false; for (const widget of page.value.widgets) if (widget.parentId && included.has(widget.parentId) && !included.has(widget.id)) { included.add(widget.id); changed = true } }; return page.value.widgets.filter(widget => included.has(widget.id)) }
  function removeSelected() { if (readOnly.value || !selected.value.length || selectedWidgets.value.some(widget => widget.editor?.locked)) return; const ids = new Set(descendants().map(widget => widget.id)); if (mutate(() => { page.value.widgets = page.value.widgets.filter(w => !ids.has(w.id)) })) selected.value = [] }
  function cloneWidgets(widgets: Widget[], offset = { x: 24, y: 24 }, parentId?: string) { const ids = new Map(widgets.map(widget => [widget.id, newId('widget')])); return widgets.map(widget => { const copy = clone(widget); copy.id = ids.get(widget.id)!; copy.name += ' 副本'; if (copy.parentId && ids.has(copy.parentId)) copy.parentId = ids.get(copy.parentId); else { copy.x += offset.x; copy.y += offset.y; if (parentId !== undefined) copy.parentId = parentId || undefined }; return copy }) }
  function duplicateSelected() { if (readOnly.value || !selected.value.length) return; const copies = cloneWidgets(descendants()); if (mutate(() => page.value.widgets.push(...copies))) selected.value = copies.filter(widget => !widget.parentId || !copies.some(parent => parent.id === widget.parentId)).map(widget => widget.id) }
  function align(direction: 'left' | 'top' | 'right' | 'bottom' | 'center' | 'middle' | 'horizontal' | 'vertical' | 'width' | 'height') {
    const widgets = selectedWidgets.value.filter(widget => !widget.editor?.locked)
    if (readOnly.value || widgets.length < 2 || widgets.some(widget => widget.parentId !== widgets[0]!.parentId)) return
    const layout = widgets[0]!.parentId ? page.value.widgets.find(parent=>parent.id===widgets[0]!.parentId)?.layout : page.value.layout
    if(layout && layout.type!=='absolute' && !['width','height'].includes(direction)) {
      const crossAxis:Record<string,'start'|'center'|'end'> = layout.type==='horizontal' ? {top:'start',middle:'center',bottom:'end'} : {left:'start',center:'center',right:'end'}
      const align = crossAxis[direction]
      if(align)mutate(()=>widgets.forEach(widget=>widget.layoutItem={...widget.layoutItem,align}))
      return
    }
    const left = Math.min(...widgets.map(widget => widget.x)), top = Math.min(...widgets.map(widget => widget.y)), right = Math.max(...widgets.map(widget => widget.x + widget.width)), bottom = Math.max(...widgets.map(widget => widget.y + widget.height)), primary = widgets.at(-1)!
    mutate(() => {
      if (direction === 'horizontal' || direction === 'vertical') { if (widgets.length < 3) return; const horizontal = direction === 'horizontal', ordered = [...widgets].sort((a, b) => horizontal ? a.x - b.x : a.y - b.y); const size = ordered.reduce((sum, widget) => sum + (horizontal ? widget.width : widget.height), 0); const gap = ((horizontal ? right - left : bottom - top) - size) / (ordered.length - 1); let position = horizontal ? left : top; for (const widget of ordered) { if (horizontal) widget.x = position; else widget.y = position; position += (horizontal ? widget.width : widget.height) + gap }; return }
      for (const widget of widgets) {
        if (direction === 'left') widget.x = left
        if (direction === 'right') widget.x = right - widget.width
        if (direction === 'center') widget.x = (left + right - widget.width) / 2
        if (direction === 'top') widget.y = top
        if (direction === 'bottom') widget.y = bottom - widget.height
        if (direction === 'middle') widget.y = (top + bottom - widget.height) / 2
        if (direction === 'width') widget.width = primary.width
        if (direction === 'height') widget.height = primary.height
      }
    })
  }
  function setContainerLayout(parentId: string | undefined, type: NonNullable<Widget['layout']>['type']) {
    const target = parentId ? page.value.widgets.find(widget=>widget.id===parentId) : page.value
    if(!target)return
    const rects = new Map<string,{x:number;y:number;width:number;height:number}>()
    if(type==='absolute' && target.layout?.type!=='absolute' && typeof document!=='undefined') {
      const board=document.querySelector<HTMLElement>('.design-board'), content=parentId?document.querySelector<HTMLElement>(`[data-container-content="${parentId}"]`):board
      if(board&&content){const area=content.getBoundingClientRect(),scale=board.getBoundingClientRect().width/project.value.width;for(const widget of page.value.widgets.filter(widget=>(widget.parentId??undefined)===parentId)){const element=board.querySelector<HTMLElement>(`[data-widget-id="${widget.id}"]`);if(element){const rect=element.getBoundingClientRect();rects.set(widget.id,{x:(rect.left-area.left)/scale+content.scrollLeft,y:(rect.top-area.top)/scale+content.scrollTop,width:rect.width/scale,height:rect.height/scale})}}}
    }
    mutate(()=>{target.layout={...target.layout,type,gap:target.layout?.gap??8,padding:target.layout?.padding??8};for(const widget of page.value.widgets){const rect=rects.get(widget.id);if(rect)Object.assign(widget,rect)}})
  }
  function hydrate(value: Project) { project.value = clone(value); ensureSelection() }
  function connectHistory(undo: () => Promise<void>, redo: () => Promise<void>) { remoteUndo = undo; remoteRedo = redo }
  return { project, page, pageId, flow, flowId, selected, selectedWidgets, activeSlots, past, future, dirty, readOnly, canUndo, canRedo, remoteDirty, remoteCanUndo, remoteCanRedo, gesturing, hydrate, connectHistory, descendants, cloneWidgets, checkpoint, mutate, undo, redo, replace, addWidget, updateWidget, removeSelected, duplicateSelected, align, setContainerLayout, markSaved: () => { saved.value = JSON.stringify(project.value) } }
}
export type Editor = ReturnType<typeof useEditor>
