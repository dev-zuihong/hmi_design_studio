import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createInterface } from 'node:readline'
import { resolve } from 'node:path'

const executable = resolve(process.argv[2] || 'backends/ing-ptb/publish/ing-ptb-backend.exe')
const child = spawn(executable, [], { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true })
let id = 0, bytes = 0, eventCount = 0, latest = null, stderr = ''
const pending = new Map()
child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk })
const lines = createInterface({ input: child.stdout })
lines.on('line', line => {
  assert.ok(Buffer.byteLength(line) <= 1024 * 1024, 'each JSON-RPC line fits 1 MiB')
  bytes += Buffer.byteLength(line)
  const response = JSON.parse(line)
  if (response.method === 'event') {
    assert.equal(response.params.kind, 'snapshot')
    assert.equal(response.params.generation, response.params.data.generation)
    if (latest) assert.ok(response.params.sequence > latest.sequence)
    latest = response.params.data; eventCount++
  } else {
    const waiter = pending.get(response.id)
    assert.ok(waiter, `unexpected response id ${response.id}`)
    pending.delete(response.id)
    clearTimeout(waiter.timer)
    if (response.error) waiter.reject(new Error(response.error.message))
    else waiter.resolve(response.result)
  }
})
const exited = new Promise((accept, reject) => {
  child.on('error', reject)
  child.on('exit', (code, signal) => {
    for (const waiter of pending.values()) { clearTimeout(waiter.timer); waiter.reject(new Error(`process exited ${code}/${signal}: ${stderr}`)) }
    if (code === 0) accept(); else reject(new Error(`provider exit ${code}/${signal}: ${stderr}`))
  })
})
function rpc(method, params = {}) {
  const requestId = ++id
  return new Promise((accept, reject) => {
    const timer = setTimeout(() => { child.kill(); reject(new Error(`timeout for ${method}: ${stderr}`)) }, 15000)
    pending.set(requestId, { resolve: accept, reject, timer })
    child.stdin.write(`${JSON.stringify({ jsonrpc: '2.0', id: requestId, method, params })}\n`)
  })
}
const invoke = (operation, args = { deviceKey: 'sim_1' }) => rpc('invoke', { operation, args })
async function completed(accepted) {
  assert.equal(accepted.accepted, true)
  const deadline = Date.now() + 10000
  while (Date.now() < deadline) {
    const state = await rpc('snapshot')
    const job = state.jobs.find(item => item.jobId === accepted.jobId)
    if (job.state !== 'running') { assert.equal(job.state, 'completed', job.error); return state }
    await new Promise(accept => setTimeout(accept, 25))
  }
  throw new Error('job did not complete')
}
try {
  const state = await rpc('initialize', { mode: 'simulation', config: { simulatedDeviceCount: 3, autoExportLogs: false }, resources: {} })
  assert.equal(state.apiVersion, 1)
  assert.ok(Number.isSafeInteger(state.generation) && state.generation > 0)
  assert.equal(state.devices.length, 3)
  assert.equal(state.stats.connected, 3)
  assert.equal(state.byDevice.sim_1.firmwareRows.length, 5)
  assert.equal(state.byDevice.sim_1.rollingPreview.count, 52)
  assert.equal(state.autoExportLogs, false)
  await invoke('device.settings.patch', { deviceKey: 'sim_1', path: 'rf.frequencyMhz', value: '2404' })
  await invoke('device.settings.apply')
  await completed(await invoke('task.configure'))
  await completed(await invoke('device.readConfiguration'))
  const burned = await completed(await invoke('task.start'))
  assert.equal(burned.byDevice.sim_1.result.passed, true)
  assert.equal(burned.stats.success, 1)
  assert.equal(burned.byDevice.sim_2.settings.rf.frequencyMhz, 2402)
  const exported = await invoke('configuration.export')
  assert.equal(JSON.parse(exported.content).version, 4)
  assert.deepEqual(exported.resources, [])
  const log = await invoke('logs.export')
  assert.ok(log.content.includes('2404') || log.content.includes('message'))
  await assert.rejects(() => invoke('device.settings.patch', { deviceKey: 'sim_1', path: '__proto__.foo', value: 1 }), /路径/)
  await assert.rejects(() => invoke('firmware.inspect', { resource: { path: 'C:\\arbitrary.bin' } }), /资源/)
  await assert.rejects(() => invoke('nonexistent'), /未知/)
  await rpc('shutdown')
  child.stdin.end()
  await exited
  assert.ok(eventCount > 5)
  console.log(`PASS: NativeAOT JSON-RPC, auto discovery, 3 devices, settings, C1-C6, export and error boundaries; ${eventCount} snapshots / ${bytes} bytes.`)
} catch (error) { child.kill(); await exited.catch(() => {}); throw error }
