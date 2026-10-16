// Reproducible editable engineering example: no PTB-specific Vue screen or scripts.
import fs from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const catalog = JSON.parse(fs.readFileSync(path.join(root, 'resources/widget-catalog.json'), 'utf8'))
const defaults = JSON.parse(fs.readFileSync(path.join(root, 'backends/ing-ptb/default-settings.json'), 'utf8'))
const literal = value => ({ source: 'literal', value })
const message = path => ({ source: 'message', path })
const context = path => ({ source: 'context', path })
const backend = path => ({ source: 'backend', key: 'ptb', path })
const device = path => backend(`byDevice.$deviceKey.${path}`)
const input = key => ({ source: 'input', key })
const base = Object.fromEntries(Object.values(catalog.commonPropertyGroups).flat().filter(item => item.default !== undefined).map(item => [item.key, item.default]))
const project = {
  schemaVersion: 3, id: 'ing-ptb-workbench', name: 'ING PTB 烧录测试工作台', width: 1480, height: 720,
  pages: [{ id: 'main', name: '设备工作台', widgets: [], layout: { type: 'absolute' }, responsive: false, guides: [] },
    { id: 'settings', name: '设备设置', widgets: [], layout: { type: 'absolute' }, responsive: false, guides: [] }],
  windows: [{ id: 'device_settings', pageId: 'settings', title: 'PTB 设备设置', width: 900, height: 650, minWidth: 820, minHeight: 590, instanceKey: 'deviceKey' }],
  backends: [{ id: 'ptb', name: 'ING PTB USB Printer', providerId: 'ing-ptb', providerVersion: '1.0.0', mode: 'simulation', config: { simulatedDeviceCount: 3, defaultSettings: defaults } }],
  connections: [], variables: [], flows: [], actions: [], assets: [],
  i18n: { defaultLocale: 'zh-CN', locales: ['zh-CN', 'en-US'], messages: { 'zh-CN': {}, 'en-US': {} } },
}
let page = project.pages[0]
function widget(id, kind, x, y, width, height, props = {}, bindings = {}, parentId, slotId = 'content') {
  const spec = catalog.widgets.find(item => item.kind === kind)
  if (!spec) throw new Error(`Widget missing from catalog: ${kind}`)
  const value = { id, kind, name: props.text || id, x, y, width, height, zIndex: spec.container ? 0 : 1,
    props: structuredClone({ ...base, ...spec.defaults, ...props }), bindings, events: {},
    ...(parentId ? { parentId, slotId } : {}),
    ...(spec.container ? { slots: structuredClone(spec.slots || [{ id: 'content', title: '内容' }]), layout: { type: 'absolute', padding: 0, gap: 0 } } : {}) }
  page.widgets.push(value)
  return value
}
const label = (id, text, x, y, width, props = {}, parentId, slotId) => widget(id, 'text', x, y, width, 28, { text, align: 'left', ...props }, {}, parentId, slotId)
const button = (id, text, x, y, width, icon, parentId, slotId) => widget(id, 'button', x, y, width, 34, { text, icon }, {}, parentId, slotId)
function flow(id, name) {
  const value = { id, name, enabled: true, timeoutMs: 30000, nodes: [], edges: [] }
  project.flows.push(value)
  return value
}
function node(flow, kind, config = {}, label = kind) {
  const id = `${flow.id}_n${flow.nodes.length}`
  flow.nodes.push({ id, kind, label, position: { x: 60 + (flow.nodes.length % 6) * 250, y: 60 + Math.floor(flow.nodes.length / 6) * 140 }, config })
  return id
}
function link(flow, source, target, port = 'out') { flow.edges.push({ id: `${flow.id}_e${flow.edges.length}`, source, sourcePort: port, target, targetPort: 'in' }); return target }
function invoke(flow, operation, args = {}) { return node(flow, 'backend.invoke', { backendId: 'ptb', operation, args }, operation) }
function ui(flow, widgetId, property, value) { return node(flow, 'ui.setProperty', { widgetId, property, value }, `设置 ${widgetId}.${property}`) }
function trigger(flow) { return node(flow, 'trigger.ui', {}, '界面事件') }
function action(widget, event, flow, entry = flow.nodes[0].id) { widget.events[event] = { kind: 'flow', flowId: flow.id, entryNodeId: entry } }
function failures(flow, status = page.id === 'main' ? 'main_error' : 'settings_error') {
  const calls = flow.nodes.filter(item => ['backend.invoke', 'file.open', 'file.save', 'window.open'].includes(item.kind))
  if (!calls.length) return
  const error = ui(flow, status, 'text', message('error.message'))
  for (const call of calls) if (!flow.edges.some(edge => edge.source === call.id && edge.sourcePort === 'error')) link(flow, call.id, error, 'error')
}
function simple(widget, event, operation, args = {}, suffix = event) {
  const f = flow(`flow_${widget.id}_${suffix}`, `${widget.name} · ${event}`)
  link(f, trigger(f), invoke(f, operation, args)); failures(f); action(widget, event, f); return f
}
function show(widget, event, target, before = []) {
  const f = flow(`flow_${widget.id}_${event}`, `${widget.name} · 打开预览`)
  let current = trigger(f)
  for (const item of before) current = link(f, current, invoke(f, item.operation, item.args))
  current = link(f, current, ui(f, target, 'visible', literal(true)))
  link(f, current, ui(f, target, 'open', literal(true)))
  failures(f); action(widget, event, f); return f
}
function close(widget, event, target) {
  const f = flow(`flow_${widget.id}_${event}`, `${widget.name} · 关闭`)
  link(f, link(f, trigger(f), ui(f, target, 'open', literal(false))), ui(f, target, 'visible', literal(false)))
  action(widget, event, f)
}
function exportFile(widget, operation, args = {}) {
  const f = flow(`flow_${widget.id}`, widget.name)
  const call = link(f, trigger(f), invoke(f, operation, args))
  link(f, call, node(f, 'file.save', { args: { name: message('payload.name'), content: message('payload.content'), resources: message('payload.resources') } }, '保存文件及固件附件'))
  failures(f); action(widget, 'click', f)
}

