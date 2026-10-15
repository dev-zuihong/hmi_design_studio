import { setTheme } from '@fluentui/web-components'
import { webLightTheme } from '@fluentui/tokens'
import '@fluentui/web-components/button.js'
import '@fluentui/web-components/text-input.js'
import '@fluentui/web-components/switch.js'
import '@fluentui/web-components/dropdown.js'
import '@fluentui/web-components/option.js'
import '@fluentui/web-components/listbox.js'
setTheme({ ...webLightTheme, fontFamilyBase: '"Segoe UI", "Microsoft YaHei UI", "Microsoft YaHei", sans-serif' })
