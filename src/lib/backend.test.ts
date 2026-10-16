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
