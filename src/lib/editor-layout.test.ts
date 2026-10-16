import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { effectScope, type EffectScope } from 'vue'
import { DEFAULT_EDITOR_LAYOUT, editorLayoutStorageKey, parseEditorLayout, resolveEditorLayoutSizes, useEditorLayout } from './editor-layout'

class TestWindow extends EventTarget {
  innerWidth = 1500
  innerHeight = 980
  values = new Map<string, string>()
  localStorage = {
    getItem: vi.fn((key: string) => this.values.get(key) ?? null),
    setItem: vi.fn((key: string, value: string) => { this.values.set(key, value) }),
  }
  listeners = new Map<string, Set<EventListenerOrEventListenerObject>>()
  override addEventListener(type: string, listener: EventListenerOrEventListenerObject | null, options?: AddEventListenerOptions | boolean) {
    if (listener) { const entries = this.listeners.get(type) ?? new Set(); entries.add(listener); this.listeners.set(type, entries) }
    super.addEventListener(type, listener, options)
  }
  override removeEventListener(type: string, listener: EventListenerOrEventListenerObject | null, options?: EventListenerOptions | boolean) {
    if (listener) this.listeners.get(type)?.delete(listener)
    super.removeEventListener(type, listener, options)
  }
  size(width: number, height: number) { this.innerWidth = width; this.innerHeight = height; this.dispatchEvent(new Event('resize')) }
}

class TestHandle extends EventTarget {
  captured: number | undefined
  setPointerCapture = vi.fn((id: number) => { this.captured = id })
  hasPointerCapture(id: number) { return this.captured === id }
  releasePointerCapture = vi.fn(() => { this.captured = undefined })
}

function pointer(type: string, x: number, y: number, pointerId = 1, handle?: TestHandle): PointerEvent {
  const event = new Event(type, { cancelable: true })
  Object.defineProperties(event, {
    clientX: { value: x }, clientY: { value: y }, pointerId: { value: pointerId },
    button: { value: 0 }, isPrimary: { value: true },
    ...(handle ? { currentTarget: { value: handle } } : {}),
  })
  return event as PointerEvent
}

let browser: TestWindow
const scopes: EffectScope[] = []
function setup(windowId: 'main' | 'logic' = 'main') {
  const scope = effectScope(); scopes.push(scope)
  const layout = scope.run(() => useEditorLayout(windowId))!
  return { ...layout, unmount: () => scope.stop() }
}
beforeEach(() => { vi.useFakeTimers(); browser = new TestWindow(); vi.stubGlobal('window', browser) })
afterEach(() => { scopes.splice(0).forEach(scope => scope.stop()); vi.useRealTimers(); vi.unstubAllGlobals() })

describe('editor layout preferences', () => {
  it('recovers defaults from damaged storage and validates individual fields', () => {
    for (const raw of [null, '{broken', 'null', '[]', '42', '"old format"']) expect(parseEditorLayout(raw)).toEqual(DEFAULT_EDITOR_LAYOUT)
    const restored = parseEditorLayout(JSON.stringify({ leftWidth: '350', rightWidth: 999, resourceHeight: 20,
      leftCollapsed: true, rightCollapsed: 1, resourcesExpanded: 'false', resourceTab: 'unknown', extra: 'ignored' }))
    expect(restored).toEqual({ ...DEFAULT_EDITOR_LAYOUT, rightWidth: 550, resourceHeight: 160, leftCollapsed: true })
    expect(parseEditorLayout('{"leftWidth":1e999,"rightWidth":null,"resourceHeight":-100}')).toEqual({ ...DEFAULT_EDITOR_LAYOUT, resourceHeight: 160 })
  })

  it('stores main and logic layouts separately and restores each window independently', () => {
    const main = setup('main'), logic = setup('logic')
    main.preferences.leftWidth = 340; main.preferences.resourceTab = 'events'
    logic.preferences.rightCollapsed = true; logic.preferences.resourceTab = 'issues'
    vi.advanceTimersByTime(200)
    const reopenedMain = setup('main'), reopenedLogic = setup('logic')
    expect(reopenedMain.preferences).toEqual({ ...DEFAULT_EDITOR_LAYOUT, leftWidth: 340, resourceTab: 'events' })
    expect(reopenedLogic.preferences).toEqual({ ...DEFAULT_EDITOR_LAYOUT, rightCollapsed: true, resourceTab: 'issues' })
    expect(browser.values.size).toBe(2)
    expect(browser.values.has(editorLayoutStorageKey('main'))).toBe(true)
    expect(browser.values.has(editorLayoutStorageKey('logic'))).toBe(true)
  })

  it('debounces non-drag preferences, flushes on unmount and resets only this window', () => {
    const main = setup('main'), logic = setup('logic')
    main.preferences.resourceTab = 'variables'; main.preferences.resourcesExpanded = true
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    vi.advanceTimersByTime(100)
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    main.unmount()
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('main'))!)).toMatchObject({ resourceTab: 'variables', resourcesExpanded: true })
    logic.preferences.rightWidth = 410; logic.reset()
    expect(logic.preferences).toEqual(DEFAULT_EDITOR_LAYOUT)
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('logic'))!)).toEqual(DEFAULT_EDITOR_LAYOUT)
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('main'))!).resourceTab).toBe('variables')
  })

  it('continues working when localStorage reads or writes are denied', () => {
    Object.defineProperty(browser, 'localStorage', { get() { throw new Error('Storage denied') } })
    const layout = setup()
    expect(layout.preferences).toEqual(DEFAULT_EDITOR_LAYOUT)
    layout.preferences.leftCollapsed = true
    expect(() => vi.advanceTimersByTime(500)).not.toThrow()
    expect(() => layout.reset()).not.toThrow()
    expect(layout.preferences).toEqual(DEFAULT_EDITOR_LAYOUT)
    expect(() => layout.unmount()).not.toThrow()
  })

  it.each(['pagehide', 'beforeunload'])('flushes pending collapse and tab preferences immediately on %s', type => {
    const layout = setup()
    layout.preferences.leftCollapsed = true; layout.preferences.rightCollapsed = true
    layout.preferences.resourceTab = 'events'; layout.preferences.resourcesExpanded = true
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    expect(browser.dispatchEvent(new Event(type, { cancelable: true }))).toBe(true)
    expect(setup().preferences).toEqual({ ...DEFAULT_EDITOR_LAYOUT, leftCollapsed: true, rightCollapsed: true, resourceTab: 'events', resourcesExpanded: true })
    browser.dispatchEvent(new Event('pagehide'))
    vi.advanceTimersByTime(500)
    expect(browser.localStorage.setItem).toHaveBeenCalledTimes(1)
    // A cancelled navigation or back/forward-cache restoration keeps editing.
    layout.preferences.resourceTab = 'variables'
    vi.advanceTimersByTime(200)
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('main'))!).resourceTab).toBe('variables')
  })
})

