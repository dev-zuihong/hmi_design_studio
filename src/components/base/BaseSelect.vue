<script setup lang="ts">
withDefaults(defineProps<{modelValue?:string;options:(string|{value:string|number;label:string})[];placeholder?:string;disabled?:boolean;label?:string}>(),{modelValue:'',disabled:false})
const emit=defineEmits<{ 'update:modelValue':[value:string] }>()
</script>
<template><select :value="modelValue" :disabled="disabled" :aria-label="label" @change="emit('update:modelValue',($event.target as HTMLSelectElement).value)"><option v-if="placeholder" value="">{{placeholder}}</option><option v-for="option in options" :key="typeof option==='string'?option:option.value" :value="String(typeof option==='string'?option:option.value)">{{typeof option==='string'?option:option.label}}</option></select></template>
