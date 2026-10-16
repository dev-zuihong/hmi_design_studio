import { createApp, h, reactive } from 'vue'
import '../../src/lib/fluent'
import '../../src/style.css'
import WidgetTree from '../../src/components/WidgetTree.vue'
import JsonValueEditor from '../../src/components/JsonValueEditor.vue'
import StructuredPropertyEditor from '../../src/components/StructuredPropertyEditor.vue'
import { createWidget, emptyProject, type Snapshot, type UiAction } from '../../src/lib/model'
import { provideRuntimeFileService } from '../../src/lib/runtime-files'
import { i18n } from '../../src/lib/i18n'

const project=reactive(emptyProject()),page=project.pages[0]!,events:any[]=[],snapshot=reactive<Snapshot>({variables:{},points:{},connections:{},flows:{},logs:[],inputs:{},backends:{host:{devices:{'usb.device.1':{label:'Bound device'}}}},context:{deviceKey:'usb.device.1'}})
project.width=1400;project.height=1200
function add(kind:string,x:number,y:number,props:Record<string,unknown>={},width?:number,height?:number){const widget=createWidget(kind,x,y);Object.assign(widget.props,props);if(width)widget.width=width;if(height)widget.height=height;page.widgets.push(widget);return page.widgets.at(-1)!}
const table=add('table',16,16,{text:'Device table',rowKey:'deviceKey',columns:[{key:'name',title:'Name',type:'text'},{key:'state',title:'State',type:'badge',badges:{ready:{label:'Ready',color:'#107c10'}}},{key:'active',title:'Active',type:'boolean',editable:true,enabledField:'canStart'},{key:'firmware',title:'Firmware',type:'file',editable:true,accept:'.bin',enabledField:'canStart'},{key:'actions',title:'Actions',type:'actions',actions:[{id:'start',label:'Start',enabledField:'canStart'},{id:'settings',label:'Settings',visibleField:'showSettings'}]}],rows:[{deviceKey:'usb.b',name:'B',state:'ready',active:false,canStart:false,showSettings:false},{deviceKey:'usb.a',name:'A',state:'ready',active:false,canStart:true,showSettings:true}]},950,200)
for(const event of ['cellEdit','rowAction','fileChange','selectionChange'])table.events[event]={kind:'flow',flowId:event}
const file=add('file-picker',16,230,{allowManualPath:true},460,85);file.events.change={kind:'flow',flowId:'file'}
const range=add('byte-range',16,330,{byteLength:32,value:{startByte:31,byteCount:1}},600,140);range.events.change={kind:'flow',flowId:'range'}
add('memory-map',650,330,{rangeStart:0,rangeSize:1024,segments:[{id:'a',label:'A segment',start:0,size:512},{id:'b',label:'B segment',start:256,size:1024}]},500,160)
const validated=add('input',16,500,{text:'Hex input',placeholder:'Hex input',format:'hex',commitMode:'enter'},300,80);validated.events.change={kind:'flow',flowId:'validated'}
add('log',650,510,{text:'Structured log',source:'entries',maxLines:3,autoScroll:true,entries:Array.from({length:5},(_,index)=>({id:index,timestamp:0,deviceKey:'usb.a',level:index===4?'error':'info',message:`Entry ${index}`}))},500,160)
const bound=add('text',16,600,{text:'Unbound'},300,40);bound.bindings.text={source:'backend',key:'host',path:'devices.$deviceKey.label'}
const tabs=add('tabs',1000,16,{tabPlacement:'left',tabWidth:100},350,250);tabs.layout={type:'vertical',gap:4,padding:8}
const first=add('text',0,0,{text:'First tab'});first.parentId=tabs.id;first.slotId='tab_1'
const second=add('text',0,0,{text:'Second tab'});second.parentId=tabs.id;second.slotId='tab_2'
add('number',16,900,{text:'Compact metric',value:3,fontSize:12,padding:8},156,68)
add('button',200,900,{text:'导出',icon:'Save',padding:8},72,34)
const numericSelect=add('select',300,900,{value:2,options:[{value:1,label:'Option A'},{value:2,label:'Option B'}]},240,34);numericSelect.events.change={kind:'flow',flowId:'numericSelect'}
const scroll=add('scroll',600,900,{},300,70)
const scrollText=add('text',0,0,{text:'Scrollable content'});scrollText.parentId=scroll.id
table.props.columns.push({key:'address',title:'Address',type:'text',format:'address',editable:true})
for(const row of table.props.rows)row.address='0x02002000'
const record={id:'file-1',name:'firmware.bin',size:32,sha256:'a'.repeat(64)}
;(window as any).__deviceEvents=events
;(window as any).__deviceProject=project
;(window as any).__deviceSnapshot=snapshot
;(window as any).__deviceFileMode='success'
const app=createApp({setup(){provideRuntimeFileService({async choose(request){events.push({kind:'choose',request});if((window as any).__deviceFileMode==='cancel')return undefined;if((window as any).__deviceFileMode==='deferred')return new Promise<any>(resolve=>(window as any).__deviceResolveFile=()=>resolve(record));if((window as any).__deviceFileMode==='error')throw new Error('Import failed');return {...record,...(request.path?{path:request.path}:{})}}});return()=>h('div',[
  h(WidgetTree,{project,pageId:page.id,snapshot,events:[],onAction:(action:UiAction,value:unknown)=>events.push({kind:'action',action,value}),onInput:(id:string,value:unknown)=>{snapshot.inputs![id]=value;events.push({kind:'input',id,value})}}),
  h('section',{style:'position:absolute;top:730px;left:20px;width:300px',id:'json-editor'},[h(JsonValueEditor,{modelValue:{enabled:true},type:'object','onUpdate:modelValue':(value:unknown)=>events.push({kind:'json',value})})]),
  h('section',{style:'position:absolute;top:730px;left:400px;width:300px',id:'columns-editor'},[h(StructuredPropertyEditor,{modelValue:table.props.columns,kind:'columns','onUpdate:modelValue':(value:any)=>{table.props.columns=value}})]),
])}})
app.use(i18n).mount('#app')
