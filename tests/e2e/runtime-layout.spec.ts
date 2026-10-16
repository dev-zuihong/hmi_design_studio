import { test, expect } from '@playwright/test'

test('responsive managed root layout reflows at scale one and fixed mode returns to design-size scaling', async ({ page }) => {
  await page.setViewportSize({ width: 700, height: 500 })
  await page.goto('/tests/e2e/runtime-layout-harness.html')
  const frame = page.locator('.runtime-frame')
  const items = page.locator('.runtime-widget')
  await expect(frame).toHaveAttribute('data-responsive-layout', 'true')
  await expect(frame).toHaveAttribute('data-runtime-scale', '1')
  await expect(items).toHaveCount(2)
  const firstNarrow = await items.nth(0).evaluate(element => element.getBoundingClientRect().width)
  const narrowFrame = await frame.evaluate(element => element.getBoundingClientRect().width)
  expect(narrowFrame).toBeCloseTo(668, 0)

  await page.setViewportSize({ width: 1000, height: 700 })
  await expect.poll(() => frame.evaluate(element => element.getBoundingClientRect().width)).toBeCloseTo(968, 0)
  const firstWide = await items.nth(0).evaluate(element => element.getBoundingClientRect().width)
  expect(firstWide).toBeGreaterThan(firstNarrow + 100)
  await expect(frame).toHaveAttribute('data-runtime-scale', '1')

  await page.evaluate(() => (window as any).__setResponsive(false))
  await expect(frame).toHaveAttribute('data-responsive-layout', 'false')
  await expect.poll(async () => Number(await frame.getAttribute('data-runtime-scale'))).not.toBe(1)
  const fixedFrame = await frame.evaluate(element => element.getBoundingClientRect().width)
  expect(fixedFrame).toBeCloseTo(800 * Number(await frame.getAttribute('data-runtime-scale')), 0)
})