describe('viewport fitting preserves preferred dimensions', () => {
  it('compresses the left panel first, then the right, leaving 320px for the canvas', () => {
    const preferences = { ...DEFAULT_EDITOR_LAYOUT, leftWidth: 400, rightWidth: 500 }
    const original = { ...preferences }
    expect(resolveEditorLayoutSizes(preferences, { width: 1000, height: 980 })).toMatchObject({ leftWidth: 180, rightWidth: 500 })
    expect(resolveEditorLayoutSizes(preferences, { width: 800, height: 980 })).toMatchObject({ leftWidth: 180, rightWidth: 300 })
    expect(resolveEditorLayoutSizes(preferences, { width: 700, height: 980 })).toMatchObject({ leftWidth: 120, rightWidth: 260 })
    expect(resolveEditorLayoutSizes(preferences, { width: 300, height: 300 })).toEqual({ leftWidth: 0, rightWidth: 0, resourceHeight: 0 })
    expect(preferences).toEqual(original)
  })

  it('restores stored widths and height after shrinking, expanding and collapsing panels', () => {
    const saved = { ...DEFAULT_EDITOR_LAYOUT, leftWidth: 400, rightWidth: 500, resourceHeight: 400, resourcesExpanded: true }
    browser.values.set(editorLayoutStorageKey('main'), JSON.stringify(saved))
    const layout = setup()
    expect(layout.sizes.value).toEqual({ leftWidth: 400, rightWidth: 500, resourceHeight: 400 })
    browser.size(800, 600)
    expect(layout.sizes.value).toEqual({ leftWidth: 180, rightWidth: 300, resourceHeight: 270 })
    expect(layout.preferences).toEqual(saved)
    browser.size(1500, 980)
    expect(layout.sizes.value).toEqual({ leftWidth: 400, rightWidth: 500, resourceHeight: 400 })
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    layout.preferences.leftCollapsed = true; layout.preferences.rightCollapsed = true; layout.preferences.resourcesExpanded = false
    expect(layout.sizes.value).toEqual({ leftWidth: 0, rightWidth: 0, resourceHeight: 0 })
    layout.preferences.leftCollapsed = false; layout.preferences.rightCollapsed = false; layout.preferences.resourcesExpanded = true
    expect(layout.sizes.value).toEqual({ leftWidth: 400, rightWidth: 500, resourceHeight: 400 })
  })

  it('caps resources at 45 percent and reserves central height on shorter windows', () => {
    const preferences = { ...DEFAULT_EDITOR_LAYOUT, resourceHeight: 600, resourcesExpanded: true }
    expect(resolveEditorLayoutSizes(preferences, { width: 1200, height: 1000 }).resourceHeight).toBe(450)
    expect(resolveEditorLayoutSizes(preferences, { width: 800, height: 600 }).resourceHeight).toBe(270)
    expect(resolveEditorLayoutSizes(preferences, { width: 1200, height: 450 }).resourceHeight).toBe(164)
    expect(preferences.resourceHeight).toBe(600)
  })
})

