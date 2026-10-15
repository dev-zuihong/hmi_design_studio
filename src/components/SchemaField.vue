<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { parseSchemaInput, schemaType, type Schema } from '../lib/schema'
const props = defineProps<{ modelValue: unknown; schema: Schema | boolean; label: string }>()
const emit = defineEmits<{ 'update:modelValue': [value: unknown] }>()
const error = ref(''), type = computed(() => schemaType(props.schema)), schema = computed<Schema>(() => typeof props.schema === 'boolean' ? {} : props.schema)
const text = computed(() => props.modelValue === undefined ? '' : typeof props.modelValue === 'string' && type.value === 'string' ? props.modelValue : JSON.stringify(props.modelValue, null, 2))
watch(() => [props.modelValue, props.schema], () => error.value = '')
function update(event: Event) { try { const value = parseSchemaInput((event.target as HTMLInputElement).value, props.schema); error.value = ''; emit('update:modelValue', value) } catch (reason) { error.value = reason instanceof Error ? reason.message : String(reason) } }
</script>

<template>
  <label :class="{ 'schema-json-field': ['object', 'array', 'json'].includes(type) }">{{ label }}
    <span v-if="props.schema === false" class="error-copy">协议声明此参数不接受任何值</span>
    <select v-else-if="schema.enum?.length" :value="modelValue === undefined ? '' : JSON.stringify(modelValue)" :aria-label="label" @change="update"><option value="" disabled>选择选项</option><option v-for="(option, index) in schema.enum" :key="index" :value="JSON.stringify(option)">{{ typeof option === 'string' ? option : JSON.stringify(option) }}</option></select>
    <input v-else-if="schema.const !== undefined" :value="typeof schema.const === 'string' ? schema.const : JSON.stringify(schema.const)" :aria-label="label" readonly />
    <select v-else-if="type === 'boolean'" :value="modelValue === undefined ? '' : String(modelValue)" :aria-label="label" @change="update"><option value="" disabled>选择布尔值</option><option value="true">true</option><option value="false">false</option><option v-if="Array.isArray(schema.type) && schema.type.includes('null')" value="null">null</option></select>
    <textarea v-else-if="['object','array','json'].includes(type)" :value="text" class="code-input" rows="3" :placeholder="type === 'object' ? '{}' : type === 'array' ? '[]' : 'JSON 值'" :aria-label="label" :aria-invalid="!!error" @change="update" />
    <input v-else :value="text" :type="['number','integer'].includes(type) ? 'number' : 'text'" :step="type === 'integer' ? '1' : 'any'" :aria-label="label" :aria-invalid="!!error" @change="update" />
    <span v-if="error" class="error-copy" role="alert">{{ error }}</span>
  </label>
</template>
