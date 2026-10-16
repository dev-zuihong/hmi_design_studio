//! Exercises the actual plugin executable, shared runtime, and FIFO connection actor.
use async_trait::async_trait;
use hmi_core::{
    ConnectionConfig, ConnectionManager, CoreError, CoreResult, HostServices, Project, Runtime,
};
use serde_json::{json, Value};
use std::{future::Future, task::Poll};
use std::{path::Path, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropping_connection_owner_releases_idle_plugin_process() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let manager = ConnectionManager::new(vec![config(30)], directory.path());
    manager.open("serial").await.unwrap();
    drop(manager);
    // Windows keeps a running executable locked. The unique temporary test copy
    // becomes removable only after the plugin process has exited.
    let executable = directory.path().join("demo-temperature/demo-protocol.exe");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if std::fs::remove_file(&executable).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("dropping a manager must not orphan its plugin");
}

#[tokio::test]
async fn close_invalidates_an_inflight_open_and_releases_the_session_lock() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let manager = ConnectionManager::new(vec![config(30)], directory.path());
    let mut events = manager.subscribe();
    let opening = manager.open("serial");
    tokio::pin!(opening);
    // A single poll registers the owned opening task, then close races its plugin startup.
    std::future::poll_fn(|context| {
        assert!(opening.as_mut().poll(context).is_pending());
        Poll::Ready(())
    })
    .await;
    manager.close("serial").await.unwrap();
    assert_eq!(opening.await.unwrap_err().code, "CANCELLED");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.data["state"]["status"] == "closed" {
                break;
            }
        }
    })
    .await
    .expect("close must publish its final state after the open task is invalidated");
    assert_eq!(
        manager
            .command("serial", "read", json!({}), CancellationToken::new())
            .await
            .unwrap_err()
            .code,
        "CONNECTION_CLOSED"
    );
    let late_open = tokio::time::timeout(Duration::from_millis(50), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.kind == "connection.changed" && event.data["state"]["status"] == "open" {
                break;
            }
        }
    })
    .await;
    assert!(
        late_open.is_err(),
        "the invalidated open task must not publish a late open state"
    );
}

