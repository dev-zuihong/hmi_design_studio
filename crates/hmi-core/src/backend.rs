//! Standalone backend providers own their transport and device sessions.
//! This protocol deliberately does not expose or reuse the serial plugin API.
use crate::{BackendConfig, CoreError, CoreResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, RwLock,
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    sync::{broadcast, mpsc, oneshot},
};
use tokio_util::{
    codec::{FramedRead, LinesCodec},
    sync::CancellationToken,
};

pub const BACKEND_MAX_BYTES: usize = 1024 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendManifest {
    pub format: String,
    pub api_version: u32,
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub version: String,
    pub target: String,
    pub executable: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_modes")]
    pub modes: Vec<String>,
    #[serde(default)]
    pub config_schema: Value,
    #[serde(default)]
    pub operations: Vec<BackendOperation>,
}
fn default_modes() -> Vec<String> {
    vec!["simulation".into(), "usb".into()]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendOperation {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendResource {
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendEvent {
    #[serde(default)]
    pub backend_id: String,
    pub kind: String,
    pub generation: u64,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    pub data: Value,
}

pub fn load_backend_manifest(path: &Path) -> CoreResult<BackendManifest> {
    if std::fs::metadata(path)?.len() > BACKEND_MAX_BYTES as u64 {
        return Err(CoreError::new("BACKEND_MANIFEST", "后端清单超过 1 MiB"));
    }
    let manifest: BackendManifest = serde_json::from_slice(&std::fs::read(path)?)?;
    if manifest.format != "hmi-backend-provider"
        || manifest.api_version != 1
        || !crate::safe_identifier(&manifest.id)
        || manifest.version.is_empty()
        || manifest.target != crate::host_target()
        || !safe_relative(&manifest.executable)
        || manifest.modes.is_empty()
        || manifest
            .modes
            .iter()
            .any(|mode| !["simulation", "usb"].contains(&mode.as_str()))
    {
        return Err(CoreError::new(
            "BACKEND_MANIFEST",
            "后端清单格式、版本、架构或执行路径无效",
        ));
    }
    crate::schema::validate_schema_definition(&manifest.config_schema)?;
    let mut names = HashSet::new();
    for operation in &manifest.operations {
        if operation.name.is_empty() || !names.insert(&operation.name) {
            return Err(CoreError::new("BACKEND_MANIFEST", "后端操作名称为空或重复"));
        }
        crate::schema::validate_schema_definition(&operation.args)?;
        if let Some(result) = &operation.result {
            crate::schema::validate_schema_definition(result)?;
        }
    }
    Ok(manifest)
}
fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(':')
        && !value.starts_with(['/', '\\'])
        && value
            .split(['/', '\\'])
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub struct BackendManager {
    configs: Vec<BackendConfig>,
    root: PathBuf,
    resources: RwLock<BTreeMap<String, BackendResource>>,
    clients: tokio::sync::RwLock<BTreeMap<String, Arc<Provider>>>,
    lifecycle: tokio::sync::Mutex<()>,
    events: broadcast::Sender<BackendEvent>,
}
impl BackendManager {
    pub fn new(
        configs: Vec<BackendConfig>,
        providers_root: PathBuf,
        resources: BTreeMap<String, BackendResource>,
    ) -> Arc<Self> {
        let (events, _) = broadcast::channel(512);
        Arc::new(Self {
            configs,
            root: providers_root,
            resources: RwLock::new(resources),
            clients: tokio::sync::RwLock::new(BTreeMap::new()),
            lifecycle: tokio::sync::Mutex::new(()),
            events,
        })
    }
    pub fn subscribe(&self) -> broadcast::Receiver<BackendEvent> {
        self.events.subscribe()
    }
    pub fn resource(&self, id: &str) -> CoreResult<BackendResource> {
        self.resources
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new("BACKEND_RESOURCE", "资源未注册"))
    }
    pub fn resource_map(&self) -> BTreeMap<String, BackendResource> {
        self.resources.read().unwrap().clone()
    }
    pub fn register_resource(&self, id: String, resource: BackendResource) -> CoreResult<()> {
        if !crate::safe_identifier(&id) || !resource.path.is_absolute() || !resource.path.is_file()
        {
            return Err(CoreError::new(
                "BACKEND_RESOURCE",
                "后端资源 ID 或已暂存文件无效",
            ));
        }
        let mut resources = self.resources.write().unwrap();
        if resources
            .get(&id)
            .is_some_and(|existing| existing != &resource)
        {
            return Err(CoreError::new(
                "BACKEND_RESOURCE",
                "已注册的资源快照不能替换",
            ));
        }
        resources.insert(id, resource);
        Ok(())
    }
    pub async fn start_all(&self) -> CoreResult<()> {
        let _lifecycle = self.lifecycle.lock().await;
        if !self.clients.read().await.is_empty() {
            return Ok(());
        }
        let mut started = BTreeMap::new();
        for config in &self.configs {
            let result = self.start_one(config).await;
            match result {
                Ok(provider) => {
                    started.insert(config.id.clone(), provider);
                }
                Err(error) => {
                    for provider in started.values() {
                        provider.shutdown().await;
                    }
                    return Err(error);
                }
            }
        }
        *self.clients.write().await = started;
        Ok(())
    }
    async fn start_one(&self, config: &BackendConfig) -> CoreResult<Arc<Provider>> {
        if !crate::safe_identifier(&config.provider_id) {
            return Err(CoreError::new("BACKEND_ID", "后端 ID 无效"));
        }
        let directory = self.root.join(&config.provider_id).canonicalize()?;
        let root = self.root.canonicalize()?;
        if !directory.starts_with(root) {
            return Err(CoreError::new("BACKEND_PATH", "后端路径超出提供程序目录"));
        }
        let manifest = load_backend_manifest(&directory.join("manifest.json"))?;
        if manifest.id != config.provider_id
            || manifest.version != config.provider_version
            || !manifest.modes.contains(&config.mode)
        {
            return Err(CoreError::new(
                "BACKEND_VERSION",
                "后端版本或运行模式与工程不符",
            ));
        }
        crate::schema::validate_schema(
            &manifest.config_schema,
            &serde_json::to_value(&config.config)?,
        )?;
        let executable = directory.join(&manifest.executable).canonicalize()?;
        if !executable.starts_with(&directory) {
            return Err(CoreError::new("BACKEND_PATH", "后端可执行文件超出其目录"));
        }
        let provider = Provider::spawn(
            config.id.clone(),
            manifest,
            &directory,
            &executable,
            self.events.clone(),
        )
        .await?;
        let resources = self.resources.read().unwrap().clone();
        let initialized = provider
            .call(
                "initialize",
                json!({"mode":config.mode,"config":config.config,"resources":resources}),
                CALL_TIMEOUT,
                CancellationToken::new(),
            )
            .await;
        match initialized {
            Ok(snapshot) => {
                if let Err(error) = provider.accept_snapshot(snapshot) {
                    provider.shutdown().await;
                    return Err(error);
                }
                Ok(provider)
            }
            Err(error) => {
                provider.shutdown().await;
                Err(error)
            }
        }
    }
    pub async fn invoke(
        &self,
        id: &str,
        operation: &str,
        args: Value,
        context: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        let provider = self
            .clients
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new("BACKEND_MISSING", "后端未启动"))?;
        if context
            .get("generation")
            .and_then(Value::as_u64)
            .is_some_and(|expected| {
                provider
                    .cursor
                    .lock()
                    .unwrap()
                    .is_some_and(|(generation, _)| generation != expected)
            })
        {
            return Err(CoreError::new(
                "BACKEND_GENERATION",
                "此窗口属于已结束的后端会话",
            ));
        }
        let descriptor = provider
            .manifest
            .operations
            .iter()
            .find(|entry| entry.name == operation)
            .ok_or_else(|| CoreError::new("BACKEND_OPERATION", "后端未声明此操作"))?;
        crate::schema::validate_schema(&descriptor.args, &args)?;
        let resources = self.resources.read().unwrap().clone();
        let value = provider
            .call(
                "invoke",
                json!({"operation":operation,"args":args,"context":context,"resources":resources}),
                CALL_TIMEOUT,
                cancel,
            )
            .await?;
        if let Some(schema) = &descriptor.result {
            crate::schema::validate_schema(schema, &value)?;
        }
        Ok(value)
    }
    pub async fn snapshots(&self) -> BTreeMap<String, Value> {
        self.clients
            .read()
            .await
            .iter()
            .map(|(id, provider)| (id.clone(), provider.snapshot.read().unwrap().clone()))
            .collect()
    }
    pub async fn is_busy(&self) -> bool {
        self.clients.read().await.values().any(|provider| {
            if !provider.alive.load(Ordering::Acquire) {
                return false;
            }
            let snapshot = provider.snapshot.read().unwrap();
            !provider.pending.lock().unwrap().is_empty()
                || snapshot
                    .pointer("/stats/running")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0
                || snapshot
                    .get("jobs")
                    .and_then(Value::as_array)
                    .is_some_and(|jobs| {
                        jobs.iter()
                            .any(|job| job.get("state").and_then(Value::as_str) == Some("running"))
                    })
        })
    }
    pub async fn shutdown(&self) {
        let _lifecycle = self.lifecycle.lock().await;
        let clients = std::mem::take(&mut *self.clients.write().await);
        futures_util::future::join_all(clients.values().map(|provider| provider.shutdown())).await;
    }
}

struct Provider {
    id: String,
    manifest: BackendManifest,
    writer: mpsc::Sender<Outgoing>,
    pending: Mutex<HashMap<u64, oneshot::Sender<CoreResult<Value>>>>,
    next_id: AtomicU64,
    alive: AtomicBool,
    closing: AtomicBool,
    child: tokio::sync::Mutex<Child>,
    tree: ProcessTree,
    tasks: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    snapshot: RwLock<Value>,
    cursor: Mutex<Option<(u64, u64)>>,
    events: broadcast::Sender<BackendEvent>,
}
struct Outgoing {
    id: u64,
    bytes: Vec<u8>,
    cancel: CancellationToken,
}
impl Provider {
    async fn spawn(
        id: String,
        manifest: BackendManifest,
        directory: &Path,
        executable: &Path,
        events: broadcast::Sender<BackendEvent>,
    ) -> CoreResult<Arc<Self>> {
        let mut command = Command::new(executable);
        command
            .args(&manifest.args)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        ProcessTree::configure(&mut command);
        let mut child = command.spawn()?;
        let tree = match ProcessTree::attach(&child) {
            Ok(tree) => tree,
            Err(error) => {
                let _ = child.kill().await;
                return Err(error);
            }
        };
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (writer, mut outgoing) = mpsc::channel::<Outgoing>(64);
        let provider = Arc::new(Self {
            id,
            manifest,
            writer,
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            closing: AtomicBool::new(false),
            child: tokio::sync::Mutex::new(child),
            tree,
            tasks: Mutex::new(Vec::new()),
            snapshot: RwLock::new(Value::Null),
            cursor: Mutex::new(None),
            events,
        });
        let weak = Arc::downgrade(&provider);
        let writer_task = tokio::spawn(async move {
            while let Some(request) = outgoing.recv().await {
                if request.cancel.is_cancelled() {
                    if let Some(provider) = weak.upgrade() {
                        if let Some(sender) = provider.pending.lock().unwrap().remove(&request.id) {
                            let _ = sender.send(Err(CoreError::cancelled()));
                        }
                    }
                    continue;
                }
                if let Err(error) = stdin.write_all(&request.bytes).await {
                    if let Some(provider) = weak.upgrade() {
                        provider.fail(CoreError::new("BACKEND_IO", error.to_string()));
                    }
                    break;
                }
            }
        });
        let weak = Arc::downgrade(&provider);
        let reader_task = tokio::spawn(async move {
            let mut lines =
                FramedRead::new(stdout, LinesCodec::new_with_max_length(BACKEND_MAX_BYTES));
            while let Some(line) = lines.next().await {
                let Some(provider) = weak.upgrade() else {
                    return;
                };
                let result = line
                    .map_err(|error| CoreError::new("BACKEND_FRAME", error.to_string()))
                    .and_then(|line| serde_json::from_str::<Value>(&line).map_err(CoreError::from))
                    .and_then(|frame| provider.receive(frame));
                if let Err(error) = result {
                    provider.fail(error);
                    return;
                }
            }
            if let Some(provider) = weak.upgrade() {
                provider.fail(CoreError::new("BACKEND_EXITED", "后端进程已退出"));
            }
        });
        let stderr_task = tokio::spawn(async move {
            let mut lines = FramedRead::new(stderr, LinesCodec::new_with_max_length(64 * 1024));
            // Bound each allocation; stderr is diagnostic and never treated as protocol data.
            while lines.next().await.is_some() {}
        });
        provider
            .tasks
            .lock()
            .unwrap()
            .extend([writer_task, reader_task, stderr_task]);
        Ok(provider)
    }
    fn fail(&self, error: CoreError) {
        if self.alive.swap(false, Ordering::AcqRel) {
            for (_, sender) in self.pending.lock().unwrap().drain() {
                let _ = sender.send(Err(error.clone()));
            }
            self.tree.terminate();
            if !self.closing.load(Ordering::Acquire) {
                let mut cursor = self.cursor.lock().unwrap();
                if let Some((generation, sequence)) = *cursor {
                    let sequence = sequence.saturating_add(1);
                    *cursor = Some((generation, sequence));
                    let mut snapshot = self.snapshot.write().unwrap();
                    if snapshot.is_object() {
                        snapshot["providerStatus"] = json!("failed");
                        snapshot["error"] = json!(error);
                        snapshot["sequence"] = json!(sequence);
                    }
                    let _ = self.events.send(BackendEvent {
                        backend_id: self.id.clone(),
                        kind: "snapshot".into(),
                        generation,
                        sequence,
                        device_key: None,
                        job_id: None,
                        data: snapshot.clone(),
                    });
                }
            }
        }
    }
    fn receive(&self, frame: Value) -> CoreResult<()> {
        if frame.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Err(CoreError::new("BACKEND_FRAME", "后端 JSON-RPC 版本无效"));
        }
        if let Some(id) = frame.get("id").and_then(Value::as_u64) {
            let result = match (frame.get("result"), frame.get("error")) {
                (Some(value), None) => Ok(value.clone()),
                (None, Some(error)) if error.is_object() => Err(CoreError::new(
                    error
                        .get("data")
                        .and_then(|value| value.get("code"))
                        .and_then(Value::as_str)
                        .unwrap_or("BACKEND_REMOTE"),
                    error
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("后端操作失败"),
                )),
                _ => {
                    return Err(CoreError::new(
                        "BACKEND_FRAME",
                        "后端响应必须包含 result 或 error",
                    ))
                }
            };
            if let Some(sender) = self.pending.lock().unwrap().remove(&id) {
                let _ = sender.send(result);
            }
            return Ok(());
        }
        if frame.get("method").and_then(Value::as_str) == Some("event") && frame.get("id").is_none()
        {
            let mut event: BackendEvent =
                serde_json::from_value(frame.get("params").cloned().unwrap_or(Value::Null))?;
            event.backend_id = self.id.clone();
            return self.accept_event(event);
        }
        Err(CoreError::new("BACKEND_FRAME", "未知后端协议消息"))
    }
    fn accept_snapshot(&self, data: Value) -> CoreResult<()> {
        if data.get("apiVersion").and_then(Value::as_u64) != Some(1) {
            return Err(CoreError::new("BACKEND_VERSION", "后端快照 API 版本无效"));
        }
        let generation = data
            .get("generation")
            .and_then(Value::as_u64)
            .ok_or_else(|| CoreError::new("BACKEND_FRAME", "后端快照缺少代数"))?;
        let sequence = data
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| CoreError::new("BACKEND_FRAME", "后端快照缺少序号"))?;
        self.accept_event(BackendEvent {
            backend_id: self.id.clone(),
            kind: "snapshot".into(),
            generation,
            sequence,
            device_key: None,
            job_id: None,
            data,
        })
    }
    fn accept_event(&self, event: BackendEvent) -> CoreResult<()> {
        let mut cursor = self.cursor.lock().unwrap();
        if let Some((generation, sequence)) = *cursor {
            if generation != event.generation {
                return Err(CoreError::new(
                    "BACKEND_GENERATION",
                    "运行中后端进程变更了代数",
                ));
            }
            if sequence >= event.sequence {
                return Ok(());
            }
        }
        *cursor = Some((event.generation, event.sequence));
        if event.kind == "snapshot" {
            *self.snapshot.write().unwrap() = event.data.clone();
        }
        let _ = self.events.send(event);
        Ok(())
    }
    async fn call(
        self: &Arc<Self>,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        if !self.alive.load(Ordering::Acquire)
            || (self.closing.load(Ordering::Acquire) && method != "shutdown")
        {
            return Err(CoreError::new("BACKEND_EXITED", "后端未运行或正在关闭"));
        }
        if cancel.is_cancelled() {
            return Err(CoreError::cancelled());
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let mut bytes =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))?;
        if bytes.len() > BACKEND_MAX_BYTES {
            return Err(CoreError::new("BACKEND_FRAME", "后端请求超过 1 MiB"));
        }
        bytes.push(b'\n');
        let (sender, receiver) = oneshot::channel();
        {
            let mut pending = self.pending.lock().unwrap();
            if pending.len() >= 128 {
                return Err(CoreError::new("BACKEND_BUSY", "后端请求队列已满"));
            }
            pending.insert(id, sender);
        }
        let weak = Arc::downgrade(self);
        // The watchdog survives a dropped flow future; a sent device transaction must drain.
        tokio::spawn(async move {
            tokio::time::sleep(timeout).await;
            if let Some(provider) = weak.upgrade() {
                let pending = provider.pending.lock().unwrap().contains_key(&id);
                if pending {
                    provider.fail(CoreError::new(
                        "BACKEND_TIMEOUT",
                        "后端未在截止时间内收尾请求",
                    ));
                }
            }
        });
        let result = tokio::time::timeout(timeout, async {
            self.writer.send(Outgoing { id, bytes, cancel: cancel.clone() }).await.map_err(|_| CoreError::new("BACKEND_EXITED", "后端输入已关闭"))?;
            // Once sent, keep the provider transaction pending even if the caller cancels.
            tokio::select! {
                result = receiver => result.unwrap_or_else(|_| Err(CoreError::new("BACKEND_EXITED", "后端请求通道已关闭"))),
                _ = cancel.cancelled() => Err(CoreError::cancelled()),
            }
        }).await;
        match result {
            Ok(Err(error)) if error.code == "CANCELLED" => Err(error),
            Ok(value) => {
                self.pending.lock().unwrap().remove(&id);
                value
            }
            Err(_) => {
                // A timed out response is ambiguous. Stop this process; never retry a device write.
                let error =
                    CoreError::new("BACKEND_TIMEOUT", "后端请求超时；会话已关闭以避免重复执行");
                self.fail(error.clone());
                Err(error)
            }
        }
    }
    async fn shutdown(self: &Arc<Self>) {
        if self.closing.swap(true, Ordering::AcqRel) {
            return;
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(16);
        let _ = tokio::time::timeout_at(deadline, async {
            let _ = self
                .call(
                    "shutdown",
                    json!({}),
                    Duration::from_secs(16),
                    CancellationToken::new(),
                )
                .await;
            let _ = self.child.lock().await.wait().await;
        })
        .await;
        self.fail(CoreError::new("BACKEND_CLOSED", "后端会话已关闭"));
        self.tree.terminate();
        let mut child = self.child.lock().await;
        let _ = child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.tree.terminate();
        for task in self.tasks.get_mut().unwrap().drain(..) {
            task.abort();
        }
    }
}

