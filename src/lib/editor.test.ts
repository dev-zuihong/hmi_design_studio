import { describe, expect, it } from 'vitest'
import { checkConnection, parseProject, useEditor } from './editor'
import { createNode, emptyProject, type Flow } from './model'

describe('project editing', () => {
  it('restores widget geometry and selection safely through undo/redo', () => {
    const editor = useEditor(); editor.addWidget('button', 40, 50); const id = editor.selected.value[0]!
    editor.updateWidget(id, { x: 240 }); expect(editor.page.value.widgets[0]!.x).toBe(240)
    editor.undo(); expect(editor.page.value.widgets[0]!.x).toBe(40)
    editor.redo(); expect(editor.page.value.widgets[0]!.x).toBe(240)
    editor.removeSelected(); expect(editor.selected.value).toEqual([]); editor.undo(); expect(editor.page.value.widgets).toHaveLength(1)
  })
  it('duplicates independently and keeps bindings', () => {
    const editor = useEditor(); editor.addWidget('number'); const first = editor.page.value.widgets[0]!
    first.bindings.value = { source: 'variable', key: 'temperature' }; editor.duplicateSelected()
    const second = editor.page.value.widgets[1]!; expect(second.id).not.toBe(first.id); expect(second.bindings).toEqual(first.bindings)
    second.props.text = 'changed'; expect(first.props.text).not.toBe('changed')
  })
  it('refuses malformed or unsupported files before replacing the document', () => {
    expect(() => parseProject('{')).toThrow(); expect(() => parseProject(JSON.stringify({ ...emptyProject(), schemaVersion: 2 }))).toThrow('版本')
    expect(() => parseProject(JSON.stringify({ ...emptyProject(), pages: [] }))).toThrow('页面')
    expect(parseProject(JSON.stringify(emptyProject())).schemaVersion).toBe(1)
  })
})
describe('flow connections', () => {
  const a = createNode('trigger.ui'), b = createNode('assign'), c = createNode('log')
  const flow: Flow = { id: 'f', name: 'test', enabled: true, timeoutMs: 30000, nodes: [a,b,c], edges: [{ id: '1', source: a.id, target: b.id, sourcePort: 'out', targetPort: 'in' }, { id: '2', source: b.id, target: c.id, sourcePort: 'out', targetPort: 'in' }] }
  it('rejects cyclic graphs, trigger inputs, and duplicate edges', () => {
    expect(checkConnection(flow, { source: c.id, target: b.id, sourcePort: 'out', targetPort: 'in' })).toContain('回环')
    expect(checkConnection(flow, { source: c.id, target: a.id, sourcePort: 'out', targetPort: 'in' })).toContain('触发')
    expect(checkConnection(flow, flow.edges[0]!)).toContain('已经存在')
  })
  it('allows multiple incoming branches without inventing a join', () => { expect(checkConnection(flow, { source: a.id, target: c.id, sourcePort: 'out', targetPort: 'in' })).toBeUndefined() })
})
