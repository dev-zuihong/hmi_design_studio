// Windows WebView2 integration using only the PTB simulator. Source projects and
// published files are never changed; test flows/resources live under artifacts.
import { chromium, expect } from '@playwright/test'
import { spawn, execFile } from 'node:child_process'
import { promisify } from 'node:util'
import fs from 'node:fs'
import path from 'node:path'
import assert from 'node:assert/strict'
import crypto from 'node:crypto'

const execute = promisify(execFile)
const args = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index]
  assert(key?.startsWith('--') && process.argv[index + 1], 'Arguments: --exe PATH --project PATH --resources PATH [--output PATH] [--port NUMBER]')
  args.set(key.slice(2), process.argv[index + 1])
}
assert.equal(process.platform, 'win32', 'Native PTB verification requires Windows x64 / WebView2')
const executable = path.resolve(args.get('exe') ?? 'target/x86_64-pc-windows-msvc/release/hmi-runner.exe')
const sourceProject = path.resolve(args.get('project') ?? 'examples/ing-ptb/project.json')
const sourceResources = path.resolve(args.get('resources') ?? '.cache')
const output = path.resolve(args.get('output') ?? `artifacts/native-ptb-${Date.now()}`)
const port = Number(args.get('port') ?? 9345)
assert(Number.isInteger(port) && port > 1024 && port < 65536, 'Invalid debugging port')
assert(fs.existsSync(executable), `Runner missing: ${executable}`)
fs.mkdirSync(output, { recursive: true })
const resources = path.join(output, 'resources')
fs.mkdirSync(resources, { recursive: true })
const project = JSON.parse(fs.readFileSync(sourceProject, 'utf8'))
assert.equal(project.backends?.length, 1, 'The fixture must have exactly one simulator backend')
assert.equal(project.backends[0].providerId, 'ing-ptb')
assert.equal(project.backends[0].mode, 'simulation', 'Refusing a project configured for real hardware')
assert.equal(project.connections?.length ?? 0, 0, 'Native simulator verification must not open serial devices')
project.id = `native-ptb-${Date.now()}-${crypto.randomBytes(3).toString('hex')}`
project.backends[0].config.simulatedDeviceCount = 3
project.backends[0].config.logDirectory = path.join(output, 'provider-logs')
const backendId = project.backends[0].id
const settingsPath = path.join(process.env.LOCALAPPDATA, 'HmiStudio', 'projects', project.id, 'settings.json')
const target = 'x86_64-pc-windows-msvc'
const providerSource = [path.join(sourceResources, 'backends', target, 'ing-ptb'), path.join(sourceResources, 'backends', 'ing-ptb')].find(folder => fs.existsSync(path.join(folder, 'manifest.json')))
assert(providerSource, `PTB provider missing under ${sourceResources}`)
fs.cpSync(providerSource, path.join(resources, 'backends', target, 'ing-ptb'), { recursive: true })
const webviewSource = path.join(sourceResources, 'webview2')
assert(fs.existsSync(path.join(webviewSource, 'msedgewebview2.exe')), `Fixed WebView2 runtime missing: ${webviewSource}`)
fs.cpSync(webviewSource, path.join(resources, 'webview2'), { recursive: true })
for (const asset of project.assets ?? []) {
  const relative = asset.path.replaceAll('\\', '/')
  assert(!path.isAbsolute(relative) && !relative.split('/').includes('..'), `Unsafe fixture asset: ${relative}`)
  const source = [path.join(path.dirname(sourceProject), relative), path.join(sourceResources, relative)].find(file => fs.existsSync(file))
  assert(source, `Fixture asset missing: ${relative}`)
  const destination = path.join(resources, relative)
  fs.mkdirSync(path.dirname(destination), { recursive: true }); fs.copyFileSync(source, destination)
}
// Bypass only native OS file dialogs: the same runtime-import IPC, resource
// registration, backend operations and flow scheduler still execute unchanged.
function testFlow(id, operation, operationArgs) {
  project.flows.push({ id, name: id, enabled: true, timeoutMs: 15000,
    nodes: [{ id: `${id}_entry`, kind: 'trigger.ui', label: 'test', position: { x: 0, y: 0 }, config: {} },
      { id: `${id}_call`, kind: 'backend.invoke', label: operation, position: { x: 200, y: 0 }, config: { backendId, operation, args: operationArgs } }],
    edges: [{ id: `${id}_edge`, source: `${id}_entry`, sourcePort: 'out', target: `${id}_call`, targetPort: 'in' }] })
}
const contextValue = name => ({ source: 'context', path: name })
testFlow('native_import_configuration', 'configuration.import', { deviceKey: contextValue('deviceKey'), resource: { source: 'message', path: 'payload' } })
testFlow('native_export_configuration', 'configuration.export', { deviceKey: contextValue('deviceKey'), useDraft: { source: 'literal', value: true } })
testFlow('native_inspect_firmware', 'firmware.inspect', { resource: { source: 'message', path: 'payload' } })
const projectFile = path.join(resources, 'project.json')
fs.writeFileSync(projectFile, JSON.stringify(project, null, 2))
const fixtureDirectory = path.join(output, 'fixtures')
fs.mkdirSync(fixtureDirectory, { recursive: true })
const firmwareFile = path.join(fixtureDirectory, 'native firmware 测试.bin')
const firmwareBytes = Buffer.from(Array.from({ length: 256 }, (_, index) => index))
fs.writeFileSync(firmwareFile, firmwareBytes)
const config = structuredClone(project.backends[0].config.defaultSettings)
config.tvccVoltage = 2.9; config.burnCount = 23
config.firmwareFunction = 0
config.firmware = [{ enabled: true, filePath: path.basename(firmwareFile), destinationAddress: 0x02002000 }]
const configFile = path.join(fixtureDirectory, 'native configuration 测试.ptbcfg')
fs.writeFileSync(configFile, JSON.stringify(config, null, 2))

