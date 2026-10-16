# HMI Design Studio

用于 Windows 10/11 x64 的上位机界面与流程设计器，采用 Tauri 2、Vue 3/TypeScript、自有 Vue 组件、借鉴 Penpot 的本地设计系统、Vue Flow 和 Rust。支持拖拽界面、通信资源、数据绑定、流程块及同步 Python 代码块，并可导出包含界面和后端的独立运行包。

当前默认创建空白工程，不再提供内置示例、演示协议插件或 PTB demo。已有工程仍可打开；通信、通用 USB Printer 传输和用户导入的协议插件继续可用。方案见 [PLAN.md](docs/PLAN.md)，能力说明见 [DESIGN-SYSTEM.md](docs/DESIGN-SYSTEM.md)，实际验证及边界见 [VERIFICATION.md](docs/VERIFICATION.md)。所有源码改动按 [CHANGELOG.md](docs/CHANGELOG.md) 与 Git 留痕。

## Windows 开发

需要 Windows x64、Node.js 24、Rust MSVC 和 Visual Studio C++ Build Tools。

```powershell
pnpm install
pnpm tauri dev
```

根目录的 `tauri` 脚本进入 `apps/studio`；直接执行 `pnpm exec tauri dev` 会绕过此入口。浏览器模式使用 `pnpm dev`，支持设计和工程包编辑；真实通信、Python 和独立程序导出使用桌面应用。

预览需要 Runner 模板，首次可执行 `scripts/stage-dev.ps1` 构建并暂存。使用 Python 节点前按 [Python 资源说明](resources/python/README.md) 准备固定运行资源；程序不调用系统 Python，也不在目标机器运行 pip。

## 设计与运行

1. 新建空白工程或打开已有 `.hmiproj`。从组件库拖放控件，设置属性、布局、绑定和事件；右键和工具栏提供复制、对齐、层级与布局操作。
2. 在“设计系统”管理令牌、主题、主组件、实例、变体和离线库。页面与容器支持绝对定位、Flex 和 Grid，设计器、预览与 Runner 使用共享渲染。
3. 在资源栏添加串口或通用传输资源，按需导入自己的协议插件。通过“逻辑流程”连接流程节点，用 Python 节点及工程代码库补充业务逻辑。
4. “预览运行”打开独立 Runner；有效修改可热更新。界面窗和逻辑窗共享工程，各自保留撤销与布局状态。
5. “保存工程包”保存页面、流程、代码、素材、语言和已导入依赖。“导出程序”生成 Windows x64 运行 ZIP，完整解压后启动 `HmiRunner.exe`。

通信与流程停止遵循事务收尾规则；Python 每次调用使用独立进程，并以 Windows Job Object 管理超时、取消和内存。协议插件与 Python 运行面向可信工程，以当前用户权限执行。仅工程含 Python 节点时，运行包才携带固定 Python；插件自身依赖按其清单发布。

## 构建与验证

```powershell
pnpm build
pnpm test
pnpm test:e2e
cargo test --workspace
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build.ps1 -SkipInstall
```

完整构建会准备固定 WebView2 和按需 Python 资源，生成设计器、CLI 与 Runner 模板；发行目录不再附带示例工程或示例插件。依赖未安装时去掉 `-SkipInstall`。构建前关闭占用本项目构建输出的开发进程。

`hmi-cli` 提供 `validate`、`smoke`、`package` 和 `export`，通过 `--project` 指定自己的工程。包文件集合与摘要可使用 `scripts/verify-package.py` 校验。接口见 [IMPLEMENTATION.md](docs/IMPLEMENTATION.md)，协议扩展见 [PROTOCOL-PLUGINS.md](docs/PROTOCOL-PLUGINS.md)。

本轮仅构建和交付 Windows x64。已有 macOS 源码与构建脚本保留兼容，本轮没有 macOS 验收；真实硬件、Windows 10、干净机器和长期运行范围以验证记录为准。
