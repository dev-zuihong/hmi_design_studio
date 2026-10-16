import { invoke, isTauri } from '@tauri-apps/api/core'
import { emit, listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open, save } from '@tauri-apps/plugin-dialog'
import { ref } from 'vue'
import type { Environment, Project, RuntimeEvent, Snapshot, UiAction, ValidationIssue } from './model'
import { applyEnvironment, platformLabel } from './platform'
import { downloadProjectPackage } from './project-package'
import { projectLocale } from './i18n'
export const desktop = isTauri()
export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) return Promise.reject(new Error('请在桌面版中执行此操作；浏览器模式支持项目设计与保存。'))
  return invoke<T>(command, args)
}
export async function pickFile(kind: 'project' | 'plugin' | 'directory' | 'asset'): Promise<string | undefined> {
  const result = await open({ directory: kind === 'directory', multiple: false, filters: kind === 'project' ? [{ name: 'HMI 工程包', extensions: ['hmiproj', 'json'] }] : kind === 'asset' ? [{ name: '图片和字体', extensions: ['png', 'jpg', 'jpeg', 'webp', 'gif', 'woff', 'woff2', 'ttf', 'otf'] }] : undefined })
  return typeof result === 'string' ? result : undefined
}
export async function pickSave(name: string, zip = false) { return (await save({ defaultPath: `${name}.${zip ? 'zip' : 'hmiproj'}`, filters: [{ name: zip ? `${platformLabel.value} 运行包` : 'HMI 工程包', extensions: [zip ? 'zip' : 'hmiproj'] }] })) ?? undefined }
export const downloadProject = downloadProjectPackage
export const blankSnapshot = (): Snapshot => ({ variables: {}, points: {}, connections: {}, flows: {}, logs: [] })
export interface PreviewState {
  project: Project | null
  running: boolean
  currentPage: string | null
  generation: number
  snapshot?: Snapshot
  revision?: number
  status?: string
  updateStatus?: string
  pid?: number
  locale?: string
}
export function useRuntime(options: { previewWindow?: boolean } = {}) {
  const running = ref(false), busy = ref(false), snapshot = ref<Snapshot>(blankSnapshot()), events = ref<RuntimeEvent[]>([]), inputs = ref<Record<string, unknown>>({}), currentPage = ref('')
  const previewProject = ref<Project>(), generation = ref(-1), revision = ref(0), updateStatus = ref('applied'), pid = ref<number>()
  let unlistenLocale: UnlistenFn | undefined
  let unlisten: UnlistenFn | undefined, unlistenState: UnlistenFn | undefined, unlistenInputs: UnlistenFn | undefined
  let subscribing: Promise<void> | undefined, subscribingState: Promise<void> | undefined, subscribingInputs: Promise<void> | undefined
  let refreshPending = false, refreshAgain = false, disposed = false, epoch = 0, lifecycleRevision = 0, hydratedGeneration = -1
  let stateSync: Promise<void> | undefined, stateAgain = false
  const replayPending = new Map<string, number>()
  const liveInputs = new Map<string, { generation: number; value: unknown }>()
  let livePage: { generation: number; pageId: string } | undefined
  const eventKey = (event: RuntimeEvent) => {
    const body = { ...event }; delete body.generation
    return JSON.stringify(body, (_key, value) => value && typeof value === 'object' && !Array.isArray(value)
      ? Object.fromEntries(Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)) : value)
  }
  const expectedGeneration = () => generation.value >= 0 ? { expectedGeneration: generation.value } : {}
  function initializeInputs(project: Project, live?: Snapshot) {
    inputs.value = {}
    for (const widget of project.pages.flatMap(page => page.widgets).filter(widget => ['input','textarea','select','switch','checkbox','radio','spinbox','slider','date','time','datetime','table','list','tree','tabs','stack'].includes(widget.kind))) {
      const binding = widget.bindings.value
      if (binding?.source === 'variable') inputs.value[widget.id] = live?.variables[binding.key] ?? project.variables.find(variable => variable.id === binding.key)?.initial
      else if (binding?.source === 'literal') inputs.value[widget.id] = binding.value
      else if (binding?.source === 'point') {
        const point = live?.points[binding.key]
        inputs.value[widget.id] = point && typeof point === 'object' && 'value' in point ? (point.valid === false ? undefined : point.value) : point
      } else inputs.value[widget.id] = widget.props.value ?? (widget.kind === 'switch' ? false : '')
      if (live?.inputs && widget.id in live.inputs) inputs.value[widget.id] = live.inputs[widget.id]
    }
  }
  function stopped() {
    epoch++; running.value = false; previewProject.value = undefined; snapshot.value = blankSnapshot(); replayPending.clear()
    for (const [id, input] of liveInputs) if (generation.value < 0 || input.generation <= generation.value) liveInputs.delete(id)
    if (generation.value < 0 || livePage && livePage.generation <= generation.value) livePage = undefined
  }
  function hydrateEvents(history: unknown[]) {
    const backlog = history.filter((event): event is RuntimeEvent => !!event && typeof event === 'object' && 'kind' in event && 'timestamp' in event && 'data' in event)
    replayPending.clear()
    for (const event of backlog) { const key = eventKey(event); replayPending.set(key, (replayPending.get(key) ?? 0) + 1) }
    const extra = events.value.filter(event => {
      const key = eventKey(event), remaining = replayPending.get(key) ?? 0
      if (!remaining) return true
      replayPending.set(key, remaining - 1)
      return false
    })
    events.value = [...backlog, ...extra].slice(-1500)
  }
  function applyPreviewState(state: PreviewState) {
    if (state.generation < generation.value) return
    generation.value = state.generation
    revision.value = state.revision ?? revision.value
    updateStatus.value = state.updateStatus ?? state.status ?? 'applied'
    pid.value = state.pid
    if (state.locale) projectLocale.value = state.locale
    if (!state.running || !state.project) { stopped(); return }
    if (hydratedGeneration !== state.generation) {
      epoch++
      initializeInputs(state.project, state.snapshot)
      for (const [id, input] of liveInputs) if (input.generation === state.generation) inputs.value[id] = input.value
      currentPage.value = livePage?.generation === state.generation ? livePage.pageId : state.currentPage ?? state.project.pages[0]?.id ?? ''
      if (state.snapshot) hydrateEvents(state.snapshot.logs)
      hydratedGeneration = state.generation
    }
    if (previewProject.value && hydratedGeneration === state.generation) {
      const previous = previewProject.value; const oldInputs = { ...inputs.value }; initializeInputs(state.project, state.snapshot)
      for (const widget of state.project.pages.flatMap(page => page.widgets)) { const old = previous.pages.flatMap(page => page.widgets).find(candidate => candidate.id === widget.id); if (old && old.kind === widget.kind && JSON.stringify(old.bindings) === JSON.stringify(widget.bindings) && widget.id in oldInputs && !(state.snapshot?.inputs && widget.id in state.snapshot.inputs)) inputs.value[widget.id] = oldInputs[widget.id] }
    }
    for (const [id,input] of liveInputs) if (input.generation === state.generation) inputs.value[id] = input.value
    previewProject.value = state.project
    running.value = true
    if (state.snapshot) snapshot.value = state.snapshot
  }
  async function refresh() {
    if (!running.value || disposed) return
    if (refreshPending) { refreshAgain = true; return }
    refreshPending = true
    try {
      do {
        refreshAgain = false
        const requestedEpoch = epoch, next = await call<Snapshot>('runtime_snapshot', expectedGeneration())
        if (running.value && !disposed && requestedEpoch === epoch) snapshot.value = next
      } while (refreshAgain && running.value && !disposed)
    } finally { refreshPending = false }
  }
  async function subscribe() {
    if (!desktop || unlisten || disposed) return
    if (subscribing) return subscribing
    subscribing = (async () => {
      const off = await listen<RuntimeEvent>('runtime-event', event => {
        if (disposed) return
        const eventGeneration = event.payload.generation ?? generation.value
        if (generation.value >= 0 && eventGeneration < generation.value) return
        if (event.payload.kind === 'page.changed') livePage = { generation: eventGeneration, pageId: event.payload.data.pageId ?? event.payload.data.id ?? '' }
        if (generation.value >= 0 && eventGeneration > generation.value) return
        const key = eventKey(event.payload), replay = replayPending.get(key) ?? 0
        if (replay) replayPending.set(key, replay - 1)
        else { events.value.push(event.payload); if (events.value.length > 1500) events.value.splice(0, events.value.length - 1500) }
        if (event.payload.kind === 'page.changed') {
          currentPage.value = event.payload.data.pageId ?? event.payload.data.id ?? ''
        }
        void refresh().catch(() => {})
      })
      if (disposed) off(); else unlisten = off
    })().finally(() => subscribing = undefined)
    return subscribing
  }
  function syncPreviewState(): Promise<void> {
    if (stateSync) { stateAgain = true; return stateSync }
    stateSync = (async () => {
      do {
        stateAgain = false
        const revision = lifecycleRevision, state = await call<PreviewState | null>('get_preview_state')
        if (!disposed && revision === lifecycleRevision) {
          if (state) applyPreviewState(state)
          else stopped()
        }
      } while (stateAgain && !disposed)
    })().finally(() => stateSync = undefined)
    return stateSync
  }
  async function connectPreview() {
    if (!desktop || disposed) return
    await subscribe()
    if (!unlistenState) {
      subscribingState ??= (async () => {
        const off = await listen<{ running: boolean; generation: number; revision?: number; locale?: string; status?: string }>('preview-state-changed', event => {
          if (disposed || event.payload.generation < generation.value) return
          if (event.payload.revision !== undefined) revision.value = event.payload.revision
          if (event.payload.locale) projectLocale.value = event.payload.locale
          if (event.payload.status) updateStatus.value = event.payload.status
          lifecycleRevision++
          generation.value = event.payload.generation
          if (!event.payload.running) stopped()
          else { running.value = true; void syncPreviewState().catch(() => {}) }
        })
        if (disposed) off(); else unlistenState = off
      })().finally(() => subscribingState = undefined)
      await subscribingState
    }
    if (!unlistenInputs) {
      subscribingInputs ??= (async () => {
        const off = await listen<{ generation: number; id: string; value: unknown }>('preview-input-changed', event => {
          if (options.previewWindow) return
          if (!disposed && event.payload.generation >= generation.value) {
            if (generation.value < 0 || event.payload.generation === generation.value) inputs.value[event.payload.id] = event.payload.value
            liveInputs.set(event.payload.id, event.payload)
          }
        })
        if (disposed) off(); else unlistenInputs = off
      })().finally(() => subscribingInputs = undefined)
      await subscribingInputs
    }
    if (!unlistenLocale) unlistenLocale = await listen<{locale:string}>('preview-locale-changed', event => { projectLocale.value = event.payload.locale })
    await syncPreviewState()
  }
  async function updateInput(id: string, value: unknown) {
    inputs.value[id] = value
    liveInputs.set(id, { generation: generation.value, value })
    if (running.value && desktop) await call('update_preview_input', { id, value, ...expectedGeneration() })
    if (options.previewWindow && running.value) await emit('preview-input-changed', { generation: generation.value, id, value })
  }
  async function openPreview(project: Project) {
    busy.value = true
    try {
      await connectPreview()
      if (!running.value) { events.value = []; hydratedGeneration = -1; replayPending.clear() }
      await call('open_preview_window', { project })
      running.value = true
      await syncPreviewState()
    } finally { busy.value = false }
  }
  async function updatePreview(project: Project, nextRevision: number) {
    updateStatus.value = 'validating'
    const result = await call<{status:string;revision:number;issues:ValidationIssue[]}>('apply_preview_update', {project,revision:nextRevision,...expectedGeneration()})
    updateStatus.value = result.status
    if(result.status==='applied'||result.status==='stale'){revision.value=result.revision;await syncPreviewState()}
    return result
  }
  async function start(project: Project) {
    busy.value = true
    try {
      await subscribe(); epoch++; events.value = []; replayPending.clear(); initializeInputs(project)
      currentPage.value = project.pages[0]?.id ?? ''; await call('start_preview', { project }); running.value = true; await refresh()
    } finally { busy.value = false }
  }
  async function stop() { busy.value = true; try { await call('stop_preview', expectedGeneration()); lifecycleRevision++; stopped() } finally { busy.value = false } }
  async function act(action: UiAction, value?: unknown) {
    if (!running.value) throw new Error('请先启动运行会话')
    if (action.kind === 'startFlow') await call('start_flow', { flowId: action.flowId, ...expectedGeneration() })
    else if (action.kind === 'stopFlow') await call('stop_flow', { flowId: action.flowId, ...expectedGeneration() })
    else await call('trigger_flow', { flowId: action.flowId, entryNodeId: action.entryNodeId, message: { payload: value ?? {} }, inputs: { ...inputs.value }, ...expectedGeneration() })
    await refresh()
  }
  function dispose() { unlistenLocale?.(); disposed = true; epoch++; unlisten?.(); unlisten = undefined; unlistenState?.(); unlistenState = undefined; unlistenInputs?.(); unlistenInputs = undefined }
  return { running, busy, snapshot, events, inputs, currentPage, previewProject, generation, revision, updateStatus, pid, start, stop, act, refresh, subscribe, connectPreview, openPreview, updatePreview, updateInput, dispose }
}
export type Runtime = ReturnType<typeof useRuntime>
export const getEnvironment = async () => { const environment = await call<Environment>('get_environment'); applyEnvironment(environment); return environment }
export const validate = (project: Project) => call<ValidationIssue[]>('validate_project', { project })
