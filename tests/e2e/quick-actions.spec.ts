import { test, expect, type Page } from '@playwright/test'
import { savedProject,placeWidget } from './helpers/project'

test('button shortcut creates and persists a single protocol action with a typed parameter', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await placeWidget(page,'按钮')
  await page.locator('.inspector-tabs').getByRole('button', { name: '事件', exact: true }).click()
  await page.getByLabel('快捷动作类型', { exact: true }).selectOption('protocol.command')
  await page.getByLabel('快捷动作连接', { exact: true }).selectOption('sensor')
  await page.getByLabel('快捷动作命令', { exact: true }).selectOption('setTarget')
  await page.locator('.quick-action-editor .value-editor input').fill('37.5')
  await page.getByRole('button', { name: '创建并绑定动作', exact: true }).click()
  await expect(page.locator('.quick-action-editor')).toContainText('已创建流程并绑定到当前控件')
  const project = await savedProject(page)
  const button = project.pages[0].widgets.at(-1), action = button.events.click
  const flow = project.flows.find((item: any) => item.id === action.flowId)
  expect(flow.nodes.map((node: any) => node.kind)).toEqual(['trigger.ui', 'protocol.command'])
  expect(flow.nodes[1].config).toEqual({ connectionId: 'sensor', command: 'setTarget', params: { target: { source: 'literal', value: 37.5 } } })
  expect(flow.edges).toHaveLength(1)
  expect(action.entryNodeId).toBe(flow.nodes[0].id)
  expect(errors).toEqual([])
})

test('serial panel drops at the pointer location and binds both connection buttons', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  const count = await page.locator('.design-widget').count()
  const board = page.locator('.design-board'), box = (await board.boundingBox())!
  const scale = box.width / 1280
  await page.locator('.serial-library-item').dragTo(board, { targetPosition: { x: 160, y: 150 } })
  await expect(page.locator('.design-widget')).toHaveCount(count + 3)
  const project = await savedProject(page), widgets = project.pages[0].widgets.slice(-3)
  expect(widgets.map((widget: any) => widget.kind)).toEqual(['group', 'button', 'button'])
  expect(widgets[0].x).toBeCloseTo(Math.round(Math.max(0,160 / scale - 180) / 8) * 8, 0)
  expect(widgets[0].y).toBeCloseTo(Math.round(Math.max(0,150 / scale - 70) / 8) * 8, 0)
  expect(widgets.slice(1).map((widget: any) => project.flows.find((flow: any) => flow.id === widget.events.click.flowId).nodes[1].kind)).toEqual(['connection.open', 'connection.close'])
  expect(errors).toEqual([])
})

test('plugin configuration editor preserves boolean, enum, object and array types', async ({ page }) => {
  const errors: string[] = []; page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await page.evaluate(async () => {
    const modulePath = performance.getEntriesByType('resource').map(entry=>entry.name).find(name=>name.includes('/src/lib/resources.ts')) ?? '/src/lib/resources.ts'
    const { pluginCatalog } = await import(/* @vite-ignore */ modulePath)
    pluginCatalog.value[0].configSchema = { type: 'object', properties: { enabled: { type: 'boolean', title: '设备启用' }, mode: { type: 'string', title: '模式', enum: ['fast', 'slow'] }, options: { type: 'object', title: '设备选项' }, channels: { type: 'array', title: '通道列表' } } }
  })
  await page.getByLabel('设备启用', { exact: true }).selectOption('true')
  await page.getByLabel('模式', { exact: true }).selectOption('"fast"')
  await page.getByLabel('设备选项', { exact: true }).fill('{"address": 3}')
  await page.getByLabel('通道列表', { exact: true }).fill('[1, 2]')
  await page.getByLabel('通道列表', { exact: true }).blur()
  const project = await savedProject(page)
  expect(project.connections[0].protocolConfig).toEqual({ enabled: true, mode: 'fast', options: { address: 3 }, channels: [1, 2] })
  expect(errors).toEqual([])
})
