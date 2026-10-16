export type ValueRef = { source: 'literal'; value: unknown } | { source: 'message'; path: string } | { source: 'variable' | 'input' | 'point'; key: string } | { source: 'property'; key: string; property: string }
export interface UiAction { kind: 'flow' | 'startFlow' | 'stopFlow' | 'shared'; flowId: string; entryNodeId?: string; actionId?: string }
export type LayoutType = 'absolute' | 'horizontal' | 'vertical' | 'grid' | 'form'
export type LocalizedText = string | { i18nKey: string; fallback: string }
export interface LayoutSpec { type: LayoutType; gap?: number; padding?: number; columns?: number }
export interface LayoutItem { grow?: number; minWidth?: number; maxWidth?: number; minHeight?: number; maxHeight?: number; align?: 'start' | 'center' | 'end' | 'stretch'; row?: number; column?: number; rowSpan?: number; columnSpan?: number }
export type CustomPropertyType = 'string' | 'number' | 'boolean' | 'enum' | 'object' | 'array'
export interface CustomProperty { type: CustomPropertyType; value: unknown; options?: string[]; binding?: ValueRef; event?: UiAction }
export interface Widget { id: string; kind: string; name: string; x: number; y: number; width: number; height: number; zIndex: number; props: Record<string, any>; bindings: Record<string, ValueRef>; events: Record<string, UiAction>; parentId?: string | null; slotId?: string; layout?: LayoutSpec; layoutItem?: LayoutItem; slots?: { id: string; title: LocalizedText }[]; customProperties?: Record<string, CustomProperty>; editor?: { locked?: boolean; hidden?: boolean } }
export interface Page { id: string; name: string; widgets: Widget[]; layout?: LayoutSpec; responsive?: boolean; guides?: { id: string; axis: 'x' | 'y'; position: number }[] }
export interface Variable { id: string; name: string; type: 'number' | 'string' | 'boolean' | 'enum' | 'array' | 'object'; initial: unknown; options?: string[] }
export interface ConnectionConfig { id: string; name: string; port: string; baudRate: number; dataBits: number; stopBits: number; parity: 'none' | 'odd' | 'even'; protocolId: string; protocolVersion?: string; protocolConfig: Record<string, unknown> }
export interface FlowNode { id: string; kind: string; label: string; position: { x: number; y: number }; config: Record<string, any> }
export interface FlowEdge { id: string; source: string; sourcePort: string; target: string; targetPort: string }
export interface Flow { id: string; name: string; enabled: boolean; timeoutMs: number; nodes: FlowNode[]; edges: FlowEdge[] }
export interface ProjectAction { id: string; text: string | { i18nKey: string; fallback: string }; icon?: string; shortcut?: string; enabled?: boolean; visible?: boolean; action: UiAction }
export interface ProjectI18n { defaultLocale: string; locales: string[]; messages: Record<string, Record<string, string>> }
export interface Project { schemaVersion: 1 | 2; id: string; name: string; width: number; height: number; pages: Page[]; connections: ConnectionConfig[]; variables: Variable[]; flows: Flow[]; python?: { runtimeId: string; runtimeSha256?: string; dependencyLock?: unknown[] }; assets?: { id: string; path: string }[]; i18n?: ProjectI18n; actions?: ProjectAction[] }
export interface ValidationIssue { severity: 'error' | 'warning'; path: string; message: string }
export interface RuntimeEvent { generation?: number; revision?: number; kind: string; timestamp: string | number; runId?: string; flowId?: string; nodeId?: string; data: any }
export interface Snapshot { variables: Record<string, any>; points: Record<string, any>; connections: Record<string, any>; flows: Record<string, any>; logs: unknown[]; ui?: Record<string, any>; inputs?: Record<string, unknown> }
export interface PluginManifest { id: string; name: string; version: string; configSchema?: Record<string, any>; commands?: any[] | Record<string, any>; points?: any[] | Record<string, any> }
export interface Environment { mode: 'studio' | 'runner' | 'preview'; version: string; pythonAvailable: boolean; runnerAvailable: boolean; webviewAvailable: boolean; projectPath?: string; platform?: string; target?: string; pythonRuntimeId?: string }
export const newId = (prefix: string) => `${prefix}_${crypto.randomUUID().slice(0, 8)}`
export const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value))
import widgetCatalog from '../../resources/widget-catalog.json' with { type: 'json' }
export interface WidgetPropertySpec { key: string; type: string; label: string; default?: unknown; options?: string[]; translatable?: boolean }
export interface WidgetKindSpec { kind: string; label: string; icon: string; group: string; width: number; height: number; container?: boolean; slots?: { id: string; title: string }[]; defaults: Record<string, unknown>; propertyGroups?: Record<string, WidgetPropertySpec[]> }
export const commonWidgetPropertyGroups = widgetCatalog.commonPropertyGroups as Record<string, WidgetPropertySpec[]>
export const widgetKinds = widgetCatalog.widgets as WidgetKindSpec[]
export const nodeKinds = [
  { kind: 'trigger.ui', label: '界面事件', group: '触发', color: '#6f67c4' },
  { kind: 'trigger.timer', label: '定时触发', group: '触发', color: '#6f67c4' },
  { kind: 'trigger.data', label: '数据变化', group: '触发', color: '#6f67c4' },
  { kind: 'connection.open', label: '打开连接', group: '设备', color: '#087f8c' },
  { kind: 'connection.close', label: '关闭连接', group: '设备', color: '#087f8c' },
  { kind: 'protocol.command', label: '协议命令', group: '设备', color: '#087f8c' },
  { kind: 'condition', label: '条件分支', group: '逻辑', color: '#b77a18' },
  { kind: 'assign', label: '消息赋值', group: '逻辑', color: '#b77a18' },
  { kind: 'variable.get', label: '读取变量', group: '逻辑', color: '#b77a18' },
  { kind: 'variable.set', label: '写入变量', group: '逻辑', color: '#b77a18' },
  { kind: 'delay', label: '延时等待', group: '逻辑', color: '#b77a18' },
  { kind: 'python', label: 'Python 代码', group: '代码', color: '#346faa' },
  { kind: 'ui.setProperty', label: '设置控件属性', group: '输出', color: '#49804b' },
  { kind: 'page.navigate', label: '切换页面', group: '输出', color: '#49804b' },
  { kind: 'log', label: '调试日志', group: '输出', color: '#49804b' },
]
export function createWidget(kind: string, x = 64, y = 64): Widget {
  const spec = widgetKinds.find(item => item.kind === kind) ?? widgetKinds[0]
  const id = newId('widget')
  const commonDefaults = Object.fromEntries(Object.values(commonWidgetPropertyGroups).flat().filter(property => property.default !== undefined).map(property => [property.key, property.default]))
  return { id, kind, name: spec.label, x, y, width: spec.width, height: spec.height, zIndex: kind === 'group' || kind === 'frame' ? 0 : 1, props: clone({ ...commonDefaults, ...spec.defaults }), bindings: {}, events: {}, ...(spec.container ? { slots: clone(spec.slots ?? [{ id: 'content', title: '内容' }]), layout: { type: 'absolute' as const, gap: 8, padding: 8 } } : {}) }
}
export function createNode(kind: string, position = { x: 80, y: 80 }): FlowNode {
  const configs: Record<string, Record<string, any>> = {
    'trigger.timer': { intervalMs: 1000 }, 'trigger.data': { key: '' },
    'connection.open': { connectionId: '' }, 'connection.close': { connectionId: '' },
    'protocol.command': { connectionId: '', command: '', params: {} },
    condition: { left: { source: 'message', path: 'payload' }, operator: 'gt', right: { source: 'literal', value: 0 } },
    assign: { path: 'payload', value: { source: 'literal', value: 0 } },
    'variable.get': { variableId: '', path: 'payload' }, 'variable.set': { variableId: '', value: { source: 'message', path: 'payload' } },
    delay: { durationMs: 1000 }, python: { code: 'def transform(msg):\n    # 在此处理输入消息\n    return msg\n', timeoutMs: 5000, memoryMb: 256 },
    'ui.setProperty': { widgetId: '', property: 'visible', value: { source: 'literal', value: true } },
    'page.navigate': { pageId: '' }, log: { value: { source: 'message', path: 'payload' }, level: 'info' },
  }
  return { id: newId('node'), kind, label: nodeKinds.find(item => item.kind === kind)?.label ?? kind, position, config: clone(configs[kind] ?? {}) }
}
export function emptyProject(): Project {
  return { schemaVersion: 2, id: newId('project'), name: '未命名项目', width: 1280, height: 720, pages: [{ id: 'page_main', name: '主界面', widgets: [], responsive: true, layout: { type: 'absolute' }, guides: [] }], connections: [], variables: [], flows: [], assets: [], actions: [], i18n: { defaultLocale: 'zh-CN', locales: ['zh-CN','en-US'], messages: { 'zh-CN': {}, 'en-US': {} } } }
}
export function demoProject(): Project {
  const project = emptyProject()
  project.id = 'temperature-demo'; project.name = '温度监测工作台'
  const title = createWidget('text', 40, 32); title.props.text = '温度监测'; title.props.fontSize = 26; title.width = 380; title.height = 48
  const subtitle = createWidget('text', 42, 83); subtitle.props.text = '连接设备，开始采集与控制'; subtitle.props.color = '#707070'; subtitle.width = 500
  const number = createWidget('number', 40, 150); number.width = 270; number.height = 150; number.props.text = '当前温度'; number.props.unit = '°C'; number.bindings.value = { source: 'variable', key: 'temperature' }
  const status = createWidget('indicator', 350, 160); status.props.text = '采集已启用'; status.bindings.value = { source: 'variable', key: 'collecting' }
  const button = createWidget('button', 350, 235); button.props.text = '读取温度'; button.events.click = { kind: 'flow', flowId: 'flow_read', entryNodeId: 'read_trigger' }
  const open = createWidget('button', 40, 620); open.props.text = '打开连接'; open.events.click = { kind: 'flow', flowId: 'flow_connect', entryNodeId: 'connect_trigger' }
  const close = createWidget('button', 196, 620); close.props.text = '关闭连接'; close.events.click = { kind: 'flow', flowId: 'flow_close', entryNodeId: 'close_trigger' }
  const chart = createWidget('chart', 40, 330); chart.width = 720; chart.height = 250; chart.props.text = '温度趋势'; chart.props.unit = '°C'; chart.bindings.value = { source: 'variable', key: 'temperature' }
  const log = createWidget('log', 800, 150); log.width = 430; log.height = 430; log.props.text = '运行记录'
  project.pages[0]!.widgets = [title, subtitle, number, status, button, open, close, chart, log]
  project.variables = [{ id: 'temperature', name: '当前温度', type: 'number', initial: 0 }, { id: 'collecting', name: '采集状态', type: 'boolean', initial: false }]
  project.connections = [{ id: 'sensor', name: '温度传感器', port: 'sim://temperature', baudRate: 115200, dataBits: 8, stopBits: 1, parity: 'none', protocolId: 'demo-temperature', protocolVersion: '1.0.0', protocolConfig: {} }]
  const makeFlow = (id: string, name: string, kinds: string[], ids: string[]) => {
    const nodes = kinds.map((kind, i) => ({ ...createNode(kind, { x: 60 + i * 250, y: 130 }), id: ids[i]! }))
    return { id, name, enabled: true, timeoutMs: 30000, nodes, edges: nodes.slice(1).map((node, i) => ({ id: newId('edge'), source: nodes[i]!.id, sourcePort: 'out', target: node.id, targetPort: 'in' })) }
  }
  const connect = makeFlow('flow_connect', '打开设备连接', ['trigger.ui', 'connection.open'], ['connect_trigger', 'connect_open']); connect.nodes[1]!.config.connectionId = 'sensor'
  const closeFlow = makeFlow('flow_close', '关闭设备连接', ['trigger.ui', 'connection.close'], ['close_trigger', 'close_close']); closeFlow.nodes[1]!.config.connectionId = 'sensor'
  const read = makeFlow('flow_read', '读取温度', ['trigger.ui', 'protocol.command', 'variable.set', 'log'], ['read_trigger', 'read_command', 'read_set', 'read_log'])
  read.nodes[1]!.config = { connectionId: 'sensor', command: 'read', params: {} }; read.nodes[2]!.config = { variableId: 'temperature', value: { source: 'message', path: 'payload.temperature' } }
  project.flows = [connect, read, closeFlow]
  return project
}