label('main_title', '设备状态总览', 16, 14, 900, { fontSize: 20 })
const metrics = [['total','总设备数','#0f6cbd'],['idle','空闲','#6f55a4'],['burning','烧录中','#b36b00'],['rfTesting','RF 测试中','#6f55a4'],['success','成功','#107c10'],['failure','失败','#c50f1f']]
metrics.forEach(([key, text, color], index) => widget(`metric_${key}`, 'number', 16 + index * 164, 50, 156, 68,
  { text, precision: 0, background: '#ffffff', borderColor: '#e2e8f0', borderWidth: 1, borderRadius: 6, padding: 8, fontSize: 12, color }, { value: backend(`stats.${key}`) }))
// Headless package smoke entry; the reference window has no batch toolbar.
{
  const f = flow('flow_scan', '扫描设备（包验证入口）')
  link(f, trigger(f), invoke(f, 'devices.scan')); failures(f)
}
widget('device_panel', 'frame', 16, 130, 976, 550, { text: '', borderWidth: 1, borderColor: '#e2e8f0', borderRadius: 6 })
label('device_list_title', '设备列表', 30, 144, 140, { fontSize: 15 })
label('device_selected_label', '已激活', 174, 146, 54, { fontSize: 12, color: '#64748b' })
widget('device_selected_count', 'text', 226, 146, 34, 24, { fontSize: 12, color: '#0f6cbd' }, { text: backend('stats.selected') })
label('device_selected_unit', '台设备', 262, 146, 80, { fontSize: 12, color: '#64748b' })
const deviceTable = widget('devices', 'table', 26, 180, 956, 488,
  { text: '', rowKey: 'deviceKey', selectable: false, sortable: true, background: '#ffffff', borderWidth: 0,
    columns: [
      { key: 'selected', title: '激活', type: 'boolean', editable: true, enabledField: 'canActivate', width: 42 },
      { key: 'deviceId', title: '设备 ID', type: 'number', width: 56 },
      { key: 'location', title: '位置', width: 160 }, { key: 'version', title: '版本', width: 56 },
      { key: 'targetChip', title: '目标芯片', width: 76 },
      { key: 'stateText', title: '当前状态', type: 'badge', badges: { '未连接': { color: '#64748b' }, '连接中': { color: '#64748b' }, '就绪': { color: '#6f55a4' }, '已配置': { color: '#007f7a' }, '下发中': { color: '#0f6cbd' }, '启动中': { color: '#0f6cbd' }, '烧录中': { color: '#0f6cbd' }, '失败': { color: '#c50f1f' }, '成功': { color: '#107c10' }, 'ID冲突': { color: '#c50f1f' }, '需复位': { color: '#c50f1f' }, '已取消': { color: '#64748b' }, '监控已停止': { color: '#b36b00' } } },
      { key: 'passed', title: '成功次数', type: 'number', width: 66 }, { key: 'failed', title: '失败次数', type: 'number', width: 66 },
      { key: 'lastActiveTime', title: '最后活跃时间', width: 130 },
      { key: 'actions', title: '操作', type: 'actions', width: 110, actions: [{ id: 'start', label: '', tooltip: '启动任务', icon: 'Play', visibleField: 'showStartAction', enabledField: 'canActivate' }, { id: 'stop', label: '', tooltip: '停止主动查询', icon: 'SquareStop', visibleField: 'showStopAction' }, { id: 'settings', label: '', tooltip: '设备设置', icon: 'Settings' }] },
    ] }, { value: backend('devices') })
