<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import WidgetTree from './WidgetTree.vue'
import type { Project } from '../lib/model'
import type { Runtime } from '../lib/backend'
const props = defineProps<{ project: Project; runtime: Runtime }>()
const emit = defineEmits<{ error: [message: string] }>()
const host = ref<HTMLElement>()
const hostSize = ref({ width: 0, height: 0 })
const page = computed(() => props.project.pages.find(page => page.id === props.runtime.currentPage.value) ?? props.project.pages[0]!)
const responsiveLayout = computed(() => page.value.responsive !== false && (page.value.layout?.type ?? 'absolute') !== 'absolute')
const availableSize = computed(() => ({ width: Math.max(1, hostSize.value.width - 32), height: Math.max(1, hostSize.value.height - 32) }))
const scale = computed(() => responsiveLayout.value ? 1 : Math.max(0.1, Math.min(availableSize.value.width / props.project.width, availableSize.value.height / props.project.height)))
const pageWidth = computed(() => responsiveLayout.value ? availableSize.value.width : props.project.width)
const pageHeight = computed(() => responsiveLayout.value ? availableSize.value.height : props.project.height)
const frameWidth = computed(() => pageWidth.value * scale.value)
const frameHeight = computed(() => pageHeight.value * scale.value)
const renderSnapshot = computed(() => {
  const snapshot = props.runtime.snapshot.value
  return { ...snapshot, inputs: { ...(snapshot.inputs ?? {}), ...props.runtime.inputs.value } }
})
let resize: ResizeObserver | undefined, resizeFrame = 0
onMounted(() => {
  const updateSize = (width: number, height: number) => {
    cancelAnimationFrame(resizeFrame)
    resizeFrame = requestAnimationFrame(() => { hostSize.value = { width, height } })
  }
  if (host.value) {
    const rect = host.value.getBoundingClientRect()
    updateSize(rect.width, rect.height)
    resize = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect
      if (rect) updateSize(rect.width, rect.height)
    })
    resize.observe(host.value)
  }
})
onBeforeUnmount(() => { resize?.disconnect(); cancelAnimationFrame(resizeFrame) })
</script>

<template>
  <div ref="host" class="runtime-host" :inert="!runtime.running.value || runtime.busy.value"><div class="runtime-frame" :data-responsive-layout="responsiveLayout" :data-runtime-scale="scale" :style="{width: `${frameWidth}px`, height: `${frameHeight}px`}"><div class="runtime-page" :style="{width: `${pageWidth}px`, height: `${pageHeight}px`, transform: `scale(${scale})`}"><WidgetTree :project="project" :page-id="page.id" :snapshot="renderSnapshot" :events="runtime.events.value" @input="(id, value) => runtime.updateInput(id, value).catch(error => emit('error', String(error)))" @action="(action, value) => runtime.act(action, value).catch(error => emit('error', String(error)))" /></div></div></div>
</template>
