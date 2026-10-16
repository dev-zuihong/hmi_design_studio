import { test, expect, type Page } from '@playwright/test'

type Platform = 'macos-arm64' | 'macos-x64'

function makeProject(withPythonNode = false, python?: { runtimeId: string; runtimeSha256?: string; dependencyLock: unknown[] }) {
  const project: any = {
    schemaVersion: 2,
    id: 'platform-fixture',
    name: '平台测试项目',
    width: 1280,
    height: 720,
    pages: [{ id: 'page_main', name: '主界面', widgets: [], responsive:true, layout:{type:'absolute'}, guides:[] }],
    connections: [],
    variables: [],
    flows: withPythonNode ? [{
      id: 'python-flow',
      name: 'Python 流程',
      enabled: true,
      timeoutMs: 30000,
      nodes: [{
        id: 'python-node', kind: 'python', label: 'Python 代码',
        position: { x: 80, y: 80 }, config: { code: 'def transform(msg):\n    return msg\n' },
      }],
      edges: [],
    }] : [],
    assets: [], actions:[], i18n:{defaultLocale:'zh-CN',locales:['zh-CN','en-US'],messages:{'zh-CN':{},'en-US':{}}},
  }
  if (python) project.python = python
  return project
}

async function installDesktopMock(page: Page, options: {
  platform: Platform
  project?: ReturnType<typeof makeProject>
}) {
  const runtimeId = options.platform === 'macos-arm64'
    ? '3.14.8-aarch64-apple-darwin'
    : '3.14.8-x86_64-apple-darwin'
  const project = options.project ?? makeProject()
  await page.addInitScript(({ project, platform, runtimeId }) => {
    ;(window as any).isTauri = true
    const state = (window as any).__platformMock = {
      project,
      workspaceId:project.id,revision:0,savedRevision:0,path:undefined as string|undefined,dirty:false,
      history:{main:{undo:0,redo:0},logic:{undo:0,redo:0}},
      calls: [] as Array<{ command: string; args: any }>,
      environment: {
        mode: 'studio', version: 'test', pythonAvailable: false,
        runnerAvailable: true, webviewAvailable: true, platform, target: runtimeId.replace(/^3\.14\.8-/, ''),
        pythonRuntimeId: runtimeId,
      },
      importedPython: {
        runtimeId,
        runtimeSha256: 'native-runtime-sha256',
        dependencyLock: [{ name: 'base-runtime-package', version: '1.0.0' }],
      },
      callbacks: new Map<number, (event: any) => void>(),
      nextCallback: 1,
    }
    function applyOps(document:any,operations:any[]){const result=structuredClone(document);for(const operation of operations){let parent=result;for(const segment of operation.path.slice(0,-1))parent=Array.isArray(parent)?parent.find((item:any)=>item.id===segment):parent[segment];const key=operation.path.at(-1);if(key==='$order'&&Array.isArray(parent)){const order=operation.after as string[];const sorted=order.map(id=>parent.find((item:any)=>item.id===id)).filter(Boolean);sorted.push(...parent.filter((item:any)=>!order.includes(item.id)));parent.splice(0,parent.length,...sorted)}else if(Array.isArray(parent)){const index=parent.findIndex((item:any)=>item.id===key);if(!operation.afterExists){if(index>=0)parent.splice(index,1)}else if(index>=0)parent[index]=structuredClone(operation.after);else parent.splice(operation.index??parent.length,0,structuredClone(operation.after))}else if(operation.afterExists)parent[key]=structuredClone(operation.after);else delete parent[key]}return result}
    ;(window as any).__TAURI_INTERNALS__ = {
      transformCallback(callback: (event: any) => void) {
        const id = state.nextCallback++
        state.callbacks.set(id, callback)
        return id
      },
      unregisterCallback(id: number) { state.callbacks.delete(id) },
      invoke(command: string, args: any = {}) {
        state.calls.push({ command, args })
        if (command === 'get_environment') return Promise.resolve(state.environment)
        if (command === 'workspace_state') return Promise.resolve({workspaceId:state.workspaceId,project:structuredClone(state.project),revision:state.revision,savedRevision:state.savedRevision,dirty:state.dirty,path:state.path,histories:structuredClone(state.history)})
        if (command === 'apply_workspace_operations') { state.project=applyOps(state.project,args.operations);state.revision++;state.dirty=true;(state.history as any)[args.windowId].undo++;(state.history as any)[args.windowId].redo=0;return Promise.resolve({workspaceId:state.workspaceId,project:structuredClone(state.project),revision:state.revision,savedRevision:state.savedRevision,dirty:true,path:state.path,histories:structuredClone(state.history)}) }
        if (command === 'undo_workspace' || command === 'redo_workspace') return Promise.resolve({workspaceId:state.workspaceId,project:structuredClone(state.project),revision:state.revision,savedRevision:state.savedRevision,dirty:state.dirty,path:state.path,histories:structuredClone(state.history)})
        if (command === 'save_workspace_package') {state.path=args.path;state.savedRevision=state.revision;state.dirty=false;return Promise.resolve({workspaceId:state.workspaceId,project:structuredClone(state.project),revision:state.revision,savedRevision:state.savedRevision,dirty:false,path:state.path,histories:structuredClone(state.history)})}
        if (command === 'get_initial_project') return Promise.resolve(structuredClone(state.project))
        if (command === 'resolve_assets') return Promise.resolve({})
        if (command === 'list_serial_ports' || command === 'list_plugins') return Promise.resolve([])
        if (command === 'plugin:dialog|open') return Promise.resolve('/mock/python-runtime')
        if (command === 'plugin:dialog|save') return Promise.resolve('/mock/platform-project.hmiproj')
        if (command === 'import_python_runtime') return Promise.resolve(structuredClone(state.importedPython))
        if (command === 'save_project') return Promise.resolve()
        if (command === 'plugin:event|listen') return Promise.resolve(1)
        if (command === 'plugin:event|unlisten') return Promise.resolve()
        return Promise.resolve(null)
      },
      convertFileSrc(path: string) { return path },
    }
  }, { project, platform: options.platform, runtimeId })
}

