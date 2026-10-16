# 首版实施与验证记录

关联变更：CHG-0004。验证日期：2026-10-15，北京时间。实际机器为 Windows 11 Home China x64，版本 10.0.26200；构建目标 `x86_64-pc-windows-msvc`。Windows 10 兼容验收、无开发环境的干净机器验收和真实串口硬件验收仍需对应环境。

## 实现范围

| 模块 | 已实现 |
| --- | --- |
| 设计器 | Vue 3 / Fluent、拖放、尺寸、多选、对齐、图层、复制删除、撤销重做、多页、项目保存打开、图片与字体资源 |
| 流程 | Vue Flow 编辑、全部首版节点、DAG 校验、Python 源码编辑、变量/输入/数据点引用、错误输出、运行路径与节点输入输出 |
| 运行核心 | 共享 Rust Runtime、自动触发限流、运行/消息/队列限制、停止恢复、取消、状态与日志 |
| 通信 | tokio-serial、每连接独立协议进程、FIFO 命令队列、配置与协议版本、示例温控协议、CRC16、超时与迟到应答处理 |
| Python | 固定官方 3.14.8 amd64 embeddable、每调用新进程、Windows Job 进程树、AST 预检、离线 wheel 依赖锁定、无系统解释器回退 |
| 发布 | Studio / Runner、固定 WebView2 155.0.4283.45 x64、按需 Python、完整插件包、本地资源、ZIP 文件校验清单、独立端口设置 |

## 自动验证入口

```powershell
npm run build
npm test
npm run test:e2e
cargo test --workspace
python -B -m unittest discover -s resources/python -p test_*.py
```

第三方 Python fixture 准备及专门测试见 [Python 资源说明](../resources/python/README.md)。默认测试不会在线下载依赖；运行资源由准备脚本明确下载/导入。

## 本轮实际结果

| 检查 | 结果 |
| --- | --- |
| `npm run build` | Vue 类型检查及 Studio / Runner 两个生产前端构建通过；设计器存在单个 JS chunk 超过 500 kB 的体积提示 |
| `npm test` | 9 项通过，覆盖项目编辑、撤销重做、快捷动作及协议参数 JSON 类型 |
| `npm run test:e2e` | 7 项通过，包含真实指针拖拽/缩放、端口连线、保存重开、串口面板拖入、快捷命令及参数类型 |
| `cargo test --workspace` | 37 项通过，1 项需要预置第三方依赖的测试默认忽略 |
| Python 原生依赖专项 | 设置 `HMI_TEST_PYTHON_DEPENDENCIES_ROOT` 后单独运行忽略测试，1 项通过 |
| Python 依赖准备单元测试 | 5 项通过 |
| `cargo fmt --all -- --check` | 通过 |
| 示例插件 DLL 依赖 | `dumpbin /DEPENDENTS` 确认仅依赖 Windows 系统 DLL，无单独 `VCRUNTIME140.dll` 依赖 |

Rust 测试包括 DAG、错误输出、事务排队与取消、迟到应答、资源缺失与架构检查、导出清单、Python 超时/内存/进程树及会话停止后的事件排空。测试不调用真实 COM 设备。

## 原生桌面验证

`scripts/test-native.mjs` 通过本机 WebView2 调试接口测试真正的桌面程序，调试端口只由测试子进程环境变量启用，不写入交付配置：

```powershell
node scripts/test-native.mjs artifacts/HmiStudio-win-x64-20261015-181343/HmiStudio.exe studio --bundled
node scripts/test-native.mjs artifacts/basic-runtime/HmiRunner.exe runner
node scripts/test-native.mjs "artifacts/native Python 运行包/HmiRunner.exe" runner
```

原生脚本验证实际控件点击、串口模拟状态、读取流程终止和界面变量更新；Studio 还验证中文/空格路径保存重开，以及 Fluent 输入框/下拉框/开关经过 Rust 流程写入变量；Runner 验证停止与端口设置保存。测试启动工作目录为系统临时目录，并设置无效 `PYTHONPATH`，检查应用不依赖当前工作目录或系统 Python 搜索路径。`--bundled` 用于检查设计器发布目录自带资源，关闭开发缓存路径覆盖。

