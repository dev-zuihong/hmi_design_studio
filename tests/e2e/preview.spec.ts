import { test, expect, type BrowserContext, type Page } from '@playwright/test'
import { applyOperations,diffDocument } from '../../src/lib/operations'
import { createWidget } from '../../src/lib/model'

type PreviewState = {
  project: any | null
  running: boolean
  currentPage: string | null
  generation: number
  revision?:number
  status?:string
  snapshot: { variables: Record<string, unknown>; points: Record<string, unknown>; connections: Record<string, unknown>; flows: Record<string, unknown>; logs: unknown[];inputs?:Record<string,unknown>;ui?:Record<string,unknown> }
}

function fixtureProject() {
  return {
    schemaVersion: 2,
    id: 'preview-window-fixture',
    name: '独立窗口测试项目',
    width: 1280,
    height: 720,
    pages: [{ responsive:true,layout:{type:'absolute'},guides:[],
      id: 'page_main', name: '主界面', widgets: [
        { id: 'read_button', kind: 'button', name: '读取温度', x: 40, y: 40, width: 140, height: 40, zIndex: 1, props: { text: '读取温度', enabled: true, visible: true }, bindings: {}, events: { click: { kind: 'flow', flowId: 'flow_read', entryNodeId: 'read_trigger' } } },
        { id: 'setpoint_input', kind: 'input', name: '目标温度', x: 40, y: 100, width: 180, height: 40, zIndex: 1, props: { text: '目标温度', value: '10', enabled: true, visible: true }, bindings: {}, events: {} },
        { id: 'temperature', kind: 'number', name: '当前温度', x: 220, y: 40, width: 190, height: 100, zIndex: 1, props: { text: '当前温度', unit: '°C', precision: 1, enabled: true, visible: true }, bindings: { value: { source: 'variable', key: 'temperature' } }, events: {} },
      ],
    }],
    connections: [],
    variables: [{ id: 'temperature', name: '当前温度', type: 'number', initial: 0 }],
    flows: [{ id: 'flow_read', name: '读取温度', enabled: true, timeoutMs: 30000, nodes: [{ id: 'read_trigger', kind: 'trigger.ui', label: '界面事件', position: { x: 0, y: 0 }, config: {} }], edges: [] }],
    assets: [], actions:[], i18n:{defaultLocale:'zh-CN',locales:['zh-CN','en-US'],messages:{'zh-CN':{},'en-US':{}}},
  }
}

