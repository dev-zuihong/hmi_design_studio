use crate::resources::*;
use async_trait::async_trait;
use hmi_core::backend::{BackendManager, BackendResource};
use hmi_core::{
    CodeModule, ConnectionManager, CoreError, CoreResult, HostServices, Project, Runtime,
    RuntimeEvent, ValidationIssue,
};
use hmi_python::{PythonExecutor, PythonLimits};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct HostActionRequest {
    pub kind: String,
    pub args: Value,
    pub context: Value,
    pub reply: tokio::sync::oneshot::Sender<CoreResult<Value>>,
}

pub struct Session {
    pub runtime: Arc<Runtime>,
    connections: Arc<ConnectionManager>,
    services: Arc<Services>,
    resources: tokio::sync::RwLock<Resources>,
    asset_root: tokio::sync::RwLock<PathBuf>,
    backend_identity: tokio::sync::RwLock<BTreeMap<String, String>>,
    backend_bridge: tokio::sync::Mutex<Option<BackendBridge>>,
    shutdown_requested: CancellationToken,
    pub cancel: CancellationToken,
    bridge_cancel: CancellationToken,
    bridge: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    forwarders: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    stopping: tokio::sync::Mutex<()>,
}
struct BackendBridge {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}
struct RevisionBarrier(Arc<Runtime>);
impl Drop for RevisionBarrier {
    fn drop(&mut self) {
        self.0.finish_revision_barrier();
    }
}
struct Services {
    connections: Arc<ConnectionManager>,
    python: tokio::sync::RwLock<Option<PythonExecutor>>,
    runtime: Mutex<Weak<Runtime>>,
    backends: tokio::sync::RwLock<Arc<BackendManager>>,
    flow_services: Arc<crate::flow_services::FlowServices>,
    host_actions: tokio::sync::RwLock<Option<tokio::sync::mpsc::Sender<HostActionRequest>>>,
}
#[async_trait]
impl HostServices for Services {
    async fn backend_invoke(
        &self,
        id: &str,
        operation: &str,
        args: Value,
        context: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        self.backends
            .read()
            .await
            .clone()
            .invoke(id, operation, args, context, cancel)
            .await
    }
    async fn host_action(
        &self,
        kind: &str,
        args: Value,
        context: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        if crate::flow_services::FlowServices::supports(kind) {
            return self.flow_services.action(kind, args, cancel).await;
        }
        let sender = self
            .host_actions
            .read()
            .await
            .clone()
            .ok_or_else(|| CoreError::new("HOST_UNAVAILABLE", "此操作需要桌面窗口"))?;
        let (reply, response) = tokio::sync::oneshot::channel();
        tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(CoreError::cancelled()),
            result = sender.send(HostActionRequest {kind:kind.into(), args, context, reply}) => result.map_err(|_|CoreError::new("HOST_CLOSED", "桌面会话已关闭"))?,
        }
        tokio::select! {
            result = response => result.map_err(|_|CoreError::new("HOST_CLOSED", "桌面操作已终止"))?,
            _ = cancel.cancelled() => Err(CoreError::new("CANCELLED", "桌面操作已取消")),
        }
    }
    async fn open_connection(&self, id: &str) -> CoreResult<()> {
        self.connections.open(id).await
    }
    async fn close_connection(&self, id: &str) -> CoreResult<()> {
        self.connections.close(id).await
    }
    async fn command(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        self.connections.command(id, name, params, cancel).await
    }
    async fn command_with_origin(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
        origin_flow_id: &str,
    ) -> CoreResult<Value> {
        self.connections
            .command_with_origin(id, name, params, cancel, Some(origin_flow_id))
            .await
    }
    async fn python(
        &self,
        source: &str,
        msg: Value,
        timeout_ms: u64,
        memory_mb: u64,
        cancel: CancellationToken,
    ) -> CoreResult<Option<Value>> {
        self.python_with_modules(source, msg, &[], timeout_ms, memory_mb, cancel)
            .await
    }
    async fn python_with_modules(
        &self,
        source: &str,
        mut msg: Value,
        modules: &[CodeModule],
        timeout_ms: u64,
        memory_mb: u64,
        cancel: CancellationToken,
    ) -> CoreResult<Option<Value>> {
        let executor = self
            .python
            .read()
            .await
            .clone()
            .ok_or_else(|| CoreError::new("PYTHON_MISSING", "未加载项目 Python 环境"))?;
        msg["host"] = self.flow_services.python_host();
        let mut result = executor
            .execute_with_modules(
                source,
                msg,
                serde_json::to_value(modules)?,
                PythonLimits {
                    timeout: Duration::from_millis(timeout_ms),
                    memory_bytes: (memory_mb * 1024 * 1024) as usize,
                },
                cancel,
            )
            .await
            .map_err(|e| CoreError::new("PYTHON", e.to_string()))?;
        if let Some(Value::Object(message)) = result.message.as_mut() {
            message.remove("host");
        }
        if !result.logs.is_empty() {
            if let Some(runtime) = self.runtime.lock().unwrap().upgrade() {
                runtime.publish_event(RuntimeEvent::new(
                    "log",
                    json!({"level":"info","source":"python","message":result.logs}),
                ));
            }
        }
        Ok(result.message)
    }
}
impl Session {
    pub async fn new(project: Project, resources: Resources) -> Result<Arc<Self>> {
        Self::new_with_assets(project, resources, None).await
    }
    pub async fn new_with_assets(
        project: Project,
        resources: Resources,
        asset_root: Option<PathBuf>,
    ) -> Result<Arc<Self>> {
        let python = check_python(&resources, &project).await?;
        let connections = ConnectionManager::new(project.connections.clone(), resources.plugins());
        let asset_resources = Resources {
            root: asset_root.unwrap_or_else(|| resources.root.clone()),
            runner: resources.runner,
        };
        let resource_map = backend_resources(&asset_resources, &project)?;
        let identity = backend_identity(&resources, &project, &resource_map)?;
        let flow_services = crate::flow_services::FlowServices::new(
            &project.id,
            project.transports.clone(),
            resource_map.clone(),
        )?;
        let backends =
            BackendManager::new(project.backends.clone(), resources.backends(), resource_map);
        let host = Arc::new(Services {
            connections: connections.clone(),
            python: tokio::sync::RwLock::new(python),
            runtime: Mutex::new(Weak::new()),
            backends: tokio::sync::RwLock::new(backends.clone()),
            flow_services: flow_services.clone(),
            host_actions: tokio::sync::RwLock::new(None),
        });
        let runtime = Runtime::new(project, host.clone()).map_err(error)?;
        *host.runtime.lock().unwrap() = Arc::downgrade(&runtime);
        flow_services.set_runtime(Arc::downgrade(&runtime));
        let bridge_cancel = CancellationToken::new();
        let mut events = connections.subscribe();
        let cancel = bridge_cancel.clone();
        let weak = Arc::downgrade(&runtime);
        let bridge = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _ = cancel.cancelled() => {
                        loop { match events.try_recv() {
                            Ok(event) => { if let Some(runtime) = weak.upgrade() { forward_connection_event(&runtime, event).await; } },
                            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                                if let Some(runtime) = weak.upgrade() { runtime.publish_event(RuntimeEvent::new("log", json!({"level":"error","message":format!("连接事件队列溢出，丢失 {count} 条事件")}))); }
                            },
                            Err(_) => break,
                        }}
                        break;
                    },
                    event = events.recv() => match event {
                        Ok(event) => {
                            let Some(runtime) = weak.upgrade() else { break };
                            forward_connection_event(&runtime, event).await;
                        },
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                            if let Some(runtime) = weak.upgrade() { runtime.publish_event(RuntimeEvent::new("log", json!({"level":"error","message":format!("连接事件队列溢出，丢失 {count} 条事件")}))); }
                        },
                        Err(_) => break,
                    }
                }
            }
        });
        let backend_bridge = spawn_backend_bridge(&runtime, &backends);
        if let Err(reason) = backends.start_all().await {
            backends.shutdown().await;
            bridge_cancel.cancel();
            backend_bridge.cancel.cancel();
            let _ = bridge.await;
            let _ = backend_bridge.task.await;
            return Err(error(reason));
        }
        hydrate_backends(&runtime, &backends).await;
        Ok(Arc::new(Self {
            runtime,
            connections,
            services: host,
            resources: tokio::sync::RwLock::new(resources),
            asset_root: tokio::sync::RwLock::new(asset_resources.root),
            backend_identity: tokio::sync::RwLock::new(identity),
            backend_bridge: tokio::sync::Mutex::new(Some(backend_bridge)),
            shutdown_requested: CancellationToken::new(),
            cancel: CancellationToken::new(),
            bridge_cancel,
            bridge: tokio::sync::Mutex::new(Some(bridge)),
            forwarders: tokio::sync::Mutex::new(Vec::new()),
            stopping: tokio::sync::Mutex::new(()),
        }))
    }
    pub async fn add_event_forwarder(&self, task: tokio::task::JoinHandle<()>) {
        self.forwarders.lock().await.push(task);
    }
    pub async fn attach_host(&self) -> tokio::sync::mpsc::Receiver<HostActionRequest> {
        let (sender, receiver) = tokio::sync::mpsc::channel(32);
        *self.services.host_actions.write().await = Some(sender);
        receiver
    }
    pub async fn backend_manager(&self) -> Arc<BackendManager> {
        self.services.backends.read().await.clone()
    }
    pub fn flow_services(&self) -> Arc<crate::flow_services::FlowServices> {
        self.services.flow_services.clone()
    }
    pub fn file_resource(&self, id: &str) -> CoreResult<BackendResource> {
        self.services.flow_services.resource(id)
    }
    pub async fn register_file_resource(
        &self,
        id: String,
        resource: BackendResource,
    ) -> CoreResult<()> {
        self.services
            .flow_services
            .register_resource(id.clone(), resource.clone())?;
        self.backend_manager().await.register_resource(id, resource)
    }

    pub async fn connection_manager(&self) -> Arc<ConnectionManager> {
        self.connections.clone()
    }

    pub async fn current_resources(&self) -> Resources {
        self.resources.read().await.clone()
    }

    pub fn requires_resource_barrier(&self, project: &Project) -> bool {
        let previous = self.runtime.project();
        let old_connections = serde_json::to_value(&previous.connections).ok();
        let new_connections = serde_json::to_value(&project.connections).ok();
        let old_python = serde_json::to_value(&previous.python).ok();
        let new_python = serde_json::to_value(&project.python).ok();
        let variable_changed = previous.variables.iter().any(|old| {
            project
                .variables
                .iter()
                .find(|new| new.id == old.id)
                .is_none_or(|new| new.value_type != old.value_type || new.options != old.options)
        });
        old_connections != new_connections
            || serde_json::to_value(&previous.transports).ok()
                != serde_json::to_value(&project.transports).ok()
            || old_python != new_python
            || variable_changed
            || serde_json::to_value(&previous.backends).ok()
                != serde_json::to_value(&project.backends).ok()
            || serde_json::to_value(&previous.assets).ok()
                != serde_json::to_value(&project.assets).ok()
    }

    pub async fn resource_busy(&self) -> bool {
        self.runtime.active_run_count() > 0
            || self.backend_manager().await.is_busy().await
            || self.services.flow_services.transports.is_busy()
    }

    pub async fn requires_resource_barrier_with_resources(
        &self,
        project: &Project,
        resources: &Resources,
    ) -> Result<bool> {
        if self.requires_resource_barrier(project) {
            return Ok(true);
        }
        let current = self.resources.read().await;
        let root = if current.root == resources.root {
            self.asset_root.read().await.clone()
        } else {
            resources.root.clone()
        };
        let assets = backend_resources(
            &Resources {
                root,
                runner: resources.runner,
            },
            project,
        )?;
        Ok(*self.backend_identity.read().await != backend_identity(resources, project, &assets)?)
    }

    pub async fn wait_for_resource_idle(&self) {
        loop {
            if self.shutdown_requested.is_cancelled() || !self.resource_busy().await {
                return;
            }
            tokio::select! {
                _ = self.shutdown_requested.cancelled() => return,
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    }

    pub async fn apply_revision(
        self: &Arc<Self>,
        project: Project,
        resources: Resources,
    ) -> Result<&'static str> {
        // Finish the transaction and rollback even if the IPC caller disappears.
        let session = self.clone();
        tokio::spawn(async move { session.apply_revision_inner(project, resources).await })
            .await
            .map_err(error)?
    }

    async fn apply_revision_inner(
        &self,
        project: Project,
        resources: Resources,
    ) -> Result<&'static str> {
        let _maintenance = self.stopping.lock().await;
        let _barrier = RevisionBarrier(self.runtime.clone());
        if self.shutdown_requested.is_cancelled() {
            return Err("运行会话正在关闭".into());
        }
        let issues = validate(&resources, &project).await;
        if let Some(issue) = issues.iter().find(|issue| issue.severity == "error") {
            return Err(format!("{}: {}", issue.path, issue.message));
        }
        let new_python = check_python(&resources, &project).await?;
        let old_project = self.runtime.project();
        let old_resources = self.resources.read().await.clone();
        let old_asset_root = self.asset_root.read().await.clone();
        let asset_root = if old_resources.root == resources.root {
            old_asset_root
        } else {
            resources.root.clone()
        };
        let assets = Resources {
            root: asset_root.clone(),
            runner: resources.runner,
        };
        let mut next_resource_map = backend_resources(&assets, &project)?;
        let identity = backend_identity(&resources, &project, &next_resource_map)?;
        let backend_change = *self.backend_identity.read().await != identity;
        let old_manager = self.backend_manager().await;
        // Runtime file picker resources are immutable session data. Preserve them
        // across a provider restart; project-owned IDs are replaced deliberately.
        for (id, resource) in old_manager.resource_map() {
            if !old_project.assets.iter().any(|asset| asset.id == id) {
                next_resource_map.entry(id).or_insert(resource);
            }
        }
        for (id, resource) in self.services.flow_services.resource_map() {
            if !old_project.assets.iter().any(|asset| asset.id == id) {
                next_resource_map.entry(id).or_insert(resource);
            }
        }
        let transport_change = serde_json::to_value(&old_project.transports).map_err(error)?
            != serde_json::to_value(&project.transports).map_err(error)?;
        let connection_change = serde_json::to_value(&old_project.connections).map_err(error)?
            != serde_json::to_value(&project.connections).map_err(error)?;
        let python_change = serde_json::to_value(&old_project.python).map_err(error)?
            != serde_json::to_value(&project.python).map_err(error)?;
        let variable_change = old_project.variables.iter().any(|old| {
            project
                .variables
                .iter()
                .find(|new| new.id == old.id)
                .is_none_or(|new| new.value_type != old.value_type || new.options != old.options)
        });
        if !connection_change
            && !python_change
            && !variable_change
            && !backend_change
            && !transport_change
        {
            self.connections
                .reconfigure(project.connections.clone(), resources.plugins())
                .await
                .map_err(error)?;
            self.runtime
                .apply_project_revision(project)
                .map_err(error)?;
            *self.services.python.write().await = new_python;
            *self.resources.write().await = resources;
            *self.asset_root.write().await = asset_root;
            return Ok("applied");
        }

        self.runtime.begin_revision_barrier();
        self.wait_for_resource_idle().await;
        if self.shutdown_requested.is_cancelled() {
            return Err("资源更新已取消：会话正在关闭".into());
        }
        let old_snapshot = self.runtime.snapshot().await;
        let old_open: Vec<String> = old_snapshot
            .connections
            .iter()
            .filter(|(_, state)| state.get("status").and_then(Value::as_str) == Some("open"))
            .map(|(id, _)| id.clone())
            .collect();
        let changed = self
            .connections
            .reconfigure(project.connections.clone(), resources.plugins())
            .await
            .map_err(error)?;
        for id in changed {
            if old_open.contains(&id)
                && project
                    .connections
                    .iter()
                    .any(|connection| connection.id == id)
            {
                if let Err(reason) = self.connections.open(&id).await {
                    let rollback = self
                        .restore_connections(&old_project, &old_resources, &old_open)
                        .await;
                    return Err(format!("连接资源切换失败: {reason}; {rollback}"));
                }
            }
        }

        let replacement = if backend_change {
            old_manager.shutdown().await;
            self.stop_backend_bridge().await;
            for backend in &old_project.backends {
                self.runtime.reset_backend(&backend.id);
            }
            let next = BackendManager::new(
                project.backends.clone(),
                resources.backends(),
                next_resource_map.clone(),
            );
            if let Err(reason) = next.start_all().await {
                next.shutdown().await;
                let backend_rollback = self
                    .restore_backends(&old_project, old_manager.clone())
                    .await;
                let connections_rollback = self
                    .restore_connections(&old_project, &old_resources, &old_open)
                    .await;
                return Err(format!(
                    "后端切换失败: {reason}; {backend_rollback}; {connections_rollback}"
                ));
            }
            Some(next)
        } else {
            None
        };

        if let Err(reason) = self.runtime.apply_project_revision(project.clone()) {
            if let Some(next) = &replacement {
                next.shutdown().await;
            }
            let backend_rollback = if backend_change {
                self.restore_backends(&old_project, old_manager).await
            } else {
                "后端未变更".into()
            };
            let connections_rollback = self
                .restore_connections(&old_project, &old_resources, &old_open)
                .await;
            return Err(format!(
                "工程切换失败: {reason}; {backend_rollback}; {connections_rollback}"
            ));
        }
        if let Some(next) = replacement {
            for backend in &project.backends {
                self.runtime.reset_backend(&backend.id);
            }
            *self.services.backends.write().await = next.clone();
            *self.backend_bridge.lock().await = Some(spawn_backend_bridge(&self.runtime, &next));
            hydrate_backends(&self.runtime, &next).await;
        }
        // Switch only after all fallible revision steps succeeded.
        if transport_change {
            self.services
                .flow_services
                .transports
                .reconfigure(project.transports.clone())
                .await;
        }
        self.services
            .flow_services
            .replace_resources(next_resource_map);
        *self.services.python.write().await = new_python;
        *self.resources.write().await = resources;
        *self.asset_root.write().await = asset_root;
        *self.backend_identity.write().await = identity;
        Ok("applied")
    }

    async fn restore_connections(
        &self,
        project: &Project,
        resources: &Resources,
        open: &[String],
    ) -> String {
        let mut failures = Vec::new();
        if let Err(reason) = self
            .connections
            .reconfigure(project.connections.clone(), resources.plugins())
            .await
        {
            failures.push(reason.to_string());
        }
        for id in open {
            if let Err(reason) = self.connections.open(id).await {
                failures.push(format!("{id}: {reason}"));
            }
        }
        if failures.is_empty() {
            "串口资源已恢复".into()
        } else {
            format!("串口恢复失败: {}", failures.join("; "))
        }
    }

    async fn restore_backends(&self, project: &Project, manager: Arc<BackendManager>) -> String {
        for backend in &project.backends {
            self.runtime.reset_backend(&backend.id);
        }
        let result = manager.start_all().await;
        *self.services.backends.write().await = manager.clone();
        *self.backend_bridge.lock().await = Some(spawn_backend_bridge(&self.runtime, &manager));
        hydrate_backends(&self.runtime, &manager).await;
        match result {
            Ok(()) => "后端已恢复".into(),
            Err(reason) => format!("后端恢复失败: {reason}"),
        }
    }

    async fn stop_backend_bridge(&self) {
        if let Some(bridge) = self.backend_bridge.lock().await.take() {
            bridge.cancel.cancel();
            let _ = bridge.task.await;
        }
    }
    pub async fn stop(&self) {
        self.shutdown_requested.cancel();
        let _stopping = self.stopping.lock().await;
        self.runtime.stop().await;
        self.services.flow_services.transports.shutdown().await;
        self.services.backends.read().await.clone().shutdown().await;
        self.stop_backend_bridge().await;
        self.services.host_actions.write().await.take();
        let connections = self.connection_manager().await;
        for connection in connections.configs() {
            let _ = connections.close(&connection.id).await;
        }
        self.bridge_cancel.cancel();
        let bridge = self.bridge.lock().await.take();
        if let Some(bridge) = bridge {
            let _ = bridge.await;
        }
        // Connection.closed is now in the runtime channel. Let consumers drain it
        // and all run terminal events before returning from session shutdown.
        self.cancel.cancel();
        let forwarders = std::mem::take(&mut *self.forwarders.lock().await);
        for task in forwarders {
            let _ = task.await;
        }
    }
}

