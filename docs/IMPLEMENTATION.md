# 首版实现接口

本文件固定首版模块间契约，补充 [PLAN.md](PLAN.md)。实现期归入 CHG-0004；字段采用 camelCase，项目 `schemaVersion` 为 1。Rust 模型在 `hmi-core`，前端模型在 `src/lib/model.ts`，两者须用同一示例项目交叉验证。

## 项目 JSON

```typescript
interface Project {
  schemaVersion: 1; id: string; name: string;
  width: number; height: number;
  pages: Page[]; connections: ConnectionConfig[];
  variables: Variable[]; flows: Flow[];
  python?: { runtimeId: string; runtimeSha256?: string; dependencyLock?: unknown[] };
  assets?: { id: string; path: string }[];
}
interface Page { id: string; name: string; widgets: Widget[] }
interface Widget {
  id: string; kind: string; name: string;
  x: number; y: number; width: number; height: number; zIndex: number;
  props: Record<string, unknown>;
  bindings: Record<string, ValueRef>;
  events: Record<string, UiAction>;
}
type ValueRef =
  | { source: 'literal'; value: unknown }
  | { source: 'message'; path: string }
  | { source: 'variable' | 'input' | 'point'; key: string };
type UiAction = { kind: 'flow' | 'startFlow' | 'stopFlow'; flowId: string; entryNodeId?: string };
interface Variable { id: string; name: string; type: 'number' | 'string' | 'boolean' | 'enum'; initial: unknown; options?: string[] }
interface ConnectionConfig {
  id: string; name: string; port: string; baudRate: number;
  dataBits: number; stopBits: number; parity: 'none' | 'odd' | 'even';
  protocolId: string; protocolConfig: Record<string, unknown>;
  protocolVersion?: string;
}
interface Flow {
  id: string; name: string; enabled: boolean; timeoutMs: number;
  nodes: FlowNode[]; edges: FlowEdge[];
}
interface FlowNode { id: string; kind: string; label: string; position: { x: number; y: number }; config: Record<string, unknown> }
interface FlowEdge { id: string; source: string; sourcePort: string; target: string; targetPort: string }
interface ValidationIssue { severity: 'error' | 'warning'; path: string; message: string }
```

控件 kind：`text`, `button`, `input`, `number`, `select`, `switch`, `indicator`, `group`, `chart`, `log`。通用属性：`text`, `fontSize`, `color`, `background`, `value`, `options`, `unit`, `precision`, `enabled`, `visible`；bindings 用同名属性键。状态值通过变量 ID 或 `connectionId.pointKey` 引用。输入快照按控件 ID 保存值。

## 流程节点配置

| kind | config |
| --- | --- |
| `trigger.ui` | 无；由 UI 业务事件触发 |
| `trigger.timer` | `intervalMs` |
| `trigger.data` | `key`：完整数据点键 |
| `connection.open` / `connection.close` | `connectionId` |
| `protocol.command` | `connectionId`, `command`, `params`（参数名到 ValueRef 的映射）；结果替换 `msg.payload` |
| `condition` | `left`: ValueRef, `operator`: `eq/ne/gt/gte/lt/lte/contains`, `right`: ValueRef |
| `assign` | `path`: 消息字段路径, `value`: ValueRef |
| `variable.get` | `variableId`, `path`（默认 payload） |
| `variable.set` | `variableId`, `value`: ValueRef |
| `delay` | `durationMs` |
| `python` | `code`, `timeoutMs`（默认 5000）, `memoryMb`（默认 256） |
| `page.navigate` | `pageId` |
| `log` | `value`: ValueRef, `level`: info/warn/error |

普通输出 `out`，条件输出 `true/false`，可失败节点支持 `error`，输入 `in`。所有节点版本首版为 1，由项目格式统一约束。资源路径相对项目目录或已验证资源库，不能以配置中任意路径覆盖宿主文件。

## Rust 模块边界

