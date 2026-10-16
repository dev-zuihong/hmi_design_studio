use async_trait::async_trait;
use hmi_core::{CodeModule, CoreResult, HostServices, Project, Runtime};
use serde_json::{json, Value};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct Host {
    calls: Mutex<Vec<Value>>,
}
#[async_trait]
impl HostServices for Host {
    async fn open_connection(&self, _: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn close_connection(&self, _: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn command(&self, _: &str, _: &str, _: Value, _: CancellationToken) -> CoreResult<Value> {
        Ok(json!({}))
    }
    async fn python(
        &self,
        _: &str,
        msg: Value,
        _: u64,
        _: u64,
        _: CancellationToken,
    ) -> CoreResult<Option<Value>> {
        Ok(Some(msg))
    }
    async fn python_with_modules(
        &self,
        _: &str,
        mut msg: Value,
        modules: &[CodeModule],
        _: u64,
        _: u64,
        _: CancellationToken,
    ) -> CoreResult<Option<Value>> {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"context":msg["context"],"args":msg["args"],"source":modules[0].source}));
        tokio::time::sleep(Duration::from_millis(30)).await;
        msg["state"]["count"] = json!(msg["state"]["count"].as_u64().unwrap() + 1);
        Ok(Some(msg))
    }
}
fn node(id: &str, kind: &str, config: Value) -> Value {
    json!({"id":id,"kind":kind,"label":id,"position":{"x":0,"y":0},"config":config})
}
fn flow(id: &str, scope: &str, nodes: Vec<Value>) -> Value {
    let edges:Vec<_>=nodes.windows(2).enumerate().map(|(i,p)|json!({"id":format!("e{i}"),"source":p[0]["id"],"sourcePort":"out","target":p[1]["id"],"targetPort":"in"})).collect();
    json!({"id":id,"name":id,"scope":scope,"enabled":true,"timeoutMs":5000,"nodes":nodes,"edges":edges})
}
fn project(flows: Vec<Value>) -> Project {
    serde_json::from_value(json!({
    "schemaVersion":4,"id":"v4-runtime-tests","name":"V4","width":800,"height":600,
    "pages":[{"id":"main","name":"Main","widgets":[]}],
    "variables":[{"id":"state","name":"State","type":"object","initial":{"count":0,"byDevice":{"usb.path":{"value":42}}}}],
    "codeModules":[{"id":"module","name":"application","source":"COUNT=1"}],
    "python":{"runtimeId":"test"},"flows":flows})).unwrap()
}
fn work() -> Value {
    flow(
        "work",
        "device",
        vec![
            node("entry", "trigger.ui", json!({})),
            node(
                "code",
                "python",
                json!({
                    "code":"def transform(msg): return msg","stateVariable":"state","args":{"value":{"source":"variable","key":"state","path":"byDevice.$deviceKey.value"}}
                }),
            ),
        ],
    )
}
async fn wait_count(runtime: &Arc<Runtime>, count: u64) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if runtime.snapshot().await.variables["state"]["count"] == count {
                break;
            };
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn device_scope_survives_window_close_and_atomic_python_commits_do_not_lose_updates() {
    let host = Arc::new(Host::default());
    let runtime = Runtime::new(project(vec![work()]), host.clone()).unwrap();
    runtime.start().await.unwrap();
    let message = json!({"context":{"windowInstanceId":"settings","transportId":"usb","deviceKey":"usb.path","deviceGeneration":1}});
    runtime
        .trigger("work", None, message.clone(), json!({}))
        .await
        .unwrap();
    assert_eq!(
        runtime
            .trigger("work", None, message.clone(), json!({}))
            .await
            .unwrap_err()
            .code,
        "FLOW_BUSY"
    );
    let mut next = message;
    next["context"]["deviceGeneration"] = json!(2);
    runtime
        .trigger("work", None, next, json!({}))
        .await
        .unwrap();
    runtime.clear_window("settings");
    wait_count(&runtime, 2).await;
    let calls = host.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert!(calls
        .iter()
        .all(|c| c["context"].get("windowInstanceId").is_none() && c["args"]["value"] == 42));
    drop(calls);
    runtime.stop().await;
}

#[tokio::test]
async fn start_entry_can_dispatch_into_later_enabled_flow() {
    let bootstrap = flow(
        "bootstrap",
        "window",
        vec![
            node("start", "trigger.start", json!({})),
            node(
                "dispatch",
                "flow.dispatch",
                json!({"flowId":"work","items":{"source":"literal","value":[{"context":{"transportId":"usb","deviceKey":"usb.path","deviceGeneration":1}}]}}),
            ),
        ],
    );
    let runtime =
        Runtime::new(project(vec![bootstrap, work()]), Arc::new(Host::default())).unwrap();
    runtime.start().await.unwrap();
    wait_count(&runtime, 1).await;
    runtime.stop().await;
}
