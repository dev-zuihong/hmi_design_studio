import { test, expect } from '@playwright/test'
import { placeWidget, savedProject } from './helpers/project'

test('page and widget layout editors persist responsive tracks, spacing and fill-fit constraints', async ({ page }) => {
  await page.goto('/')
  await placeWidget(page, '文本')
  const widgetNode = page.locator('.design-widget').first()
  const widgetId = await widgetNode.getAttribute('data-widget-id')
  await widgetNode.click()
  await page.locator('[data-section="constraints"] .section-heading button').click()
  await page.getByLabel('Width size mode').selectOption('fill')
  await page.getByLabel('Height size mode').selectOption('fit')

  await page.locator('.page-item.active button').click()
  await expect(page.getByLabel('Layout type')).toBeVisible()
  await page.getByLabel('Layout type').selectOption('grid')
  await page.getByLabel('Column tracks JSON').fill('[{"kind":"fr","value":2},{"kind":"px","value":180}]')
  await page.getByLabel('Column tracks JSON').press('Tab')
  await page.getByLabel('Horizontal gap').fill('14')
  await page.getByLabel('Horizontal gap').press('Tab')
  await page.getByLabel('Padding right').fill('22')
  await page.getByLabel('Padding right').press('Tab')

  const saved = await savedProject(page)
  const stored = saved.pages[0].widgets.find((item: any) => item.id === widgetId)
  expect(stored.layoutItem.size).toEqual({ width: { mode: 'fill' }, height: { mode: 'fit' } })
  expect(saved.pages[0].layout).toMatchObject({ type: 'grid', gapX: 14, paddingSides: { right: 22 }, columnTracks: [{ kind: 'fr', value: 2 }, { kind: 'px', value: 180 }] })
})
