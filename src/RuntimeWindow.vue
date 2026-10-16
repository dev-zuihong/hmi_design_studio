<script setup lang="ts">
import { onMounted, onBeforeUnmount, ref, watch } from 'vue'
import RuntimeView from './components/RuntimeView.vue'
import { useRuntime } from './lib/backend'
import { refreshAssets } from './lib/resources'
const runtime = useRuntime({runtimeWindow:true}), error = ref('')
watch(runtime.previewProject, async project => { if (project) { try { await refreshAssets(project) } catch (reason) { error.value = String(reason) } } })
onMounted(async()=>{try{await runtime.connectWindow()}catch(reason){error.value=String(reason)}})
onBeforeUnmount(runtime.dispose)
</script>
<template><main class="runtime-window-shell"><div v-if="error && runtime.previewProject.value?.presentation?.chrome !== 'project'" class="runner-error" role="alert">{{error}}<button @click="error=''">关闭提示</button></div><RuntimeView v-if="runtime.previewProject.value" :project="runtime.previewProject.value" :runtime="runtime" :error="error" @error="error=$event" @retry="runtime.connectWindow().catch(reason=>error=String(reason))"/></main></template>
<style scoped>.runtime-window-shell{height:100dvh;display:flex;flex-direction:column;min-width:0;overflow:hidden}</style>
