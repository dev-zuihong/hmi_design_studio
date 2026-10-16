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

## CHG-0014：紧凑工作区布局验证（2026-10-16）

本轮仅修改编辑器布局、交互与对应前端测试，方案升至 v0.9，应用仍为 0.2.0。Windows 本机使用现有依赖与 Vite 服务，未修改用户已有的 `pnpm-lock.yaml`。

### 尺寸与操作

| 项目（1366×768 默认布局） | 调整前 | 调整后 | 用途 |
| --- | --- | --- | --- |
| 顶部栏总高 | 141px | 48px | 项目和全局命令集中到一行 |
| 画布上下文栏与标尺 | 63px | 54px | 保留对象操作及定位入口 |
| 底栏、分隔条与状态栏 | 251px | 32px | 默认收起，按标签展开资源 |
| 画布可滚动高度 | 313px | 634px | 增加 321px，超过 150px 验收目标 |

真实 Chromium 页面覆盖 1920×1080、1366×768、800×600，zh-CN/en-US，主窗/逻辑窗共 12 个布局组合；均无整页或顶部工具栏横向溢出，最窄窗口中央工作区宽度为 320px。实际拖动到左 400px、右 500px、底 400px 后缩至 800×600，有效尺寸变为 180/300/270px，画布可滚动区域仍至少 180px 高；恢复大窗或重新打开后保留首选尺寸。

自动检查还确认：主/逻辑窗分开保存偏好；恢复默认布局不改变工程内容、未保存状态或撤销历史；菜单动作后焦点返回触发按钮；搜索展开命中分组且清空恢复折叠；布局托管、多选、常用文案、Python 参数、协议参数类型、只读限制、变量搜索、日志筛选和错误计数保留。预览热更新、运行时控件和共享布局使用既有回归检查。

测量与截图保存于忽略目录：`artifacts/layout-before.json`、`artifacts/layout-before-1366.png`、`artifacts/layout-after-zh-CN.json`、`artifacts/layout-after-en-US.json`、`artifacts/layout-{main|logic}-{locale}-{width}.png`。

### 实际执行结果

| 检查 | 结果 |
| --- | --- |
| `pnpm check` | Vue/TypeScript 检查通过 |
| `pnpm test` | 51 项单元测试通过，包括 14 项布局偏好及 2 项搜索测试；覆盖页面立即刷新/关闭时同步保存偏好与拖拽收尾 |
| `pnpm build` | Studio 与 Runner 生产前端构建通过；保留 Studio 单个 JS chunk 超过 500 kB 的体积提示 |
| `pnpm exec playwright test --workers=3 --output=artifacts/layout-final-results` | 当时完整 Chromium 集合 47 项通过，含拖放、保存重开、双窗冲突、热更新、组件运行及新增面板交互 |
| `pnpm exec playwright test tests/e2e/designer-layout.spec.ts --workers=2 --output=artifacts/layout-matrix-results` | 6 项通过；在前述集合基础上增加 2 项中英文尺寸矩阵测试，并复验布局/焦点。当前共 49 个不同交互用例均有通过结果 |
| 资源展开专项（`--grep 'serial panel\|validation and preview\|editor stays editable'`） | 3 项通过：拖入串口展开设备、主动校验失败展开检查、后台热更新失败保持底栏收起 |
| 实际 `pnpm tauri dev` + WebView2 调试验证 | Windows x64 Studio 0.2.0 启动通过，原生主窗/逻辑窗均为 48px 工具栏、32px 底栏；侧栏操作保持分窗独立，工程内容与修订号不变，无页面未处理异常 |
| 文档、日志及 Git 检查 | UTF-8、文档相对链接、历史日志前缀、CHG 编号唯一性、`git diff --check` 与新增文件暂存后的 `git diff --cached --check` 通过 |

首轮交互回归曾有 8 项失败：旧语言/平台/状态栏入口定位、协议字段展开时序、原生 select 名称定位，以及按钮文案被折入行为组。已同步测试入口并将常用文案放回默认展开的内容组，随后上述最终回归通过；失败不计作通过结果。