最终设计器 `artifacts/HmiStudio-win-x64-20261015-181343/HmiStudio.exe` 与无 Python 的 `artifacts/basic-runtime/HmiRunner.exe` 原生测试通过，模拟读取均为 23.6°C；页面没有未处理的 JavaScript 异常。报告与截图保存在 `artifacts/native-studio-report.json`、`artifacts/native-basic-runner-report.json`、`artifacts/native-studio.png`、`artifacts/native-basic-runner.png`。

带 NumPy/requests 的最终 Runner 在 `artifacts/native Python 运行包/` 中文及空格目录中通过相同原生验证，模拟读取为 23.6°C，产生 31 条运行事件；报告和截图为 `artifacts/native-runner-report.json`、`artifacts/native-runner.png`。

## 最终独立运行包

`scripts/verify-package.py` 校验 ZIP 与解压目录的完整文件集合、每个文件 SHA-256、Python 按需包含及依赖锁。加 `--cli` 后通过同一后端执行包内资源模拟测试：只保留 Windows 系统 PATH、设置无效 `PYTHONHOME/PYTHONPATH`、从系统临时目录启动。它不删除或覆盖解压目录；重复验证时显式加 `--reuse`。

```powershell
python -B scripts/verify-package.py artifacts/basic-final.zip --directory artifacts/basic-runtime --reuse --python absent --cli target/x86_64-pc-windows-msvc/release/hmi-cli.exe --expected-temperature 23.6 --report artifacts/basic-verification.json
python -B scripts/verify-package.py artifacts/python-final.zip --directory artifacts/python-runtime --reuse --python stdlib --cli target/x86_64-pc-windows-msvc/release/hmi-cli.exe --expected-temperature 23.6 --report artifacts/python-verification.json
python -B scripts/verify-package.py artifacts/native-python-final.zip --directory "artifacts/native Python 运行包" --reuse --python present --native --cli target/x86_64-pc-windows-msvc/release/hmi-cli.exe --expected-temperature 23.6 --report artifacts/native-python-final-verification.json
```

上述无 Python / 标准库包均校验通过，分别包含 264 / 304 个文件，模拟温度均为 23.6°C。前者不含代码节点解释器，后者包含固定 CPython 3.14.8 与空第三方依赖锁。压缩包 SHA-256 与字节数保存在对应验证 JSON。构建产物与报告位于忽略目录 `artifacts/`，源码、锁文件、资源源清单和复验脚本纳入 Git。

第三个包 `native-python-final.zip` 包含 1,363 个文件，校验及隔离环境模拟运行通过。锁定 NumPy、requests 和四个传递依赖；19 个 NumPy `.pyd`、2 个附属 DLL、22 份依赖许可文件完整保留。包大小 351,767,160 字节，SHA-256 为 `4bbc7bccd5d4742aa66e2da62e1b4bc0c78c7f17a435415c08f232b6e507d9e9`。这些包为本次构建产物，后续构建以新生成清单和验证报告为准。

## 资源与模拟边界

- 官方 CPython ZIP 固定 SHA-256，并校验资源文件清单、PE x64、`._pth` 与依赖锁。
- 固定 WebView2 CAB 已通过 Microsoft Corporation Authenticode 验证，并固定下载 URL、版本、SHA-256 和大小；导出前及复制后复核整个文件集合。
- NumPy 2.5.4、requests 2.32.5 及完整传递依赖在实际 embeddable 中导入、计算。组合示例配置 512 MiB；256 MiB 下可能触发真实内存分配失败，运行器不会忽略限制。
- `sim://temperature` 为明确的软件模拟模式；实际子进程 RPC、调度器和变量更新均执行，模拟模式不打开 COM 设备。
- 真实串口对端工具为 `demo-protocol.exe --device COM8 --baud 115200`。使用一对虚拟或实体串口可进一步测试 OS 串口驱动与拔插；本轮没有这类设备，不将模拟结果记为硬件结果。