- `hmi-core`：项目模型与校验、Runtime、HostServices trait、ConnectionManager、插件清单及进程协议；`demo-protocol` 提供示例插件。
- `hmi-python`：`PythonEnvironment::load(root)`、`PythonExecutor::new(environment)`、`execute(source, msg, limits, cancel)`、`validate_source(source)`；导出环境使用与缓存相同布局。
- `hmi-desktop`：组合连接管理器及 Python 执行器、项目文件、资源缓存、导出和 Tauri 命令。`apps/studio` / `apps/runner` 分别构建设计器与独立运行程序。
- `HostServices`：异步 open_connection、close_connection、command、python；设备命令接收 CancellationToken，由连接 actor 保持在途事务完整性。
- `Runtime`：new(project, Arc<dyn HostServices>)、subscribe、start/stop、start_flow/stop_flow、trigger(flowId, entryNodeId, msg, inputs)、update_point；方法的具体 Rust 参数由核心模块暴露并在整合时统一。
- `ConnectionManager::new(connections, plugins_root)`，open/close/command/subscribe。插件包目录 `<plugins_root>/<protocolId>/manifest.json`，可执行路径相对该包目录。`sim://temperature` 是示例连接的明确模拟模式，真实 COM 端口使用 tokio-serial。

协议配置及命令参数采用明确受限的 JSON Schema 子集，详见 [插件接口](../crates/demo-protocol/README.md)。导入时拒绝不支持的约束关键字，打开连接和发送命令前再次验证实际值；动态引用值不能仅凭设计时校验放行。

## 桌面 IPC

前端使用 Tauri invoke；浏览器开发态只提供本地编辑与校验，不能伪装设备已连接或流程已执行。

| 命令 | 参数 / 返回 |
| --- | --- |
| `get_environment` | 返回 `{mode: 'studio'|'runner', version, pythonAvailable, runnerAvailable, webviewAvailable, projectPath?: string}` |
| `get_initial_project` | 返回 Project；Runner 加载 `--project` 指定项目，设计器加载内置示例 |
| `open_project` | `{path}` → Project |
| `save_project` | `{path, project}` → void |
| `validate_project` | `{project}` → ValidationIssue[] |
| `list_serial_ports` | → `{name, description}[]` |
| `list_plugins` | → 插件清单数组，含 id/name/version/configSchema/commands/points |
| `import_plugin` | `{path}`：导入插件目录或 manifest.json → 插件清单 |
| `start_preview` | `{project}` → void；校验后启动共享运行核心，Runner 使用相同入口 |
| `stop_preview` | → void |
| `trigger_flow` | `{flowId, entryNodeId?, message, inputs}` → runId |
| `start_flow` / `stop_flow` | `{flowId}` → void；宿主管理操作 |
| `runtime_snapshot` | → `{variables, points, connections, flows, logs}`，各 map 以 ID 为键 |
| `open_connection` / `close_connection` | `{connectionId}` → void；仅运行会话有效 |
| `export_project` | `{project, destination}` → `{path, files, includesPython}`；destination 是新 ZIP 路径 |
| `import_python_runtime` | `{path}`：导入已准备资源目录 → void |
| `import_python_dependencies` | `{path}`：导入含 lock 的完整 Python 环境目录 → `{runtimeId,runtimeSha256,dependencyLock}`，更新当前项目后保存 |
| `import_asset` | `{path}`：复制图片/字体到已保存项目的 assets 目录 → `{id,path,absolutePath}` |
| `resolve_assets` | `{project}` → 资源 ID 到项目内绝对路径；Vue 使用 Tauri asset URL |
| `save_runtime_settings` | `{project}`：Runner 停止状态下保存端口选择到当前用户目录 |

文件选择由 `@tauri-apps/plugin-dialog` 完成，读写由上述命令负责。运行事件通过 `runtime-event` 推送，统一信封 `{kind, timestamp, runId?, flowId?, nodeId?, data}`；kind 为 `run.started/run.completed/run.failed/run.cancelled/node.started/node.completed/node.failed/variable.changed/point.changed/connection.changed/page.changed/log/flow.changed`。`data` 内含对应键和值或结构化错误。前端可在收到事件后刷新快照，日志以事件信封保留。

Python 资源缓存为 `.cache/python/3.14.8-win_amd64`，插件缓存 `.cache/plugins`，Runner 模板 `.cache/templates/windows-x64`，固定 WebView2 `.cache/webview2`。发布目录使用 `project.json`、`plugins/`、按需 `runtime/python/` 和运行可执行文件；路径由发布器组合。
