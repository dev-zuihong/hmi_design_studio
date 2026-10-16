import { ref, watch } from 'vue'
import { listen } from '@tauri-apps/api/event'
import { call, desktop } from './backend'
import type { Editor } from './editor'
import { clone, type Project } from './model'
import { applyOperations, diffDocument, invertOperations, type WorkspaceOperation } from './operations'

export interface WorkspaceState { workspaceId: string; project: Project; revision: number; savedRevision: number; dirty: boolean; path?: string; canUndo?: boolean; canRedo?: boolean; recovered?:boolean; missingDependencies?:string[]; histories?: Record<string, {canUndo?: boolean;canRedo?: boolean;undo?:number;redo?:number}> }
export const editorWindowId = new URLSearchParams(typeof location === 'undefined' ? '' : location.search).get('view') === 'logic' ? 'logic' : 'main'
export const workspaceRevision = ref(0), workspacePath = ref<string>(), workspaceError = ref(''), workspaceReady = ref(false), workspaceRecovered=ref(false)
export const workspaceMissingDependencies=ref<string[]>([])
type Transaction = { forward: WorkspaceOperation[]; inverse: WorkspaceOperation[] }
const fingerprint=(value:unknown)=>JSON.stringify(value,(_key,entry)=>entry&&typeof entry==='object'&&!Array.isArray(entry)?Object.fromEntries(Object.entries(entry).sort(([a],[b])=>a.localeCompare(b))):entry)
function protectReferences(project:Project,operations:WorkspaceOperation[]){
  const removed=new Set(operations.filter(operation=>!operation.afterExists&&operation.beforeExists&&operation.before&&typeof operation.before==='object'&&'id' in operation.before).map(operation=>(operation.before as {id:string}).id))
  if(!removed.size)return
  function visit(value:any,path:string[]=[]){if(!value||typeof value!=='object')return;for(const [key,entry]of Object.entries(value)){if(typeof entry==='string'&&removed.has(entry)&&['parentId','flowId','entryNodeId','connectionId','variableId','pageId','key','source','target','actionId'].includes(key))throw Error(`撤销冲突：${path.concat(key).join('.')} 已引用另一窗口使用的对象`);visit(entry,path.concat(key))}}
  visit(project)
}

/** Browser uses the same command semantics. Desktop authority lives in Rust. */
export class MemoryWorkspace {
  readonly workspaceId=crypto.randomUUID()
  project: Project; revision = 0; savedRevision = 0; saved: string; histories = new Map<string, {past: Transaction[];future: Transaction[]}>()
  constructor(project: Project) { this.project = clone(project); this.saved = fingerprint(project) }
  history(windowId: string) { let history = this.histories.get(windowId); if (!history) { history = {past:[],future:[]}; this.histories.set(windowId,history) }; return history }
  state(): WorkspaceState { return { workspaceId: this.workspaceId, project: clone(this.project), revision:this.revision, savedRevision:this.savedRevision, dirty:fingerprint(this.project)!==this.saved, histories:Object.fromEntries([...this.histories].map(([id,h])=>[id,{canUndo:!!h.past.length,canRedo:!!h.future.length}])) } }
  apply(windowId: string, operations: WorkspaceOperation[]) { this.project=applyOperations(this.project,operations); this.revision++; const h=this.history(windowId); h.past.push({forward:operations,inverse:invertOperations(operations)}); if(h.past.length>100)h.past.shift();h.future=[];return this.state() }
  undo(windowId: string) { const h=this.history(windowId), entry=h.past.at(-1); if(!entry)return this.state();const next=applyOperations(this.project,entry.inverse);protectReferences(next,entry.inverse);this.project=next;h.past.pop();h.future.push(entry);this.revision++;return this.state() }
  redo(windowId: string) { const h=this.history(windowId), entry=h.future.at(-1);if(!entry)return this.state();const next=applyOperations(this.project,entry.forward);protectReferences(next,entry.forward);this.project=next;h.future.pop();h.past.push(entry);this.revision++;return this.state() }
  markSaved(revision = this.revision, project = this.project) {this.saved=fingerprint(project);this.savedRevision=revision;return this.state()}
}