原生验证先等待已有 Studio/Vite 会话退出，随后在独立测试子进程环境启用 WebView2 调试端口 9335 并实际执行 `pnpm tauri dev`。验证完成后恢复测试前 localStorage 布局/语言值，停止本次启动的进程树；未替换或编辑恢复的工程，未启动设备或预览。报告与截图为 `artifacts/layout-native-report.json`、`artifacts/layout-native-main.png`、`artifacts/layout-native-logic.png`，启动输出为 `artifacts/layout-native-dev.log`。日志保留既有 Rust 未使用代码警告及 Tauri 自定义 IPC 协议回退到 postMessage 的提示；环境读取、工作区读取与逻辑窗打开均实际成功。初始连接预览状态时出现“预览 host 未运行”通知，本轮未修改该运行连接行为，也未以布局验证代替预览执行验证。

### 未覆盖范围

本轮原生检查限定为开发启动、双窗布局和偏好/工程隔离，不代替原生设备、Python 或 Runner 执行验收。`scripts/test-native.mjs` 仅同步了布局定位，未重新运行其完整设备测试。Rust、Python 执行核心未修改，本轮未重跑其测试；WebKit、真实串口、Windows 10、不同 DPI、干净机器和其他平台验收仍待相应环境。早期针对既有进程的只读窗口截图未取得有效画面，不计作验证；最终原生结果来自上述独立启动。

## CHG-0015：PTB Demo 与设备组件（2026-10-16）

本轮在 Windows 11 x64 执行，应用 0.3.0、方案 v1.0、项目 schema 3。参考 `ING_PTB/07-gui` 只读；所提取源码及摘要记录于 `backends/ing-ptb/source-provenance.json`。演示工程由生成脚本产生，包含 2 页、147 个可编辑控件、53 个流程、1 个设备窗口模板和 1 个明确为 simulation 的后端实例。

### 自动化与协议

| 检查 | 实际结果 |
| --- | --- |
| `npm run build` | TypeScript 检查与 Studio/Runner Vite 生产构建通过；Studio 仍有既有大 chunk 提示 |
| `npm test` | 最终全部 59 项通过，包含独立设备窗口状态、草稿/上下文、热更新隔离，以及绑定变化清除过期输入而保留未提交设备 ID |
| `npm run test:e2e` | 最终完整 Chromium 56 项通过；新增设备控件及既有 Gallery 共 10 项在 WebKit 通过，包含真实样式下的小控件可见性与数值选项类型 |
| `cargo test --workspace` | 89 项通过、3 项默认忽略；其中 40 项 core、33 项 desktop、8 项 Python，其余为示例协议与 headless 测试 |
| 后端工程包 ignored 专项 | 设置 `HMI_BACKEND_PACKAGE_IN/OUT` 后显式执行 `verify_backend_package_interop_when_requested -- --ignored`，1 项通过；另外实际依赖工程包、numpy/requests 专项未运行 |
| `dotnet run --project backends/ing-ptb/tests/IngPtb.Backend.Tests.csproj -c Release` | 66 项断言通过，覆盖 C1–C7/CRC、分片/取消/结果未知隔离、配置 v2/v3/v4、滚码边界、日志预算和重复 C2 结果不重复计数 |
| `backends/ing-ptb/build.ps1` | .NET 10 Windows x64 NativeAOT 发布成功，0 警告、0 错误；目标无需 .NET |
| `node backends/ing-ptb/tests/rpc-smoke.mjs .../ing-ptb-backend.exe` | 实际原生进程 RPC 通过：三设备自动握手、20 个快照、设置/任务/导出、非法路径/操作拒绝及退出 |
| 温度示例 CLI smoke | 既有 `resources/example/project.json` 使用 `sim://temperature` 通过，结果 23.6°C |

会话回归使用实际子进程 fixture，验证初始快照可用、相同资源内容复制保持 provider、失败切换恢复旧模型与连接并释放屏障、独立工程资源根和动态文件快照保留、已受理后台任务阻止资源切换而停止会话可打断等待、资源字节变化及 provider 崩溃恢复。文件测试验证不可变快照、配置附件往返、缺文件报告、同名冲突拒绝和路径越界拒绝；窗口测试覆盖需复位状态保留、拔出、代次变化和 provider 失败。

### 集成发现与修正

