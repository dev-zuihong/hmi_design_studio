# 当前实现接口（0.3.0）

本文件固定当前模块间契约，补充 [PLAN.md](PLAN.md)。CHG-0004 为首版基线，CHG-0008～0012 扩展工程、编辑和预览；字段采用 camelCase，PTB 工程 `schemaVersion` 为 3，兼容读取 v1/v2，既有工程保留 v2。Rust 模型在 `hmi-core`，前端模型在 `src/lib/model.ts`，两者须用同一示例项目交叉验证。自动布局页面在 responsive 开启时按运行窗口可用尺寸重排、比例为 1；关闭 responsive 或使用绝对布局时按项目设计尺寸等比适配。

## 项目 JSON

```typescript
interface Project {
  schemaVersion: 1 | 2 | 3; id: string; name: string;
  width: number; height: number;
  pages: Page[]; connections: ConnectionConfig[];
  variables: Variable[]; flows: Flow[];
  python?: { runtimeId: string; runtimeSha256?: string; dependencyLock?: unknown[] };
  assets?: { id: string; path: string }[];
  i18n?: {defaultLocale:string;locales:string[];messages:Record<string,Record<string,string>>};
  actions?: {id:string;text:unknown;action:UiAction;shortcut?:string;enabled?:boolean;visible?:boolean}[];
}
interface Page { id: string; name: string; widgets: Widget[]; layout?: LayoutSpec; responsive?: boolean; guides?: {id:string;axis:'x'|'y';position:number}[] }
interface Widget {
  id: string; kind: string; name: string;
  x: number; y: number; width: number; height: number; zIndex: number;
  props: Record<string, unknown>;
  bindings: Record<string, ValueRef>;
  events: Record<string, UiAction>;
  parentId?: string|null; slotId?:string; layout?:LayoutSpec; layoutItem?:Record<string,unknown>;
  slots?:{id:string;title:string|{i18nKey:string;fallback?:string}}[]; customProperties?:Record<string,unknown>; editor?:{locked?:boolean;hidden?:boolean};
}
type ValueRef =
  | { source: 'literal'; value: unknown }
  | { source: 'message'; path: string }
  | { source: 'variable' | 'input' | 'point'; key: string }
  | { source: 'property'; key: string; property: string };
type UiAction = { kind: 'flow' | 'startFlow' | 'stopFlow' | 'shared'; flowId: string; entryNodeId?: string; actionId?: string };
interface LayoutSpec { type: 'absolute'|'horizontal'|'vertical'|'grid'|'form'; gap?:number; padding?:number; columns?:number }
interface Variable { id: string; name: string; type: 'number' | 'string' | 'boolean' | 'enum' | 'array' | 'object'; initial: unknown; options?: string[] }
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

控件 kind 和属性分组以共享 `resources/widget-catalog.json` 为准，Rust 校验与前端组件库共同读取；保留首版 10 个 kind，新增输入、数据视图及容器/窗口组件。通用属性：`text`, `fontSize`, `color`, `background`, `value`, `options`, `unit`, `precision`, `enabled`, `visible`；bindings 用同名属性键。状态值通过变量 ID 或 `connectionId.pointKey` 引用。输入快照按控件 ID 保存值。

自定义属性支持 string/number/boolean/enum/object/array、有类型初值、绑定及变更事件；事件只引用 `kind:'flow'` 的界面入口，负载为 `{widgetId,property,oldValue,value}`。属性解析顺序为自定义属性绑定、普通属性绑定、运行时 UI 覆盖、自定义初值、组件默认值；界面与流程读取共用这一约定。对象的编辑锁阻止画布拖动和调整尺寸，属性仍可在检查器中修改。

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
| `ui.setProperty` | `widgetId`, `property`, `value`: ValueRef；写已绑定属性应改写绑定变量 |
| `page.navigate` | `pageId` |
| `log` | `value`: ValueRef, `level`: info/warn/error |

普通输出 `out`，条件输出 `true/false`，可失败节点支持 `error`，输入 `in`。所有节点版本首版为 1，由项目格式统一约束。资源路径相对项目目录或已验证资源库，不能以配置中任意路径覆盖宿主文件。

## Rust 模块边界

- `hmi-core`：项目模型与校验、Runtime、HostServices trait、ConnectionManager、插件清单及进程协议；`demo-protocol` 提供示例插件。
- `hmi-python`：`PythonEnvironment::load(root)`、`PythonExecutor::new(environment)`、`execute(source, msg, limits, cancel)`、`validate_source(source)`；导出环境使用与缓存相同布局。
- `hmi-desktop`：组合连接管理器及 Python 执行器、项目文件、资源缓存、导出和 Tauri 命令。`apps/studio` / `apps/runner` 分别构建设计器与独立运行程序。
- `HostServices`：异步 open_connection、close_connection、command、python；设备命令接收 CancellationToken，由连接 actor 保持在途事务完整性。
- `Runtime`：new(project, Arc<dyn HostServices>)、subscribe、start/stop、start_flow/stop_flow、trigger(flowId, entryNodeId, msg, inputs)、update_point；方法的具体 Rust 参数由核心模块暴露并在整合时统一。
- `ConnectionManager::new(connections, plugins_root)`，open/close/command/subscribe，以及 `reconfigure` 原位替换变动资源并保留未变连接 actor。插件包目录 `<plugins_root>/<protocolId>/manifest.json`，可执行路径相对该包目录。`sim://temperature` 是示例连接的明确模拟模式，真实串口使用 tokio-serial。

