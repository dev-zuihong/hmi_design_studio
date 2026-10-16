# 协议插件与温控示例

`demo-temperature` 是独立进程协议插件，版本 `1.0.0`，API 版本 `1`。它既支持 `sim://temperature` 明示模拟连接，也支持真实串口字节流。模拟不能验证物理串口、电气接线或目标硬件。

构建 `cargo build -p demo-protocol --release` 后，将当前平台的 `demo-protocol`（Windows 为 `demo-protocol.exe`）与 [package/manifest.json](package/manifest.json) 放在同一个 `demo-temperature/` 目录。设计器可导入这个目录；发布程序使用 `plugins/demo-temperature/manifest.json` 定位插件。清单中的 `target` 必须与原生可执行文件的 Rust target triple 一致：Windows x64 为 `x86_64-pc-windows-msvc`，Apple Silicon 为 `aarch64-apple-darwin`，Intel Mac 为 `x86_64-apple-darwin`。暂存脚本会根据本机构建目标重写示例清单；API 和项目锁定版本仍须匹配。

## 宿主与插件接口

标准输入/输出每行一个 UTF-8 JSON-RPC 2.0 对象，以 LF 结束，单行最多 1 MiB。标准输出只写协议消息，诊断日志写标准错误。宿主请求 ID 是整数，插件发起请求可使用字符串 ID。插件必须持续读输入，同时等待设备响应；不能在处理命令时阻塞后续 `transport.data`。示例使用异步事件循环和带截止时间的待处理事务。

| 方向 | 方法 | 参数 / 结果 |
| --- | --- | --- |
| 宿主 → 插件 | `initialize` | 参数 `{apiVersion:1, protocolId, config, simulation}`；结果至少 `{apiVersion:1}` |
| 宿主 → 插件 | `command.execute` | 参数 `{command, params, timeoutMs, originFlowId?}`；结果作为流程消息 `payload` |
| 宿主 → 插件 | `transport.data` | 通知 `{bytes: base64}`，是任意长度的串口读取块，并非完整帧 |
| 插件 → 宿主 | `transport.write` | 请求 `{bytes: base64}`，最多 64 KiB；宿主回复 `{written: 字节数}` 或串口写错误 |
| 插件 → 宿主 | `point.publish` | 通知 `{key, value, originFlowId?}`；键及值类型须在清单 `points` 中声明 |
| 插件 → 宿主 | `log` | 通知日志对象，例如 `{level:"info", message:"..."}` |

错误格式为 `{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Device timeout","data":{"code":"DEVICE_TIMEOUT"}}}`。`data.code` 是供节点错误出口使用的稳定错误码。宿主关闭连接会终止插件进程并释放串口；协议插件不能依赖优雅退出 RPC 完成必要的设备动作。

每个连接由宿主串行执行命令，等待队列上限 64。停止流程会取消未发出的请求；已发出的请求仍排空至结果或截止时间。命令不会自动重试。插件退出、插件 RPC 超时或串口写失败将使连接进入故障，用户须关闭后重开。

`initialize` 结果可额外声明 `lateResponsesDiscarded:true`，**仅限协议能可靠关联事务并丢弃晚到响应时**。示例线协议使用不复用的相关 ID，符合这一条件。若不声明，设备超时会使连接故障，避免把上一条应答误认成下一条的结果。

由命令产生的数据点应原样回传该命令的 `originFlowId`。运行内核会更新点位与界面，但抑制同一流程的直接数据回授触发。无因果来源的设备主动上报省略此字段。其他流程仍可响应变化；跨流程回授依靠每流程每秒 10 次自动触发上限暂停并提示恢复。重复相同值不会再次触发。

## 温控命令与配置

| 命令 | 参数 | 结果 |
| --- | --- | --- |
| `read` | `{}` | `{temperature, rawTemperature, target}` |
| `setTarget` | `{target: number}`，范围 -40 至 200 °C | 同上，返回新的目标温度 |

`temperature`、`target` 为摄氏度；`rawTemperature` 为十分之一摄氏度的整数。每个成功命令发布这三个同名数据点。Python 节点可用 `msg["payload"]["rawTemperature"] / 10` 完成换算。未知参数和类型不匹配会明确返回 `PARAMETERS`。