fn spawn_backend_bridge(runtime: &Arc<Runtime>, manager: &Arc<BackendManager>) -> BackendBridge {
    let mut events = manager.subscribe();
    let cancel = CancellationToken::new();
    let stopping = cancel.clone();
    let runtime = Arc::downgrade(runtime);
    let manager = Arc::downgrade(manager);
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = stopping.cancelled() => {
                    while let Ok(event) = events.try_recv() {
                        if let Some(runtime) = runtime.upgrade() { runtime.update_backend(event); }
                    }
                    break;
                },
                event = events.recv() => match event {
                    Ok(event) => if let Some(runtime) = runtime.upgrade() { runtime.update_backend(event); } else { break; },
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        if let (Some(runtime), Some(manager)) = (runtime.upgrade(), manager.upgrade()) {
                            runtime.publish_event(RuntimeEvent::new("log", json!({"level":"warn","message":format!("后端事件丢失 {count} 条，已恢复最新快照")})));
                            hydrate_backends(&runtime, &manager).await;
                        } else { break; }
                    },
                    Err(_) => break,
                }
            }
        }
    });
    BackendBridge { cancel, task }
}

async fn hydrate_backends(runtime: &Arc<Runtime>, manager: &Arc<BackendManager>) {
    for (id, data) in manager.snapshots().await {
        let (Some(generation), Some(sequence)) =
            (data["generation"].as_u64(), data["sequence"].as_u64())
        else {
            continue;
        };
        runtime.update_backend(hmi_core::BackendEvent {
            backend_id: id,
            kind: "snapshot".into(),
            generation,
            sequence,
            device_key: None,
            job_id: None,
            data,
        });
    }
}

