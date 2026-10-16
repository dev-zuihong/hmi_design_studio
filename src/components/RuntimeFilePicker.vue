<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { useRuntimeFileService, type RuntimeFileRef } from '../lib/runtime-files'
const props = withDefaults(defineProps<{ modelValue?: RuntimeFileRef | null; accept?: string; allowManualPath?: boolean; clearable?: boolean; disabled?: boolean; readOnly?: boolean; status?: string; message?: string; locale?: string }>(), { accept: '', clearable: true })
const emit = defineEmits<{ change: [payload: { value: RuntimeFileRef | null; reason: 'browse' | 'manual' | 'clear' }] }>()
const service = useRuntimeFileService(), pending = ref(false), error = ref(''), path = ref('')
let revision = 0, mounted = true
onBeforeUnmount(() => { mounted = false })
const english = computed(() => props.locale?.startsWith('en'))
watch(() => props.modelValue, value => { revision++; path.value = value?.path ?? value?.name ?? '' }, { immediate: true })
const blocked = computed(() => props.disabled || props.readOnly || pending.value)
async function choose(reason: 'browse' | 'manual') {
  if (blocked.value) return
  if (!service) { error.value = english.value ? 'File service unavailable' : '文件服务不可用'; return }
  if (reason === 'manual' && (!path.value.trim() || path.value.trim() === props.modelValue?.path)) return
  pending.value = true; error.value = ''
  const requestRevision = revision
  try { const value = await service.choose({ accept: props.accept, value: props.modelValue, ...(reason === 'manual' ? { path: path.value.trim() } : {}) }); if (value && mounted && requestRevision === revision) emit('change', { value, reason }) }
  catch (err) { error.value = String(err) }
  finally { pending.value = false }
}
function clear() { if (!blocked.value) { error.value = ''; path.value = ''; emit('change', { value: null, reason: 'clear' }) } }
</script>
<template><div class="runtime-file-picker" :aria-busy="pending">
  <div class="runtime-file-controls"><input v-model="path" :readonly="!allowManualPath || readOnly" :disabled="disabled || pending" :aria-label="english ? 'File path' : '文件路径'" :placeholder="english ? 'Select a file' : '选择文件'" :title="path" @change="allowManualPath && choose('manual')" @keydown.enter.prevent="allowManualPath && choose('manual')"><button type="button" :disabled="blocked" @click="choose('browse')">{{ pending ? (english ? 'Loading…' : '导入中…') : (english ? 'Browse…' : '浏览…') }}</button><button v-if="clearable && modelValue" type="button" :disabled="blocked" :aria-label="english ? 'Clear file' : '清除文件'" @click="clear">×</button></div>
  <small v-if="error || message" role="status" :class="{ 'file-error': error || status === 'error' }">{{ error || message }}</small><small v-else-if="status" role="status">{{ status }}</small>
</div></template>
<style scoped>.runtime-file-picker{display:flex;flex-direction:column;gap:4px;min-width:0}.runtime-file-controls{display:flex;gap:5px;min-width:0}.runtime-file-controls input{flex:1;min-width:0;border:1px solid #b7c1cd;border-radius:4px;padding:6px 8px;font:inherit}.runtime-file-controls button{border:1px solid #b7c1cd;border-radius:4px;padding:5px 8px;white-space:nowrap;background:#fff;color:inherit;cursor:pointer}.runtime-file-controls button:disabled{opacity:.5;cursor:default}.file-error{color:#b10e1e}small{overflow-wrap:anywhere}</style>
