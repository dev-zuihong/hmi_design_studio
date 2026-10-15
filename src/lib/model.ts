export type ValueRef = { source: 'literal'; value: unknown } | { source: 'message'; path: string } | { source: 'variable' | 'input' | 'point'; key: string }
export interface UiAction { kind: 'flow' | 'startFlow' | 'stopFlow'; flowId: string; entryNodeId?: string }
export interface Widget { id: string; kind: string; name: string; x: number; y: number; width: number; height: number; zIndex: number; props: Record<string, any>; bindings: Record<string, ValueRef>; events: Record<string, UiAction> }
export interface Page { id: string; name: string; widgets: Widget[] }
export interface Variable { id: string; name: string; type: 'number' | 'string' | 'boolean' | 'enum'; initial: unknown; options?: string[] }
export interface ConnectionConfig { id: string; name: string; port: string; baudRate: number; dataBits: number; stopBits: number; parity: 'none' | 'odd' | 'even'; protocolId: string; protocolVersion?: string; protocolConfig: Record<string, unknown> }
export interface FlowNode { id: string; kind: string; label: string; position: { x: number; y: number }; config: Record<string, any> }
export interface FlowEdge { id: string; source: string; sourcePort: string; target: string; targetPort: string }
export interface Flow { id: string; name: string; enabled: boolean; timeoutMs: number; nodes: FlowNode[]; edges: FlowEdge[] }
export interface Project { schemaVersion: 1; id: string; name: string; width: number; height: number; pages: Page[]; connections: ConnectionConfig[]; variables: Variable[]; flows: Flow[]; python?: { runtimeId: string; runtimeSha256?: string; dependencyLock?: unknown[] }; assets?: { id: string; path: string }[] }
export interface ValidationIssue { severity: 'error' | 'warning'; path: string; message: string }
export interface RuntimeEvent { kind: string; timestamp: string | number; runId?: string; flowId?: string; nodeId?: string; data: any }
export interface Snapshot { variables: Record<string, any>; points: Record<string, any>; connections: Record<string, any>; flows: Record<string, any>; logs: unknown[] }
export interface PluginManifest { id: string; name: string; version: string; configSchema?: Record<string, any>; commands?: any[] | Record<string, any>; points?: any[] | Record<string, any> }
export interface Environment { mode: 'studio' | 'runner'; version: string; pythonAvailable: boolean; runnerAvailable: boolean; webviewAvailable: boolean; projectPath?: string }
export const newId = (prefix: string) => `${prefix}_${crypto.randomUUID().slice(0, 8)}`
export const clone = <T>(value: T): T => JSON.parse(JSON.stringify(value))
export const widgetKinds = [
  { kind: 'text', label: '文本', icon: 'Type', width: 160, height: 38 },
  { kind: 'button', label: '按钮', icon: 'MousePointer2', width: 140, height: 40 },
  { kind: 'input', label: '输入框', icon: 'TextCursorInput', width: 180, height: 40 },
  { kind: 'number', label: '数值显示', icon: 'Hash', width: 190, height: 110 },
  { kind: 'select', label: '下拉框', icon: 'ListFilter', width: 180, height: 40 },
  { kind: 'switch', label: '开关', icon: 'ToggleLeft', width: 160, height: 40 },
  { kind: 'indicator', label: '状态灯', icon: 'CircleDot', width: 160, height: 44 },
  { kind: 'group', label: '分组框', icon: 'Square', width: 300, height: 180 },
  { kind: 'chart', label: '实时曲线', icon: 'ChartNoAxesCombined', width: 500, height: 240 },
  { kind: 'log', label: '日志面板', icon: 'Logs', width: 500, height: 180 },
] as const
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
  { kind: 'page.navigate', label: '切换页面', group: '输出', color: '#49804b' },
  { kind: 'log', label: '调试日志', group: '输出', color: '#49804b' },
]
export function createWidget(kind: string, x = 64, y = 64): Widget {
  const spec = widgetKinds.find(item => item.kind === kind) ?? widgetKinds[0]
  return { id: newId('widget'), kind, name: spec.label, x, y, width: spec.width, height: spec.height, zIndex: kind === 'group' ? 0 : 1, props: { text: spec.label, fontSize: 14, color: '#242424', background: kind === 'group' ? '#fafafa' : '#ffffff', value: kind === 'switch' || kind === 'indicator' ? false : '', unit: '', precision: 1, enabled: true, visible: true, options: ['选项一', '选项二'] }, bindings: {}, events: {} }
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
    'page.navigate': { pageId: '' }, log: { value: { source: 'message', path: 'payload' }, level: 'info' },
  }
  return { id: newId('node'), kind, label: nodeKinds.find(item => item.kind === kind)?.label ?? kind, position, config: clone(configs[kind] ?? {}) }
}
export function emptyProject(): Project {
  return { schemaVersion: 1, id: newId('project'), name: '未命名项目', width: 1280, height: 720, pages: [{ id: 'page_main', name: '主界面', widgets: [] }], connections: [], variables: [], flows: [], assets: [] }
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
