# Python 运行资源

`runtime-source.json` 固定官方 CPython 3.14.8 Windows x64 embeddable 档案及 SHA-256。摘要来自同网址的官方 Sigstore 元数据；准备脚本核对固定摘要，运行时不下载、不安装 pip、不搜索系统 Python。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/prepare-python.ps1
cargo test -p hmi-python
```

离线准备可传 `-ArchivePath` 指向已下载的官方 ZIP；默认生成 `.cache/python/x86_64-pc-windows-msvc/3.14.8-win_amd64`。重复运行核对哈希后复用，不覆盖已有目录。更新 worker 后，使用新 `-CacheDirectory` 重新准备，然后导入该环境。

资源布局为 `python.exe`、官方完整运行文件和标准库、`executor/worker.py`、`dependencies/`、`dependencies.lock.json`、`runtime-manifest.json`。解释器的 `python314._pth` 仅允许包内标准库、执行器和依赖，不启用 site。清单记录每个文件的 SHA-256，整个目录可直接复制到导出包 `runtime/python/`。

项目包含任意 Python 节点（包含停用流程中的节点）才需要资源。设计器/Runner 每项目共享一个 `PythonExecutor`，预览和导出前先 `PythonEnvironment::load`，然后 `validate_imports()`，对每段代码 `validate_source()`；后者只解析 AST 与编译，不执行用户源码。

第三方依赖在开发机器的标准 CPython 3.14 Windows x64 环境准备。使用自己的依赖文件声明所需版本，再通过准备工具生成锁定资源：

```powershell
python scripts/prepare-python-deps.py --runtime .cache/python/x86_64-pc-windows-msvc/3.14.8-win_amd64 --requirements path/to/requirements.txt --imports your_module --output .cache/python/project-env
```

添加 `--wheelhouse D:/offline/wheels` 则仅使用离线 wheel。工具解析完整传递依赖，锁定版本、目标 ABI/平台、wheel 和展开后所有文件的 SHA-256；不接受源码构建、目录覆盖、依赖冲突、错误架构或需要 `.pth` 初始化的包。原生扩展、DLL、元数据和许可证保持完整。准备输出仍须通过设计器的受限进程导入检查，下载成功不代表可运行。

工程代码库的模块随工程保存，在 Python 节点中通过导入名称使用。源码预检只编译、不执行模块顶层代码；每次调用捕获启动时的工程源码版本，并在新进程中加载，不在调用之间共享模块全局状态。代码库本身不会让无 Python 节点的工程携带解释器。

每次代码调用在新 Windows Job 中挂起创建、加入后恢复，默认 5 秒墙钟限制、256 MiB Job 提交内存限制；进程树在成功、失败、取消和超时后结束。每项目一项执行、最多 64 项等待。消息必须是严格 JSON 字典，返回 `None` 结束当前分支。Python、文件描述符和 Windows 标准句柄输出重定向到日志，JSON-RPC 保留单独通道；日志最多保留 1 MiB。可信项目代码以当前用户权限运行，这些限制不是安全沙箱。

依赖准备单元测试：`python -B -m unittest discover -s resources/python -p test_*.py`。准备好 numpy/requests 环境后设置 `HMI_TEST_PYTHON_DEPENDENCIES_ROOT` 并运行 `cargo test -p hmi-python -- --ignored`，验证原生 DLL、标准库和传递依赖在实际 embeddable 解释器中加载。

## macOS

macOS 使用 `runtime-source-macos.json` 固定的 python-build-standalone CPython 3.14.8（release `20261013`）完整 install-only 分发，分别锁定 Apple Silicon arm64 和 Intel x64 的 URL 与 SHA-256。Windows 的 embeddable ZIP 和 `._pth` 不用于 macOS。

```sh
python3 scripts/prepare-python-macos.py --architecture arm64
# Intel Mac 使用 --architecture x86_64
cargo test -p hmi-python
```

默认输出 `.cache/python/3.14.8-aarch64-apple-darwin` 或 `.cache/python/3.14.8-x86_64-apple-darwin`，可用 `--archive-path` 进行离线准备。解释器入口为 `bin/python3.14`，宿主传入 `-I -S -B` 并显式加载包内项目依赖；不依赖系统 Python、site 或用户的 Python 搜索路径。基础归档全部文件和依赖均通过清单哈希验证，准备脚本会将归档中的文件别名实化，运行资源不保留符号链接。已验证缓存只有 worker 更新时可原子切换并保留旧副本；损坏归档或文件仍会被拒绝。

在开发机器准备 macOS 离线依赖：

```sh
python3 scripts/prepare-python-macos.py --architecture arm64 \
  --requirements resources/python/requirements-macos-test.txt --imports numpy requests \
  --output .cache/python/projects/dependency-test
HMI_TEST_PYTHON_DEPENDENCIES_ROOT=.cache/python/projects/dependency-test \
  cargo test -p hmi-python -- --include-ignored
```

依赖只接受与 CPython 3.14 和目标 macOS 架构兼容的预构建 wheel；`--wheelhouse` 可指定离线 wheel 目录。准备阶段可以使用开发机器的 pip 下载资源，锁定完整传递依赖、wheel 摘要、展开文件及导入模块，再由包内解释器实际导入验证。Runner 不下载、不调用 pip。macOS 资源和锁不能与 Windows 混用，已有项目通过设计器的资源/依赖导入显式切换配置；同一 runtime 重导入保留项目现有依赖锁。

macOS 每次代码调用创建独立进程组，成功、失败、超时和取消后回收组内后代；每约 100 ms 用系统 `/bin/ps` 汇总组 RSS，超限后终止并返回 `MemoryLimit`。RSS 检测存在采样窗口、共享页重复计数等差异，不等同 Windows Job 提交内存硬限制，也不阻止可信代码主动逃离进程组。用户代码仍按当前用户权限运行。

Apple Silicon 的标准库、NumPy/requests、超时/取消、RSS 超限和后代清理已在本机测试；Intel x64 资源下载与 Mach-O 架构检查不能替代 Intel Mac 的实际执行验收。
