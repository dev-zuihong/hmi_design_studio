import { clone, createNode, newId, type Project, type ValueRef } from './model'
export interface QuickAction { kind: 'connection.open' | 'connection.close' | 'protocol.command'; connectionId: string; command?: string; params?: Record<string, ValueRef> }
export function addQuickAction(project: Project, widgetId: string, action: QuickAction): string {
  const widget = project.pages.flatMap(page => page.widgets).find(widget => widget.id === widgetId)
  if (!widget) throw new Error('目标控件不存在')
  if (!project.connections.some(connection => connection.id === action.connectionId)) throw new Error('请选择一个有效连接资源')
  if (action.kind === 'protocol.command' && !action.command) throw new Error('请选择协议命令')
  const trigger = createNode('trigger.ui', { x: 80, y: 120 }), operation = createNode(action.kind, { x: 360, y: 120 })
  operation.config = { connectionId: action.connectionId, ...(action.kind === 'protocol.command' ? { command: action.command, params: clone(action.params ?? {}) } : {}) }
  const id = newId('flow')
  project.flows.push({ id, name: `${widget.name} · ${action.kind === 'connection.open' ? '打开连接' : action.kind === 'connection.close' ? '关闭连接' : action.command}`, enabled: true, timeoutMs: 30000, nodes: [trigger, operation], edges: [{ id: newId('edge'), source: trigger.id, sourcePort: 'out', target: operation.id, targetPort: 'in' }] })
  widget.events[widget.kind === 'button' ? 'click' : 'change'] = { kind: 'flow', flowId: id, entryNodeId: trigger.id }
  return id
}