/** Share one simulated native session across the main page and the preview popup. */
async function installDesktopMock(context: BrowserContext, page: Page, options: { invalid?: boolean; failOpen?: boolean } = {}) {
  const state = {
    calls: [] as Array<{ page: string; command: string; args: any }>,
    project: fixtureProject(),
    workspaceId:'preview-window-fixture',workspaceRevision:0,savedRevision:0,workspacePath:undefined as string|undefined,workspaceDirty:false,
    histories:{main:{undo:0,redo:0},logic:{undo:0,redo:0}},runtimeProject:fixtureProject(),previewRevision:0,updateStatusByText:{} as Record<string,'applied'|'waiting'|'invalid'>,updateStatus:'applied',updateLog:[] as Array<{text:string;status:string;revision:number;runtimeText:string}>,
    running: false,
    currentPage: 'page_main',
    generation: 0,
    snapshot: { variables: { temperature: 0 }, points: {}, connections: {}, flows: {}, logs: [] as unknown[],inputs:{} as Record<string,unknown>,ui:{} as Record<string,unknown> },
    previewPage: undefined as Page | undefined,
    logicPage: undefined as Page | undefined,
    invalid: options.invalid ?? false,
    failOpen: options.failOpen ?? false,
    failStop: false,
    failStateReads: 0,
    failPreviewStateReads:0,
    sessionStarts: 0,
    focusCount: 0,
    listeners: [] as Array<{ page: Page; event: string; handler: number; eventId: number }>,
    nextEventId: 1,
    getStateCalls: 0,
    runtimeSnapshotCalls: 0,
    deliveredEvents: [] as Array<{ page: string; event: string; payload: any }>,
  }

  async function dispatchEvent(event: string, payload: any) {
    const listeners = state.listeners.filter(listener => listener.event === event)
    for (const listener of listeners) {
      if (listener.page.isClosed()) continue
      state.deliveredEvents.push({ page: listener.page.url(), event, payload })
      await listener.page.evaluate(({ handler, eventId, payload }) => {
        const item = (window as any).__TAURI_INTERNALS__.callbacks.get(handler)
        if (item) {
          const callback = typeof item === 'function' ? item : item.callback
          callback({ event, id: eventId, payload })
        }
      }, { handler: listener.handler, eventId: listener.eventId, payload, event })
    }
  }

  const workspaceState=()=>({workspaceId:state.workspaceId,project:structuredClone(state.project),revision:state.workspaceRevision,savedRevision:state.savedRevision,dirty:state.workspaceDirty,path:state.workspacePath,histories:structuredClone(state.histories)})
  async function emitState() { await dispatchEvent('preview-state-changed', { running: state.running, generation: state.generation,revision:state.previewRevision,status:state.updateStatus }) }

  await context.exposeBinding('__previewHarnessInvoke', async (source, command: string, args: any = {}) => {
    state.calls.push({ page: source.page.url(), command, args })
    if (command === 'get_environment') return { mode: 'studio', version: 'test', pythonAvailable: false, runnerAvailable: true, webviewAvailable: true, platform: 'macos-arm64', target: 'aarch64-apple-darwin', pythonRuntimeId: '3.14.8-aarch64-apple-darwin' }
    if (command === 'workspace_state') return workspaceState()
    if (command === 'resolve_assets') return Object.fromEntries((args.project?.assets??[]).map((asset:any)=>[asset.id,`/fixture-assets/${asset.path}`]))
    if (command === 'apply_workspace_operations') {
      state.project=applyOperations(state.project,args.operations);state.workspaceRevision++;state.workspaceDirty=true
      const history=state.histories[args.windowId]??={undo:0,redo:0};history.undo++;history.redo=0
      const next=workspaceState();await dispatchEvent('workspace-changed',next);return next
    }
    if (command === 'undo_workspace' || command === 'redo_workspace') return workspaceState()
    if (command === 'workspace_mark_saved') {state.savedRevision=args.revision;state.workspaceDirty=false;return workspaceState()}
    if (command === 'save_workspace_package') {state.workspacePath=args.path;state.savedRevision=state.workspaceRevision;state.workspaceDirty=false;const next=workspaceState();await dispatchEvent('workspace-changed',next);return next}
    if (command === 'get_initial_project') return structuredClone(state.project)
    if(command==='open_logic_window'){
      if(state.logicPage&&!state.logicPage.isClosed()){await state.logicPage.bringToFront();return null}
      const popup=await context.newPage();state.logicPage=popup;popup.on('close',()=>{if(state.logicPage===popup)state.logicPage=undefined})
      const url=new URL('/?view=logic',source.page.url());if(args.flowId)url.searchParams.set('flowId',args.flowId);if(args.nodeId)url.searchParams.set('nodeId',args.nodeId);void popup.goto(url.toString());return null
    }
    if (command === 'validate_project') return state.invalid ? [{ severity: 'error', path: 'flows', message: '模拟校验失败' }] : []
    if (command === 'list_serial_ports' || command === 'list_plugins') return []
    if (command === 'open_preview_window') {
      if (state.failOpen) throw new Error('模拟预览窗口创建失败')
      state.runtimeProject = structuredClone(args.project);state.previewRevision=state.workspaceRevision
      if (state.running) {
        state.focusCount += 1
        if (state.previewPage && !state.previewPage.isClosed()) await state.previewPage.bringToFront()
        return null
      }
      state.running = true
      state.updateStatus='applied'
      state.currentPage = state.project.pages[0]?.id ?? ''
      state.generation += 1
      state.sessionStarts += 1
      const popup = await context.newPage()
      state.previewPage = popup
      popup.on('close', () => { if (state.previewPage === popup) state.previewPage = undefined })
      const previewUrl = new URL('/?view=preview', source.page.url()).toString()
      void popup.goto(previewUrl)
      await emitState()
      return null
    }
    if (command === 'get_preview_state') {
      state.getStateCalls += 1
      if (state.failStateReads > 0) {
        state.failStateReads -= 1
        throw new Error('模拟初次预览状态读取失败')
      }
      if(source.page.url().includes('view=preview')&&state.failPreviewStateReads>0){state.failPreviewStateReads--;throw new Error('模拟初次预览状态读取失败')}
      const result: PreviewState = {
        project: structuredClone(state.runtimeProject),
        running: state.running,
        currentPage: state.currentPage,
        generation: state.generation,
        snapshot: structuredClone(state.snapshot),
        revision:state.previewRevision,status:state.updateStatus,
      }
      return result
    }
    if (command === 'runtime_snapshot') {
      state.runtimeSnapshotCalls += 1
      return structuredClone(state.snapshot)
    }
    if(command==='apply_preview_update'){
      const text=args.project.pages.flatMap((page:any)=>page.widgets).find((widget:any)=>widget.id==='read_button')?.props?.text;const status=state.updateStatusByText[String(text)]??'applied';state.updateStatus=status
      if(status==='applied'){state.runtimeProject=structuredClone(args.project);state.previewRevision=args.revision}
      state.updateLog.push({text:String(text),status,revision:args.revision,runtimeText:String(state.runtimeProject.pages.flatMap((page:any)=>page.widgets).find((widget:any)=>widget.id==='read_button')?.props?.text)})
      await emitState()
      return {status,revision:status==='applied'?args.revision:state.previewRevision,issues:status==='invalid'?[{severity:'error',path:'pages',message:'候选界面无效'}]:[]}
    }
    if(command==='update_preview_input'){state.snapshot.inputs??={};state.snapshot.inputs[args.id]=args.value;return null}
    if(command==='set_preview_locale'){await dispatchEvent('preview-locale-changed',{locale:args.locale});return null}
    if (command === 'trigger_flow') {
      state.snapshot.variables.temperature = 23.6
      await emitState()
      return null
    }
    if (command === 'plugin:event|emit') {
      await dispatchEvent(args.event, args.payload)
      return null
    }
    if (command === 'stop_preview') {
      if (state.failStop) throw new Error('模拟停止失败')
      state.running = false
      state.updateStatus='stopped'
      state.generation += 1
      await emitState()
      const popup = state.previewPage
      if (popup && !popup.isClosed()) setTimeout(() => { if (!popup.isClosed()) void popup.close() }, 80)
      return null
    }
    if (command === 'plugin:event|listen') {
      const eventId = state.nextEventId++
      state.listeners.push({ page: source.page, event: args.event, handler: args.handler, eventId })
      return eventId
    }
    if (command === 'plugin:event|unlisten') {
      state.listeners = state.listeners.filter(listener => listener.eventId !== args.eventId)
      return null
    }
    return null
  })

  await context.addInitScript(() => {
    ;(window as any).isTauri = true
    const callbacks = new Map<number, any>()
    let nextCallback = 1
    ;(window as any).__TAURI_INTERNALS__ = {
      callbacks,
      transformCallback(callback: (event: any) => void, once = false) {
        const id = nextCallback++
        callbacks.set(id, once ? { callback, once } : callback)
        return id
      },
      unregisterCallback(id: number) { callbacks.delete(id) },
      invoke(command: string, args: any = {}) { return (window as any).__previewHarnessInvoke(command, args) },
      convertFileSrc(path: string) { return path },
    }
    ;(window as any).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener() {} }
  })

  return state
}

