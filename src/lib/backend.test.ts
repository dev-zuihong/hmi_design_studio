import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Project, RuntimeEvent, Snapshot } from './model'

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  isTauri: vi.fn(() => true),
  listen: vi.fn(),
  emit: vi.fn(),
  open: vi.fn(),
  save: vi.fn(),
  handlers: new Map<string, Array<(event: { payload: any }) => void>>(),
  unlisteners: new Map<string, ReturnType<typeof vi.fn>[]>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke, isTauri: mocks.isTauri }))
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen, emit: mocks.emit }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: mocks.open, save: mocks.save }))

import { useRuntime } from './backend'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}

function project(): Project {
  return {
    schemaVersion: 1, id: 'project-a', name: 'Preview test', width: 800, height: 600,
    pages: [{ id: 'page-a', name: 'A', widgets: [
      { id: 'input-a', kind: 'input', name: 'Input', x: 0, y: 0, width: 100, height: 30, zIndex: 1, props: { value: 'fallback' }, bindings: { value: { source: 'variable', key: 'temperature' } }, events: {} },
      { id: 'select-a', kind: 'select', name: 'Select', x: 0, y: 40, width: 100, height: 30, zIndex: 1, props: { value: 'first' }, bindings: { value: { source: 'literal', value: 'preset' } }, events: {} },
      { id: 'switch-a', kind: 'switch', name: 'Switch', x: 0, y: 80, width: 100, height: 30, zIndex: 1, props: { value: false }, bindings: {}, events: {} },
    ] }, { id: 'page-b', name: 'B', widgets: [] }],
    connections: [], variables: [{ id: 'temperature', name: 'Temperature', type: 'number', initial: 10 }], flows: [],
  }
}

function snapshot(version: string, logs: RuntimeEvent[] = []): Snapshot {
  return { variables: { version, temperature: 25 }, points: {}, connections: {}, flows: {}, logs }
}

function previewState(generation: number, currentPage: string, value: string) {
  return { running: true, generation, project: project(), currentPage, snapshot: snapshot(value, [
    { kind: 'page.changed', timestamp: generation, data: { pageId: currentPage } },
    { kind: 'flow.completed', timestamp: generation + 1, flowId: 'flow-a', data: { value } },
  ]) }
}

function fire(channel: string, payload: unknown, index = 0) {
  mocks.handlers.get(channel)?.[index]?.({ payload })
}

beforeEach(() => {
  vi.clearAllMocks()
  mocks.handlers.clear()
  mocks.unlisteners.clear()
  mocks.listen.mockImplementation(async (channel: string, handler: (event: { payload: any }) => void) => {
    const handlers = mocks.handlers.get(channel) ?? []
    handlers.push(handler)
    mocks.handlers.set(channel, handlers)
    const off = vi.fn()
    const unlisteners = mocks.unlisteners.get(channel) ?? []
    unlisteners.push(off)
    mocks.unlisteners.set(channel, unlisteners)
    return off
  })
  mocks.invoke.mockResolvedValue(undefined)
  mocks.emit.mockResolvedValue(undefined)
})

