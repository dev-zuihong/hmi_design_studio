import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { access } from 'node:fs/promises'
import path from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const args = process.argv.slice(2)
function option(name, fallback) {
  const index = args.indexOf(name)
  return index < 0 ? fallback : args[index + 1]
}
const studio = path.resolve(option('--studio', 'artifacts/HMI Design Studio.app'))
const runnerArgument = option('--runner')
const runner = runnerArgument ? path.resolve(runnerArgument) : undefined
assert.equal(process.platform, 'darwin', 'Native macOS validation must run on macOS.')
const macOSVersion = Number((await new Promise((resolve, reject) => {
  const command = spawn('sw_vers', ['-productVersion'], { stdio: ['ignore', 'pipe', 'inherit'] })
  let stdout = ''
  command.stdout.setEncoding('utf8').on('data', chunk => stdout += chunk)
  command.once('error', reject)
  command.once('exit', code => code === 0 ? resolve(stdout.trim()) : reject(new Error(`sw_vers exited ${code}`)))
})).split('.')[0])
assert.ok(macOSVersion >= 14, `Native app verification requires macOS 14 or newer; found ${macOSVersion}.`)
await access(studio)
if (runner) await access(runner)

async function launch(bundle, binary) {
  const executable = path.join(bundle, 'Contents', 'MacOS', binary)
  await access(executable)
  const child = spawn(executable, [], {
    cwd: path.dirname(bundle),
    env: { ...process.env },
    stdio: 'ignore',
  })
  let exit = null
  child.once('error', error => { exit = { error: String(error) } })
  child.once('exit', (code, signal) => { exit = { code, signal } })
  try {
    await delay(4500)
    assert.equal(exit, null, `${binary} exited during startup: ${JSON.stringify(exit)}`)
    console.log(`${binary} stayed open for the native startup check.`)
  } finally {
    if (exit === null) {
      child.kill('SIGTERM')
      await Promise.race([new Promise(resolve => child.once('exit', resolve)), delay(5000)])
      if (exit === null) child.kill('SIGKILL')
    }
  }
}

await launch(studio, 'hmi-studio')
if (runner) await launch(runner, 'hmi-runner')
