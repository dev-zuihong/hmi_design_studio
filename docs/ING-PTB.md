# ING PTB 工程与运行说明

本示例以 `D:\zuihong.yu\ING_PTB\07-gui` 当前 C# 源码为行为及协议基准；源文件摘要见后端的 `source-provenance.json`。本次交付目标为 Windows 10/11 x64。使用 Vue/Fluent 通用控件和工程流程定义页面，独立 .NET 10 NativeAOT 提供程序处理 PTB USB Printer 通信，目标机器不需要安装 .NET 或 Python。

## 工程与构建

- `examples/ing-ptb/project.json`：可编辑工程源，通过 `node scripts/generate-ptb-demo.mjs` 重建。
- `backends/ing-ptb`：后端源码、协议与配置提取代码、模拟器、清单和测试。
- `scripts/build-ptb-demo.ps1`：生成示例、发布后端、构建 Studio/Runner，调用同一 CLI 校验、模拟 smoke、工程打包和运行包导出。`-SkipBuild` 仅复用已经构建的产物，不代表重新验证了源码对应的二进制。
- 产物目录包含 `ING-PTB.hmiproj`、`ING-PTB-win-x64.zip`、`ing-ptb-backend-source.zip` 和实际执行的 JSON 报告。工程包携带所需后端及资源摘要；运行包携带 Runner、后端与固定 WebView2；源码包包含提供程序、协议、测试及构建说明。

在设计器打开 `.hmiproj` 后，可以编辑主窗口及设备设置页面、拖拽组件、修改绑定和流程。设备连接资源中可管理后端实例、窗口模板和工程文件。运行时选择的文件仅影响本次运行快照；需要随工程发布的初始固件通过“导入工程文件”加入工程资源。

## 操作与兼容

默认是明确标识的模拟模式。Runner 停止后在“连接设置”中选择模拟或真实 USB Printer，再启动新会话；连接失败报告错误，不回退为模拟数据。真实设备 VID/PID 为 `36B0:5320`，主机连接不是 COM 串口；设置里的 SWD/UART 是 PTB 到 DUT 的接口。

主窗口保留统计、设备表格、逐行开始/停止/设置、设备日志及状态。每台设备独立设置窗口，同设备再次打开聚焦现有实例，不同设备草稿分离；设备移除关闭相应窗口。设置分为设备 ID、基本、RF、产品固件、滚码、测试固件六页。更改先进入草稿，应用后成为后续设备任务的不可变设置快照。

产品固件最多五个槽位，支持 BIN/HEX/IHX、地址与 CRC32、内存占用和重叠确认；测试固件单独走 C7。滚码采用 4–32 字节和连续字节区间，起始值用十六进制字符串保存，预览 52 项。配置读取 v2/v3/v4，保存 v4；C6 将回读头字段载入草稿，不覆盖本地固件资源列表和 DUT 设置，应用后保存。`.ptbcfg` 仍使用外部路径：导出配置时将已注册固件复制为相邻资源；导入从原配置目录解析伴随文件，缺失必须补齐后才能应用和下发。多连续段 HEX/IHX 可下发，但原 GUI 无法按原配置路径完整重载，此类配置导出会明确拒绝，需先改用分段 BIN。

C1 表示启动受理，C2 每 500 ms 查询，等待上限 300 s。停止只停止结果查询，不能宣称物理设备已经停止。C1/C3/C7 不自动重发；取消收尾已发请求后停止后续分片。超时或不能确认 ACK 归属时报告结果未知并隔离会话，设备实际复位且恢复被确认后才能再次下发。USB close/open 不能当作设备协议状态复位。

设备 ID 页提供“继续查询结果”（仅 C2）和“复位后恢复连接”；恢复前的确认框要求操作者已实际复位设备。USB 每 650 ms 枚举拓扑变化，当前尚未使用 WM_DEVICECHANGE。业务日志保留最近 1000 条，默认按天自动导出到当前用户数据目录；单条日志有长度限制，状态快照超过消息预算时减少最旧显示日志并暴露截断标记，手动导出保留内存中的完整日志。

## 组件与公共接口

新增 `file-picker`、`byte-range`、`memory-map`；扩展表格动作/状态/文件列、左侧页签、结构化日志、输入校验和进度。组件渲染、绑定和事件在预览与独立运行中共用。

- `file-picker` 值为 `{id,name,size,sha256,path?}`；选择、手动输入和清除产生 `{value,reason}`，取消不改变值。文件经宿主导入不可变快照，流程不承载固件字节。
- `byte-range` 值为 `{startByte,byteCount}`，按字节索引选择连续范围；协议掩码由后端生成。
- `memory-map` 使用 `rangeStart/rangeSize/segments`，片段为 `{id,label,start,size,color,enabled}`，区间为半开区间，只读显示占用、重叠和越界。
- 表格使用稳定 `rowKey`；`rowAction` 携带 `{rowKey,actionId,row}`，`cellEdit/fileChange` 携带列名、新旧值及操作原因，排序不改变设备身份。
- 日志绑定 `{id,timestamp,deviceKey,level,message}` 条目；保留原运行事件模式，业务视图和磁盘输出分别由组件与后端/宿主负责。

项目 schema 3 增加 `backends`、`windows`，仍读取 v1/v2。`backend.invoke` 调用提供程序清单声明的操作，`trigger.backend` 接收事件；值来源增加 `backend:{key,path}` 和 `context:{path}`。后端路径中的 `$deviceKey` 从当前窗口上下文取值，作为单一字段名解析，不执行表达式。`window.open/focus/close` 操作项目窗口模板；`file.open/save` 使用宿主文件服务。

后端 JSON-RPC 通过独立进程的标准输入/输出传输，每行上限 1 MiB。方法包括 `initialize/invoke/snapshot/shutdown`，事件为 `event` 通知，带 generation、sequence、deviceKey、jobId。长事务迅速返回 jobId，随后发布进度/结果。Rust 管理进程树、会话代次与资源快照；同设备事务排他，不同设备并行。

普通 UI 修改继续热更新；后端/资源变化进入收尾屏障。窗口上下文、输入和 UI 属性按实例隔离，旧代次窗口操作被拒绝。后端 snapshot 仅保留最新状态，不将每次携带日志的完整快照重复写入运行历史。

## 验收边界

模拟器、协议向量、组件测试、原生窗口测试和运行包测试分别记录在 [VERIFICATION.md](VERIFICATION.md)。模拟成功不等于真实 USB、ZLP、实际烧录、Windows 10 或干净机器已验收。真实设备写入应使用确认匹配当前 GUI 协议的固件与目标芯片配置。
