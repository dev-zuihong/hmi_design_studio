import { createApp, h, reactive } from 'vue'
import WidgetTree from '../../src/components/WidgetTree.vue'
import { createWidget, emptyProject, type RuntimeEvent, type UiAction } from '../../src/lib/model'
import { i18n, projectLocale } from '../../src/lib/i18n'

const project=reactive(emptyProject()),page=project.pages[0]!
function add(kind:string,x:number,y:number,props:Record<string,unknown>={}){const widget=createWidget(kind,x,y);Object.assign(widget.props,props);page.widgets.push(widget);return widget}
const checkbox=add('checkbox',16,16,{text:'启用采集',value:false});checkbox.events.change={kind:'flow',flowId:'checkbox-flow'}
const radio=add('radio',16,65,{text:'模式',options:['自动','手动'],value:'自动'})
const select=add('select',16,170,{text:'设备',options:[{value:'a',label:{i18nKey:'device.a',fallback:'设备 A'}},{value:'b',label:'设备 B'}],value:'a'})
const tabs=add('tabs',260,16);tabs.width=360;tabs.height=210;tabs.slots=[{id:'tab_1',title:{i18nKey:'tab.one',fallback:'概览'} as any},{id:'tab_2',title:'详情'}];tabs.props.activeSlot='tab_1';tabs.layout={type:'vertical',gap:8,padding:8}
const restoredTabs=add('tabs',1000,120);restoredTabs.width=220;restoredTabs.height=150;restoredTabs.slots=[{id:'restored_first',title:'默认选项卡'},{id:'restored_second',title:'恢复选项卡'}];restoredTabs.props.activeSlot='restored_first'
const tab1=add('text',8,8,{text:{i18nKey:'title.one',fallback:'第一页标题'}});tab1.parentId=tabs.id;tab1.slotId='tab_1'
const nested=add('button',8,54,{text:'子按钮'});nested.parentId=tabs.id;nested.slotId='tab_1';nested.events.click={kind:'flow',flowId:'nested-flow'}
const tab2=add('text',8,8,{text:'第二页内容'});tab2.parentId=tabs.id;tab2.slotId='tab_2'
const table=add('table',16,250,{text:'可编辑表格',columns:[{key:'name',title:{i18nKey:'column.name',fallback:'名称'},type:'text',editable:true},{key:'value',title:'数值',type:'number',editable:true}],rows:[{id:'r1',name:'温度',value:22}],sortable:true,selectable:true});table.width=390;table.height=180;table.events.cellEdit={kind:'flow',flowId:'cell-flow'};table.events.selectionChange={kind:'flow',flowId:'row-flow'}
const splitter=add('splitter',680,16,{text:'可拖动分隔面板',orientation:'horizontal',ratio:.5});splitter.width=340;splitter.height=180;splitter.slots=[{id:'first',title:'左'},{id:'second',title:'右'}];splitter.layout={type:'absolute',padding:4,gap:4};splitter.events.resize={kind:'flow',flowId:'resize-flow'}
const a=add('text',4,4,{text:'左侧'});a.parentId=splitter.id;a.slotId='first'
const b=add('text',4,4,{text:'右侧'});b.parentId=splitter.id;b.slotId='second'
const dialog=add('dialog',680,230,{text:'非模态对话框',modal:false,open:true});dialog.width=300;dialog.height=180
const d=add('text',8,8,{text:'对话框内容'});d.parentId=dialog.id;d.slotId='content'
add('menu',16,470,{groups:[{id:'tools-menu',title:{i18nKey:'menu.tools',fallback:'工具'},actionIds:['shared-test','hidden-test']}],items:[]})
add('toolbar',260,470,{items:['shared-test','hidden-test']})
const actionWidget=add('button',680,440,{text:'共享动作'});project.actions=[{id:'shared-test',text:{i18nKey:'shared.test',fallback:'共享动作'},icon:'Play',shortcut:'Ctrl+Alt+J',enabled:true,visible:true,action:{kind:'flow',flowId:'shortcut-flow'}},{id:'hidden-test',text:'隐藏动作',visible:false,action:{kind:'flow',flowId:'hidden-flow'}}];actionWidget.events.click={kind:'shared',flowId:'',actionId:'shared-test'}
const disabledList=add('list',680,470,{items:['禁用项目'],enabled:false});disabledList.events.selectionChange={kind:'flow',flowId:'disabled-list-flow'}
const restoredInput=add('input',900,560,{text:'恢复输入',value:'默认值'});restoredInput.props.placeholder='恢复输入'
const boundInput=add('input',900,610,{text:'绑定输入',value:'默认值'});boundInput.props.placeholder='绑定输入';boundInput.bindings.value={source:'variable',key:'bound-value'}
project.variables.push({id:'bound-value',name:'绑定值',type:'string',initial:'绑定值'} as any)
project.i18n={defaultLocale:'zh-CN',locales:['zh-CN','en-US'],messages:{'zh-CN':{},'en-US':{'device.a':'Device A','tab.one':'Overview','title.one':'First page title','column.name':'Name','shared.test':'Shared action','menu.tools':'Tools'}}}
const events:any[]=[]
const snapshot=reactive({variables:{'bound-value':'绑定值'},points:{},connections:{},flows:{},logs:[],ui:{},inputs:{[restoredInput.id]:'会话输入',[boundInput.id]:'过期输入',[table.id]:[{id:'r1',name:'温度',value:23}],[restoredTabs.id]:'restored_second'}})
;(window as any).__widgetEvents=events
;(window as any).__widgetSetLocale=(locale:string)=>projectLocale.value=locale
const app=createApp({render:()=>h(WidgetTree,{project,pageId:page.id,snapshot,events:[] as RuntimeEvent[],onAction:(action:UiAction,value:unknown)=>events.push({kind:'action',action,value}),onInput:(id:string,value:unknown)=>events.push({kind:'input',id,value}),onSelection:(id:string,value:unknown)=>events.push({kind:'selection',id,value})})})
app.use(i18n).mount('#app')