function previewOpenButton(page: Page) {
  return page.getByRole('button', { name: /预览运行|运行预览|打开预览|显示预览|查看预览/ }).first()
}

async function openPreview(page: Page, context: BrowserContext, state: Awaited<ReturnType<typeof installDesktopMock>>) {
  const popupPromise = context.waitForEvent('page', { timeout: 8000 })
  await previewOpenButton(page).click()
  await expect.poll(() => state.calls.filter(call => call.command === 'open_preview_window').length).toBeGreaterThan(0)
  return popupPromise
}
async function openLogic(page:Page,context:BrowserContext,state:Awaited<ReturnType<typeof installDesktopMock>>){const popupPromise=context.waitForEvent('page',{timeout:8000});await page.getByRole('button',{name:/逻辑流程/}).click();await expect.poll(()=>state.calls.filter(call=>call.command==='open_logic_window').length).toBeGreaterThan(0);return popupPromise}

test('opens a separate preview window while the editor stays open; runtime actions and stop state are shared', async ({ page, context }) => {
  const state = await installDesktopMock(context, page)
  await page.goto('/')
  await expect(page.locator('.studio-shell')).toBeVisible()
  const popup = await openPreview(page, context, state)
  await expect(popup).toHaveURL(/view=preview/)
  await expect(popup.getByRole('button', { name: /停止预览/ })).toBeVisible()
  await expect(page.locator('.studio-shell')).toBeVisible()
  await expect(page.locator('.status-ready')).toContainText('运行中')
  await expect(page.locator('.preview-screen')).toHaveCount(0)
  await expect(popup.locator('.studio-shell')).toHaveCount(0)
  await expect(popup.locator('body')).toContainText('独立窗口测试项目')
  await expect.poll(() => state.listeners.filter(listener => listener.event === 'preview-state-changed').length).toBeGreaterThanOrEqual(2)

  await popup.getByRole('button', { name: '读取温度', exact: true }).click()
  await expect.poll(() => state.calls.filter(call => call.command === 'trigger_flow').length).toBe(1)
  await expect(popup.locator('body')).toContainText('23.6')
  expect(state.calls.some(call => call.command === 'start_preview')).toBe(false)

  const popupClosed = popup.waitForEvent('close')
  await popup.getByRole('button', { name: '停止预览', exact: true }).click()
  await popupClosed
  await expect(page.locator('.studio-shell')).toBeVisible()
  await expect(page.locator('.status-ready')).toContainText('就绪')
  expect(state.running).toBe(false)
  expect(state.deliveredEvents.some(event => event.event === 'preview-state-changed' && event.page.includes('view=preview') && !event.payload.running)).toBe(true)
  expect(state.deliveredEvents.some(event => event.event === 'preview-state-changed' && !event.page.includes('view=preview') && !event.payload.running)).toBe(true)
  expect(state.calls.some(call => call.command === 'start_preview')).toBe(false)
})

