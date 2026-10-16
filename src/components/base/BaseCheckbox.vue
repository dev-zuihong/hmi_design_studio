<script setup lang="ts">
withDefaults(defineProps<{modelValue:boolean;disabled?:boolean;label?:string}>(),{disabled:false})
const emit=defineEmits<{ 'update:modelValue':[value:boolean] }>()
</script>
<template><label class="base-checkbox"><input type="checkbox" :checked="modelValue" :disabled="disabled" :aria-label="label" @change="emit('update:modelValue',($event.target as HTMLInputElement).checked)"/><slot/></label></template>
