use crate::resources::*;
use async_trait::async_trait;
use hmi_core::{
    ConnectionManager, CoreError, CoreResult, HostServices, Project, Runtime, RuntimeEvent,
    ValidationIssue,
};
use hmi_python::{PythonExecutor, PythonLimits};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct Session {
    pub runtime: Arc<Runtime>,
    pub connections: Arc<ConnectionManager>,
    pub cancel: CancellationToken,
    bridge_cancel: CancellationToken,
    bridge: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    forwarders: tokio::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>,
    stopping: tokio::sync::Mutex<()>,
}
struct Services {
    connections: Arc<ConnectionManager>,
    python: Option<PythonExecutor>,
    runtime: Mutex<Weak<Runtime>>,
}
#[async_trait]
impl HostServices for Services {
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
        let executor = self
            .python
            .as_ref()
            .ok_or_else(|| CoreError::new("PYTHON_MISSING", "未加载项目 Python 环境"))?;
        let result = executor
            .execute(
                source,
                msg,
                PythonLimits {
                    timeout: Duration::from_millis(timeout_ms),
                    memory_bytes: (memory_mb * 1024 * 1024) as usize,
                },
                cancel,
            )
            .await
            .map_err(|e| CoreError::new("PYTHON", e.to_string()))?;
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
        let python = check_python(&resources, &project).await?;
        let connections = ConnectionManager::new(project.connections.clone(), resources.plugins());
        let host = Arc::new(Services {
            connections: connections.clone(),
            python,
            runtime: Mutex::new(Weak::new()),
        });
        let runtime = Runtime::new(project, host.clone()).map_err(error)?;
        *host.runtime.lock().unwrap() = Arc::downgrade(&runtime);
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
        Ok(Arc::new(Self {
            runtime,
            connections,
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
    pub async fn stop(&self) {
        let _stopping = self.stopping.lock().await;
        self.runtime.stop().await;
        for connection in self.connections.configs() {
            let _ = self.connections.close(&connection.id).await;
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
    check_amd64(&executable)?;
    Ok(())
}
pub async fn validate(resources: &Resources, project: &Project) -> Vec<ValidationIssue> {
    let mut issues = hmi_core::validate_project(project);
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
        if python.runtime_sha256.is_none() {
            python.runtime_sha256 = Some(hmi_python::ARCHIVE_SHA256.into());
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
    Ok(())
}
pub fn save_settings(project: &Project) -> Result<()> {
    let ports: std::collections::BTreeMap<_, _> = project
        .connections
        .iter()
        .map(|c| (&c.id, &c.port))
        .collect();
    write_json(
        &user_directory(&project.id)?.join("settings.json"),
        &json!({"ports":ports}),
    )
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
            "connections":[{"id":"serial","name":"Serial","port":"sim://temperature","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"demo-temperature"}],
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