test('opening preview again focuses the existing window without starting a second session', async ({ page, context }) => {
  const state = await installDesktopMock(context, page)
  await page.goto('/')
  const popup = await openPreview(page, context, state)
  await expect(popup).toHaveURL(/view=preview/)
  await expect(popup.getByRole('button', { name: /停止预览/ })).toBeVisible()
  await expect(previewOpenButton(page)).toBeEnabled()
  await previewOpenButton(page).click()
  await expect.poll(() => state.focusCount).toBe(1)
  expect(state.sessionStarts).toBe(1)
  expect(context.pages()).toHaveLength(2)
  expect(state.calls.filter(call => call.command === 'open_preview_window')).toHaveLength(2)
  expect(state.calls.some(call => call.command === 'start_preview')).toBe(false)
})

test('preview input changes reach the main window flow trigger snapshot', async ({ page, context }) => {
  const state = await installDesktopMock(context, page)
  await page.goto('/')
  const logic = await openLogic(page,context,state)
  await expect(logic).toHaveURL(/view=logic/)
  await expect(logic.locator('.flow-workspace')).toBeVisible()
  await expect.poll(()=>state.listeners.some(listener=>listener.event==='preview-state-changed'&&listener.page.url().includes('view=logic'))).toBe(true)
  const popup = await openPreview(page, context, state)
  await expect(popup).toHaveURL(/view=preview/)
  await expect(popup.getByRole('button', { name: /停止预览/ })).toBeVisible()
  await expect.poll(() => state.listeners.filter(listener => listener.event === 'preview-input-changed').length).toBeGreaterThanOrEqual(2)

  await popup.locator('.widget-input fluent-text-input input').fill('42')
  await expect.poll(() => state.deliveredEvents.some(event => event.event === 'preview-input-changed' && !event.page.includes('view=preview') && event.payload.id === 'setpoint_input' && event.payload.value === '42')).toBe(true)

  await logic.locator('.vue-flow__node').first().click()
  await logic.getByRole('button', { name: '触发此入口', exact: true }).click()
  await expect.poll(() => state.calls.some(call => call.command === 'trigger_flow')).toBe(true)
  const trigger = state.calls.find(call => call.command === 'trigger_flow')!
  expect(trigger.args.inputs.setpoint_input).toBe('42')
  const popupClosed = popup.waitForEvent('close')
  await popup.getByRole('button', { name: '停止预览', exact: true }).click()
  await popupClosed
})

