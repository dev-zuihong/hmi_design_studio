//! Private, per-preview local RPC transport. A Studio owns the listener and child
//! process; the Runner connects back and receives only authenticated JSON frames.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    process::Stdio,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    sync::{broadcast, mpsc, oneshot, Mutex},
};
use tokio_util::sync::CancellationToken;

type Result<T> = std::result::Result<T, String>;
const MAX_FRAME_BYTES: usize = 32 * 1024 * 1024;
const MAX_RUNTIME_EVENT_BYTES: usize = 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Frame {
    Hello {
        token: String,
    },
    Request {
        id: u64,
        method: String,
        payload: Value,
    },
    Response {
        id: u64,
        result: Option<Value>,
        error: Option<String>,
    },
    Event {
        name: String,
        payload: Value,
    },
    Disconnect,
}

#[derive(Clone)]
pub struct RpcPeer {
    outgoing: mpsc::Sender<Frame>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value>>>>>,
    events: broadcast::Sender<(String, Value)>,
    next_id: Arc<AtomicU64>,
    closed: CancellationToken,
}

pub struct PreviewHostProcess {
    pub peer: Arc<RpcPeer>,
    child: Mutex<tokio::process::Child>,
    base_resources: PathBuf,
    resource_cache: std::sync::Mutex<HashMap<String, PathBuf>>,
    _resources: tempfile::TempDir,
}

impl PreviewHostProcess {
    pub async fn launch(
        runner: &Path,
        base_resources: &Path,
        workspace_root: &Path,
        project: &hmi_core::Project,
        revision: u64,
    ) -> Result<Arc<Self>> {
        Self::launch_inner(
            runner,
            base_resources,
            workspace_root,
            project,
            revision,
            false,
        )
        .await
    }

    #[cfg(feature = "native-preview-smoke")]
    pub async fn launch_disconnect_probe(
        runner: &Path,
        base_resources: &Path,
        workspace_root: &Path,
        project: &hmi_core::Project,
        revision: u64,
    ) -> Result<Arc<Self>> {
        Self::launch_inner(
            runner,
            base_resources,
            workspace_root,
            project,
            revision,
            true,
        )
        .await
    }