`protocolConfig` 支持 `timeoutMs`（10–60000，默认 2000）与 `simulationDelayMs`（0–30000，默认 30）。后者仅影响模拟事务延迟，可用来测试超时和排队。模拟温度从 23.5 °C 开始，每次读取向目标 25 °C 接近；真实串口连接不会生成这些模拟值。

宿主在打开连接前校验 `configSchema`，发送任何设备命令前校验已解析引用的参数；不合法的参数不会进入协议进程。启动及导出预检也检查连接配置、常量参数和动态参数的必填字段/名称，动态值的类型及范围在运行时校验。支持的 JSON Schema 子集为：`type`（包括类型数组）、`properties`、`required`、`additionalProperties`、`enum`、`const`、`minimum`、`maximum`、`exclusiveMinimum`、`exclusiveMaximum`、`minLength`、`maxLength`、`items`、`minItems`、`maxItems`。支持布尔 schema，省略 schema 表示不限制。

`title`、`description`、`default`、`examples`、`$schema`、`$comment`、`readOnly`、`writeOnly`、`deprecated` 仅作元数据。首版不支持 `$ref`、`oneOf`、`anyOf`、`allOf`、`pattern`、`format` 等其他关键字；导入时明确拒绝未知关键字，避免忽略约束后发送无效设备参数。Rust 可复用 `hmi_core::protocol::validate_schema`，返回的错误包含数据字段路径。

## 线协议与参考设备

每帧为：`UTF-8 JSON 字节 + TAB + 四位大写十六进制 CRC + LF`。CRC 使用 CRC-16/MODBUS，初值 `0xFFFF`，反射多项式 `0xA001`，覆盖 JSON 字节，不包含 TAB、校验字符或 LF。串口参数默认 115200、8N1，由连接配置决定。单帧上限 64 KiB。

请求 JSON 示例：`{"id":1,"command":"read","params":{}}`。应答 JSON 示例：`{"id":1,"result":{"temperature":23.6,"rawTemperature":236,"target":25.0}}`。设备错误用相同 `id` 与 `error:{code,message}`。这些示例省略帧尾，实际发送请调用 `hmi_core::framing::encode_demo_frame` 计算校验和，不能直接发送裸 JSON。

增量解析用 `LineDecoder` 聚合分包并拆解粘包，再用 `decode_demo_frame` 验证 CRC。CRC 错误返回 `CHECKSUM`，帧超限返回 `FRAME_TOO_LARGE`；相关 ID 不匹配的晚到帧直接丢弃。CRC 只能发现传输损坏，并非身份认证。

可通过实体串口对接或系统已安装的虚拟串口对测试。例如设计器连接 COM7，对端运行：

```powershell
target/release/demo-protocol.exe --device COM8 --baud 115200
```

在 macOS 上可使用原生构建产物，例如：

```sh
cargo build -p demo-protocol --release
target/release/demo-protocol --device /dev/cu.usbserial-XXXXX --baud 115200
```

`--device-stdio` 在 stdin/stdout 上提供相同线协议，便于设备实现者做互操作测试。默认不传参数时为宿主 JSON-RPC 插件模式。串口对、驱动及电气接线须自行准备，本仓库不安装串口驱动。

## 扩展协议

可用任何能读写标准输入输出的语言实现上述 RPC，并提供同样的 manifest 结构。Rust 插件可复用 `hmi-core` 的 `PluginManifest`、`CommandDescriptor`、`PointDescriptor`、`CoreError`、`LineDecoder` 和 CRC 辅助函数，或独立实现自己的帧解析。协议自带运行时/依赖应放在插件包内，发布时随插件完整复制；宿主的可选 Python 节点环境与插件依赖独立。

验证命令：`cargo test -p hmi-core -p demo-protocol`。测试覆盖 DAG 语义、错误与停止路径、消息/队列/步骤限额、增量 CRC 解析、超时后的旧应答隔离、真实子进程模拟连接、排空已发命令及取消未发命令。串口硬件回环需在目标设备现场单独验证。