## 尚需目标环境验收

1. 在 Windows 10 x64 和无 Node/Rust/Python 的干净 Windows 11 x64 机器上离线启动完整运行包。
2. 验证实际设备的驱动、串口占用、权限、拔插、收发时序与现场协议。
3. 长时间运行、现场规模下的性能、不同 DPI/显示器组合及安装目录权限。

首版使用预编译 Runner 组包；源码与 Python 节点源码保留在项目/运行包。没有代码签名证书，因此产物未做应用签名。文档和测试不替代现场设备验收。

## CHG-0005：独立预览界面

本次验证环境为 macOS arm64，日期为 2026-10-15（北京时间）。此处记录独立预览页的增量验证，前文 CHG-0004 的 Windows 结果属于历史验证，不代表本次已复验 Windows 桌面程序。

- `npm run build`：Vue 类型检查及 Studio / Runner 两个前端生产构建通过；设计器仍有 JS chunk 超过 500 kB 的体积提示。
- `npm test`：现有 9 项单元测试通过。
- `npm run test:e2e`：14 项浏览器交互测试通过，包含原有 7 项与新增 7 项预览测试；覆盖独立布局、流程动作及快照更新、页面切换、返回恢复修改和工作区、停止失败重试、校验/启动失败、首次快照失败后的停止入口，以及浏览器模式提示。
- `node --check scripts/test-native.mjs`：语法检查通过；脚本新增原生 Studio 预览独立显示及停止后恢复编辑器的断言，本机未运行 Windows 原生测试。
- Playwright 使用真实 Vue 页面和模拟 Tauri IPC 验证界面交互，模拟结果不能替代共享 Rust 核心、真实串口或 Windows WebView2 验收。预览页 1500×980 及 720×600 截图已人工检查，独立布局、工具栏及画面缩放正常，截图位于忽略目录 `artifacts/preview.png` 和 `artifacts/preview-compact.png`。

本次未修改 Rust、项目格式及发布资源；Windows x64 原生构建、实际会话连接收尾及真实设备交互仍需在目标环境中复验。

## CHG-0006：macOS 支持与本机验证

验证环境：2026-10-15，北京时间；macOS 26.6.2（25G83）Apple Silicon arm64，Xcode 27.0 / SDK 27.0，Rust 1.98.1。交付基线为 macOS 14.0+，分别提供 arm64 / Intel x64 构建路径；Intel Mac 和 macOS 14 最低系统没有在本机替代验收。

| 检查 | 实际结果 |
| --- | --- |
| `npm run build:macos -- --skip-install --skip-python` | 使用已安装锁定依赖及已校验 Python 缓存，Studio/Runner 前端、Rust 程序、协议插件、CLI、app 组装及签名通过 |
| `npm test` | 9 项通过 |
| `npm run test:e2e` | Chromium 19 项通过 |
| `npx playwright test --browser=webkit` | WebKit 27.2 的 19 项通过，覆盖真实指针编辑、Fluent 控件、独立预览、平台标签及 Python 资源导入；桌面 IPC 为测试模拟 |
| `cargo test --workspace` | macOS 下 37 项通过，1 项第三方依赖专项默认忽略；Windows 专用测试在本机不执行 |
| Python 原生依赖专项 | 指定 `HMI_TEST_PYTHON_DEPENDENCIES_ROOT` 后，`cargo test -p hmi-python -- --include-ignored` 的 4 项通过，包含 NumPy/requests、超时取消、RSS 超限与后代回收 |
| Python 准备器 | 既有 5 项依赖准备测试、新增 4 项 macOS wheel 标签测试通过；拒绝高于基线的标签和错误架构，接受兼容低版本及 universal2 wheel |
| Windows Python 编译检查 | `cargo check -p hmi-python --target x86_64-pc-windows-msvc` 通过；不代表 Windows 程序构建或运行已复验 |
| 格式和脚本 | `cargo fmt --all -- --check`、macOS shell/Node/Python 语法检查通过 |

