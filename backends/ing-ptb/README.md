# ING PTB backend provider

独立 C# .NET 10 NativeAOT Windows x64 进程，无 Avalonia、NuGet UI、系统 .NET 或 Python 运行时依赖。协议基线来自只读参考工程当前 GUI；来源及 SHA256 见 `source-provenance.json`。PTB 真实固件存在不同历史协议，必须按当前 GUI 三阶段协议完成硬件验收。

## 构建与验证

```powershell
dotnet run --project backends/ing-ptb/tests/IngPtb.Backend.Tests.csproj -c Release
./backends/ing-ptb/build.ps1 -OutputDirectory ./backends/ing-ptb/publish
node backends/ing-ptb/tests/rpc-smoke.mjs backends/ing-ptb/publish/ing-ptb-backend.exe
```

需要 .NET 10 SDK、Windows SDK 和 MSVC NativeAOT 链接工具。产物包括 `manifest.json`、`ing-ptb-backend.exe` 以及配置/来源说明。构建输出与本地测试缓存均忽略。

## 主机协议

stdin/stdout 使用 UTF-8、JSON-RPC 2.0、一行一个对象；每行最多 1 MiB。stdout 仅协议消息，异常诊断使用 stderr。RPC 必须带 `id`。

- `initialize`: `{mode:"simulation"|"usb", config, resources}`，直接返回 snapshot。也接受 `real` 作为 `usb` 别名。
- `invoke`: `{operation,args,context?,resources?}`。context 可带 `deviceKey`；args 的显式值优先。完整主机资源表可在每次调用附带。
- `snapshot`: 返回完整状态。`generation` 区分进程，`sequence` 单调递增。`devices` 和 `byDevice[deviceKey]` 内容相同。
- `shutdown`: 停止接收任务、取消后续发送、等待已发事务完成或超时、关闭设备。RPC 成功响应后退出。

通知为 `{jsonrpc:"2.0",method:"event",params:{kind:"snapshot",generation,sequence,deviceKey?,jobId?,data:<snapshot>}}`。stdout 背压时可丢弃冗余通知，主机可调用 snapshot 重同步。日志内存保留1000条、任务保留64条、设备最多16台，资源最多256项；每台配置最多8 KiB。消息按JSON转义后256字节预算裁剪，保证1000条手动导出低于1 MiB。若设备状态与日志合计超过900 KiB，snapshot仅减少最旧的显示日志并标记logTruncated/logTotal；业务ring和手动导出仍保留完整1000条。

初始化 config 支持 `simulatedDeviceCount`（1..16，默认2）、`defaultSettings`、`deviceSettings:{deviceKey:settings}`、`autoExportLogs`（默认true）。模拟与USB均自动连接/读取C4 ID/C5版本。USB每650ms枚举检测热插拔，仅拓扑改变时推送快照；当前使用轮询，没有接入WM_DEVICECHANGE窗口通知。模拟设备键为 `sim_1` 等；USB设备键根据接口路径SHA256生成，ID可修改而键保持稳定。

资源描述为 `{assetId:{path:<主机解析后的绝对文件>,name,size,sha256}}`，引用为ID字符串或 `{id,name,size,sha256}`。固件只从主机注册资产读取，大小不超过16 MiB，拒绝重解析点；每次实际使用重新验证hash。前端不能通过普通operation注册任意路径。固件数据不经过JSON-RPC/base64。

## 操作

设备操作默认使用 `args.deviceKey`，否则窗口 context.deviceKey。任务也接受 `deviceKeys:string[]`；都缺省时使用已选择且在线设备。长任务立即返回 `{accepted:true,jobId}`；状态通过 jobs/device 快照报告。单设备同一时刻只运行一个任务，C3/C7全传输期间占用设备。