协议配置及命令参数采用明确受限的 JSON Schema 子集，详见 [插件接口](../crates/demo-protocol/README.md)。导入时拒绝不支持的约束关键字，打开连接和发送命令前再次验证实际值；动态引用值不能仅凭设计时校验放行。

## 工程包与共享工程状态

`.hmiproj` 为ZIP容器，`manifest.json` 包含 `{format:"hmi-project",formatVersion:1,projectSchemaVersion,files,missingDependencies}`。`files` 将每个相对文件路径映射到 `{sha256,size,mode:'file'|'executable',unixMode?}`，不包含清单自身；`project.json` 包含内存工程模型及源码，assets/i18n/plugins/python 资源随包保存。读取验证完整文件集合和摘要、路径及权限，不跟随符号链接。浏览器适配使用同格式，保留未知平台资源文件。

两端拒绝目录项、符号链接及跨平台不安全路径，单文件最多 1 GiB、资源总量最多 4 GiB、清单最多 16 MiB、条目最多 100000；权限声明只含普通权限位。保存检查每个资产引用，外置 i18n 表由项目当前词条重建；缺失依赖记录在清单并在编辑窗提示，完整运行校验仍会阻止缺资源项目预览或导出。

WorkspaceStore 是同一Studio工程的唯一权威状态；两个编辑WebView写入稳定ID路径操作 `{path,beforeExists,before,afterExists,after,index?}`，数组对象按ID定位，`$order`表示顺序。请求携带 `windowId/baseRevision/opId/workspaceId`，原子检查前置值和工程会话，冲突整批拒绝。撤销/重做按窗口分别托管，反向补丁不回滚其他窗内容；dirty比较当前与已保存内容指纹。

工作副本保存在用户可写目录，导入资源在首次保存前也可用。保存获取不可变工程和资源快照，在同目录写临时包后原子替换，仅标记该快照已保存；并发后续编辑仍dirty。恢复信息保存工程及各窗历史，active PID锁防止两个Studio实例写同一工作副本。工程资源按target分组保留多平台变体；当前平台解析优先目标目录并兼容旧flat缓存。

## 编辑窗与预览进程

主窗保留界面设计；`open_logic_window` 创建同Studio进程的唯一 `logic` 原生WebView（`?view=logic`），重复调用恢复聚焦并发送定位事件。两窗订阅workspace-changed，并分别保留选择和视口；关闭逻辑窗不停止工程或清历史。

