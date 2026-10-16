<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import type { LayoutSpec, LayoutTrack } from '../lib/model'

const props = defineProps<{ modelValue?: LayoutSpec; labelPrefix?: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: LayoutSpec] }>()
const error = ref(''), columnsDraft = ref(''), rowsDraft = ref('')
const layout = computed<LayoutSpec>(() => props.modelValue ?? { type: 'absolute', gap: 8, padding: 8 })
watch(() => [layout.value.columnTracks, layout.value.rowTracks], () => { columnsDraft.value = JSON.stringify(layout.value.columnTracks ?? [], null, 2); rowsDraft.value = JSON.stringify(layout.value.rowTracks ?? [], null, 2) }, { immediate: true, deep: true })
function update(patch: Partial<LayoutSpec>) { emit('update:modelValue', { ...layout.value, ...patch }) }
function updateNumber(key: 'gap' | 'gapX' | 'gapY' | 'padding', event: Event) { const value = Number((event.target as HTMLInputElement).value); update({ [key]: Math.max(0, Number.isFinite(value) ? value : 0) }) }
function updateSide(key: 'top' | 'right' | 'bottom' | 'left', event: Event) { const value = Number((event.target as HTMLInputElement).value); update({ paddingSides: { ...layout.value.paddingSides, [key]: Math.max(0, Number.isFinite(value) ? value : 0) } }) }
function updateTracks(axis: 'columnTracks' | 'rowTracks', raw: string) { const parsed = JSON.parse(raw); if (!Array.isArray(parsed) || parsed.some((track: LayoutTrack) => !track || !['px', 'fr', 'auto'].includes(track.kind) || (track.kind !== 'auto' && (!Number.isFinite(track.value) || track.value < 0)))) throw Error(`Invalid ${axis} track list`); update({ [axis]: parsed }) }
function editTracks(axis: 'columnTracks' | 'rowTracks', event: Event) { const raw = (event.target as HTMLTextAreaElement).value; if (axis === 'columnTracks') columnsDraft.value = raw; else rowsDraft.value = raw; try { updateTracks(axis, raw); error.value = '' } catch (reason) { error.value = String(reason) } }
</script>

<template>
  <div class="layout-spec-editor" data-testid="layout-spec-editor">
    <label>{{ labelPrefix ?? 'Layout' }}<select aria-label="Layout type" :value="layout.type" @change="update({ type: ($event.target as HTMLSelectElement).value as LayoutSpec['type'] })"><option value="absolute">Absolute</option><option value="horizontal">Horizontal</option><option value="vertical">Vertical</option><option value="grid">Grid</option><option value="form">Form</option></select></label>
    <template v-if="['horizontal','vertical'].includes(layout.type)">
      <label>Direction<select aria-label="Layout direction" :value="layout.direction ?? (layout.type === 'horizontal' ? 'row' : 'column')" @change="update({ direction: ($event.target as HTMLSelectElement).value as LayoutSpec['direction'] })"><option value="row">Row</option><option value="row-reverse">Row reverse</option><option value="column">Column</option><option value="column-reverse">Column reverse</option></select></label>
      <label class="checkbox-label"><input type="checkbox" aria-label="Wrap items" :checked="layout.wrap ?? false" @change="update({ wrap: ($event.target as HTMLInputElement).checked })">Wrap items</label>
    </template>
    <div class="property-grid">
      <label>Gap<input type="number" min="0" aria-label="Layout gap" :value="layout.gap ?? 8" @change="updateNumber('gap',$event)"></label>
      <label>Gap X<input type="number" min="0" aria-label="Horizontal gap" :value="layout.gapX ?? layout.gap ?? 8" @change="updateNumber('gapX',$event)"></label>
      <label>Gap Y<input type="number" min="0" aria-label="Vertical gap" :value="layout.gapY ?? layout.gap ?? 8" @change="updateNumber('gapY',$event)"></label>
      <label>Padding<input type="number" min="0" aria-label="Layout padding" :value="layout.padding ?? 8" @change="updateNumber('padding',$event)"></label>
    </div>
    <div class="property-grid">
      <label>Padding top<input type="number" min="0" aria-label="Padding top" :value="layout.paddingSides?.top ?? layout.padding ?? 8" @change="updateSide('top',$event)"></label>
      <label>Padding right<input type="number" min="0" aria-label="Padding right" :value="layout.paddingSides?.right ?? layout.padding ?? 8" @change="updateSide('right',$event)"></label>
      <label>Padding bottom<input type="number" min="0" aria-label="Padding bottom" :value="layout.paddingSides?.bottom ?? layout.padding ?? 8" @change="updateSide('bottom',$event)"></label>
      <label>Padding left<input type="number" min="0" aria-label="Padding left" :value="layout.paddingSides?.left ?? layout.padding ?? 8" @change="updateSide('left',$event)"></label>
    </div>
    <label>Align items<select aria-label="Layout align" :value="layout.align ?? 'stretch'" @change="update({ align: ($event.target as HTMLSelectElement).value as LayoutSpec['align'] })"><option value="start">Start</option><option value="center">Center</option><option value="end">End</option><option value="stretch">Stretch</option></select></label>
    <label>Justify content<select aria-label="Layout justify" :value="layout.justify ?? 'start'" @change="update({ justify: ($event.target as HTMLSelectElement).value as LayoutSpec['justify'] })"><option value="start">Start</option><option value="center">Center</option><option value="end">End</option><option value="space-between">Space between</option><option value="space-around">Space around</option><option value="space-evenly">Space evenly</option></select></label>
    <template v-if="layout.type === 'grid'">
      <label>Column tracks (JSON)<textarea aria-label="Column tracks JSON" rows="3" :value="columnsDraft" @change="editTracks('columnTracks',$event)"></textarea></label>
      <label>Row tracks (JSON)<textarea aria-label="Row tracks JSON" rows="3" :value="rowsDraft" @change="editTracks('rowTracks',$event)"></textarea></label>
      <p v-if="error" class="error-copy" role="alert">{{ error }}</p>
    </template>
  </div>
</template>

<style scoped>.layout-spec-editor{display:grid;gap:7px}.layout-spec-editor label{margin:0;font-size:11px}.layout-spec-editor .property-grid{gap:4px 8px}.layout-spec-editor .checkbox-label{display:flex;flex-direction:row;align-items:center;gap:7px}.layout-spec-editor .checkbox-label input{width:14px}.layout-spec-editor textarea{font:11px ui-monospace,monospace}</style>
