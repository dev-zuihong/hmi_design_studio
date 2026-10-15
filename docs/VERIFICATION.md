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
