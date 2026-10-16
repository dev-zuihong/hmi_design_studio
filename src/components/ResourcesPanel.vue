<script setup lang="ts">
import JsonValueEditor from './JsonValueEditor.vue'
import BackendResourcesPanel from './BackendResourcesPanel.vue'
import TransportResourcesPanel from './TransportResourcesPanel.vue'
import { computed, onMounted, ref } from 'vue'
import { Cable, Plus, RefreshCw, Trash2, Variable, Upload, Circle, Terminal, X, AlertCircle, Search } from 'lucide-vue-next'
import BaseButton from './base/BaseButton.vue'
import { call, desktop, type Runtime } from '../lib/backend'
import { newId, type ConnectionConfig, type PluginManifest, type ValidationIssue } from '../lib/model'
import type { Editor } from '../lib/editor'
import { pluginCatalog } from '../lib/resources'
import SchemaField from './SchemaField.vue'
import { schemaDefaults } from '../lib/schema'
type ResourceTab = 'connections' | 'variables' | 'events' | 'issues'
const props = defineProps<{ editor: Editor; runtime: Runtime; issues: ValidationIssue[]; expanded: boolean; activeTab: ResourceTab }>()
const emit = defineEmits<{ error: [message: string]; 'update:expanded': [value: boolean]; 'update:activeTab': [value: ResourceTab]; importPlugin: [] }>()
const selectedConnection = ref(''), ports = ref<{ name: string; description: string }[]>([]), plugins = pluginCatalog, runFilter = ref(''), variableFilter = ref('')
const connectionKind = ref<'serial'|'backends'|'windows'|'transports'>('serial')
const tab = computed(() => props.activeTab)
const variables = computed(() => {
  const query = variableFilter.value.trim().toLocaleLowerCase()
  return props.editor.project.value.variables.filter(variable => !query || variable.name.toLocaleLowerCase().includes(query) || variable.id.toLocaleLowerCase().includes(query))
})
const connection = computed(() => props.editor.project.value.connections.find(c => c.id === selectedConnection.value) ?? props.editor.project.value.connections[0])
const eventList = computed(() => props.runtime.events.value.filter(event => !runFilter.value || event.runId?.includes(runFilter.value)).slice(-300).reverse())
const protocol = computed(() => plugins.value.find(plugin => plugin.id === connection.value?.protocolId))
const configFields = computed(() => protocol.value?.configSchema?.properties ?? {})
const disabled = computed(() => props.editor.readOnly.value)
async function refresh() { if (!desktop) return; try { [ports.value, plugins.value] = await Promise.all([call<{ name: string; description: string }[]>('list_serial_ports'), call<PluginManifest[]>('list_plugins')]) } catch (error) { emit('error', String(error)) } }
onMounted(refresh)
function addConnection() { const item: ConnectionConfig = { id: newId('connection'), name: '新串口连接', port: '', baudRate: 115200, dataBits: 8, stopBits: 1, parity: 'none', protocolId: plugins.value[0]?.id ?? '', protocolVersion: plugins.value[0]?.version ?? '1.0.0', protocolConfig: schemaDefaults(plugins.value[0]?.configSchema) }; if (props.editor.mutate(p => p.connections.push(item))) selectedConnection.value = item.id }
function editConnection(key: keyof ConnectionConfig, value: unknown) { if (connection.value) props.editor.mutate(() => { (connection.value as any)[key] = value; if (key === 'protocolId') { const plugin = plugins.value.find(p => p.id === value); connection.value!.protocolVersion = plugin?.version; connection.value!.protocolConfig = schemaDefaults(plugin?.configSchema) } }) }
function addVariable() { if (props.editor.mutate(p => p.variables.push({ id: newId('variable'), name: `变量 ${p.variables.length + 1}`, type: 'number', initial: 0 }))) variableFilter.value = '' }
function variableValue(id: string, event: Event) { const variable = props.editor.project.value.variables.find(v => v.id === id)!; const value = (event.target as HTMLInputElement).value; props.editor.mutate(() => variable.initial = variable.type === 'number' ? Number(value) : variable.type === 'boolean' ? value === 'true' : value) }
function status(id: string) { const state = props.runtime.snapshot.value.connections[id]; if (!state) return '未连接'; const value = typeof state === 'string' ? state : state.status ?? state.state; return value === 'open' || value === 'connected' ? '已连接' : value === 'fault' || value === 'faulted' || value === 'error' ? '连接故障' : value === 'opening' ? '连接中' : value === 'closed' ? '已关闭' : String(value ?? '未连接') }
function selectTab(value: ResourceTab) { emit('update:activeTab', value); emit('update:expanded', !(props.expanded && props.activeTab === value)) }
function showIssues() { emit('update:activeTab', 'issues'); emit('update:expanded', true) }
function showConnections() { emit('update:activeTab', 'connections'); emit('update:expanded', true) }
defineExpose({ refresh, showIssues, showConnections })
</script>

