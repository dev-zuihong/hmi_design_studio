import { createApp } from 'vue'
import './lib/fluent'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'
import './style.css'
import App from './App.vue'
import Preview from './Preview.vue'
import { i18n, installDesignerTranslations } from './lib/i18n'
createApp(new URLSearchParams(window.location.search).get('view') === 'preview' ? Preview : App).use(i18n).mount('#app')
installDesignerTranslations()