最终设计器目录为 `artifacts/HMIStudio-macos-arm64-20261015-234217/`，包含 `HMI Design Studio.app`、带基础示例的 `HmiRunner.app`、`hmi-cli`、示例插件及匹配本机平台的标准库 Python 示例。两个 app 的 Info.plist 最低系统为 14.0，二进制为 thin arm64，`codesign --verify --deep --strict` 通过。模板 app 不携带旧项目、插件或 Python，设计器内的 Python 运行资源完整文件哈希与固定源清单一致。

`node scripts/test-native-macos.mjs --studio <Studio.app> --runner <Runner.app>` 通过：两个原生进程启动并保持运行；脚本以 SIGTERM 收尾，不将此结果作为正常退出及会话停止的完整验收。中间构建创建的用户资源缓存已保留备份，再由最终 app 初始化新缓存；新缓存中的 Runner 最低系统为 14.0，标准库 Python 示例经 CLI 校验通过。

原生界面点击验收未执行：电脑控制工具返回 `Computer Use was not approved to use HMI Design Studio`。WebKit 浏览器测试与原生进程存活检查不能替代真正 WKWebView 窗口中的点击、焦点和弹层验收。

### 最终 macOS 运行包

使用最终交付目录的 CLI 从同一 Rust 核心导出，然后以 `scripts/verify-package-macos.py` 验证：

| 文件 | 文件数 | 大小（字节） | Python |
| --- | --- | --- | --- |
| `artifacts/macos-basic-final.zip` | 9 | 4,750,941 | 不包含代码节点解释器 |
| `artifacts/macos-stdlib-final.zip` | 1,711 | 49,785,526 | 固定 CPython 3.14.8，空第三方锁 |
| `artifacts/macos-native-final.zip` | 2,748 | 56,405,430 | NumPy 2.5.4、requests 2.32.5 及四个传递依赖，21 个原生扩展 |

三包的完整文件集合与 SHA-256、Unix 可执行权限、签名、插件/解释器架构、Python 清单和项目依赖锁均通过；兼容的 universal2 扩展按包含宿主架构检查。验证目录包含中文和空格；从临时工作目录、`PATH=/usr/bin:/bin` 且无效 `PYTHONHOME/PYTHONPATH` 环境执行包内资源模拟流程，三包均成功得到 23.6°C，无在线 pip 或系统 Python 回退。原生依赖项目使用 15 秒代码节点超时与 512 MiB RSS 预算；产品默认 5 秒不变。摘要及完整报告位于 `artifacts/macos-{basic,stdlib,native}-final-verification.json`。

初次集成中出现的插件 `.exe` 文件名、Tauri 默认 PNG 图标和导出接口/所有权编译错误已修正。功能测试在并发负载下曾触发流程截止，随后仅为该子进程功能测试增加启动时间余量；生产时限及专门超时测试未放宽。原生依赖 CLI 曾在并发验证期间触发 5 秒时限，串行复测通过，最终依赖示例显式配置 15 秒。初次 ZIP 验证对标准库 bytecode 缓存与运行时清单的处理不一致、原生启动脚本相对路径错误，均已修复并复验；发布清单仍覆盖全部交付文件。

尚需验收：Intel x64 的原生构建与运行、macOS 14 最低系统及干净机器部署、真实串口硬件、原生 UI 点击/焦点/弹层、正常退出和长期运行。Intel Python 归档已核对 SHA-256 与 Mach-O x86_64，未在 Intel 或 Rosetta 执行。macOS 进程组 RSS 采样不是 Windows Job 提交内存硬限制；app 仅 ad-hoc 签名，未做 Developer ID 签名或 Apple 公证。本次未重新构建或运行 Windows 原生产物。

## CHG-0007：独立原生预览弹窗

