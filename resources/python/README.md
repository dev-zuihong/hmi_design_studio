# Python 运行资源

`runtime-source.json` 固定官方 CPython 3.14.8 Windows x64 embeddable 档案及 SHA-256。摘要来自同网址的官方 Sigstore 元数据；准备脚本核对固定摘要，运行时不下载、不安装 pip、不搜索系统 Python。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/prepare-python.ps1
cargo test -p hmi-python
```

离线准备可传 `-ArchivePath` 指向已下载的官方 ZIP；默认生成 `.cache/python/3.14.8-win_amd64`。重复运行核对哈希后复用，不覆盖已有目录。更新 worker 后，使用新 `-CacheDirectory` 重新准备，然后导入该环境。

资源布局为 `python.exe`、官方完整运行文件和标准库、`executor/worker.py`、`dependencies/`、`dependencies.lock.json`、`runtime-manifest.json`。解释器的 `python314._pth` 仅允许包内标准库、执行器和依赖，不启用 site。清单记录每个文件的 SHA-256，整个目录可直接复制到导出包 `runtime/python/`。

项目包含任意 Python 节点（包含停用流程中的节点）才需要资源。设计器/Runner 每项目共享一个 `PythonExecutor`，预览和导出前先 `PythonEnvironment::load`，然后 `validate_imports()`，对每段代码 `validate_source()`；后者只解析 AST 与编译，不执行用户源码。

第三方依赖在开发机器的标准 CPython 3.14 Windows x64 环境准备。示例 `requirements.txt` 声明 `numpy==2.5.4` 与 `requests==2.32.5`：

```powershell
python scripts/prepare-python-deps.py --runtime .cache/python/3.14.8-win_amd64 --requirements requirements.txt --imports numpy requests --output .cache/python/project-env
```

添加 `--wheelhouse D:/offline/wheels` 则仅使用离线 wheel。工具解析完整传递依赖，锁定版本、目标 ABI/平台、wheel 和展开后所有文件的 SHA-256；不接受源码构建、目录覆盖、依赖冲突、错误架构或需要 `.pth` 初始化的包。原生扩展、DLL、元数据和许可证保持完整。准备输出仍须通过设计器的受限进程导入检查，下载成功不代表可运行。

每次代码调用在新 Windows Job 中挂起创建、加入后恢复，默认 5 秒墙钟限制、256 MiB Job 提交内存限制；进程树在成功、失败、取消和超时后结束。每项目一项执行、最多 64 项等待。消息必须是严格 JSON 字典，返回 `None` 结束当前分支。Python、文件描述符和 Windows 标准句柄输出重定向到日志，JSON-RPC 保留单独通道；日志最多保留 1 MiB。可信项目代码以当前用户权限运行，这些限制不是安全沙箱。

依赖准备单元测试：`python -B -m unittest discover -s resources/python -p test_*.py`。准备好 numpy/requests 环境后设置 `HMI_TEST_PYTHON_DEPENDENCIES_ROOT` 并运行 `cargo test -p hmi-python -- --ignored`，验证原生 DLL、标准库和传递依赖在实际 embeddable 解释器中加载。
