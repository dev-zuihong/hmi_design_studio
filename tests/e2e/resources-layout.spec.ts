import { test, expect, type Page } from '@playwright/test'
import { emptyProject } from '../../src/lib/model'
import { savedProject, prepareProtocolConnection } from './helpers/project'

const resourceTab = (page: Page, name: string) => page.locator('.resource-tabs').getByRole('button', { name: new RegExp(`^${name}`) })

test('resource bar starts at 32px and toggles or restores the selected tab', async ({ page }) => {
  await page.goto('/')
  const panel = page.locator('.resources-panel')
  await expect(panel).toHaveClass(/collapsed/)
  expect((await panel.boundingBox())!.height).toBe(32)
  expect((await page.locator('.resource-tabs').boundingBox())!.height).toBe(32)
  await expect(page.locator('.resource-tabs')).toContainText('就绪')
  await resourceTab(page, '项目变量').click()
  await expect(page.locator('.variables-content')).toBeVisible()
  await resourceTab(page, '项目变量').click()
  await expect(panel).toHaveClass(/collapsed/)
  await page.getByTitle('展开面板', { exact: true }).click()
  await expect(page.locator('.variables-content')).toBeVisible()
  await resourceTab(page, '设备连接').click()
  await expect(page.locator('.connections-content')).toBeVisible()
  await resourceTab(page, '设备连接').click()
  await expect(panel).toHaveClass(/collapsed/)
  await page.reload()
  await expect(panel).toHaveClass(/collapsed/)
  await page.getByTitle('展开面板', { exact: true }).click()
  await expect(page.locator('.connections-content')).toBeVisible()
})

test('serial basics remain visible while advanced and typed protocol fields expand on demand', async ({ page }) => {
  await page.goto('/')
  await prepareProtocolConnection(page)
  await page.evaluate(async () => {
    const path = performance.getEntriesByType('resource').map(entry => entry.name).find(name => name.includes('/src/lib/resources.ts')) ?? '/src/lib/resources.ts'
    const { pluginCatalog } = await import(/* @vite-ignore */ path)
    pluginCatalog.value[0].configSchema = { type: 'object', properties: {
      enabled: { type: 'boolean', title: '设备启用' }, mode: { type: 'string', title: '模式', enum: ['fast', 'slow'] },
      options: { type: 'object', title: '设备选项' }, channels: { type: 'array', title: '通道列表' },
    } }
  })
  const settings = page.locator('.connection-settings')
  await expect(settings.getByRole('textbox', { name: '资源名称', exact: true })).toBeVisible()
  for (const name of ['串口', '波特率', '协议插件']) await expect(settings.getByRole('combobox', { name, exact: true })).toBeVisible()
  await expect(settings.locator('.advanced-serial')).not.toHaveAttribute('open')
  await expect(settings.getByLabel('设备启用', { exact: true })).toBeHidden()
  await settings.locator('.advanced-serial > summary').click()
  await settings.getByRole('combobox', { name: '数据位', exact: true }).selectOption('7')
  await settings.getByRole('combobox', { name: '停止位', exact: true }).selectOption('2')
  await settings.getByRole('combobox', { name: '校验', exact: true }).selectOption('even')
  await settings.locator('.protocol-parameters > summary').click()
  await settings.getByLabel('设备启用', { exact: true }).selectOption('true')
  await settings.getByLabel('模式', { exact: true }).selectOption('"slow"')
  await settings.getByLabel('设备选项', { exact: true }).fill('{"address":3}')
  await settings.getByLabel('通道列表', { exact: true }).fill('[1,2]')
  await settings.getByLabel('通道列表', { exact: true }).blur()
  const connection = (await savedProject(page)).connections[0]
  expect(connection).toMatchObject({ dataBits: 7, stopBits: 2, parity: 'even', protocolConfig: { enabled: true, mode: 'slow', options: { address: 3 }, channels: [1, 2] } })
})