验证环境为 2026-10-16（北京时间）的本机 macOS 26.6.2 arm64。本次将应用内独立页面替换为 Tauri 原生弹窗；前文 CHG-0005/0006 保留其历史验证范围。

| 检查 | 实际结果 |
| --- | --- |
| `npm test` | 3 个测试文件、15 项通过，包含编辑器只读、会话附加、输入同步、历史去重、旧快照过滤、订阅释放及 Runner 调用兼容 |
| `npx playwright test` | Chromium 18 项通过；覆盖编辑器及平台既有操作、独立预览窗口、主窗保留、重复打开、跨窗输入、失败及停止重试 |
| `npx playwright test --browser=webkit` | WebKit 18 项通过；与 Chromium 串行执行 |
| `cargo test --workspace` | 37 项通过、1 项原生依赖专项默认忽略；本次未重复 CHG-0006 的完整第三方依赖发布专项 |
| `npm run build:macos -- --skip-install --skip-python` | Vue 类型、Studio/Runner 生产前端、Rust、原生 app 和资源组装通过；使用已锁定依赖及已校验 Python 缓存；普通构建只启用 `custom-protocol`，不包含原生测试 feature |
| `node scripts/test-native-macos.mjs --studio <Studio.app> --runner <Runner.app>` | 对下述最终生产目录执行通过，两个 app 进程均稳定存活 4.5 秒；此项仅为启动检查，正常关闭验证见后文原生双窗测试 |
| `node --check scripts/test-native.mjs` | 通过；Windows 脚本改为操作独立预览 WebView，本机未执行 Windows 原生验收 |
| 格式、脚本与文档 | `cargo fmt --all -- --check`、新增原生测试 shell 语法检查通过；8 份 Markdown 的 UTF-8、换行、代码围栏及 34 个本地链接通过，历史日志前缀保持原样，变更编号连续唯一；Git 工作区及暂存区空白检查通过 |

浏览器测试使用真实 Vue 页面、独立 popup 与共享模拟 IPC，验证前端交互和跨窗状态，不作为原生窗创建或 Rust 执行证据。初次并发执行 Chromium/WebKit 时，Vite 1420 端口与 `test-results` 发生冲突，WebKit 出现 5 项基础设施失败；停止并发后串行全套复测通过。

### 实际原生双窗与运行验证

运行 `npm run test:native:preview:macos`：脚本以 `custom-protocol,native-preview-smoke` 构建专用 debug app，嵌入最终前端并放入本机模拟插件。Rust 验证代码通过实际 Tauri 窗口及 `eval_with_callback` 执行自有页面交互，不启用远程调试端口，也不写入普通交付构建。该路径不依赖电脑控制工具对 app 的授权。

报告 `artifacts/native-preview-window-arm64-20261016-005350.json` 的 `success` 为 `true`，实际结果包括：

- `main` 与 `preview` 两个原生窗口同时可见；预览实际加载 `?view=preview` 并挂载 `.preview-screen`，主编辑器保留且进入只读状态。
- 重复打开只聚焦已有窗口，会话代号保持不变。
- 实际预览 DOM 中的“打开连接 / 读取温度”经 Tauri → Rust → 示例协议进程执行，快照及页面温度均为 **23.6°C**；此为软件模拟，不打开真实串口。
- 调用原生 `WebviewWindow.close()` 经 `CloseRequested` 收尾，窗口及会话都清除；重新打开后，实际预览停止按钮也正确关闭窗口。
- 再次重新打开后关闭主窗，最终窗口集合为空且无残留运行会话。

普通生产产物位于 `artifacts/HMIStudio-macos-arm64-20261016-005730/`。Studio/Runner 的 Info.plist 最低系统均为 14.0，二进制为 thin arm64，`codesign --verify --deep --strict` 通过；捆绑 Python 清单摘要与示例插件 target/可执行文件检查通过。使用新版时先保存并退出已运行的旧 app，再打开此目录内的 `HMI Design Studio.app`，避免相同 bundle ID 使系统继续聚焦旧实例。

