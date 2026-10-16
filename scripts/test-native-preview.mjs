// Windows Studio -> independent Runner lifecycle regression. Build Studio with
// custom-protocol,native-preview-smoke so its workspace can be isolated.
// No mocked IPC, hardware resources, or user workspace are used by this test.
import { chromium, expect } from '@playwright/test'
import { spawn, execFile } from 'node:child_process'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
import fs from 'node:fs'
import path from 'node:path'
import net from 'node:net'
import assert from 'node:assert/strict'
import crypto from 'node:crypto'

const execute = promisify(execFile)
const repository = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const args = new Map()
for (let index = 2; index < process.argv.length; index += 2) {
  const key = process.argv[index]?.slice(2)
  assert(['exe', 'resources', 'output', 'port'].includes(key) && process.argv[index].startsWith('--') && process.argv[index + 1],
    'Arguments: --exe PATH --resources PATH --output PATH --port NUMBER')
  args.set(key, process.argv[index + 1])
}
assert.equal(process.platform, 'win32', 'Native preview verification requires Windows x64 / WebView2')
assert.equal(process.arch, 'x64', 'Native preview verification requires Windows x64')
const executable = path.resolve(args.get('exe') ?? 'target/x86_64-pc-windows-msvc/release/hmi-studio.exe')
const sourceResources = path.resolve(args.get('resources') ?? '.cache')
const output = path.resolve(args.get('output') ?? `artifacts/native-preview-${Date.now()}`)
const port = Number(args.get('port') ?? 9351)
assert(Number.isInteger(port) && port > 1024 && port < 65536, 'Invalid debugging port')
assert(fs.existsSync(executable), `Studio missing: ${executable}`)
assert(!fs.existsSync(output) || fs.readdirSync(output).length === 0, 'Use a new or empty output directory; previous artifacts are preserved')
// Refuse an occupied endpoint before connecting, so a developer's existing
// WebView2 instance can never accidentally become the automation target.
await new Promise((resolve, reject) => {
  const server = net.createServer()
  server.once('error', reject)
  server.listen(port, '127.0.0.1', () => server.close(resolve))
})
fs.mkdirSync(output, { recursive: true })
const resources = path.join(output, 'resources')
const runnerSource = path.join(sourceResources, 'templates', 'windows-x64', 'HmiRunner.exe')
assert(fs.existsSync(runnerSource), `Runner template missing: ${runnerSource}`)
assert(fs.existsSync(path.join(sourceResources, 'webview2', 'msedgewebview2.exe')), 'Fixed WebView2 runtime missing')
fs.mkdirSync(path.join(resources, 'templates', 'windows-x64'), { recursive: true })
fs.copyFileSync(runnerSource, path.join(resources, 'templates', 'windows-x64', 'HmiRunner.exe'))
fs.cpSync(path.join(sourceResources, 'webview2'), path.join(resources, 'webview2'), { recursive: true })

const fixture = JSON.parse(fs.readFileSync(path.join(repository, 'resources', 'example', 'project.json'), 'utf8'))
fixture.schemaVersion = 3
fixture.id = `native-preview-${Date.now()}-${crypto.randomBytes(4).toString('hex')}`
fixture.name = 'Native preview lifecycle fixture'
fixture.pages = [fixture.pages[0]]
fixture.pages[0].layout = { type: 'absolute' }
fixture.pages[0].responsive = false
fixture.i18n = { defaultLocale: 'zh-CN', locales: ['zh-CN'], messages: { 'zh-CN': {} } }
fixture.pages[0].widgets = [fixture.pages[0].widgets.find(widget => widget.kind === 'text')]
fixture.pages[0].widgets[0].props.text = 'Independent preview is running'
fixture.pages[0].widgets[0].bindings = {}
fixture.pages[0].widgets[0].events = {}
for (const key of ['connections', 'backends', 'variables', 'flows', 'actions', 'windows', 'assets']) fixture[key] = []
delete fixture.python
const projectFile = path.join(output, 'project.json')
fs.writeFileSync(projectFile, JSON.stringify(fixture, null, 2))
const workspaceRoot = path.join(output, 'workspace', 'active')
const errors = [], steps = [], trackedProcesses = new Map()
const report = { executable, sourceResources, projectFile, projectId: fixture.id, workspaceRoot, port,
  studioSha256: crypto.createHash('sha256').update(fs.readFileSync(executable)).digest('hex'),
  runnerSha256: crypto.createHash('sha256').update(fs.readFileSync(path.join(resources, 'templates', 'windows-x64', 'HmiRunner.exe'))).digest('hex'),
  success: false, steps, errors, startedAt: new Date().toISOString() }
