# HMI Design Studio

用于 Windows 10/11 x64 与 macOS 14+（arm64、x86_64）的上位机界面与流程设计器：Vue 3、Fluent UI Web Components、Vue Flow、Tauri 2、Rust。拖拽控件，配置串口和协议插件，连接流程节点，通过 Python 代码处理消息，再导出包含 UI 和后端的独立运行包。macOS 两种架构分别在对应架构的 Mac 上原生构建，不提供 universal 二进制。

当前实施与验证范围见 [验证记录](docs/VERIFICATION.md)。设计方案见 [PLAN.md](docs/PLAN.md)，所有源码改动按 [CHANGELOG.md](docs/CHANGELOG.md) 与 Git 提交留痕。

## Windows 开发运行

需要 Windows x64、Node.js 24、Rust MSVC 工具链、Visual Studio C++ Build Tools。Python 节点运行资源由准备脚本下载，应用不会调用系统 Python。

```powershell
npm ci
npm run prepare:python
cargo build -p demo-protocol
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/stage-dev.ps1
npm run desktop
```

`npm run dev` 只启动浏览器编辑器，可编辑、保存完整 `.hmiproj` 工程包和校验结构；串口、Python、实际流程执行和独立程序导出需要桌面应用。所有模拟连接明确使用 `sim://temperature`。

## 设计与运行

1. 打开示例、旧 JSON 或 `.hmiproj`，也可新建工程。将组件拖到画布释放定位，或点击组件库后再点击画布放置；Esc 取消。右键提供复制、层级、对齐、布局和绑定操作。
2. 组件支持嵌套容器、水平/垂直/网格/表单布局、选项卡、表格、树、对话框和共享动作；属性可搜索、批量修改、恢复默认、配置翻译词条和有类型的业务属性。自动布局页面可随运行窗口重新排布，绝对定位页面按设计尺寸等比适配。滚轮在画布缩放，空格/中键平移；标尺可拖出辅助线，侧栏与底栏可调整和滚动。
3. 在资源面板配置连接及协议。示例 `demo-temperature` 提供 `read`、`setTarget`；使用 `sim://temperature` 可无硬件演示。打开“逻辑流程”独立窗口，与界面同时编辑；每个窗口独立撤销/重做，共享保存状态。
4. 点击“预览运行”启动独立 Runner 子进程。编辑器保持可编辑，修改停顿 300 毫秒后校验并热更新；无效候选不影响最近有效版本。旧任务按旧逻辑完成，新任务使用新版；连接配置、变量类型/枚举值域或 Python 环境配置变更等待旧任务和事务结束。重复打开聚焦同一预览，关闭主窗会停止预览及设备会话。
5. Python 节点使用包内解释器执行同步 `def transform(msg)`，每次调用独立进程。按 [Python 资源说明](resources/python/README.md) 准备并导入离线资源。默认超时 5 秒、预算 256 MiB；Windows Job 限制提交内存，macOS 采样进程组 RSS。
6. “保存工程包”生成 `.hmiproj`，包含全部界面/流程/代码、图片字体、语言表及工程使用的已导入插件/Python 依赖。复制包即可继续编辑；未导入的平台变体明确标记缺失。设计器和项目界面可分别切换语言，项目翻译表可导入/导出 JSON/CSV。
7. “导出程序”生成当前平台的 ZIP，完整解压后启动 `HmiRunner.exe`（Windows）或 `HmiRunner.app`（macOS）。Runner 可切换项目语言及设备端口；设置与日志位于当前用户数据目录。工程包和可执行运行包是两个独立入口。

依赖更新请通过导入菜单并保存相应运行资源摘要/锁；不支持直接替换缓存文件来触发热更新，协议插件替换后需重新打开预览。

Python 及协议插件以当前用户权限执行，面向可信项目，不是权限沙箱。运行日志包含节点输入输出，位于当前用户目录。Windows 使用 Job Object 管理 Python 进程树与提交内存；macOS 以进程组管理进程，并按进程组采样 RSS 作为近似内存预算。

## 构建交付

```powershell
npm run build:desktop
```

构建脚本准备固定 Python 与 WebView2、构建 Windows x64 程序、生成 Runner 模板，并将设计器放入 `artifacts/HmiStudio-win-x64-时间/`。下载资源可在开发机器准备后离线复用；目标机器不需要 Node.js、Rust、Python、pip 或编译器。首次构建会下载依赖和固定 WebView2。

