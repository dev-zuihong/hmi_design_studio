<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Zap } from 'lucide-vue-next'
import BaseButton from './base/BaseButton.vue'
import ValueEditor from './ValueEditor.vue'
import type { Editor } from '../lib/editor'
import type { ValueRef, Widget } from '../lib/model'
import { addQuickAction, type QuickAction } from '../lib/quick-actions'
import { pluginCatalog } from '../lib/resources'
import { schemaDefault } from '../lib/schema'
const props = defineProps<{ editor: Editor; widget: Widget }>()
const kind = ref<QuickAction['kind']>('connection.open'), connectionId = ref(props.editor.project.value.connections[0]?.id ?? ''), commandName = ref(''), params = ref<Record<string, ValueRef>>({}), feedback = ref(''), failed = ref(false)
const plugin = computed(() => pluginCatalog.value.find(plugin => plugin.id === props.editor.project.value.connections.find(connection => connection.id === connectionId.value)?.protocolId))
const commands = computed(() => Array.isArray(plugin.value?.commands) ? plugin.value.commands : Object.entries(plugin.value?.commands ?? {}).map(([name, definition]) => ({ name, ...definition })))
const command = computed(() => commands.value.find(command => command.name === commandName.value))
const fields = computed(() => command.value?.params?.properties ?? {})
watch(connectionId, () => { commandName.value = ''; feedback.value = '' })
watch(commandName, () => { params.value = Object.fromEntries(Object.entries(fields.value).map(([name, schema]: [string, any]) => [name, { source: 'literal', value: schemaDefault(schema) }])); feedback.value = '' })
watch(() => props.widget.id, () => { feedback.value = '' })
function apply() { try { props.editor.mutate(project => addQuickAction(project, props.widget.id, { kind: kind.value, connectionId: connectionId.value, command: commandName.value, params: params.value })); feedback.value = '已创建流程并绑定到当前控件'; failed.value = false } catch (error) { feedback.value = error instanceof Error ? error.message : String(error); failed.value = true } }
</script>

<template><section class="property-section quick-action-editor"><h4>快捷动作</h4><label>动作类型<select v-model="kind" aria-label="快捷动作类型"><option value="connection.open">打开连接</option><option value="connection.close">关闭连接</option><option value="protocol.command">执行协议命令</option></select></label><label>连接资源<select v-model="connectionId" aria-label="快捷动作连接"><option value="">选择串口连接</option><option v-for="connection in editor.project.value.connections" :key="connection.id" :value="connection.id">{{ connection.name }}</option></select></label><template v-if="kind === 'protocol.command'"><label>协议命令<select v-if="commands.length" v-model="commandName" aria-label="快捷动作命令"><option value="">选择协议命令</option><option v-for="item in commands" :key="item.name" :value="item.name">{{ item.label || item.name }}</option></select><input v-else v-model="commandName" aria-label="快捷动作命令" placeholder="协议命令名称" /></label><label v-for="(schema, name) in fields" :key="name">{{ (schema as any).title ?? name }}<ValueEditor :model-value="params[name] ?? { source: 'literal', value: '' }" :project="editor.project.value" @update:model-value="params[name] = $event" /></label></template><BaseButton class="full-width" :disabled="!connectionId || kind === 'protocol.command' && !commandName" @click="apply"><Zap :size="14" />创建并绑定动作</BaseButton><p v-if="feedback" :class="failed ? 'error-copy' : 'muted-copy'" role="status">{{ feedback }}</p><p v-else class="muted-copy">自动生成一个动作流程，后续可在逻辑工作区继续编辑。</p></section></template>