<template>
  <section class="resources-panel" :class="{ collapsed: !expanded }"><div class="resource-tabs"><button :class="{ active: expanded && tab === 'connections' }" :aria-expanded="expanded && tab === 'connections'" @click="selectTab('connections')"><Cable :size="15" />设备连接<span>{{ editor.project.value.connections.length }}</span></button><button :class="{ active: expanded && tab === 'variables' }" :aria-expanded="expanded && tab === 'variables'" @click="selectTab('variables')"><Variable :size="15" />项目变量<span>{{ editor.project.value.variables.length }}</span></button><button :class="{ active: expanded && tab === 'events' }" :aria-expanded="expanded && tab === 'events'" @click="selectTab('events')"><Terminal :size="15" />执行记录<span>{{ runtime.events.value.length }}</span></button><button :class="{ active: expanded && tab === 'issues' }" :aria-expanded="expanded && tab === 'issues'" @click="selectTab('issues')"><AlertCircle :size="15" />检查<span :class="{ 'error-count': issues.some(i => i.severity === 'error') }">{{ issues.length }}</span></button><span class="spacer" /><slot name="status" /><button class="icon-button resource-toggle" :aria-expanded="expanded" :title="expanded ? '收起面板' : '展开面板'" @click="emit('update:expanded', !expanded)"><X v-if="expanded" :size="14" /><Plus v-else :size="14" /></button></div>
    <div v-if="expanded && tab==='connections'" class="connection-kind-tabs"><button type="button" :class="{active:connectionKind==='serial'}" @click="connectionKind='serial'">串口</button><button type="button" :class="{active:connectionKind==='transports'}" @click="connectionKind='transports'">USB 传输 <span>{{editor.project.value.transports?.length??0}}</span></button><button type="button" :class="{active:connectionKind==='backends'}" @click="connectionKind='backends'">后端实例 <span>{{editor.project.value.backends?.length??0}}</span></button><button type="button" :class="{active:connectionKind==='windows'}" @click="connectionKind='windows'">窗口模板 <span>{{editor.project.value.windows?.length??0}}</span></button></div>
    <div v-if="expanded && tab === 'connections' && connectionKind==='serial'" class="connections-content"><div class="connection-list"><div class="inline-between"><span class="small muted">串口资源</span><button class="icon-button" title="添加串口连接" :disabled="disabled" @click="addConnection"><Plus :size="15" /></button></div><button v-for="item in editor.project.value.connections" :key="item.id" class="connection-item" :class="{ active: connection?.id === item.id }" @click="selectedConnection = item.id"><Cable :size="17" /><span><strong>{{ item.name }}</strong><small>{{ item.port || '尚未配置端口' }}</small></span><i :class="{ connected: status(item.id) === '已连接' }" /></button><div v-if="!editor.project.value.connections.length" class="small muted">添加串口资源后，在流程中配置打开连接。</div></div><div v-if="connection" :key="connection.id" class="connection-settings"><div class="inline-between connection-heading"><strong>{{ connection.name }}</strong><span class="status-pill">{{ status(connection.id) }}</span><span class="spacer" /><button class="text-button" :disabled="disabled" @click="emit('importPlugin')"><Upload :size="13" />导入协议</button><button class="icon-button" title="刷新端口与插件" @click="refresh"><RefreshCw :size="14" /></button><button class="icon-button" title="删除连接资源" :disabled="disabled" @click="editor.mutate(p => p.connections = p.connections.filter(c => c.id !== connection!.id))"><Trash2 :size="14" /></button></div><fieldset :disabled="disabled" class="connection-grid connection-basic" aria-label="基本设置"><label>资源名称<input :value="connection.name" @change="editConnection('name', ($event.target as HTMLInputElement).value)" /></label><label>串口<select :value="connection.port" @change="editConnection('port', ($event.target as HTMLSelectElement).value)"><option value="">选择串口</option><option v-if="connection.port  && !ports.some(p => p.name === connection!.port)" :value="connection.port">{{ connection.port }}（当前不可用）</option><option v-for="port in ports" :key="port.name" :value="port.name">{{ port.name }} {{ port.description }}</option></select></label><label>波特率<select :value="connection.baudRate" @change="editConnection('baudRate', Number(($event.target as HTMLSelectElement).value))"><option v-for="rate in [1200,2400,4800,9600,19200,38400,57600,115200,230400,460800,921600]" :key="rate" :value="rate">{{ rate }}</option></select></label><label>协议插件<select :value="connection.protocolId" @change="editConnection('protocolId', ($event.target as HTMLSelectElement).value)"><option v-if="!plugins.some(p => p.id === connection!.protocolId)" :value="connection.protocolId">{{ connection.protocolId }}</option><option v-for="plugin in plugins" :key="plugin.id" :value="plugin.id">{{ plugin.name }} · {{ plugin.version }}</option></select></label></fieldset>
        <details class="connection-section advanced-serial"><summary>高级串口</summary><fieldset :disabled="disabled" class="connection-grid connection-advanced"><label>数据位<select :value="connection.dataBits" @change="editConnection('dataBits', Number(($event.target as HTMLSelectElement).value))"><option v-for="bits in [5,6,7,8]" :key="bits">{{ bits }}</option></select></label><label>停止位<select :value="connection.stopBits" @change="editConnection('stopBits', Number(($event.target as HTMLSelectElement).value))"><option>1</option><option>2</option></select></label><label>校验<select :value="connection.parity" @change="editConnection('parity', ($event.target as HTMLSelectElement).value)"><option value="none">无</option><option value="odd">奇校验</option><option value="even">偶校验</option></select></label></fieldset></details>
        <details v-if="Object.keys(configFields).length" class="connection-section protocol-parameters"><summary>协议参数</summary><fieldset :disabled="disabled" class="connection-grid protocol-grid"><SchemaField v-for="(schema, key) in configFields" :key="`${connection.id}:${key}`" :schema="schema as any" :label="(schema as any).title ?? String(key)" :model-value="connection.protocolConfig[key]" @update:model-value="value => editor.mutate(() => connection!.protocolConfig[key] = value)" /></fieldset></details>
        <div class="connection-foot"><span class="muted small">串口在预览运行时由连接节点打开。</span><span class="spacer" /><BaseButton v-if="runtime.running.value" @click="call(status(connection.id) === '已连接' ? 'close_connection' : 'open_connection', { connectionId: connection.id, expectedGeneration: runtime.generation.value }).then(runtime.refresh).catch(error => emit('error', String(error)))">{{ status(connection.id) === '已连接' ? '关闭连接' : '打开连接' }}</BaseButton></div></div><div v-else class="resources-empty"><Cable :size="28" /><span>添加串口连接，配置设备参数</span></div></div>
    <TransportResourcesPanel v-else-if="expanded && tab==='connections' && connectionKind==='transports'" :editor="editor"/><BackendResourcesPanel v-else-if="expanded && tab==='connections' && (connectionKind==='backends'||connectionKind==='windows')" :editor="editor" :mode="connectionKind" @error="emit('error',$event)"/>
    <div v-else-if="expanded && tab === 'variables'" class="variables-content"><div class="inline-between variables-toolbar"><label class="variable-search"><Search :size="14" /><input v-model="variableFilter" type="search" placeholder="按名称或 ID 搜索变量" aria-label="按名称或 ID 搜索变量" /></label><span class="muted small variable-hint">变量用于在流程与界面间共享数据，重启后恢复初始值。</span><BaseButton :disabled="disabled" @click="addVariable"><Plus :size="14" />添加变量</BaseButton></div><table><thead><tr><th>名称</th><th>标识</th><th>类型</th><th>初始值</th><th>运行值</th><th /></tr></thead><tbody><tr v-for="variable in variables" :key="variable.id"><td><input :value="variable.name" :disabled="disabled" @change="editor.mutate(() => variable.name = ($event.target as HTMLInputElement).value)" /></td><td><code>{{ variable.id }}</code></td><td><select :value="variable.type" :disabled="disabled" @change="editor.mutate(() => { variable.type = ($event.target as HTMLSelectElement).value as any; variable.initial = variable.type === 'number' ? 0 : variable.type === 'boolean' ? false : variable.type === 'enum' ? '选项一' : variable.type === 'array' ? [] : variable.type === 'object' ? {} : ''; if (variable.type === 'enum') variable.options = ['选项一', '选项二'] })"><option value="number">数字</option><option value="string">文本</option><option value="boolean">布尔</option><option value="enum">枚举</option><option value="array">数组</option><option value="object">对象</option></select></td><td><select v-if="variable.type === 'boolean'" :value="String(variable.initial)" :disabled="disabled" @change="variableValue(variable.id, $event)"><option value="false">false</option><option value="true">true</option></select><template v-else-if="variable.type === 'enum'"><select :value="variable.initial" :disabled="disabled" @change="variableValue(variable.id, $event)"><option v-for="option in variable.options ?? []" :key="option" :value="option">{{ option }}</option></select><input :value="variable.options?.join(', ')" placeholder="枚举选项，用逗号分隔" :disabled="disabled" @change="editor.mutate(() => { variable.options = ($event.target as HTMLInputElement).value.split(/[,，]/).map(value => value.trim()).filter(Boolean); if (!variable.options.includes(String(variable.initial))) variable.initial = variable.options[0] ?? '' })" /></template><JsonValueEditor v-else-if="variable.type==='array'||variable.type==='object'" :model-value="variable.initial" :type="variable.type" :disabled="disabled" :label="`${variable.name} JSON`" @update:model-value="editor.mutate(()=>variable.initial=$event)"/><input v-else :value="variable.initial" :type="variable.type === 'number' ? 'number' : 'text'" :disabled="disabled" @change="variableValue(variable.id, $event)" /></td><td><code>{{ JSON.stringify(runtime.snapshot.value.variables[variable.id] ?? '—') }}</code></td><td><button class="icon-button" title="删除变量" :disabled="disabled" @click="editor.mutate(p => p.variables = p.variables.filter(v => v.id !== variable.id))"><Trash2 :size="14" /></button></td></tr><tr v-if="!variables.length && variableFilter.trim()"><td colspan="6" class="variable-no-results">未找到匹配的变量</td></tr></tbody></table></div>
    <div v-else-if="expanded && tab === 'events'" class="events-content"><div class="inline-between"><input v-model="runFilter" class="run-filter" placeholder="按运行 ID 筛选…" aria-label="筛选运行记录" /><span class="muted small">{{ runtime.running.value ? '记录中' : '尚未运行或已停止' }}</span><button class="text-button" @click="runtime.events.value = []">清空</button></div><div class="event-table"><div v-if="!eventList.length" class="resources-empty"><Terminal :size="25" /><span>进入预览并触发流程后，在此查看执行路径与结果。</span></div><details v-for="(event, index) in eventList" :key="`${event.runId}-${event.nodeId}-${index}`" class="event-row" :class="{ failed: event.kind.endsWith('failed') }"><summary><time>{{ new Date(event.timestamp).toLocaleTimeString('zh-CN') }}</time><span>{{ event.kind }}</span><code>{{ event.runId?.slice(-8) ?? '—' }}</code><span>{{ editor.project.value.flows.flatMap(f => f.nodes).find(n => n.id === event.nodeId)?.label ?? event.flowId ?? '' }}</span><span class="event-preview">{{ JSON.stringify(event.data) }}</span></summary><pre>{{ JSON.stringify(event.data, null, 2) }}</pre></details></div></div>
    <div v-else-if="expanded" class="issues-content"><div v-if="!issues.length" class="resources-empty"><Circle :size="24" /><span>暂无检查结果。点击顶部「检查」验证项目。</span></div><div v-for="(issue, index) in issues" :key="index" class="issue-row"><AlertCircle :size="16" :class="issue.severity === 'error' ? 'error-copy' : 'muted'" /><code>{{ issue.path }}</code><span>{{ issue.message }}</span></div></div>
  </section>