async function importPythonRuntime(page: Page) {
  await page.getByRole('button', { name: '文件', exact: true }).click()
  await page.getByRole('button', { name: '导入 Python 运行资源', exact: true }).click()
  await expect.poll(() => page.evaluate(() => (window as any).__platformMock.calls.filter((call: any) => call.command === 'import_python_runtime').length)).toBe(1)
}

async function saveAndReadProject(page: Page) {
  await page.getByTitle('保存项目 (Ctrl+S)').click()
  await expect.poll(() => page.evaluate(() => (window as any).__platformMock.calls.filter((call: any) => call.command === 'save_workspace_package').length)).toBeGreaterThan(0)
  return page.evaluate(() => (window as any).__platformMock.project)
}

for (const [platform, label] of [
  ['macos-arm64', 'macOS Apple Silicon'],
  ['macos-x64', 'macOS Intel x64'],
] as const) {
  test(`get_environment maps ${platform} to its platform label`, async ({ page }) => {
    await installDesktopMock(page, { platform })
    await page.goto('/')
    await page.getByRole('button', { name: '工作区设置', exact: true }).click()
    await page.getByRole('button', { name: '使用帮助' }).click()
    await expect(page.getByRole('dialog', { name: '开始使用' })).toContainText(`${label} 程序`)
  })
}

test('new Python example stores the native runtime ID from get_environment', async ({ page }) => {
  await installDesktopMock(page, { platform: 'macos-arm64' })
  await page.goto('/')
  await page.getByRole('button', { name: '工作区设置', exact: true }).click()
  await page.getByRole('button', { name: '使用帮助' }).click()
  await expect(page.getByRole('dialog', { name: '开始使用' })).toContainText('macOS Apple Silicon 程序')
  await page.getByTitle('关闭帮助', { exact: true }).click()
  await page.getByRole('button', { name: '文件', exact: true }).click()
  await page.getByRole('button', { name: '添加 Python 示例流程', exact: true }).click()
  const saved = await saveAndReadProject(page)
  expect(saved.flows.some((flow: any) => flow.nodes.some((node: any) => node.kind === 'python'))).toBe(true)
  expect(saved.python.runtimeId).toBe('3.14.8-aarch64-apple-darwin')
})

test('reimporting the same Python runtime preserves the existing dependency lock', async ({ page }) => {
  const dependencyLock = [{ name: 'custom-package', version: '2.4.1', sha256: 'locked-wheel-sha' }]
  const project = makeProject(true, {
    runtimeId: '3.14.8-aarch64-apple-darwin',
    runtimeSha256: 'old-runtime-sha256',
    dependencyLock,
  })
  await installDesktopMock(page, { platform: 'macos-arm64', project })
  await page.goto('/')
  await importPythonRuntime(page)
  const saved = await saveAndReadProject(page)
  expect(saved.python).toEqual({
    runtimeId: '3.14.8-aarch64-apple-darwin',
    runtimeSha256: 'native-runtime-sha256',
    dependencyLock,
  })
})

test('importing Python runtime initializes a missing config when the project has Python nodes', async ({ page }) => {
  await installDesktopMock(page, { platform: 'macos-arm64', project: makeProject(true) })
  await page.goto('/')
  await importPythonRuntime(page)
  const saved = await saveAndReadProject(page)
  expect(saved.python).toEqual({
    runtimeId: '3.14.8-aarch64-apple-darwin',
    runtimeSha256: 'native-runtime-sha256',
    dependencyLock: [{ name: 'base-runtime-package', version: '1.0.0' }],
  })
})