`open_preview_window` 创建独立Runner `--preview-host`子进程及私有资源快照，经本机UDS/NamedPipe认证通信；Studio只代理运行命令/状态，不持有执行会话。Runner前端按environment.mode选择Preview，普通导出Runner仍按原入口自动启动静态项目。

CHG-0016 修正预览生命周期：Studio 的 `get_preview_state` 在首次启动前及停止后返回正常空闲状态（`running:false`，`project/currentPage/pid:null`，空运行快照，保留当前代次和修订号），前端仍先订阅事件、查询状态，再启动 host。已确认退出或 IPC 断开的 host 收尾后也返回空闲；存活 host 的真实 RPC 错误继续上抛。退出监听按代次清理旧会话并广播到主窗与逻辑窗，旧进程的迟到通知不得停止新预览或回写旧修订。

IPC 已断开时不再发送无法响应的关闭 RPC，直接等待子进程自行收尾退出，最多保留原有 30 秒总等待预算；通道仍连接时维持最多 25 秒关闭 RPC 和后续 5 秒退出等待。正常关窗无需耗尽超时预算，超时才强制结束进程。

前端工程操作停顿300毫秒后发送候选revision；host完成校验与资源准备后切换。Runtime每个run捕获不可变Project Arc，旧任务继续旧逻辑，新trigger用新版本；UI和普通逻辑不重启Session。资源不兼容候选进入等待屏障，停新trigger且不补发，既有run/设备事务结束后切换。状态包含revision/updateStatus/pid/locale，失败保留上一有效版本。变量枚举值域变化也进入资源屏障；资源变更以连接配置、Python运行资源摘要/依赖锁、变量schema及资产引用等工程字段判断，直接原地替换缓存文件不触发热更新。输入与语言通过IPC同步；初次状态读取失败也保留停止/重试入口。

## 桌面 IPC

前端使用 Tauri invoke；浏览器开发态只提供本地编辑与校验，不能伪装设备已连接或流程已执行。