</template>

<style scoped>
.connection-kind-tabs{display:flex;gap:12px;padding:4px 12px;border-bottom:1px solid var(--line);flex-shrink:0}.connection-kind-tabs button{font-size:11px;padding:4px 8px;color:#69798c}.connection-kind-tabs button.active{color:#0f6cbd;background:#eaf3fc;border-radius:3px}.connection-kind-tabs span{font-size:10px;opacity:.7}
.resources-panel.collapsed { height: 32px; min-height: 32px; }
.resource-tabs { height: 32px; min-height: 32px; box-sizing: border-box; padding: 0 8px; gap: 0; }
.resource-tabs > button:not(.icon-button) { height: 32px; padding: 0 12px; gap: 6px; font-size: 11px; white-space: nowrap; flex-shrink: 0; }
.resource-tabs > button > span { font-size: 10px; min-width: 14px; padding: 0 4px; }
.resource-tabs .spacer { min-width: 8px; }
.resource-toggle { flex-shrink: 0; width: 26px; height: 26px; margin-left: 6px; }
.connections-content { overflow: hidden; flex: 1; }
.connection-list { width: 190px; min-width: 150px; padding: 8px; flex-shrink: 0; }
.connection-item { padding: 7px 8px; }
.connection-item > span { min-width: 0; }
.connection-item strong, .connection-item small { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.connection-settings { padding: 8px 14px; min-width: 0; }
.connection-heading { height: 24px; margin-bottom: 6px; }
.connection-heading > strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.connection-heading .text-button, .connection-heading .status-pill, .connection-heading .icon-button { flex-shrink: 0; }
.connection-grid { grid-template-columns: repeat(auto-fit, minmax(145px, 1fr)); gap: 8px 12px; padding: 0; margin: 0; min-width: 0; }
.connection-grid label { font-size: 11px; margin-bottom: 4px; }
.connection-grid input, .connection-grid select { height: 28px; font-size: 11px; }
.connection-basic { grid-template-columns: minmax(120px, 1fr) minmax(150px, 1.2fr) minmax(100px, .7fr) minmax(160px, 1.3fr); }
.connection-advanced { max-width: 640px; grid-template-columns: repeat(3, minmax(90px, 1fr)); }
.connection-section { border-top: 1px solid var(--line); margin-top: 6px; }
.connection-section > summary { cursor: pointer; color: #617186; font-size: 11px; padding: 7px 0; width: fit-content; }
.connection-section > summary:focus-visible { outline: 2px solid #0f6cbd; outline-offset: 2px; }
.protocol-grid :deep(label) { font-size: 11px; margin-bottom: 4px; }
.protocol-grid :deep(input), .protocol-grid :deep(select) { height: 28px; font-size: 11px; }
.protocol-grid :deep(.schema-json-field) { grid-column: span 2; }
.connection-foot { min-height: 25px; margin-top: 4px; }
.variables-content { padding: 8px 14px; }
.variables-toolbar { gap: 12px; }
.variable-search { position: relative; flex: 0 0 240px; margin: 0; }
.variable-search > svg { position: absolute; left: 8px; top: 50%; transform: translateY(-50%); color: #8995a3; pointer-events: none; }
.variable-search > input { height: 28px; padding-left: 29px; }
.variable-hint { overflow: hidden; white-space: nowrap; text-overflow: ellipsis; }
.variable-no-results { text-align: center; color: #8995a3; height: 60px; }
.events-content { flex: 1; overflow: hidden; padding: 8px 14px; }
.event-table { min-height: 0; }
.issues-content { flex: 1; padding: 8px 14px; }
@media (max-width: 1000px) {
  .connection-list { width: 170px; }
  .connection-basic { grid-template-columns: repeat(2, minmax(140px, 1fr)); }
  .variable-hint { display: none; }
  .resource-tabs > button:not(.icon-button) { padding-inline: 8px; }
}
@media (max-width: 600px) {
  .connection-list { width: 150px; }
  .connection-settings { overflow-x: auto; }
  .connection-grid, .connection-advanced { grid-template-columns: minmax(150px, 1fr); }
  .protocol-grid :deep(.schema-json-field) { grid-column: span 1; }
}
</style>
