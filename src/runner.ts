import { createApp } from 'vue'
import './style.css'
import Runner from './Runner.vue'
import Preview from './Preview.vue'
import RuntimeWindow from './RuntimeWindow.vue'
import { getEnvironment } from './lib/backend'
import { i18n, installDesignerTranslations } from './lib/i18n'
const environment = await getEnvironment()
createApp(new URLSearchParams(location.search).get('view') === 'runtime-window' ? RuntimeWindow : environment.mode === 'preview' ? Preview : Runner).use(i18n).mount('#app')
installDesignerTranslations()