| 命令 | 参数 / 返回 |
| --- | --- |
| `get_environment` | 返回 `{mode: 'studio'|'runner'|'preview', version, platform, target, pythonRuntimeId, pythonAvailable, runnerAvailable, webviewAvailable, projectPath?: string}`；`platform` 为 `windows-x64` / `macos-arm64` / `macos-x64`，前端据此显示平台及选择新 Python 节点的运行资源 |
| `get_initial_project` | 返回 Project；Runner 加载 `--project` 指定项目，设计器加载内置示例 |
| `open_project` | `{path}` → Project |
| `save_project` | 旧JSON兼容入口；GUI保存统一走完整工程包命令 |
| `workspace_state` | `{windowId?}` → `{workspaceId,project,revision,savedRevision,dirty,path,histories,recovered}` |
| `apply_workspace_operations` | `{windowId,workspaceId?,baseRevision,opId,operations}` → WorkspaceState，广播workspace-changed |
| `undo_workspace` / `redo_workspace` | `{windowId,workspaceId?}` → WorkspaceState，独立历史与冲突检查 |
| `replace_workspace` | `{project,path?}` → WorkspaceState，新工程会话 |
| `open_workspace_package` / `import_legacy_project` | `{path}` → WorkspaceState，导入包或旧JSON |
| `save_workspace_package` | `{path}` → WorkspaceState，保存完整.hmiproj |
| `open_logic_window` | `{flowId?,nodeId?}` → void，打开或聚焦逻辑编辑窗 |
| `validate_project` | `{project}` → ValidationIssue[] |
| `list_serial_ports` | → `{name, description}[]` |
| `list_plugins` | → 插件清单数组，含 id/name/version/configSchema/commands/points |
| `import_plugin` | `{path}`：导入插件目录或 manifest.json → 插件清单 |
| `start_preview` | `{project}` → void；Runner 校验并启动共享运行核心，Studio 的兼容调用转到独立窗口入口 |
| `open_preview_window` | `{project}` → void；仅 Studio，启动单一Runner预览子进程，重复请求恢复并聚焦已有窗口 |
| `get_preview_state` | → `{project,running,currentPage,generation,revision,updateStatus,pendingRevision?,pid?,locale,snapshot}`；附加当前运行版本 |
| `stop_preview` | → void；等待任务/连接收尾并退出预览子进程 |
| `apply_preview_update` | `{revision,project,expectedGeneration?}` → `{status,revision,issues}` |
| `update_preview_input` | `{id,value,expectedGeneration?}` → void；跨IPC同步输入快照 |
| `set_preview_locale` | `{locale,expectedGeneration?}` → void；不重启会话 |
| `trigger_flow` | `{flowId, entryNodeId?, message, inputs}` → runId |
| `start_flow` / `stop_flow` | `{flowId}` → void；宿主管理操作 |
| `runtime_snapshot` | → `{variables, points, connections, backends, flows, logs, ui, windowUi, inputs}`，各 map 以 ID 为键，ui 为主窗运行属性，windowUi 为独立窗口属性 |
| `open_connection` / `close_connection` | `{connectionId}` → void；仅运行会话有效 |
| `export_project` | `{project, destination}` → `{path, files, includesPython}`；destination 是新 ZIP 路径 |
| `import_python_runtime` | `{path}`：导入已准备且匹配宿主的资源目录 → `{runtimeId,runtimeSha256,dependencyLock}`；含 Python 节点的项目通过此显式操作切换对应平台资源，同一 runtime 重导入只更新摘要并保留项目已有依赖锁 |
| `import_python_dependencies` | `{path}`：暂存含 lock 的完整 Python 环境目录 → `{runtimeId,runtimeSha256,dependencyLock}`；由发起窗口提交一次工程操作，纳入该窗口撤销历史 |
| `import_asset` | `{path}`：复制图片/字体或 BIN/HEX/IHX/PTBCFG/JSON 工程文件到当前工作副本的 assets 目录，首次保存前也可用 → `{id,path,absolutePath,name,size,sha256}` |
| `resolve_assets` | `{project}` → 资源 ID 到项目内绝对路径；Vue 使用 Tauri asset URL |
| `save_runtime_settings` | `{project}`：Runner 停止状态下保存端口和后端模式到当前用户目录 |

文件选择由 `@tauri-apps/plugin-dialog` 完成，读写由上述命令负责。运行事件通过 `runtime-event` 推送，统一信封 `{kind, timestamp, revision?, runId?, flowId?, nodeId?, data}`；kind 包含 `run.started/run.completed/run.failed/run.cancelled/node.started/node.completed/node.failed/variable.changed/point.changed/connection.changed/page.changed/ui.property.changed/log/flow.changed`。`data` 内含对应键和值或结构化错误。revision 为运行时实际应用序号，从初始 0 起按实际切换递增，标明任务捕获的运行版本；它与 Workspace 候选 revision 分开，候选跳号或被合并时两者不相等。预览状态的 revision 则是已应用的 Workspace 候选编号；前端可在收到事件后刷新快照，日志以事件信封保留。

Studio 的运行控制、连接、快照及停止命令支持可选 `expectedGeneration`；main/logic 传入 Studio 会话代号，代理校验后转换为子进程当前代号，拒绝旧窗口对新会话的迟到调用。Runner Preview WebView 使用子进程代号；普通导出 Runner 不传此参数。GUI 推送的 `runtime-event` 在上述信封外增加顶层 `generation`，用于跨窗过滤；核心事件、快照内日志和磁盘日志保持原有格式。前端附加历史日志时忽略该字段并按内容去重，避免快照与实时推送重复。