本次已补齐上述原生预览控件及正常关闭的验证；未覆盖全部原生焦点/弹层场景、Windows 原生回归、Intel Mac、macOS 14 最低系统、干净机器、真实串口或长期运行。上述目标环境仍需验收；Developer ID 签名和 Apple 公证仍未提供。

## CHG-0008～CHG-0012：0.2.0 完整工程、双窗设计与进程热预览

本轮环境为 macOS 26.6.2 arm64、Node.js 26.8.1、Rust/Cargo 1.98.1；开发机 Python 3.14.7 仅用于工具脚本，项目执行资源仍为锁定的 CPython 3.14.8。工程模型升级至 schemaVersion 2，归档格式版本为 1。

### 自动化检查

| 检查 | 实际结果 |
| --- | --- |
| `npm run check`、Studio/Runner 两种 Vite 生产构建 | 通过；Studio JS 约 1.16 MB，仍有超过 500 kB 的 chunk 体积提示 |
| `npm test` | 8 个文件、35 项通过；Node 26 的 localStorage 实验性提示未影响测试 |
| `npx playwright test --workers=1` | Chromium 35 项通过，结果目录 `artifacts/v02-delivery-chromium/` |
| `npx playwright test --browser=webkit --workers=1` | WebKit 35 项通过，结果目录 `artifacts/v02-delivery-webkit/`；两套浏览器串行执行 |
| `cargo test --workspace` | 68 项通过，2 项默认忽略；其中真实资源工程包生成专项已配置环境单独执行通过，第三方原生 Python 依赖专项本轮未重跑，既有结果见 CHG-0006 |
| `cargo check -p hmi-desktop --target x86_64-pc-windows-msvc` | 通过，包含新的 Named Pipe 及断线监督路径；存在 cfg 平台相关未使用项提示，不代表 Windows app 链接或原生 UI 验收 |
| `cargo fmt --all -- --check` | 通过；新增 Rust 实现已统一格式化 |

Rust 明细包括 core 14 项单元测试、9 项修订热更新测试、8 项运行时测试，示例协议 2 项单元测试及 5 项 headless 集成测试，desktop 27 项，以及 macOS Python 3 项。本轮两项默认忽略分别为真实资源工程包生成和第三方原生 Python 依赖测试；工程包生成已另外以 `--ignored` 和实际缓存执行。

画布/工程回归覆盖释放落点、缩放/滚动、放置取消、右键菜单及锁定、独立逻辑窗、按窗口撤销/并发冲突、对象树拖拽和选项卡定位、嵌套布局、解除布局保留位置、辅助线、800×600 独立滚动、翻译和跨窗保存资源。组件 Gallery 使用 reactive 工程和快照，覆盖表格复制、输入恢复、绑定优先、选项卡/表格状态、菜单/共享动作的隐藏、禁用和图标，以及分隔面板、对话框和快捷键。独立 RuntimeView 用例确认自动布局随窗口重排且比例保持 1，关闭自适应后恢复设计尺寸等比缩放。

修订测试覆盖旧任务捕获旧配置、新流程启用、定时器更新、手动停止/限流保留、删除忙碌流程后全局取消、变量及新增输入种类的兼容保留、属性绑定读取、自定义属性变更事件与旧任务 UI 写入隔离。事件 `revision` 为运行时实际应用序号；预览状态 `revision` 是 Workspace 候选编号，跳过候选时两者不相等，见 [IMPLEMENTATION.md](IMPLEMENTATION.md)。

### 工程包互通和实际资源

Rust 生成 `artifacts/v02-rust-to-browser.hmiproj`，由真实浏览器打开并保存为 `artifacts/v02-browser-to-rust.hmiproj`；Rust 校验、展开并重导出为 `artifacts/v02-rust-to-browser-back.hmiproj`，浏览器再次打开成功。7 个归档条目中的图片、opaque Windows 插件/Python 变体字节及权限全部保留，内嵌/外置英文词条正确，无页面错误。报告为 `artifacts/v02-package-browser-roundtrip.json` 和 `artifacts/v02-package-complete-roundtrip.json`。

