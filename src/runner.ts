import { createApp } from 'vue'
import './lib/fluent'
import './style.css'
import Runner from './Runner.vue'
import Preview from './Preview.vue'
import { getEnvironment } from './lib/backend'
import { i18n, installDesignerTranslations } from './lib/i18n'
const environment = await getEnvironment()
createApp(environment.mode === 'preview' ? Preview : Runner).use(i18n).mount('#app')
installDesignerTranslations()