describe('panel resizing lifecycle', () => {
  it('saves once on pointerup, ignores other pointers and releases capture', () => {
    const layout = setup(), handle = new TestHandle()
    layout.resize(pointer('pointerdown', 220, 100, 7, handle), 'left')
    expect(handle.setPointerCapture).toHaveBeenCalledWith(7)
    browser.dispatchEvent(pointer('pointermove', 350, 100, 8))
    expect(layout.preferences.leftWidth).toBe(220)
    browser.dispatchEvent(pointer('pointermove', 300, 100, 7))
    browser.dispatchEvent(pointer('pointerup', 300, 100, 8))
    expect(layout.preferences.leftWidth).toBe(300)
    vi.advanceTimersByTime(500)
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    browser.dispatchEvent(pointer('pointerup', 300, 100, 7))
    expect(browser.localStorage.setItem).toHaveBeenCalledTimes(1)
    expect(handle.releasePointerCapture).toHaveBeenCalledWith(7)
    expect(browser.listeners.get('pointermove')?.size).toBe(0)
    browser.dispatchEvent(pointer('pointermove', 440, 100, 7))
    expect(layout.preferences.leftWidth).toBe(300)
  })

  it('starts resizing from the visible boundary without overwriting an untouched compressed preference', () => {
    browser.values.set(editorLayoutStorageKey('main'), JSON.stringify({ ...DEFAULT_EDITOR_LAYOUT, leftWidth: 450 }))
    browser.size(800, 600)
    const layout = setup()
    expect(layout.sizes.value.leftWidth).toBe(180)
    layout.resize(pointer('pointerdown', 180, 100), 'left')
    browser.dispatchEvent(pointer('pointerup', 180, 100))
    expect(layout.preferences.leftWidth).toBe(450)
    expect(browser.localStorage.setItem).not.toHaveBeenCalled()
    layout.resize(pointer('pointerdown', 180, 100), 'left')
    browser.dispatchEvent(pointer('pointermove', 200, 100))
    browser.dispatchEvent(pointer('pointerup', 200, 100))
    expect(layout.preferences.leftWidth).toBe(200)
  })

  it('handles reversed right and bottom directions and clamps preferred dimensions', () => {
    const layout = setup()
    layout.resize(pointer('pointerdown', 1200, 100), 'right')
    browser.dispatchEvent(pointer('pointermove', 1150, 100)); browser.dispatchEvent(pointer('pointerup', 1150, 100))
    expect(layout.preferences.rightWidth).toBe(350)
    layout.preferences.resourcesExpanded = true
    layout.resize(pointer('pointerdown', 600, 700), 'bottom')
    browser.dispatchEvent(pointer('pointermove', 600, 630)); browser.dispatchEvent(pointer('pointerup', 600, 630))
    expect(layout.preferences.resourceHeight).toBe(310)
    layout.resize(pointer('pointerdown', 600, 630), 'bottom')
    browser.dispatchEvent(pointer('pointermove', 600, -1000)); browser.dispatchEvent(pointer('pointerup', 600, -1000))
    expect(layout.preferences.resourceHeight).toBe(980 * .45)
  })

  it('finishes an active drag and saves its dimensions when the page hides', () => {
    const layout = setup(), handle = new TestHandle()
    layout.resize(pointer('pointerdown', 220, 100, 1, handle), 'left')
    browser.dispatchEvent(pointer('pointermove', 290, 100))
    browser.dispatchEvent(new Event('pagehide'))
    expect(handle.captured).toBeUndefined()
    for (const type of ['pointermove', 'pointerup', 'pointercancel', 'blur']) expect(browser.listeners.get(type)?.size).toBe(0)
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('main'))!).leftWidth).toBe(290)
    browser.dispatchEvent(pointer('pointermove', 400, 100))
    expect(layout.preferences.leftWidth).toBe(290)
  })

  it('cleans up pointercancel and unmount, including pending writes and viewport listeners', () => {
    const layout = setup(), handle = new TestHandle()
    layout.resize(pointer('pointerdown', 220, 100, 1, handle), 'left')
    browser.dispatchEvent(pointer('pointermove', 260, 100))
    browser.dispatchEvent(pointer('pointercancel', 260, 100))
    expect(browser.listeners.get('pointermove')?.size).toBe(0)
    expect(handle.captured).toBeUndefined()
    layout.resize(pointer('pointerdown', 260, 100, 1, handle), 'left')
    browser.dispatchEvent(pointer('pointermove', 290, 100))
    layout.unmount()
    for (const type of ['pointermove', 'pointerup', 'pointercancel', 'blur', 'resize', 'pagehide', 'beforeunload']) expect(browser.listeners.get(type)?.size).toBe(0)
    expect(handle.captured).toBeUndefined()
    expect(JSON.parse(browser.values.get(editorLayoutStorageKey('main'))!).leftWidth).toBe(290)
    browser.dispatchEvent(pointer('pointermove', 400, 100)); browser.size(800, 600)
    expect(layout.preferences.leftWidth).toBe(290)
    expect(layout.sizes.value.leftWidth).toBe(290)
  })
})