fn package(root: &Path) {
    let destination = root.join("demo-temperature");
    std::fs::create_dir_all(&destination).unwrap();
    let mut manifest: Value =
        serde_json::from_slice(include_bytes!("../package/manifest.json")).unwrap();
    manifest["target"] = json!(hmi_core::host_target());
    manifest["executable"] = json!(format!("demo-protocol{}", std::env::consts::EXE_SUFFIX));
    std::fs::write(
        destination.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::copy(
        env!("CARGO_BIN_EXE_demo-protocol"),
        destination.join(format!("demo-protocol{}", std::env::consts::EXE_SUFFIX)),
    )
    .unwrap();
}
fn config(delay: u64) -> ConnectionConfig {
    serde_json::from_value(json!({"id":"serial","name":"Demo simulated device","port":"sim://temperature","baudRate":115200,
        "dataBits":8,"stopBits":1,"parity":"none","protocolId":"demo-temperature","protocolVersion":"1.0.0","protocolConfig":{"simulationDelayMs":delay,"timeoutMs":2000}})).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manifest_constraints_reject_values_before_the_device_can_accept_them() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let path = directory.path().join("demo-temperature/manifest.json");
    let mut manifest: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    manifest["commands"][1]["params"]["properties"]["target"]["maximum"] = json!(10);
    std::fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let mut invalid = config(30);
    invalid
        .protocol_config
        .insert("timeoutMs".into(), json!("wrong type"));
    let manager = ConnectionManager::new(vec![invalid], directory.path());
    assert_eq!(
        manager.open("serial").await.unwrap_err().code,
        "PROTOCOL_CONFIG"
    );
    let manager = ConnectionManager::new(vec![config(30)], directory.path());
    manager.open("serial").await.unwrap();
    // The demo device accepts 40 C, but this package's schema intentionally does not.
    assert_eq!(
        manager
            .command(
                "serial",
                "setTarget",
                json!({"target":40}),
                CancellationToken::new()
            )
            .await
            .unwrap_err()
            .code,
        "PARAMETERS"
    );
    let result = manager
        .command("serial", "read", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result["target"], 25.0);
    manager.close_all().await;
}
struct Services(Arc<ConnectionManager>);
#[async_trait]
impl HostServices for Services {
    async fn open_connection(&self, id: &str) -> CoreResult<()> {
        self.0.open(id).await
    }
    async fn close_connection(&self, id: &str) -> CoreResult<()> {
        self.0.close(id).await
    }
    async fn command(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        self.0.command(id, name, params, cancel).await
    }
    async fn command_with_origin(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
        origin: &str,
    ) -> CoreResult<Value> {
        self.0
            .command_with_origin(id, name, params, cancel, Some(origin))
            .await
    }
    async fn python(
        &self,
        _: &str,
        _: Value,
        _: u64,
        _: u64,
        _: CancellationToken,
    ) -> CoreResult<Option<Value>> {
        Err(CoreError::new(
            "PYTHON_UNUSED",
            "This test contains no Python node",
        ))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn headless_runtime_opens_plugin_reads_device_and_updates_ui_variable() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let connection = config(30);
    let project:Project=serde_json::from_value(json!({"schemaVersion":1,"id":"headless","name":"Headless runner smoke","width":800,"height":600,
        "pages":[{"id":"page","name":"Main","widgets":[]}],"connections":[connection.clone()],
        "variables":[{"id":"temperature","name":"Temperature","type":"number","initial":0}],
        "flows":[{"id":"read-flow","name":"Read","enabled":true,"timeoutMs":10000,
            "nodes":[
                {"id":"start","kind":"trigger.ui","label":"Start","position":{"x":0,"y":0},"config":{}},
                {"id":"open","kind":"connection.open","label":"Open","position":{"x":1,"y":0},"config":{"connectionId":"serial"}},
                {"id":"read","kind":"protocol.command","label":"Read","position":{"x":2,"y":0},"config":{"connectionId":"serial","command":"read","params":{}}},
                {"id":"write","kind":"variable.set","label":"Show","position":{"x":3,"y":0},"config":{"variableId":"temperature","value":{"source":"message","path":"payload.temperature"}}}],
            "edges":[
                {"id":"e1","source":"start","sourcePort":"out","target":"open","targetPort":"in"},
                {"id":"e2","source":"open","sourcePort":"out","target":"read","targetPort":"in"},
                {"id":"e3","source":"read","sourcePort":"out","target":"write","targetPort":"in"}]}]})).unwrap();
    let manager = ConnectionManager::new(vec![connection], directory.path());
    let mut device_events = manager.subscribe();
    let runtime = Runtime::new(project, Arc::new(Services(manager.clone()))).unwrap();
    let mut events = runtime.subscribe();
    let bridge_runtime = runtime.clone();
    let bridge = tokio::spawn(async move {
        while let Ok(event) = device_events.recv().await {
            match event.kind.as_str() {
                "point.changed" => {
                    bridge_runtime
                        .update_point(
                            event.data["connectionId"].as_str().unwrap(),
                            event.data["key"].as_str().unwrap(),
                            event.data["value"].clone(),
                            event.data["originFlowId"].as_str(),
                        )
                        .await
                }
                "connection.changed" => {
                    bridge_runtime
                        .update_connection(
                            event.data["id"].as_str().unwrap(),
                            event.data["state"].clone(),
                        )
                        .await
                }
                _ => bridge_runtime.publish_event(event),
            }
        }
    });
    runtime.start().await.unwrap();
    let id = runtime
        .trigger("read-flow", None, json!({"payload":null}), json!({}))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = events.recv().await.unwrap();
            if event.run_id.as_deref() != Some(&id) {
                continue;
            }
            if event.kind == "run.failed" {
                panic!("runtime failure: {}", event.data);
            }
            if event.kind == "run.completed" {
                break;
            }
        }
    })
    .await
    .unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if runtime
                .snapshot()
                .await
                .points
                .contains_key("serial.temperature")
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let snapshot = runtime.snapshot().await;
    assert!(snapshot.variables["temperature"].as_f64().unwrap() > 20.0);
    assert_eq!(
        snapshot.variables["temperature"],
        snapshot.points["serial.temperature"]
    );
    assert_eq!(snapshot.connections["serial"]["status"], "open");
    assert_eq!(snapshot.connections["serial"]["simulated"], true);
    runtime.stop().await;
    manager.close_all().await;
    bridge.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_issued_transaction_drains_and_cancelled_queued_command_is_skipped() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let manager = ConnectionManager::new(vec![config(180)], directory.path());
    manager.open("serial").await.unwrap();
    let first_cancel = CancellationToken::new();
    let first_manager = manager.clone();
    let token = first_cancel.clone();
    let first = tokio::spawn(async move {
        first_manager
            .command("serial", "setTarget", json!({"target":40}), token)
            .await
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    let skipped_cancel = CancellationToken::new();
    let skipped_manager = manager.clone();
    let token = skipped_cancel.clone();
    let skipped = tokio::spawn(async move {
        skipped_manager
            .command("serial", "setTarget", json!({"target":99}), token)
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;
    first_cancel.cancel();
    skipped_cancel.cancel();
    assert_eq!(first.await.unwrap().unwrap_err().code, "CANCELLED");
    assert_eq!(skipped.await.unwrap().unwrap_err().code, "CANCELLED");
    let result = manager
        .command("serial", "read", json!({}), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result["target"], 40.0);
    manager.close_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connection_queue_has_a_hard_capacity_and_version_mismatch_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    package(directory.path());
    let mut bad = config(500);
    bad.protocol_version = Some("0.0.0".into());
    let invalid = ConnectionManager::new(vec![bad], directory.path());
    assert_eq!(
        invalid.open("serial").await.unwrap_err().code,
        "PLUGIN_VERSION"
    );
    let manager = ConnectionManager::new(vec![config(500)], directory.path());
    manager.open("serial").await.unwrap();
    let cancel = CancellationToken::new();
    let mut tasks = Vec::new();
    for _ in 0..80 {
        let manager = manager.clone();
        let cancel = cancel.clone();
        tasks.push(tokio::spawn(async move {
            manager.command("serial", "read", json!({}), cancel).await
        }));
    }
    tokio::time::sleep(Duration::from_millis(80)).await;
    cancel.cancel();
    let mut rejected = 0;
    for task in tasks {
        if task
            .await
            .unwrap()
            .is_err_and(|e| e.code == "COMMAND_QUEUE_FULL")
        {
            rejected += 1;
        }
    }
    assert!(rejected >= 15, "one issued and 64 queued maximum");
    manager.close_all().await;
}