可重现 Rust 侧 fixture 操作：

```sh
HMI_CROSS_PACKAGE_OUT="$PWD/artifacts/v02-rust-to-browser.hmiproj" \
  cargo test -p hmi-desktop emit_example_cross_language_fixture_when_requested -- --nocapture
HMI_BROWSER_PACKAGE_IN="$PWD/artifacts/v02-browser-to-rust.hmiproj" \
HMI_BROWSER_PACKAGE_OUT="$PWD/artifacts/v02-rust-to-browser-back.hmiproj" \
  cargo test -p hmi-desktop verify_browser_package_and_reexport_when_requested -- --nocapture
HMI_ACTUAL_PACKAGE_OUT="$PWD/artifacts/v02-real-resource.hmiproj" \
HMI_PACKAGE_RESOURCES="$PWD/.cache" \
  cargo test -p hmi-desktop emit_actual_resource_package_when_requested -- --ignored --nocapture
```

真实资源工程包约 44 MiB，包含本机 demo-temperature 插件、标准库 Python 资源、图片和双语词条，1708 个文件的集合、大小和 SHA-256 全部通过，缺失依赖为空。安全展开后使用实际资源的 CLI `validate` 返回 `valid:true`，`smoke` 得到 23.6°C；原 fixture 含禁用 Python 节点，执行 Python 的派生工程验证随生产导出另列。摘要报告为 `artifacts/v02-real-resource-archive-verification.json`。

### 原生独立进程和热更新

最终运行 `bash scripts/test-native-preview-window.sh` 通过，报告为 `artifacts/native-preview-window-arm64-20261016-045036.json` 和对应 child 报告；日志为 `artifacts/v02-native-preview-final.log`。专用 feature 构建的 Studio PID 为 42848、实际 Runner PID 为 42909，断线验收另外启动 PID 42949；普通交付构建不包含此验收入口。

实际 WKWebView 和 Rust 会话验证：

- Studio 界面主窗、逻辑窗同时可见，独立 Runner 预览可见且已挂载共享渲染；重复打开保持 PID。
- 在预览真实控件中打开模拟连接并读取，Rust 快照及 DOM 均显示 23.6°C。
- 热更新期间预览 PID 保持 42909；旧任务以运行序号 1 完成，新任务用序号 2；预览候选最终应用到 Workspace revision 4。
- 运行中的资源更新进入等待，等待候选可被较新候选取代；资源恢复或切换后连接保持打开，预览 DOM 更新。
- 无效候选被拒绝，保留 revision 4 和最后有效界面，同时广播校验失败状态。
- 对另一个已打开连接的 Runner 主动关闭 IPC，该子进程自动停止 Session 并退出。
- 主窗关闭后会话停止、预览子进程退出，Studio 不再保留任何窗口。

### 普通生产交付与独立程序

最终执行 `npm run build:macos -- --skip-install --skip-python` 通过，交付目录为 `artifacts/HMIStudio-macos-arm64-20261016-050831/`，包含 `HMI Design Studio.app`、`HmiRunner.app`、CLI、示例和插件；日志为 `artifacts/v02-production-build-delivery.log`。普通构建未启用 native-preview-smoke。Studio/Runner 均为 arm64、版本 0.2.0、最低系统 14.0，`codesign --verify --deep --strict` 通过，两个生产 app 均通过原生进程启动检查。

同版本 Runner 模板缓存现在按二进制 SHA-256 比较，使用暂存目录及备份原子刷新，失败恢复旧模板；保持已有插件和 Python 缓存。实际最终 app 启动后，用户可写缓存 Runner 与 app 内捆绑模板的摘要一致，记录为 `artifacts/v02-production-app-metadata.json`；此项避免设计器更新后仍使用前一次构建的预览程序。修正后另跑 desktop 27 项测试通过。