fn backend_identity(
    resources: &Resources,
    project: &Project,
    assets: &BTreeMap<String, BackendResource>,
) -> Result<BTreeMap<String, String>> {
    let mut identity = BTreeMap::new();
    if project.backends.is_empty() {
        return Ok(identity);
    }
    identity.insert(
        "project.backends".into(),
        serde_json::to_string(&project.backends).map_err(error)?,
    );
    for backend in &project.backends {
        if !hmi_core::safe_identifier(&backend.provider_id) {
            return Err("后端提供程序 ID 无效".into());
        }
        let directory = resources.backends().join(&backend.provider_id);
        for entry in walkdir::WalkDir::new(&directory).follow_links(false) {
            let entry = entry.map_err(error)?;
            reject_reparse(entry.path())?;
            if entry.file_type().is_file() {
                let relative = entry
                    .path()
                    .strip_prefix(&directory)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/");
                identity.insert(
                    format!("provider/{}/{relative}", backend.provider_id),
                    sha256(entry.path())?,
                );
            }
        }
    }
    for (id, resource) in assets {
        identity.insert(
            format!("asset/{id}"),
            format!(
                "{}:{}:{}",
                resource.sha256.as_deref().unwrap_or(""),
                resource.size.unwrap_or(0),
                resource.name.as_deref().unwrap_or("")
            ),
        );
    }
    Ok(identity)
}
pub fn backend_resources(
    resources: &Resources,
    project: &Project,
) -> Result<BTreeMap<String, BackendResource>> {
    let mut output = BTreeMap::new();
    for asset in &project.assets {
        let path = contained_file(&resources.root, &asset.path)?;
        output.insert(
            asset.id.clone(),
            BackendResource {
                sha256: Some(sha256(&path)?),
                size: Some(std::fs::metadata(&path).map_err(error)?.len()),
                name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned()),
                path,
            },
        );
    }
    Ok(output)
}
pub fn check_backend(directory: &Path) -> Result<()> {
    let manifest = hmi_core::backend::load_backend_manifest(&directory.join("manifest.json"))
        .map_err(error)?;
    let executable = contained_file(directory, &manifest.executable)?;
    check_native_binary(&executable)
}
async fn forward_connection_event(runtime: &Arc<Runtime>, event: RuntimeEvent) {
    if event.kind == "point.changed" {
        let connection = event.data["connectionId"].as_str().unwrap_or("");
        let key = event.data["key"].as_str().unwrap_or("");
        runtime
            .update_point(
                connection,
                key,
                event.data["value"].clone(),
                event.data["originFlowId"].as_str(),
            )
            .await;
    } else if event.kind == "connection.changed" {
        runtime
            .update_connection(
                event.data["id"].as_str().unwrap_or(""),
                event.data["state"].clone(),
            )
            .await;
    } else {
        runtime.publish_event(event);
    }
}
pub fn serial_ports() -> Result<Value> {
    serde_json::to_value(hmi_core::connection::list_serial_ports().map_err(error)?).map_err(error)
}
pub fn check_plugin(directory: &Path) -> Result<()> {
    let manifest = hmi_core::protocol::load_plugin_manifest(&directory.join("manifest.json"))
        .map_err(error)?;
    if !hmi_core::safe_identifier(&manifest.id) {
        return Err("非法插件 ID".into());
    }
    let executable = contained_file(directory, &manifest.executable)?;
    check_native_binary(&executable)?;
    Ok(())
}
pub async fn validate(resources: &Resources, project: &Project) -> Vec<ValidationIssue> {
    let mut issues = hmi_core::validate_project(project);
    for (index, backend) in project.backends.iter().enumerate() {
        if !hmi_core::safe_identifier(&backend.provider_id) {
            continue;
        }
        let directory = resources.backends().join(&backend.provider_id);
        let checked = check_backend(&directory).and_then(|_| {
            let manifest =
                hmi_core::backend::load_backend_manifest(&directory.join("manifest.json"))
                    .map_err(error)?;
            if manifest.version != backend.provider_version {
                return Err("后端版本与工程锁定版本不一致".into());
            }
            if !manifest.modes.contains(&backend.mode) {
                return Err("后端不支持所选运行模式".into());
            }
            hmi_core::protocol::validate_schema(&manifest.config_schema, &json!(backend.config))
                .map_err(error)?;
            Ok(())
        });
        if let Err(message) = checked {
            issues.push(ValidationIssue {
                severity: "error".into(),
                path: format!("backends[{index}]"),
                message,
            });
        }
        if let Ok(manifest) =
            hmi_core::backend::load_backend_manifest(&directory.join("manifest.json"))
        {
            for flow in &project.flows {
                for node in &flow.nodes {
                    if node.kind != "backend.invoke"
                        || node.config.get("backendId").and_then(Value::as_str)
                            != Some(backend.id.as_str())
                    {
                        continue;
                    }
                    let checked = (|| {
                        let operation = node
                            .config
                            .get("operation")
                            .and_then(Value::as_str)
                            .ok_or("缺少后端操作名称")?;
                        let descriptor = manifest
                            .operations
                            .iter()
                            .find(|item| item.name == operation)
                            .ok_or("后端未声明此操作")?;
                        let args = node
                            .config
                            .get("args")
                            .and_then(Value::as_object)
                            .ok_or("后端参数必须是对象")?;
                        let (schema, values) = preflight_parameters(&descriptor.args, args);
                        hmi_core::protocol::validate_schema(&schema, &values).map_err(error)
                    })();
                    if let Err(message) = checked {
                        issues.push(ValidationIssue {
                            severity: "error".into(),
                            path: format!("flows[{}].nodes[{}]", flow.id, node.id),
                            message,
                        });
                    }
                }
            }
        }
    }
    let mut point_keys = std::collections::HashSet::new();
    for (index, connection) in project.connections.iter().enumerate() {
        if !hmi_core::safe_identifier(&connection.protocol_id) {
            continue;
        }
        if let Err(message) = check_plugin(&resources.plugins().join(&connection.protocol_id)) {
            issues.push(ValidationIssue {
                severity: "error".into(),
                path: format!("connections[{index}].protocolId"),
                message,
            });
        }
        if let Ok(manifest) = hmi_core::load_plugin_manifest(
            &resources
                .plugins()
                .join(&connection.protocol_id)
                .join("manifest.json"),
        ) {
            if let Err(error) = hmi_core::protocol::validate_schema(
                &manifest.config_schema,
                &json!(connection.protocol_config),
            ) {
                issues.push(ValidationIssue {
                    severity: "error".into(),
                    path: format!("connections[{index}].protocolConfig"),
                    message: error.message,
                });
            }
            for point in &manifest.points {
                point_keys.insert(format!("{}.{}", connection.id, point.key));
            }
            if manifest.id != connection.protocol_id
                || connection
                    .protocol_version
                    .as_ref()
                    .is_some_and(|v| *v != manifest.version)
            {
                issues.push(ValidationIssue {
                    severity: "error".into(),
                    path: format!("connections[{index}].protocolVersion"),
                    message: "项目协议 ID / 版本与资源不一致".into(),
                });
            }
            for (flow_index, flow) in project.flows.iter().enumerate() {
                for (node_index, node) in flow.nodes.iter().enumerate() {
                    if node.kind == "protocol.command"
                        && node.config.get("connectionId").and_then(Value::as_str)
                            == Some(&connection.id)
                    {
                        let name = node
                            .config
                            .get("command")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if let Some(command) = manifest.commands.iter().find(|c| c.name == name) {
                            let references = node
                                .config
                                .get("params")
                                .and_then(Value::as_object)
                                .cloned()
                                .unwrap_or_default();
                            let (schema, values) =
                                preflight_parameters(&command.params, &references);
                            if let Err(error) =
                                hmi_core::protocol::validate_schema(&schema, &values)
                            {
                                issues.push(ValidationIssue {
                                    severity: "error".into(),
                                    path: format!(
                                        "flows[{flow_index}].nodes[{node_index}].config.params"
                                    ),
                                    message: error.message,
                                });
                            }
                        } else {
                            issues.push(ValidationIssue {
                                severity: "error".into(),
                                path: format!(
                                    "flows[{flow_index}].nodes[{node_index}].config.command"
                                ),
                                message: format!("插件未声明命令 {name}"),
                            });
                        }
                    }
                }
            }
        }
    }
    fn check_points(
        value: &Value,
        path: &str,
        known: &std::collections::HashSet<String>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        match value {
            Value::Object(object) => {
                let key = if object.get("source").and_then(Value::as_str) == Some("point") {
                    object.get("key").and_then(Value::as_str)
                } else if object.get("kind").and_then(Value::as_str) == Some("trigger.data") {
                    object
                        .get("config")
                        .and_then(|v| v.get("key"))
                        .and_then(Value::as_str)
                } else {
                    None
                };
                if let Some(key) = key {
                    if !known.contains(key) {
                        issues.push(ValidationIssue {
                            severity: "error".into(),
                            path: path.into(),
                            message: format!("插件未声明数据点 {key}"),
                        });
                    }
                }
                for (key, child) in object {
                    check_points(child, &format!("{path}.{key}"), known, issues);
                }
            }
            Value::Array(array) => {
                for (i, child) in array.iter().enumerate() {
                    check_points(child, &format!("{path}[{i}]"), known, issues);
                }
            }
            _ => (),
        }
    }
    if let Ok(value) = serde_json::to_value(project) {
        check_points(&value, "project", &point_keys, &mut issues);
    }
    if issues.iter().all(|i| i.severity != "error") {
        if let Err(message) = check_python(resources, project).await {
            issues.push(ValidationIssue {
                severity: "error".into(),
                path: "python".into(),
                message,
            });
        }
    }
    issues
}
fn preflight_parameters(
    schema: &Value,
    references: &serde_json::Map<String, Value>,
) -> (Value, Value) {
    let mut schema = schema.clone();
    let mut values = serde_json::Map::new();
    let mut dynamic = Vec::new();
    for (key, reference) in references {
        if reference.get("source").and_then(Value::as_str) == Some("literal") {
            values.insert(
                key.clone(),
                reference.get("value").cloned().unwrap_or(Value::Null),
            );
        } else {
            values.insert(key.clone(), Value::Null);
            dynamic.push(key.clone());
        }
    }
    if !dynamic.is_empty() {
        if let Some(object) = schema.as_object_mut() {
            // Object-wide value assertions require the live input snapshot. Keep
            // required/key checks, and validate every literal value immediately.
            object.remove("enum");
            object.remove("const");
            let allows_extra = object.get("additionalProperties") != Some(&Value::Bool(false));
            let properties = object
                .entry("properties".to_owned())
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .unwrap();
            for key in dynamic {
                if allows_extra || properties.contains_key(&key) {
                    properties.insert(key, Value::Bool(true));
                }
            }
        }
    }
    (schema, Value::Object(values))
}
pub fn pin_plugins(resources: &Resources, project: &mut Project) -> Result<()> {
    for connection in &mut project.connections {
        if !hmi_core::safe_identifier(&connection.protocol_id) {
            return Err("协议 ID 无效".into());
        }
        let path = resources
            .plugins()
            .join(&connection.protocol_id)
            .join("manifest.json");
        if path.is_file() {
            let manifest = hmi_core::load_plugin_manifest(&path).map_err(error)?;
            if connection.protocol_version.is_none() {
                connection.protocol_version = Some(manifest.version);
            }
        }
    }
    Ok(())
}
pub fn pin_python(resources: &Resources, project: &mut Project) -> Result<()> {
    if !has_python(project) {
        return Ok(());
    }
    let path = resources.python(&project.id);
    if !path.exists() {
        return Ok(());
    } // Editing remains possible before resources are imported.
    let lock = read_json(&path.join("dependencies.lock.json"))?;
    if let Some(python) = &mut project.python {
        if python.runtime_id != hmi_python::runtime_id() {
            return Ok(()); // Cross-platform projects stay editable until resources are explicitly imported.
        }
        if python.runtime_sha256.is_none() {
            python.runtime_sha256 = Some(hmi_python::archive_sha256().into());
            python.dependency_lock =
                serde_json::from_value(lock["packages"].clone()).map_err(error)?;
        }
    }
    Ok(())
}
pub fn user_directory(project_id: &str) -> Result<PathBuf> {
    if !hmi_core::safe_identifier(project_id) {
        return Err("项目 ID 无效".into());
    }
    Ok(dirs::data_local_dir()
        .ok_or("用户数据目录不可用")?
        .join("HmiStudio/projects")
        .join(project_id))
}
pub fn apply_settings(project: &mut Project) -> Result<()> {
    let path = user_directory(&project.id)?.join("settings.json");
    if !path.exists() {
        return Ok(());
    }
    let settings = read_json(&path)?;
    for connection in &mut project.connections {
        if let Some(port) = settings["ports"][&connection.id].as_str() {
            connection.port = port.into();
        }
    }
    for backend in &mut project.backends {
        if let Some(mode) = settings["backendModes"][&backend.id]
            .as_str()
            .filter(|mode| ["simulation", "usb"].contains(mode))
        {
            backend.mode = mode.into();
        }
    }
    Ok(())
}
pub fn save_settings(project: &Project) -> Result<()> {
    let ports: std::collections::BTreeMap<_, _> = project
        .connections
        .iter()
        .map(|c| (&c.id, &c.port))
        .collect();
    let backend_modes: BTreeMap<_, _> = project
        .backends
        .iter()
        .map(|backend| (&backend.id, &backend.mode))
        .collect();
    write_json(
        &user_directory(&project.id)?.join("settings.json"),
        &json!({"ports":ports,"backendModes":backend_modes}),
    )
}

