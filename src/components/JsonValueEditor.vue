<script setup lang="ts">
import { ref, watch } from 'vue'
const props=withDefaults(defineProps<{modelValue:unknown;type?:'array'|'object'|'any';disabled?:boolean;label?:string;rows?:number}>(),{type:'any',rows:4})
const emit=defineEmits<{'update:modelValue':[value:unknown]}>()
const draft=ref(''),error=ref('')
watch(()=>`${props.type}:${JSON.stringify(props.modelValue)}`,()=>{draft.value=JSON.stringify(props.modelValue??(props.type==='object'?{}:[]),null,2);error.value=''},{immediate:true})
function commit(){try{const value=JSON.parse(draft.value);if(props.type==='array'&&!Array.isArray(value))throw new Error('需要 JSON 数组');if(props.type==='object'&&(!value||typeof value!=='object'||Array.isArray(value)))throw new Error('需要 JSON 对象');error.value='';emit('update:modelValue',value)}catch(err){error.value=String(err instanceof Error?err.message:err)}}
</script>
<template><div class="json-value-editor"><textarea v-model="draft" :rows="rows" :disabled="disabled" :aria-label="label??'JSON'" :aria-invalid="Boolean(error)" spellcheck="false" @change="commit"/><small v-if="error" role="alert">{{error}}</small></div></template>
<style scoped>.json-value-editor{min-width:0}.json-value-editor textarea{width:100%;box-sizing:border-box;font:11px/1.5 Consolas,monospace;resize:vertical}.json-value-editor small{display:block;color:#b10e1e;overflow-wrap:anywhere}</style>
