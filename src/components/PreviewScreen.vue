<script setup lang="ts">
import { computed } from 'vue'
import { ArrowLeft, MonitorPlay, SquareStop } from 'lucide-vue-next'
import FButton from './fluent/FButton.vue'
import RuntimeView from './RuntimeView.vue'
import type { Project } from '../lib/model'
import { call, type Runtime } from '../lib/backend'
import { projectLocale, t } from '../lib/i18n'

const props = defineProps<{ project: Project; runtime: Runtime; pending: boolean }>()
const emit = defineEmits<{ close: []; error: [message: string] }>()
function languageChanged(event:Event) { const locale=(event.target as HTMLSelectElement).value; projectLocale.value=locale; void call('set_preview_locale',{locale,expectedGeneration:props.runtime.generation.value}).catch(reason=>emit('error',String(reason))) }
const page = computed(() => props.project.pages.find(page => page.id === props.runtime.currentPage.value) ?? props.project.pages[0])
const busy = computed(() => props.pending || props.runtime.busy.value)
</script>

<template>
  <section class="preview-screen" aria-label="运行预览">
    <header class="preview-header">
      <FButton appearance="subtle" :disabled="busy" title="停止预览并返回编辑器" @click="$emit('close')"><ArrowLeft :size="16" />返回编辑器</FButton>
      <span class="header-divider" />
      <MonitorPlay class="preview-symbol" :size="20" />
      <div class="preview-project"><h1>{{ project.name }}</h1><span>运行预览</span></div>
      <span class="status-pill preview-status" role="status"><i class="live-dot" />{{ busy ? '处理中…' : runtime.running.value ? '运行中' : '已停止' }}</span>
      <span class="spacer" />
      <select v-if="project.i18n?.locales.length" class="preview-language" :value="projectLocale" :aria-label="t('预览语言')" @change="languageChanged"><option v-for="locale in project.i18n.locales" :key="locale" :value="locale">{{ locale }}</option></select>
      <span class="preview-page-name">{{ page?.name }}</span>
      <span class="preview-dimensions">{{ project.width }} × {{ project.height }}</span>
      <FButton :disabled="busy" @click="$emit('close')"><SquareStop :size="16" />停止预览</FButton>
    </header>
    <RuntimeView :project="project" :runtime="runtime" @error="$emit('error', $event)" />
    <footer class="preview-footer"><span>关闭预览窗口时将停止运行并关闭本次会话的连接</span><span>{{ project.pages.length }} 个页面 · {{ project.flows.length }} 个流程</span></footer>
  </section>
</template>

<style scoped>
.preview-screen{height:100dvh;display:flex;flex-direction:column;overflow:hidden;background:#edf0f4}
.preview-header{display:flex;align-items:center;gap:14px;min-height:68px;flex-shrink:0;padding:12px 20px;background:var(--surface);border-bottom:1px solid var(--line)}
.preview-symbol{color:var(--blue)}
.preview-project{min-width:0}
.preview-project h1{margin:0;max-width:360px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:14px;font-weight:600}
.preview-project>span{display:block;margin-top:4px;font-size:10px;color:var(--muted)}
.preview-status{display:flex;align-items:center;gap:6px;color:#34784d;background:#f0f8f2;border-color:#dceee1}
.preview-language{width:110px;font-size:11px}.preview-page-name{max-width:180px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;font-size:12px;color:#586b7f}
.preview-dimensions{font-size:11px;color:var(--muted);font-variant-numeric:tabular-nums;white-space:nowrap}
.preview-footer{display:flex;justify-content:space-between;gap:16px;padding:8px 20px;flex-shrink:0;background:var(--surface);border-top:1px solid var(--line);font-size:10px;color:var(--muted)}
@media(max-width:800px){.preview-header{flex-wrap:wrap;gap:10px;padding:10px 12px}.preview-project{flex:1}.preview-project h1{max-width:220px}.preview-header>.spacer,.preview-symbol,.preview-header>.header-divider{display:none}.preview-footer{padding:8px 12px}.preview-footer>span:last-child{display:none}}
</style>
