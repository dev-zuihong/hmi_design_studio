use async_trait::async_trait;
use hmi_core::{CoreError, CoreResult, HostServices, Project, Runtime, RuntimeEvent};
use serde_json::{json, Value};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
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
        Err(CoreError::new(
            "DEVICE_ERROR",
            "test host has no device command",
        ))
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
fn flow(id: &str, name: &str, enabled: bool, nodes: Vec<Value>, edges: Vec<Value>) -> Value {
    json!({"id":id,"name":name,"enabled":enabled,"timeoutMs":5000,"nodes":nodes,"edges":edges})
}
fn project(flows: Vec<Value>, widgets: Vec<Value>) -> Project {
    serde_json::from_value(json!({
        "schemaVersion":2,"id":"reload-project","name":"Hot reload tests","width":800,"height":600,
        "pages":[{"id":"page","name":"Page","responsive":true,"layout":{"type":"absolute"},"guides":[],"widgets":widgets}],
        "connections":[{"id":"device","name":"Device","port":"sim://temperature","baudRate":115200,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"test-protocol","protocolConfig":{}}],
        "variables":[{"id":"value","name":"Value","type":"number","initial":0}],
        "flows":flows,"assets":[],"actions":[],
        "i18n":{"defaultLocale":"zh-CN","locales":["zh-CN","en-US"],"messages":{"zh-CN":{},"en-US":{}}}
    })).unwrap()
}
fn widget(id: &str, kind: &str, props: Value) -> Value {
    json!({"id":id,"kind":kind,"name":id,"x":0,"y":0,"width":120,"height":32,"zIndex":1,"props":props,"bindings":{},"events":{}})
}
fn number_variable(id: &str, initial: i64) -> Value {
    json!({"id":id,"name":id,"type":"number","initial":initial})
}
async fn running(project: Project) -> (Arc<Runtime>, broadcast::Receiver<RuntimeEvent>) {
    let runtime = Runtime::new(project, Arc::new(Host)).unwrap();
    let events = runtime.subscribe();
    runtime.start().await.unwrap();
    (runtime, events)
}
async fn event_matching(
    events: &mut broadcast::Receiver<RuntimeEvent>,
    mut predicate: impl FnMut(&RuntimeEvent) -> bool,
) -> RuntimeEvent {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let event = events.recv().await.unwrap();
            if predicate(&event) {
                return event;
            }
        }
    })
    .await
    .expect("expected runtime event")
}
async fn run_completion(
    events: &mut broadcast::Receiver<RuntimeEvent>,
    run_id: &str,
) -> Vec<RuntimeEvent> {
    tokio::time::timeout(Duration::from_secs(4), async {
        let mut out = Vec::new();
        loop {
            let event = events.recv().await.unwrap();
            if event.run_id.as_deref() != Some(run_id) {
                continue;
            }
            let done =
                ["run.completed", "run.failed", "run.cancelled"].contains(&event.kind.as_str());
            out.push(event);
            if done {
                return out;
            }
        }
    })
    .await
    .expect("run must terminate")
}
fn trigger(id: &str) -> Value {
    node(id, "trigger.ui", json!({}))
}
fn set_value(id: &str, value: i64) -> Value {
    node(
        id,
        "variable.set",
        json!({"variableId":"value","value":{"source":"literal","value":value}}),
    )
}