test('variable search finds names and IDs without dropping hidden project data', async ({ page }) => {
  const project = emptyProject()
  project.variables = [
    { id: 'temperature', name: '当前温度', type: 'number', initial: 23.6 },
    { id: 'target', name: '目标温度', type: 'number', initial: 37.5 },
    { id: 'ready', name: '设备就绪', type: 'boolean', initial: false },
  ]
  await page.goto('/')
  await page.locator('input[accept=".hmiproj,.json"]').setInputFiles({ name: 'variables.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(project)) })
  await resourceTab(page, '项目变量').click()
  const search = page.getByRole('searchbox', { name: '按名称或 ID 搜索变量', exact: true })
  const rows = page.locator('.variables-content tbody tr')
  await expect(rows).toHaveCount(3)
  await search.fill('温度')
  await expect(rows).toHaveCount(2)
  await search.fill(' TARGET ')
  await expect(rows).toHaveCount(1)
  await expect(rows).toContainText('target')
  await rows.locator('input[type="number"]').fill('42')
  await rows.locator('input[type="number"]').blur()
  await search.fill('missing-variable')
  await expect(page.locator('.variable-no-results')).toContainText('未找到匹配的变量')
  await page.getByRole('button', { name: '添加变量', exact: true }).click()
  await expect(search).toHaveValue('')
  await expect(rows).toHaveCount(4)
  const result = await savedProject(page)
  expect(result.variables).toHaveLength(4)
  expect(result.variables.find((variable: any) => variable.id === 'target').initial).toBe(42)
  expect(result.variables.find((variable: any) => variable.id === 'ready').initial).toBe(false)
})

test('error badge, programmatic reveal, read-only fields and log details survive the compact layout', async ({ page }) => {
  await page.goto('/')
  await page.evaluate(async () => {
    const modulePath = (fragment: string) => performance.getEntriesByType('resource').map(entry => entry.name).find(name => name.includes(fragment)) ?? fragment
    const [{ createApp, h, ref }, { default: ResourcesPanel }, { useEditor }, { emptyProject }, { useRuntime }, { pluginCatalog }] = await Promise.all([
      import(/* @vite-ignore */ modulePath('/node_modules/.vite/deps/vue.js')),
      import(/* @vite-ignore */ modulePath('/src/components/ResourcesPanel.vue')),
      import(/* @vite-ignore */ modulePath('/src/lib/editor.ts')),
      import(/* @vite-ignore */ modulePath('/src/lib/model.ts')),
      import(/* @vite-ignore */ modulePath('/src/lib/backend.ts')),
      import(/* @vite-ignore */ modulePath('/src/lib/resources.ts')),
    ])
    const host = document.createElement('div')
    host.id = 'resource-test-host'
    host.style.cssText = 'position:fixed;inset:0;z-index:30000;background:white;display:flex;align-items:flex-end'
    document.body.append(host)
    createApp({ setup() {
      const project = emptyProject()
      project.connections = [{ id: 'connection-test', name: 'Test connection', port: '', baudRate: 115200, dataBits: 8, stopBits: 1, parity: 'none', protocolId: 'test-protocol', protocolConfig: {} }]
      pluginCatalog.value = [{ id: 'test-protocol', name: 'Test protocol', version: '1.0.0', configSchema: { type: 'object', properties: { enabled: { type: 'boolean', title: '设备启用' } } } }]
      const editor = useEditor(project), runtime = useRuntime(), expanded = ref(false), tab = ref('connections'), panel = ref()
      runtime.events.value = [{ kind: 'node.completed', runId: 'run-one', nodeId: 'node-one', timestamp: 1700000000000, data: { temperature: 23.6 } }, { kind: 'node.failed', runId: 'run-two', nodeId: 'node-two', timestamp: 1700000000001, data: { error: 'Timeout' } }]
      ;(window as any).__resourceTest = { showIssues: () => panel.value.showIssues(), showConnections: () => panel.value.showConnections(), readOnly: () => editor.readOnly.value = true }
      return () => h(ResourcesPanel, { ref: panel, editor, runtime, issues: [{ severity: 'error', path: 'connections.sensor', message: 'Missing port' }], expanded: expanded.value, activeTab: tab.value, 'onUpdate:expanded': (value: boolean) => expanded.value = value, 'onUpdate:activeTab': (value: string) => tab.value = value, style: { width: '100%', '--resources-height': '350px' } }, { status: () => h('span', { class: 'fixture-status' }, 'Fixture status') })
    } }).mount(host)
  })
  const host = page.locator('#resource-test-host'), panel = host.locator('.resources-panel')
  await expect(panel).toHaveClass(/collapsed/)
  await expect(host.locator('.error-count')).toHaveText('1')
  await expect(host.locator('.fixture-status')).toBeVisible()
  await page.evaluate(() => (window as any).__resourceTest.showIssues())
  await expect(host.locator('.issues-content')).toContainText('Missing port')
  await host.locator('.resource-tabs').getByRole('button', { name: /^检查/ }).click()
  await expect(panel).toHaveClass(/collapsed/)
  await expect(host.locator('.error-count')).toBeVisible()
  await page.evaluate(() => { (window as any).__resourceTest.readOnly(); (window as any).__resourceTest.showConnections() })
  await expect(host.getByLabel('资源名称', { exact: true })).toBeDisabled()
  await host.locator('.advanced-serial > summary').click()
  await expect(host.getByRole('combobox', { name: '数据位', exact: true })).toBeDisabled()
  await host.locator('.protocol-parameters > summary').click()
  await expect(host.getByLabel('设备启用', { exact: true })).toBeDisabled()
  await expect(host.getByTitle('添加串口连接', { exact: true })).toBeDisabled()
  await host.locator('.resource-tabs').getByRole('button', { name: /^执行记录/ }).click()
  await expect(host.locator('.event-row')).toHaveCount(2)
  await host.getByLabel('筛选运行记录', { exact: true }).fill('run-one')
  await expect(host.locator('.event-row')).toHaveCount(1)
  await host.locator('.event-row summary').click()
  await expect(host.locator('.event-row pre')).toContainText('23.6')
})