构建前关闭本项目的 Vite 开发服务器；`npm ci` 会重建依赖目录，运行中的开发服务器在 Windows 上会占用原生模块文件。

依赖已按锁文件安装且未变更时，可用 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -SkipInstall` 省略重新安装。该选项仍重新构建前端、Rust 程序及交付目录。

设计器导出使用预编译 Runner 模板。任何流程中出现 Python 节点（包括未启用流程），运行包才会加入一份 Python；插件自身携带的依赖完整保留。导出包带 SHA-256 文件清单，遇到缺资源、插件版本不符、Python 依赖锁不符或无效流程时阻止导出。

## macOS 开发与构建

需要 macOS 14 或更新版本、Node.js 24、Rust、Xcode Command Line Tools；准备 Python 资源还需开发环境中的 Python 3.12+ 与 pip。Apple Silicon 机器构建 arm64；Intel 机器构建 x86_64。运行资源按目标架构准备；固定归档可跨架构下载，实际导入与执行仍须在对应架构的 Mac 上验证：

```sh
npm ci
npm run prepare:python:macos
npm run desktop:macos
```

`npm run stage:dev:macos` 构建并暂存当前架构的示例插件、CLI 和用于独立预览的 Runner 模板；`npm run desktop:macos` 再启动 Studio 开发版。创建发布版 Studio.app、Runner.app 模板、CLI 和示例插件：

```sh
npm run build:macos
```

成品位于 `artifacts/HMIStudio-macos-arm64-时间/` 或 `artifacts/HMIStudio-macos-x64-时间/`。Runner 模板路径为 `.cache/templates/macos-arm64/HmiRunner.app` 或 `.cache/templates/macos-x64/HmiRunner.app`。构建后应用使用 ad-hoc 签名，便于本机启动和验证；未进行 Developer ID 签名、公证或跨架构打包。Python 代码节点的 macOS 进程组 RSS 限制是近似值，不等价于 Windows Job Object 的提交内存硬限制。

可用 macOS verifier 安全解压并检查 ZIP 清单、所有文件哈希、符号链接目标、可执行权限、Runner 签名以及 Python 资源和项目锁：

```sh
python3 scripts/verify-package-macos.py artifacts/temperature-macos.zip \
  --directory artifacts/temperature-runtime-check --python absent \
  --cli artifacts/HMIStudio-macos-arm64-时间/hmi-cli --expected-temperature 23.6
```

含 Python 节点的包将 `--python` 改为 `stdlib` 或 `present`；`--native` 可额外检查第三方原生扩展架构。验证目录须是新的路径，脚本不会覆盖已有目录。`npm run test:native:macos` 只检查 Studio/Runner 原生进程能启动并保持运行，不覆盖应用正常退出和 Rust 会话收尾验收。

手动重复原生启动检查时，传入实际构建目录：

```sh
npm run test:native:macos -- \
  --studio "artifacts/HMIStudio-macos-arm64-时间/HMI Design Studio.app" \
  --runner "artifacts/HMIStudio-macos-arm64-时间/HmiRunner.app"
```

## 验证与命令行工具

```powershell
npm run build
npm test
npm run test:e2e
npm run test:native:preview:macos
cargo test --workspace
python -B -m unittest discover -s resources/python -p test_*.py
cargo run -p hmi-desktop --bin hmi-cli -- validate --project resources/example/project.json
cargo run -p hmi-desktop --bin hmi-cli -- smoke --project resources/example/project.json
cargo run -p hmi-desktop --bin hmi-cli -- export --project resources/example/project.json --destination artifacts/temperature.zip
```

命令行验证、模拟运行和导出使用桌面应用同一个 Rust 服务。`smoke` 只接受显式模拟端口，避免自动操作真实设备。导出前先通过对应平台构建脚本准备模板；固定 WebView2 仅用于 Windows。

`test:native:preview:macos` 使用专用 debug feature 构建 Studio 和 Runner，在实际 WKWebView 中检查独立进程 PID、同时打开的界面/逻辑/预览窗口、模拟控件、版本热更新、资源等待及主窗退出清理。普通交付构建不包含该测试入口；实际执行结果和未覆盖环境见验证记录。

模块约定见 [IMPLEMENTATION.md](docs/IMPLEMENTATION.md)，协议扩展见 [插件接口](crates/demo-protocol/README.md)。代码入口：`src/`（设计器/共享渲染），`crates/hmi-core/`（流程/串口/插件），`crates/hmi-python/`（Python），`crates/hmi-desktop/`（桌面命令/发布），`apps/`（两个程序）。
