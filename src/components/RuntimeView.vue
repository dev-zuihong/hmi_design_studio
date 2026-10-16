<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import WidgetTree from './WidgetTree.vue'
import type { Project } from '../lib/model'
import type { Runtime } from '../lib/backend'
import { call } from '../lib/backend'
import { open } from '@tauri-apps/plugin-dialog'
import { provideRuntimeFileService } from '../lib/runtime-files'
const props = defineProps<{ project: Project; runtime: Runtime }>()
const emit = defineEmits<{ error: [message: string] }>()
watch(()=>props.runtime.lastError?.value,error=>{if(error)emit('error',error)})
provideRuntimeFileService({async choose(options){
  let path=options.path
  if(!path){const extensions=options.accept.split(',').map(value=>value.trim().replace(/^\*?\./,'')).filter(Boolean);const selected=await open({multiple:false,filters:extensions.length?[{name:'文件',extensions}]:undefined});if(typeof selected!=='string')return;path=selected}
  return call('runtime_import_file',{path})
}})
const host = ref<HTMLElement>()
const hostSize = ref({ width: 0, height: 0 })
const page = computed(() => props.project.pages.find(page => page.id === props.runtime.currentPage.value) ?? props.project.pages[0]!)
const responsiveLayout = computed(() => page.value.responsive !== false && (page.value.layout?.type ?? 'absolute') !== 'absolute')
const availableSize = computed(() => ({ width: Math.max(1, hostSize.value.width - 32), height: Math.max(1, hostSize.value.height - 32) }))
const designSize = computed(()=>props.runtime.windowSize?.value??{width:props.project.width,height:props.project.height})
const scale = computed(() => responsiveLayout.value ? 1 : Math.max(0.1, Math.min(availableSize.value.width / designSize.value.width, availableSize.value.height / designSize.value.height)))
const pageWidth = computed(() => responsiveLayout.value ? availableSize.value.width : designSize.value.width)
const pageHeight = computed(() => responsiveLayout.value ? availableSize.value.height : designSize.value.height)
const frameWidth = computed(() => pageWidth.value * scale.value)
const frameHeight = computed(() => pageHeight.value * scale.value)
const renderSnapshot = computed(() => {
  const snapshot = props.runtime.snapshot.value
  const context=props.runtime.context?.value??{}, windowId=context.windowInstanceId
  return { ...snapshot, context, ui:{...(snapshot.ui??{}),...(windowId?snapshot.windowUi?.[windowId]??{}:{})}, inputs: { ...(windowId?{}:snapshot.inputs ?? {}), ...props.runtime.inputs.value } }
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
