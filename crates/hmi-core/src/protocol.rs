pub use crate::schema::validate_schema;
use crate::{CoreError, CoreResult};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::AsyncWriteExt,
    process::{Child, Command},
    sync::{mpsc, oneshot, Mutex},
};
use tokio_util::codec::{FramedRead, LinesCodec};

pub const RPC_MAX_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: u32,
    pub executable: String,
    #[serde(default = "default_target")]
    pub target: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub config_schema: Value,
    #[serde(default)]
    pub commands: Vec<CommandDescriptor>,
    #[serde(default)]
    pub points: Vec<PointDescriptor>,
}
fn default_target() -> String {
    "x86_64-pc-windows-msvc".into()
}

/// Return the Rust target triple this core was compiled for.
///
/// Protocol plugins are native processes, so their manifest target must match
/// this value exactly before the executable can be used.
pub fn host_target() -> &'static str {
    #[cfg(all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"))]
    {
        "x86_64-pc-windows-msvc"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64", target_env = "msvc"),
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
    )))]
    {
        "unsupported"
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandDescriptor {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointDescriptor {
    pub key: String,
    #[serde(default)]
    pub label: String,
    #[serde(rename = "type")]
    pub value_type: String,
}

pub fn load_plugin_manifest(path: &Path) -> CoreResult<PluginManifest> {
    let metadata = std::fs::metadata(path)?;
    if metadata.len() > RPC_MAX_BYTES as u64 {
        return Err(CoreError::new("MANIFEST_SIZE", "插件清单超过 1 MiB"));
    }
    let manifest: PluginManifest = serde_json::from_slice(&std::fs::read(path)?)?;
    crate::schema::validate_schema_definition(&manifest.config_schema)?;
    for command in &manifest.commands {
        crate::schema::validate_schema_definition(&command.params)?;
    }
    if manifest.api_version != 1
        || !crate::safe_identifier(&manifest.id)
        || manifest.version.is_empty()
        || manifest.target != host_target()
        || !safe_relative_path(&manifest.executable)
    {
        return Err(CoreError::new(
            "MANIFEST_INVALID",
            "插件清单的版本、ID、架构或可执行路径无效",
        ));
    }
    let mut names = std::collections::HashSet::new();
    if manifest
        .commands
        .iter()
        .any(|c| c.name.is_empty() || !names.insert(&c.name))
    {
        return Err(CoreError::new("MANIFEST_INVALID", "插件命令名称为空或重复"));
    }
    let mut keys = std::collections::HashSet::new();
    if manifest.points.iter().any(|p| {
        p.key.is_empty()
            || !keys.insert(&p.key)
            || !["number", "string", "boolean", "enum"].contains(&p.value_type.as_str())
    }) {
        return Err(CoreError::new("MANIFEST_INVALID", "插件数据点无效"));
    }
    Ok(manifest)
}

pub fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with(['/', '\\'])
        && !path.contains(':')
        && !path.split(['/', '\\']).any(|p| p == ".." || p.is_empty())
}

#[cfg(test)]
mod target_tests {
    use super::{host_target, load_plugin_manifest};
    use serde_json::{json, Value};
    use std::fs;

    fn write_manifest(target: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("manifest.json");
        let mut manifest = json!({
            "id": "test-protocol",
            "name": "Test protocol",
            "version": "1.0.0",
            "apiVersion": 1,
            "executable": "bin/plugin",
            "configSchema": {"type": "object"},
            "commands": [],
            "points": []
        });
        if let Some(target) = target {
            manifest["target"] = Value::String(target.to_owned());
        }
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        (dir, path)
    }

    #[test]
    fn host_target_is_one_of_the_supported_native_process_targets() {
        assert!(matches!(
            host_target(),
            "x86_64-pc-windows-msvc" | "aarch64-apple-darwin" | "x86_64-apple-darwin"
        ));
    }

    #[test]
    fn manifest_target_must_exactly_match_the_host() {
        let (_dir, path) = write_manifest(Some(host_target()));
        assert!(load_plugin_manifest(&path).is_ok());

        for target in [
            "x86_64-pc-windows-msvc",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
        ] {
            if target == host_target() {
                continue;
            }
            let (_dir, path) = write_manifest(Some(target));
            assert!(
                load_plugin_manifest(&path).is_err(),
                "accepted foreign target {target}"
            );
        }
    }

    #[test]
    fn missing_target_keeps_the_legacy_windows_default_only_on_windows() {
        let (_dir, path) = write_manifest(None);
        if host_target() == "x86_64-pc-windows-msvc" {
            assert!(load_plugin_manifest(&path).is_ok());
        } else {
            assert!(load_plugin_manifest(&path).is_err());
        }
    }
}

pub fn discover_plugins(root: &Path) -> CoreResult<Vec<PluginManifest>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut plugins = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path().join("manifest.json");
        if path.is_file() {
            plugins.push(load_plugin_manifest(&path)?);
        }
    }
    plugins.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(plugins)
}

#[derive(Debug)]
pub(crate) struct PluginMessage {
    pub method: String,
    pub params: Value,
    pub id: Option<Value>,
}

pub(crate) struct RpcClient {
    next_id: AtomicU64,
    alive: AtomicBool,
    writer: mpsc::Sender<Value>,
    pending: Mutex<HashMap<u64, oneshot::Sender<CoreResult<Value>>>>,
    child: Mutex<Child>,
}

