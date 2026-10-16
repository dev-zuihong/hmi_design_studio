<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted } from 'vue'
import WidgetView from './WidgetView.vue'
import type { Project, RuntimeEvent, Snapshot, Widget } from '../lib/model'
import { getRootWidgets, getWidgetChildren, widgetBoxStyle, layoutStyle } from '../lib/widgets'
const props = defineProps<{ project: Project; pageId?: string; widgets?: Widget[]; snapshot?: Snapshot; events?: RuntimeEvent[]; editing?: boolean; locale?: string }>()
const emit = defineEmits<{ action: [action: any, value?: unknown]; input: [id: string, value: unknown]; selection: [id: string, payload: unknown] }>()
const page = computed(() => props.project.pages.find(item => item.id === props.pageId) ?? props.project.pages[0]!)
const roots = computed(() => props.widgets ?? getRootWidgets(page.value))
const parentLayout = computed(() => { const parentId = roots.value[0]?.parentId; return parentId ? page.value.widgets.find(item=>item.id===parentId)?.layout : page.value.layout })
function children(widget: Widget, slotId: string) { return getWidgetChildren(page.value, widget.id, slotId) }
function boxStyle(widget:Widget){const style=widgetBoxStyle(widget,parentLayout.value);const flowLayout=parentLayout.value&&parentLayout.value.type!=='absolute'?{...style,position:'relative' as const}:style;return widget.kind==='dialog'?{...flowLayout,zIndex:10000}:flowLayout}
function shortcutKey(event:KeyboardEvent){const parts:string[]=[];if(event.ctrlKey||event.metaKey)parts.push('ctrl');if(event.altKey)parts.push('alt');if(event.shiftKey)parts.push('shift');parts.push(event.key.length===1?event.key.toLocaleLowerCase():event.key.toLocaleLowerCase());return parts.join('+')}
function normalizeShortcut(value:string){return value.toLocaleLowerCase().replace(/command|cmd|meta|⌘/g,'ctrl').replace(/control/g,'ctrl').split('+').map(part=>part.trim()).filter(Boolean).join('+')}
function refersToAction(widget:Widget,actionId:string){if(Object.values(widget.events??{}).some(action=>action.kind==='shared'&&action.actionId===actionId))return true;const ids=[...(widget.props.items??[]),...(widget.props.groups??[]).flatMap((group:any)=>group.actionIds??[])].map((item:any)=>typeof item==='string'?item:item?.id);return ids.includes(actionId)}
function actionHasEnabledWidget(actionId:string){const references=props.project.pages.flatMap(item=>item.widgets).filter(widget=>refersToAction(widget,actionId));if(!references.length)return true;return references.some(widget=>{const enabled=props.snapshot?.ui?.[`${widget.id}.enabled`]??widget.props.enabled,visible=props.snapshot?.ui?.[`${widget.id}.visible`]??widget.props.visible;return enabled!==false&&visible!==false})}
function onShortcut(event:KeyboardEvent){if(props.editing||props.widgets!==undefined)return;const target=event.target as HTMLElement|null;if(target?.closest('input,textarea,select,[contenteditable="true"]'))return;const key=shortcutKey(event);const action=props.project.actions?.find(item=>item.visible!==false&&item.enabled!==false&&item.shortcut&&normalizeShortcut(item.shortcut)===key&&actionHasEnabledWidget(item.id));if(!action)return;event.preventDefault();event.stopImmediatePropagation();emit('action',action.action)}
onMounted(()=>window.addEventListener('keydown',onShortcut,true))
onBeforeUnmount(()=>window.removeEventListener('keydown',onShortcut,true))
</script>
<template>
  <div class="widget-tree-root" :style="layoutStyle(parentLayout)">
    <div v-for="widget in roots" :key="widget.id" class="runtime-widget" :data-widget-id="widget.id" :style="boxStyle(widget)">
      <WidgetView :widget="widget" :project="project" :snapshot="snapshot" :events="events" :editing="editing" :locale="locale" @action="(action,value)=>emit('action',action,value)" @input="(id,value)=>emit('input',id,value)" @selection="(id,value)=>emit('selection',id,value)">
        <template #default="{slotId}"><WidgetTree :project="project" :page-id="page.id" :widgets="children(widget,slotId)" :snapshot="snapshot" :events="events" :editing="editing" :locale="locale" @action="(action,value)=>emit('action',action,value)" @input="(id,value)=>emit('input',id,value)" @selection="(id,value)=>emit('selection',id,value)" /></template>
      </WidgetView>
    </div>
  </div>
</template>