    async fn launch_inner(
        runner: &Path,
        base_resources: &Path,
        workspace_root: &Path,
        project: &hmi_core::Project,
        revision: u64,
        suppress_child_smoke: bool,
    ) -> Result<Arc<Self>> {
        let resources = tempfile::Builder::new()
            .prefix("hmi-preview-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        crate::resources::copy_tree(base_resources, resources.path())?;
        for directory in ["assets", "plugins", "python", "backends"] {
            let source = workspace_root.join(directory);
            if source.is_dir() {
                crate::resources::copy_tree(&source, &resources.path().join(directory))?;
            }
        }
        if crate::resources::has_python(project) {
            let target = hmi_core::host_target();
            let project_python = workspace_root
                .join("python")
                .join(target)
                .join("projects")
                .join(&project.id);
            let shared_project_python = base_resources
                .join("python")
                .join(target)
                .join("projects")
                .join(&project.id);
            let runtime_python = workspace_root
                .join("python")
                .join(target)
                .join(hmi_python::runtime_id());
            let shared_runtime_python = base_resources
                .join("python")
                .join(target)
                .join(hmi_python::runtime_id());
            let legacy_project_python = workspace_root.join("python/projects").join(&project.id);
            let legacy_shared_project_python =
                base_resources.join("python/projects").join(&project.id);
            let legacy_runtime_python =
                workspace_root.join("python").join(hmi_python::runtime_id());
            let legacy_shared_runtime_python =
                base_resources.join("python").join(hmi_python::runtime_id());
            let source = [
                project_python,
                shared_project_python,
                runtime_python,
                shared_runtime_python,
            ]
            .into_iter()
            .chain([
                legacy_project_python,
                legacy_shared_project_python,
                legacy_runtime_python,
                legacy_shared_runtime_python,
            ])
            .find(|path| path.is_dir())
            .ok_or("工程 Python 环境未导入")?;
            crate::resources::copy_tree(&source, &resources.path().join("runtime/python"))?;
        }
        let project_file = resources.path().join("project.json");
        std::fs::write(
            &project_file,
            serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let listener_directory = resources.path().join("ipc");
        let listener = local::Listener::bind(&listener_directory)?;
        #[cfg(unix)]
        let endpoint = listener.endpoint.to_string_lossy().into_owned();
        #[cfg(windows)]
        let endpoint = listener.endpoint().to_owned();
        let token = uuid::Uuid::new_v4().to_string();
        let mut command = tokio::process::Command::new(runner);
        command
            .arg("--preview-host")
            .arg("--project")
            .arg(&project_file)
            .env("HMI_PREVIEW_HOST", "1")
            .env("HMI_PREVIEW_ENDPOINT", &endpoint)
            .env("HMI_PREVIEW_TOKEN", &token)
            .env("HMI_PREVIEW_REVISION", revision.to_string())
            .env("HMI_RESOURCE_ROOT", resources.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        if suppress_child_smoke {
            command.env_remove("HMI_NATIVE_PREVIEW_CHILD_REPORT");
        }
        let child = command
            .spawn()
            .map_err(|e| format!("启动预览进程失败: {e}"))?;
        let stream = tokio::time::timeout(std::time::Duration::from_secs(20), listener.accept())
            .await
            .map_err(|_| "等待预览进程连接超时".to_string())??;
        let peer = serve(stream, &token, |_method, _payload| async {
            Err("预览 host 不接受客户端主动请求".to_string())
        })
        .await?;
        let host = Arc::new(Self {
            peer,
            child: Mutex::new(child),
            base_resources: base_resources.to_path_buf(),
            resource_cache: std::sync::Mutex::new(HashMap::new()),
            _resources: resources,
        });
        let _ = revision;
        Ok(host)
    }

    pub fn stage_revision_resources(
        &self,
        workspace_root: &Path,
        project: &hmi_core::Project,
        revision: u64,
    ) -> Result<PathBuf> {
        let key = resource_tree_key(workspace_root, project)?;
        let mut cache = self.resource_cache.lock().map_err(|e| e.to_string())?;
        if let Some(path) = cache.get(&key).cloned() {
            return Ok(path);
        }
        let revisions = self._resources.path().join("revisions");
        std::fs::create_dir_all(&revisions).map_err(|e| e.to_string())?;
        let destination = revisions.join(&key);
        let staging = tempfile::tempdir_in(&revisions).map_err(|e| e.to_string())?;
        crate::resources::copy_tree(&self.base_resources, staging.path())?;
        for directory in ["assets", "plugins", "python", "backends"] {
            let source = workspace_root.join(directory);
            if source.is_dir() {
                crate::resources::copy_tree(&source, &staging.path().join(directory))?;
            }
        }
        if crate::resources::has_python(project) {
            let target = hmi_core::host_target();
            let project_python = workspace_root
                .join("python")
                .join(target)
                .join("projects")
                .join(&project.id);
            let shared_runtime = workspace_root
                .join("python")
                .join(target)
                .join(hmi_python::runtime_id());
            let source = if project_python.is_dir() {
                project_python
            } else {
                shared_runtime
            };
            if !source.is_dir() {
                return Err("工程 Python 环境未导入".into());
            }
            crate::resources::copy_tree(&source, &staging.path().join("runtime/python"))?;
        }
        if destination.exists() {
            std::fs::remove_dir_all(&destination).map_err(|e| e.to_string())?;
        }
        std::fs::rename(staging.path(), &destination).map_err(|e| e.to_string())?;
        cache.insert(key, destination.clone());
        let _ = revision;
        Ok(destination)
    }

    pub async fn request(&self, method: &str, payload: Value) -> Result<Value> {
        self.peer.request(method, payload).await
    }

    pub async fn stop(&self) {
        let mut child = self.child.lock().await;
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let disconnected = self.peer.closed_token().is_cancelled();
        if !disconnected {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(25),
                self.peer.request("shutdown", serde_json::json!({})),
            )
            .await;
        }
        // The host already cleans up on IPC loss. Do not send an unanswerable
        // shutdown RPC, but retain the full grace period for device transactions.
        let grace = std::time::Duration::from_secs(if disconnected { 30 } else { 5 });
        if tokio::time::timeout(grace, child.wait()).await.is_err() {
            let _ = child.kill().await;
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
        }
    }

    pub async fn is_alive(&self) -> bool {
        self.child
            .lock()
            .await
            .try_wait()
            .map(|status| status.is_none())
            .unwrap_or(false)
    }

    pub async fn pid(&self) -> Option<u32> {
        self.child.lock().await.id()
    }
}

fn resource_tree_key(workspace_root: &Path, project: &hmi_core::Project) -> Result<String> {
    let mut hash = Sha256::new();
    let metadata = serde_json::to_vec(&(&project.id, &project.assets, &project.python, &project.backends))
        .map_err(|e| e.to_string())?;
    hash.update(metadata);
    let mut files = Vec::new();
    for directory in ["assets", "plugins", "python", "backends"] {
        let root = workspace_root.join(directory);
        if root.is_dir() {
            for entry in walkdir::WalkDir::new(&root).follow_links(false) {
                let entry = entry.map_err(|e| e.to_string())?;
                if entry.file_type().is_file() {
                    files.push((directory.to_owned(), entry.path().to_path_buf()));
                } else if entry.file_type().is_symlink() {
                    return Err("预览资源不能包含符号链接".into());
                }
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for (directory, path) in files {
        let relative = path
            .strip_prefix(workspace_root.join(&directory))
            .map_err(|e| e.to_string())?;
        let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
        hash.update(directory.as_bytes());
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified() {
            if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
                hash.update(duration.as_nanos().to_le_bytes());
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl RpcPeer {
    pub fn subscribe(&self) -> broadcast::Receiver<(String, Value)> {
        self.events.subscribe()
    }

    pub fn closed_token(&self) -> CancellationToken {
        self.closed.clone()
    }

    pub fn disconnect(&self) -> Result<()> {
        self.outgoing
            .try_send(Frame::Disconnect)
            .map_err(|_| "预览 IPC 发送队列已满".into())
    }

    pub async fn request(&self, method: &str, payload: Value) -> Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let frame = Frame::Request {
            id,
            method: method.into(),
            payload,
        };
        if serde_json::to_vec(&frame)
            .map(|bytes| bytes.len() > MAX_FRAME_BYTES)
            .unwrap_or(true)
        {
            self.pending.lock().await.remove(&id);
            return Err("预览 RPC 请求超过 32 MiB".into());
        }
        if self.outgoing.try_send(frame).is_err() {
            self.pending.lock().await.remove(&id);
            return Err("预览进程 IPC 已关闭".into());
        }
        match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(result) => result.map_err(|_| "预览进程 IPC 已关闭".to_string())?,
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err("预览进程 RPC 等待超时".into())
            }
        }
    }

    pub fn event(&self, name: impl Into<String>, payload: Value) -> Result<()> {
        let name = name.into();
        let frame = Frame::Event {
            name: name.clone(),
            payload,
        };
        let bytes = serde_json::to_vec(&frame).map_err(|e| e.to_string())?;
        let limit = if name == "runtime-event" {
            MAX_RUNTIME_EVENT_BYTES
        } else {
            MAX_FRAME_BYTES
        };
        if bytes.len() > limit {
            return Err(format!("预览 IPC 事件超过 {} MiB", limit / (1024 * 1024)));
        }
        self.outgoing
            .try_send(frame)
            .map_err(|_| "预览进程 IPC 已关闭或发送队列已满".into())
    }
}

type RpcHandler =
    Arc<dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<Value>> + Send>> + Send + Sync>;

fn peer<R, W>(reader: R, mut writer: W, handler: Option<RpcHandler>) -> RpcPeer
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (outgoing, mut queue) = mpsc::channel::<Frame>(256);
    let pending = Arc::new(Mutex::new(
        HashMap::<u64, oneshot::Sender<Result<Value>>>::new(),
    ));
    let events = broadcast::channel(1024).0;
    let closed = CancellationToken::new();
    let pending_reader = pending.clone();
    let events_reader = events.clone();
    let outgoing_reader = outgoing.clone();
    let closed_reader = closed.clone();
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader);
        loop {
            let line = match read_frame(&mut lines, MAX_FRAME_BYTES).await {
                Ok(Some(line)) => line,
                _ => break,
            };
            if line.len() > MAX_FRAME_BYTES {
                break;
            }
            let Ok(frame) = serde_json::from_slice::<Frame>(&line) else {
                continue;
            };
            match frame {
                Frame::Response { id, result, error } => {
                    if let Some(sender) = pending_reader.lock().await.remove(&id) {
                        let _ = sender
                            .send(error.map_or_else(|| Ok(result.unwrap_or(Value::Null)), Err));
                    }
                }
                Frame::Event { name, payload } => {
                    if name == "runtime-event" && line.len() > MAX_RUNTIME_EVENT_BYTES {
                        continue;
                    }
                    let _ = events_reader.send((name, payload));
                }
                Frame::Request {
                    id,
                    method,
                    payload,
                } => {
                    if let Some(handler) = handler.clone() {
                        let outgoing = outgoing_reader.clone();
                        tokio::spawn(async move {
                            let (result, error) = match handler(method, payload).await {
                                Ok(value) => (Some(value), None),
                                Err(error) => (None, Some(error)),
                            };
                            let _ = outgoing.send(Frame::Response { id, result, error }).await;
                        });
                    }
                }
                Frame::Disconnect => break,
                _ => {}
            }
        }
        let mut pending = pending_reader.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err("预览进程 IPC 连接已断开".into()));
        }
        closed_reader.cancel();
    });
    let closed_writer = closed.clone();
    tokio::spawn(async move {
        while let Some(frame) = queue.recv().await {
            if matches!(frame, Frame::Disconnect) {
                break;
            }
            let Ok(mut bytes) = serde_json::to_vec(&frame) else {
                continue;
            };
            bytes.push(b'\n');
            if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
        closed_writer.cancel();
    });
    RpcPeer {
        outgoing,
        pending,
        events,
        next_id: Arc::new(AtomicU64::new(1)),
        closed,
    }
}

