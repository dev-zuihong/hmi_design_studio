import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { open, save } from '@tauri-apps/plugin-dialog'
import { ref } from 'vue'
import type { Environment, Project, RuntimeEvent, Snapshot, UiAction, ValidationIssue } from './model'
export const desktop = isTauri()
export function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!desktop) return Promise.reject(new Error('请在桌面版中执行此操作；浏览器模式支持项目设计与保存。'))
  return invoke<T>(command, args)
}
export async function pickFile(kind: 'project' | 'plugin' | 'directory' | 'asset'): Promise<string | undefined> {
  const result = await open({ directory: kind === 'directory', multiple: false, filters: kind === 'project' ? [{ name: 'HMI 项目', extensions: ['json'] }] : kind === 'asset' ? [{ name: '图片和字体', extensions: ['png', 'jpg', 'jpeg', 'webp', 'svg', 'woff', 'woff2', 'ttf', 'otf'] }] : undefined })
  return typeof result === 'string' ? result : undefined
}
export async function pickSave(name: string, zip = false) { return (await save({ defaultPath: `${name}.${zip ? 'zip' : 'json'}`, filters: [{ name: zip ? 'Windows 运行包' : 'HMI 项目', extensions: [zip ? 'zip' : 'json'] }] })) ?? undefined }
export function downloadProject(project: Project) {
  const link = document.createElement('a'), url = URL.createObjectURL(new Blob([JSON.stringify(project, null, 2)], { type: 'application/json' }))
  link.href = url; link.download = `${project.name}.json`; link.click(); setTimeout(() => URL.revokeObjectURL(url), 1000)
}
export const blankSnapshot = (): Snapshot => ({ variables: {}, points: {}, connections: {}, flows: {}, logs: [] })
export function useRuntime() {
  const running = ref(false), busy = ref(false), snapshot = ref<Snapshot>(blankSnapshot()), events = ref<RuntimeEvent[]>([]), inputs = ref<Record<string, unknown>>({}), currentPage = ref('')
  let unlisten: UnlistenFn | undefined, refreshPending = false, refreshAgain = false, disposed = false
  async function refresh() {
    if (!running.value || disposed) return
    if (refreshPending) { refreshAgain = true; return }
    refreshPending = true
    try {
      do {
        refreshAgain = false
        const next = await call<Snapshot>('runtime_snapshot')
        if (running.value && !disposed) snapshot.value = next
      } while (refreshAgain && running.value && !disposed)
    } finally { refreshPending = false }
  }
  async function subscribe() {
    if (!desktop || unlisten) return
    unlisten = await listen<RuntimeEvent>('runtime-event', event => {
      if (disposed) return
      events.value.push(event.payload); if (events.value.length > 1500) events.value.splice(0, events.value.length - 1500)
      if (event.payload.kind === 'page.changed') currentPage.value = event.payload.data.pageId ?? event.payload.data.id ?? ''
      void refresh().catch(() => {})
    })
  }
  async function start(project: Project) {
    busy.value = true
    try {
      await subscribe(); events.value = []; inputs.value = {}
      for (const widget of project.pages.flatMap(page => page.widgets).filter(widget => ['input', 'select', 'switch'].includes(widget.kind))) {
        const binding = widget.bindings.value
        inputs.value[widget.id] = binding?.source === 'variable' ? project.variables.find(variable => variable.id === binding.key)?.initial : binding?.source === 'literal' ? binding.value : widget.props.value ?? (widget.kind === 'switch' ? false : '')
      }
      currentPage.value = project.pages[0]?.id ?? ''; await call('start_preview', { project }); running.value = true; await refresh()
    } finally { busy.value = false }
  }
  async function stop() { busy.value = true; try { await call('stop_preview'); running.value = false; snapshot.value = blankSnapshot() } finally { busy.value = false } }
  async function act(action: UiAction, value?: unknown) {
    if (!running.value) throw new Error('请先启动运行会话')
    if (action.kind === 'startFlow') await call('start_flow', { flowId: action.flowId })
    else if (action.kind === 'stopFlow') await call('stop_flow', { flowId: action.flowId })
    else await call('trigger_flow', { flowId: action.flowId, entryNodeId: action.entryNodeId, message: { payload: value ?? {} }, inputs: { ...inputs.value } })
    await refresh()
  }
  function dispose() { disposed = true; unlisten?.(); unlisten = undefined }
  return { running, busy, snapshot, events, inputs, currentPage, start, stop, act, refresh, subscribe, dispose }
}
export type Runtime = ReturnType<typeof useRuntime>
export const getEnvironment = () => call<Environment>('get_environment')
export const validate = (project: Project) => call<ValidationIssue[]>('validate_project', { project })