simple(deviceTable, 'cellEdit', 'devices.select', { deviceKey: message('payload.rowKey'), selected: message('payload.newValue') })
{
  const f = flow('flow_device_actions', '设备行操作分派')
  let previous = trigger(f)
  for (const [actionId, operation] of [['settings', null], ['start', 'task.start'], ['stop', 'task.stop']]) {
    const condition = node(f, 'condition', { left: message('payload.actionId'), operator: 'eq', right: literal(actionId) }, `操作 = ${actionId}`)
    link(f, previous, condition, previous === f.nodes[0].id ? 'out' : 'false')
    const target = operation ? invoke(f, operation, { deviceKey: message('payload.rowKey') })
      : node(f, 'window.open', { windowId: 'device_settings', args: { deviceKey: message('payload.rowKey'), backendId: literal('ptb') } }, '打开此设备设置窗口')
    link(f, condition, target, 'true'); previous = condition
  }
  failures(f); action(deviceTable, 'rowAction', f)
}
widget('log_panel', 'frame', 1004, 14, 462, 666, { text: '', borderWidth: 1, borderColor: '#e2e8f0', borderRadius: 6 })
label('logs_title', '操作日志', 1018, 27, 160, { fontSize: 15 })
const realtimeLogs = widget('realtime_logs', 'checkbox', 1180, 26, 114, 28, { text: '实时导出', value: true, fontSize: 12 }, { value: backend('autoExportLogs'), tooltip: backend('logDirectory') })
simple(realtimeLogs, 'change', 'logs.setAutoExport', { enabled: message('payload') })
widget('business_logs', 'log', 1014, 62, 442, 606, { text: '', source: 'entries', maxLines: 1000, autoScroll: true, wrap: false, borderWidth: 0, padding: 4, fontSize: 11, fontFamily: 'Consolas' }, { entries: backend('logs') })
const clearLog = button('clear_logs', '清空', 1382, 20, 72, 'Trash2')
simple(clearLog, 'click', 'logs.clear')
const saveLog = button('save_logs', '导出', 1302, 20, 72, 'Save')
exportFile(saveLog, 'logs.export')
widget('main_status', 'statusbar', 0, 692, 1480, 28, { text: '● 就绪', status: '版本 1.0.0', fontSize: 12, color: '#64748b', padding: 8 })
label('main_error', '', 110, 694, 1210, { color: '#c50f1f', fontSize: 12 })