- 首轮原生脚本未复制固定 WebView2，端点未启动；随后补齐真实运行资源。旧构建对清单缺省 modes 的校验不一致，修正统一清单加载并在 PTB 清单明确列出 simulation/usb。
- 初轮桌面新增测试的无后端 fixture 仍保留后端绑定、Windows canonical 路径前缀断言不一致；修正 fixture 和规范化比较后全部通过。
- 真机 WebView2 前七阶段通过，但主窗关闭后子窗阻止 Runner 退出；改为主窗先停止会话、关闭设备子窗再销毁主窗。每窗 favicon 请求 500 确认为缺少资源，已补本地图标。
- 截图发现通用 scroll 容器误显示未知类型、较小统计卡片/按钮裁剪和数值下拉未显示选中值；按共享组件修复并增加实际样式可见性检查。地址统一为 uint32 十进制或带 `0x` 的十六进制，表格地址单元格使用同一校验。
- 试跑时本仓库无命名的 Playwright `chromium` project，也无 `test:unit` npm 脚本；改用默认 Chromium 配置、`--browser=webkit` 专项和现有 `npm test`，上述结果均来自修正后的实际命令。
- 首次源码 ZIP 打包在 PowerShell 5 缺少显式加载的 `System.IO.Compression`，补齐程序集后完整构建脚本重跑成功。最终截图还检出只有颜色配置的徽标丢失默认文本；修正通用回退并通过对应表格回归，随后重建并重新导出运行包。

### 最终交付与工程包互通

最终构建命令为 `cargo build --workspace --release --target x86_64-pc-windows-msvc --features hmi-studio/custom-protocol,hmi-runner/custom-protocol`。随后执行 `scripts/build-ptb-demo.ps1 -SkipBuild -OutputDirectory artifacts/ing-ptb-final`，复用本轮刚构建的产物；脚本重新暂存 Runner 模板并生成、校验、模拟运行和导出。

| 产物 | 内容 |
| --- | --- |
| `artifacts/ing-ptb-final/ING-PTB.hmiproj` | 完整可编辑工程、后端与示例素材，1,892,057 字节，依赖无缺失 |
| `artifacts/ing-ptb-final/ING-PTB-win-x64.zip` | Runner、NativeAOT 后端和固定 WebView2，329,126,386 字节，269 个文件，无 Python |
| `artifacts/ing-ptb-final/ing-ptb-backend-source.zip` | 31 个后端源码/配置/测试/构建文件，与当前源码逐字节核对通过 |
| `artifacts/ing-ptb-final/runtime/` | 经完整文件集合与 SHA-256 校验保留的解压目录 |

运行包 SHA-256：`b3170ecc00fb8953c1eb05e28764cce23d02694b92e2799c34abac0c66bacbe1`。提供程序 EXE SHA-256：`07a3f13b3bc443c85341f17ec6988eda4fdbc3113dafaa79c461f24490471fb8`。包内 Runner 和后端与最终构建逐字节一致。

执行 `python scripts/verify-package.py artifacts/ing-ptb-final/ING-PTB-win-x64.zip --directory artifacts/ing-ptb-final/runtime --python absent --cli target/x86_64-pc-windows-msvc/release/hmi-cli.exe --flow flow_scan --report artifacts/ing-ptb-final/package-verification.json` 成功：文件清单、全部摘要、Python 按需排除及仅 Windows 系统 PATH 的包内模拟流程通过。报告为该目录中的 `validate.json`、`smoke.json`、`package.json`、`export.json`、`package-verification.json` 和 `source-verification.json`。

工程包经真实 `BrowserProjectPackage.open/save`、Rust `read_package/write_package` 再到浏览器打开保存，最终整个项目模型一致，7 个后端/素材文件字节一致；报告为 `artifacts/ing-ptb-final/package-interop.json`。这些构建产物和本机测试报告位于忽略目录，不纳入源码提交。

### 原生与交付验收边界

使用 `node scripts/test-native-ptb.mjs --exe artifacts/ing-ptb-final/runtime/HmiRunner.exe --project artifacts/ing-ptb-final/runtime/project.json --resources artifacts/ing-ptb-final/runtime --output artifacts/native-ptb-package-final --port 9345` 实测最终解压包：9 个阶段全部通过，WebView2 错误 0，主窗正常关闭退出码 0，无强制清理。覆盖三设备、两个独立设置窗口及同设备复用、TVCC/次数草稿隔离、六页签、固件快照不随源文件变化、配置伴随资源、模拟 C3 下发和 C1/C2 完成、停止关窗及 provider 退出、停止时模式设置保存并恢复 simulation、重启及主窗关闭全进程收尾。启动子进程 PATH 仅含 Windows 系统目录，Python/.NET 环境路径指向不存在的位置；测试宿主自身使用开发工具，不等同干净机器验收。

