<script setup lang="ts">
withDefaults(defineProps<{ modelValue: boolean; disabled?: boolean; label?: string }>(), { disabled: false })
const emit = defineEmits<{ 'update:modelValue': [value: boolean] }>()
function update(event: Event) { emit('update:modelValue', (event.target as HTMLInputElement).checked) }
</script>

<template>
  <fluent-switch :checked.prop="modelValue" :disabled.prop="disabled" :aria-label="label" @change="update"><slot /></fluent-switch>
</template>