page = project.pages[1]
label('settings_title', '设备设置', 20, 16, 600, { fontSize: 22 })
widget('settings_device_key', 'text', 440, 20, 438, 28, { color: '#64748b', align: 'right', fontSize: 12 }, { text: context('deviceKey') })
const tabs = widget('settings_tabs', 'tabs', 20, 60, 860, 488, { tabPlacement: 'left', tabWidth: 120, activeSlot: 'basic', background: '#ffffff', borderColor: '#e2e8f0' })
tabs.slots = [{ id: 'id', title: '设备 ID' }, { id: 'basic', title: '基础配置' }, { id: 'rf', title: 'RF 测试' }, { id: 'firmware', title: '产品固件' }, { id: 'rolling', title: '滚码' }, { id: 'dut', title: '测试固件' }]
for (const slot of tabs.slots) widget(`panel_${slot.id}`, 'scroll', 0, 0, 730, 480, { text: '', borderWidth: 0, background: '#ffffff' }, {}, tabs.id, slot.id)
function setting(slot, index, id, title, description, kind, field, props = {}) {
  const y = 12 + index * 66
  label(`${id}_label`, title, 16, y, 370, { fontSize: 13 }, `panel_${slot}`)
  label(`${id}_help`, description, 16, y + 26, 390, { fontSize: 11, color: '#64748b' }, `panel_${slot}`)
  const control = widget(id, kind, 432, y + 6, 260, 42, { text: '', ...props }, { value: device(`draft.${field}`) }, `panel_${slot}`)
  simple(control, 'change', 'device.settings.patch', { deviceKey: context('deviceKey'), path: literal(field), value: message('payload') })
  return control
}
const chips = [{ value: 1, label: 'ING916' }, { value: 2, label: 'ING920 / ING20' }]
const options = labels => labels.map((label, value) => ({ value, label }))
label('device_id_title', '设备 ID', 16, 20, 350, {}, 'panel_id')
label('device_id_help', '取值范围 1–255；不能与在线设备重复。', 16, 54, 550, { color: '#64748b', fontSize: 12 }, 'panel_id')
widget('device_id_input', 'spinbox', 450, 20, 240, 36, { text: '', value: 1, min: 1, max: 255, step: 1 }, { value: device('deviceId') }, 'panel_id')
const saveId = button('save_device_id', '保存设备 ID', 450, 84, 150, 'Save', 'panel_id')
simple(saveId, 'click', 'device.setId', { deviceKey: context('deviceKey'), deviceId: input('device_id_input') })
widget('duplicate_id', 'indicator', 16, 130, 650, 30, { text: 'ID 冲突状态', onColor: '#c50f1f', offColor: '#107c10' }, { value: device('duplicateId') }, 'panel_id')
label('readback_title', '最近读取的设备配置', 16, 188, 650, { fontSize: 13 }, 'panel_id')
widget('readback_value', 'text', 16, 224, 680, 220, { text: '点击窗口底部“读取设备配置”获取设备返回值。', fontSize: 12, fontFamily: 'Consolas' }, { text: device('readConfiguration') }, 'panel_id')
const resumeQuery = button('resume_query', '继续查询结果', 16, 454, 160, 'RefreshCcw', 'panel_id')
resumeQuery.bindings.enabled = device('hardwareRunning')
simple(resumeQuery, 'click', 'task.readResult', { deviceKey: context('deviceKey') })
const recoverDevice = button('recover_device', '复位后恢复连接…', 194, 454, 190, 'Plug', 'panel_id')
show(recoverDevice, 'click', 'recovery_dialog')
label('recovery_help', '停止仅停止查询。结果未知时须实际复位设备，再恢复连接。', 16, 496, 680, { color: '#64748b', fontSize: 12 }, 'panel_id')
setting('basic', 0, 'target_chip', '目标芯片', '选择当前设备需要烧录的芯片系列。', 'select', 'targetChip', { options: chips })
setting('basic', 1, 'burn_mode', '烧录方式', '当前 USB PTB 使用 SWD 接口烧录。', 'select', 'burnMode', { options: [{ value: 1, label: 'SWD' }] })
setting('basic', 2, 'clear_lock', '开始前清除芯片锁', '清除芯片锁会擦除 Flash 全部内容。', 'switch', 'clearLock')
setting('basic', 3, 'lock_enabled', '芯片锁（读保护）', '烧录完成后启用芯片读保护。', 'switch', 'lockEnabled')
setting('basic', 4, 'tvcc', 'TVCC 电压', '目标板供电电压，范围 1.8–3.3 V。', 'spinbox', 'tvccVoltage', { min: 1.8, max: 3.3, step: .1 })
setting('basic', 5, 'burn_count', '烧录次数', '0 表示不限制累计烧录次数。', 'spinbox', 'burnCount', { min: 0, max: 4294967295, step: 1 })
setting('basic', 6, 'post_action', '烧录后动作', '烧录完成后保持停止，或复位并运行。', 'select', 'postAction', { options: options(['保持 Halt', '复位并运行']) })
setting('rf', 0, 'rf_enabled', 'RF 测试', '在任务中执行 RF 收发测试。', 'switch', 'rf.enabled')
setting('rf', 1, 'rf_frequency', '频点', '范围 2402–2480 MHz，步进 2 MHz。', 'select', 'rf.frequencyMhz', { options: Array.from({ length: 40 }, (_, i) => ({ value: 2402 + i * 2, label: `${2402 + i * 2} MHz` })) })
for (const [index, id, title, field, min, max, description] of [
  [2,'rf_packets','总包数','packetCount',1,65535,'单次测试发送和接收的总包数。'],
  [3,'rf_tx','TX Power 阈值','txPower',-128,127,'TX 功率合格阈值，单位 dBm。'],
  [4,'rf_rx','RX 灵敏度阈值','rxPower',-128,127,'RX 灵敏度合格阈值，单位 dBm。'],
  [5,'rf_tx_pass','TX 通过包数','txPassCount',0,65535,'TX 判定通过所需的最少成功包数。'],
  [6,'rf_rx_pass','RX 通过包数','rxPassCount',0,65535,'RX 判定通过所需的最少成功包数。'],
  [7,'rf_delay','设备间延迟','delayMilliseconds',0,65535,'连续设备测试之间的等待时间，单位 ms。'],
]) setting('rf', index, id, title, description, 'spinbox', `rf.${field}`, { min, max, step: 1 })
setting('rf', 8, 'rf_address', '频偏值烧录地址', '支持十进制或以 0x 开头的十六进制地址。', 'input', 'rf.frequencyOffsetAddress', { format: 'address', commitMode: 'change', placeholder: '0x0207F000' })
setting('firmware', 0, 'firmware_function', '产品固件烧录', '关闭产品固件操作、烧录固件或校验已有固件。', 'select', 'firmwareFunction', { options: options(['关闭', '烧录', '校验']) })
setting('firmware', 1, 'erase_mode', '擦除方式', '烧录时按需擦除或擦除全部 Flash。', 'select', 'eraseMode', { options: options(['按需擦除', '全部擦除']) })
function memory(id, slot, y, binding) {
  label(`${id}_title`, '内存布局预览', 16, y, 650, { fontSize: 13 }, `panel_${slot}`)
  return widget(id, 'memory-map', 16, y + 32, 680, 148, { rangeStart: 0x02000000, rangeSize: 0x80000 }, { segments: device(binding), rangeStart: device('memoryRange.start'), rangeSize: device('memoryRange.size') }, `panel_${slot}`)
}
memory('product_map', 'firmware', 148, 'memorySegments')
const firmwareColumns = [
  { key: 'enabled', title: '启用', type: 'boolean', editable: true, width: 38 },
  { key: 'id', title: '槽位', width: 38 },
  { key: 'file', title: '文件', type: 'file', editable: true, accept: '.bin,.hex,.ihx', allowManualPath: true, width: 230 },
  { key: 'destinationAddress', title: '目标地址', type: 'text', format: 'address', editable: true, width: 110 },
  { key: 'size', title: '大小', type: 'number', width: 70 }, { key: 'crc32', title: 'CRC32', width: 80 },
]
const firmwareTable = widget('firmware_table', 'table', 16, 338, 680, 270, { text: '产品固件 · 最多 5 段', rowKey: 'id', columns: firmwareColumns, selectable: false, sortable: false }, { value: device('firmwareRows') }, 'panel_firmware')
function firmwareEdits(table, isDut = false) {
  const f = flow(`flow_${table.id}_edit`, `${table.name} · 修改文件、地址或启用状态`)
  let previousRow = trigger(f)
  for (let index = 0; index < (isDut ? 1 : 5); index++) {
    const row = node(f, 'condition', { left: message('payload.rowKey'), operator: 'eq', right: literal(String(index)) }, `槽位 ${index + 1}`)
    link(f, previousRow, row, previousRow === f.nodes[0].id ? 'out' : 'false')
    let previousColumn = row
    for (const [column, field] of [['file','resource'], ['destinationAddress','address'], ...(!isDut ? [['enabled','enabled']] : [])]) {
      const test = node(f, 'condition', { left: message('payload.columnKey'), operator: 'eq', right: literal(column) }, `字段 ${column}`)
      link(f, previousColumn, test, previousColumn === row ? 'true' : 'false')
      link(f, test, invoke(f, 'device.settings.patch', { deviceKey: context('deviceKey'), path: literal(`${isDut ? 'dutFirmware' : `firmware.${index}`}.${field}`), value: message('payload.newValue') }), 'true')
      previousColumn = test
    }
    previousRow = row
  }
  failures(f); action(table, 'cellEdit', f)
}
firmwareEdits(firmwareTable)
setting('rolling', 0, 'rolling_enabled', '滚码烧录', '在任务中生成并写入滚码。', 'switch', 'rollingCode.enabled')
setting('rolling', 1, 'rolling_mode', '滚码模式', '可变长、多字节 HEX，支持连续字节位域。', 'select', 'rollingCode.mode', { options: [{ value: 0, label: '可变长多字节 HEX' }] })
setting('rolling', 2, 'rolling_length', '字节长度', '滚码长度范围 4–32 字节。', 'select', 'rollingCode.byteLength', { options: Array.from({ length: 29 }, (_, i) => ({ value: i + 4, label: `${i + 4} 字节` })) })
setting('rolling', 3, 'rolling_step', '步进', '每次烧录后的递增值，范围 1–255。', 'spinbox', 'rollingCode.step', { min: 1, max: 255, step: 1 })
setting('rolling', 4, 'rolling_address', '烧录地址', '支持十进制或以 0x 开头的十六进制地址。', 'input', 'rollingCode.address', { format: 'address', commitMode: 'change', placeholder: '0x0207E000' })
setting('rolling', 5, 'rolling_initial', '开始值', '输入完整十六进制字节，例如 11223344。', 'input', 'rollingCode.initialValueHex', { format: 'hex', commitMode: 'change', placeholder: '0000000000000000', maxLength: 64 })
label('rolling_range_label', '滚动位域 · 连续 Byte', 16, 412, 650, {}, 'panel_rolling')
const range = widget('rolling_range', 'byte-range', 16, 448, 680, 160, { value: { startByte: 0, byteCount: 4 }, byteLength: 8 }, { value: device('rollingRange'), byteLength: device('draft.rollingCode.byteLength') }, 'panel_rolling')
simple(range, 'change', 'device.settings.patch', { deviceKey: context('deviceKey'), path: literal('rollingCode.range'), value: message('payload') })
const rollingPreview = button('rolling_preview_button', '查看 52 条滚码预览', 16, 622, 220, 'Search', 'panel_rolling')
show(rollingPreview, 'click', 'rolling_dialog', [{ operation: 'rolling.preview', args: { deviceKey: context('deviceKey') } }])
setting('dut', 0, 'dut_chip', '测试固件目标芯片', '选择测试固件适用的芯片系列。', 'select', 'dutTargetChip', { options: chips })
memory('dut_map', 'dut', 86, 'dutSegments')
const dutTable = widget('dut_table', 'table', 16, 282, 680, 160, { text: '测试固件', rowKey: 'id', columns: firmwareColumns.filter(column => column.key !== 'enabled'), selectable: false, sortable: false }, { value: device('dutFirmwareRows') }, 'panel_dut')
firmwareEdits(dutTable, true)
label('dut_help', '下发前会校验文件类型、地址格式、长度与 CRC32。', 16, 458, 680, { color: '#64748b', fontSize: 12 }, 'panel_dut')

