<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import FButton from './fluent/FButton.vue'
import FInput from './fluent/FInput.vue'
import FToggle from './fluent/FToggle.vue'
import FSelect from './fluent/FSelect.vue'
import type { Project, RuntimeEvent, Snapshot, UiAction, Widget } from '../lib/model'
import { assetUrls, assetFonts } from '../lib/resources'
const props = defineProps<{ widget: Widget; project: Project; snapshot?: Snapshot; events?: RuntimeEvent[]; editing?: boolean }>()
const emit = defineEmits<{ action: [action: UiAction, value?: unknown]; input: [id: string, value: unknown] }>()
const input = ref<unknown>(props.widget.props.value ?? '')
const values = computed(() => {
  const values = { ...props.widget.props }
  for (const [property, binding] of Object.entries(props.widget.bindings)) {
    if (binding.source === 'literal') values[property] = binding.value
    if (binding.source === 'variable') values[property] = props.snapshot?.variables[binding.key] ?? props.project.variables.find(v => v.id === binding.key)?.initial
    if (binding.source === 'point') { const point = props.snapshot?.points[binding.key]; values[property] = point && typeof point === 'object' && 'value' in point ? (point.valid === false ? undefined : point.value) : point }
  }
  return values
})
watch(() => values.value.value, value => { input.value = value ?? '' })
const style = computed(() => ({ color: values.value.color, fontSize: `${values.value.fontSize ?? 14}px`, fontFamily: assetFonts.value[values.value.fontAssetId] || values.value.fontFamily || undefined, backgroundColor: ['group','number','chart','log'].includes(props.widget.kind) ? values.value.background : undefined, backgroundImage: assetUrls.value[values.value.backgroundAssetId] ? `url("${assetUrls.value[values.value.backgroundAssetId]}")` : undefined, backgroundSize: 'cover', backgroundPosition: 'center' }))
const number = computed(() => { const value = values.value.value; return value === null || value === undefined || value === '' || !Number.isFinite(Number(value)) ? '—' : Number(value).toFixed(Math.max(0, Math.min(8, Number(values.value.precision ?? 1)))) })
const samples = ref<number[]>([])
watch(() => values.value.value, value => { if (props.editing || value === undefined || value === null || value === '') return; const numeric = Number(value); if (Number.isFinite(numeric)) { samples.value.push(numeric); if (samples.value.length > 120) samples.value.shift() } })
const series = computed(() => { const data = samples.value; if (data.length < 2) return ''; const min = Math.min(...data) - 1, max = Math.max(...data) + 1; return data.map((v, i) => `${(i / (data.length - 1)) * 560 + 8},${130 - ((v - min) / (max - min)) * 112}`).join(' ') })
function action(event: string, value?: unknown) { const action = props.widget.events[event]; if (action && !props.editing) emit('action', action, value) }
function update(value: unknown) { input.value = value; emit('input', props.widget.id, value); action('change', value) }
</script>

<template>
  <div v-if="editing || values.visible !== false" class="widget-content" :inert="editing" :class="[`widget-${widget.kind}`, { 'widget-disabled': values.enabled === false }]" :style="style">
    <span v-if="widget.kind === 'text'">{{ values.text }}</span>
    <FButton v-else-if="widget.kind === 'button'" appearance="primary" :disabled="values.enabled === false" @click="action('click')">{{ values.text }}</FButton>
    <FInput v-else-if="widget.kind === 'input'" :model-value="String(input ?? '')" :placeholder="values.text" :disabled="values.enabled === false" @update:model-value="update" />
    <div v-else-if="widget.kind === 'number'" class="metric"><span class="metric-label">{{ values.text }}</span><div class="metric-value">{{ number }} <span class="metric-unit">{{ values.unit }}</span></div><span class="metric-caption">{{ editing ? '绑定数据后实时更新' : '当前读数' }}</span></div>
    <FSelect v-else-if="widget.kind === 'select'" :model-value="String(input ?? '')" :options="values.options ?? []" :placeholder="values.text" :disabled="values.enabled === false" @update:model-value="update" />
    <FToggle v-else-if="widget.kind === 'switch'" :model-value="Boolean(input)" :disabled="values.enabled === false" @update:model-value="update">{{ values.text }}</FToggle>
    <div v-else-if="widget.kind === 'indicator'" class="indicator"><span :class="['indicator-dot', { on: !!values.value }]" />{{ values.text }}</div>
    <div v-else-if="widget.kind === 'group'" class="group-label">{{ values.text }}</div>
    <div v-else-if="widget.kind === 'chart'" class="chart"><div class="widget-panel-title">{{ values.text }}<span>{{ values.unit }}</span></div><div class="chart-area"><div class="chart-grid" /><svg viewBox="0 0 576 146" preserveAspectRatio="none" aria-label="实时数据曲线"><polyline v-if="series" :points="series" fill="none" stroke="#0f6cbd" stroke-width="2" vector-effect="non-scaling-stroke" /></svg><span v-if="!series" class="chart-empty">{{ editing ? '数据趋势将在运行时显示' : '等待数据采集' }}</span></div><div class="chart-axis"><span>历史</span><span>实时</span></div></div>
    <div v-else-if="widget.kind === 'log'" class="runtime-log"><div class="widget-panel-title">{{ values.text }}</div><div v-if="!events?.length" class="log-empty">运行记录将在这里显示</div><div v-for="(event, index) in events?.slice(-100)" :key="index" class="runtime-log-line"><span>{{ new Date(event.timestamp).toLocaleTimeString('zh-CN') }}</span><code>{{ event.kind }} {{ JSON.stringify(event.data) }}</code></div></div>
    <span v-else>未知控件：{{ widget.kind }}</span>
  </div>
</template>
