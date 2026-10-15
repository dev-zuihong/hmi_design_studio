import { test, expect } from '@playwright/test'

test('designer adds, edits, duplicates, undoes and reloads a saved project', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await expect(page.getByText('浏览器设计模式', { exact: true })).toBeVisible()
  expect(await page.evaluate(() => ['fluent-button','fluent-text-input','fluent-switch','fluent-dropdown'].every(tag => !!customElements.get(tag)))).toBe(true)
  const widgets = page.locator('.design-widget')
  const count = await widgets.count()
  await page.locator('.widget-library-item').filter({ hasText: '文本' }).click()
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
  await page.locator('input[type=file]').setInputFiles(path!)
  await expect(widgets).toHaveCount(count + 2)
  await page.screenshot({ path: 'artifacts/designer.png', fullPage: true })
  expect(errors).toEqual([])
})

test('canvas drag and resize change persisted geometry', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
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
  await page.getByRole('button', { name: '逻辑流程', exact: true }).click()
  await page.getByTitle('新建流程', { exact: true }).click()
  await page.locator('.node-library-item').filter({ hasText: '界面事件' }).click()
  await page.locator('.node-library-item').filter({ hasText: '调试日志' }).click()
  const source = page.locator('.vue-flow__node').first().locator('.vue-flow__handle.source')
  const target = page.locator('.vue-flow__node').nth(1).locator('.vue-flow__handle.target')
  const a = (await source.boundingBox())!, b = (await target.boundingBox())!
  await page.mouse.move(a.x + a.width / 2, a.y + a.height / 2)
  await page.mouse.down()
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 12 })
  await page.mouse.up()
  await expect(page.locator('.vue-flow__edge')).toHaveCount(1)
  expect(errors).toEqual([])
})

test('flow editor loads and creates an editable Python flow without executing devices', async ({ page }) => {
  const errors: string[] = []
  page.on('pageerror', error => errors.push(error.message))
  await page.goto('/')
  await page.getByRole('button', { name: '文件', exact: true }).click()
  await page.getByRole('button', { name: '添加 Python 示例流程', exact: true }).click()
  await expect(page.locator('.vue-flow__node')).toHaveCount(6)
  await page.screenshot({ path: 'artifacts/flow-editor.png', fullPage: true })
  await page.getByRole('button', { name: '预览运行', exact: true }).click()
  await expect(page.getByRole('status')).toContainText('浏览器模式支持设计')
  expect(errors).toEqual([])
})