/// Serve a connected host. Each RPC is handled independently and responses are
/// multiplexed with runtime events over the same authenticated local stream.
pub async fn serve<S, F, Fut>(stream: S, expected_token: &str, handler: F) -> Result<Arc<RpcPeer>>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<Value>> + Send + 'static,
{
    let (read, write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read);
    let line = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        read_frame(&mut lines, MAX_FRAME_BYTES),
    )
    .await
    .map_err(|_| "预览进程握手超时".to_string())?
    .map_err(|e| e.to_string())?
    .ok_or("预览进程提前关闭 IPC")?;
    match serde_json::from_slice::<Frame>(&line).map_err(|e| e.to_string())? {
        Frame::Hello { token } if constant_time_eq(token.as_bytes(), expected_token.as_bytes()) => {
        }
        _ => return Err("预览进程认证失败".into()),
    }
    // Reconstitute the buffered reader as a stream by keeping the line loop here.
    let (outgoing, mut queue) = mpsc::channel::<Frame>(256);
    let pending = Arc::new(Mutex::new(
        HashMap::<u64, oneshot::Sender<Result<Value>>>::new(),
    ));
    let events = broadcast::channel(1024).0;
    let event_clone = events.clone();
    let closed = CancellationToken::new();
    let closed_writer = closed.clone();
    tokio::spawn(async move {
        let mut writer = tokio::io::BufWriter::new(write);
        while let Some(frame) = queue.recv().await {
            if matches!(frame, Frame::Disconnect) {
                break;
            }
            let Ok(mut bytes) = serde_json::to_vec(&frame) else {
                continue;
            };
            if bytes.len() > MAX_FRAME_BYTES {
                continue;
            }
            bytes.push(b'\n');
            if writer.write_all(&bytes).await.is_err() || writer.flush().await.is_err() {
                break;
            }
        }
        let _ = writer.shutdown().await;
        closed_writer.cancel();
    });
    let peer = Arc::new(RpcPeer {
        outgoing,
        pending: pending.clone(),
        events,
        next_id: Arc::new(AtomicU64::new(1)),
        closed: closed.clone(),
    });
    let handler = Arc::new(handler);
    let response_peer = peer.clone();
    let pending_reader = pending.clone();
    let closed_reader = closed;
    tokio::spawn(async move {
        loop {
            let line = match read_frame(&mut lines, MAX_FRAME_BYTES).await {
                Ok(Some(line)) => line,
                _ => break,
            };
            if line.len() > MAX_FRAME_BYTES {
                break;
            }
            let Ok(frame) = serde_json::from_slice::<Frame>(&line) else {
                continue;
            };
            match frame {
                Frame::Request {
                    id,
                    method,
                    payload,
                } => {
                    let handler = handler.clone();
                    let peer = response_peer.clone();
                    tokio::spawn(async move {
                        let (result, error) = match handler(method, payload).await {
                            Ok(value) => (Some(value), None),
                            Err(error) => (None, Some(error)),
                        };
                        let _ = peer
                            .outgoing
                            .send(Frame::Response { id, result, error })
                            .await;
                    });
                }
                Frame::Response { id, result, error } => {
                    if let Some(sender) = pending_reader.lock().await.remove(&id) {
                        let _ = sender
                            .send(error.map_or_else(|| Ok(result.unwrap_or(Value::Null)), Err));
                    }
                }
                Frame::Event { name, payload } => {
                    if name == "runtime-event" && line.len() > MAX_RUNTIME_EVENT_BYTES {
                        continue;
                    }
                    let _ = event_clone.send((name, payload));
                }
                Frame::Disconnect => break,
                _ => {}
            }
        }
        closed_reader.cancel();
    });
    Ok(peer)
}

