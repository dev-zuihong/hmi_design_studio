<script setup lang="ts">
import { computed } from 'vue'
import WidgetView from './WidgetView.vue'
import type { Editor } from '../lib/editor'
import type { Widget } from '../lib/model'
import { getWidgetChildren, layoutStyle, layoutItemStyle } from '../lib/widgets'
const props=defineProps<{editor:Editor;widget:Widget}>()
const emit=defineEmits<{select:[event:MouseEvent,id:string];context:[event:MouseEvent,id:string]}>()
const parentLayout=computed(()=>props.widget.parentId?props.editor.page.value.widgets.find(widget=>widget.id===props.widget.parentId)?.layout:props.editor.page.value.layout)
const managed=computed(()=>parentLayout.value?.type&&parentLayout.value.type!=='absolute')
const style=computed(()=>managed.value?{...layoutItemStyle(props.widget.layoutItem,parentLayout.value),position:'relative' as const,width:props.widget.layoutItem?.grow?'auto':`${props.widget.width}px`,height:parentLayout.value?.type==='vertical'&&props.widget.layoutItem?.grow?'auto':`${props.widget.height}px`,zIndex:props.widget.zIndex}: {position:'absolute' as const,transform:`translate(${props.widget.x}px,${props.widget.y}px)`,width:`${props.widget.width}px`,height:`${props.widget.height}px`,zIndex:props.widget.zIndex})
function dragStart(event:DragEvent){event.stopPropagation();event.dataTransfer?.setData('application/hmi-existing-widget',props.widget.id)}
</script>
<template><div v-if="!widget.editor?.hidden" :data-widget-id="widget.id" class="design-widget" :class="{selected:editor.selected.value.includes(widget.id),locked:widget.editor?.locked,'layout-managed':managed}" :style="style" :draggable="!!managed&&!widget.editor?.locked" @dragstart="dragStart" @mousedown.stop="emit('select',$event,widget.id)" @contextmenu.stop.prevent="emit('context',$event,widget.id)"><WidgetView :widget="widget" :project="editor.project.value" editing :editor-slot="editor.activeSlots.value[widget.id]" @slot-change="(id,slotId)=>editor.activeSlots.value[id]=slotId"><template #default="{slotId}"><div class="design-container-content" :data-container-content="widget.id" :data-slot-id="slotId" :style="layoutStyle(widget.layout)"><DesignWidget v-for="child in getWidgetChildren(editor.page.value,widget.id,slotId)" :key="child.id" :widget="child" :editor="editor" @select="(event,id)=>emit('select',event,id)" @context="(event,id)=>emit('context',event,id)"/></div></template></WidgetView><span v-if="editor.selected.value.includes(widget.id)" class="widget-label">{{ widget.name }}{{ widget.editor?.locked?' 🔒':'' }}</span></div></template>
<style scoped>.design-container-content{position:relative;min-width:0;min-height:0;width:100%;height:100%}.design-widget.locked{outline:1px dashed #ac8b46}.layout-managed{flex-shrink:1}.design-widget{overflow:visible}</style>
