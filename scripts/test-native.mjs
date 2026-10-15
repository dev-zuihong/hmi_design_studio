import { chromium } from '@playwright/test'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'

const executable = path.resolve(process.argv[2] ?? 'target/x86_64-pc-windows-msvc/release/hmi-studio.exe')
const mode = process.argv[3] ?? 'studio'
const port = mode === 'runner' ? 9334 : 9333
const env = { ...process.env, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` }
if (mode === 'studio' && !process.argv.includes('--bundled')) env.HMI_RESOURCE_ROOT = path.resolve('.cache')
else delete env.HMI_RESOURCE_ROOT
// The child receives a deliberately invalid system Python search path.
env.PYTHONPATH = 'C:/nonexistent-hmi-test-python'
const child = spawn(executable, [], { env, windowsHide: true, stdio: 'ignore', cwd: process.env.TEMP })
let browser
try {
  for (let attempt = 0; attempt < 90; attempt++) {
    if (child.exitCode !== null) throw new Error(`Native application exited: ${child.exitCode}`)
    try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); break } catch { await new Promise(resolve => setTimeout(resolve, 500)) }
  }
  assert(browser, 'WebView2 debugging endpoint did not become available')
  const context = browser.contexts()[0]
  let page = context.pages()[0]
  if (!page) page = await context.waitForEvent('page')
  await page.waitForLoadState('domcontentloaded')
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  if (mode === 'studio') {
    await page.getByText('桌面工作区', { exact: true }).waitFor()
    await page.getByRole('button', { name: '预览运行', exact: true }).click()
  } else {
    await page.locator('.runner-header .status-pill').filter({ hasText: '运行中' }).waitFor({ timeout: 30000 })
  }
  await page.locator('.runtime-widget').filter({ hasText: '打开连接' }).getByRole('button').click()
  async function pollSnapshot(predicate) {
    for (let attempt = 0; attempt < 100; attempt++) {
      const value = await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('runtime_snapshot'))
      if (predicate(value)) return value
      await new Promise(resolve => setTimeout(resolve, 100))
    }
    throw new Error('Timed out waiting for the native runtime state')
  }
  await pollSnapshot(snapshot => snapshot.connections.sensor?.status === 'open')
  await page.locator('.runtime-widget').filter({ hasText: '读取温度' }).getByRole('button').click()
  const snapshot = await pollSnapshot(snapshot => snapshot.variables.temperature > 0 && snapshot.logs.some(event => event.kind === 'run.completed' && event.flowId === 'flow_read'))
  assert(snapshot.variables.temperature > 0)
  assert(snapshot.logs.some(event => event.kind === 'run.completed'))
  assert(snapshot.connections.sensor.simulated)
  fs.mkdirSync('artifacts', { recursive: true })
  fs.writeFileSync(`artifacts/native-${mode}-report.json`, JSON.stringify(snapshot, null, 2))
  await page.screenshot({ path: `artifacts/native-${mode}.png` })
  if (mode === 'studio') {
    await page.getByRole('button', { name: '停止预览', exact: true }).click()
    // IPC save/open exercises Unicode/space paths without automating a system file dialog.
    const projectFile = path.resolve('artifacts/native project 测试.json')
    const loaded = await page.evaluate(async projectFile => {
      const invoke = window.__TAURI_INTERNALS__.invoke
      const project = await invoke('get_initial_project')
      await invoke('save_project', { path: projectFile, project })
      return invoke('open_project', { path: projectFile })
    }, projectFile)
    assert.equal(loaded.id, 'temperature-demo')
    const controls = [
      { id: 'test_input', kind: 'input', x: 520, y: 600, value: 'initial', variable: 'test_text', type: 'string' },
      { id: 'test_select', kind: 'select', x: 740, y: 600, value: 'Alpha', variable: 'test_choice', type: 'string' },
      { id: 'test_switch', kind: 'switch', x: 980, y: 620, value: false, variable: 'test_toggle', type: 'boolean' },
    ]
    for (const control of controls) {
      loaded.variables.push({ id: control.variable, name: control.id, type: control.type, initial: control.value })
      loaded.pages[0].widgets.push({ id: control.id, kind: control.kind, name: control.id, x: control.x, y: control.y, width: 210, height: 45, zIndex: 10, props: { text: control.id, value: control.value, options: ['Alpha','Beta'], enabled: true, visible: true }, bindings: {}, events: { change: { kind: 'flow', flowId: `flow_${control.id}`, entryNodeId: 'trigger' } } })
      loaded.flows.push({ id: `flow_${control.id}`, name: control.id, enabled: true, timeoutMs: 5000, nodes: [
        { id: 'trigger', kind: 'trigger.ui', label: 'change', position: { x: 0, y: 0 }, config: {} },
        { id: 'set', kind: 'variable.set', label: 'capture', position: { x: 200, y: 0 }, config: { variableId: control.variable, value: { source: 'input', key: control.id } } },
      ], edges: [{ id: 'edge', source: 'trigger', sourcePort: 'out', target: 'set', targetPort: 'in' }] })
    }
    await page.evaluate(async ({ projectFile, loaded }) => {
      await window.__TAURI_INTERNALS__.invoke('save_project', { path: projectFile, project: loaded })
      await window.__TAURI_INTERNALS__.invoke('open_project', { path: projectFile })
    }, { projectFile, loaded })
    await page.reload()
    await page.getByRole('button', { name: '预览运行', exact: true }).click()
    await page.locator('.runtime-widget fluent-text-input input').fill('native input')
    await pollSnapshot(snapshot => snapshot.variables.test_text === 'native input')
    await page.locator('.runtime-widget fluent-dropdown').click()
    await page.locator('.runtime-widget fluent-option').filter({ hasText: 'Beta' }).click()
    await pollSnapshot(snapshot => snapshot.variables.test_choice === 'Beta')
    await page.locator('.runtime-widget fluent-switch').click()
    await pollSnapshot(snapshot => snapshot.variables.test_toggle === true)
    await page.getByRole('button', { name: '停止预览', exact: true }).click()
  } else {
    await page.getByRole('button', { name: '停止运行', exact: true }).click()
    await page.getByRole('button', { name: '串口设置', exact: true }).click()
    await page.getByRole('button', { name: '保存设置', exact: true }).click()
  }
  assert.deepEqual(errors, [])
  console.log(JSON.stringify({ mode, success: true, temperature: snapshot.variables.temperature, events: snapshot.logs.length }))
  await page.evaluate(() => window.__TAURI_INTERNALS__.invoke('stop_preview'))
} finally {
  if (browser) await browser.close()
  if (child.exitCode === null) { child.kill(); await Promise.race([once(child, 'exit'), new Promise(resolve => setTimeout(resolve, 5000))]) }
}
