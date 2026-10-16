import { describe, expect, it } from 'vitest'
import { ref } from 'vue'
import { checkConnection, parseProject, useEditor } from './editor'
import { createNode, emptyProject, type Flow } from './model'

describe('project editing', () => {
  it('blocks project mutations while readonly and resumes editing when unlocked', () => {
    const readOnly = ref(false), editor = useEditor(emptyProject(), readOnly)
    editor.addWidget('button', 40, 50)
    editor.addWidget('number', 100, 120)
    const [first, second] = editor.page.value.widgets
    editor.selected.value = [first!.id, second!.id]
    editor.updateWidget(first!.id, { x: 72 })
    const before = JSON.stringify(editor.project.value), past = [...editor.past.value], future = [...editor.future.value]

    readOnly.value = true
    expect(editor.readOnly).toBe(readOnly)
    expect(editor.mutate(project => { project.name = 'blocked' })).toBe(false)
    expect(editor.checkpoint()).toBe(false)
    expect(editor.undo()).toBe(false)
    expect(editor.redo()).toBe(false)
    editor.addWidget('text')
    editor.updateWidget(first!.id, { x: 900 })
    editor.removeSelected()
    editor.duplicateSelected()
    editor.align('left')
    expect(editor.replace(emptyProject())).toBe(false)
    expect(JSON.stringify(editor.project.value)).toBe(before)
    expect(editor.past.value).toEqual(past)
    expect(editor.future.value).toEqual(future)

    readOnly.value = false
    editor.updateWidget(first!.id, { x: 88 })
    expect(first!.x).toBe(88)
    expect(editor.undo()).toBe(true)
    expect(editor.page.value.widgets.find(widget => widget.id === first!.id)?.x).toBe(72)
    expect(editor.redo()).toBe(true)
    expect(editor.page.value.widgets.find(widget => widget.id === first!.id)?.x).toBe(88)
  })

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
    expect(() => parseProject('{')).toThrow(); expect(() => parseProject(JSON.stringify({ ...emptyProject(), schemaVersion: 99 }))).toThrow('版本')
    expect(() => parseProject(JSON.stringify({ ...emptyProject(), pages: [] }))).toThrow('页面')
    expect(parseProject(JSON.stringify(emptyProject())).schemaVersion).toBe(2)
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