开发态 Python 资源缓存为 `.cache/python/<runtimeId>`：Windows 使用 `3.14.8-win_amd64`，macOS 使用 `3.14.8-aarch64-apple-darwin` / `3.14.8-x86_64-apple-darwin`。`hmi-python` 的 `runtime_id()`、`runtime_platform()`、`archive_sha256()` 固定当前目标资源；`PythonEnvironment::executable()` 返回已验证资源内解释器。插件缓存为 `.cache/plugins`，清单 target 与 `hmi_core::host_target()` 完全一致，缺失 target 的旧清单仍默认为 Windows x64。

Runner 模板为 `.cache/templates/<platform>`：Windows 包含 `HmiRunner.exe`，macOS 包含无项目资源的 `HmiRunner.app/Contents/MacOS/hmi-runner`。Windows 发布目录保持 `project.json`、`plugins/`、按需 `runtime/python/` 与 `.cache/webview2` 固定运行时；macOS 将项目、插件、资源和按需 Python 放入 `HmiRunner.app/Contents/Resources`，由 app 可执行文件位置解析，使用系统 WKWebView。

macOS Studio 初次启动从 app 的 `Contents/Resources` 为当前用户初始化 `HmiStudio/resources/<applicationVersion>/<platform>` 可写缓存（在系统 Application Support 目录），之后导入不修改已签名的 app。同版本构建的 Runner 模板按可执行文件 SHA-256 比较并原子刷新，失败恢复原模板；已有用户插件/Python 缓存不被模板刷新覆盖。开发时可用 `HMI_RESOURCE_ROOT` 指定资源目录；打包 Runner 优先使用 app 内资源，不被该开发变量覆盖。macOS ZIP 保留 Unix 权限，重新 ad-hoc 签名后再计算发布文件 SHA-256；组内 Python 子进程使用独立进程组清理与采样 RSS 限制，具体边界见 [PLAN.md](PLAN.md)。


## PTB 扩展接口（CHG-0015）

schema 3 的 `backends` 元素为 `{id,name,providerId,providerVersion,mode,config}`；`windows` 元素为 `{id,pageId,title,width,height,minWidth?,minHeight?,instanceKey?}`。`ValueRef` 增加 `{source:'backend',key,path}`、`{source:'context',path}`。快照增加 `backends` 与按窗口实例组织的 `windowUi`；控件事件消息的 `context` 携带 `windowInstanceId/deviceKey/backendId/generation`。相同流程按窗口实例单飞，停止流程仍能停止全部实例。

`HostServices.backend_invoke` 由 `BackendManager` 执行；`HostServices.host_action` 经有界请求队列交给桌面宿主，返回实际窗口或文件操作结果。支持 `backend.invoke/trigger.backend/window.open/window.focus/window.close/file.open/file.save`。文件引用是宿主注册的不可变快照；`.ptbcfg` 选择时解析本地伴随文件引用，保存时校验并复制附件。`hmi-cli package` 使用与设计器相同的工程包写入器；CLI smoke 仅允许显式模拟连接和模拟后端。

后端资源目录为 `backends/<target>/<providerId>`（工程/缓存）或 `backends/<providerId>`（独立运行）。工程导入、保存、预览资源快照和独立运行导出均包含这些目录。具体清单、协议、组件属性及 PTB 操作参见 [ING-PTB.md](ING-PTB.md) 和后端清单。

新增桌面命令 `import_backend({path})` 导入后端目录；`runtime_import_file({path})` 返回运行文件快照及配置附件引用；`get_runtime_window_state()` 按调用窗口返回实例、工程和快照，`update_runtime_window_input({id,value})` 仅更新该实例输入。设备拔出、后端失败或代次变化关闭对应窗口，模板或后端删除在工程应用后清理；关闭 Runner 主窗先收尾会话并关闭所有子窗。

资源屏障同时等待流程和已受理后端任务；资源身份包含配置与文件摘要，等内容复制不重启后端。同路径内容变化可被新候选识别。重建保留运行文件快照；失败恢复旧后端和连接，释放触发屏障。窗口 UI 热更仍使用实例上下文与独立输入，不套用主窗状态。
