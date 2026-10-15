<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import WidgetView from './WidgetView.vue'
import type { Project } from '../lib/model'
import type { Runtime } from '../lib/backend'
const props = defineProps<{ project: Project; runtime: Runtime }>()
const emit = defineEmits<{ error: [message: string] }>()
const host = ref<HTMLElement>(), scale = ref(1)
const page = computed(() => props.project.pages.find(page => page.id === props.runtime.currentPage.value) ?? props.project.pages[0]!)
let resize: ResizeObserver
onMounted(() => { resize = new ResizeObserver(entries => { const rect = entries[0]!.contentRect; scale.value = Math.max(0.1, Math.min((rect.width - 32) / props.project.width, (rect.height - 32) / props.project.height)) }); if (host.value) resize.observe(host.value) })
onBeforeUnmount(() => resize?.disconnect())
</script>

<template>
  <div ref="host" class="runtime-host" :inert="!runtime.running.value || runtime.busy.value"><div class="runtime-frame" :style="{width: `${project.width * scale}px`, height: `${project.height * scale}px`}"><div class="runtime-page" :style="{width: `${project.width}px`, height: `${project.height}px`, transform: `scale(${scale})`}"><div v-for="widget in page.widgets" :key="widget.id" class="runtime-widget" :style="{ left: `${widget.x}px`, top: `${widget.y}px`, width: `${widget.width}px`, height: `${widget.height}px`, zIndex: widget.zIndex }"><WidgetView :widget="widget" :project="project" :snapshot="runtime.snapshot.value" :events="runtime.events.value" @input="(id, value) => runtime.inputs.value[id] = value" @action="(action, value) => runtime.act(action, value).catch(error => emit('error', String(error)))" /></div></div></div></div>
</template>
