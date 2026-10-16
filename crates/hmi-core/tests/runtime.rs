use async_trait::async_trait;
use hmi_core::{
    validate_project, CoreError, CoreResult, HostServices, Project, Runtime, RuntimeEvent,
};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

struct Host;
#[async_trait]
impl HostServices for Host {
    async fn open_connection(&self, _: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn close_connection(&self, _: &str) -> CoreResult<()> {
        Ok(())
    }
    async fn command(&self, _: &str, _: &str, _: Value, _: CancellationToken) -> CoreResult<Value> {
        Err(CoreError::new("DEVICE_ERROR", "test device"))
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
}
fn node(id: &str, kind: &str, config: Value) -> Value {
    json!({"id":id,"kind":kind,"label":id,"position":{"x":0,"y":0},"config":config})
}
fn edge(id: &str, from: &str, port: &str, to: &str) -> Value {
    json!({"id":id,"source":from,"sourcePort":port,"target":to,"targetPort":"in"})
}
fn project(nodes: Vec<Value>, edges: Vec<Value>) -> Project {
    serde_json::from_value(json!({"schemaVersion":1,"id":"project","name":"Tests","width":800,"height":600,
        "pages":[{"id":"page","name":"Page","widgets":[]}],
        "connections":[{"id":"device","name":"Device","port":"sim://temperature","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"test-protocol","protocolVersion":"1.0.0","protocolConfig":{}}],
        "variables":[{"id":"value","name":"Value","type":"number","initial":0}],
        "flows":[{"id":"flow","name":"Flow","enabled":true,"timeoutMs":2000,"nodes":nodes,"edges":edges}]})).unwrap()
}
async fn runtime(project: Project) -> (Arc<Runtime>, broadcast::Receiver<RuntimeEvent>) {
    let runtime = Runtime::new(project, Arc::new(Host)).unwrap();
    let events = runtime.subscribe();
    runtime.start().await.unwrap();
    (runtime, events)
}
async fn completion(
    events: &mut broadcast::Receiver<RuntimeEvent>,
    run: &str,
) -> Vec<RuntimeEvent> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut result = Vec::new();
        loop {
            let event = events.recv().await.unwrap();
            if event.run_id.as_deref() != Some(run) {
                continue;
            }
            let done =
                ["run.completed", "run.failed", "run.cancelled"].contains(&event.kind.as_str());
            result.push(event);
            if done {
                return result;
            }
        }
    })
    .await
    .expect("runtime must terminate")
}

#[test]
fn validates_error_edges_cycles_references_and_versions() {
    let mut project = project(
        vec![
            node("start", "trigger.ui", json!({})),
            node("a", "delay", json!({"durationMs":1})),
            node("b", "delay", json!({"durationMs":1})),
        ],
        vec![
            edge("e1", "start", "out", "a"),
            edge("e2", "a", "error", "b"),
            edge("e3", "b", "out", "a"),
        ],
    );
    assert!(validate_project(&project)
        .iter()
        .any(|e| e.path.ends_with("edges")));
    project.flows[0].edges.pop();
    assert!(validate_project(&project).is_empty());
    project.flows[0].nodes[1].kind = "variable.set".into();
    project.flows[0].nodes[1].config = serde_json::from_value(
        json!({"variableId":"missing","value":{"source":"literal","value":1}}),
    )
    .unwrap();
    assert!(!validate_project(&project).is_empty());
    let roundtrip: Project =
        serde_json::from_value(serde_json::to_value(&project).unwrap()).unwrap();
    assert_eq!(
        roundtrip.connections[0].protocol_version.as_deref(),
        Some("1.0.0")
    );
}

#[tokio::test]
async fn independent_branches_and_multiple_inputs_do_not_implicitly_join() {
    let (rt, mut events) = runtime(project(
        vec![
            node("start", "trigger.ui", json!({})),
            node(
                "left",
                "assign",
                json!({"path":"payload","value":{"source":"literal","value":2}}),
            ),
            node(
                "right",
                "assign",
                json!({"path":"payload","value":{"source":"literal","value":3}}),
            ),
            node(
                "log",
                "log",
                json!({"value":{"source":"message","path":"payload"}}),
            ),
        ],
        vec![
            edge("e1", "start", "out", "left"),
            edge("e2", "start", "out", "right"),
            edge("e3", "left", "out", "log"),
            edge("e4", "right", "out", "log"),
        ],
    ))
    .await;
    let id = rt
        .trigger("flow", None, json!({"payload":1}), json!({}))
        .await
        .unwrap();
    let trace = completion(&mut events, &id).await;
    let values: Vec<_> = trace
        .iter()
        .filter(|e| e.kind == "log")
        .map(|e| e.data["value"].clone())
        .collect();
    assert_eq!(values, vec![json!(2), json!(3)]);
    assert_eq!(trace.last().unwrap().kind, "run.completed");
    rt.stop().await;
}

#[tokio::test]
async fn explicit_error_path_and_condition_are_executed() {
    let (rt,mut events)=runtime(project(vec![node("start","trigger.ui",json!({})),
        node("bad","variable.set",json!({"variableId":"value","value":{"source":"literal","value":"wrong type"}})),
        node("condition","condition",json!({"left":{"source":"message","path":"error.code"},"operator":"eq","right":{"source":"literal","value":"VARIABLE_TYPE"}})),
        node("fixed","variable.set",json!({"variableId":"value","value":{"source":"literal","value":42}})),
        node("wrong","variable.set",json!({"variableId":"value","value":{"source":"literal","value":99}}))],
        vec![edge("e1","start","out","bad"),edge("e2","bad","error","condition"),edge("e3","condition","true","fixed"),edge("e4","condition","false","wrong")])).await;
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    let trace = completion(&mut events, &id).await;
    assert_eq!(trace.last().unwrap().kind, "run.completed");
    assert!(trace.iter().any(|e| e.kind == "node.failed"));
    assert_eq!(rt.snapshot().await.variables["value"], 42);
    rt.stop().await;
}

#[tokio::test]
async fn management_stop_bypasses_busy_and_restarts_disabled_flow() {
    let (rt, mut events) = runtime(project(
        vec![
            node("start", "trigger.ui", json!({})),
            node("wait", "delay", json!({"durationMs":200})),
            node(
                "write",
                "variable.set",
                json!({"variableId":"value","value":{"source":"literal","value":42}}),
            ),
        ],
        vec![
            edge("e1", "start", "out", "wait"),
            edge("e2", "wait", "out", "write"),
        ],
    ))
    .await;
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    assert_eq!(
        rt.trigger("flow", None, json!({}), json!({}))
            .await
            .unwrap_err()
            .code,
        "FLOW_BUSY"
    );
    rt.stop_flow("flow").await.unwrap();
    assert_eq!(
        completion(&mut events, &id).await.last().unwrap().kind,
        "run.cancelled"
    );
    assert_eq!(rt.snapshot().await.variables["value"], 0);
    assert_eq!(
        rt.trigger("flow", None, json!({}), json!({}))
            .await
            .unwrap_err()
            .code,
        "FLOW_DISABLED"
    );
    rt.start_flow("flow").await.unwrap();
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    assert_eq!(
        completion(&mut events, &id).await.last().unwrap().kind,
        "run.completed"
    );
    assert_eq!(rt.snapshot().await.variables["value"], 42);
    rt.stop().await;
}

#[tokio::test]
async fn automatic_feedback_is_suppressed_and_rate_limit_requires_recovery() {
    let (rt, _) = runtime(project(
        vec![
            node("data", "trigger.data", json!({"key":"device.temperature"})),
            node("wait", "delay", json!({"durationMs":1000})),
        ],
        vec![edge("e1", "data", "out", "wait")],
    ))
    .await;
    rt.update_point("device", "temperature", json!(1), Some("flow"))
        .await;
    assert_eq!(rt.snapshot().await.flows["flow"]["busy"], false);
    for value in 2..14 {
        rt.update_point("device", "temperature", json!(value), None)
            .await;
    }
    assert_eq!(rt.snapshot().await.flows["flow"]["autoPaused"], true);
    rt.stop_flow("flow").await.unwrap();
    rt.start_flow("flow").await.unwrap();
    assert_eq!(rt.snapshot().await.flows["flow"]["autoPaused"], false);
    rt.stop().await;
}

#[tokio::test]
async fn total_timeout_and_message_bound_are_enforced() {
    let mut p = project(
        vec![
            node("start", "trigger.ui", json!({})),
            node("wait", "delay", json!({"durationMs":1000})),
        ],
        vec![edge("e1", "start", "out", "wait")],
    );
    p.flows[0].timeout_ms = 20;
    let (rt, mut events) = runtime(p).await;
    assert_eq!(
        rt.trigger(
            "flow",
            None,
            json!({"value":"x".repeat(1024*1024)}),
            json!({})
        )
        .await
        .unwrap_err()
        .code,
        "MESSAGE_LIMIT"
    );
    assert_eq!(
        rt.trigger(
            "flow",
            None,
            json!({}),
            json!({"value":"x".repeat(1024*1024)})
        )
        .await
        .unwrap_err()
        .code,
        "MESSAGE_LIMIT"
    );
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    assert_eq!(
        completion(&mut events, &id).await.last().unwrap().data["error"]["code"],
        "RUN_TIMEOUT"
    );
    rt.stop().await;
}

#[tokio::test]
async fn pending_branch_queue_is_bounded() {
    let edges = (0..257)
        .map(|i| edge(&format!("e{i}"), "start", "out", "leaf"))
        .collect();
    let (rt, mut events) = runtime(project(
        vec![
            node("start", "trigger.ui", json!({})),
            node(
                "leaf",
                "log",
                json!({"value":{"source":"literal","value":0}}),
            ),
        ],
        edges,
    ))
    .await;
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    assert_eq!(
        completion(&mut events, &id).await.last().unwrap().data["error"]["code"],
        "QUEUE_LIMIT"
    );
    rt.stop().await;
}

#[tokio::test]
async fn run_steps_are_bounded_even_for_valid_acyclic_graph() {
    let mut nodes = vec![node("start", "trigger.ui", json!({}))];
    let mut edges = Vec::new();
    let mut previous = "start".to_owned();
    for i in 0..1000 {
        let id = format!("n{i}");
        nodes.push(node(
            &id,
            "assign",
            json!({"path":"payload","value":{"source":"literal","value":0}}),
        ));
        edges.push(edge(&format!("e{i}"), &previous, "out", &id));
        previous = id;
    }
    let (rt, mut events) = runtime(project(nodes, edges)).await;
    let id = rt
        .trigger("flow", None, json!({}), json!({}))
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match events.recv().await {
                Ok(event) if event.run_id.as_deref() == Some(&id) && event.kind == "run.failed" => {
                    break event
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => (),
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(result.data["error"]["code"], "STEP_LIMIT");
    rt.stop().await;
}
