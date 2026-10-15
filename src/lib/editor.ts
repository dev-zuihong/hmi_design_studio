import { computed, ref } from 'vue'
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
  if (!value || value.schemaVersion !== 1) throw new Error('不支持的项目格式版本；当前支持版本 1')
  if (typeof value.id !== 'string' || typeof value.name !== 'string' || !Number.isFinite(value.width) || !Number.isFinite(value.height) || value.width < 100 || value.height < 100) throw new Error('项目名称或画布尺寸无效')
  for (const key of ['pages', 'connections', 'variables', 'flows']) if (!Array.isArray(value[key])) throw new Error(`项目缺少 ${key}`)
  if (!value.pages.length || value.pages.some((page: any) => !page.id || !Array.isArray(page.widgets))) throw new Error('项目至少需要一个有效页面')
  for (const page of value.pages) for (const widget of page.widgets) {
    if (!widget.id || !widget.kind || !widget.props || !widget.bindings || !widget.events || ['x','y','width','height','zIndex'].some(key => !Number.isFinite(widget[key]))) throw new Error('控件属性不完整')
  }
  if (value.flows.some((flow: any) => !flow.id || !Array.isArray(flow.nodes) || !Array.isArray(flow.edges))) throw new Error('流程格式无效')
  return value as Project
}

export function useEditor(initial = emptyProject()) {
  const project = ref<Project>(clone(initial)), selected = ref<string[]>([]), pageId = ref(initial.pages[0]!.id), flowId = ref(initial.flows[0]?.id ?? '')
  const past = ref<string[]>([]), future = ref<string[]>([]), saved = ref(JSON.stringify(initial))
  const page = computed(() => project.value.pages.find(p => p.id === pageId.value) ?? project.value.pages[0]!)
  const flow = computed(() => project.value.flows.find(f => f.id === flowId.value))
  const selectedWidgets = computed(() => page.value.widgets.filter(w => selected.value.includes(w.id)))
  const dirty = computed(() => JSON.stringify(project.value) !== saved.value)
  function checkpoint() { const state = JSON.stringify(project.value); if (past.value.at(-1) !== state) { past.value.push(state); if (past.value.length > 100) past.value.shift() }; future.value = [] }
  function mutate(change: (p: Project) => void) { checkpoint(); change(project.value) }
  function ensureSelection() { selected.value = selected.value.filter(id => page.value.widgets.some(w => w.id === id)); if (!project.value.pages.some(p => p.id === pageId.value)) pageId.value = project.value.pages[0]!.id }
  function undo() { const state = past.value.pop(); if (!state) return; future.value.push(JSON.stringify(project.value)); project.value = JSON.parse(state); ensureSelection() }
  function redo() { const state = future.value.pop(); if (!state) return; past.value.push(JSON.stringify(project.value)); project.value = JSON.parse(state); ensureSelection() }
  function replace(value: Project) { project.value = clone(value); saved.value = JSON.stringify(value); past.value = []; future.value = []; pageId.value = value.pages[0]!.id; flowId.value = value.flows[0]?.id ?? ''; selected.value = [] }
  function addWidget(kind: string, x?: number, y?: number) { const widget = createWidget(kind, x, y); mutate(() => page.value.widgets.push(widget)); selected.value = [widget.id] }
  function updateWidget(id: string, patch: Partial<Widget>) { mutate(() => { const widget = page.value.widgets.find(w => w.id === id); if (widget) Object.assign(widget, patch) }) }
  function removeSelected() { if (!selected.value.length) return; mutate(() => { page.value.widgets = page.value.widgets.filter(w => !selected.value.includes(w.id)) }); selected.value = [] }
  function duplicateSelected() { if (!selected.value.length) return; const copies = selectedWidgets.value.map(w => ({ ...clone(w), id: newId('widget'), name: `${w.name} 副本`, x: w.x + 24, y: w.y + 24 })); mutate(() => page.value.widgets.push(...copies)); selected.value = copies.map(w => w.id) }
  function align(direction: 'left' | 'top') { if (selectedWidgets.value.length < 2) return; const key = direction === 'left' ? 'x' : 'y'; const minimum = Math.min(...selectedWidgets.value.map(w => w[key])); mutate(() => selectedWidgets.value.forEach(w => { w[key] = minimum })) }
  return { project, page, pageId, flow, flowId, selected, selectedWidgets, past, future, dirty, checkpoint, mutate, undo, redo, replace, addWidget, updateWidget, removeSelected, duplicateSelected, align, markSaved: () => { saved.value = JSON.stringify(project.value) } }
}
export type Editor = ReturnType<typeof useEditor>
