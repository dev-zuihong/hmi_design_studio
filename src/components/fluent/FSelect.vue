<script setup lang="ts">
withDefaults(defineProps<{ modelValue?: string; options: (string | { value: string | number; label: string })[]; placeholder?: string; disabled?: boolean; label?: string }>(), { modelValue: '', disabled: false })
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
function update(event: Event) { const element = event.target as HTMLElement & { value?: string; selectedOptions?: { value?: string; textContent: string }[] }; emit('update:modelValue', element.value ?? element.selectedOptions?.[0]?.value ?? element.selectedOptions?.[0]?.textContent ?? '') }
</script>
<template><fluent-dropdown :disabled.prop="disabled" :placeholder="placeholder" :aria-label="label" @change="update"><fluent-listbox><fluent-option v-for="option in options" :key="typeof option==='string'?option:option.value" :value="String(typeof option==='string'?option:option.value)" :selected.prop="String(typeof option==='string'?option:option.value) === modelValue">{{ typeof option==='string'?option:option.label }}</fluent-option></fluent-listbox></fluent-dropdown></template>
