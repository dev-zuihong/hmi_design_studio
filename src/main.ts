import { createApp } from 'vue'
import './lib/fluent'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'
import './style.css'
import App from './App.vue'
import Preview from './Preview.vue'
import RuntimeWindow from './RuntimeWindow.vue'
import { i18n, installDesignerTranslations } from './lib/i18n'
const view = new URLSearchParams(window.location.search).get('view')
createApp(view === 'runtime-window' ? RuntimeWindow : view === 'preview' ? Preview : App).use(i18n).mount('#app')
installDesignerTranslations()
