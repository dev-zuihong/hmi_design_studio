import { expect, test, type Page } from '@playwright/test'
import { mkdir, writeFile } from 'node:fs/promises'
import { openLogic, placeWidget, savedProject } from './helpers/project'

async function dimension(page: Page, selector: string, axis: 'width' | 'height') {
  return page.locator(selector).evaluate((element, dimension) => element.getBoundingClientRect()[dimension], axis)
}

async function resizePanel(page: Page, side: 'left' | 'right' | 'bottom', delta: number) {
  const handle = page.locator(side === 'bottom' ? '.resource-resizer' : `.pane-resizer.${side}`)
  const box = (await handle.boundingBox())!
  const x = box.x + box.width / 2, y = box.y + box.height / 2
  await page.mouse.move(x, y)
  await page.mouse.down()
  await page.mouse.move(x + (side === 'left' ? delta : side === 'right' ? -delta : 0), y - (side === 'bottom' ? delta : 0), { steps: 6 })
  await page.mouse.up()
}

async function settingsAction(page: Page, name: string) {
  await page.getByRole('button', { name: '工作区设置', exact: true }).click()
  await page.getByRole('button', { name, exact: true }).click()
  await expect(page.getByRole('button', { name: '工作区设置', exact: true })).toBeFocused()
}

async function storedLayout(page: Page, windowId: 'main' | 'logic') {
  return page.evaluate(id => JSON.parse(localStorage.getItem(`hmi.studio.layout.v1.${id}`) ?? 'null'), windowId)
}

test('default layout keeps a single compact toolbar and collapsed resources', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('.studio-toolbar')).toBeVisible()
  expect(await dimension(page, '.studio-toolbar', 'height')).toBe(48)
  expect(await dimension(page, '.library', 'width')).toBe(220)
  expect(await dimension(page, '.inspector', 'width')).toBe(300)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(32)
  await expect(page.locator('.resource-toggle')).toHaveAttribute('aria-expanded', 'false')
  await expect(page.locator('.resource-resizer')).toHaveCount(0)
  await expect(page.getByTitle('保存项目 (Ctrl+S)', { exact: true })).toBeVisible()
  await expect(page.locator('.unsaved-dot')).toHaveCount(0)
})

for (const locale of ['zh-CN', 'en-US']) test(`compact workspaces remain reachable at three window sizes in ${locale}`, async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.addInitScript(value => localStorage.setItem('hmi.designerLocale', value), locale)
  await page.goto('/')
  const opened = page.waitForEvent('popup')
  await page.locator('.logic-launch').click()
  const logic = await opened
  logic.on('pageerror', error => errors.push(error.message))
  const measurements = []
  for (const [windowId, editor] of [['main', page], ['logic', logic]] as const) {
    await editor.locator('.studio-toolbar').waitFor()
    for (const [width, height] of [[1920, 1080], [1366, 768], [800, 600]]) {
      await editor.setViewportSize({ width: width!, height: height! })
      await expect.poll(() => dimension(editor, '.studio-toolbar', 'width')).toBe(width)
      const metrics = await editor.evaluate(() => {
        const rect = (selector: string) => {
          const box = document.querySelector(selector)!.getBoundingClientRect()
          return { x: box.x, y: box.y, width: box.width, height: box.height, right: box.right, bottom: box.bottom }
        }
        return { toolbar: rect('.studio-toolbar'), library: rect('.library'), inspector: rect('.inspector'),
          workspace: rect('.canvas-workspace,.flow-workspace'), dock: rect('.resources-panel'),
          canvasHeight: document.querySelector('.canvas-scroll')?.getBoundingClientRect().height,
          toolbarOverflow: document.querySelector('.studio-toolbar')!.scrollWidth > innerWidth,
          pageOverflow: document.documentElement.scrollWidth > innerWidth }
      })
      expect(metrics.toolbarOverflow).toBe(false)
      expect(metrics.pageOverflow).toBe(false)
      expect(metrics.workspace.width).toBeGreaterThanOrEqual(320)
      expect(metrics.inspector.right).toBeLessThanOrEqual(width!)
      expect(metrics.dock.height).toBe(32)
      expect(metrics.dock.bottom).toBe(height)
      if (windowId === 'main' && width === 1366) expect(metrics.canvasHeight! - 313).toBeGreaterThanOrEqual(150)
      await editor.screenshot({ path: `artifacts/layout-${windowId}-${locale}-${width}.png` })
      measurements.push({ windowId, locale, viewport: { width, height }, ...metrics })
    }
  }
  expect(errors).toEqual([])
  await mkdir('artifacts', { recursive: true })
  await writeFile(`artifacts/layout-after-${locale}.json`, JSON.stringify({ measurements, errors }, null, 2))
})

