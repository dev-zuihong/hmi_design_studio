use async_trait::async_trait;
use hmi_core::{BackendEvent, CoreResult, HostServices, Project, Runtime};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
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
    async fn backend_invoke(
        &self,
        id: &str,
        operation: &str,
        args: Value,
        context: Value,
        _: CancellationToken,
    ) -> CoreResult<Value> {
        self.calls
            .lock()
            .unwrap()
            .push(json!({"id":id,"operation":operation,"args":args,"context":context}));
        Ok(json!({"accepted":true,"jobId":"job-1"}))
    }
}
fn node(id: &str, kind: &str, config: Value) -> Value {
    json!({"id":id,"kind":kind,"label":id,"position":{"x":0,"y":0},"config":config})
}
fn project(nodes: Vec<Value>) -> Project {
    let edges: Vec<_> = nodes.windows(2).enumerate().map(|(index,pair)| json!({"id":format!("edge{index}"),"source":pair[0]["id"],"target":pair[1]["id"],"sourcePort":"out","targetPort":"in"})).collect();
    serde_json::from_value(json!({"schemaVersion":3,"id":"backend-test","name":"Backend tests","width":800,"height":600,
        "pages":[{"id":"main","name":"Main","widgets":[{"id":"field","kind":"input","name":"Field","x":0,"y":0,"width":100,"height":30,"zIndex":0,"props":{"value":"default"}}]}],
        "backends":[{"id":"test-backend","name":"PTB","providerId":"test-provider","providerVersion":"1.0.0","mode":"simulation","config":{}}],
        "windows":[{"id":"settings","pageId":"main","title":"Settings","width":800,"height":600}],
        "flows":[{"id":"flow","name":"Flow","enabled":true,"timeoutMs":5000,"nodes":nodes,"edges":edges}]})).unwrap()
}
fn snapshot(generation: u64, sequence: u64, value: &str) -> BackendEvent {
    BackendEvent {
        backend_id: "test-backend".into(),
        kind: "snapshot".into(),
        generation,
        sequence,
        device_key: Some("USB.path\\device".into()),
        job_id: None,
        data: json!({"byDevice":{"USB.path\\device":{"value":value}}}),
    }
}

#[test]
fn contextual_paths_substitute_one_key_and_reject_expressions() {
    let data = json!({"byDevice":{"USB.path\\device":{"value":7}}});
    let context = json!({"deviceKey":"USB.path\\device"});
    assert_eq!(
        hmi_core::runtime::get_context_path(&data, "byDevice.$deviceKey.value", &context),
        Some(&json!(7))
    );
    assert!(
        hmi_core::runtime::get_context_path(&data, "byDevice.$missing.value", &context).is_none()
    );
    for path in [
        "byDevice.$(deviceKey)",
        "byDevice.__proto__",
        "byDevice..value",
    ] {
        assert!(!hmi_core::valid_backend_path(path));
    }
}

#[tokio::test]
async fn windows_have_independent_single_flight_inputs_and_ui_overrides() {
    let project = project(vec![
        node("start", "trigger.ui", json!({})),
        node("delay", "delay", json!({"durationMs":30})),
        node(
            "write",
            "ui.setProperty",
            json!({"widgetId":"field","property":"value","value":{"source":"input","key":"field"}}),
        ),
    ]);
    let runtime = Runtime::new(project, Arc::new(Host::default())).unwrap();
    runtime.start().await.unwrap();
    runtime.update_input("field".into(), json!("global"));
    for (window, value) in [("a", "first"), ("b", "second")] {
        runtime
            .trigger_with_runtime_inputs(
                "flow",
                None,
                json!({"context":{"windowInstanceId":window}}),
                json!({"field":value}),
            )
            .await
            .unwrap();
    }
    assert_eq!(
        runtime
            .trigger(
                "flow",
                None,
                json!({"context":{"windowInstanceId":"a"}}),
                json!({})
            )
            .await
            .unwrap_err()
            .code,
        "FLOW_BUSY"
    );
    runtime.wait_for_runs().await;
    let state = runtime.snapshot().await;
    assert_eq!(state.window_ui["a"]["field.value"], json!("first"));
    assert_eq!(state.window_ui["b"]["field.value"], json!("second"));
    assert!(state.ui.is_empty());
    runtime.clear_window("a");
    assert!(!runtime.snapshot().await.window_ui.contains_key("a"));
}

