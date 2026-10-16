<script setup lang="ts">
import { useId } from 'vue'
import { ChevronRight } from 'lucide-vue-next'
import { t } from '../lib/i18n'

defineProps<{ title: string; open: boolean; forcedOpen?: boolean }>()
const emit = defineEmits<{ toggle: [] }>()
const contentId = useId()
</script>

<template>
  <section class="inspector-section">
    <h4 class="section-heading">
      <button type="button" :aria-expanded="open" :aria-controls="contentId" :aria-disabled="forcedOpen || undefined" @click="!forcedOpen && emit('toggle')">
        <ChevronRight :size="13" :class="{ expanded: open }" />{{ t(title) }}
      </button>
    </h4>
    <div v-show="open" :id="contentId" class="section-content"><slot /></div>
  </section>
</template>

<style scoped>
.inspector-section{border-bottom:1px solid var(--line);min-width:0;flex-shrink:0}
.section-heading{margin:0;font-size:11px}
.section-heading button{display:flex;align-items:center;gap:6px;width:100%;min-height:30px;padding:6px 12px;text-align:left;font-weight:600;color:#46515f}
.section-heading button:hover{background:#f5f7fa}
.section-heading svg{color:#7a8491;transform:rotate(0deg)}
.section-heading svg.expanded{transform:rotate(90deg)}
.section-content{padding:6px 12px 12px;min-width:0}
</style>
