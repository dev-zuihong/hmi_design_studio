import { computed, getCurrentScope, onScopeDispose, reactive, watch, type ComputedRef } from 'vue'

export type EditorLayoutWindowId = 'main' | 'logic'
export type EditorResourceTab = 'connections' | 'variables' | 'events' | 'issues'
export type EditorLayoutSide = 'left' | 'right' | 'bottom'

export interface EditorLayoutPreferences {
  leftWidth: number
  rightWidth: number
  resourceHeight: number
  leftCollapsed: boolean
  rightCollapsed: boolean
  resourcesExpanded: boolean
  resourceTab: EditorResourceTab
}

export interface EditorLayoutSizes { leftWidth: number; rightWidth: number; resourceHeight: number }
export interface EditorLayoutViewport { width: number; height: number }

export const DEFAULT_EDITOR_LAYOUT: Readonly<EditorLayoutPreferences> = Object.freeze({
  leftWidth: 220,
  rightWidth: 300,
  resourceHeight: 240,
  leftCollapsed: false,
  rightCollapsed: false,
  resourcesExpanded: false,
  resourceTab: 'connections',
})

const LEFT_MIN = 180, LEFT_MAX = 450, RIGHT_MIN = 260, RIGHT_MAX = 550
const RESOURCE_MIN = 160, CANVAS_MIN_WIDTH = 320, CANVAS_MIN_HEIGHT = 180
// Header (48), canvas context/ruler (54), and the resource splitter (4).
const VERTICAL_CHROME = 106
const SAVE_DELAY = 180
const FALLBACK_VIEWPORT: EditorLayoutViewport = { width: 1500, height: 980 }
const RESOURCE_TABS: readonly EditorResourceTab[] = ['connections', 'variables', 'events', 'issues']

export function editorLayoutStorageKey(windowId: EditorLayoutWindowId): string {
  return `hmi.studio.layout.v1.${windowId}`
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value))
}

function finitePreference(value: unknown, fallback: number, minimum: number, maximum: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? clamp(value, minimum, maximum) : fallback
}

function normalizePreferences(value: unknown): EditorLayoutPreferences {
  const saved = value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : {}
  return {
    leftWidth: finitePreference(saved.leftWidth, DEFAULT_EDITOR_LAYOUT.leftWidth, LEFT_MIN, LEFT_MAX),
    rightWidth: finitePreference(saved.rightWidth, DEFAULT_EDITOR_LAYOUT.rightWidth, RIGHT_MIN, RIGHT_MAX),
    // Viewport limits apply to displayed height, not the preference. Otherwise
    // opening a small window would permanently discard a larger saved height.
    resourceHeight: finitePreference(saved.resourceHeight, DEFAULT_EDITOR_LAYOUT.resourceHeight, RESOURCE_MIN, Number.MAX_SAFE_INTEGER),
    leftCollapsed: typeof saved.leftCollapsed === 'boolean' ? saved.leftCollapsed : DEFAULT_EDITOR_LAYOUT.leftCollapsed,
    rightCollapsed: typeof saved.rightCollapsed === 'boolean' ? saved.rightCollapsed : DEFAULT_EDITOR_LAYOUT.rightCollapsed,
    resourcesExpanded: typeof saved.resourcesExpanded === 'boolean' ? saved.resourcesExpanded : DEFAULT_EDITOR_LAYOUT.resourcesExpanded,
    resourceTab: RESOURCE_TABS.includes(saved.resourceTab as EditorResourceTab) ? saved.resourceTab as EditorResourceTab : DEFAULT_EDITOR_LAYOUT.resourceTab,
  }
}

export function parseEditorLayout(raw: string | null): EditorLayoutPreferences {
  try { return normalizePreferences(raw === null ? undefined : JSON.parse(raw)) }
  catch { return { ...DEFAULT_EDITOR_LAYOUT } }
}

function viewportDimension(value: number, fallback: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : fallback
}

function maximumResourceHeight(height: number): number {
  return Math.max(0, Math.min(height * .45, height - VERTICAL_CHROME - CANVAS_MIN_HEIGHT))
}

/** Derive visible dimensions without changing or persisting the user's choices. */
export function resolveEditorLayoutSizes(preferences: EditorLayoutPreferences, viewport: EditorLayoutViewport): EditorLayoutSizes {
  const preferred = normalizePreferences(preferences)
  const width = viewportDimension(viewport.width, FALLBACK_VIEWPORT.width)
  const height = viewportDimension(viewport.height, FALLBACK_VIEWPORT.height)
  let leftWidth = preferred.leftCollapsed ? 0 : preferred.leftWidth
  let rightWidth = preferred.rightCollapsed ? 0 : preferred.rightWidth
  let excess = Math.max(0, leftWidth + rightWidth - Math.max(0, width - CANVAS_MIN_WIDTH))
  const reduceLeft = Math.min(excess, Math.max(0, leftWidth - LEFT_MIN))
  leftWidth -= reduceLeft; excess -= reduceLeft
  const reduceRight = Math.min(excess, Math.max(0, rightWidth - RIGHT_MIN))
  rightWidth -= reduceRight; excess -= reduceRight
  // Below the supported minimum window width, keep the canvas usable rather
  // than letting minimum panel widths force the whole shell off screen.
  const reduceLeftBelowMinimum = Math.min(excess, leftWidth)
  leftWidth -= reduceLeftBelowMinimum; excess -= reduceLeftBelowMinimum
  rightWidth = Math.max(0, rightWidth - excess)
  return {
    leftWidth,
    rightWidth,
    resourceHeight: preferred.resourcesExpanded ? Math.min(preferred.resourceHeight, maximumResourceHeight(height)) : 0,
  }
}

