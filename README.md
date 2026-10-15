# HMI Design Studio

用于 Windows 10/11 x64 的上位机界面与流程设计器：Vue 3、Fluent UI Web Components、Vue Flow、Tauri 2、Rust。拖拽控件，配置串口和协议插件，连接流程节点，通过 Python 代码处理消息，再导出包含 UI 和后端的独立运行包。

当前实施与验证范围见 [验证记录](docs/VERIFICATION.md)。设计方案见 [PLAN.md](docs/PLAN.md)，所有源码改动按 [CHANGELOG.md](docs/CHANGELOG.md) 与 Git 提交留痕。

## 开发运行

需要 Windows x64、Node.js 24、Rust MSVC 工具链、Visual Studio C++ Build Tools。Python 节点运行资源由准备脚本下载，应用不会调用系统 Python。

```powershell
npm ci
npm run prepare:python
cargo build -p demo-protocol
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/stage-dev.ps1
npm run desktop
```

`npm run dev` 只启动浏览器编辑器，可编辑、保存 JSON 和校验结构；串口、Python、实际流程执行和独立程序导出需要桌面应用。所有模拟连接明确使用 `sim://temperature`。

## 设计与运行

1. 打开内置温度监测示例，或新建项目。界面工作区支持拖放、框选、调整尺寸、对齐、复制、删除、层级和撤销重做。
2. 在资源面板配置串口连接。示例协议 `demo-temperature` 提供 `read`、`setTarget` 命令；选择 `sim://temperature` 可在没有设备时演示，真实设备需按协议接线。
3. 将按钮事件绑定流程，或在逻辑工作区添加 UI/定时/数据触发、连接、命令、条件、赋值、变量、延时、Python、页面和日志节点。首版流程必须为 DAG。
4. 保存项目并启动预览，点击“打开连接”，再点击“读取温度”。停止或恢复流程通过独立宿主管理入口执行。修改项目后重新启动预览以应用快照。
5. Python 节点使用同步 `def transform(msg)`，返回字典或 `None`。默认 5 秒、256 MiB Job 提交内存限制，每次调用独立进程。第三方库按 [Python 资源说明](resources/python/README.md) 在开发环境准备完整依赖目录，保存当前项目后导入并再次保存，锁定依赖。
6. 导出 Windows x64 ZIP，完整解压到本地磁盘，启动 `HmiRunner.exe`。Runner 中可重新选择端口；用户设置和日志按项目 ID 保存到 `%LOCALAPPDATA%/HmiStudio/projects/`。

Python 及协议插件以当前用户权限执行，面向可信项目。Job Object 管理进程树、时间和提交内存；不是权限沙箱。运行日志包含节点输入输出，位于当前用户目录。

## 构建交付

```powershell
npm run build:desktop
```

构建脚本准备固定 Python 与 WebView2、构建 Windows x64 程序、生成 Runner 模板，并将设计器放入 `artifacts/HmiStudio-win-x64-时间/`。下载资源可在开发机器准备后离线复用；目标机器不需要 Node.js、Rust、Python、pip 或编译器。首次构建会下载依赖和固定 WebView2。

构建前关闭本项目的 Vite 开发服务器；`npm ci` 会重建依赖目录，运行中的开发服务器在 Windows 上会占用原生模块文件。

依赖已按锁文件安装且未变更时，可用 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -SkipInstall` 省略重新安装。该选项仍重新构建前端、Rust 程序及交付目录。

设计器导出使用预编译 Runner 模板。任何流程中出现 Python 节点（包括未启用流程），运行包才会加入一份 Python；插件自身携带的依赖完整保留。导出包带 SHA-256 文件清单，遇到缺资源、插件版本不符、Python 依赖锁不符或无效流程时阻止导出。

## 验证与命令行工具

```powershell
npm run build
npm test
npm run test:e2e
cargo test --workspace
python -B -m unittest discover -s resources/python -p test_*.py
cargo run -p hmi-desktop --bin hmi-cli -- validate --project resources/example/project.json
cargo run -p hmi-desktop --bin hmi-cli -- smoke --project resources/example/project.json
cargo run -p hmi-desktop --bin hmi-cli -- export --project resources/example/project.json --destination artifacts/temperature.zip
```

命令行验证、模拟运行和导出使用桌面应用同一个 Rust 服务。`smoke` 只接受显式模拟端口，避免自动操作真实设备。导出前先通过构建脚本准备模板与固定 WebView2。

模块约定见 [IMPLEMENTATION.md](docs/IMPLEMENTATION.md)，协议扩展见 [插件接口](crates/demo-protocol/README.md)。代码入口：`src/`（设计器/共享渲染），`crates/hmi-core/`（流程/串口/插件），`crates/hmi-python/`（Python），`crates/hmi-desktop/`（桌面命令/发布），`apps/`（两个程序）。
