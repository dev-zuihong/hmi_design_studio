<script setup lang="ts">
defineProps<{ items: unknown[]; selected: string | null }>()
const emit = defineEmits<{ select: [item: unknown, index: number] }>()
const key = (item: any, index: number) => String(item?.id ?? item?.key ?? index)
const label = (item: any) => String(item?.label ?? item?.name ?? item?.id ?? '')
</script>
<template><ul class="widget-tree-list"><li v-for="(item,index) in items" :key="key(item,index)"><button :class="{selected:selected===key(item,index)}" @click="emit('select',item,index)">{{ label(item) }}</button><WidgetTreeItems v-if="(item as any)?.children?.length" :items="(item as any).children" :selected="selected" @select="(child,childIndex)=>emit('select',child,childIndex)" /></li></ul></template>
