<script setup lang="ts">
import { computed, ref } from 'vue'
import { byteRangeMask, validByteRange, type ByteRange } from '../lib/widget-data'
const props = withDefaults(defineProps<{ modelValue?: ByteRange; byteLength?: number; allowEmpty?: boolean; disabled?: boolean; readOnly?: boolean; locale?: string }>(), { byteLength: 4 })
const emit = defineEmits<{ change: [value: ByteRange] }>()
const anchor = ref<number | null>(null)
const length = computed(() => Number.isInteger(props.byteLength) && props.byteLength >= 1 && props.byteLength <= 32 ? props.byteLength : 0)
const valid = computed(() => validByteRange(props.modelValue, length.value, props.allowEmpty))
const selection = computed(() => valid.value ? props.modelValue! : { startByte: 0, byteCount: 0 })
const mask = computed(() => byteRangeMask(selection.value, length.value))
const en = computed(() => props.locale?.startsWith('en'))
function select(index: number, extend = false) {
  if (props.disabled || props.readOnly) return
  if (!extend) anchor.value = index
  else if (anchor.value === null || anchor.value >= length.value) anchor.value = valid.value ? selection.value.startByte : index
  const startByte = Math.min(anchor.value, index)
  emit('change', { startByte, byteCount: Math.abs(index - anchor.value) + 1 })
}
function key(event: KeyboardEvent, index: number) {
  const offset = event.key === 'ArrowLeft' ? -1 : event.key === 'ArrowRight' ? 1 : 0
  if (!offset) return
  event.preventDefault(); const next = Math.max(0, Math.min(length.value - 1, index + offset))
  const button = (event.currentTarget as HTMLElement).parentElement?.children[next] as HTMLButtonElement | undefined
  button?.focus(); if (event.shiftKey) select(next, true)
}
</script>
<template><div class="byte-range-picker"><div class="byte-range-grid" role="group" :aria-label="en ? 'Byte selection' : '字节选择'">
  <button v-for="(_, index) in length" :key="index" type="button" :disabled="disabled || readOnly" :aria-pressed="index >= selection.startByte && index < selection.startByte + selection.byteCount" :aria-label="`${en ? 'Byte' : '字节'} ${index}`" @click="select(index, $event.shiftKey)" @keydown="key($event,index)">{{ index }}</button>
  </div><small>{{ en ? 'Shift + click / arrows extends a contiguous range' : 'Shift + 点击 / 方向键扩展连续区间' }}</small><output v-if="valid"><span v-if="selection.byteCount">{{ selection.startByte }} … {{ selection.startByte + selection.byteCount - 1 }} · </span> {{ selection.byteCount }} {{ en ? 'bytes' : '字节' }} · 0x{{ mask }}</output><small v-else role="status" class="invalid">{{ en ? 'Select a valid contiguous byte range (1–32 bytes total)' : '请选择有效连续区间（总长 1–32 字节）' }}</small><button v-if="allowEmpty" type="button" :disabled="disabled || readOnly" @click="emit('change',{startByte:0,byteCount:0})">{{ en ? 'Clear selection' : '清除选择' }}</button></div></template>
<style scoped>.byte-range-picker{display:flex;flex-direction:column;gap:6px;min-width:0}.byte-range-grid{display:flex;flex-wrap:wrap;gap:3px}.byte-range-grid button{height:28px;min-width:28px;border:1px solid #b7c1cd;border-radius:3px;background:#fff;font:inherit;color:inherit;cursor:pointer}.byte-range-grid button[aria-pressed=true]{background:#0f6cbd;color:white;border-color:#0f6cbd}.byte-range-grid button:focus-visible{outline:2px solid #242424;outline-offset:2px}.byte-range-grid button:disabled{cursor:default;opacity:.65}.byte-range-picker small{color:#6b7280;font-size:11px}.byte-range-picker output{font:12px Consolas,monospace}.byte-range-picker .invalid{color:#b10e1e}</style>