const stdout = fs.openSync(path.join(output, 'studio.stdout.log'), 'w')
const stderr = fs.openSync(path.join(output, 'studio.stderr.log'), 'w')
const environment = { ...process.env }
for (const key of Object.keys(environment)) {
  if (/^HMI_PREVIEW_|^HMI_NATIVE_PREVIEW_|^WEBVIEW2_/i.test(key)) delete environment[key]
}
const child = spawn(executable, ['--project', projectFile], { cwd: output, windowsHide: true,
  stdio: ['ignore', stdout, stderr], env: { ...environment,
    HMI_RESOURCE_ROOT: resources, HMI_NATIVE_WORKSPACE_ROOT: workspaceRoot,
    WEBVIEW2_USER_DATA_FOLDER: path.join(output, 'webview-user-data'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  } })
fs.closeSync(stdout); fs.closeSync(stderr)
report.pid = child.pid
let browser, context, main, logic, studioProcess, studioWindowHandle, failure, spawnError
child.on('error', error => { spawnError = error })
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds))
async function poll(name, read, predicate, timeout = 30000) {
  const deadline = Date.now() + timeout
  let value
  while (Date.now() < deadline) {
    value = await read()
    if (predicate(value)) return value
    await delay(150)
  }
  throw new Error(`${name} timed out; last value: ${JSON.stringify(value)?.slice(0, 1600)}`)
}
async function powershell(script) {
  const { stdout } = await execute('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script],
    { windowsHide: true, maxBuffer: 8 * 1024 * 1024, timeout: 15000 })
  return stdout.trim()
}
async function processes() {
  return JSON.parse(await powershell('ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,@{Name="Started";Expression={$_.CreationDate.ToUniversalTime().ToString("o")}})'))
}
const identity = process => `${process.ProcessId}:${process.Started}`
async function captureDescendants() {
  const list = await processes()
  const studio = list.find(process => process.ProcessId === child.pid)
  if (!studioProcess && studio && child.exitCode === null) studioProcess = studio
  const descendants = new Set(studio && studioProcess && identity(studio) === identity(studioProcess) ? [child.pid] : [])
  for (let pass = 0; pass < list.length; pass++) {
    for (const process of list) if (descendants.has(process.ParentProcessId)) descendants.add(process.ProcessId)
  }
  for (const process of list) if (descendants.has(process.ProcessId)) trackedProcesses.set(identity(process), process)
  return list
}
async function waitExited(process) {
  await poll(`process ${process.ProcessId} exit`, processes, list => !list.some(item => identity(item) === identity(process)), 20000)
}
async function checkedProcess(process) {
  assert(process && trackedProcesses.has(identity(process)), 'Refusing to operate on an untracked process')
  return (await processes()).find(item => identity(item) === identity(process))
}
async function forceKill(process) {
  if (await checkedProcess(process)) {
    await execute('taskkill.exe', ['/PID', String(process.ProcessId), '/F'], { windowsHide: true, timeout: 10000 })
  }
}
async function nativeClose(process, handle) {
  assert(await checkedProcess(process), 'Native close target already exited')
  assert(handle === undefined || /^[1-9]\d*$/.test(handle), 'Invalid native window handle')
  // Validate the creation timestamp again inside the process that sends
  // WM_CLOSE, then confirm ownership of the handle immediately before posting.
  await powershell(`$hmiIdentity = Get-CimInstance Win32_Process -Filter 'ProcessId = ${process.ProcessId}'; if ($null -eq $hmiIdentity -or $hmiIdentity.CreationDate.ToUniversalTime().ToString('o') -ne '${process.Started}') { throw 'Process identity changed' }; $hmiTarget = Get-Process -Id ${process.ProcessId} -ErrorAction Stop; $hmiHandle = ${handle ? `[IntPtr]${handle}` : '$hmiTarget.MainWindowHandle'}; if ($hmiHandle -eq 0) { throw 'Native window not found' }; Add-Type -TypeDefinition 'using System;using System.Runtime.InteropServices;public static class HmiPreviewClose{[DllImport("user32.dll")]public static extern uint GetWindowThreadProcessId(IntPtr h,out uint p);[DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);}'; [uint32]$hmiWindowPid=0; [HmiPreviewClose]::GetWindowThreadProcessId($hmiHandle,[ref]$hmiWindowPid)|Out-Null; if($hmiWindowPid -ne ${process.ProcessId}){throw 'Native window ownership changed'}; if(-not [HmiPreviewClose]::PostMessage($hmiHandle,16,[IntPtr]::Zero,[IntPtr]::Zero)){throw 'WM_CLOSE failed'}`)
}
async function invoke(command, args = {}) {
  return main.evaluate(async ({ command, args }) => {
    let timeout
    try {
      return await Promise.race([
        window.__TAURI_INTERNALS__.invoke(command, args),
        new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(`IPC ${command} timed out`)), 90000) }),
      ])
    } finally { clearTimeout(timeout) }
  }, { command, args })
}
const state = () => invoke('get_preview_state')
function instrument(page) {
  page.on('pageerror', error => errors.push({ type: 'pageerror', page: page.url(), message: error.message }))
  page.on('console', message => { if (message.type() === 'error') errors.push({ type: 'console', page: page.url(), message: message.text() }) })
}
async function idle(timeout = 60000) {
  // Observe unsolicited stopped events before querying native state. A query
  // that reaps a dead process must not conceal a missing UI notification.
  await expect(main.getByRole('button', { name: '预览运行', exact: true })).toBeEnabled({ timeout })
  await expect(main.getByRole('button', { name: '停止预览', exact: true })).toHaveCount(0)
  if (logic && !logic.isClosed()) {
    await expect(logic.getByRole('button', { name: '预览运行', exact: true })).toBeEnabled({ timeout })
    await expect(logic.getByRole('button', { name: '停止预览', exact: true })).toHaveCount(0)
  }
  const value = await poll('stopped preview state', state, value => !value.running)
  assert.equal(value.project, null)
  assert.equal(value.pid, null)
  assert.deepEqual(value.snapshot.connections, {})
  assert.deepEqual(value.snapshot.logs, [])
  return value
}
async function promptlyIdle(started) {
  // This fixture has no device transactions or flows to drain. Waiting for a
  // shutdown reply from an already disconnected peer would waste 25 seconds.
  // Real device projects retain their own transaction completion budgets.
  const budgetMs = 15000
  assert(Date.now() - started < budgetMs, 'An idle fixture took too long to close its preview process')
  const value = await idle(budgetMs - (Date.now() - started))
  assert(Date.now() - started < budgetMs, 'An idle fixture took too long to restore both editor windows')
  return value
}
async function startPreview() {
  await main.getByRole('button', { name: '预览运行', exact: true }).click()
  const value = await poll('live independent preview', state, value => value.running && value.pid, 120000)
  assert.notEqual(value.pid, child.pid, 'Preview must be an independent process')
  assert.equal(value.project.id, fixture.id)
  assert.equal(value.project.connections.length, 0)
  assert.equal(value.project.flows.length, 0)
  const list = await captureDescendants()
  const runner = list.find(process => process.ProcessId === value.pid)
  assert(runner && trackedProcesses.has(identity(runner)), 'Reported Runner is not a child of this Studio')
  assert.equal(runner.ParentProcessId, child.pid)
  await poll('native Runner window', () => powershell(`(Get-Process -Id ${runner.ProcessId}).MainWindowHandle.ToInt64()`), handle => handle !== '0' && handle !== '')
  await expect(main.getByRole('button', { name: '显示预览', exact: true })).toBeEnabled()
  if (logic && !logic.isClosed()) await expect(logic.getByRole('button', { name: '显示预览', exact: true })).toBeEnabled()
  await expect(main.locator('.toast.error')).toHaveCount(0)
  return { state: value, process: runner }
}
async function step(name, run) {
  process.stdout.write(`${name}\n`)
  const started = Date.now()
  const detail = await run()
  steps.push({ name, durationMs: Date.now() - started, detail })
  fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2))
  return detail
}

