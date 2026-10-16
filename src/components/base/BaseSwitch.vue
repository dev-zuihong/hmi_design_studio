<script setup lang="ts">
withDefaults(defineProps<{modelValue:boolean;disabled?:boolean;label?:string}>(),{disabled:false})
const emit=defineEmits<{ 'update:modelValue':[value:boolean] }>()
</script>
<template><label class="base-switch"><input type="checkbox" role="switch" :checked="modelValue" :disabled="disabled" :aria-label="label" @change="emit('update:modelValue',($event.target as HTMLInputElement).checked)"/><span class="switch-track"><i/></span><slot/></label></template>
