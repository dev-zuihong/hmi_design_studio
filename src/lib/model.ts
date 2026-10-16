export type ValueRef = { source: 'literal'; value: unknown } | { source: 'message' | 'context' | 'runtime'; path: string } | { source: 'variable'; key: string; path?: string } | { source: 'input' | 'point'; key: string } | { source: 'backend'; key: string; path: string } | { source: 'property'; key: string; property: string }
export interface UiAction { kind: 'flow' | 'startFlow' | 'stopFlow' | 'shared' | 'runtimeRetry'; flowId: string; entryNodeId?: string; actionId?: string }
export type LayoutType = 'absolute' | 'horizontal' | 'vertical' | 'grid' | 'form'
export type LocalizedText = string | { i18nKey: string; fallback: string }
export type LayoutTrack = { kind: 'px' | 'fr'; value: number } | { kind: 'auto' }
export interface LayoutSpec { type: LayoutType; gap?: number; padding?: number; paddingSides?: { top?: number; right?: number; bottom?: number; left?: number }; gapX?: number; gapY?: number; direction?: 'row' | 'column' | 'row-reverse' | 'column-reverse'; wrap?: boolean; justify?: 'start' | 'center' | 'end' | 'space-between' | 'space-around' | 'space-evenly'; align?: 'start' | 'center' | 'end' | 'stretch'; columns?: number; rows?: number; columnTracks?: LayoutTrack[]; rowTracks?: LayoutTrack[] }
export interface LayoutItem { grow?: number; minWidth?: number; maxWidth?: number; minHeight?: number; maxHeight?: number; align?: 'start' | 'center' | 'end' | 'stretch'; row?: number; column?: number; rowSpan?: number; columnSpan?: number; size?: { width?: { mode: 'fixed' | 'fit' | 'fill'; value?: number }; height?: { mode: 'fixed' | 'fit' | 'fill'; value?: number } } }
export type CustomPropertyType = 'string' | 'number' | 'boolean' | 'enum' | 'object' | 'array'
export interface CustomProperty { type: CustomPropertyType; value: unknown; options?: string[]; binding?: ValueRef; event?: UiAction }
export interface Widget { id: string; kind: string; name: string; x: number; y: number; width: number; height: number; zIndex: number; props: Record<string, any>; bindings: Record<string, ValueRef>; events: Record<string, UiAction>; styleTokens?: Record<string, string>; componentInstance?: ComponentInstanceMetadata; parentId?: string | null; slotId?: string; layout?: LayoutSpec; layoutItem?: LayoutItem; slots?: { id: string; title: LocalizedText }[]; customProperties?: Record<string, CustomProperty>; editor?: { locked?: boolean; hidden?: boolean } }
export type DesignTokenType = 'color' | 'dimension' | 'number' | 'boolean' | 'typography' | 'shadow' | 'fontFamily' | 'opacity' | 'radius'
export interface DesignToken { $type: DesignTokenType; $value: unknown; $description?: string }
export interface DesignTokenSet { id: string; name: string; tokens: Record<string, DesignToken> }
export interface DesignTheme { id: string; name: string; tokenSets: string[] }
export interface ComponentVariantSpec { axes: Record<string, string[]>; combinations: Record<string, Record<string, string>>; values?: Record<string, Record<string, Record<string, unknown>>> }
export interface DesignComponent { id: string; name: string; rootWidgetId: string; widgets: Widget[]; variants?: ComponentVariantSpec }
export interface DesignLibraryRef { id: string; name: string; version: string; namespace: string; assetIds?: string[] }
export interface ComponentInstanceMetadata { componentId: string; instanceId: string; rootWidgetId: string; widgetIdMap: Record<string, string>; variant?: Record<string, string>; overrides?: Record<string, Record<string, unknown>> }
export interface DesignSystem { version: 1; tokenSets: Record<string, DesignTokenSet>; themes: Record<string, DesignTheme>; components: Record<string, DesignComponent>; libraries: Record<string, DesignLibraryRef>; activeThemeId?: string }
export interface Page { id: string; name: string; widgets: Widget[]; layout?: LayoutSpec; responsive?: boolean; guides?: { id: string; axis: 'x' | 'y'; position: number }[] }
export interface Variable { id: string; name: string; type: 'number' | 'string' | 'boolean' | 'enum' | 'array' | 'object'; initial: unknown; options?: string[] }
export interface ConnectionConfig { id: string; name: string; port: string; baudRate: number; dataBits: number; stopBits: number; parity: 'none' | 'odd' | 'even'; protocolId: string; protocolVersion?: string; protocolConfig: Record<string, unknown> }
export interface FlowNode { id: string; kind: string; label: string; position: { x: number; y: number }; config: Record<string, any> }
export interface FlowEdge { id: string; source: string; sourcePort: string; target: string; targetPort: string }
export interface Flow { id: string; name: string; enabled: boolean; timeoutMs: number; scope?: 'window' | 'device'; nodes: FlowNode[]; edges: FlowEdge[] }
export interface ProjectAction { id: string; text: string | { i18nKey: string; fallback: string }; icon?: string; shortcut?: string; enabled?: boolean; visible?: boolean; action: UiAction }
export interface ProjectI18n { defaultLocale: string; locales: string[]; messages: Record<string, Record<string, string>> }
export interface BackendConfig { id: string; name: string; providerId: string; providerVersion: string; mode: 'simulation' | 'usb'; config: Record<string, unknown> }
export interface WindowTemplate { id: string; pageId: string; title: string; width: number; height: number; minWidth?: number; minHeight?: number; instanceKey?: string }
export interface CodeModule { id: string; name: string; source: string }
export interface TransportConfig { id: string; name: string; kind: 'usb-printer'; interfaceGuid: string; vendorId: number; productId: number; packetSize?: number }
export interface Project { schemaVersion: 1 | 2 | 3 | 4; id: string; name: string; width: number; height: number; pages: Page[]; connections: ConnectionConfig[]; variables: Variable[]; flows: Flow[]; designSystem?: DesignSystem; backends?: BackendConfig[]; windows?: WindowTemplate[]; codeModules?: CodeModule[]; transports?: TransportConfig[]; presentation?: { chrome: 'project' | 'host' }; python?: { runtimeId: string; runtimeSha256?: string; dependencyLock?: unknown[] }; assets?: { id: string; path: string }[]; i18n?: ProjectI18n; actions?: ProjectAction[] }
export interface ValidationIssue { severity: 'error' | 'warning'; path: string; message: string }
export interface RuntimeEvent { generation?: number; revision?: number; kind: string; timestamp: string | number; runId?: string; flowId?: string; nodeId?: string; data: any }
export interface Snapshot { variables: Record<string, any>; points: Record<string, any>; connections: Record<string, any>; flows: Record<string, any>; logs: unknown[]; ui?: Record<string, any>; inputs?: Record<string, unknown>; backends?: Record<string, any>; windowUi?: Record<string, Record<string, any>>; context?: Record<string, any>; runtime?: {state: string; error: string; locale: string} }
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
  { kind: 'trigger.backend', label: '后端事件', group: '触发', color: '#6f67c4' },
  { kind: 'backend.invoke', label: '调用后端', group: '设备', color: '#087f8c' },
  ...Object.entries({ enumerate:'枚举设备', open:'打开传输', close:'关闭传输', exchange:'收发事务', exchangeMany:'执行传输计划', recover:'恢复设备', cancel:'取消传输', completeEffect:'完成设备事务' }).map(([kind,label])=>({kind:`transport.${kind}`,label,group:'设备',color:'#087f8c'})),
  { kind: 'connection.open', label: '打开连接', group: '设备', color: '#087f8c' },
  { kind: 'connection.close', label: '关闭连接', group: '设备', color: '#087f8c' },
  { kind: 'protocol.command', label: '协议命令', group: '设备', color: '#087f8c' },
  { kind: 'condition', label: '条件分支', group: '逻辑', color: '#b77a18' },
  { kind: 'assign', label: '消息赋值', group: '逻辑', color: '#b77a18' },
  { kind: 'variable.get', label: '读取变量', group: '逻辑', color: '#b77a18' },
  { kind: 'variable.set', label: '写入变量', group: '逻辑', color: '#b77a18' },
  { kind: 'delay', label: '延时等待', group: '逻辑', color: '#b77a18' },
  { kind: 'flow.dispatch', label: '派发设备流程', group: '逻辑', color: '#b77a18' },
  { kind: 'flow.cancel', label: '取消设备流程', group: '逻辑', color: '#b77a18' },
  { kind: 'python', label: 'Python 代码', group: '代码', color: '#346faa' },
  { kind: 'ui.setProperty', label: '设置控件属性', group: '输出', color: '#49804b' },
  { kind: 'page.navigate', label: '切换页面', group: '输出', color: '#49804b' },
  { kind: 'window.open', label: '打开窗口', group: '输出', color: '#49804b' },
  { kind: 'window.focus', label: '聚焦窗口', group: '输出', color: '#49804b' },
  { kind: 'window.close', label: '关闭窗口', group: '输出', color: '#49804b' },
  { kind: 'file.open', label: '选择文件', group: '资源', color: '#087f8c' },
  { kind: 'file.save', label: '保存文件', group: '资源', color: '#087f8c' },
  ...Object.entries({ 'state.load':'加载状态', 'state.save':'保存状态', 'file.resolve':'解析文件资源', 'file.workspace':'创建工作目录', 'file.register':'登记输出文件', 'file.importCompanions':'导入伴随文件' }).map(([kind,label])=>({kind,label,group:'资源',color:'#087f8c'})),
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
    'trigger.backend': { backendId: '', event: 'snapshot' },
    'backend.invoke': { backendId: '', operation: '', args: {} },
    'window.open': { windowId: '', args: {} }, 'window.focus': { windowId: '', args: {} }, 'window.close': { args: {} },
    'file.open': { args: {} }, 'file.save': { args: {} },
    ...Object.fromEntries(['transport.enumerate','transport.open','transport.close','transport.exchange','transport.exchangeMany','transport.recover','transport.cancel','transport.completeEffect','state.load','state.save','file.resolve','file.workspace','file.register'].map(kind=>[kind,{args:{}}])),
    'file.importCompanions': { args: { resourceId: { source: 'literal', value: '' }, paths: { source: 'literal', value: [] } } },
    'flow.dispatch': { flowId: '', items: { source: 'message', path: 'payload' } },
    'flow.cancel': { flowId: '', context: { source: 'context', path: '' } },
    'connection.open': { connectionId: '' }, 'connection.close': { connectionId: '' },
    'protocol.command': { connectionId: '', command: '', params: {} },
    condition: { left: { source: 'message', path: 'payload' }, operator: 'gt', right: { source: 'literal', value: 0 } },
    assign: { path: 'payload', value: { source: 'literal', value: 0 } },
    'variable.get': { variableId: '', path: 'payload' }, 'variable.set': { variableId: '', value: { source: 'message', path: 'payload' } },
    delay: { durationMs: 1000 }, python: { code: 'def transform(msg):\n    # 在此处理输入消息\n    return msg\n', inputs: {}, timeoutMs: 5000, memoryMb: 256 },
    'ui.setProperty': { widgetId: '', property: 'visible', value: { source: 'literal', value: true } },
    'page.navigate': { pageId: '' }, log: { value: { source: 'message', path: 'payload' }, level: 'info' },
  }
  return { id: newId('node'), kind, label: nodeKinds.find(item => item.kind === kind)?.label ?? kind, position, config: clone(configs[kind] ?? {}) }
}
export function emptyProject(): Project {
  return { schemaVersion: 2, id: newId('project'), name: '未命名项目', width: 1280, height: 720, pages: [{ id: 'page_main', name: '主界面', widgets: [], responsive: true, layout: { type: 'absolute' }, guides: [] }], connections: [], variables: [], flows: [], assets: [], actions: [], i18n: { defaultLocale: 'zh-CN', locales: ['zh-CN','en-US'], messages: { 'zh-CN': {}, 'en-US': {} } } }
}
