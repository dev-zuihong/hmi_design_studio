<script setup lang="ts">
withDefaults(defineProps<{ modelValue?: string; options: string[]; placeholder?: string; disabled?: boolean; label?: string }>(), { modelValue: '', disabled: false })
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
function update(event: Event) { const element = event.target as HTMLElement & { value?: string; selectedOptions?: { value?: string; textContent: string }[] }; emit('update:modelValue', element.value ?? element.selectedOptions?.[0]?.value ?? element.selectedOptions?.[0]?.textContent ?? '') }
</script>
<template><fluent-dropdown :value.prop="modelValue" :disabled.prop="disabled" :placeholder="placeholder" :aria-label="label" @change="update"><fluent-listbox><fluent-option v-for="option in options" :key="option" :value="option" :selected.prop="option === modelValue">{{ option }}</fluent-option></fluent-listbox></fluent-dropdown></template>
