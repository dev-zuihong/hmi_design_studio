<script setup lang="ts">
import {ref} from 'vue'
import ValueEditor from './ValueEditor.vue'
import type {Project,ValueRef} from '../lib/model'
const props=defineProps<{modelValue:Record<string,ValueRef>;project:Project}>()
const emit=defineEmits<{'update:modelValue':[Record<string,ValueRef>]}>()
const name=ref('')
function set(key:string,value?:ValueRef){const next={...props.modelValue};if(value)next[key]=value;else delete next[key];emit('update:modelValue',next)}
function add(){const key=name.value.trim();if(!key||['__proto__','constructor','prototype'].includes(key))return;set(key,{source:'literal',value:''});name.value=''}
</script>
<template><div class="flow-args"><label v-for="(value,key) in modelValue" :key="key"><span>{{key}} <button type="button" class="text-button" @click="set(String(key))">删除</button></span><ValueEditor :model-value="value" :project="project" @update:model-value="set(String(key),$event)"/></label><div class="flow-arg-add"><input v-model="name" placeholder="参数名称" @keydown.enter.prevent="add"/><button type="button" @click="add">添加参数</button></div></div></template>
<style scoped>.flow-arg-add{display:flex;gap:6px}.flow-arg-add input{min-width:0;flex:1}.flow-arg-add button{white-space:nowrap}.flow-args label>span{display:flex;justify-content:space-between}</style>
