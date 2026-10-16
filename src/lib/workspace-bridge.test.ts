import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { createWidget, emptyProject } from './model'
import { useEditor } from './editor'

const mocks = vi.hoisted(() => ({ call: vi.fn(), listen: vi.fn(), handlers: new Map<string, (event: {payload: any}) => void>() }))
vi.mock('./backend', () => ({ desktop: true, call: mocks.call }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }))

import { connectWorkspace, MemoryWorkspace, workspaceReady, workspaceRevision } from './workspace'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>(yes => { resolve = yes })
  return { promise, resolve }
}
const disposers: (() => void)[] = []
beforeEach(() => {
  vi.clearAllMocks(); mocks.handlers.clear(); workspaceReady.value = false; workspaceRevision.value = 0
  vi.stubGlobal('window', new EventTarget()); vi.stubGlobal('BroadcastChannel', undefined)
  mocks.listen.mockImplementation(async (name, handler) => { mocks.handlers.set(name, handler); return () => mocks.handlers.delete(name) })
})
afterEach(() => { disposers.splice(0).forEach(dispose => dispose()); vi.unstubAllGlobals() })

async function setup() {
  const project = emptyProject(); project.pages[0]!.widgets.push(createWidget('button', 80, 90))
  let authority = new MemoryWorkspace(project)
  const pending = deferred<ReturnType<MemoryWorkspace['state']>>()
  let firstState: ReturnType<MemoryWorkspace['state']> | undefined, applications = 0
  mocks.call.mockImplementation(async (command, args) => {
    if (command === 'apply_workspace_operations') {
      const state = authority.apply('main', args.operations)
      if (++applications === 1) { firstState = state; return pending.promise }
      return state
    }
    if (command === 'undo_workspace') return authority.undo('main')
    return authority.state()
  })
  const editor = useEditor(project), errors = vi.fn(), bridge = connectWorkspace(editor, errors)
  disposers.push(bridge.dispose); await bridge.init()
  return { editor, bridge, errors, get authority() { return authority }, get applications() { return applications },
    acknowledge() { pending.resolve(firstState!) },
    replaceFromOtherWindow() { authority = new MemoryWorkspace(emptyProject()); mocks.handlers.get('workspace-changed')!({payload: authority.state()}); return authority },
  }
}

describe('workspace bridge while commands await acknowledgments', () => {
  it('submits input entered after a pending command without dropping or duplicating it', async () => {
    const context = await setup(), { editor, bridge } = context
    editor.project.value.name = 'First input'; const flushing = bridge.flush()
    await vi.waitFor(() => expect(context.applications).toBe(1))
    editor.project.value.width = 960
    context.acknowledge(); await flushing
    expect(context.authority.project.name).toBe('First input'); expect(context.authority.project.width).toBe(960)
    expect(context.applications).toBe(2); expect(context.errors).not.toHaveBeenCalled()
    editor.undo(); await vi.waitFor(() => expect(editor.project.value.width).toBe(1280))
    expect(editor.project.value.name).toBe('First input')
  })

  it('keeps a drag as one undo transaction while a prior edit is acknowledged', async () => {
    const context = await setup(), { editor, bridge } = context
    editor.project.value.name = 'Before dragging'; const flushing = bridge.flush()
    await vi.waitFor(() => expect(context.applications).toBe(1))
    editor.selected.value = [editor.page.value.widgets[0]!.id]; editor.gesturing.value = true
    editor.page.value.widgets[0]!.x = 140; context.acknowledge()
    await vi.waitFor(() => expect(bridge.snapshot.project.name).toBe('Before dragging'))
    expect(editor.page.value.widgets[0]!.x).toBe(140)
    editor.page.value.widgets[0]!.x = 260; editor.project.value.height = 800
    editor.gesturing.value = false; await flushing; await bridge.flush()
    expect(context.authority.project.pages[0]!.widgets[0]!.x).toBe(260)
    expect(context.authority.project.height).toBe(800); expect(context.errors).not.toHaveBeenCalled()
    editor.undo(); await vi.waitFor(() => expect(editor.page.value.widgets[0]!.x).toBe(80))
    expect(editor.project.value.height).toBe(800); expect(editor.project.value.name).toBe('Before dragging')
  })

  it('ignores the old project acknowledgment after another window replaces the workspace', async () => {
    const context = await setup(), { editor, bridge } = context
    editor.project.value.name = 'Old workspace edit'; const flushing = bridge.flush()
    await vi.waitFor(() => expect(context.applications).toBe(1))
    const replacement = context.replaceFromOtherWindow()
    editor.project.value.name = 'New workspace edit'
    context.acknowledge(); await flushing; await bridge.flush()
    expect(bridge.snapshot.project.id).toBe(replacement.project.id)
    expect(editor.project.value.name).toBe('New workspace edit'); expect(replacement.project.name).toBe('New workspace edit')
    expect(context.errors).not.toHaveBeenCalled()
  })
})