#[cfg(windows)]
struct ProcessTree(isize);
#[cfg(windows)]
impl ProcessTree {
    fn configure(command: &mut Command) {
        use windows_sys::Win32::System::Threading::{CREATE_NO_WINDOW, CREATE_SUSPENDED};
        command.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
    }
    fn attach(child: &Child) -> CoreResult<Self> {
        use windows_sys::Win32::System::JobObjects::*;
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let tree = Self(handle as isize);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
            || unsafe { AssignProcessToJobObject(handle, child.raw_handle().unwrap() as _) } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        resume_process(child.id().unwrap())?;
        Ok(tree)
    }
    fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0 as _, 1);
        }
    }
}
#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.terminate();
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0 as _);
        }
    }
}
#[cfg(windows)]
fn resume_process(id: u32) -> CoreResult<()> {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
        System::{Diagnostics::ToolHelp::*, Threading::*},
    };
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of_val(&entry) as u32;
    let mut found = unsafe { Thread32First(snapshot, &mut entry) };
    let mut result = Err(CoreError::new(
        "BACKEND_PROCESS",
        "未找到挂起的后端启动线程",
    ));
    while found != 0 {
        if entry.th32OwnerProcessID == id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
            if thread.is_null() {
                result = Err(std::io::Error::last_os_error().into());
                break;
            }
            result = if unsafe { ResumeThread(thread) } == u32::MAX {
                Err(std::io::Error::last_os_error().into())
            } else {
                Ok(())
            };
            unsafe {
                CloseHandle(thread);
            }
            break;
        }
        found = unsafe { Thread32Next(snapshot, &mut entry) };
    }
    unsafe {
        CloseHandle(snapshot);
    }
    result
}
#[cfg(unix)]
struct ProcessTree(i32);
#[cfg(unix)]
impl ProcessTree {
    fn configure(command: &mut Command) {
        command.process_group(0);
    }
    fn attach(child: &Child) -> CoreResult<Self> {
        Ok(Self(child.id().unwrap() as i32))
    }
    fn terminate(&self) {
        unsafe {
            libc::kill(-self.0, libc::SIGKILL);
        }
    }
}
#[cfg(unix)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    async fn fixture(mode: &str) -> Arc<Provider> {
        let directory = tempfile::tempdir().unwrap();
        let script = r#"