#[cfg(all(test, windows))]
mod backend_session_tests {
    use super::*;
    use std::sync::OnceLock;

    fn executable() -> &'static Path {
        static COMPILED: OnceLock<tempfile::TempDir> = OnceLock::new();
        let directory = COMPILED.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap();
            let source =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/backend_fixture.rs");
            let result = std::process::Command::new("rustc")
                .arg("--edition=2021")
                .arg(source)
                .arg("-o")
                .arg(directory.path().join("fixture.exe"))
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            directory
        });
        directory.path()
    }
    fn fixture() -> (tempfile::TempDir, Resources, Project) {
        let directory = tempfile::tempdir().unwrap();
        let provider = directory.path().join("backends/test");
        std::fs::create_dir_all(&provider).unwrap();
        std::fs::copy(
            executable().join("fixture.exe"),
            provider.join("fixture.exe"),
        )
        .unwrap();
        write_json(&provider.join("manifest.json"), &json!({"format":"hmi-backend-provider","apiVersion":1,
            "id":"test","version":"1","target":hmi_core::host_target(),"executable":"fixture.exe",
            "operations":[{"name":"busy","args":{"type":"object"}},{"name":"crash","args":{"type":"object"}}]})).unwrap();
        let project = serde_json::from_value(json!({"schemaVersion":3,"id":"session-backend-test","name":"Session tests","width":400,"height":300,
            "pages":[{"id":"page","name":"Page","widgets":[]}],
            "backends":[{"id":"backend","name":"Test","providerId":"test","providerVersion":"1","mode":"simulation","config":{}}],
            "flows":[{"id":"flow","name":"Flow","enabled":true,"timeoutMs":1000,"nodes":[{"id":"start","kind":"trigger.ui","label":"Start","position":{"x":0,"y":0},"config":{}}],"edges":[]}]})).unwrap();
        let resources = Resources {
            root: directory.path().to_owned(),
            runner: false,
        };
        (directory, resources, project)
    }

    #[tokio::test]
    async fn initial_snapshot_is_ready_and_identical_resource_copies_preserve_provider() {
        let (_directory, resources, project) = fixture();
        let session = Session::new(project.clone(), resources.clone())
            .await
            .unwrap();
        let generation = session.runtime.snapshot().await.backends["backend"]["generation"].clone();
        assert!(generation.is_u64());
        let old_manager = session.backend_manager().await;
        let next_directory = tempfile::tempdir().unwrap();
        copy_tree(&resources.root, next_directory.path()).unwrap();
        let next_resources = Resources {
            root: next_directory.path().to_owned(),
            runner: false,
        };
        let mut next = project;
        next.name = "UI revision".into();
        assert!(!session
            .requires_resource_barrier_with_resources(&next, &next_resources)
            .await
            .unwrap());
        session.apply_revision(next, next_resources).await.unwrap();
        assert!(Arc::ptr_eq(&old_manager, &session.backend_manager().await));
        assert_eq!(
            session.runtime.snapshot().await.backends["backend"]["generation"],
            generation
        );
        session.stop().await;
    }

    #[tokio::test]
    async fn failed_provider_switch_restores_old_model_snapshot_and_releases_barrier() {
        let (_directory, resources, project) = fixture();
        let session = Session::new(project.clone(), resources.clone())
            .await
            .unwrap();
        session.runtime.start().await.unwrap();
        let mut next = project.clone();
        next.backends[0]
            .config
            .insert("failStartup".into(), json!(true));
        assert!(session
            .apply_revision(next, resources)
            .await
            .unwrap_err()
            .contains("fixture startup failure"));
        assert_eq!(session.runtime.project().backends, project.backends);
        assert!(session.runtime.snapshot().await.backends["backend"]["generation"].is_u64());
        session
            .runtime
            .trigger("flow", None, json!({}), json!({}))
            .await
            .unwrap();
        session.runtime.wait_for_runs().await;
        session.stop().await;
    }

    #[tokio::test]
    async fn asset_root_and_runtime_imports_survive_reconfiguration() {
        let (_directory, resources, mut project) = fixture();
        let assets = tempfile::tempdir().unwrap();
        std::fs::write(assets.path().join("firmware.bin"), [1, 2, 3]).unwrap();
        project.assets.push(hmi_core::Asset {
            id: "firmware".into(),
            path: "firmware.bin".into(),
        });
        let session = Session::new_with_assets(
            project.clone(),
            resources.clone(),
            Some(assets.path().into()),
        )
        .await
        .unwrap();
        let imported = assets.path().join("imported.bin");
        std::fs::write(&imported, [4, 5, 6]).unwrap();
        let reference = BackendResource {
            path: imported,
            sha256: None,
            size: Some(3),
            name: Some("imported.bin".into()),
        };
        session
            .backend_manager()
            .await
            .register_resource("runtime_import".into(), reference.clone())
            .unwrap();
        let mut next = project;
        next.backends[0].config.insert("tag".into(), json!(1));
        session.apply_revision(next, resources).await.unwrap();
        let manager = session.backend_manager().await;
        assert_eq!(manager.resource("runtime_import").unwrap(), reference);
        assert_eq!(
            manager.resource("firmware").unwrap().path,
            assets.path().join("firmware.bin").canonicalize().unwrap()
        );
        session.stop().await;
    }

    #[tokio::test]
    async fn accepted_jobs_hold_resource_barrier_and_session_stop_interrupts_waiting_update() {
        let (_directory, resources, project) = fixture();
        let session = Session::new(project.clone(), resources.clone())
            .await
            .unwrap();
        let manager = session.backend_manager().await;
        manager
            .invoke(
                "backend",
                "busy",
                json!({}),
                json!({}),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(session.resource_busy().await);
        let mut next = project;
        next.backends[0].config.insert("tag".into(), json!(2));
        let mut update = {
            let session = session.clone();
            tokio::spawn(async move { session.apply_revision(next, resources).await })
        };
        assert!(tokio::time::timeout(Duration::from_millis(50), &mut update)
            .await
            .is_err());
        tokio::time::timeout(Duration::from_secs(3), session.stop())
            .await
            .unwrap();
        assert!(update.await.unwrap().is_err());
        assert!(session.cancel.is_cancelled());
        assert!(session.backend_manager().await.snapshots().await.is_empty());
    }

    #[tokio::test]
    async fn same_path_asset_changes_are_detected_and_crashed_provider_does_not_block_recovery() {
        let (_directory, resources, mut project) = fixture();
        let asset = resources.root.join("firmware.bin");
        std::fs::write(&asset, [1, 2, 3]).unwrap();
        project.assets.push(hmi_core::Asset {
            id: "firmware".into(),
            path: "firmware.bin".into(),
        });
        let session = Session::new(project.clone(), resources.clone())
            .await
            .unwrap();
        let manager = session.backend_manager().await;
        assert!(manager
            .invoke(
                "backend",
                "crash",
                json!({}),
                json!({}),
                CancellationToken::new()
            )
            .await
            .is_err());
        assert!(!session.resource_busy().await);
        tokio::time::timeout(Duration::from_secs(2), async {
            while session.runtime.snapshot().await.backends["backend"]["providerStatus"] != "failed"
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        std::fs::write(&asset, [4, 5, 6]).unwrap();
        assert!(session
            .requires_resource_barrier_with_resources(&project, &resources)
            .await
            .unwrap());
        session.apply_revision(project, resources).await.unwrap();
        assert!(session.runtime.snapshot().await.backends["backend"]
            .get("providerStatus")
            .is_none());
        assert_eq!(
            session
                .backend_manager()
                .await
                .resource("firmware")
                .unwrap()
                .sha256,
            Some(sha256(&asset).unwrap())
        );
        session.stop().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dynamic_parameters_preserve_required_names_and_literal_validation() {
        let schema = json!({"type":"object","required":["target"],"properties":{"target":{"type":"number","maximum":100},"mode":{"enum":["auto","manual"]}},"additionalProperties":false});
        for (params, valid) in [
            (
                json!({"target":{"source":"input","key":"target-input"}}),
                true,
            ),
            (json!({"target":{"source":"literal","value":101}}), false),
            (
                json!({"target":{"source":"input","key":"target-input"},"mode":{"source":"literal","value":"bad"}}),
                false,
            ),
            (
                json!({"unknown":{"source":"input","key":"target-input"}}),
                false,
            ),
            (json!({}), false),
        ] {
            let (schema, values) = preflight_parameters(&schema, params.as_object().unwrap());
            assert_eq!(
                hmi_core::protocol::validate_schema(&schema, &values).is_ok(),
                valid
            );
        }
    }
    #[tokio::test]
    async fn stop_waits_for_run_terminals_connection_bridge_and_consumers() {
        let directory = tempfile::tempdir().unwrap();
        let project:Project=serde_json::from_value(json!({"schemaVersion":1,"id":"shutdown-test","name":"Shutdown","width":400,"height":300,
            "pages":[{"id":"page","name":"Page","widgets":[]}],
            "connections":[{"id":"serial","name":"Serial","port":"sim://temperature","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"test-protocol"}],
            "variables":[],"flows":[{"id":"flow","name":"Flow","enabled":true,"timeoutMs":5000,
                "nodes":[{"id":"start","kind":"trigger.ui","label":"Start","position":{"x":0,"y":0},"config":{}},{"id":"wait","kind":"delay","label":"Wait","position":{"x":1,"y":0},"config":{"durationMs":4000}}],
                "edges":[{"id":"edge","source":"start","sourcePort":"out","target":"wait","targetPort":"in"}]}]})).unwrap();
        let session = Session::new(
            project,
            Resources {
                root: directory.path().to_owned(),
                runner: false,
            },
        )
        .await
        .unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let collected = records.clone();
        let mut events = session.runtime.subscribe();
        let cancel = session.cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    biased;
                    _=cancel.cancelled()=>{while let Ok(event)=events.try_recv(){collected.lock().unwrap().push(event);}break;},
                    event=events.recv()=>match event {Ok(event)=>collected.lock().unwrap().push(event),Err(_)=>break},
                }
            }
        });
        session.add_event_forwarder(task).await;
        session.runtime.start().await.unwrap();
        session
            .runtime
            .update_point("serial", "temperature", json!(23), None)
            .await;
        let run = session
            .runtime
            .trigger("flow", None, json!({}), json!({}))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(2), session.stop())
            .await
            .unwrap();
        let events = records.lock().unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind == "run.cancelled" && event.run_id.as_deref() == Some(&run)));
        assert!(events
            .iter()
            .any(|event| event.kind == "connection.changed"
                && event.data["state"]["status"] == "closed"));
        assert!(events
            .iter()
            .any(|event| event.kind == "point.changed" && event.data["valid"] == false));
    }
}