async fn read_frame<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Ok(Some(frame))
            };
        }
        let count = available
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1)
            .unwrap_or(available.len());
        if frame.len().saturating_add(count) > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "IPC frame exceeds limit",
            ));
        }
        let complete = available.get(count.saturating_sub(1)) == Some(&b'\n');
        frame.extend_from_slice(&available[..count]);
        reader.consume(count);
        if complete {
            frame.pop();
            return Ok(Some(frame));
        }
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(unix)]
pub mod local {
    use super::*;
    use std::path::PathBuf;
    use tokio::net::{UnixListener, UnixStream};

    pub struct Listener {
        listener: UnixListener,
        pub endpoint: PathBuf,
    }
    impl Listener {
        pub fn bind(directory: &std::path::Path) -> Result<Self> {
            std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
            let endpoint = directory.join("preview.sock");
            let _ = std::fs::remove_file(&endpoint);
            let listener = UnixListener::bind(&endpoint).map_err(|e| e.to_string())?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&endpoint, std::fs::Permissions::from_mode(0o600))
                    .map_err(|e| e.to_string())?;
            }
            Ok(Self { listener, endpoint })
        }
        pub async fn accept(&self) -> Result<UnixStream> {
            let (stream, _) = self.listener.accept().await.map_err(|e| e.to_string())?;
            Ok(stream)
        }
    }

    pub async fn connect(endpoint: &std::path::Path, token: &str) -> Result<RpcPeer> {
        let stream = UnixStream::connect(endpoint)
            .await
            .map_err(|e| e.to_string())?;
        let (reader, mut writer) = stream.into_split();
        let hello = serde_json::to_vec(&Frame::Hello {
            token: token.into(),
        })
        .map_err(|e| e.to_string())?;
        writer.write_all(&hello).await.map_err(|e| e.to_string())?;
        writer.write_all(b"\n").await.map_err(|e| e.to_string())?;
        Ok(peer(reader, writer, None))
    }

    pub async fn connect_with_handler<F, Fut>(
        endpoint: &std::path::Path,
        token: &str,
        handler: F,
    ) -> Result<RpcPeer>
    where
        F: Fn(String, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let stream = UnixStream::connect(endpoint)
            .await
            .map_err(|e| e.to_string())?;
        let (reader, mut writer) = stream.into_split();
        let hello = serde_json::to_vec(&Frame::Hello {
            token: token.into(),
        })
        .map_err(|e| e.to_string())?;
        writer.write_all(&hello).await.map_err(|e| e.to_string())?;
        writer.write_all(b"\n").await.map_err(|e| e.to_string())?;
        let handler: RpcHandler =
            Arc::new(move |method, payload| Box::pin(handler(method, payload)));
        Ok(peer(reader, writer, Some(handler)))
    }
}