widget('settings_error', 'text', 20, 550, 860, 15, { text: '', align: 'left', color: '#c50f1f', fontSize: 11, padding: 0 })
widget('settings_device_error', 'text', 20, 566, 860, 15, { text: '', align: 'left', color: '#c50f1f', fontSize: 11, padding: 0 }, { text: device('error'), tooltip: device('error') })
const readConfig = button('read_config', '读取设备配置', 20, 582, 118, 'Download')
simple(readConfig, 'click', 'device.readConfiguration', { deviceKey: context('deviceKey') })
const loadConfig = button('load_config', '加载配置', 146, 582, 104, 'FolderOpen')
{
  const f = flow('flow_load_configuration', '选择配置文件并载入设备草稿')
  const opened = link(f, trigger(f), node(f, 'file.open', { args: { extensions: literal(['ptbcfg']) } }, '选择 .ptbcfg'))
  const check = link(f, opened, node(f, 'condition', { left: message('payload'), operator: 'ne', right: literal(null) }, '已选择文件'))
  const imported = link(f, check, invoke(f, 'configuration.import', { deviceKey: context('deviceKey'), resource: message('payload') }), 'true')
  const valid = link(f, imported, node(f, 'condition', { left: message('payload.valid'), operator: 'eq', right: literal(true) }, '配置附件完整'))
  link(f, valid, ui(f, 'settings_error', 'text', literal('配置已载入草稿；请应用或下发。')), 'true')
  link(f, valid, ui(f, 'settings_error', 'text', message('payload.missingResources')), 'false')
  failures(f); action(loadConfig, 'click', f)
}
const saveConfig = button('save_config', '保存配置', 258, 582, 104, 'Save')
exportFile(saveConfig, 'configuration.export', { deviceKey: context('deviceKey'), useDraft: literal(true) })
const revert = button('revert_config', '撤销草稿', 370, 582, 104, 'RefreshCcw')
simple(revert, 'click', 'device.settings.revert', { deviceKey: context('deviceKey') })
const apply = button('apply_config', '应用', 482, 582, 76, 'Check')
simple(apply, 'click', 'device.settings.apply', { deviceKey: context('deviceKey') })
const configure = button('send_config', '下发配置', 566, 582, 118, 'Download')
configure.props.appearance = 'accent'
{
  const f = flow('flow_send_configuration', '验证草稿、确认地址重叠并下发配置')
  const validated = link(f, trigger(f), invoke(f, 'configuration.validate', { deviceKey: context('deviceKey') }))
  const valid = link(f, validated, node(f, 'condition', { left: message('payload.valid'), operator: 'eq', right: literal(true) }, '配置有效'))
  const applied = link(f, valid, invoke(f, 'device.settings.apply', { deviceKey: context('deviceKey') }), 'true')
  link(f, applied, invoke(f, 'task.configure', { deviceKey: context('deviceKey') }))
  const allowOverlap = link(f, valid, invoke(f, 'configuration.validate', { deviceKey: context('deviceKey'), allowOverlap: literal(true) }), 'false')
  const onlyOverlap = link(f, allowOverlap, node(f, 'condition', { left: message('payload.valid'), operator: 'eq', right: literal(true) }, '仅有地址重叠'))
  link(f, link(f, onlyOverlap, ui(f, 'overlap_dialog', 'visible', literal(true)), 'true'), ui(f, 'overlap_dialog', 'open', literal(true)))
  link(f, onlyOverlap, ui(f, 'settings_error', 'text', message('payload.errors')), 'false')
  failures(f); action(configure, 'click', f)
}
const sendDut = button('send_dut', '下发测试固件', 692, 582, 148, 'Download')
{
  const f = flow('flow_send_dut', '应用草稿并下发测试固件')
  const applied = link(f, trigger(f), invoke(f, 'device.settings.apply', { deviceKey: context('deviceKey') }))
  link(f, applied, invoke(f, 'task.sendDutFirmware', { deviceKey: context('deviceKey') }))
  failures(f); action(sendDut, 'click', f)
}
widget('settings_status', 'statusbar', 20, 624, 860, 26, { text: '草稿修改后请应用或下发配置；关闭窗口不会停止运行中的设备任务。', fontSize: 11, color: '#64748b' }, { status: device('state') })

