<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { CSSProperties } from 'vue'
import BaseButton from './base/BaseButton.vue'
import BaseCheckbox from './base/BaseCheckbox.vue'
import ValidatedInput from './ValidatedInput.vue'
import RuntimeFilePicker from './RuntimeFilePicker.vue'
import ByteRangePicker from './ByteRangePicker.vue'
import MemoryMapView from './MemoryMapView.vue'
import RuntimeTable from './RuntimeTable.vue'
import StructuredLog from './StructuredLog.vue'
import BaseSwitch from './base/BaseSwitch.vue'
import BaseSelect from './base/BaseSelect.vue'
import { Activity, AlertCircle, Check, CircleDot, Download, FileText, FolderOpen, Link2, Play, Power, RefreshCcw, Save, Search, Settings, SquareStop, Trash2, X, Zap } from 'lucide-vue-next'
import type { Project, RuntimeEvent, Snapshot, UiAction, Widget } from '../lib/model'
import { assetUrls, assetFonts } from '../lib/resources'
import { isContainer, resolveRuntimePath } from '../lib/widgets'
import { resolveWidgetStyle } from '../lib/design-system'
import { projectLocale, projectText } from '../lib/i18n'
import './widget-style.css'
const props = defineProps<{ widget: Widget; project: Project; snapshot?: Snapshot; events?: RuntimeEvent[]; editing?: boolean; locale?: string; editorSlot?:string }>()
const emit = defineEmits<{ action: [action: UiAction, value?: unknown]; input: [id: string, value: unknown]; selection: [id: string, payload: unknown]; slotChange:[id:string,slotId:string] }>()
const actionIconMap:Record<string,any>={Activity,AlertCircle,Check,CircleDot,Download,FileText,FolderOpen,Link2,Play,Power,RefreshCcw,Save,Search,Settings,SquareStop,Trash2,X,Zap}
const values = computed<Record<string, any>>(() => {
  const result: Record<string, any> = { ...props.widget.props }
  try { Object.assign(result, resolveWidgetStyle(props.project, props.widget)) } catch { /* project validation reports invalid token references */ }
  for(const [key,custom] of Object.entries(props.widget.customProperties??{}))result[key]=custom.value
  for(const [key,value] of Object.entries(props.snapshot?.ui??{})){if(key.startsWith(`${props.widget.id}.`))result[key.slice(props.widget.id.length+1)]=value}
  Object.assign(result, props.snapshot?.ui?.[props.widget.id] ?? {})
  for (const [property, binding] of Object.entries(props.widget.bindings ?? {})) {
    result[property] = resolveRef(binding, new Set([`${props.widget.id}.${property}`]))
  }
  for (const [key, custom] of Object.entries(props.widget.customProperties ?? {})) if(custom.binding)result[key]=resolveRef(custom.binding,new Set([`${props.widget.id}.${key}`]))
  for (const key of ['text','placeholder','tooltip','unit']) if(result[key]!==undefined) result[key]=translate(result[key])
  if(Array.isArray(result.options)) result.options=result.options.map((item:any)=>typeof item==='string'?translate(item):{...item,label:translate(item.label ?? item.text ?? item.value)})
  if(Array.isArray(result.columns)) result.columns=result.columns.map((column:any)=>({...column,title:translate(column.title ?? column.label ?? column.key),options:column.options?.map((option:any)=>typeof option==='string'?translate(option):{...option,label:translate(option.label ?? option.value)})}))
  result.slots=(props.widget.slots??[]).map((slot:any)=>({...slot,title:translate(slot.title)}))
  if(Array.isArray(result.groups))result.groups=result.groups.map((group:any)=>({...group,title:translate(group.title)}))
  return result
})
function translate(value: any): any { const locale=props.locale??projectLocale.value;if(typeof value==='string'&&props.project.i18n?.messages[locale]?.[value]!==undefined)return projectText({i18nKey:value,fallback:value},props.project,locale);return projectText(value, props.project, locale) }
const systemEnglish:Record<string,string>={'绑定数据后实时更新':'Updates when data is bound','当前读数':'Current reading','数据趋势将在运行时显示':'Trend appears while running','等待数据采集':'Waiting for data','历史':'History','实时':'Live','运行记录将在这里显示':'Runtime logs appear here','未知控件':'Unknown widget'}
function systemText(source:string){const locale=props.locale??projectLocale.value, fallback=locale.toLowerCase().startsWith('en')?systemEnglish[source]??source:source;return projectText({i18nKey:`$system.${source}`,fallback},props.project,locale)}
function findWidget(id: string) { return props.project.pages.flatMap(page=>page.widgets).find(widget=>widget.id===id) }
function snapshotProperty(widgetId: string, property: string): { found: boolean; value?: unknown } {
  const ui = props.snapshot?.ui ?? {}
  const flatKey = `${widgetId}.${property}`
  if (Object.prototype.hasOwnProperty.call(ui, flatKey)) return { found: true, value: ui[flatKey] }
  const nested = ui[widgetId]
  if (nested && typeof nested === 'object' && Object.prototype.hasOwnProperty.call(nested, property)) {
    return { found: true, value: (nested as Record<string, unknown>)[property] }
  }
  return { found: false }
}
function resolveWidgetProperty(widgetId: string, property: string, visited: Set<string>): any {
  const token=`${widgetId}.${property}`
  if(visited.has(token)) return undefined
  visited.add(token)
  const target=findWidget(widgetId)
  if(!target) return undefined
  const custom=target.customProperties?.[property]
  if(custom?.binding) return resolveRef(custom.binding,visited)
  const binding=target.bindings?.[property]
  if(binding) return resolveRef(binding,visited)
  const ui = snapshotProperty(widgetId, property)
  if(ui.found) return ui.value
  if(custom&&Object.prototype.hasOwnProperty.call(custom,'value'))return custom.value
  if(Object.prototype.hasOwnProperty.call(target.props,property))return target.props[property]
  return undefined
}
function resolveRef(ref: NonNullable<Widget['bindings'][string]>, visited=new Set<string>()) {
  if (ref.source === 'literal') return ref.value
  if (ref.source === 'backend') return resolveRuntimePath(props.snapshot?.backends?.[ref.key],ref.path,props.snapshot?.context)
  if (ref.source === 'context') return resolveRuntimePath(props.snapshot?.context,ref.path)
  if (ref.source === 'variable') return resolveRuntimePath(props.snapshot?.variables[ref.key] ?? props.project.variables.find(v => v.id === ref.key)?.initial, ref.path, props.snapshot?.context)
  if (ref.source === 'runtime') return resolveRuntimePath(props.snapshot?.runtime, ref.path)
  if (ref.source === 'input') return props.snapshot?.inputs?.[ref.key] ?? props.snapshot?.ui?.[`${ref.key}.value`] ?? props.snapshot?.ui?.[ref.key]?.value ?? props.snapshot?.ui?.[ref.key]?.input
  if (ref.source === 'property') return resolveWidgetProperty(ref.key,ref.property,visited)
  if (ref.source === 'point') { const point=props.snapshot?.points[ref.key];return point&&typeof point==='object'&&'value' in point?(point.valid===false?undefined:point.value):point }
  return undefined
}
const valueIsBound = () => Object.prototype.hasOwnProperty.call(props.widget.bindings ?? {}, 'value') || !!props.widget.customProperties?.value?.binding
function snapshotInput() {
  const inputs = props.snapshot?.inputs ?? {}
  return Object.prototype.hasOwnProperty.call(inputs, props.widget.id)
    ? { found: true, value: inputs[props.widget.id] }
    : { found: false, value: undefined }
}
const runtimeScope = computed(() => JSON.stringify([props.snapshot?.context?.windowInstanceId ?? '', props.snapshot?.context?.deviceKey ?? '']))
const inputValue = computed(() => {
  if (valueIsBound()) return values.value.value
  const persisted = snapshotInput()
  if (persisted.found) return persisted.value
  return values.value.value
})
const input = ref<unknown>(inputValue.value === undefined ? '' : inputValue.value)
const slotIds = computed(() => (values.value.slots ?? props.widget.slots ?? []).map((slot:any) => String(slot.id)))
const activeSlotValue = computed(() => {
  const ids = slotIds.value
  if (props.editing && props.editorSlot && ids.includes(props.editorSlot)) return props.editorSlot
  let requested: unknown
  if (valueIsBound()) requested = values.value.value
  else {
    const persisted = snapshotInput()
    if (persisted.found && ids.includes(String(persisted.value))) requested = persisted.value
    else requested = values.value.activeSlot
  }
  return ids.includes(String(requested ?? '')) ? String(requested) : ids[0] ?? 'content'
})
const activeSlot = ref(activeSlotValue.value)
watch(activeSlotValue, value => { activeSlot.value = value }, { immediate: true })
const openMenuGroup=ref<{id:string;x:number;y:number}|null>(null)
const menuPosition=computed(()=>openMenuGroup.value??{id:'',x:0,y:0})
function closeMenuOutside(event:PointerEvent){const target=event.target as HTMLElement|null;if(!target?.closest('.widget-menu-popup,.widget-menu-toggle'))openMenuGroup.value=null}
function toggleMenuGroup(event:MouseEvent,id:string){if(openMenuGroup.value?.id===id){openMenuGroup.value=null;return}const rect=(event.currentTarget as HTMLElement).getBoundingClientRect();openMenuGroup.value={id,x:rect.left,y:rect.bottom};requestAnimationFrame(()=>{const menu=document.querySelector<HTMLElement>(`.widget-menu-popup[data-menu-id="${CSS.escape(id)}"]`);if(menu&&openMenuGroup.value?.id===id){const popup=menu.getBoundingClientRect();openMenuGroup.value={id,x:Math.max(6,Math.min(rect.left,window.innerWidth-popup.width-6)),y:Math.max(6,Math.min(rect.bottom,window.innerHeight-popup.height-6))}}})}
function runMenuAction(id:string){runShared(id);openMenuGroup.value=null}
onMounted(()=>document.addEventListener('pointerdown',closeMenuOutside,true))
onBeforeUnmount(()=>document.removeEventListener('pointerdown',closeMenuOutside,true))
const dialogOpen=ref(values.value.open!==false)
watch(()=>values.value.open,value=>{if(value!==undefined)dialogOpen.value=Boolean(value)})
watch(()=>props.snapshot?.ui?.[`${props.widget.id}.open`],value=>{if(value!==undefined)dialogOpen.value=Boolean(value)})
const splitterRatio=ref(Number(values.value.ratio??.5))
watch(()=>values.value.ratio,value=>{if(value!==undefined)splitterRatio.value=Number(value)})
function resizeSplitter(event:PointerEvent){if(props.editing)return;const handle=event.currentTarget as HTMLElement,root=handle.parentElement;if(!root)return;event.preventDefault();handle.setPointerCapture(event.pointerId);const vertical=String(values.value.orientation??'horizontal')==='vertical';const move=(moveEvent:PointerEvent)=>{const rect=root.getBoundingClientRect(),fraction=vertical?(moveEvent.clientY-rect.top)/rect.height:(moveEvent.clientX-rect.left)/rect.width;splitterRatio.value=Math.max(.05,Math.min(.95,fraction))};const done=()=>{handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',done);handle.removeEventListener('pointercancel',done);action('resize',{ratio:splitterRatio.value})};handle.addEventListener('pointermove',move);handle.addEventListener('pointerup',done);handle.addEventListener('pointercancel',done)}
function closeDialog(){dialogOpen.value=false;action('close')}
watch(inputValue, value => { input.value = value === undefined ? '' : value })
const resolvedTokens = computed(() => { try { return resolveWidgetStyle(props.project, props.widget) } catch { return {} } })
const style = computed(() => ({ ...resolvedTokens.value, ...({ color: values.value.color ?? resolvedTokens.value.color as string | undefined, fontSize: `${values.value.fontSize ?? resolvedTokens.value.fontSize ?? 14}px`, fontFamily: assetFonts.value[values.value.fontAssetId] || values.value.fontFamily || undefined, backgroundColor: ['group','frame','scroll','tabs','stack','splitter','dialog','number','chart','log','table','list','tree'].includes(props.widget.kind) ? values.value.background ?? resolvedTokens.value.background as string | undefined : undefined, backgroundImage: assetUrls.value[values.value.backgroundAssetId] ? `url("${assetUrls.value[values.value.backgroundAssetId]}")` : undefined, backgroundSize: 'cover', backgroundPosition: 'center', borderColor: values.value.borderColor ?? resolvedTokens.value.borderColor as string | undefined, borderWidth: `${values.value.borderWidth ?? 0}px`, borderStyle: (values.value.borderWidth ?? 0) ? 'solid' : undefined, borderRadius: `${values.value.borderRadius ?? resolvedTokens.value.borderRadius ?? 4}px`, padding: `${isContainer(props.widget) ? 0 : values.value.padding ?? 0}px`, textAlign: values.value.align ?? 'left' } as Record<string, string | undefined>) } as CSSProperties))
const controlStyle = computed(() => ({ color: values.value.color ?? resolvedTokens.value.color, backgroundColor: values.value.background ?? resolvedTokens.value.background, borderColor: values.value.borderColor ?? resolvedTokens.value.borderColor, fontSize: values.value.fontSize != null ? `${values.value.fontSize}px` : undefined, fontFamily: values.value.fontFamily, borderRadius: values.value.borderRadius != null ? `${values.value.borderRadius}px` : undefined, boxShadow: values.value.boxShadow }))
const number = computed(() => { const value = values.value.value; if(value===null||value===undefined||value===''||!Number.isFinite(Number(value)))return '—';const precision=Math.max(0,Math.min(8,Number(values.value.precision??1))),locale=props.locale??projectLocale.value;return new Intl.NumberFormat(locale,{minimumFractionDigits:precision,maximumFractionDigits:precision}).format(Number(value)) })
const samples = ref<number[]>([])
watch(() => values.value.value, value => { if (props.editing || value === undefined || value === null || value === '') return; const numeric = Number(value); if (Number.isFinite(numeric)) { samples.value.push(numeric); if (samples.value.length > 120) samples.value.shift() } })
const series = computed(() => { const data = samples.value; if (data.length < 2) return ''; const min = Math.min(...data) - 1, max = Math.max(...data) + 1; return data.map((v, i) => `${(i / (data.length - 1)) * 560 + 8},${130 - ((v - min) / (max - min)) * 112}`).join(' ') })
const listItems = computed<any[]>(() => Array.isArray(values.value.items) ? values.value.items.map(translateTreeItem) : [])
function translateTreeItem(item:any):any { if(typeof item==='string')return translate(item); if(!item||typeof item!=='object')return item; const labelKey='label' in item?'label':'text' in item?'text':undefined; return {...item,...(labelKey?{[labelKey]:translate(item[labelKey])}:{}),...(Array.isArray(item.children)?{children:item.children.map(translateTreeItem)}:{})} }
const rowsValue = computed(() => {
  if (valueIsBound()) return Array.isArray(values.value.value) ? values.value.value : values.value.rows
  const persisted = snapshotInput()
  return persisted.found && Array.isArray(persisted.value) ? persisted.value : values.value.rows
})
const selectedItem = ref<string | null>(null)
function itemLabel(item: any) { if(typeof item==='object'&&item&&'i18nKey' in item)return translate(item); return translate(typeof item === 'string' ? item : item?.label ?? item?.name ?? item?.id ?? '') }
function itemKey(item: any, index: number) { return String(item?.id ?? item?.key ?? index) }
function optionValue(option:any) { return String(typeof option==='string'?option:option?.value ?? option?.id ?? option?.label ?? '') }
function optionLabel(option:any) { return translate(typeof option==='string'?option:option?.label ?? option?.text ?? option?.value ?? '') }
function action(event: string, value?: unknown) { const direct = props.widget.events?.[event]; const sharedId = direct?.kind === 'shared' ? direct.actionId : undefined; const shared = sharedId ? props.project.actions?.find(item => item.id === sharedId) : undefined; const resolved = shared?.action ?? direct; if (resolved && shared?.enabled !== false && shared?.visible !== false && !props.editing) emit('action', resolved, value) }
function runShared(id: string) { const shared = props.project.actions?.find(item => item.id === id); if (shared?.action && shared.enabled !== false && shared.visible !== false && !props.editing) emit('action', shared.action) }
function actionLabel(id: string) { const action = props.project.actions?.find(item => item.id === id); return projectText(action?.text ?? id, props.project, props.locale ?? projectLocale.value) }
function actionEnabled(id: string) { return props.project.actions?.find(item=>item.id===id)?.enabled!==false }
function actionVisible(id:string){return props.project.actions?.find(item=>item.id===id)?.visible!==false}
function visibleActions(items:unknown[]){return items.map(item=>typeof item==='string'?item:(item as any)?.id).filter((id):id is string=>typeof id==='string'&&actionVisible(id))}
function actionIcon(id?:string){const name=id?props.project.actions?.find(item=>item.id===id)?.icon:undefined;return name?actionIconMap[name]:undefined}
function widgetIcon(event:string){const configured=props.widget.events?.[event];return configured?.kind==='shared'?actionIcon(configured.actionId):actionIconMap[values.value.icon]}
function update(value: unknown, event = 'change') { input.value = value; if (!props.editing) emit('input', props.widget.id, value); action(event, value) }
function updateSelection(value: string) { const option = (values.value.options ?? []).find((item: any) => optionValue(item) === value); update(option && typeof option === 'object' ? option.value : value) }
function selectItem(item: any, index: number) { selectedItem.value = itemKey(item,index); const payload = { key: selectedItem.value, value: item }; emit('selection', props.widget.id, payload); action('selectionChange', payload) }
function setSlot(slot: string) { activeSlot.value = slot; emit('slotChange',props.widget.id,slot); if (!props.editing) update(slot,'tabChange') }
function setInput(value:unknown) { input.value=value;if(!props.editing)emit('input',props.widget.id,value) }
function changeFile(payload:any){setInput(payload.value);action('change',payload)}
const progressFraction=computed(()=>{const min=Number(values.value.min??0),max=Number(values.value.max??100),value=Number(values.value.value??0);return Number.isFinite(value)&&max>min?Math.max(0,Math.min(1,(value-min)/(max-min))):0})
const logEntries=computed(()=>values.value.source==='entries'?(Array.isArray(values.value.entries)?values.value.entries:[]):props.events??[])
const tabsLike = computed(() => ['tabs','stack'].includes(props.widget.kind))
const actionHidden = computed(()=>Object.values(props.widget.events??{}).some(event=>event.kind==='shared'&&props.project.actions?.find(item=>item.id===event.actionId)?.visible===false))
const widgetVisible=computed(()=>values.value.visible!==false&&!actionHidden.value)
function actionEnabledFor(event:string){const action=props.widget.events?.[event];if(action?.kind!=='shared')return true;return props.project.actions?.find(item=>item.id===action.actionId)?.enabled!==false}
const disabledSharedAction=computed(()=>Object.values(props.widget.events??{}).some(event=>event.kind==='shared'&&props.project.actions?.find(item=>item.id===event.actionId)?.enabled===false))
const runtimeBlocked=computed(()=>!props.editing&&props.snapshot?.runtime&&props.snapshot.runtime.state!=='running'&&!isContainer(props.widget)&&(['button','tool-button','input','textarea','select','switch','checkbox','radio','spinbox','slider','date','time','datetime','table','file-picker','byte-range','menu','toolbar'].includes(props.widget.kind))&&!(props.snapshot.runtime.state!=='starting'&&Object.values(props.widget.events??{}).some(action=>(action.kind==='shared'?props.project.actions?.find(item=>item.id===action.actionId)?.action:action)?.kind==='runtimeRetry')))
const widgetDisabled=computed(()=>values.value.enabled===false||disabledSharedAction.value||runtimeBlocked.value)
const changeDisabled=computed(()=>widgetDisabled.value||!actionEnabledFor('change'))
watch(runtimeScope, () => { input.value = inputValue.value; activeSlot.value = activeSlotValue.value; selectedItem.value = null; dialogOpen.value = values.value.open !== false; openMenuGroup.value = null })
</script>

<template>
  <div v-if="editing || widgetVisible" class="widget-content" :class="[`widget-${widget.kind}`, { 'widget-disabled': widgetDisabled, 'widget-container': isContainer(widget), 'widget-tabs-left': tabsLike && values.tabPlacement==='left' }]" :style="style" :inert="(!editing && widgetDisabled) || (editing && !isContainer(widget))">
    <span v-if="widget.kind === 'text'">{{ values.text }}</span>
    <BaseButton v-else-if="widget.kind === 'button' || widget.kind === 'tool-button'" :style="controlStyle" :appearance="values.appearance === 'accent' ? 'primary' : values.appearance" :disabled="changeDisabled || !actionEnabledFor('click')" :title="values.tooltip" @click="action('click')"><component v-if="widgetIcon('click')" :is="widgetIcon('click')" :size="15"/><span v-else-if="widget.kind==='tool-button'&&values.icon">{{values.icon}}</span>{{ values.text }}</BaseButton>
    <ValidatedInput v-else-if="widget.kind === 'input'" :key="runtimeScope" :model-value="input" :placeholder="values.placeholder || values.text" :disabled="changeDisabled" :read-only="values.readOnly" :format="values.format" :pattern="values.pattern" :required="values.required" :max-length="values.maxLength" :commit-mode="values.commitMode" :locale="locale??projectLocale" @change="update" />
    <RuntimeFilePicker v-else-if="widget.kind === 'file-picker'" :key="runtimeScope" :model-value="input as any" :accept="values.accept" :allow-manual-path="values.allowManualPath" :clearable="values.clearable" :disabled="changeDisabled" :read-only="values.readOnly" :status="values.status" :message="values.message" :locale="locale??projectLocale" @change="changeFile"/>
    <ByteRangePicker v-else-if="widget.kind === 'byte-range'" :key="runtimeScope" :model-value="input as any" :byte-length="Number(values.byteLength)" :allow-empty="values.allowEmpty" :disabled="changeDisabled" :read-only="values.readOnly" :locale="locale??projectLocale" @change="update"/>
    <MemoryMapView v-else-if="widget.kind === 'memory-map'" :range-start="Number(values.rangeStart)" :range-size="Number(values.rangeSize)" :segments="values.segments" :locale="locale??projectLocale"/>
    <textarea v-else-if="widget.kind === 'textarea'" class="widget-native-input" :value="String(input ?? '')" :placeholder="values.placeholder" :rows="values.rows ?? 4" :disabled="changeDisabled" @input="update(($event.target as HTMLTextAreaElement).value)" />
    <BaseCheckbox v-else-if="widget.kind === 'checkbox'" class="widget-check" :model-value="Boolean(input)" :disabled="changeDisabled" @update:model-value="update">{{ values.text }}</BaseCheckbox>
    <fieldset v-else-if="widget.kind === 'radio'" class="widget-radio"><legend>{{ values.text }}</legend><label v-for="option in values.options ?? []" :key="optionValue(option)"><input type="radio" :name="widget.id" :value="optionValue(option)" :checked="input === optionValue(option)" :disabled="changeDisabled" @change="update(optionValue(option))">{{ optionLabel(option) }}</label></fieldset>
    <BaseSelect v-else-if="widget.kind === 'select'" :model-value="String(input ?? '')" :options="values.options ?? []" :placeholder="values.placeholder || values.text" :disabled="changeDisabled" @update:model-value="updateSelection" />
    <BaseSwitch v-else-if="widget.kind === 'switch'" :model-value="Boolean(input)" :disabled="changeDisabled" @update:model-value="update">{{ values.text }}</BaseSwitch>
    <label v-else-if="widget.kind === 'spinbox'" class="widget-labeled-input"><span>{{ values.text }}</span><input class="widget-native-input" type="number" :value="Number(input ?? 0)" :min="values.min" :max="values.max" :step="values.step ?? 1" :disabled="changeDisabled" @input="update(Number(($event.target as HTMLInputElement).value))"></label>
    <label v-else-if="widget.kind === 'slider'" class="widget-slider"><span>{{ values.text }}</span><input type="range" :value="Number(input ?? values.min ?? 0)" :min="values.min ?? 0" :max="values.max ?? 100" :step="values.step ?? 1" :disabled="changeDisabled" @input="update(Number(($event.target as HTMLInputElement).value))"><output>{{ input }}</output></label>
    <label v-else-if="['date','time','datetime'].includes(widget.kind)" class="widget-labeled-input"><span>{{ values.text }}</span><input class="widget-native-input" :type="widget.kind === 'datetime' ? 'datetime-local' : widget.kind" :value="String(input ?? '')" :disabled="changeDisabled" @input="update(($event.target as HTMLInputElement).value)"></label>
    <img v-else-if="widget.kind === 'image'" class="widget-image" :src="assetUrls[values.assetId] ?? assetUrls[values.backgroundAssetId]" :alt="values.text" :style="{objectFit: values.fit ?? 'contain'}">
    <div v-else-if="widget.kind === 'number'" class="metric"><span class="metric-label">{{ values.text }}</span><div class="metric-value">{{ number }} <span class="metric-unit">{{ values.unit }}</span></div><span class="metric-caption">{{ systemText(editing ? '绑定数据后实时更新' : '当前读数') }}</span></div>
    <div v-else-if="widget.kind === 'indicator'" class="indicator"><span class="indicator-dot" :style="{backgroundColor: values.value ? values.onColor : values.offColor}" />{{ values.text }}</div>
    <div v-else-if="widget.kind === 'progress'" class="widget-progress"><div class="widget-panel-title">{{ values.text }}<span v-if="values.showValue !== false && !values.indeterminate">{{ Math.round(progressFraction*100) }}%</span></div><progress :value="values.indeterminate?undefined:progressFraction" :max="1" :aria-label="values.text" /></div>
    <div v-else-if="widget.kind === 'chart'" class="chart"><div class="widget-panel-title">{{ values.text }}<span>{{ values.unit }}</span></div><div class="chart-area"><div class="chart-grid" /><svg viewBox="0 0 576 146" preserveAspectRatio="none" :aria-label="systemText('实时')"><polyline v-if="series" :points="series" fill="none" stroke="#0f6cbd" stroke-width="2" vector-effect="non-scaling-stroke" /></svg><span v-if="!series" class="chart-empty">{{ systemText(editing ? '数据趋势将在运行时显示' : '等待数据采集') }}</span></div><div class="chart-axis"><span>{{ systemText('历史') }}</span><span>{{ systemText('实时') }}</span></div></div>
    <div v-else-if="widget.kind === 'log'" class="runtime-log"><div class="widget-panel-title">{{ values.text }}</div><StructuredLog :entries="logEntries" :max-lines="values.maxLines" :auto-scroll="values.autoScroll!==false" :wrap="values.wrap!==false" :level="values.level" :device-key="values.deviceKey" :locale="locale??projectLocale"/></div>
    <div v-else-if="widget.kind === 'list'" class="widget-list"><div class="widget-panel-title">{{ values.text }}</div><button v-for="(item,index) in listItems" :key="itemKey(item,index)" :class="{selected:selectedItem===itemKey(item,index)}" @click="selectItem(item,index)">{{ itemLabel(item) }}</button></div>
    <div v-else-if="widget.kind === 'tree'" class="widget-tree"><div class="widget-panel-title">{{ values.text }}</div><TreeItems :items="listItems" :selected="selectedItem" @select="selectItem" /></div>
    <RuntimeTable v-else-if="widget.kind === 'table'" :key="runtimeScope" :rows="rowsValue" :columns="values.columns" :row-key="values.rowKey" :selectable="values.selectable!==false" :sortable="values.sortable!==false" :disabled="widgetDisabled" :label="values.text" :locale="locale??projectLocale" :translate="translate" @input="setInput" @event="action" @selection="emit('selection',widget.id,$event)"/>
    <div v-else-if="widget.kind === 'group' || widget.kind === 'frame'" class="widget-container-caption">{{ values.text }}</div>
    <div v-else-if="tabsLike" class="widget-tabs" :class="`tabs-${widget.kind}`" :style="values.tabPlacement==='left'?{width:`${values.tabWidth??120}px`}:undefined"><div class="widget-tabs-header"><button v-for="slot in values.slots ?? widget.slots ?? []" :key="slot.id" :class="{active:activeSlot===slot.id}" @click="setSlot(slot.id)">{{ slot.title }}</button></div></div>
    <div v-else-if="widget.kind === 'splitter'" class="widget-splitter" :class="`splitter-${values.orientation ?? 'horizontal'}`"><span>{{ values.text }}</span></div>
    <div v-else-if="widget.kind === 'dialog'" v-show="editing || dialogOpen" class="widget-dialog-overlay" :class="{'dialog-modal':values.modal!==false&&!editing,'dialog-design':editing}" @click.self="values.modal!==false&&!editing&&closeDialog()"><section class="widget-dialog-surface" role="dialog" :aria-modal="values.modal!==false" :aria-label="values.text" @keydown.esc="closeDialog"><header><strong>{{ values.text }}</strong><button type="button" aria-label="关闭对话框" @click="closeDialog">×</button></header><div class="widget-dialog-content"><slot slot-id="content" /></div></section></div>
    <nav v-else-if="widget.kind === 'menu'" class="widget-menu"><template v-if="values.groups?.length || values.items?.length"><template v-for="group in values.groups ?? []" :key="group.id"><button class="widget-menu-toggle" :disabled="widgetDisabled" :aria-expanded="menuPosition.id===group.id" aria-haspopup="menu" @click="toggleMenuGroup($event,group.id)">{{ group.title }}<span aria-hidden="true">⌄</span></button><Teleport to="body"><div v-if="menuPosition.id===group.id" class="widget-menu-popup" role="menu" :data-menu-id="group.id" :style="{left:`${menuPosition.x}px`,top:`${menuPosition.y}px`}" @pointerdown.stop @keydown.esc.stop.prevent="openMenuGroup=null"><button v-for="actionId in visibleActions(group.actionIds ?? [])" :key="actionId" role="menuitem" :disabled="widgetDisabled||!actionEnabled(actionId)" @click="runMenuAction(actionId)"><component v-if="actionIcon(actionId)" :is="actionIcon(actionId)" :size="14"/>{{ actionLabel(actionId) }}</button><span v-if="!visibleActions(group.actionIds ?? []).length" class="widget-menu-empty">此菜单暂无动作</span></div></Teleport></template><button v-for="actionId in visibleActions(values.items ?? [])" :key="actionId" :disabled="widgetDisabled||!actionEnabled(actionId)" @click="runShared(actionId)"><component v-if="actionIcon(actionId)" :is="actionIcon(actionId)" :size="14"/>{{ actionLabel(actionId) }}</button></template><span v-else>{{ values.text }}</span></nav>
    <div v-else-if="widget.kind === 'toolbar'" class="widget-toolbar"><BaseButton v-for="actionId in visibleActions(values.items ?? [])" :key="actionId" appearance="subtle" :disabled="widgetDisabled||!actionEnabled(actionId)" @click="runShared(actionId)"><component v-if="actionIcon(actionId)" :is="actionIcon(actionId)" :size="14"/>{{ actionLabel(actionId) }}</BaseButton></div>
    <div v-else-if="widget.kind === 'statusbar'" class="widget-statusbar"><span>{{ values.text }}</span><span>{{ values.status ?? '' }}</span></div>
    <hr v-else-if="widget.kind === 'separator'" class="widget-separator" :class="`separator-${values.orientation ?? 'horizontal'}`">
    <span v-else-if="widget.kind === 'spacer'" class="widget-spacer" :style="{flex: values.flex ?? 1}" />
    <span v-else-if="!isContainer(widget)">未知控件：{{ widget.kind }}</span>
    <div v-if="widget.kind==='splitter' && $slots.default" class="widget-splitter-panes" :class="`splitter-${values.orientation ?? 'horizontal'}`"><div class="widget-splitter-pane" :style="{flex:`${splitterRatio}`}" ><slot slot-id="first" /></div><div class="widget-splitter-handle" :class="`splitter-handle-${values.orientation ?? 'horizontal'}`" role="separator" tabindex="0" :aria-orientation="values.orientation==='vertical'?'horizontal':'vertical'" @pointerdown="resizeSplitter" /><div class="widget-splitter-pane" :style="{flex:`${1-splitterRatio}`}" ><slot slot-id="second" /></div></div>
    <div v-else-if="isContainer(widget) && !['dialog','splitter'].includes(widget.kind) && $slots.default" class="widget-container-slot" :class="{'widget-scroll-slot':widget.kind==='scroll'}"><slot :slot-id="activeSlot" /></div>
  </div>
</template>

<script lang="ts">
import TreeItems from './WidgetTreeItems.vue'
export default { components: { TreeItems } }
</script>
