<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue'
import { Activity, ChevronDown, Download, MoreHorizontal, Play, Redo2, Save, Settings2, ShieldCheck, SquareStop, Undo2, Workflow } from 'lucide-vue-next'
import FButton from './fluent/FButton.vue'
import type { Editor } from '../lib/editor'
import { designerLocale, setDesignerLocale, t } from '../lib/i18n'

defineProps<{ editor: Editor; logic: boolean; desktop: boolean; pending: boolean; running: boolean; busy: boolean; leftCollapsed: boolean; rightCollapsed: boolean }>()
const emit = defineEmits<{
  new: []; open: []; save: [saveAs?: boolean]; import: [kind: 'plugin' | 'python' | 'dependencies']; asset: [];
  example: []; languages: []; help: []; reset: []; logic: []; check: []; preview: []; stop: []; export: [];
  toggleLeft: []; toggleRight: [];
}>()
const host = ref<HTMLElement>()
const menu = ref<'file' | 'settings' | 'more' | ''>('')
function close(restoreFocus = true) {
  const trigger = menu.value && host.value?.querySelector<HTMLElement>(`[data-menu-trigger="${menu.value}"]`)
  menu.value = ''
  if (restoreFocus && trigger) trigger.focus()
}
function toggle(value: typeof menu.value) { menu.value = menu.value === value ? '' : value }
function outside(event: PointerEvent) { if (!event.composedPath().includes(host.value!)) close(false) }
function keyboard(event: KeyboardEvent) {
  if (event.key === 'Escape' && menu.value) {
    const trigger = host.value?.querySelector<HTMLElement>(`[data-menu-trigger="${menu.value}"]`)
    close(); trigger?.focus()
  }
}
onMounted(() => { document.addEventListener('pointerdown', outside); document.addEventListener('keydown', keyboard) })
onBeforeUnmount(() => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', keyboard) })
</script>

<template>
  <header ref="host" class="studio-toolbar" :aria-label="t('主操作栏')">
    <div class="studio-brand" :title="logic ? t('独立逻辑编辑器') : 'HMI Studio'"><Activity :size="19" /></div>
    <div class="toolbar-menu-host">
      <button class="toolbar-text-button" data-menu-trigger="file" :aria-expanded="menu === 'file'" aria-controls="studio-file-menu" @click="toggle('file')">{{ t('文件') }}<ChevronDown :size="12" /></button>
      <div v-if="menu === 'file'" id="studio-file-menu" class="studio-menu" @click="close()">
        <button @click="emit('new')">{{ t('新建项目') }}</button>
        <button title="打开项目" @click="emit('open')">{{ t('打开项目') }}</button>
        <button @click="emit('save')">{{ t('保存工程包') }}<kbd>Ctrl / ⌘ S</kbd></button>
        <button @click="emit('save', true)">{{ t('另存为') }}</button>
        <hr />
        <button :disabled="!desktop" @click="emit('import', 'plugin')">{{ t('导入协议插件') }}</button>
        <button :disabled="!desktop" @click="emit('import', 'python')">{{ t('导入 Python 运行资源') }}</button>
        <button :disabled="!desktop" @click="emit('import', 'dependencies')">{{ t('导入 Python 依赖') }}</button>
        <button @click="emit('asset')">{{ t('导入图片或字体') }}</button>
        <hr />
        <button @click="emit('languages')">{{ t('工程语言') }}</button>
        <button @click="emit('example')">{{ t('添加 Python 示例流程') }}</button>
      </div>
    </div>
    <div class="toolbar-project">
      <input class="project-title-input" :aria-label="t('项目名称')" :value="editor.project.value.name" @change="editor.mutate(project => project.name = ($event.target as HTMLInputElement).value.trim() || t('未命名项目'))" />
      <span v-if="editor.dirty.value" class="unsaved-dot" :title="t('有未保存修改')" :aria-label="t('有未保存修改')" />
    </div>
    <div class="toolbar-history">
      <button class="icon-button" title="保存项目 (Ctrl+S)" :disabled="pending" @click="emit('save')"><Save :size="17" /></button>
      <button class="icon-button" title="撤销 (Ctrl+Z)" :disabled="!editor.canUndo.value" @click="editor.undo"><Undo2 :size="17" /></button>
      <button class="icon-button" title="重做 (Ctrl+Y)" :disabled="!editor.canRedo.value" @click="editor.redo"><Redo2 :size="17" /></button>
    </div>
    <span class="toolbar-divider" />
    <button v-if="!logic" class="toolbar-text-button logic-launch" :aria-label="t('逻辑流程') + ' ↗'" :title="t('打开独立逻辑窗口')" @click="emit('logic')"><Workflow :size="17" /><span>{{ t('逻辑流程') }} ↗</span></button>
    <span v-else class="logic-window-label"><Workflow :size="16" />{{ t('独立逻辑编辑器') }}</span>
    <span class="spacer" />
    <FButton class="wide-command" appearance="subtle" :disabled="pending" @click="emit('check')"><ShieldCheck :size="16" />{{ t('检查') }}</FButton>
    <FButton appearance="primary" class="toolbar-preview" :disabled="pending || busy" @click="emit('preview')"><Play :size="16" />{{ t(running ? '显示预览' : '预览运行') }}</FButton>
    <button v-if="running" class="icon-button" :title="t('停止预览')" :aria-label="t('停止预览')" :disabled="pending || busy" @click="emit('stop')"><SquareStop :size="17" /></button>
    <FButton class="wide-command" :disabled="pending || !desktop" @click="emit('export')"><Download :size="16" />{{ t('导出程序') }}</FButton>
    <div class="toolbar-menu-host compact-more">
      <button class="icon-button" data-menu-trigger="more" :title="t('更多操作')" :aria-label="t('更多操作')" :aria-expanded="menu === 'more'" @click="toggle('more')"><MoreHorizontal :size="18" /></button>
      <div v-if="menu === 'more'" class="studio-menu menu-right" @click="close()">
        <button :disabled="pending" @click="emit('check')">{{ t('检查') }}</button>
        <button :disabled="pending || !desktop" @click="emit('export')">{{ t('导出程序') }}</button>
      </div>
    </div>
    <div class="toolbar-menu-host">
      <button class="icon-button" data-menu-trigger="settings" :title="t('工作区设置')" :aria-label="t('工作区设置')" :aria-expanded="menu === 'settings'" aria-controls="studio-settings-menu" @click="toggle('settings')"><Settings2 :size="18" /></button>
      <div v-if="menu === 'settings'" id="studio-settings-menu" class="studio-menu menu-right">
        <label class="toolbar-language">{{ t('设计器语言') }}<select :value="designerLocale" :aria-label="t('设计器语言')" @change="setDesignerLocale(($event.target as HTMLSelectElement).value)"><option value="zh-CN">简体中文</option><option value="en-US">English</option></select></label>
        <hr />
        <button @click="emit('toggleLeft'); close()">{{ t(leftCollapsed ? '展开组件面板' : '收起组件面板') }}</button>
        <button @click="emit('toggleRight'); close()">{{ t(rightCollapsed ? '展开属性面板' : '收起属性面板') }}</button>
        <button @click="emit('reset'); close()">{{ t('恢复默认布局') }}</button>
        <hr />
        <button @click="emit('help'); close()">{{ t('使用帮助') }}</button>
      </div>
    </div>
  </header>
</template>

<style scoped>
.studio-toolbar{height:48px;min-height:48px;display:flex;align-items:center;gap:6px;padding:0 12px;border-bottom:1px solid #e1e5eb;background:#fff;flex-shrink:0;position:relative;z-index:100;min-width:0}
.studio-brand{width:28px;height:28px;background:#0f6cbd;color:white;border-radius:6px;display:grid;place-items:center;flex-shrink:0;margin-right:4px}
.toolbar-text-button{height:30px;display:flex;align-items:center;gap:6px;padding:0 8px;white-space:nowrap;border-radius:4px;font-size:12px;color:#35485b}
.toolbar-text-button:hover{background:#f0f3f7}.toolbar-project{display:flex;align-items:center;gap:4px;min-width:80px;flex:0 1 240px}
.toolbar-project .project-title-input{width:100%;min-width:60px;max-width:240px;font-size:13px;height:30px;padding:5px 6px}
.toolbar-history{display:flex;gap:2px}.toolbar-divider{height:20px;width:1px;background:#e3e7ed;margin:0 3px;flex-shrink:0}
.toolbar-menu-host{position:relative;flex-shrink:0}.studio-menu{position:absolute;top:calc(100% + 8px);left:0;width:248px;padding:5px;border:1px solid #dce2e9;background:#fff;border-radius:6px;box-shadow:0 8px 24px #182d4626;max-height:calc(100dvh - 60px);overflow-y:auto;z-index:200}
.menu-right{left:auto;right:0}.studio-menu>button{display:flex;width:100%;align-items:center;gap:8px;padding:7px 9px;min-height:30px;text-align:left;font-size:12px;border-radius:4px;color:#35485b}
.studio-menu>button:hover:not(:disabled){background:#edf4fb}.studio-menu kbd{margin-left:auto;color:#778596;font-size:10px}.studio-menu hr{border:0;border-top:1px solid #edf0f4;margin:5px 2px}
.toolbar-language{display:flex;gap:8px;align-items:center;padding:7px 9px;margin:0;font-size:12px}.toolbar-language select{width:116px;margin-left:auto;padding:4px;font-size:12px}
.logic-window-label{display:flex;align-items:center;gap:6px;white-space:nowrap;font-size:12px;color:#667789}.compact-more{display:none}
.studio-toolbar :deep(fluent-button){flex-shrink:0;font-size:12px}.studio-toolbar .icon-button{width:30px;height:30px;flex-shrink:0}
.studio-toolbar button:focus-visible{outline:2px solid #0f6cbd;outline-offset:1px}.studio-toolbar button:disabled{opacity:.4;cursor:default}
@media(max-width:1100px){.studio-toolbar{gap:4px;padding-inline:8px}.wide-command{display:none}.compact-more{display:block}.toolbar-project{flex-basis:180px}.logic-launch span{display:none}}
@media(max-width:850px){.toolbar-project{flex-basis:130px}.logic-window-label{max-width:120px;overflow:hidden;text-overflow:ellipsis}.studio-toolbar{gap:2px}.studio-brand{margin-right:0}}
</style>
