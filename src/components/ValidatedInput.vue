<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import BaseInput from './base/BaseInput.vue'
import { validateText } from '../lib/widget-data'
const props=withDefaults(defineProps<{modelValue?:unknown;placeholder?:string;disabled?:boolean;readOnly?:boolean;format?:string;pattern?:string;maxLength?:number;required?:boolean;commitMode?:string;locale?:string}>(),{commitMode:'input'})
const emit=defineEmits<{change:[value:string]}>()
const draft=ref(String(props.modelValue??'')),last=ref(draft.value)
watch(()=>props.modelValue,value=>{draft.value=String(value??'');last.value=draft.value})
const error=computed(()=>validateText(draft.value,props))
const english:Record<string,string>={'请输入值':'A value is required','超过最大长度':'Maximum length exceeded','请输入完整的十六进制字节':'Enter complete hexadecimal bytes','请输入 0–4294967295 的十进制地址或 0x 开头的十六进制地址':'Enter a decimal address from 0 to 4294967295 or hexadecimal prefixed with 0x','输入格式不匹配':'Input format does not match','校验规则无效':'Invalid validation rule'}
function commit(){if(!error.value&&!props.disabled&&!props.readOnly&&draft.value!==last.value){last.value=draft.value;emit('change',draft.value)}}
function input(value:string){if(props.readOnly)return;draft.value=value;if(props.commitMode==='input')commit()}
</script>
<template><div class="validated-input" @focusout="commitMode==='change'&&commit()" @keydown.enter.prevent="commit" @keydown.esc="draft=last"><BaseInput :model-value="draft" :placeholder="placeholder" :disabled="disabled" :readonly.prop="readOnly" :maxlength="maxLength||undefined" :aria-invalid="Boolean(error)" @update:model-value="input"/><small v-if="error" role="status">{{locale?.startsWith('en')?english[error]??error:error}}</small></div></template>
<style scoped>.validated-input{display:flex;flex-direction:column;gap:4px;min-width:0}.validated-input input{width:100%;min-width:0}.validated-input small{color:#b10e1e;font-size:11px}</style>