$ErrorActionPreference = 'Stop'
while ($line = [Console]::ReadLine()) {
    $request = $line | ConvertFrom-Json
    if ($request.method -eq 'shutdown') {
        [Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=@{closed=$true}} | ConvertTo-Json -Compress)); break
    }
    if ($request.method -eq 'invoke') {
        if ($request.params.operation -eq 'bad') { [Console]::WriteLine('not-json'); continue }
        if ($request.params.operation -eq 'large') { [Console]::WriteLine(('x' * 1048577)); continue }
        Start-Sleep -Milliseconds 120
    }
    $result = @{apiVersion=1;generation=1;sequence=0;devices=@();stats=@{running=0}}
    [Console]::WriteLine((@{jsonrpc='2.0';id=$request.id;result=$result} | ConvertTo-Json -Depth 8 -Compress))
}
"#;
        let manifest = BackendManifest {
            format: "hmi-backend-provider".into(),
            api_version: 1,
            id: "test".into(),
            name: "Test".into(),
            version: "1".into(),
            target: crate::host_target().into(),
            executable: "powershell.exe".into(),
            args: vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                script.into(),
            ],
            modes: vec![mode.into()],
            config_schema: Value::Null,
            operations: vec![],
        };
        let executable = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
            .join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let (events, _) = broadcast::channel(16);
        Provider::spawn(
            "test".into(),
            manifest,
            directory.path(),
            &executable,
            events,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn cancelled_requests_drain_without_ending_provider_and_shutdown_reaps_process() {
        let provider = fixture("simulation").await;
        let cancel = CancellationToken::new();
        let task = {
            let provider = provider.clone();
            let cancel = cancel.clone();
            tokio::spawn(async move {
                provider
                    .call(
                        "invoke",
                        json!({"operation":"delay"}),
                        Duration::from_secs(5),
                        cancel,
                    )
                    .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel.cancel();
        assert_eq!(task.await.unwrap().unwrap_err().code, "CANCELLED");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !provider.pending.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        assert!(provider.alive.load(Ordering::Acquire));
        provider.shutdown().await;
        assert!(provider.child.lock().await.try_wait().unwrap().is_some());
    }

    #[tokio::test]
    async fn invalid_and_oversized_frames_fail_pending_requests_and_close_process() {
        for operation in ["bad", "large"] {
            let provider = fixture("simulation").await;
            assert!(provider
                .call(
                    "invoke",
                    json!({"operation":operation}),
                    Duration::from_secs(5),
                    CancellationToken::new()
                )
                .await
                .is_err());
            assert!(!provider.alive.load(Ordering::Acquire));
            provider.shutdown().await;
            assert!(provider.child.lock().await.try_wait().unwrap().is_some());
        }
    }

    #[tokio::test]
    async fn snapshots_reject_generation_changes_and_ignore_reordered_events() {
        let provider = fixture("simulation").await;
        provider
            .accept_snapshot(json!({"apiVersion":1,"generation":1,"sequence":3,"value":"new"}))
            .unwrap();
        provider
            .accept_snapshot(json!({"apiVersion":1,"generation":1,"sequence":2,"value":"old"}))
            .unwrap();
        assert_eq!(provider.snapshot.read().unwrap()["value"], json!("new"));
        assert!(provider
            .accept_snapshot(json!({"apiVersion":1,"generation":2,"sequence":4}))
            .is_err());
        provider.shutdown().await;
    }
}