最终报告 `artifacts/native-ptb-package-final/report.json`，截图 `main.png`、`main-completed.png`、`settings-firmware.png`、`settings-second-device.png` 已逐张检查：统计数字、按钮文字、下拉选值、滚动区、中文状态徽标正常，未出现未知控件占位。窗口及任务均使用模拟设备；USB 模式只在停止状态下测试设置保存，启动前恢复并断言 simulation。

原生脚本使用真实 WebView2、Rust 会话与 NativeAOT 后端，在独立测试工程中固定 simulation，禁止运行真实 USB。文件选择对话框由同一宿主文件导入命令代替，以便自动化选择确定文件；真实选择器仍需人工交互验收。模拟测试不代表真实 USB Printer/ZLP、硬件固件 C1–C7、实际烧录、热插拔驱动行为、Windows 10、干净机器、长期运行或不同 DPI 已通过。

当前 USB 热插拔采用 650 ms 枚举轮询，未接入 WM_DEVICECHANGE。多连续段 HEX/IHX 可下发，但原 GUI 配置重载有局限，导出这类 `.ptbcfg` 会明确拒绝并要求分段 BIN。`rfTesting` 统计为 0，因为当前协议没有单独 RF 阶段通知。本轮没有 macOS PTB 后端构建或平台验收。

## CHG-0016：预览 host 首次启动与退出恢复（2026-10-16）

根因为真实 Rust `get_preview_state` 在 host 尚未创建时返回错误，阻断前端 `connectPreview → open_preview_window`。既有浏览器模拟的空闲状态与真实后端不一致，原生 smoke 又直接调用启动命令，因此此前测试未覆盖用户按钮路径。本轮保持前端先订阅事件，修复 Rust 空闲状态与退出回收。

| 检查 | 实际结果 |
| --- | --- |
| `npm run build` | Vue/TypeScript 检查及 Studio/Runner 前端生产构建通过 |
| `npm test` | 64 项通过；新增 5 项覆盖空闲连接、首次启动、停止重开及真实读取/启动/同步错误与重试 |
| `npm run test:e2e -- tests/e2e/preview.spec.ts` | Chromium 预览专项 9 项通过；这些用例使用 IPC 模拟，不代替下面的原生验证 |
| `cargo test -p hmi-desktop` | 最终语义修改后 33 项通过、2 项依赖环境专项默认忽略；覆盖会话收尾、IPC 断连、工作区和设备窗口等既有回归 |
| Windows x64 原生构建 | Studio/Runner Release 构建通过；命令如下，测试工作区覆盖仅在 `native-preview-smoke` 开启 |
| 普通 Studio 隔离保护 | 对真实 debug Studio 传入测试工作区环境变量，确认立即拒绝且未创建该目录，不会使用用户活动工作区 |
| 脚本与格式 | `node --check scripts/test-native-preview.mjs`、`lib.rs` 独立 rustfmt 检查通过；全仓格式检查仍有 `preview_host.rs` 的既有长元组及 `resources.rs` 的既有单行条件格式差异，本轮未扩大格式整理范围 |

```powershell
cargo build -p hmi-studio -p hmi-runner --release --target x86_64-pc-windows-msvc --features hmi-studio/custom-protocol,hmi-studio/native-preview-smoke,hmi-runner/custom-protocol
```

将构建的 Runner 暂存到 `.cache/templates/windows-x64/HmiRunner.exe`，将 Studio 复制到 `artifacts/native-preview-final-binaries/hmi-studio.exe` 以避免测试锁定构建输出。原生验证命令：

```powershell
node scripts/test-native-preview.mjs --exe artifacts/native-preview-final-binaries/hmi-studio.exe --resources .cache --output artifacts/native-preview-final-fixed --port 9351
```