#[cfg(windows)]
pub mod local {
    use super::*;
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions};

    pub struct Listener {
        pipe_name: String,
    }
    impl Listener {
        pub fn bind(_directory: &std::path::Path) -> Result<Self> {
            Ok(Self {
                pipe_name: format!(r"\\.\pipe\hmi-preview-{}", uuid::Uuid::new_v4()),
            })
        }
        pub fn endpoint(&self) -> &str {
            &self.pipe_name
        }
        pub async fn accept(&self) -> Result<tokio::net::windows::named_pipe::NamedPipeServer> {
            let server = ServerOptions::new()
                .first_pipe_instance(true)
                .create(&self.pipe_name)
                .map_err(|e| e.to_string())?;
            server.connect().await.map_err(|e| e.to_string())?;
            Ok(server)
        }
    }

    pub async fn connect(endpoint: &str, token: &str) -> Result<RpcPeer> {
        let mut stream = ClientOptions::new()
            .open(endpoint)
            .map_err(|e| e.to_string())?;
        let hello = serde_json::to_vec(&Frame::Hello {
            token: token.into(),
        })
        .map_err(|e| e.to_string())?;
        stream.write_all(&hello).await.map_err(|e| e.to_string())?;
        stream.write_all(b"\n").await.map_err(|e| e.to_string())?;
        let (reader, writer) = tokio::io::split(stream);
        Ok(peer(reader, writer, None))
    }

    pub async fn connect_with_handler<F, Fut>(
        endpoint: &str,
        token: &str,
        handler: F,
    ) -> Result<RpcPeer>
    where
        F: Fn(String, Value) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Value>> + Send + 'static,
    {
        let mut stream = ClientOptions::new()
            .open(endpoint)
            .map_err(|e| e.to_string())?;
        let hello = serde_json::to_vec(&Frame::Hello {
            token: token.into(),
        })
        .map_err(|e| e.to_string())?;
        stream.write_all(&hello).await.map_err(|e| e.to_string())?;
        stream.write_all(b"\n").await.map_err(|e| e.to_string())?;
        let (reader, writer) = tokio::io::split(stream);
        let handler: RpcHandler =
            Arc::new(move |method, payload| Box::pin(handler(method, payload)));
        Ok(peer(reader, writer, Some(handler)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnect_frame_shutdown_notifies_remote_peer() {
        let (left, right) = tokio::io::duplex(4096);
        let (left_read, left_write) = tokio::io::split(left);
        let (right_read, right_write) = tokio::io::split(right);
        let local = peer(left_read, left_write, None);
        let remote = peer(right_read, right_write, None);
        let remote_closed = remote.closed_token();

        local.disconnect().unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(1), remote_closed.cancelled())
            .await
            .expect("remote peer must detect closed IPC");
    }
}