test('dragged panel preferences survive constrained viewports and reload', async ({ page }) => {
  await page.goto('/')
  await resizePanel(page, 'left', 180)
  await resizePanel(page, 'right', 200)
  await page.locator('.resource-tabs').getByRole('button', { name: /设备连接/ }).click()
  await resizePanel(page, 'bottom', 160)
  await expect.poll(() => dimension(page, '.library', 'width')).toBe(400)
  expect(await dimension(page, '.inspector', 'width')).toBe(500)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(400)
  await expect.poll(() => storedLayout(page, 'main')).toMatchObject({ leftWidth: 400, rightWidth: 500, resourceHeight: 400, resourcesExpanded: true })

  await page.setViewportSize({ width: 800, height: 600 })
  await expect.poll(() => dimension(page, '.library', 'width')).toBe(180)
  expect(await dimension(page, '.inspector', 'width')).toBe(300)
  expect(await dimension(page, '.canvas-workspace', 'width')).toBeGreaterThanOrEqual(320)
  expect(await dimension(page, '.canvas-scroll', 'height')).toBeGreaterThanOrEqual(180)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(270)
  expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBe(800)
  expect(await storedLayout(page, 'main')).toMatchObject({ leftWidth: 400, rightWidth: 500, resourceHeight: 400 })

  await page.setViewportSize({ width: 1500, height: 980 })
  await expect.poll(() => dimension(page, '.library', 'width')).toBe(400)
  expect(await dimension(page, '.inspector', 'width')).toBe(500)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(400)
  await page.reload()
  await expect.poll(() => dimension(page, '.library', 'width')).toBe(400)
  expect(await dimension(page, '.inspector', 'width')).toBe(500)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(400)
})

test('UI and logic windows persist separate panel dimensions and resource tabs', async ({ page }) => {
  await page.goto('/')
  await resizePanel(page, 'left', 100)
  await page.locator('.resource-tabs').getByRole('button', { name: /项目变量/ }).click()
  await expect.poll(() => storedLayout(page, 'main')).toMatchObject({ leftWidth: 320, resourceTab: 'variables', resourcesExpanded: true })

  const logic = await openLogic(page)
  expect(await dimension(logic, '.library', 'width')).toBe(220)
  expect(await dimension(logic, '.inspector', 'width')).toBe(300)
  expect(await dimension(logic, '.resources-panel', 'height')).toBe(32)
  await resizePanel(logic, 'left', 60)
  await settingsAction(logic, '收起属性面板')
  await logic.locator('.resource-tabs').getByRole('button', { name: /执行记录/ }).click()
  await expect.poll(() => storedLayout(logic, 'logic')).toMatchObject({ leftWidth: 280, rightCollapsed: true, resourceTab: 'events', resourcesExpanded: true })
  expect(await dimension(page, '.library', 'width')).toBe(320)
  await expect(page.locator('.inspector')).toBeVisible()
  await expect(page.locator('.variables-content')).toBeVisible()

  await logic.close()
  const reopened = await openLogic(page)
  expect(await dimension(reopened, '.library', 'width')).toBe(280)
  await expect(reopened.locator('.inspector')).toBeHidden()
  await expect(reopened.locator('.events-content')).toBeVisible()
  await expect(page.locator('.variables-content')).toBeVisible()
})

test('reset restores layout while preserving saved project content and undo history', async ({ page }) => {
  await page.goto('/')
  await placeWidget(page, '按钮')
  const project = await savedProject(page)
  const undo = page.getByTitle('撤销 (Ctrl+Z)', { exact: true })
  await expect(undo).toBeEnabled()
  await expect(page.locator('.unsaved-dot')).toHaveCount(0)
  await resizePanel(page, 'left', 100)
  await resizePanel(page, 'right', 80)
  await settingsAction(page, '收起组件面板')
  await settingsAction(page, '收起属性面板')
  await expect(page.locator('.library')).toBeHidden()
  await expect(page.locator('.inspector')).toBeHidden()
  await page.locator('.resource-tabs').getByRole('button', { name: /项目变量/ }).click()
  await expect(page.locator('.unsaved-dot')).toHaveCount(0)

  await settingsAction(page, '恢复默认布局')
  expect(await dimension(page, '.library', 'width')).toBe(220)
  expect(await dimension(page, '.inspector', 'width')).toBe(300)
  expect(await dimension(page, '.resources-panel', 'height')).toBe(32)
  expect(await storedLayout(page, 'main')).toEqual({ leftWidth: 220, rightWidth: 300, resourceHeight: 240, leftCollapsed: false, rightCollapsed: false, resourcesExpanded: false, resourceTab: 'connections' })
  await expect(page.locator('.unsaved-dot')).toHaveCount(0)
  await expect(undo).toBeEnabled()
  expect(await savedProject(page)).toEqual(project)
})