脚本创建唯一、无通信/后端/Python 的工程，只复制模板及固定 WebView2，隔离工作区、恢复文件、锁和 WebView 用户目录。以真实 UI 按钮检查首次启动、重复聚焦同一 PID、逻辑窗加入、逻辑窗停止并同步主窗、停止重开、原生关闭 Runner、关闭后重开、强制结束测试 Runner 后恢复、再次重开和关闭 Studio 清理。停止断言先观察两窗按钮恢复，再查询状态，防止状态查询本身回收 host 掩盖退出事件问题。进程清理核对父子关系和创建时间，不按名称结束用户进程。

首轮 fixture 错误地加入仅属于快照的 `points` 字段，导致项目解析失败；修正 fixture 后 11 阶段通过、WebView 错误 0，实际检视了 Runner 渲染。随后发现正常关窗和强制退出后的按钮恢复多等约 25 秒（原生关闭 27.442 秒，强制退出恢复 26.101 秒）。修复为已断连时不发关闭 RPC，直接等待子进程自行收尾，保留原有最多 30 秒退出预算。最终脚本为此无通信 fixture 增加 15 秒恢复断言；该断言不缩短真实设备事务的产品收尾边界。

最终报告为 `artifacts/native-preview-final-fixed/report.json`：11 阶段全部通过、WebView 错误 0、Studio 正常退出码 0，所有测试进程收尾完成，无额外强制清理。真实 Runner 与 Studio 的 PID 不同，`rendererInspected:true`；`studio-idle.png`、`studio-preview-running.png`、`preview-running.png` 保存于同目录，已检查运行界面截图。无通信工程的原生关窗恢复降至 2.212 秒，强制退出恢复降至 1.807 秒，均通过 15 秒断言。报告中的实际二进制 SHA-256 与暂存副本一致：Studio `bc04a52a278e69d692e07ea709b3109ad52951fd263838d8dbf6f0e580c1c203`，Runner `2fc367c0c6c7a8d48d187cbe7d069ad9a58d435e164a35d3120ed9706b01b09f`。前两轮功能验证报告保留在 `native-preview-first-2` 和 `native-preview-final`，它们尚含 25 秒等待，不用作最终耗时结论。

本轮限 Windows 11 x64 本机预览生命周期，无真实串口/USB、Python 节点或 PTB 后端运行。未重做完整产品回归、PTB 独立运行包发布、Windows 10/干净机器、其他平台、长期或 DPI 验收。构建保留既有 JS chunk 及 Rust 未使用代码提示；用户 `pnpm-lock.yaml` 修改不纳入本次提交。

## CHG-0017：设计系统、通用流程能力与移除所有示例

验证日期为 2026-10-16，北京时间；环境 Windows 11 x64。应用版本 0.4.0、工程 schema 4，方案 v1.2。最终范围按用户最新指令改为空白工程和通用能力，不交付任何温度、Python 或 PTB 示例；历史章节的演示包、示例协议和 PTB 后端均已移除。

本轮实现令牌/主题、主组件/实例/覆写/变体、本地版本库、Flex/Grid 及自有 Vue 基础组件；保留串口、用户协议插件、工程 Python 模块、设备作用域分派、通用 USB Printer、文件快照及持久未知结果边界。业务代码与协议由用户工程提供。

在删除示例前，设计系统版本已执行 `pnpm test`（79 项）、`pnpm test:e2e`（58 项）、`pnpm build` 和 Rust workspace（103 项通过、3 项忽略）。这些结果属于删除前基线；下方记录删除后重新验证的实际结果，不把旧测试数当作当前测试数。

原生设计系统验证报告保留在 `artifacts/chg0017-native-preview-v2/report.json`：12 阶段通过、WebView 错误 0；覆盖 schema 4 IPC、实例映射、库图片字节与落盘、实际主题色变化、覆写热更新保持 Runner PID，以及预览首次启动/停止/关窗/异常退出后重开和主窗收尾。该报告在移除示例之前执行，使用独立最小合成工程，不覆盖最终空白启动默认值。

PTB 演示工作已停止。停止前 CLI 模拟和运行包清单曾通过，真实 Runner 的双窗口全链路尚未通过；用户随后要求删除，故没有继续完成或交付 PTB。相关源码、专用脚本、运行包、缓存插件及 PTB 原生测试目录均已删除，不再提供其下载路径或验收承诺。

### 删除范围