const rollingDialog = widget('rolling_dialog', 'dialog', 210, 160, 480, 320, { text: '滚码预览 · 52 次', modal: true, open: false, visible: false })
widget('rolling_preview_table', 'table', 12, 8, 450, 230, { rowKey: 'index', selectable: false, sortable: false, columns: [{ key: 'index', title: '次数', width: 45 }, { key: 'hex', title: '滚码（HEX）', width: 320 }] }, { value: device('rollingPreview.rows') }, rollingDialog.id)
const rollingClose = button('rolling_close', '关闭', 350, 242, 100, 'X', rollingDialog.id)
close(rollingClose, 'click', rollingDialog.id); close(rollingDialog, 'close', rollingDialog.id)
const recoveryDialog = widget('recovery_dialog', 'dialog', 210, 190, 480, 240, { text: '确认设备已实际复位', modal: true, open: false, visible: false })
widget('recovery_description', 'text', 16, 12, 440, 112, { text: '请先对对应 PTB 设备执行实际复位，确认原任务已结束。此操作只重新握手，不会替设备执行复位；仅关闭和打开 USB 连接不足以清除传输状态。', fontSize: 14 }, {}, recoveryDialog.id)
const recoveryCancel = button('recovery_cancel', '取消', 190, 144, 110, 'X', recoveryDialog.id)
close(recoveryCancel, 'click', recoveryDialog.id); close(recoveryDialog, 'close', recoveryDialog.id)
const recoveryConfirm = button('recovery_confirm', '已复位，恢复', 312, 144, 140, 'Check', recoveryDialog.id)
{
  const f = flow('flow_confirm_recovery', '确认物理复位后恢复设备会话')
  let current = link(f, trigger(f), ui(f, recoveryDialog.id, 'visible', literal(false)))
  current = link(f, current, ui(f, recoveryDialog.id, 'open', literal(false)))
  link(f, current, invoke(f, 'device.recover', { deviceKey: context('deviceKey'), resetConfirmed: literal(true) }))
  failures(f); action(recoveryConfirm, 'click', f)
}
const overlap = widget('overlap_dialog', 'dialog', 210, 160, 480, 320, { text: '确认地址重叠', modal: true, open: false, visible: false })
widget('overlap_description', 'text', 16, 12, 440, 86, { text: '产品固件分段存在地址重叠。继续下发可能覆盖先前数据。确认固件布局符合预期后再继续。', fontSize: 14, color: '#b36b00' }, {}, overlap.id)
widget('overlap_map', 'memory-map', 16, 98, 440, 112, { rangeStart: 0x02000000, rangeSize: 0x80000 }, { segments: device('memorySegments'), rangeStart: device('memoryRange.start'), rangeSize: device('memoryRange.size') }, overlap.id)
const overlapCancel = button('overlap_cancel', '返回修改', 204, 238, 110, 'X', overlap.id)
close(overlapCancel, 'click', overlap.id); close(overlap, 'close', overlap.id)
const overlapConfirm = button('overlap_confirm', '确认下发', 326, 238, 120, 'Download', overlap.id)
overlapConfirm.props.appearance = 'accent'
{
  const f = flow('flow_confirm_overlap', '确认重叠并应用、下发配置')
  let current = link(f, trigger(f), ui(f, overlap.id, 'visible', literal(false)))
  current = link(f, current, ui(f, overlap.id, 'open', literal(false)))
  current = link(f, current, invoke(f, 'device.settings.apply', { deviceKey: context('deviceKey'), allowOverlap: literal(true) }))
  link(f, current, invoke(f, 'task.configure', { deviceKey: context('deviceKey'), allowOverlap: literal(true) }))
  failures(f); action(overlapConfirm, 'click', f)
}