export function connectWorkspace(editor: Editor, onError: (error: unknown) => void) {
  let authority = new MemoryWorkspace(editor.project.value), backend = false, disposed=false, applying=false, accepted=clone(editor.project.value), projected=clone(editor.project.value)
  let activeWorkspaceId='', submitting=false, queued=false, changeTimer:ReturnType<typeof setTimeout>|undefined
  let gestureBefore:Project|undefined,gestureIds=new Set<string>()
  let off: (()=>void)|undefined
  const queue: {id:string;operations:WorkspaceOperation[]}[]=[]
  const channel=typeof BroadcastChannel==='undefined'?undefined:new BroadcastChannel('hmi-studio-workspace-v2')
  const requests=new Map<string,{resolve:(state:WorkspaceState)=>void;reject:(error:unknown)=>void}>()
  function install(state:WorkspaceState, ackId?:string) {
    if(disposed || state.revision < workspaceRevision.value && state.workspaceId===activeWorkspaceId)return
    const changingWorkspace=!!activeWorkspaceId&&state.workspaceId!==activeWorkspaceId
    if(changingWorkspace){queue.splice(0)}
    if(ackId){const index=queue.findIndex(item=>item.id===ackId);if(index>=0)queue.splice(index,1)}
    const draft=changingWorkspace?[]:diffDocument(gestureBefore??projected,editor.project.value).filter(operation=>!gestureBefore||isGestureOperation(operation))
    if(draft.length&&!editor.gesturing.value)queue.push({id:crypto.randomUUID(),operations:draft})
    activeWorkspaceId=state.workspaceId;accepted=clone(state.project);let optimistic=clone(accepted)
    for(const transaction of queue)try{optimistic=applyOperations(optimistic,transaction.operations,false)}catch{}
    if(editor.gesturing.value)try{optimistic=applyOperations(optimistic,draft,false)}catch{}
    applying=true;editor.hydrate(optimistic);projected=clone(optimistic);applying=false
    workspaceRevision.value=state.revision;workspacePath.value=state.path;workspaceRecovered.value=state.recovered??false
    if(state.missingDependencies!==undefined||changingWorkspace)workspaceMissingDependencies.value=state.missingDependencies??[]
    editor.remoteDirty.value=state.dirty||queue.length>0||draft.length>0
    const history=state.histories?.[editorWindowId]
    editor.remoteCanUndo.value=history?.canUndo ?? (!!history?.undo || !!state.canUndo)
    editor.remoteCanRedo.value=history?.canRedo ?? (!!history?.redo || !!state.canRedo)
    if(draft.length&&!editor.gesturing.value)void drain()
  }
  function broadcast(state:WorkspaceState, ackId?:string){channel?.postMessage({type:'state',state,ackId})}
  async function command(method:string,args:Record<string,unknown>={}):Promise<WorkspaceState>{
    if(backend)return call<WorkspaceState>(method,{windowId:editorWindowId,workspaceId:activeWorkspaceId||undefined,...args})
    if(editorWindowId==='logic'&&channel)return new Promise((resolve,reject)=>{const id=crypto.randomUUID();requests.set(id,{resolve,reject});channel.postMessage({type:'request',id,method,args:{windowId:editorWindowId,...args}});setTimeout(()=>{if(requests.delete(id))reject(new Error('界面设计窗口未响应，请重新打开逻辑窗口。'))},10000)})
    let state:WorkspaceState
    if(method==='apply_workspace_operations')state=authority.apply(String(args.windowId??editorWindowId),args.operations as WorkspaceOperation[])
    else if(method==='undo_workspace')state=authority.undo(String(args.windowId??editorWindowId))
    else if(method==='redo_workspace')state=authority.redo(String(args.windowId??editorWindowId))
    else if(method==='replace_workspace'){authority=new MemoryWorkspace(args.project as Project);state=authority.state()}
    else if(method==='workspace_mark_saved')state=authority.markSaved(args.revision as number,args.project as Project)
    else state=authority.state()
    if(editorWindowId==='main')try{localStorage.setItem('hmi.workspace.recovery',JSON.stringify({project:authority.project,revision:authority.revision,savedRevision:authority.savedRevision,saved:authority.saved,histories:[...authority.histories]}))}catch{}
    broadcast(state, args.opId as string|undefined);return state
  }
  if(channel)channel.onmessage=async event=>{
    const message=event.data
    if(message.type==='request'&&editorWindowId==='main'&&!backend){try{const state=await command(message.method,message.args);install(state,message.args?.opId);channel.postMessage({type:'response',id:message.id,state})}catch(error){channel.postMessage({type:'response',id:message.id,error:String(error)})}}
    else if(message.type==='response'){const pending=requests.get(message.id);if(pending){requests.delete(message.id);message.error?pending.reject(message.error):pending.resolve(message.state)}}
    else if(message.type==='state'&&!backend){install(message.state,message.ackId)}
  }
  function capture(force=false) {
    if(applying||disposed||!workspaceReady.value||editor.gesturing.value&&!force)return
    const operations=diffDocument(projected,editor.project.value);if(!operations.length)return
    queue.push({id:crypto.randomUUID(),operations});projected=clone(editor.project.value);editor.remoteDirty.value=true;void drain()
  }
  async function drain(){
    if(submitting){queued=true;return}submitting=true
    try{do{queued=false;while(queue.length&&!disposed){const item=queue[0]!,workspaceId=activeWorkspaceId;try{const state=await command('apply_workspace_operations',{baseRevision:workspaceRevision.value,opId:item.id,operations:item.operations});if(activeWorkspaceId===workspaceId)install(state,item.id)}catch(error){if(activeWorkspaceId!==workspaceId)continue;const index=queue.findIndex(entry=>entry.id===item.id);if(index>=0)queue.splice(index,1);workspaceError.value=String(error);onError(error);try{install(await command('workspace_state'))}catch{} }}}while(queued&&!disposed)}finally{submitting=false}
  }
  const stopWatch=watch(editor.project,()=>{
    if(applying||disposed)return
    if(gestureBefore){const operations=diffDocument(projected,editor.project.value).filter(operation=>!isGestureOperation(operation));if(operations.length){queue.push({id:crypto.randomUUID(),operations});projected=applyOperations(projected,operations,false);void drain()}return}
    if(changeTimer)clearTimeout(changeTimer);changeTimer=setTimeout(capture,30)
  },{deep:true,flush:'sync'})
  function isGestureOperation(operation:WorkspaceOperation){return operation.path.length>=5&&operation.path[0]==='pages'&&operation.path[2]==='widgets'&&gestureIds.has(operation.path[3]!)&&['x','y','width','height','parentId','slotId','layoutItem'].includes(operation.path[4]!)}
  const stopGesture=watch(editor.gesturing,active=>{
    if(active){capture(true);gestureBefore=clone(editor.project.value);gestureIds=new Set(editor.selected.value)}
    else if(gestureBefore){const operations=diffDocument(gestureBefore,editor.project.value).filter(isGestureOperation);gestureBefore=undefined;gestureIds.clear();if(operations.length){queue.push({id:crypto.randomUUID(),operations});projected=clone(editor.project.value);editor.remoteDirty.value=true;void drain()}else capture()}
    else capture()
  },{flush:'sync'})
  async function flush(){if(changeTimer)clearTimeout(changeTimer);capture();while(submitting||queue.length){await new Promise(resolve=>setTimeout(resolve,10));if(disposed)break}}
  async function undo(){await flush();try{install(await command('undo_workspace'))}catch(error){onError(error)}}
  async function redo(){await flush();try{install(await command('redo_workspace'))}catch(error){onError(error)}}
  editor.connectHistory(undo,redo)
  async function init(){
    if(desktop){try{off=await listen<WorkspaceState>('workspace-changed',event=>install(event.payload));const state=await call<WorkspaceState|null>('workspace_state',{windowId:editorWindowId});if(state?.project){backend=true;install(state)}}catch(error){onError(error)}}
    if(!backend){if(editorWindowId==='logic')try{install(await command('workspace_state'))}catch(error){onError(error)}else{try{const raw=localStorage.getItem('hmi.workspace.recovery');if(raw){const recovery=JSON.parse(raw);authority=new MemoryWorkspace(recovery.project);authority.saved=recovery.saved;authority.revision=recovery.revision;authority.savedRevision=recovery.savedRevision;authority.histories=new Map(recovery.histories??[]);install({...authority.state(),recovered:authority.state().dirty})}else install(authority.state())}catch{install(authority.state())}}}
    workspaceReady.value=true
  }
  async function replace(project:Project,path?:string){await flush();queue.splice(0);const state=await command('replace_workspace',{project,path});projected=clone(state.project);accepted=clone(state.project);install(state)}
  async function receive(state:WorkspaceState){await flush();queue.splice(0);projected=clone(state.project);install(state)}
  async function markSaved(revision:number,project:Project){install(await command('workspace_mark_saved',{revision,project}))}
  const onBlur=()=>{capture()}
  window.addEventListener('blur',onBlur)
  function dispose(){window.removeEventListener('blur',onBlur);disposed=true;stopWatch();stopGesture();off?.();channel?.close();if(changeTimer)clearTimeout(changeTimer)}
  return {init,flush,replace,receive,markSaved,dispose,get backend(){return backend},get snapshot(){return {project:clone(accepted),revision:workspaceRevision.value}}}
}
