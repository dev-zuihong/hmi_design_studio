<script setup lang="ts">
withDefaults(defineProps<{ appearance?: 'primary' | 'secondary' | 'subtle' | 'outline'; disabled?: boolean; title?: string }>(), { appearance: 'secondary', disabled: false })
defineEmits<{ click: [event: MouseEvent] }>()
</script>
<template><button class="base-button" :class="`button-${appearance}`" :disabled="disabled" :title="title" @click="$emit('click',$event)"><slot/></button></template>
