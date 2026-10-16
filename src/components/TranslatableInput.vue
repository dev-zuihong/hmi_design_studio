<script setup lang="ts">
import { computed, ref, watch } from 'vue'
const props=defineProps<{modelValue:unknown;label?:string;placeholder?:string}>()
const emit=defineEmits<{ 'update:modelValue':[value:string|{i18nKey:string;fallback:string}] }>()
const key=ref('')
const fallback=computed(()=>typeof props.modelValue==='object'&&props.modelValue?String((props.modelValue as any).fallback??''):String(props.modelValue??''))
watch(()=>props.modelValue,value=>{key.value=typeof value==='object'&&value?String((value as any).i18nKey??''):''},{immediate:true})
function updateFallback(value:string){emit('update:modelValue',key.value?{i18nKey:key.value,fallback:value}:value)}
function updateKey(value:string){key.value=value;emit('update:modelValue',value?{i18nKey:value,fallback:fallback.value}:fallback.value)}
</script>
<template><div class="translatable-field"><input :aria-label="label" :value="fallback" :placeholder="placeholder" @change="updateFallback(($event.target as HTMLInputElement).value)"><details><summary title="配置翻译词条键">文</summary><input class="translation-key" aria-label="翻译词条键" :value="key" placeholder="如 dashboard.title" @change="updateKey(($event.target as HTMLInputElement).value)"></details></div></template>
<style scoped>.translatable-field{display:flex;align-items:center;gap:4px;min-width:0}.translatable-field>input{flex:1;min-width:0}.translatable-field details{position:relative;flex:0 0 28px}.translatable-field summary{list-style:none;display:grid;place-items:center;height:28px;border:1px solid #d8dbdf;border-radius:4px;cursor:pointer;color:#586b7f}.translation-key{position:absolute;z-index:10;right:0;top:30px;width:220px;box-shadow:0 4px 14px #0002}</style>
