<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import Moveable from 'vue3-moveable'
import { VueSelecto } from 'vue3-selecto'
import { Minus, Plus, Maximize } from 'lucide-vue-next'
import WidgetView from './WidgetView.vue'
import type { Editor } from '../lib/editor'
const props = defineProps<{ editor: Editor }>()
const emit = defineEmits<{ 'serial-panel-drop': [x: number, y: number] }>()
const host = ref<HTMLElement>(), board = ref<HTMLElement>(), moveable = ref<any>(), zoom = ref(0.65), targets = ref<HTMLElement[]>([])
let observer: ResizeObserver, transforming = false
async function refreshTargets() {
  await nextTick()
  const next = props.editor.selected.value.map(id => board.value?.querySelector<HTMLElement>(`[data-widget-id="${id}"]`)).filter((el): el is HTMLElement => !!el)
  // Moveable treats a new target array as a new gesture target. Keep its identity
  // while dragging; geometry edits must not reinitialize an active gesture.
  if (next.length !== targets.value.length || next.some((target, index) => target !== targets.value[index])) targets.value = next
  await nextTick()
}
watch(() => [props.editor.selected.value.join(','), props.editor.pageId.value, props.editor.page.value.widgets.map(widget => widget.id).join(',')], refreshTargets)
watch(() => props.editor.selectedWidgets.value.map(widget => [widget.x, widget.y, widget.width, widget.height]), async () => { if (!transforming) { await nextTick(); if (!transforming) moveable.value?.updateRect() } }, { deep: true })
watch(zoom, async () => { await nextTick(); moveable.value?.updateRect() })
function fit() { if (host.value) zoom.value = Math.max(.2, Math.min(1, (host.value.clientWidth - 100) / props.editor.project.value.width, (host.value.clientHeight - 100) / props.editor.project.value.height)) }
onMounted(() => { observer = new ResizeObserver(() => { if (!targets.value.length) fit() }); if (host.value) observer.observe(host.value); fit(); void refreshTargets() })
onBeforeUnmount(() => observer?.disconnect())
async function select(event: MouseEvent, id: string) {
  if (event.button !== 0) return
  if (event.shiftKey || event.ctrlKey) { props.editor.selected.value = props.editor.selected.value.includes(id) ? props.editor.selected.value.filter(value => value !== id) : [...props.editor.selected.value, id]; return }
  // Established targets already receive Moveable's own press handler. Starting
  // another gesture on the same press would cancel the native one.
  if (props.editor.selected.value.includes(id)) return
  let released = false
  const release = () => { released = true }
  window.addEventListener('mouseup', release, { once: true })
  if (!props.editor.selected.value.includes(id)) {
    const targetReady = moveable.value?.waitToChangeTarget()
    props.editor.selected.value = [id]
    await refreshTargets()
    await targetReady
  }
  // The Vue wrapper delegates to Moveable's asynchronous internal renderer.
  // Its geometry update must commit before handing over the initial press.
  await new Promise<void>(resolve => requestAnimationFrame(() => resolve()))
  window.removeEventListener('mouseup', release)
  if (!released) moveable.value?.dragStart(event)
}
function dragStart(event: any) { transforming = true; props.editor.checkpoint(); const widget = props.editor.page.value.widgets.find(w => w.id === event.target.dataset.widgetId); if (widget) event.set([widget.x, widget.y]) }
function drag(event: any) { const widget = props.editor.page.value.widgets.find(w => w.id === event.target.dataset.widgetId); if (widget) { widget.x = Math.round(event.beforeTranslate[0]); widget.y = Math.round(event.beforeTranslate[1]); event.target.style.transform = `translate(${widget.x}px,${widget.y}px)` } }
function resizeStart(event: any) { transforming = true; props.editor.checkpoint(); const widget = props.editor.page.value.widgets.find(w => w.id === event.target.dataset.widgetId); if (widget) event.dragStart?.set([widget.x, widget.y]) }
function resize(event: any) { const widget = props.editor.page.value.widgets.find(w => w.id === event.target.dataset.widgetId); if (widget) { widget.width = Math.max(24, Math.round(event.width)); widget.height = Math.max(24, Math.round(event.height)); widget.x = Math.round(event.drag.beforeTranslate[0]); widget.y = Math.round(event.drag.beforeTranslate[1]); Object.assign(event.target.style, { width: `${widget.width}px`, height: `${widget.height}px`, transform: `translate(${widget.x}px,${widget.y}px)` }) } }
async function transformEnd() { transforming = false; await nextTick(); moveable.value?.updateRect() }
function drop(event: DragEvent) { const kind = event.dataTransfer?.getData('application/hmi-widget'); if (!kind || !board.value) return; const rect = board.value.getBoundingClientRect(), x = Math.round((event.clientX - rect.left) / zoom.value / 8) * 8, y = Math.round((event.clientY - rect.top) / zoom.value / 8) * 8; if (kind === 'serial-panel') emit('serial-panel-drop', x, y); else props.editor.addWidget(kind, x, y) }
</script>

