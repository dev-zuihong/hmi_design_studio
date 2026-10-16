<script setup lang="ts">
import { computed, ref, useId, watch } from 'vue'
import RuntimeFilePicker from './RuntimeFilePicker.vue'
import ValidatedInput from './ValidatedInput.vue'
import { rowCondition, tableRowKey } from '../lib/widget-data'
import type { RuntimeFileRef } from '../lib/runtime-files'
import { Play, SquareStop, Settings, FolderOpen, Download, Save, X, Pencil, RefreshCw } from 'lucide-vue-next'
const icons:Record<string,any>={Play,SquareStop,Settings,FolderOpen,Download,Save,X,Pencil,RefreshCw}
const props = withDefaults(defineProps<{ rows?: any[]; columns?: any[]; rowKey?: string; selectable?: boolean; sortable?: boolean; disabled?: boolean; locale?: string; label?: string; translate?: (value:any)=>any }>(), { rows:()=>[], columns:()=>[], rowKey:'id', selectable:true, sortable:true })
const emit = defineEmits<{ input:[rows:any[]]; event:[name:string,payload:unknown]; selection:[payload:unknown] }>()
const selectionGroup = useId()
const localRows = ref<any[]>([]), sortKey = ref(''), ascending = ref(true), selected = ref<string | number | null>(null)
watch(() => props.rows, value => { localRows.value = Array.isArray(value) ? value.map(row=>({...row})) : [] }, { immediate:true, deep:true })
const rows = computed(() => sortKey.value ? [...localRows.value].sort((a,b)=>String(a[sortKey.value]??'').localeCompare(String(b[sortKey.value]??''),props.locale,{numeric:true})*(ascending.value?1:-1)) : localRows.value)
function key(row:any) { return tableRowKey(row,props.rowKey,localRows.value.indexOf(row)) }
function text(value:any) { return props.translate ? props.translate(value) : String(value ?? '') }
function enabled(row:any, column:any) { return !props.disabled && rowCondition(row,column.enabledField) }
function visible(row:any, item:any) { return rowCondition(row,item.visibleField) }
function select(row:any) { if(props.disabled||!props.selectable)return;selected.value=key(row);const payload={rowKey:key(row),row};emit('selection',payload);emit('event','selectionChange',payload) }
function edit(row:any,column:any,value:unknown,reason?:string) {
  if(!column.editable || !enabled(row,column))return
  const oldValue=row[column.key], rowKey=key(row)
  if(typeof value==='number'&&!Number.isFinite(value))return
  localRows.value=localRows.value.map(item=>item===row?{...item,[column.key]:value}:item)
  emit('input',localRows.value)
  const payload={rowKey,columnKey:column.key,oldValue,newValue:value,...(reason?{reason}:{})}
  emit('event','cellEdit',payload);if(column.type==='file')emit('event','fileChange',payload)
}
function editEvent(row:any,column:any,event:Event) { const target=event.target as HTMLInputElement;edit(row,column,column.type==='number'?Number(target.value):column.type==='boolean'?target.checked:target.value) }
function run(row:any,column:any,item:any) { if(enabled(row,column)&&enabled(row,item)&&visible(row,item))emit('event','rowAction',{rowKey:key(row),actionId:item.id,row}) }
function badge(row:any,column:any) { const value=row[column.key],style=column.badges?.[String(value)];return {...style,label:style?.label??value,color:style?.color??'#616161'} }
function sort(column:any) { if(!props.sortable||column.sortable===false||column.type==='actions')return;if(sortKey.value===column.key)ascending.value=!ascending.value;else{sortKey.value=column.key;ascending.value=true} }
</script>
<template><div class="widget-table-wrap"><div v-if="label" class="widget-panel-title">{{label}}</div><table class="widget-table"><thead><tr><th v-if="selectable"></th><th v-for="column in columns" :key="column.key" :style="{width:column.width?`${column.width}px`:undefined}" :aria-sort="sortKey===column.key?(ascending?'ascending':'descending'):'none'"><button type="button" class="table-heading" :disabled="!sortable||column.sortable===false||column.type==='actions'" @click="sort(column)">{{text(column.title??column.key)}}<span v-if="sortKey===column.key">{{ascending?' ▲':' ▼'}}</span></button></th></tr></thead><tbody><tr v-for="row in rows" :key="key(row)" :class="{selected:selected===key(row)}" :data-row-key="key(row)" @click="select(row)"><td v-if="selectable"><input type="radio" :name="selectionGroup" :checked="selected===key(row)" :aria-label="String(key(row))" :disabled="disabled" @change="select(row)" @click.stop></td><td v-for="column in columns" :key="column.key"><template v-if="visible(row,column)">
  <span v-if="column.type==='actions'" class="table-actions"><button v-for="item in (column.actions??[]).filter((item:any)=>visible(row,item))" :key="item.id" type="button" :disabled="!enabled(row,column)||!enabled(row,item)" :title="text(item.tooltip??item.label)" @click.stop="run(row,column,item)"><component v-if="icons[item.icon]" :is="icons[item.icon]" :size="13"/>{{text(item.label??item.id)}}</button></span>
  <span v-else-if="column.type==='badge'" class="table-badge" :style="{color:badge(row,column).color,backgroundColor:badge(row,column).background}">{{text(badge(row,column).label)}}</span>
  <RuntimeFilePicker v-else-if="column.type==='file'" :model-value="row[column.key] as RuntimeFileRef" :accept="column.accept??''" :read-only="!column.editable" :disabled="!enabled(row,column)" :allow-manual-path="column.allowManualPath" :clearable="column.clearable!==false" :locale="locale" @click.stop @change="edit(row,column,$event.value,$event.reason)"/>
  <select v-else-if="column.editable&&column.type==='enum'" :value="row[column.key]" :disabled="!enabled(row,column)" @click.stop @change="editEvent(row,column,$event)"><option v-for="option in column.options??[]" :key="typeof option==='object'?option.value:option" :value="typeof option==='object'?option.value:option">{{text(typeof option==='object'?option.label:option)}}</option></select>
  <input v-else-if="column.editable&&column.type==='boolean'" type="checkbox" :checked="Boolean(row[column.key])" :disabled="!enabled(row,column)" @click.stop @change="editEvent(row,column,$event)">
  <ValidatedInput v-else-if="column.editable&&column.format" :model-value="row[column.key]" :format="column.format" :pattern="column.pattern" :max-length="column.maxLength" :required="column.required" :disabled="!enabled(row,column)" :locale="locale" commit-mode="change" @click.stop @change="edit(row,column,$event)"/>
  <input v-else-if="column.editable" :type="column.type==='number'?'number':'text'" :value="row[column.key]" :disabled="!enabled(row,column)" @click.stop @change="editEvent(row,column,$event)"><span v-else>{{row[column.key]}}</span>
</template></td></tr></tbody></table></div></template>
<style scoped>.table-heading{border:0;background:transparent;color:inherit;font:inherit;padding:0;text-align:left;width:100%;cursor:pointer}.table-heading:disabled{opacity:1;cursor:default}.table-actions{display:flex;gap:5px;white-space:nowrap}.table-actions button{display:inline-flex;align-items:center;gap:4px;background:#fff;border:1px solid #b7c1cd;border-radius:4px;padding:4px 8px;cursor:pointer}.table-actions button:disabled{opacity:.45;cursor:default}.table-badge{display:inline-block;padding:2px 7px;border-radius:10px;background:#edf2f7;white-space:nowrap;font-size:11px}</style>