- 删除 `examples/`、`resources/example/`、`crates/demo-protocol/`、`backends/ing-ptb/`，以及示例专用生成、变体、构建与原生演示脚本。
- 删除本仓库内示例插件/后端缓存、PTB 工程互通缓存、温度/Python/PTB 运行包和含示例的旧发行目录，共清理 36 个经绝对路径核对的源/缓存/产物目标；另移除 145 个名称明确的 demo-protocol 构建缓存目标。
- 前后端默认空白工程，去除“添加 Python 示例流程”、模拟示例端口和演示插件入口；发行脚本不再构建或附带示例。表格组件默认数据行为空，两份包验证脚本移除温度演示参数及断言。自动测试采用测试专用最小合成数据。
- 保留通用组件、通信与流程/Python 能力；仓库外用户工程、恢复数据和用户进程不在删除范围。

### 最终验证

| 检查 | 删除后的实际结果 |
| --- | --- |
| `pnpm test` | 79 项通过；清除表格默认示例行后再次执行通过 |
| `pnpm build` | Vue/TypeScript 检查及 Studio/Runner 双入口生产构建通过 |
| `pnpm exec playwright test --workers=2` | 最终 57 项全部通过，退出码 0，用时 1.8 分钟；前几轮修正了依赖已删除示例的测试数据，以及节点创建和预览弹出窗口正常/错误态的就绪等待 |
| `cargo test --workspace` | 设置 `HMI_TEST_PYTHON_ROOT=.cache/flow-resources/python/3.14.8-win_amd64` 后 94 项通过、0 失败、3 项依赖环境专项忽略；共享管道 import 修正后已重跑 |
| Python 准备工具 | `python -B -m unittest discover -s resources/python -p 'test_*.py'`：5 项通过 |
| 格式与脚本 | `cargo fmt --all -- --check` 通过；此前仅一处长元组格式差异已修正。修改的 PowerShell 脚本解析、Git Bash `-n`、原生预览脚本 `node --check` 通过；两份包验证 Python 脚本语法及 `--help` 通过 |
| Windows x64 构建 | `scripts/build.ps1 -SkipInstall -SkipWebview` 通过，复用已准备的固定 WebView2；随后在最终源码及前端构建完成后再执行 workspace Release 构建通过 |
| 空白工程与删除路径 | 默认资源为一页空画布，控件、连接、变量、流程、后端和工程代码均为空，无 Python 配置；所有已删除示例路径缺失。最终 `hmi-cli validate --project resources/default/project.json --resources .cache` 返回 `valid:true`、`issues:[]` |
| 发行目录与 Python 资源 | 发行目录不含示例/演示插件/PTB 后端，Studio/Runner 与最终 Release 二进制摘要一致；测试缓存、目标缓存及发行 Python 的全部资源清单摘要通过，worker 与当前源码一致 |
| 文档与 Git | UTF-8、相对链接、历史日志前缀、17 个唯一递增编号及工作区/暂存差异检查通过；pnpm 只暂存 55 行旧 UI 依赖移除，用户原有 65 行 zip/i18n 增补保留未暂存 |

3 项默认忽略分别是本机真实插件/Python 工程包输出、指定后端工程包互通，以及 NumPy/requests 原生依赖导入专项；本轮没有为这些专项准备新的外部工程或依赖环境。

最终设计器目录为 `artifacts/HmiStudio-win-x64-20261016-162742/`；构建后已用最后一次 Release 产物更新该目录与 `.cache/templates/windows-x64/HmiRunner.exe`。目录内 `build-verification.json` 记录摘要与清理检查：Studio `fca448a9e8e625ac282c97ae0bc9b64297f29f2307904ef6bc5350bbcd457019`，Runner `485a74482e675428b0adaba113b57c5af7e2ebb3e6efc706d9d729e10e79c277`。没有生成新的演示运行包。

删除后的原生桌面验证限最终构建和 CLI 空白工程校验；没有重跑真实 WebView2 全链路，不能用前述删除前报告替代。构建仍有既有 JS chunk 体积和 Windows 下两处未使用变量提示，不影响上述构建结果。

真实串口/USB Printer/ZLP、Windows 10、干净机器、长期和 DPI 验收未完成；本轮未重新验证 macOS。设计系统使用边界见 [DESIGN-SYSTEM.md](DESIGN-SYSTEM.md)。