test('preview restores session inputs and keeps them when an unbound default changes during hot update',async({page,context})=>{
  const state=await installDesktopMock(context,page);state.snapshot.inputs.setpoint_input='会话中的目标值';await page.goto('/');const popup=await openPreview(page,context,state);await expect(popup).toHaveURL(/view=preview/)
  const input=popup.locator('.widget-input fluent-text-input input');await expect(input).toHaveValue('会话中的目标值')
  await input.fill('刚输入的目标值');await expect.poll(()=>state.snapshot.inputs.setpoint_input).toBe('刚输入的目标值')
  const next=structuredClone(state.project),target=next.pages[0].widgets.find((widget:any)=>widget.id==='setpoint_input');target.props.value='热更新默认值'
  const operations=diffDocument(state.project,next);await page.evaluate(operations=>(window as any).__TAURI_INTERNALS__.invoke('apply_workspace_operations',{windowId:'main',operations}),operations)
  await expect.poll(()=>state.calls.some(call=>call.command==='apply_preview_update'&&call.args.project.pages[0].widgets.find((widget:any)=>widget.id==='setpoint_input')?.props.value==='热更新默认值')).toBe(true)
  await expect(input).toHaveValue('刚输入的目标值')
  const closed=popup.waitForEvent('close');await popup.getByRole('button',{name:'停止预览',exact:true}).click();await closed
})

test('editor stays editable during preview and update states preserve the last valid runtime revision',async({page,context})=>{
  const state=await installDesktopMock(context,page);await page.goto('/');const popup=await openPreview(page,context,state);await expect(popup).toHaveURL(/view=preview/)
  async function editButtonText(value:string){await page.getByRole('button',{name:'图层'}).click();await page.locator('.object-tree-row').filter({hasText:/读取温度|排队中的修改|无效候选|热更新完成/}).click();await expect(page.locator('.inspector-object')).toContainText(value==='排队中的修改'||value==='无效候选'||value==='热更新完成'?'读取温度':'读取温度');const field=page.locator('.inspector .translatable-field input').first();await expect(field).toBeEnabled();await field.fill(value);await field.press('Tab')}
  async function waitForCandidate(text:string){await expect.poll(()=>state.calls.some(call=>call.command==='apply_preview_update'&&call.args.project.pages.flatMap((item:any)=>item.widgets).find((widget:any)=>widget.id==='read_button')?.props?.text===text)).toBe(true)}
  state.updateStatusByText['排队中的修改']='waiting';state.updateStatusByText['无效候选']='invalid'
  await editButtonText('排队中的修改');await waitForCandidate('排队中的修改');await expect(page.locator('.preview-status')).toContainText('等待更新');await expect(popup.getByRole('button',{name:'读取温度',exact:true})).toBeVisible()
  await editButtonText('无效候选');await waitForCandidate('无效候选');await expect(page.locator('.preview-status')).toContainText('校验失败');await expect(page.locator('.resources-panel')).toHaveClass(/collapsed/);await expect(popup.getByRole('button',{name:'读取温度',exact:true})).toBeVisible()
  await editButtonText('热更新完成');await waitForCandidate('热更新完成');await expect.poll(()=>state.updateLog.some(item=>item.text==='热更新完成'&&item.status==='applied'&&item.runtimeText==='热更新完成')).toBe(true);await expect.poll(()=>state.runtimeProject.pages.flatMap((item:any)=>item.widgets).find((widget:any)=>widget.id==='read_button')?.props?.text).toBe('热更新完成');await expect(popup.getByRole('button',{name:'热更新完成',exact:true})).toBeVisible()
  const popupClosed=popup.waitForEvent('close');await popup.getByRole('button',{name:'停止预览',exact:true}).click();await popupClosed
})

