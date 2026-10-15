<script setup lang="ts">
withDefaults(defineProps<{ appearance?: 'primary' | 'secondary' | 'subtle' | 'outline'; disabled?: boolean; title?: string }>(), { appearance: 'secondary', disabled: false })
defineEmits<{ click: [event: MouseEvent] }>()
</script>

<template>
  <fluent-button role="button" :aria-disabled="disabled" :appearance="appearance" :disabled.prop="disabled" :title="title" @click="!disabled && $emit('click', $event)"><slot /></fluent-button>
</template>