impl RpcClient {
    pub async fn spawn(
        executable: PathBuf,
        args: &[String],
        directory: &Path,
    ) -> CoreResult<(Arc<Self>, mpsc::Receiver<PluginMessage>)> {
        let mut command = Command::new(executable);
        command
            .args(args)
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command
            .spawn()
            .map_err(|e| CoreError::new("PLUGIN_START", e.to_string()))?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let (writer, mut outgoing) = mpsc::channel::<Value>(128);
        let (incoming, receiver) = mpsc::channel(256);
        let client = Arc::new(Self {
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            writer,
            pending: Mutex::new(HashMap::new()),
            child: Mutex::new(child),
        });
        let weak = Arc::downgrade(&client);
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let Ok(mut bytes) = serde_json::to_vec(&message) else {
                    break;
                };
                if bytes.len() > RPC_MAX_BYTES {
                    break;
                }
                bytes.push(b'\n');
                if stdin.write_all(&bytes).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            if let Some(client) = weak.upgrade() {
                client.fail_pending("插件输入管道已关闭").await;
            }
        });
        let weak = Arc::downgrade(&client);
        let events = incoming.clone();
        tokio::spawn(async move {
            let mut lines = FramedRead::new(stdout, LinesCodec::new_with_max_length(RPC_MAX_BYTES));
            while let Some(line) = lines.next().await {
                let value: Value = match line
                    .ok()
                    .and_then(|l| serde_json::from_str::<Value>(&l).ok())
                {
                    Some(v) if v.get("jsonrpc").and_then(Value::as_str) == Some("2.0") => v,
                    _ => break,
                };
                let Some(client) = weak.upgrade() else { break };
                if let Some(method) = value.get("method").and_then(Value::as_str) {
                    if events
                        .send(PluginMessage {
                            method: method.to_owned(),
                            params: value.get("params").cloned().unwrap_or(Value::Null),
                            id: value.get("id").cloned(),
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                } else if let Some(id) = value.get("id").and_then(Value::as_u64) {
                    if let Some(sender) = client.pending.lock().await.remove(&id) {
                        let result = if let Some(error) = value.get("error") {
                            Err(CoreError::new(
                                error
                                    .pointer("/data/code")
                                    .and_then(Value::as_str)
                                    .unwrap_or("PLUGIN_ERROR"),
                                error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .unwrap_or("协议插件执行错误"),
                            ))
                        } else if let Some(result) = value.get("result") {
                            Ok(result.clone())
                        } else {
                            Err(CoreError::new("PLUGIN_RPC", "插件返回缺少 result/error"))
                        };
                        let _ = sender.send(result);
                    }
                } else {
                    break;
                }
            }
            if let Some(client) = weak.upgrade() {
                client.fail_pending("插件退出或输出了无效 JSON-RPC").await;
            }
            let _ = events
                .send(PluginMessage {
                    method: "process.exited".into(),
                    params: json!({}),
                    id: None,
                })
                .await;
        });
        tokio::spawn(async move {
            let mut lines = FramedRead::new(stderr, LinesCodec::new_with_max_length(64 * 1024));
            while let Some(Ok(line)) = lines.next().await {
                if incoming
                    .send(PluginMessage {
                        method: "log".into(),
                        params: json!({"level":"info","message":line}),
                        id: None,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        Ok((client, receiver))
    }

    async fn fail_pending(&self, reason: &str) {
        self.alive.store(false, Ordering::Release);
        for (_, sender) in self.pending.lock().await.drain() {
            let _ = sender.send(Err(CoreError::new("PLUGIN_EXITED", reason)));
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub async fn call(&self, method: &str, params: Value, timeout: Duration) -> CoreResult<Value> {
        if !self.is_alive() {
            return Err(CoreError::new("PLUGIN_EXITED", "插件未运行"));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (send, receive) = oneshot::channel();
        self.pending.lock().await.insert(id, send);
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        if serde_json::to_vec(&request)?.len() > RPC_MAX_BYTES {
            self.pending.lock().await.remove(&id);
            return Err(CoreError::new("MESSAGE_LIMIT", "插件请求超过 1 MiB"));
        }
        let result = tokio::time::timeout(timeout, async {
            self.writer
                .send(request)
                .await
                .map_err(|_| CoreError::new("PLUGIN_EXITED", "插件写管道关闭"))?;
            receive
                .await
                .map_err(|_| CoreError::new("PLUGIN_EXITED", "插件响应通道关闭"))?
        })
        .await;
        self.pending.lock().await.remove(&id);
        result.unwrap_or_else(|_| {
            Err(CoreError::new(
                "PLUGIN_TIMEOUT",
                format!("协议调用 {method} 超时"),
            ))
        })
    }

    pub async fn notify(&self, method: &str, params: Value) -> CoreResult<()> {
        self.writer
            .send(json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
            .map_err(|_| CoreError::new("PLUGIN_EXITED", "插件输入管道关闭"))
    }

    pub async fn respond(&self, id: Value, result: CoreResult<Value>) {
        let message = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.message,"data":{"code":error.code}}})
            }
        };
        let _ = self.writer.send(message).await;
    }

    pub async fn shutdown(&self) {
        self.alive.store(false, Ordering::Release);
        self.fail_pending("插件已关闭").await;
        let mut child = self.child.lock().await;
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}
