import { test, expect, type Page } from '@playwright/test'
import { createWidget, emptyProject, type Project } from '../../src/lib/model'
import { openLogic, savedProject } from './helpers/project'

async function loadProject(page: Page, project: Project) {
  await page.goto('/')
  await page.locator('input[accept=".hmiproj,.json"]').setInputFiles({
    name: 'inspector-layout.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(project)),
  })
  await expect(page.locator('.design-widget')).toHaveCount(project.pages[0]!.widgets.length)
}

test('property search reveals every field group and restores folds without changing the project', async ({ page }) => {
  const project = emptyProject(), widget = createWidget('button', 120, 100)
  widget.customProperties = { calibration: { type: 'number', value: 2 } }
  project.pages[0]!.widgets = [widget]
  await loadProject(page, project)
  await page.locator('.design-widget').click()
  const inspector = page.locator('.inspector'), search = inspector.getByLabel('搜索属性', { exact: true })
  const appearance = inspector.locator('[data-section="appearance"]'), geometry = inspector.locator('[data-section="geometry"]')
  await expect(inspector.locator('[data-section="content"] .translatable-field>input').first()).toBeVisible()
  await expect(appearance.getByRole('button', { name: '外观', exact: true })).toHaveAttribute('aria-expanded', 'false')
  await geometry.getByRole('button', { name: '位置与尺寸', exact: true }).click()
  const before = await savedProject(page)
  await search.fill('name')
  await expect(inspector.getByLabel('控件名称', { exact: true })).toBeVisible()
  await search.fill('width')
  await expect(inspector.getByLabel('宽度', { exact: true })).toBeVisible()
  await search.fill('fontSize')
  await expect(appearance.getByRole('button', { name: '外观', exact: true })).toHaveAttribute('aria-expanded', 'true')
  await expect(appearance.locator('input[type="number"]')).toBeVisible()
  await search.fill('minWidth')
  await expect(inspector.getByLabel('最小宽度', { exact: true })).toBeVisible()
  await expect(inspector.getByLabel('控件名称', { exact: true })).toBeHidden()
  await search.fill('calibration')
  await expect(inspector.locator('.custom-property>strong')).toHaveText('calibration')
  await expect(inspector.locator('.custom-property')).toBeVisible()
  await search.fill('no-such-property')
  await expect(inspector.getByText('没有匹配的属性', { exact: true })).toBeVisible()
  await search.fill('')
  await expect(geometry.getByRole('button', { name: '位置与尺寸', exact: true })).toHaveAttribute('aria-expanded', 'false')
  await expect(appearance.getByRole('button', { name: '外观', exact: true })).toHaveAttribute('aria-expanded', 'false')
  expect(await savedProject(page)).toEqual(before)
})

test('filtered fields preserve layout ownership, multiselection and undo', async ({ page }) => {
  const project = emptyProject(), frame = createWidget('frame', 60, 60), first = createWidget('button'), second = createWidget('button')
  frame.width = 600; frame.height = 180; frame.layout = { type: 'horizontal', gap: 16, padding: 12 }
  first.parentId = frame.id; second.parentId = frame.id
  project.pages[0]!.widgets = [frame, first, second]
  await loadProject(page, project)
  await page.locator(`[data-widget-id="${first.id}"]`).click()
  const inspector = page.locator('.inspector'), search = inspector.getByLabel('搜索属性', { exact: true })
  await search.fill('X')
  await expect(inspector.locator('[data-section="geometry"] label').filter({ hasText: /^X/ }).locator('input')).toBeDisabled()
  await page.locator(`[data-widget-id="${second.id}"]`).click({ modifiers: ['Control'] })
  await expect(inspector.locator('.panel-title')).toContainText('2 个选中')
  await search.fill('minWidth')
  await inspector.getByLabel('最小宽度', { exact: true }).fill('44')
  await inspector.getByLabel('最小宽度', { exact: true }).blur()
  const changed = await savedProject(page)
  expect(changed.pages[0].widgets.slice(1).map((widget: any) => widget.layoutItem.minWidth)).toEqual([44, 44])
  await page.getByTitle('撤销 (Ctrl+Z)', { exact: true }).click()
  const reverted = await savedProject(page)
  expect(reverted.pages[0].widgets.slice(1).map((widget: any) => widget.layoutItem?.minWidth)).toEqual([undefined, undefined])
})

test('localized property and node searches find visible labels and keep empty groups hidden', async ({ page }) => {
  const project = emptyProject(), frame = createWidget('frame', 100, 100)
  project.pages[0]!.widgets = [frame]
  await loadProject(page, project)
  await page.locator('.design-widget').click()
  const logic = await openLogic(page)
  await page.getByRole('button', { name: '工作区设置', exact: true }).click()
  await page.getByLabel('设计器语言', { exact: true }).selectOption('en-US')
  await page.keyboard.press('Escape')
  await page.locator('.inspector-search input').fill('Padding')
  await expect(page.locator('[data-section="layout"]').getByLabel('Padding', { exact: true })).toBeVisible()
  await page.locator('.inspector-search input').fill('内边距')
  await expect(page.locator('[data-section="layout"]').getByLabel('Padding', { exact: true })).toBeVisible()
  const nodeSearch = logic.locator('.flow-library .library-search input')
  await nodeSearch.fill('protocol.command')
  await expect(logic.locator('.node-library-item')).toHaveCount(1)
  await expect(logic.locator('.node-library-item')).toContainText('Protocol command')
  await expect(logic.locator('.node-library-list .library-section-heading')).toHaveCount(1)
  await nodeSearch.fill('Timer')
  await expect(logic.locator('.node-library-item')).toHaveCount(1)
  await expect(logic.locator('.node-library-item')).toContainText('Timer')
  await nodeSearch.fill('no-such-node')
  await expect(logic.locator('.node-library-item')).toHaveCount(0)
  await expect(logic.locator('.node-library-list .library-section-heading')).toHaveCount(0)
  await expect(logic.locator('.node-search-empty')).toBeVisible()
})

test('Python editor uses available height and execution limits remain editable when expanded', async ({ page }) => {
  await loadProject(page, emptyProject())
  const logic = await openLogic(page)
  await logic.locator('.node-library-item').filter({ hasText: 'Python' }).click()
  const area = (await logic.locator('.flow-workspace').boundingBox())!
  await logic.mouse.click(area.x + 160, area.y + 160)
  const code = logic.getByLabel('Python 代码', { exact: true }), limits = logic.locator('.python-code-section .section-heading button')
  await expect(code).toBeVisible()
  await expect(limits).toHaveAttribute('aria-expanded', 'false')
  await expect(logic.getByLabel('超时（毫秒）', { exact: true })).toBeHidden()
  const height = (await code.boundingBox())!.height
  expect(height).toBeGreaterThanOrEqual(180)
  await logic.setViewportSize({ width: 1500, height: 1100 })
  await expect.poll(async () => (await code.boundingBox())!.height).toBeGreaterThan(height)
  await limits.click()
  await logic.getByLabel('超时（毫秒）', { exact: true }).fill('2500')
  await logic.getByLabel('超时（毫秒）', { exact: true }).blur()
  await code.fill('def transform(msg):\n    return msg')
  const project = await savedProject(logic)
  expect(project.flows[0].nodes[0].config.timeoutMs).toBe(2500)
  expect(project.flows[0].nodes[0].config.code).toBe('def transform(msg):\n    return msg')
  expect(await logic.locator('.node-library-item').first().evaluate(element => element.getBoundingClientRect().height)).toBe(30)
})