<template>
  <div ref="host" class="canvas-workspace" @dragover.prevent @drop.prevent="drop">
    <div class="canvas-document-caption"><span>{{ editor.page.value.name }}</span><span>{{ editor.project.value.width }} × {{ editor.project.value.height }}</span></div>
    <div class="canvas-scroll"><div class="canvas-spacer" :style="{ width: `${editor.project.value.width * zoom}px`, height: `${editor.project.value.height * zoom}px` }"><div ref="board" class="design-board" :style="{ width: `${editor.project.value.width}px`, height: `${editor.project.value.height}px`, transform: `scale(${zoom})` }">
      <div v-for="widget in editor.page.value.widgets" :key="widget.id" :data-widget-id="widget.id" class="design-widget" :class="{ selected: editor.selected.value.includes(widget.id) }" :style="{ transform: `translate(${widget.x}px,${widget.y}px)`, width: `${widget.width}px`, height: `${widget.height}px`, zIndex: widget.zIndex }" @mousedown.stop="select($event, widget.id)"><WidgetView :widget="widget" :project="editor.project.value" editing /><span v-if="editor.selected.value.includes(widget.id)" class="widget-label">{{ widget.name }}</span></div>
      <div v-if="!editor.page.value.widgets.length" class="canvas-empty"><div class="canvas-empty-symbol">+</div><strong>从这里开始设计</strong><span>将左侧控件拖入画布，或点击添加</span></div>
      <Moveable ref="moveable" :target="targets.length === 1 ? targets[0] : targets" :draggable="true" :resizable="true" :snappable="true" :snap-grid-width="8" :snap-grid-height="8" :snap-threshold="5" :zoom="1 / zoom" :origin="false" :throttle-drag="1" :throttle-resize="1" @drag-start="dragStart" @drag="drag" @drag-end="transformEnd" @resize-start="resizeStart" @resize="resize" @resize-end="transformEnd" @drag-group-start="(event: any) => event.events.forEach(dragStart)" @drag-group="(event: any) => event.events.forEach(drag)" @drag-group-end="transformEnd" @resize-group-start="(event: any) => event.events.forEach(resizeStart)" @resize-group="(event: any) => event.events.forEach(resize)" @resize-group-end="transformEnd" />
    </div></div></div>
    <VueSelecto v-if="board" :container="host" :drag-container="board" :selectable-targets="['.design-widget']" :select-by-click="false" :select-from-inside="false" :toggle-continue-select="'shift'" :hit-rate="0" @drag-start="(event: any) => { if (event.inputEvent.target.closest('.design-widget,.moveable-control-box')) event.stop() }" @select="(event: any) => editor.selected.value = event.selected.map((el: HTMLElement) => el.dataset.widgetId!)" />
    <div class="canvas-zoom"><button title="缩小" @click="zoom = Math.max(.2, zoom - .1)"><Minus :size="15" /></button><span>{{ Math.round(zoom * 100) }}%</span><button title="放大" @click="zoom = Math.min(2, zoom + .1)"><Plus :size="15" /></button><i /><button title="适应画布" @click="fit"><Maximize :size="15" /></button></div>
  </div>
</template>
