<script setup lang="ts">
withDefaults(defineProps<{ modelValue?: string|number; placeholder?: string; disabled?: boolean; type?: string; label?: string }>(),{modelValue:'',type:'text',disabled:false})
const emit=defineEmits<{ 'update:modelValue':[value:string] }>()
</script>
<template><input :value="String(modelValue??'')" :placeholder="placeholder" :disabled="disabled" :type="type" :aria-label="label" @input="emit('update:modelValue',($event.target as HTMLInputElement).value)"/></template>
