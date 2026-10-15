<script setup lang="ts">
withDefaults(defineProps<{ modelValue?: string | number; placeholder?: string; disabled?: boolean; type?: string; label?: string }>(), { modelValue: '', type: 'text', disabled: false })
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
function update(event: Event) { emit('update:modelValue', (event.target as HTMLInputElement).value) }
</script>

<template>
  <fluent-text-input :value.prop="String(modelValue ?? '')" :placeholder="placeholder" :disabled.prop="disabled" :type="type" :aria-label="label" @input="update" />
</template>
