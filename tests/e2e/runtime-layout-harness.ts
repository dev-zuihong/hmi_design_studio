import { createApp, h, reactive, ref } from 'vue'
import RuntimeView from '../../src/components/RuntimeView.vue'
import { createWidget, emptyProject } from '../../src/lib/model'

const project = reactive(emptyProject())
project.width = 800
project.height = 600
project.pages[0]!.responsive = true
project.pages[0]!.layout = { type: 'horizontal', gap: 16, padding: 16 }
for (const text of ['第一项', '第二项']) {
  const widget = createWidget('button', 0, 0)
  widget.props.text = text
  widget.width = 120
  widget.height = 100
  widget.layoutItem = { grow: 1, minWidth: 80 }
  project.pages[0]!.widgets.push(widget)
}

const runtime = {
  currentPage: ref(project.pages[0]!.id),
  snapshot: ref({ variables: {}, points: {}, connections: {}, flows: {}, logs: [], ui: {}, inputs: {} }),
  inputs: ref({}), events: ref([]), running: ref(true), busy: ref(false),
  updateInput: async () => {}, act: async () => {},
}

;(window as any).__layoutProject = project
;(window as any).__setResponsive = (responsive: boolean) => { project.pages[0]!.responsive = responsive }
createApp({ render: () => h(RuntimeView, { project, runtime: runtime as any }) }).mount('#app')