describe('preview runtime window synchronization', () => {
  it('retains uncommitted device ID edits but discards local overrides when the bound backend value changes', async () => {
    const fixture = project()
    fixture.pages[0]!.widgets[0]!.bindings.value = { source: 'backend', key: 'ptb', path: 'byDevice.$deviceKey.deviceId' }
    fixture.pages[0]!.widgets[1]!.bindings.value = { source: 'backend', key: 'ptb', path: 'byDevice.$deviceKey.draft.tvccVoltage' }
    let state = { ...snapshot('original'), backends: { ptb: { byDevice: { sim_1: { deviceId: 1, draft: { tvccVoltage: 3.3 } } } } } }
    mocks.invoke.mockImplementation(async (command: string) => command === 'get_runtime_window_state' ? {
      instance: { context: { windowInstanceId: 'window_sim_1', deviceKey: 'sim_1' }, pageId: 'page-a', windowId: 'settings', inputs: { 'input-a': 9, 'select-a': 2.4 } }, project: fixture, snapshot: state, generation: 4, revision: 1,
    } : command === 'runtime_snapshot' ? state : undefined)
    const runtime = useRuntime({ runtimeWindow: true })
    await runtime.connectWindow()
    await runtime.updateInput('input-a', 9)
    await runtime.updateInput('select-a', 2.4)
    await runtime.refresh()
    expect(runtime.inputs.value).toMatchObject({ 'input-a': 9, 'select-a': 2.4 })
    state = { ...snapshot('imported'), backends: { ptb: { byDevice: { sim_1: { deviceId: 1, draft: { tvccVoltage: 2.9 } } } } } }
    await runtime.refresh()
    expect(runtime.inputs.value['input-a']).toBe(9)
    expect(runtime.inputs.value).not.toHaveProperty('select-a')
    // Rehydrating a window must not resurrect the server's older input overlay.
    await runtime.connectWindow()
    expect(runtime.inputs.value).not.toHaveProperty('select-a')
    await runtime.act({ kind: 'flow', flowId: 'save' })
    expect(mocks.invoke).toHaveBeenCalledWith('trigger_flow', expect.objectContaining({ inputs: { 'input-a': 9, 'select-a': 2.9 } }))
    runtime.dispose()
  })
  it('hot updates a device window without hydrating main inputs or losing edits made during the update', async () => {
    const initialProject = project()
    initialProject.windows = [{ id: 'settings', pageId: 'page-a', title: 'Settings', width: 640, height: 480 }]
    const instance = { context: { windowInstanceId: 'window_sim_1', deviceKey: 'sim_1', backendId: 'ptb' }, pageId: 'page-a', windowId: 'settings', inputs: { 'input-a': 'device draft' } }
    const initialState = { instance, project: initialProject, snapshot: { ...snapshot('initial'), inputs: { 'input-a': 'main input' } }, generation: 4, revision: 1 }
    const pending = deferred<typeof initialState>()
    let requests = 0
    mocks.invoke.mockImplementation((command: string) => command === 'get_runtime_window_state'
      ? ++requests === 1 ? Promise.resolve(initialState) : pending.promise
      : Promise.resolve(undefined))
    const runtime = useRuntime({ runtimeWindow: true })
    await runtime.connectWindow()
    expect(runtime.inputs.value['input-a']).toBe('device draft')

    fire('preview-input-changed', { generation: 4, id: 'input-a', value: 'another window' })
    fire('preview-state-changed', { running: true, generation: 4, revision: 2 })
    await vi.waitFor(() => expect(requests).toBe(2))
    await runtime.updateInput('input-a', 'typed during update')
    const nextProject = structuredClone(initialProject)
    nextProject.name = 'Updated device UI'
    nextProject.windows![0]!.width = 900
    pending.resolve({ ...initialState, project: nextProject, snapshot: { ...snapshot('updated'), inputs: { 'input-a': 'unrelated main input' } }, revision: 2 })
    await vi.waitFor(() => expect(runtime.previewProject.value?.name).toBe('Updated device UI'))
    expect(runtime.windowSize.value).toEqual({ width: 900, height: 480 })
    expect(runtime.snapshot.value.variables.version).toBe('updated')
    expect(runtime.context.value).toEqual(instance.context)
    expect(runtime.inputs.value['input-a']).toBe('typed during update')
    expect(mocks.invoke.mock.calls.some(([command]) => command === 'get_preview_state')).toBe(false)
    fire('preview-state-changed', { running: true, generation: 4, revision: 1 })
    expect(requests).toBe(2)
    runtime.dispose()
    expect(mocks.unlisteners.get('preview-state-changed')?.[0]).toHaveBeenCalledOnce()
  })
  it('keeps device-window drafts and action contexts isolated without starting another session', async () => {
    let deviceKey='sim_1'
    mocks.invoke.mockImplementation(async(command:string)=> command==='get_runtime_window_state'?{
      instance:{context:{windowInstanceId:`window_${deviceKey}`,deviceKey,backendId:'ptb'},pageId:'page-a',windowId:'settings',inputs:{}},
      project:project(),snapshot:snapshot('window'),revision:1,
    }:command==='runtime_snapshot'?snapshot('window'):undefined)
    const first=useRuntime({runtimeWindow:true});await first.connectWindow()
    deviceKey='sim_2'
    const second=useRuntime({runtimeWindow:true});await second.connectWindow()
    await first.updateInput('input-a','device one')
    expect(second.inputs.value['input-a']).toBeUndefined()
    await first.act({kind:'flow',flowId:'edit',entryNodeId:'start'},42)
    expect(mocks.invoke).toHaveBeenCalledWith('trigger_flow',expect.objectContaining({message:{payload:42,context:{windowInstanceId:'window_sim_1',deviceKey:'sim_1',backendId:'ptb'}},inputs:{'input-a':'device one'}}))
    expect(mocks.invoke.mock.calls.some(([command])=>command==='start_preview'||command==='update_preview_input')).toBe(false)
    fire('runtime-event',{kind:'page.changed',data:{pageId:'page-b',windowInstanceId:'window_sim_1'},timestamp:1})
    expect(first.currentPage.value).toBe('page-b');expect(second.currentPage.value).toBe('page-a')
    first.dispose();second.dispose()
  })
  it('attaches to the active preview and hydrates its page, history, snapshot, and bound inputs', async () => {
    const state = previewState(7, 'page-b', 'saved')
    mocks.invoke.mockImplementation(async (command: string) => command === 'get_preview_state' ? state : command === 'runtime_snapshot' ? state.snapshot : undefined)
    const runtime = useRuntime({ previewWindow: true })

    await runtime.connectPreview()

    expect(mocks.invoke).toHaveBeenCalledWith('get_preview_state', undefined)
    expect(mocks.invoke).not.toHaveBeenCalledWith('start_preview', expect.anything())
    expect(runtime.running.value).toBe(true)
    expect(runtime.generation.value).toBe(7)
    expect(runtime.currentPage.value).toBe('page-b')
    expect(runtime.events.value).toEqual(state.snapshot.logs)
    expect(runtime.snapshot.value.variables.version).toBe('saved')
    expect(runtime.inputs.value).toEqual({ 'input-a': 25, 'select-a': 'preset', 'switch-a': false })
    fire('runtime-event', { data: { pageId: 'page-b' }, generation: 7, timestamp: 7, kind: 'page.changed' })
    expect(runtime.events.value).toEqual(state.snapshot.logs)
  })

  it('ignores an old snapshot request that resolves after stop and a newer session attach', async () => {
    const oldSnapshot = deferred<Snapshot>()
    let stateRequests = 0
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_preview_state') return Promise.resolve(++stateRequests === 1
        ? previewState(1, 'page-a', 'initial')
        : previewState(3, 'page-b', 'new-session'))
      if (command === 'runtime_snapshot') return oldSnapshot.promise
      return Promise.resolve(undefined)
    })
    const runtime = useRuntime()
    await runtime.connectPreview()

    fire('runtime-event', { kind: 'flow.changed', timestamp: 2, data: {} })
    await vi.waitFor(() => expect(mocks.invoke).toHaveBeenCalledWith('runtime_snapshot', { expectedGeneration: 1 }))
    fire('preview-state-changed', { running: false, generation: 2 })
    expect(runtime.running.value).toBe(false)
    fire('preview-state-changed', { running: true, generation: 3 })
    await vi.waitFor(() => expect(runtime.snapshot.value.variables.version).toBe('new-session'))

    oldSnapshot.resolve(snapshot('stale-session'))
    await oldSnapshot.promise
    await vi.waitFor(() => expect(runtime.running.value).toBe(true))
    expect(runtime.generation.value).toBe(3)
    expect(runtime.snapshot.value.variables.version).toBe('new-session')
    expect(runtime.currentPage.value).toBe('page-b')
  })

  it('preserves page and input updates received while a new generation is hydrating', async () => {
    const pendingState = deferred<ReturnType<typeof previewState>>()
    let stateRequests = 0
    mocks.invoke.mockImplementation((command: string) => {
      if (command === 'get_preview_state') {
        stateRequests++
        return stateRequests === 1 ? Promise.resolve(previewState(21, 'page-a', 'current')) : pendingState.promise
      }
      if (command === 'runtime_snapshot') return Promise.resolve(snapshot('live-refresh'))
      return Promise.resolve(undefined)
    })
    const runtime = useRuntime()
    await runtime.connectPreview()

    fire('preview-state-changed', { running: true, generation: 22 })
    await vi.waitFor(() => expect(stateRequests).toBe(2))
    fire('runtime-event', {
      kind: 'page.changed', timestamp: 23, generation: 22, data: { pageId: 'page-b' },
    })
    fire('preview-input-changed', { generation: 22, id: 'input-a', value: 'typed-during-hydration' })

    pendingState.resolve(previewState(22, 'page-a', 'stale-hydration'))
    await vi.waitFor(() => expect(runtime.snapshot.value.variables.version).toBe('stale-hydration'))

    expect(runtime.currentPage.value).toBe('page-b')
    expect(runtime.inputs.value['input-a']).toBe('typed-during-hydration')
  })

  it('unlistens if dispose happens before the runtime event listener resolves', async () => {
    const pendingListen = deferred<() => void>()
    const off = vi.fn()
    mocks.listen.mockImplementation((channel: string) => channel === 'runtime-event' ? pendingListen.promise : Promise.resolve(vi.fn()))
    const runtime = useRuntime()
    const subscribe = runtime.subscribe()

    runtime.dispose()
    pendingListen.resolve(off)
    await subscribe

    expect(off).toHaveBeenCalledOnce()
  })

  it('broadcasts preview input updates, guards Studio actions by generation, and leaves Runner actions unversioned', async () => {
    mocks.invoke.mockImplementation(async (command: string) => command === 'get_preview_state' ? previewState(12, 'page-a', 'shared') : snapshot('shared'))
    const main = useRuntime()
    const preview = useRuntime({ previewWindow: true })
    await main.connectPreview()
    await preview.connectPreview()

    await preview.updateInput('input-a', 'latest-value')
    expect(mocks.emit).toHaveBeenCalledWith('preview-input-changed', { generation: 12, id: 'input-a', value: 'latest-value' })
    fire('preview-input-changed', { generation: 12, id: 'input-a', value: 'older-echo' }, 1)
    expect(preview.inputs.value['input-a']).toBe('latest-value')
    fire('preview-input-changed', { generation: 12, id: 'input-a', value: 'latest-value' }, 0)
    expect(main.inputs.value['input-a']).toBe('latest-value')

    await main.act({ kind: 'flow', flowId: 'flow-a', entryNodeId: 'entry-a' })
    expect(mocks.invoke).toHaveBeenCalledWith('trigger_flow', {
      flowId: 'flow-a', entryNodeId: 'entry-a', message: { payload: {} }, inputs: { 'input-a': 'latest-value', 'select-a': 'preset', 'switch-a': false }, expectedGeneration: 12,
    })

    const runner = useRuntime()
    await runner.start(project())
    await runner.act({ kind: 'flow', flowId: 'runner-flow', entryNodeId: 'runner-entry' })
    expect(mocks.invoke.mock.calls.find(([command, args]) => command === 'trigger_flow' && args.flowId === 'runner-flow')?.[1]).toEqual({
      flowId: 'runner-flow', entryNodeId: 'runner-entry', message: { payload: {} }, inputs: { 'input-a': 10, 'select-a': 'preset', 'switch-a': false },
    })
  })
})