export function useEditorLayout(windowId: EditorLayoutWindowId): {
  preferences: EditorLayoutPreferences
  sizes: ComputedRef<EditorLayoutSizes>
  reset: () => void
  resize: (event: PointerEvent, side: EditorLayoutSide) => void
} {
  const browser = typeof window === 'undefined' ? undefined : window
  const key = editorLayoutStorageKey(windowId)
  let stored: string | null = null
  try { stored = browser?.localStorage.getItem(key) ?? null } catch { /* Storage may be unavailable in a webview. */ }
  const preferences = reactive(parseEditorLayout(stored))
  const viewport = reactive({ width: browser?.innerWidth ?? FALLBACK_VIEWPORT.width, height: browser?.innerHeight ?? FALLBACK_VIEWPORT.height })
  const sizes = computed(() => resolveEditorLayoutSizes(preferences, viewport))
  let saveTimer: ReturnType<typeof setTimeout> | undefined
  let pendingSave = false
  let disposed = false
  let finishDrag: (() => void) | undefined

  function cancelSaveTimer() {
    if (saveTimer !== undefined) { clearTimeout(saveTimer); saveTimer = undefined }
  }
  function save(force = false) {
    cancelSaveTimer()
    if (!pendingSave && !force) return
    pendingSave = false
    try { browser?.localStorage.setItem(key, JSON.stringify(normalizePreferences(preferences))) }
    catch { /* A full or disabled store must not prevent editing. */ }
  }
  const stopWatching = watch(preferences, () => {
    pendingSave = true
    cancelSaveTimer()
    if (!finishDrag && !disposed) saveTimer = setTimeout(save, SAVE_DELAY)
  }, { deep: true, flush: 'sync' })

  function updateViewport() {
    if (!browser) return
    viewport.width = browser.innerWidth; viewport.height = browser.innerHeight
  }
  function flushPending() {
    finishDrag?.()
    save()
  }
  browser?.addEventListener('resize', updateViewport)
  // Refresh and window close do not unmount Vue before discarding the page.
  // Flush synchronously without preventing navigation or requesting a prompt.
  browser?.addEventListener('pagehide', flushPending)
  browser?.addEventListener('beforeunload', flushPending)

  function reset() {
    if (disposed) return
    finishDrag?.()
    Object.assign(preferences, DEFAULT_EDITOR_LAYOUT)
    save(true)
  }

  function resize(event: PointerEvent, side: EditorLayoutSide) {
    if (!browser || disposed || finishDrag || event.button !== 0 || event.isPrimary === false) return
    if (side === 'left' && preferences.leftCollapsed || side === 'right' && preferences.rightCollapsed || side === 'bottom' && !preferences.resourcesExpanded) return
    event.preventDefault()
    cancelSaveTimer()
    const pointerId = event.pointerId
    const startX = event.clientX, startY = event.clientY
    const startSize = side === 'left' ? sizes.value.leftWidth : side === 'right' ? sizes.value.rightWidth : sizes.value.resourceHeight
    const handle = event.currentTarget as Element | null
    const captureOptions = { capture: true, passive: false }
    let active = true
    let moved = false
    function move(next: PointerEvent) {
      if (!active || disposed || next.pointerId !== pointerId) return
      next.preventDefault()
      const delta = side === 'left' ? next.clientX - startX : side === 'right' ? startX - next.clientX : startY - next.clientY
      if (delta === 0 && !moved) return
      moved = true
      if (side === 'left') preferences.leftWidth = clamp(startSize + delta, LEFT_MIN, LEFT_MAX)
      else if (side === 'right') preferences.rightWidth = clamp(startSize + delta, RIGHT_MIN, RIGHT_MAX)
      else preferences.resourceHeight = clamp(startSize + delta, RESOURCE_MIN, Math.max(RESOURCE_MIN, maximumResourceHeight(viewport.height)))
    }
    function end(next: PointerEvent) { if (next.pointerId === pointerId) finish() }
    function finish() {
      if (!active) return
      active = false
      browser!.removeEventListener('pointermove', move, captureOptions)
      browser!.removeEventListener('pointerup', end, captureOptions)
      browser!.removeEventListener('pointercancel', end, captureOptions)
      browser!.removeEventListener('blur', blur)
      handle?.removeEventListener('lostpointercapture', end as EventListener)
      finishDrag = undefined
      try { if (handle?.hasPointerCapture(pointerId)) handle.releasePointerCapture(pointerId) } catch { /* The webview may have already released it. */ }
      save()
    }
    finishDrag = finish
    function blur() { finish() }
    browser.addEventListener('pointermove', move, captureOptions)
    browser.addEventListener('pointerup', end, captureOptions)
    browser.addEventListener('pointercancel', end, captureOptions)
    browser.addEventListener('blur', blur)
    handle?.addEventListener('lostpointercapture', end as EventListener)
    try { handle?.setPointerCapture(pointerId) } catch { /* Window listeners cover hosts without pointer capture. */ }
  }

  function dispose() {
    disposed = true
    stopWatching()
    finishDrag?.()
    browser?.removeEventListener('resize', updateViewport)
    browser?.removeEventListener('pagehide', flushPending)
    browser?.removeEventListener('beforeunload', flushPending)
    save()
  }
  if (getCurrentScope()) onScopeDispose(dispose)
  return { preferences, sizes, reset, resize }
}