从已通过校验的真实工程包展开资源后，派生工程在读取流程插入 Python 节点，对温度取一位小数并写入 `packagedPython:true` 标记。使用最终生产 CLI 校验和导出，再在中文/空格目录、临时工作目录、限制 PATH 且设置无效系统 Python 环境下运行包内流程：

```sh
artifacts/HMIStudio-macos-arm64-20261016-050831/hmi-cli export \
  --project artifacts/v02-real-resource-workspace/project-executable.json \
  --resources artifacts/v02-real-resource-workspace \
  --destination artifacts/v02-full-project-runtime-final-20261016.zip
python3 scripts/verify-package-macos.py artifacts/v02-full-project-runtime-final-20261016.zip \
  --directory "artifacts/v02-最终 工程运行验证" --python stdlib \
  --cli artifacts/HMIStudio-macos-arm64-20261016-050831/hmi-cli --expected-temperature 23.6 \
  --report artifacts/v02-final-runtime-verification.json
```

导出为 1712 文件、51653549 字节；文件集合、摘要、权限、签名、Mach-O 架构、Python 运行资源及依赖锁检查均通过。代码节点的实际完成事件包含 `packagedPython:true`，最终温度 23.6°C，使用包内 CPython 3.14.8；报告为 `artifacts/v02-final-runtime-verification.json`，ZIP SHA-256 为 `94d8d03591f4789d59711b2a568ad6c2c8b5d8071217eb07c1efee2ddc3d896f`。模板使用构建脚本产生的纯 Runner 模板，不使用带示例项目的分发演示 app 作为导出模板。

### 集成中发现并修正的问题

WebKit 曾在保存后的资源缺失提示改变画布尺寸时报告两处 ResizeObserver 循环通知，改用 rAF 合并尺寸处理后，最终 35 项全套通过。最终 Rust 全套首次停在 headless 的打开/关闭竞态：调用方暂停轮询时仍持有维护锁，导致 close 等待；修正异步任务锁归属和 generation 捕获后，测试确认最终 closed、打开返回 CANCELLED、关闭后命令返回 CONNECTION_CLOSED，且无迟到 open，随后全套 68 项通过。较早的自定义属性事件测试还曾过滤掉先到的 listener 事件，已修正测试事件收集。

原生脚本调试先后发现 Runner app 产品名、模板未暂存到测试 app 资源目录，以及修订号/布尔报告的旧断言问题，均已修正；最终上述结果来自真实子进程执行和退出检查。前述失败未计作通过，较早失败报告及测试输出保留在 artifacts 中。

### 文档与变更留痕

8 份 Markdown 的 UTF-8、末尾换行、代码围栏及 38 个相对链接检查通过；109 份本次暂存文本源文件均可按 UTF-8 解码。CHG-0001 至 CHG-0007 的历史前缀逐字节保留，CHG-0001 至 CHG-0012 连续唯一；方案 v0.8 与应用 0.2.0 一致。Rust 格式、原生脚本语法、`git diff --check` 和包含所有新增文件的 `git diff --cached --check` 均通过。

全部 112 个变更路径已明确暂存，工作区无额外未暂存源文件；既有 CHG-0005～0007 的暂存变更保留。Git 姓名/邮箱及显式作者环境仍未配置，按协作规则未编造身份、未创建提交，因此没有提交 ID；完整暂存差异另备份于忽略目录 `.cache/changes/completed-CHG-0005-0012.patch`。

### 未覆盖环境

本轮未执行 Windows 0.2.0 原生应用、Intel Mac、macOS 14 最低系统、干净机器、真实串口、长期运行和不同 DPI 验收；本机模拟及跨目标编译不能代替这些目标环境。应用仍使用本机 ad-hoc 签名，未做 Developer ID 签名或公证。直接替换缓存文件不触发热更新；依赖更新须经过导入及工程摘要/锁变更，协议插件替换后需重新打开预览。
