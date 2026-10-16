import { test, expect } from '@playwright/test'
import { placeWidget,openLogic,placeNode } from './helpers/project'

test('designer adds, edits, duplicates, undoes and reloads a saved project', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await expect(page.locator('.dock-status')).toHaveAttribute('title', '浏览器设计模式')
  await expect(page.locator('fluent-button,fluent-text-input,fluent-switch,fluent-dropdown')).toHaveCount(0)
  await expect(page.locator('.base-button').first()).toBeVisible()
  const widgets = page.locator('.design-widget')
  const count = await widgets.count()
  await placeWidget(page,'文本')
  await expect(widgets).toHaveCount(count + 1)
  await page.getByTitle('复制控件 (Ctrl+D)').click()
  await expect(widgets).toHaveCount(count + 2)
  await page.getByTitle('撤销 (Ctrl+Z)').click()
  await expect(widgets).toHaveCount(count + 1)
  await page.getByTitle('重做 (Ctrl+Y)').click()
  await expect(widgets).toHaveCount(count + 2)
  const downloadPromise = page.waitForEvent('download')
  await page.getByTitle('保存项目 (Ctrl+S)').click()
  const download = await downloadPromise
  const path = await download.path()
  expect(path).toBeTruthy()
  await page.locator('input[accept=".hmiproj,.json"]').setInputFiles(path!)
  await expect(widgets).toHaveCount(count + 2)
  await page.screenshot({ path: 'artifacts/designer.png', fullPage: true })
  expect(errors).toEqual([])
})

test('canvas drag and resize change persisted geometry', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await placeWidget(page, '文本')
  const widget = page.locator('.design-widget').first()
  const before = await widget.evaluate(el => ({ transform: (el as HTMLElement).style.transform, width: (el as HTMLElement).style.width }))
  const bounds = (await widget.boundingBox())!
  await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + bounds.height / 2)
  await page.mouse.down()
  await page.mouse.move(bounds.x + bounds.width / 2 + 60, bounds.y + bounds.height / 2 + 40, { steps: 8 })
  await page.mouse.up()
  await expect.poll(() => widget.evaluate(el => (el as HTMLElement).style.transform)).not.toBe(before.transform)
  const resize = page.locator('.moveable-se')
  await expect(resize).toBeVisible()
  const handle = (await resize.boundingBox())!
  await page.mouse.move(handle.x + handle.width / 2, handle.y + handle.height / 2)
  await page.mouse.down()
  await page.mouse.move(handle.x + handle.width / 2 + 45, handle.y + handle.height / 2 + 25, { steps: 8 })
  await page.mouse.up()
  await expect.poll(() => widget.evaluate(el => (el as HTMLElement).style.width)).not.toBe(before.width)
  expect(errors).toEqual([])
})

test('flow ports can be joined with an actual pointer drag', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  const logic = await openLogic(page)
  await logic.getByTitle('新建流程', { exact: true }).click()
  await placeNode(logic,'界面事件',160,180)
  await placeNode(logic,'调试日志',440,180)
  const source = logic.locator('.vue-flow__node').first().locator('.vue-flow__handle.source')
  const target = logic.locator('.vue-flow__node').nth(1).locator('.vue-flow__handle.target')
  const a = (await source.boundingBox())!, b = (await target.boundingBox())!
  await logic.mouse.move(a.x + a.width / 2, a.y + a.height / 2)
  await logic.mouse.down()
  await logic.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 12 })
  await logic.mouse.up()
  await expect(logic.locator('.vue-flow__edge')).toHaveCount(1)
  expect(errors).toEqual([])
})

test('new workspace starts blank and has no built-in example entry', async ({ page }) => {
  await page.goto('/')
  await expect(page.locator('.design-canvas .widget-view')).toHaveCount(0)
  await expect(page.locator('.page-item')).toHaveCount(1)
  await page.locator('[data-menu-trigger="file"]').click()
  await expect(page.locator('.studio-menu button')).toHaveCount(9)
})