try {
  await step('connect isolated Studio and inspect initial stopped state', async () => {
    await poll('WebView2 endpoint', async () => {
      assert(!spawnError, String(spawnError))
      assert(child.exitCode === null, `Studio exited with ${child.exitCode}`)
      try { browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`, { timeout: 2000 }); return true } catch { return false }
    }, Boolean, 90000)
    context = browser.contexts()[0]
    main = context.pages()[0] ?? await context.waitForEvent('page')
    instrument(main)
    await main.locator('.studio-shell').waitFor({ timeout: 30000 })
    await expect(main.locator('.project-title-input')).toHaveValue(fixture.name)
    assert.equal((await invoke('get_environment')).mode, 'studio')
    const workspaceLock = path.join(output, 'workspace', 'active.lock')
    assert(fs.existsSync(workspaceLock), 'Workspace override was ignored; build Studio with native-preview-smoke')
    assert.equal(Number(fs.readFileSync(workspaceLock, 'utf8').trim()), child.pid, 'Isolated workspace lock belongs to another process')
    const workspace = await invoke('workspace_state')
    assert.equal(workspace.project.id, fixture.id)
    assert.equal(workspace.dirty, false, 'Fixture must begin clean')
    await captureDescendants()
    studioWindowHandle = await poll('Studio native window', () => powershell(`(Get-Process -Id ${child.pid}).MainWindowHandle.ToInt64()`), handle => handle !== '0' && handle !== '')
    const initial = await idle()
    await expect(main.locator('.toast.error')).toHaveCount(0)
    await main.screenshot({ path: path.join(output, 'studio-idle.png') })
    return { generation: initial.generation, workspaceId: workspace.workspaceId }
  })
  const first = await step('first UI preview launch creates a running Runner', async () => {
    const opened = await startPreview()
    await main.screenshot({ path: path.join(output, 'studio-preview-running.png') })
    // WebView2 may share the inherited debugging endpoint, or only expose the
    // Studio browser process. Native window + Rust RPC are mandatory in either
    // case; record renderer inspection separately instead of assuming one CDP.
    const previewPage = await poll('optional Runner CDP target', async () => {
      const pages = context.pages().filter(page => page !== main && !page.isClosed())
      for (const page of pages) if (await page.locator('.preview-screen').count()) return page
      return null
    }, Boolean, 2500).catch(() => null)
    if (previewPage) {
      await expect(previewPage.locator('.runtime-widget').filter({ hasText: 'Independent preview is running' })).toBeVisible()
      await previewPage.screenshot({ path: path.join(output, 'preview-running.png') })
    }
    return { ...opened, rendererInspected: !!previewPage }
  })
  await step('show preview reuses the same PID and generation', async () => {
    await main.getByRole('button', { name: '显示预览', exact: true }).click()
    await expect(main.getByRole('button', { name: '显示预览', exact: true })).toBeEnabled()
    const current = await state()
    assert.equal(current.pid, first.state.pid)
    assert.equal(current.generation, first.state.generation)
    return { pid: current.pid, generation: current.generation }
  })
  await step('logic window joins the current preview state', async () => {
    await main.getByRole('button', { name: '逻辑流程 ↗', exact: true }).click()
    logic = await poll('logic window', async () => {
      for (const page of context.pages()) if (page !== main && !page.isClosed() && await page.locator('.logic-editor-shell').count()) return page
      return null
    }, Boolean)
    instrument(logic)
    await expect(logic.getByRole('button', { name: '显示预览', exact: true })).toBeEnabled()
    await expect(logic.getByRole('button', { name: '停止预览', exact: true })).toBeEnabled()
    return { url: logic.url() }
  })
  await step('logic UI stop exits Runner and restores both editor windows', async () => {
    await logic.getByRole('button', { name: '停止预览', exact: true }).click()
    const stopped = await idle()
    await waitExited(first.process)
    return { generation: stopped.generation, exitedPid: first.state.pid }
  })
  const second = await step('UI reopens a fresh Runner after stop', async () => {
    const opened = await startPreview()
    assert(opened.state.generation > first.state.generation)
    assert.notEqual(identity(opened.process), identity(first.process))
    return opened
  })
  await step('native Runner close restores Studio stopped state', async () => {
    const started = Date.now()
    await nativeClose(second.process)
    await waitExited(second.process)
    return { exitedPid: second.state.pid, generation: (await promptlyIdle(started)).generation }
  })
  const third = await step('UI reopens after native close', startPreview)
  await step('forced Runner exit is detected without a stuck running state', async () => {
    const started = Date.now()
    await forceKill(third.process)
    await waitExited(third.process)
    return { exitedPid: third.state.pid, generation: (await promptlyIdle(started)).generation }
  })
  const fourth = await step('UI recovers with another preview after process failure', startPreview)
  await step('native Studio close cleans up its active Runner', async () => {
    await captureDescendants()
    await nativeClose(studioProcess, studioWindowHandle)
    await poll('Studio exit', async () => child.exitCode, value => value !== null, 30000)
    await waitExited(fourth.process)
    assert.equal(child.exitCode, 0)
    return { studioExitCode: child.exitCode, exitedRunnerPid: fourth.state.pid }
  })
  assert.deepEqual(errors, [], 'Unexpected Studio WebView2 errors')
  report.success = true
} catch (error) {
  failure = error
  report.failure = String(error.stack ?? error)
  try { if (main && !main.isClosed()) { await main.screenshot({ path: path.join(output, 'failure.png') }); fs.writeFileSync(path.join(output, 'failure.html'), await main.content()) } } catch {}
  try { report.lastState = await state() } catch (error) { report.lastStateError = String(error) }
} finally {
  // Identify only descendants created by this test. Creation times protect
  // against PID reuse; never kill an application by name or global search.
  try {
    await captureDescendants()
    if (child.exitCode === null && main && !main.isClosed()) {
      try { await invoke('stop_preview') } catch {}
    }
    const remaining = new Set((await processes()).map(identity))
    for (const process of [...trackedProcesses.values()].reverse()) {
      if (remaining.has(identity(process))) {
        report.forcedCleanup ??= []
        report.forcedCleanup.push(process.ProcessId)
        await forceKill(process)
      }
    }
  } catch (error) { report.cleanupError = String(error); report.success = false; failure ??= error }
  try { await browser?.close() } catch {}
  report.processes = [...trackedProcesses.values()]
  report.finishedAt = new Date().toISOString()
  fs.writeFileSync(path.join(output, 'report.json'), JSON.stringify(report, null, 2))
}
if (failure) { console.error(failure); process.exitCode = 1 }
console.log(JSON.stringify({ success: report.success, output, steps: steps.length, errors: errors.length }))