| 操作 | 参数 / 行为 |
| --- | --- |
| devices.scan | 重新枚举设备，异步任务 |
| devices.select | `{deviceKeys:[...]}`整体选择，或`{deviceKey,selected}` |
| device.connect / device.disconnect | 建立或关闭设备会话，异步任务 |
| device.recover | `{deviceKey,resetConfirmed:true}`；用户已物理复位后重新握手 |
| device.setId | `{deviceKey,deviceId:1..255}`；拒绝已占用ID，异步C4 |
| device.readConfiguration | 异步C6，结果写`readConfiguration`；默认将头字段载入draft，保留本地固件资产和DUT页。`applyToDraft:false`可仅查询；需Apply后保存 |
| device.settings.get | 返回`{settings,draft,dirty}` |
| device.settings.set | `{deviceKey,settings:<深合并补丁>}`，仅编辑draft |
| device.settings.patch | `{deviceKey,path:"rf.frequencyMhz",value:2404}`，仅编辑draft；数字字段接受数值或数字字符串 |
| device.settings.apply / revert | 校验并保存draft / 放弃draft |
| configuration.validate | `{deviceKey,settings?,allowOverlap?}` → `{valid,errors,packageBytes?}` |
| configuration.import | `{deviceKey,resource:<已导入ptbcfg>,references?}`，也可用`document`；references也可位于resource.references；返回`{valid,settings,missingResources,includesDutSettings}`并更新草稿 |
| configuration.export | `{deviceKey,useDraft?:true,name?}` → `{name,content:<JSON字符串>,document,resources:[{id,relativePath,...}]}` |
| firmware.inspect | `{resource:<引用>}` → `{resource,segments:[{assetId,name,destinationAddress,sourceAddress,size,crc32}]}` |
| rolling.preview | `{deviceKey,settings?}` → `{rows,count:52}`，第1项为初始值，共52项 |
| task.configure | 已保存settings的不可变快照经C3下发；可传`allowOverlap:true` |
| task.sendDutFirmware | 已保存DUT固件与地址经C7下发 |
| task.start | C1启动后C2轮询（500ms，最多300s），返回结构化RF/固件/滚码/锁结果 |
| task.readResult | 继续C2监控，不重复C1 |
| task.stop | 停止所选任务的后续发送/监控，物理烧录不保证停止 |
| jobs.cancel | `{jobId}`取消后续发送；已发出的事务仍收尾 |
| logs.clear / logs.export | 清空内存日志 / 导出当前保留日志的`{name,content,resources:[]}` |
| logs.setAutoExport | `{enabled:boolean}`；返回开关、目录及写入错误 |

`firmware.{0..4}.resource`和`dutFirmware.resource`可用资源引用增量设置；BIN自动取所选芯片默认地址，HEX/IHX自动拆连续段，超5槽拒绝。产品槽位还支持enabled、address/destinationAddress；DUT地址用`dutFirmware.address`或`dutFirmwareAddress`。`rollingCode.range`接受零起点`{startByte,byteCount}`并生成连续bitmask。

snapshot设备派生字段包括固定5行firmwareRows、memorySegments、dutFirmwareRows、dutSegments、memoryRange、rollingRange和rollingPreview，便于通用控件直接绑定；错误位于对应row.error或rollingPreview.error。settings/draft分别保存已生效值和未保存值，任务不读取正在编辑的草稿。

`.ptbcfg` v2/v3/v4导入兼容大小写，统一保存v4。v2不覆盖DUT页。旧路径必须映射到已注册资源；缺失时valid=false并明确列出，不能视为成功。host负责基于原文件目录导入伴随固件。导出返回相对`firmware/<assetId>.<ext>`，由host复制资源。原GUI不支持多连续段HEX/IHX配置重载，因此这种导出明确拒绝，须改用分段BIN；BIN/单连续段HEX可往返。

## 设备事务与隔离

C1/C3/C7不自动重发。调用方取消只阻止新事务，已经发出的请求独立等待ACK或自身超时；普通请求1–2秒、传输每帧5秒。C3/C7半途结束、超时或格式错误均隔离设备并要求物理复位，不能靠重开USB句柄续传。C1监控停止后保留hardwareRunning，需C2确认结果或复位后再开始任务。

USB使用Windows全局命名Mutex（专用线程持有）避免多个provider交错访问；跨进程未知写操作在LocalAppData恢复标记中留痕，进程崩溃/重启后仍要求确认物理复位。该互斥只约束本provider，外部原GUI等软件需由操作者避免同时访问。USB Printer独立同步句柄保留整64字节写后的ZLP。主机最终进程回收仍用于不可取消的驱动调用；未验证真实USB驱动响应延迟。

每日自动日志位于 `%LOCALAPPDATA%/HmiDesignStudio/ing-ptb/logs/yyyy-MM-dd/ptb-<实例>.jsonl`；默认启用，避免写应用安装目录。snapshot暴露autoExportLogs、logDirectory和logExportError。六统计沿用GUI语义：total/idle/burning/rfTesting/success/failure；rfTesting固定0，因为当前协议没有独立RF阶段通知。passed/failed另外记录累计结果。

已建立模拟协议及RPC检查；真实USB Printer、真实烧录器固件、拔插恢复、ZLP驱动行为、Windows10与干净机器仍需硬件验收，不得用模拟通过替代。