const directory = path.join(root, 'examples/ing-ptb')
fs.mkdirSync(path.join(directory, 'assets'), { recursive: true })
// Test bytes, deliberately not selected by the default project and not executable firmware.
const sample = Buffer.from(Array.from({ length: 256 }, (_, index) => index))
fs.writeFileSync(path.join(directory, 'assets/simulation-sample.bin'), sample)
function hexRecord(address, type, bytes) {
  const record = [bytes.length, address >> 8, address & 255, type, ...bytes]
  record.push((-record.reduce((sum, byte) => sum + byte, 0)) & 255)
  return ':' + Buffer.from(record).toString('hex').toUpperCase()
}
const hex = [hexRecord(0, 4, [2, 0]), ...Array.from({ length: 4 }, (_, index) => hexRecord(0x2000 + index * 16, 0, [...sample.subarray(index * 16, index * 16 + 16)])), hexRecord(0, 1, [])].join('\n') + '\n'
fs.writeFileSync(path.join(directory, 'assets/simulation-sample.hex'), hex)
const sampleConfiguration = { ...structuredClone(defaults), firmwareFunction: 0, firmware: [{ enabled: true, filePath: 'assets/simulation-sample.bin', destinationAddress: 0x02002000 }] }
fs.writeFileSync(path.join(directory, 'sample.ptbcfg'), JSON.stringify(sampleConfiguration, null, 2) + '\n')
project.assets = [{ id: 'sample_bin', path: 'assets/simulation-sample.bin' }, { id: 'sample_hex', path: 'assets/simulation-sample.hex' }]
fs.writeFileSync(path.join(directory, 'project.json'), JSON.stringify(project, null, 2) + '\n')
console.log(`Generated ${project.pages.length} pages, ${project.pages.reduce((count, page) => count + page.widgets.length, 0)} editable widgets and ${project.flows.length} flows.`)
