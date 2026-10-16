<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { AlertCircle, RefreshCw, SquareStop } from 'lucide-vue-next'
import PreviewScreen from './components/PreviewScreen.vue'
import BaseButton from './components/base/BaseButton.vue'
import { desktop, useRuntime } from './lib/backend'
import { refreshAssets } from './lib/resources'
import { projectLocale,setDesignerLocale,t } from './lib/i18n'

const runtime = useRuntime({ previewWindow: true }), loading = ref(true), pending = ref(false), error = ref('')
async function load() {
  loading.value = true; error.value = ''
  try {
    if (!desktop) throw new Error('运行预览需要桌面环境，请从设计器打开预览窗口。')
    await runtime.connectPreview()
    const project = runtime.previewProject.value
    if (!project) throw new Error('预览会话已结束，请从设计器重新启动预览。')
    if (!project.i18n?.locales.includes(projectLocale.value)) projectLocale.value=project.i18n?.defaultLocale??'zh-CN'
    document.title = `${project.name} · ${t('运行预览')}`
  } catch (reason) { error.value = String(reason).replace(/^Error:\s*/, '') }
  finally { loading.value = false }
}
async function retryRuntime() { const project=runtime.previewProject.value; if(!project)return; pending.value=true; error.value=''; try { await runtime.start(project) } catch(reason) { error.value=String(reason).replace(/^Error:\s*/, '') } finally { pending.value=false } }
async function stop() {
  if (pending.value) return
  pending.value = true; error.value = ''
  try { await runtime.stop() } catch (reason) { error.value = String(reason).replace(/^Error:\s*/, '') }
  finally { pending.value = false }
}
watch(projectLocale,locale=>{setDesignerLocale(locale);if(runtime.previewProject.value)document.title=`${runtime.previewProject.value.name} · ${t('运行预览')}`})
watch(()=>[runtime.revision.value,runtime.previewProject.value?.id,runtime.previewProject.value?.name,JSON.stringify(runtime.previewProject.value?.assets)],()=>{const project=runtime.previewProject.value;if(project){document.title=`${project.name} · ${t('运行预览')}`;void refreshAssets(project).catch(reason=>error.value=String(reason))}})
onMounted(load)
onBeforeUnmount(runtime.dispose)
</script>

<template>
  <div class="preview-window-shell">
    <div v-if="error && runtime.previewProject.value?.presentation?.chrome !== 'project'" class="runner-error" role="alert"><AlertCircle :size="18" /><span>{{ error }}</span><BaseButton :disabled="loading || pending" @click="load"><RefreshCw :size="15" />重新加载</BaseButton></div>
    <PreviewScreen v-if="runtime.previewProject.value" :project="runtime.previewProject.value" :runtime="runtime" :pending="pending || loading" :error="error" @close="stop" @error="error = $event" @retry="retryRuntime" />
    <div v-else class="preview-loading"><p>{{ loading ? '正在连接预览会话…' : '无法载入预览' }}</p><BaseButton :disabled="pending || !desktop" @click="stop"><SquareStop :size="15" />关闭预览</BaseButton></div>
  </div>
</template>

<style scoped>
.preview-window-shell{height:100dvh;display:flex;flex-direction:column}
.preview-window-shell>.preview-screen{height:auto;flex:1;min-height:0}
.preview-loading{flex:1;display:flex;align-items:center;justify-content:center;flex-direction:column;gap:18px;color:var(--muted)}
</style>