#[tokio::test]
async fn an_admitted_run_keeps_its_old_node_config_and_revision_while_the_same_flow_is_single_flight(
) {
    let old = project(
        vec![flow(
            "flow",
            "Flow",
            true,
            vec![
                trigger("start"),
                node("wait", "delay", json!({"durationMs":140})),
                set_value("write", 1),
            ],
            vec![
                edge("e1", "start", "out", "wait"),
                edge("e2", "wait", "out", "write"),
            ],
        )],
        vec![],
    );
    let (runtime, mut events) = running(old.clone()).await;
    let started = Instant::now();
    let run = runtime
        .trigger("flow", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    event_matching(&mut events, |event| {
        event.run_id.as_deref() == Some(&run)
            && event.kind == "node.started"
            && event.node_id.as_deref() == Some("wait")
    })
    .await;

    let mut next = old;
    next.flows[0].nodes[1]
        .config
        .insert("durationMs".into(), json!(1));
    next.flows[0].nodes[2]
        .config
        .insert("value".into(), json!({"source":"literal","value":2}));
    runtime.apply_project_revision(next).unwrap();
    assert_eq!(
        runtime
            .trigger("flow", Some("start"), json!({}), json!({}))
            .await
            .unwrap_err()
            .code,
        "FLOW_BUSY"
    );

    let old_events = run_completion(&mut events, &run).await;
    assert_eq!(old_events.last().unwrap().kind, "run.completed");
    assert!(old_events.iter().all(|event| event.revision == Some(0)));
    let wait = old_events
        .iter()
        .find(|event| event.kind == "node.completed" && event.node_id.as_deref() == Some("wait"))
        .unwrap();
    assert!(
        wait.data["durationMs"].as_u64().unwrap() >= 100,
        "old delay config should finish after the captured 140ms delay"
    );
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert_eq!(runtime.snapshot().await.variables["value"], json!(1));

    let next_run = runtime
        .trigger("flow", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    let new_events = run_completion(&mut events, &next_run).await;
    assert_eq!(new_events.last().unwrap().kind, "run.completed");
    assert!(new_events.iter().all(|event| event.revision == Some(1)));
    assert_eq!(runtime.snapshot().await.variables["value"], json!(2));
    runtime.stop().await;
}

#[tokio::test]
async fn adding_an_enabled_flow_to_a_running_revision_makes_it_triggerable_without_restart() {
    let old = project(vec![], vec![]);
    let (runtime, mut events) = running(old.clone()).await;
    let added = flow(
        "new-flow",
        "New",
        true,
        vec![trigger("new-start"), set_value("new-write", 8)],
        vec![edge("new-edge", "new-start", "out", "new-write")],
    );
    let mut next = old;
    next.flows.push(serde_json::from_value(added).unwrap());
    runtime.apply_project_revision(next).unwrap();
    let run = runtime
        .trigger("new-flow", Some("new-start"), json!({}), json!({}))
        .await
        .unwrap();
    let trace = run_completion(&mut events, &run).await;
    assert_eq!(trace.last().unwrap().kind, "run.completed");
    assert!(trace.iter().all(|event| event.revision == Some(1)));
    assert_eq!(runtime.snapshot().await.variables["value"], json!(8));
    runtime.stop().await;
}

#[tokio::test]
async fn timer_changes_restart_only_the_timer_and_preserve_manual_stop_and_rate_limit_pause() {
    let old = project(
        vec![
            flow(
                "timer",
                "Timer",
                true,
                vec![
                    node("tick", "trigger.timer", json!({"intervalMs":800})),
                    node("timer-wait", "delay", json!({"durationMs":1})),
                ],
                vec![edge("timer-edge-1", "tick", "out", "timer-wait")],
            ),
            flow(
                "manual",
                "Manual",
                true,
                vec![
                    trigger("manual-start"),
                    node("manual-wait", "delay", json!({"durationMs":1})),
                ],
                vec![edge("manual-edge", "manual-start", "out", "manual-wait")],
            ),
            flow(
                "auto",
                "Auto",
                true,
                vec![
                    node("data", "trigger.data", json!({"key":"device.temperature"})),
                    node("auto-wait", "delay", json!({"durationMs":500})),
                ],
                vec![edge("auto-edge", "data", "out", "auto-wait")],
            ),
        ],
        vec![],
    );
    let (runtime, mut events) = running(old.clone()).await;
    runtime.stop_flow("manual").await.unwrap();
    for value in 0..12 {
        runtime
            .update_point("device", "temperature", json!(value), None)
            .await;
    }
    assert_eq!(
        runtime.snapshot().await.flows["auto"]["autoPaused"],
        json!(true)
    );

    let mut next = old;
    next.flows
        .iter_mut()
        .find(|flow| flow.id == "timer")
        .unwrap()
        .nodes[0]
        .config
        .insert("intervalMs".into(), json!(100));
    runtime.apply_project_revision(next).unwrap();
    let snapshot = runtime.snapshot().await;
    assert_eq!(
        snapshot.flows["manual"]["enabled"],
        json!(false),
        "manual stop state must survive compatible revisions"
    );
    assert_eq!(
        snapshot.flows["auto"]["autoPaused"],
        json!(true),
        "rate-limit pause must survive compatible revisions"
    );
    let mut new_timer_runs = 0;
    tokio::time::timeout(Duration::from_millis(350), async {
        while new_timer_runs < 2 {
            let event = events.recv().await.unwrap();
            if event.flow_id.as_deref() == Some("timer")
                && event.kind == "run.started"
                && event.revision == Some(1)
            {
                new_timer_runs += 1
            }
        }
    })
    .await
    .expect("new timer interval should be registered promptly");
    assert_eq!(
        runtime.snapshot().await.flows["auto"]["autoPaused"],
        json!(true)
    );
    runtime.stop().await;
}

#[tokio::test]
async fn global_stop_cancels_a_busy_run_even_after_its_flow_is_removed() {
    let old = project(
        vec![flow(
            "busy",
            "Busy",
            true,
            vec![
                trigger("start"),
                node("wait", "delay", json!({"durationMs":5000})),
                set_value("write", 9),
            ],
            vec![
                edge("e1", "start", "out", "wait"),
                edge("e2", "wait", "out", "write"),
            ],
        )],
        vec![],
    );
    let (runtime, mut events) = running(old.clone()).await;
    let run = runtime
        .trigger("busy", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    event_matching(&mut events, |event| {
        event.run_id.as_deref() == Some(&run)
            && event.kind == "node.started"
            && event.node_id.as_deref() == Some("wait")
    })
    .await;
    runtime
        .apply_project_revision(project(vec![], vec![]))
        .unwrap();
    runtime.stop().await;
    let trace = run_completion(&mut events, &run).await;
    assert_eq!(trace.last().unwrap().kind, "run.cancelled");
    assert_eq!(runtime.snapshot().await.variables["value"], json!(0));
}

#[tokio::test]
async fn compatible_ui_only_edits_preserve_runtime_variables_and_widget_inputs() {
    let mut old = project(
        vec![flow(
            "set",
            "Set",
            true,
            vec![trigger("start"), set_value("write", 7)],
            vec![edge("edge", "start", "out", "write")],
        )],
        vec![widget(
            "input",
            "input",
            json!({"text":"Old","value":"default"}),
        )],
    );
    for (id, kind, initial) in [
        ("notes", "textarea", json!("")),
        ("level", "slider", json!(0)),
        ("date", "date", json!("")),
        ("checked", "checkbox", json!(false)),
    ] {
        old.pages[0]
            .widgets
            .push(serde_json::from_value(widget(id, kind, json!({"value":initial}))).unwrap());
    }
    let (runtime, mut events) = running(old.clone()).await;
    runtime.update_input("input".into(), json!("42"));
    runtime.update_input("notes".into(), json!("user notes"));
    runtime.update_input("level".into(), json!(37));
    runtime.update_input("date".into(), json!("2026-10-16"));
    runtime.update_input("checked".into(), json!(true));
    let run = runtime
        .trigger("set", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    run_completion(&mut events, &run).await;

    let mut next = old;
    next.pages[0].widgets[0]
        .props
        .insert("text".into(), json!("Updated"));
    next.pages[0].widgets[0]
        .props
        .insert("value".into(), json!("new default"));
    next.variables[0].initial = json!(100);
    runtime.apply_project_revision(next).unwrap();
    let snapshot = runtime.snapshot().await;
    assert_eq!(snapshot.variables["value"], json!(7));
    assert_eq!(snapshot.inputs["input"], json!("42"));
    assert_eq!(snapshot.inputs["notes"], json!("user notes"));
    assert_eq!(snapshot.inputs["level"], json!(37));
    assert_eq!(snapshot.inputs["date"], json!("2026-10-16"));
    assert_eq!(snapshot.inputs["checked"], json!(true));
    runtime.stop().await;
}

#[tokio::test]
async fn ui_property_override_is_visible_to_property_references() {
    let flow = flow(
        "ui",
        "UI",
        true,
        vec![
            trigger("start"),
            node(
                "set-level",
                "ui.setProperty",
                json!({"widgetId":"w","property":"level","value":{"source":"literal","value":5}}),
            ),
            node(
                "read-level",
                "variable.set",
                json!({"variableId":"value","value":{"source":"property","key":"w","property":"level"}}),
            ),
        ],
        vec![
            edge("e1", "start", "out", "set-level"),
            edge("e2", "set-level", "out", "read-level"),
        ],
    );
    let (runtime, mut events) = running(project(
        vec![flow],
        vec![widget("w", "text", json!({"text":"Level","level":1}))],
    ))
    .await;
    let run = runtime
        .trigger("ui", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    let trace = run_completion(&mut events, &run).await;
    assert_eq!(trace.last().unwrap().kind, "run.completed");
    assert_eq!(runtime.snapshot().await.ui["w.level"], json!(5));
    assert_eq!(runtime.snapshot().await.variables["value"], json!(5));
    runtime.stop().await;
}

#[tokio::test]
async fn a_stale_run_cannot_apply_ui_side_effects_to_the_new_revision() {
    let old = project(
        vec![flow(
            "stale",
            "Stale",
            true,
            vec![
                trigger("start"),
                node("wait", "delay", json!({"durationMs":80})),
                node(
                    "set-ui",
                    "ui.setProperty",
                    json!({"widgetId":"w","property":"level","value":{"source":"literal","value":99}}),
                ),
            ],
            vec![
                edge("e1", "start", "out", "wait"),
                edge("e2", "wait", "out", "set-ui"),
            ],
        )],
        vec![widget("w", "text", json!({"text":"old","level":1}))],
    );
    let (runtime, mut events) = running(old.clone()).await;
    let run = runtime
        .trigger("stale", Some("start"), json!({}), json!({}))
        .await
        .unwrap();
    event_matching(&mut events, |event| {
        event.run_id.as_deref() == Some(&run)
            && event.kind == "node.started"
            && event.node_id.as_deref() == Some("wait")
    })
    .await;
    let mut next = old;
    next.pages[0].widgets[0]
        .props
        .insert("text".into(), json!("new"));
    next.pages[0].widgets[0]
        .props
        .insert("level".into(), json!(10));
    runtime.apply_project_revision(next).unwrap();
    let trace = run_completion(&mut events, &run).await;
    assert_eq!(trace.last().unwrap().kind, "run.completed");
    assert_eq!(trace.last().unwrap().revision, Some(0));
    assert!(trace
        .iter()
        .any(|event| event.kind == "ui.property.ignored"
            && event.node_id.as_deref() == Some("set-ui")));
    assert!(!runtime.snapshot().await.ui.contains_key("w.level"));
    runtime.stop().await;
}

#[tokio::test]
async fn property_references_follow_widget_and_custom_property_bindings_before_old_ui_overrides() {
    let mut bound_widget = widget("w", "text", json!({"text":"Widget","level":1}));
    bound_widget["bindings"] = json!({});
    let mut custom_widget = widget("c", "text", json!({"text":"Custom"}));
    custom_widget["customProperties"] = json!({"score":{"type":"number","value":3}});
    let override_flow = flow(
        "override",
        "Set UI overrides",
        true,
        vec![
            trigger("override-start"),
            node(
                "widget-override",
                "ui.setProperty",
                json!({"widgetId":"w","property":"level","value":{"source":"literal","value":99}}),
            ),
            node(
                "custom-override",
                "ui.setProperty",
                json!({"widgetId":"c","property":"score","value":{"source":"literal","value":88}}),
            ),
        ],
        vec![
            edge("e1", "override-start", "out", "widget-override"),
            edge("e2", "widget-override", "out", "custom-override"),
        ],
    );
    let mut old = project(
        vec![override_flow],
        vec![bound_widget.clone(), custom_widget.clone()],
    );
    old.variables
        .push(serde_json::from_value(number_variable("source", 42)).unwrap());
    old.variables
        .push(serde_json::from_value(number_variable("result-widget", 0)).unwrap());
    old.variables
        .push(serde_json::from_value(number_variable("result-custom", 0)).unwrap());
    let (runtime, mut events) = running(old.clone()).await;
    let override_run = runtime
        .trigger("override", Some("override-start"), json!({}), json!({}))
        .await
        .unwrap();
    run_completion(&mut events, &override_run).await;
    assert_eq!(runtime.snapshot().await.ui["w.level"], json!(99));
    assert_eq!(runtime.snapshot().await.ui["c.score"], json!(88));

    bound_widget["bindings"] = json!({"level":{"source":"variable","key":"source"}});
    custom_widget["customProperties"]["score"]["binding"] =
        json!({"source":"variable","key":"source"});
    let read_flow = flow(
        "read",
        "Read bound properties",
        true,
        vec![
            trigger("read-start"),
            node(
                "read-widget",
                "variable.set",
                json!({"variableId":"result-widget","value":{"source":"property","key":"w","property":"level"}}),
            ),
            node(
                "read-custom",
                "variable.set",
                json!({"variableId":"result-custom","value":{"source":"property","key":"c","property":"score"}}),
            ),
        ],
        vec![
            edge("read-e1", "read-start", "out", "read-widget"),
            edge("read-e2", "read-widget", "out", "read-custom"),
        ],
    );
    let mut next = old;
    next.pages[0].widgets[0] = serde_json::from_value(bound_widget).unwrap();
    next.pages[0].widgets[1] = serde_json::from_value(custom_widget).unwrap();
    // Bound properties cannot be written by ui.setProperty. Remove the writer
    // flow while retaining its already-applied UI overrides in the runtime.
    next.flows.retain(|flow| flow.id != "override");
    next.flows.push(serde_json::from_value(read_flow).unwrap());
    runtime.apply_project_revision(next).unwrap();
    let read_run = runtime
        .trigger("read", Some("read-start"), json!({}), json!({}))
        .await
        .unwrap();
    run_completion(&mut events, &read_run).await;
    let snapshot = runtime.snapshot().await;
    assert_eq!(snapshot.variables["result-widget"], json!(42));
    assert_eq!(snapshot.variables["result-custom"], json!(42));
    runtime.stop().await;
}

#[tokio::test]
async fn changing_a_custom_property_triggers_its_configured_flow_with_property_payload() {
    let mut target = widget("w", "text", json!({"text":"Target"}));
    target["customProperties"] = json!({"score":{"type":"number","value":0,"event":{"kind":"flow","flowId":"listener","entryNodeId":"listener-start"}}});
    let writer = flow(
        "writer",
        "Writer",
        true,
        vec![
            trigger("writer-start"),
            node(
                "set-score",
                "ui.setProperty",
                json!({"widgetId":"w","property":"score","value":{"source":"literal","value":5}}),
            ),
        ],
        vec![edge("writer-edge", "writer-start", "out", "set-score")],
    );
    let listener = flow(
        "listener",
        "Listener",
        true,
        vec![
            trigger("listener-start"),
            node(
                "capture",
                "variable.set",
                json!({"variableId":"value","value":{"source":"message","path":"payload.value"}}),
            ),
        ],
        vec![edge("listener-edge", "listener-start", "out", "capture")],
    );
    let (runtime, mut events) = running(project(vec![writer, listener], vec![target])).await;
    let run = runtime
        .trigger("writer", Some("writer-start"), json!({}), json!({}))
        .await
        .unwrap();
    // The internal event flow can start before the writer's completion event.
    // Collect both run IDs in one pass so the writer helper cannot discard it.
    let mut writer_trace = Vec::new();
    let mut listener_start = None;
    tokio::time::timeout(Duration::from_secs(4), async {
        while writer_trace.last().is_none_or(|event: &RuntimeEvent| {
            !["run.completed", "run.failed", "run.cancelled"].contains(&event.kind.as_str())
        }) || listener_start.is_none()
        {
            let event = events.recv().await.unwrap();
            if event.run_id.as_deref() == Some(&run) {
                writer_trace.push(event.clone());
            }
            if event.kind == "run.started" && event.flow_id.as_deref() == Some("listener") {
                listener_start = Some(event);
            }
        }
    })
    .await
    .expect("writer and property listener should both start and finish");
    assert_eq!(writer_trace.last().unwrap().kind, "run.completed");
    let listener_start = listener_start.unwrap();
    let listener_id = listener_start.run_id.clone().unwrap();
    let mut listener_trace = run_completion(&mut events, &listener_id).await;
    listener_trace.insert(0, listener_start);
    assert_eq!(listener_trace.last().unwrap().kind, "run.completed");
    assert_eq!(
        listener_trace.first().unwrap().data["message"]["payload"]["widgetId"],
        json!("w")
    );
    assert_eq!(
        listener_trace.first().unwrap().data["message"]["payload"]["property"],
        json!("score")
    );
    assert_eq!(
        listener_trace.first().unwrap().data["message"]["payload"]["value"],
        json!(5)
    );
    assert_eq!(runtime.snapshot().await.variables["value"], json!(5));
    runtime.stop().await;
}