#[tokio::test]
async fn backend_nodes_resolve_contextual_state_and_keep_context_for_following_nodes() {
    let host = Arc::new(Host::default());
    let project = project(vec![
        node("start", "trigger.ui", json!({})),
        node(
            "invoke",
            "backend.invoke",
            json!({"backendId":"test-backend","operation":"start","args":{
        "value":{"source":"backend","key":"test-backend","path":"byDevice.$deviceKey.value"},"deviceKey":{"source":"context","path":"deviceKey"}}}),
        ),
        node(
            "write",
            "ui.setProperty",
            json!({"widgetId":"field","property":"value","value":{"source":"message","path":"payload.jobId"}}),
        ),
    ]);
    let runtime = Runtime::new(project, host.clone()).unwrap();
    runtime.start().await.unwrap();
    runtime.update_backend(snapshot(1, 1, "configured"));
    runtime
        .trigger(
            "flow",
            None,
            json!({"context":{"windowInstanceId":"settings-1","deviceKey":"USB.path\\device"}}),
            json!({}),
        )
        .await
        .unwrap();
    runtime.wait_for_runs().await;
    let calls = host.calls.lock().unwrap();
    assert_eq!(calls[0]["args"]["value"], json!("configured"));
    assert_eq!(calls[0]["context"]["windowInstanceId"], json!("settings-1"));
    drop(calls);
    assert_eq!(
        runtime.snapshot().await.window_ui["settings-1"]["field.value"],
        json!("job-1")
    );
}

#[tokio::test]
async fn backend_triggers_ignore_reordered_and_retired_generations() {
    let host = Arc::new(Host::default());
    let project = project(vec![
        node(
            "event",
            "trigger.backend",
            json!({"backendId":"test-backend","event":"snapshot"}),
        ),
        node(
            "invoke",
            "backend.invoke",
            json!({"backendId":"test-backend","operation":"observe","args":{}}),
        ),
    ]);
    let runtime = Runtime::new(project, host.clone()).unwrap();
    runtime.start().await.unwrap();
    runtime.update_backend(snapshot(10, 5, "new"));
    runtime.wait_for_runs().await;
    runtime.update_backend(snapshot(10, 4, "old"));
    runtime.update_backend(snapshot(9, 8, "retired"));
    runtime.wait_for_runs().await;
    assert_eq!(host.calls.lock().unwrap().len(), 1);
    assert_eq!(
        runtime.snapshot().await.backends["test-backend"]["byDevice"]["USB.path\\device"]["value"],
        json!("new")
    );
    runtime.reset_backend("test-backend");
    runtime.update_backend(snapshot(11, 0, "replacement"));
    runtime.wait_for_runs().await;
    assert_eq!(host.calls.lock().unwrap().len(), 2);
}

#[test]
fn schema_three_rejects_unknown_backends_and_invalid_window_bounds() {
    let mut project = project(vec![
        node("start", "trigger.ui", json!({})),
        node(
            "invoke",
            "backend.invoke",
            json!({"backendId":"missing","operation":"start"}),
        ),
    ]);
    assert!(hmi_core::validate_project(&project)
        .iter()
        .any(|issue| issue.path.ends_with("config")));
    project.flows.clear();
    project.windows[0].min_width = Some(900.0);
    assert!(hmi_core::validate_project(&project)
        .iter()
        .any(|issue| issue.path == "windows[0]"));
}

#[test]
fn blank_default_project_has_valid_schema_and_no_preloaded_behavior() {
    let project: Project =
        serde_json::from_str(include_str!("../../../resources/default/project.json")).unwrap();
    let errors: Vec<_> = hmi_core::validate_project(&project)
        .into_iter()
        .filter(|issue| issue.severity == "error")
        .collect();
    assert!(errors.is_empty(), "{errors:#?}");
    assert_eq!(project.schema_version, 4);
    assert_eq!(project.pages.len(), 1);
    assert!(project.pages[0].widgets.is_empty());
    assert!(project.flows.is_empty());
    assert!(project.connections.is_empty());
    assert!(project.variables.is_empty());
    assert!(project.backends.is_empty());
    assert!(project.code_modules.is_empty());
}