const errors = [], steps = [], providerProcesses = new Map()
const report = { executable, sourceProject, sourceResources, projectFile, projectId: project.id, mode: 'simulation', steps, errors, success: false, startedAt: new Date().toISOString() }
const stdout = fs.openSync(path.join(output, 'runner.stdout.log'), 'w'), stderr = fs.openSync(path.join(output, 'runner.stderr.log'), 'w')
const systemRoot = process.env.SystemRoot ?? 'C:\\Windows'
const isolatedEnvironment = { ...process.env }
for (const key of Object.keys(isolatedEnvironment)) if (['PATH', 'PYTHONHOME', 'PYTHONPATH', 'DOTNET_ROOT'].includes(key.toUpperCase())) delete isolatedEnvironment[key]
report.runtimeEnvironment = {
  PATH: [path.join(systemRoot, 'System32'), systemRoot, path.join(systemRoot, 'System32', 'Wbem'), path.join(systemRoot, 'System32', 'WindowsPowerShell', 'v1.0')].join(';'),
  PYTHONHOME: 'C:/nonexistent-hmi-native-test-python', PYTHONPATH: 'C:/nonexistent-hmi-native-test-python', DOTNET_ROOT: 'C:/nonexistent-hmi-native-test-dotnet',
}
const child = spawn(executable, ['--project', projectFile], { cwd: output, windowsHide: true, stdio: ['ignore', stdout, stderr], env: {
  ...isolatedEnvironment, ...report.runtimeEnvironment, HMI_RESOURCE_ROOT: resources, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
} })
fs.closeSync(stdout); fs.closeSync(stderr)
report.pid = child.pid
let browser, main, context, mainHandle, failure
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
async function poll(name, read, predicate, timeout = 30000) {
  const deadline = Date.now() + timeout
  let value
  while (Date.now() < deadline) { value = await read(); if (predicate(value)) return value; await delay(150) }
  throw new Error(`${name} timed out; last value: ${JSON.stringify(value)?.slice(0, 1800)}`)
}
async function powershell(script) { const { stdout } = await execute('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { windowsHide: true, maxBuffer: 8 * 1024 * 1024 }); return stdout.trim() }
async function processes() {
  const output = await powershell('Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,@{Name="Started";Expression={$_.CreationDate.ToUniversalTime().ToString("o")}} | ConvertTo-Json -Compress')
  return JSON.parse(output)
}
async function captureProviders() {
  const list = await processes(), descendantIds = new Set([child.pid])
  for (let count = 0; count < list.length; count++) for (const item of list) if (descendantIds.has(item.ParentProcessId)) descendantIds.add(item.ProcessId)
  const providers = list.filter(item => descendantIds.has(item.ProcessId) && /ing-ptb-backend/i.test(item.Name))
  for (const item of providers) providerProcesses.set(`${item.ProcessId}:${item.Started}`, item)
  return providers
}
async function providersExited(tracked) {
  await poll('provider process shutdown', processes, list => tracked.every(item => !list.some(current => current.ProcessId === item.ProcessId && current.Started === item.Started)), 15000)
}
const instrumentedPages = new WeakSet()
function instrument(page) {
  if (instrumentedPages.has(page)) return
  instrumentedPages.add(page)
  page.on('pageerror', error => errors.push({ page: page.url(), message: error.message }))
  page.on('console', message => { if (message.type() === 'error') errors.push({ page: page.url(), console: message.text(), location: message.location() }) })
  page.on('response', response => { if (response.status() >= 400) errors.push({ page: page.url(), resource: response.url(), status: response.status() }) })
}
async function invoke(page, command, args = {}) { return page.evaluate(({ command, args }) => window.__TAURI_INTERNALS__.invoke(command, args), { command, args }) }
async function snapshot() { const value = await invoke(main, 'runtime_snapshot'); if (value.backends?.[backendId]) assert.equal(value.backends[backendId].mode, 'simulation', 'Unexpected real-hardware backend mode'); return value }
async function waitState(predicate, label) { return poll(label, snapshot, value => value.backends?.[backendId] && predicate(value.backends[backendId], value)) }
const widget = (page, id) => page.locator(`.runtime-widget[data-widget-id="${id}"]`).first()
async function assertTextFits(locator) {
  for (const element of await locator.all()) {
    if (!await element.isVisible()) continue
    const measured = await element.evaluate(element => {
      const outer = element.closest('.widget-content').getBoundingClientRect(), range = document.createRange()
      range.selectNodeContents(element)
      const inner = range.getBoundingClientRect()
      return { text: element.textContent.trim(), outer: outer.toJSON(), inner: inner.toJSON(), fits: inner.left >= outer.left - 1 && inner.top >= outer.top - 1 && inner.right <= outer.right + 1 && inner.bottom <= outer.bottom + 1 }
    })
    assert(measured.fits, `Visible widget text is clipped: ${JSON.stringify(measured)}`)
  }
}
async function step(name, run) { const started = Date.now(); process.stdout.write(`${name}\n`); const detail = await run(); steps.push({ name, durationMs: Date.now() - started, detail }); fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2)); return detail }
async function openSettings(deviceKey) {
  const current = new Set(context.pages())
  await widget(main, 'devices').locator(`[data-row-key="${deviceKey}"]`).getByTitle('设备设置', { exact: true }).click()
  const page = await poll(`settings window ${deviceKey}`, async () => context.pages().find(candidate => !current.has(candidate)), Boolean)
  instrument(page); await page.locator('.runtime-window-shell').waitFor()
  const state = await poll('window context', () => invoke(page, 'get_runtime_window_state'), value => value.instance?.context?.deviceKey === deviceKey)
  assert.equal(state.instance.windowId, 'device_settings'); assert.equal(state.instance.context.backendId, backendId)
  await expect(widget(page, 'settings_tabs').locator('.widget-tabs-header > button')).toHaveCount(6)
  return page
}
async function flow(page, id, payload) {
  const windowState = await invoke(page, 'get_runtime_window_state')
  const runId = await invoke(page, 'trigger_flow', { flowId: id, message: { payload, context: windowState.instance.context }, inputs: windowState.instance.inputs })
  const state = await poll(`flow ${id}`, snapshot, state => state.logs.some(event => event.runId === runId && ['run.completed', 'run.failed'].includes(event.kind)))
  const failed = state.logs.find(event => event.runId === runId && event.kind === 'run.failed')
  assert(!failed, JSON.stringify(failed))
  return state.logs.findLast(event => event.runId === runId && event.nodeId === `${id}_call` && event.kind === 'node.completed')?.data.output.payload
}
try {
  await step('connect real WebView2', async () => {
    await poll('WebView2 endpoint', async () => {
      assert(child.exitCode === null, `Runner exited with ${child.exitCode}`)
      try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`); return true } catch { return false }
    }, Boolean, 60000)
    context = browser.contexts()[0]; context.on('page', instrument)
    main = context.pages()[0] ?? await context.waitForEvent('page'); instrument(main)
    await expect(main.locator('.runner-header .status-pill')).toHaveText('运行中', { timeout: 30000 })
    mainHandle = await powershell(`(Get-Process -Id ${child.pid}).MainWindowHandle.ToInt64()`)
    assert.notEqual(mainHandle, '0', 'Main native window handle missing')
    const state = await waitState(backend => backend.devices.length === 3, 'three simulated devices')
    assert.deepEqual(state.backends[backendId].devices.map(device => device.deviceKey).sort(), ['sim_1', 'sim_2', 'sim_3'])
    await expect(widget(main, 'devices').locator('tbody tr')).toHaveCount(3)
    for (const badge of await widget(main, 'devices').locator('.table-badge').all()) await expect(badge).not.toHaveText('')
    await assertTextFits(main.locator('.widget-number .metric-value, .widget-button fluent-button'))
    const providers = await captureProviders(); assert(providers.length > 0, 'Native provider process was not observed')
    await main.screenshot({ path: path.join(output, 'main.png') })
    return { providers, mode: state.backends[backendId].mode }
  })
  const first = await openSettings('sim_1'), second = await openSettings('sim_2')
  await step('two native windows and reuse existing device window', async () => {
    const count = context.pages().length
    const previousRun = (await snapshot()).logs.findLast(event => event.flowId === 'flow_device_actions' && event.kind === 'run.completed')?.runId
    await widget(main, 'devices').locator('[data-row-key="sim_1"]').getByTitle('设备设置', { exact: true }).click()
    await waitState((_, state) => state.logs.some(event => event.flowId === 'flow_device_actions' && event.kind === 'run.completed' && event.runId !== previousRun), 'focus flow complete')
    assert.equal(context.pages().length, count)
    assert.equal((await invoke(first, 'get_runtime_window_state')).instance.context.deviceKey, 'sim_1')
    return { windows: count }
  })
  await step('independent device drafts and six left tabs', async () => {
    await expect(widget(first, 'target_chip').getByRole('combobox')).not.toHaveText('')
    await expect(first.getByText('未知控件：scroll', { exact: false })).toHaveCount(0)
    await assertTextFits(first.locator('.widget-button fluent-button'))
    const firstVoltage = widget(first, 'tvcc').locator('input'), secondVoltage = widget(second, 'tvcc').locator('input')
    await firstVoltage.fill('2.4'); await firstVoltage.press('Tab')
    await waitState(backend => backend.byDevice.sim_1.draft.tvccVoltage === 2.4, 'first device voltage')
    await expect(secondVoltage).toHaveValue('3.3')
    await secondVoltage.fill('3.0'); await secondVoltage.press('Tab')
    await waitState(backend => backend.byDevice.sim_2.draft.tvccVoltage === 3, 'second device voltage')
    await expect(firstVoltage).toHaveValue('2.4')
    for (const title of ['设备 ID', '基础配置', 'RF 测试', '产品固件', '滚码', '测试固件']) {
      await widget(first, 'settings_tabs').getByRole('button', { name: title, exact: true }).click()
      await expect(widget(first, 'settings_tabs').getByRole('button', { name: title, exact: true })).toHaveClass(/active/)
    }
    await widget(first, 'settings_tabs').getByRole('button', { name: '产品固件', exact: true }).click()
    await expect(widget(first, 'firmware_table').locator('tbody tr')).toHaveCount(5)
    await first.screenshot({ path: path.join(output, 'settings-firmware.png') })
    await second.screenshot({ path: path.join(output, 'settings-second-device.png') })
  })
  await step('manual file import creates an immutable firmware reference', async () => {
    const input = widget(first, 'firmware_table').locator('tbody tr').first().getByRole('textbox', { name: '文件路径' })
    await input.fill(firmwareFile); await input.press('Enter')
    const state = await waitState(backend => backend.byDevice.sim_1.firmwareRows?.[0]?.file?.id, 'firmware reference')
    const resource = state.backends[backendId].byDevice.sim_1.firmwareRows[0].file
    assert.equal(resource.sha256, crypto.createHash('sha256').update(firmwareBytes).digest('hex'))
    fs.writeFileSync(firmwareFile, Buffer.alloc(256, 0xaa))
    const inspected = await flow(first, 'native_inspect_firmware', resource)
    assert.equal(inspected.resource.sha256, resource.sha256, 'Imported resource changed with its source file')
    fs.writeFileSync(firmwareFile, firmwareBytes)
    assert.equal(state.backends[backendId].byDevice.sim_2.draft.tvccVoltage, 3)
    return { resource, inspected }
  })
  await step('configuration imports companion files and exports references', async () => {
    const resource = await invoke(first, 'runtime_import_file', { path: configFile })
    assert.equal(resource.missingResources.length, 0); assert.equal(Object.keys(resource.references).length, 1)
    const result = await flow(first, 'native_import_configuration', resource)
    assert.equal(result.valid, true)
    await waitState(backend => backend.byDevice.sim_1.draft.tvccVoltage === 2.9 && backend.byDevice.sim_1.draft.burnCount === 23, 'imported configuration draft')
    const exported = await flow(first, 'native_export_configuration', null)
    assert.equal(exported.resources.length, 1)
    assert.equal(JSON.parse(exported.content).tvccVoltage, 2.9)
    assert.equal(exported.resources[0].sha256, resource.references[path.basename(firmwareFile)].sha256)
    await widget(first, 'settings_tabs').getByRole('button', { name: '基础配置', exact: true }).click()
    await expect(widget(first, 'tvcc').locator('input')).toHaveValue('2.9')
    await expect(widget(second, 'tvcc').locator('input')).toHaveValue('3')
    return { imported: resource, exported }
  })
  await step('UI configuration and task completion use the simulated wire protocol', async () => {
    assert.equal((await snapshot()).backends[backendId].mode, 'simulation')
    await widget(first, 'send_config').getByRole('button', { name: '下发配置', exact: true }).click()
    await waitState(backend => backend.byDevice.sim_1.state === 'configured' && !backend.byDevice.sim_1.jobId, 'simulated configuration transfer')
    await widget(main, 'devices').locator('[data-row-key="sim_1"]').getByTitle('启动任务', { exact: true }).click()
    const state = await waitState(backend => backend.byDevice.sim_1.passed > 0 && !backend.byDevice.sim_1.jobId, 'simulated C1/C2 completion')
    assert.equal(state.backends[backendId].byDevice.sim_1.hardwareRunning, false)
    assert.equal(state.backends[backendId].byDevice.sim_2.passed, 0)
    await main.screenshot({ path: path.join(output, 'main-completed.png') })
    return { passed: state.backends[backendId].byDevice.sim_1.passed, state: state.backends[backendId].byDevice.sim_1.state }
  })
  await step('stop closes runtime windows and terminates provider', async () => {
    const tracked = await captureProviders()
    await main.getByRole('button', { name: '停止运行', exact: true }).click()
    await expect(main.locator('.runner-header .status-pill')).toHaveText('已停止')
    await poll('runtime window cleanup', async () => context.pages().length, count => count === 1)
    await providersExited(tracked)
    return { terminatedProviders: tracked.map(item => item.ProcessId) }
  })
  await step('stopped connection settings persist both modes and restore simulation before starting', async () => {
    for (const mode of ['usb', 'simulation']) {
      await main.getByRole('button', { name: '连接设置', exact: true }).click()
      const dialog = main.getByRole('dialog')
      await dialog.locator('select').selectOption(mode)
      await dialog.getByRole('button', { name: '保存设置', exact: true }).click()
      await expect(dialog).toHaveCount(0)
      assert.equal(JSON.parse(fs.readFileSync(settingsPath, 'utf8')).backendModes[backendId], mode)
      await expect(main.locator('.runner-header .status-pill')).toHaveText('已停止')
      assert.equal((await captureProviders()).length, 0, 'Saving a stopped connection setting must not start a backend')
    }
    return { persistedMode: JSON.parse(fs.readFileSync(settingsPath, 'utf8')).backendModes[backendId] }
  })
  await step('main native close stops a fresh provider and child windows', async () => {
    await main.getByRole('button', { name: '开始运行', exact: true }).click()
    await waitState(backend => backend.devices.length === 3, 'restarted simulator')
    await openSettings('sim_3')
    const tracked = await captureProviders(); assert(tracked.length > 0)
    await powershell(`Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class HmiNativeClose{[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);}'; [uint32]$hmiTargetPid=0; [HmiNativeClose]::GetWindowThreadProcessId([IntPtr]${mainHandle},[ref]$hmiTargetPid)|Out-Null; if($hmiTargetPid -ne ${child.pid}){throw 'Native window ownership changed'}; [HmiNativeClose]::PostMessage([IntPtr]${mainHandle},16,[IntPtr]::Zero,[IntPtr]::Zero)|Out-Null`)
    await poll('runner process exit', async () => child.exitCode, value => value !== null, 20000)
    await providersExited(tracked)
    return { exitCode: child.exitCode, terminatedProviders: tracked.map(item => item.ProcessId) }
  })
  assert.deepEqual(errors, [], 'Unexpected WebView2 errors')
  report.success = true
} catch (error) {
  failure = error; report.failure = String(error.stack ?? error)
  for (const [index, page] of (context?.pages() ?? []).entries()) {
    try { await page.screenshot({ path: path.join(output, `failure-${index}.png`) }) } catch {}
    try { fs.writeFileSync(path.join(output, `failure-${index}.html`), await page.content()) } catch {}
  }
  try { report.lastSnapshot = await snapshot() } catch {}
} finally {
  // Even a failed settings assertion must leave this simulator-only fixture in
  // simulation mode if it is opened again for inspection.
  if (fs.existsSync(settingsPath)) {
    try { const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf8')); settings.backendModes = { ...settings.backendModes, [backendId]: 'simulation' }; fs.writeFileSync(settingsPath, JSON.stringify(settings, null, 2)) } catch (error) { report.settingsCleanupError = String(error) }
  }
  if (child.exitCode === null) {
    try { if (main && !main.isClosed()) await invoke(main, 'stop_preview') } catch {}
    try { await execute('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true }) } catch {}
  }
  try { await browser?.close() } catch {}
  try {
    const remaining = await processes()
    for (const tracked of providerProcesses.values()) if (remaining.some(item => item.ProcessId === tracked.ProcessId && item.Started === tracked.Started)) {
      report.forcedProviderCleanup ??= []; report.forcedProviderCleanup.push(tracked.ProcessId)
      await execute('taskkill.exe', ['/PID', String(tracked.ProcessId), '/T', '/F'], { windowsHide: true })
    }
  } catch (error) { report.cleanupError = String(error) }
  report.finishedAt = new Date().toISOString(); report.providerProcesses = [...providerProcesses.values()]
  fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2))
  fs.writeFileSync(path.join(output, '.native-errors.json'), JSON.stringify({ errors, failure: report.failure, cleanupError: report.cleanupError }, null, 2))
}
if (failure) { console.error(failure); process.exitCode = 1 }
console.log(JSON.stringify({ success: report.success, output, steps: steps.length, errors: errors.length }))