test('preview refreshes resource URLs after an image is added or changed by hot update',async({page,context})=>{
 const state=await installDesktopMock(context,page);await context.route('**/fixture-assets/**',route=>route.fulfill({contentType:'image/svg+xml',body:'<svg xmlns="http://www.w3.org/2000/svg" width="12" height="12"><rect width="12" height="12" fill="blue"/></svg>'}));await page.goto('/');const popup=await openPreview(page,context,state);await expect(popup.locator('.preview-screen')).toBeVisible()
 async function submit(next:any){const operations=diffDocument(state.project,next);await page.evaluate(operations=>(window as any).__TAURI_INTERNALS__.invoke('apply_workspace_operations',{windowId:'main',operations}),operations)}
 const next:any=structuredClone(state.project),image=createWidget('image',450,100);image.props.assetId='hot-image';next.pages[0].widgets.push(image);next.assets=[{id:'hot-image',path:'assets/first.svg'}];await submit(next)
 const rendered=popup.locator('img.widget-image');await expect(rendered).toHaveAttribute('src','/fixture-assets/assets/first.svg');await expect.poll(()=>rendered.evaluate((element:HTMLImageElement)=>element.naturalWidth)).toBe(12)
 const changed:any=structuredClone(state.project);changed.assets[0].path='assets/second.svg';await submit(changed);await expect(rendered).toHaveAttribute('src','/fixture-assets/assets/second.svg');expect(state.sessionStarts).toBe(1)
 const closed=popup.waitForEvent('close');await popup.getByRole('button',{name:'停止预览',exact:true}).click();await closed
})

test('validation and preview-window creation failures leave the editor as the only window', async ({ page, context }) => {
  const state = await installDesktopMock(context, page, { invalid: true })
  await page.goto('/')
  await previewOpenButton(page).click()
  await expect(page.locator('.studio-shell')).toBeVisible()
  await expect.poll(() => state.calls.filter(call => call.command === 'open_preview_window').length).toBe(0)
  await expect(page.locator('.issues-content')).toBeVisible()
  expect(context.pages()).toHaveLength(1)

  state.invalid = false
  state.failOpen = true
  await previewOpenButton(page).click()
  await expect(page.locator('.studio-shell')).toBeVisible()
  await expect.poll(() => state.calls.filter(call => call.command === 'open_preview_window').length).toBe(1)
  expect(context.pages()).toHaveLength(1)
  expect(state.running).toBe(false)
})

test('stop failure keeps the preview window open and can be retried', async ({ page, context }) => {
  const state = await installDesktopMock(context, page)
  await page.goto('/')
  const popup = await openPreview(page, context, state)
  await expect(popup).toHaveURL(/view=preview/)
  await expect(popup.getByRole('button', { name: /停止预览/ })).toBeVisible()
  state.failStop = true
  await popup.getByRole('button', { name: '停止预览', exact: true }).click()
  await expect(popup).toHaveURL(/view=preview/)
  await expect(popup.getByRole('alert')).toContainText('模拟停止失败')
  expect(state.running).toBe(true)

  state.failStop = false
  const popupClosed = popup.waitForEvent('close')
  await popup.getByRole('button', { name: '停止预览', exact: true }).click()
  await popupClosed
  expect(state.running).toBe(false)
})

test('a failed initial preview-state read still leaves a preview window with a stop/retry path', async ({ page, context }) => {
  const state = await installDesktopMock(context, page)
  state.failPreviewStateReads = 1
  await page.goto('/')
  const popup = await openPreview(page, context, state)
  await expect(popup).toHaveURL(/view=preview/)
  const closeButton=popup.getByRole('button',{name:/关闭预览|停止预览/})
  await expect(closeButton).toBeVisible()
  expect(state.running).toBe(true)
  expect(state.getStateCalls).toBeGreaterThanOrEqual(1)
  expect(state.calls.some(call => call.command === 'start_preview')).toBe(false)
  const popupClosed = popup.waitForEvent('close')
  await closeButton.click()
  await popupClosed
  expect(state.running).toBe(false)
})
