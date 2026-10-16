use crate::{validate_project, CoreError, CoreResult, Flow, FlowNode, Project, ValueRef};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast, Notify};
use tokio_util::sync::CancellationToken;

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_RUN_STEPS: usize = 1000;
pub const MAX_RUN_QUEUE: usize = 256;

#[async_trait]
pub trait HostServices: Send + Sync + 'static {
    async fn open_connection(&self, id: &str) -> CoreResult<()>;
    async fn close_connection(&self, id: &str) -> CoreResult<()>;
    /// Once a transaction starts, the implementation must drain its response even if cancelled.
    async fn command(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value>;
    async fn command_with_origin(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
        _origin_flow_id: &str,
    ) -> CoreResult<Value> {
        self.command(id, name, params, cancel).await
    }
    async fn python(
        &self,
        source: &str,
        msg: Value,
        timeout_ms: u64,
        memory_mb: u64,
        cancel: CancellationToken,
    ) -> CoreResult<Option<Value>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeEvent {
    pub kind: String,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub data: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub revision: Option<u64>,
}

impl RuntimeEvent {
    pub fn new(kind: impl Into<String>, data: Value) -> Self {
        Self {
            kind: kind.into(),
            timestamp: now_ms(),
            run_id: None,
            flow_id: None,
            node_id: None,
            data,
            revision: None,
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub variables: BTreeMap<String, Value>,
    pub inputs: BTreeMap<String, Value>,
    pub points: BTreeMap<String, Value>,
    pub connections: BTreeMap<String, Value>,
    pub flows: BTreeMap<String, Value>,
    pub ui: BTreeMap<String, Value>,
    pub logs: Vec<RuntimeEvent>,
}

struct FlowState {
    enabled: bool,
    running: Option<(String, CancellationToken)>,
    trigger_cancel: CancellationToken,
    auto_times: VecDeque<Instant>,
    auto_paused: bool,
    dropped: u64,
}

struct State {
    started: bool,
    revision_barrier: bool,
    variables: BTreeMap<String, Value>,
    inputs: BTreeMap<String, Value>,
    points: BTreeMap<String, Value>,
    connections: BTreeMap<String, Value>,
    ui: BTreeMap<String, Value>,
    flows: HashMap<String, FlowState>,
    logs: VecDeque<RuntimeEvent>,
    run_revisions: HashMap<String, u64>,
    run_tokens: HashMap<String, CancellationToken>,
    run_flows: HashMap<String, String>,
}

pub struct Runtime {
    project: RwLock<Arc<Project>>,
    project_revision: AtomicU64,
    host: Arc<dyn HostServices>,
    state: Mutex<State>,
    events: broadcast::Sender<RuntimeEvent>,
    active_runs: AtomicUsize,
    runs_finished: Notify,
}

struct ActiveRun(Arc<Runtime>);
impl Drop for ActiveRun {
    fn drop(&mut self) {
        if self.0.active_runs.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.0.runs_finished.notify_waiters();
        }
    }
}

impl Runtime {
    pub fn new(project: Project, host: Arc<dyn HostServices>) -> CoreResult<Arc<Self>> {
        let issues = validate_project(&project);
        if let Some(first) = issues.iter().find(|i| i.severity == "error") {
            return Err(CoreError::new(
                "INVALID_PROJECT",
                format!("{}: {}", first.path, first.message),
            ));
        }
        let (events, _) = broadcast::channel(2048);
        let state = State {
            started: false,
            revision_barrier: false,
            variables: project
                .variables
                .iter()
                .map(|v| (v.id.clone(), v.initial.clone()))
                .collect(),
            inputs: BTreeMap::new(),
            points: BTreeMap::new(),
            connections: BTreeMap::new(),
            ui: BTreeMap::new(),
            logs: VecDeque::new(),
            run_revisions: HashMap::new(),
            run_tokens: HashMap::new(),
            run_flows: HashMap::new(),
            flows: project
                .flows
                .iter()
                .map(|f| {
                    (
                        f.id.clone(),
                        FlowState {
                            enabled: false,
                            running: None,
                            trigger_cancel: CancellationToken::new(),
                            auto_times: VecDeque::new(),
                            auto_paused: false,
                            dropped: 0,
                        },
                    )
                })
                .collect(),
        };
        Ok(Arc::new(Self {
            project: RwLock::new(Arc::new(project)),
            project_revision: AtomicU64::new(0),
            host,
            state: Mutex::new(state),
            events,
            active_runs: AtomicUsize::new(0),
            runs_finished: Notify::new(),
        }))
    }

    pub fn project(&self) -> Arc<Project> {
        self.project.read().unwrap().clone()
    }

    /// Publishes a validated immutable revision. Runs already admitted retain their
    /// captured project Arc; new triggers use the replacement.
    pub fn apply_project_revision(self: &Arc<Self>, project: Project) -> CoreResult<()> {
        let issues = validate_project(&project);
        if let Some(first) = issues.iter().find(|i| i.severity == "error") {
            return Err(CoreError::new(
                "INVALID_PROJECT",
                format!("{}: {}", first.path, first.message),
            ));
        }
        let next = Arc::new(project);
        let previous = self.project();
        let mut timers_to_start = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            let mut variables = BTreeMap::new();
            for definition in &next.variables {
                let value = state
                    .variables
                    .get(&definition.id)
                    .filter(|value| definition.accepts(value))
                    .cloned()
                    .unwrap_or_else(|| definition.initial.clone());
                variables.insert(definition.id.clone(), value);
            }
            state.variables = variables;
            let compatible_inputs: std::collections::HashSet<String> = next
                .pages
                .iter()
                .flat_map(|page| page.widgets.iter())
                .filter(|widget| {
                    previous
                        .pages
                        .iter()
                        .flat_map(|page| page.widgets.iter())
                        .any(|old| old.id == widget.id && old.kind == widget.kind)
                })
                .map(|widget| widget.id.clone())
                .collect();
            state.inputs.retain(|id, _| compatible_inputs.contains(id));
            state.ui.retain(|key, _| {
                let Some((widget_id, property)) = key.split_once('.') else {
                    return false;
                };
                next.pages
                    .iter()
                    .flat_map(|page| page.widgets.iter())
                    .any(|widget| {
                        widget.id == widget_id
                            && !widget.bindings.contains_key(property)
                            && !widget
                                .custom_properties
                                .get(property)
                                .is_some_and(|custom| custom.binding.is_some())
                            && (widget.props.contains_key(property)
                                || widget.custom_properties.contains_key(property))
                    })
            });
            let old_flows = std::mem::take(&mut state.flows);
            let was_started = state.started;
            state.flows = next
                .flows
                .iter()
                .map(|flow| {
                    let old = old_flows.get(&flow.id);
                    let old_flow = previous
                        .flows
                        .iter()
                        .find(|candidate| candidate.id == flow.id);
                    let timer_unchanged = old_flow.is_some_and(|old_flow| {
                        old_flow
                            .nodes
                            .iter()
                            .filter(|n| n.kind == "trigger.timer")
                            .map(|n| (&n.id, &n.config))
                            .eq(flow
                                .nodes
                                .iter()
                                .filter(|n| n.kind == "trigger.timer")
                                .map(|n| (&n.id, &n.config)))
                    });
                    let definition_changed =
                        old_flow.is_some_and(|old_flow| old_flow.enabled != flow.enabled);
                    let enabled = if old.is_some() && !definition_changed {
                        old.is_some_and(|value| value.enabled)
                    } else {
                        was_started && flow.enabled
                    };
                    let trigger_cancel = if enabled && timer_unchanged {
                        old.unwrap().trigger_cancel.clone()
                    } else {
                        if let Some(old) = old {
                            old.trigger_cancel.cancel();
                        }
                        CancellationToken::new()
                    };
                    if enabled && (!timer_unchanged || old.is_none() || definition_changed) {
                        timers_to_start.push((flow.clone(), trigger_cancel.clone()));
                    }
                    (
                        flow.id.clone(),
                        FlowState {
                            enabled,
                            running: old.and_then(|value| value.running.clone()),
                            trigger_cancel,
                            auto_times: old
                                .map(|value| value.auto_times.clone())
                                .unwrap_or_default(),
                            auto_paused: old.is_some_and(|value| value.auto_paused),
                            dropped: old.map(|value| value.dropped).unwrap_or_default(),
                        },
                    )
                })
                .collect();
            for (id, old) in old_flows {
                if !next.flows.iter().any(|flow| flow.id == id) {
                    old.trigger_cancel.cancel();
                }
            }
        }
        *self.project.write().unwrap() = next;
        self.project_revision.fetch_add(1, Ordering::AcqRel);
        for (flow, token) in timers_to_start {
            self.spawn_timer_triggers(&flow, token);
        }
        Ok(())
    }

    pub fn begin_revision_barrier(&self) {
        self.state.lock().unwrap().revision_barrier = true;
    }

    pub fn finish_revision_barrier(&self) {
        self.state.lock().unwrap().revision_barrier = false;
    }

    pub fn update_input(&self, id: String, value: Value) {
        self.state.lock().unwrap().inputs.insert(id, value);
    }

    pub fn active_run_count(&self) -> usize {
        self.active_runs.load(Ordering::Acquire)
    }

    pub async fn wait_for_runs(&self) {
        loop {
            let finished = self.runs_finished.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if self.active_runs.load(Ordering::Acquire) == 0 {
                break;
            }
            finished.await;
        }
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        let state = self.state.lock().unwrap();
        RuntimeSnapshot {
            variables: state.variables.clone(), inputs: state.inputs.clone(), points: state.points.clone(), connections: state.connections.clone(), ui: state.ui.clone(),
            flows: state.flows.iter().map(|(id,f)|(id.clone(),json!({
                "enabled": f.enabled, "busy": f.running.is_some(), "autoPaused": f.auto_paused,
                "dropped": f.dropped, "runId": f.running.as_ref().map(|(id,_)|id),
            }))).collect(),
            logs: state.logs.iter().cloned().collect(),
        }
    }

    pub fn publish_event(&self, event: RuntimeEvent) {
        let mut event = event;
        {
            let mut state = self.state.lock().unwrap();
            if event.revision.is_none() {
                event.revision = Some(
                    event
                        .run_id
                        .as_ref()
                        .and_then(|run_id| state.run_revisions.get(run_id).copied())
                        .unwrap_or_else(|| self.project_revision.load(Ordering::Acquire)),
                );
            }
            state.logs.push_back(event.clone());
            while state.logs.len() > 500 {
                state.logs.pop_front();
            }
        }
        let _ = self.events.send(event);
    }

    fn emit(
        &self,
        kind: &str,
        flow: Option<&str>,
        run: Option<&str>,
        node: Option<&str>,
        data: Value,
    ) {
        self.publish_event(RuntimeEvent {
            kind: kind.into(),
            timestamp: now_ms(),
            flow_id: flow.map(str::to_owned),
            run_id: run.map(str::to_owned),
            node_id: node.map(str::to_owned),
            data,
            revision: None,
        });
    }

    pub async fn start(self: &Arc<Self>) -> CoreResult<()> {
        {
            let mut state = self.state.lock().unwrap();
            if state.started {
                return Ok(());
            }
            state.started = true;
        }
        let project = self.project();
        for f in &project.flows {
            if f.enabled {
                self.start_flow(&f.id).await?;
            }
        }
        Ok(())
    }

    pub async fn stop(&self) {
        // Reject new admissions before cancelling any existing run. The admission
        // counter is incremented under this same state lock, before task spawning.
        self.state.lock().unwrap().started = false;
        let ids: Vec<String> = self.project().flows.iter().map(|f| f.id.clone()).collect();
        for id in ids {
            let _ = self.stop_flow(&id).await;
        }
        for token in self.state.lock().unwrap().run_tokens.values() {
            token.cancel();
        }
        loop {
            let finished = self.runs_finished.notified();
            tokio::pin!(finished);
            finished.as_mut().enable();
            if self.active_runs.load(Ordering::Acquire) == 0 {
                break;
            }
            finished.await;
        }
    }

    pub async fn start_flow(self: &Arc<Self>, id: &str) -> CoreResult<()> {
        let project = self.project();
        let flow = project
            .flows
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| CoreError::new("FLOW_MISSING", "流程不存在"))?;
        let token = {
            let mut state = self.state.lock().unwrap();
            if !state.started {
                return Err(CoreError::new("NOT_RUNNING", "请先启动运行会话"));
            }
            if state.revision_barrier {
                return Err(CoreError::new(
                    "REVISION_WAITING",
                    "资源更新等待当前流程和设备事务结束",
                ));
            }
            let f = state.flows.get_mut(id).unwrap();
            f.trigger_cancel.cancel();
            f.trigger_cancel = CancellationToken::new();
            f.enabled = true;
            f.auto_paused = false;
            f.auto_times.clear();
            f.trigger_cancel.clone()
        };
        self.spawn_timer_triggers(flow, token);
        self.emit(
            "flow.changed",
            Some(id),
            None,
            None,
            json!({"enabled":true,"autoPaused":false}),
        );
        Ok(())
    }

    fn spawn_timer_triggers(self: &Arc<Self>, flow: &Flow, token: CancellationToken) {
        for node in flow.nodes.iter().filter(|n| n.kind == "trigger.timer") {
            let weak = Arc::downgrade(self);
            let cancel = token.clone();
            let flow_id = flow.id.clone();
            let node_id = node.id.clone();
            let interval = Duration::from_millis(node.config["intervalMs"].as_u64().unwrap());
            tokio::spawn(async move {
                let mut timer =
                    tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
                timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        biased;
                        _ = cancel.cancelled() => break,
                        _ = timer.tick() => {
                            let Some(runtime) = weak.upgrade() else { break };
                            let _ = runtime.trigger_internal(&flow_id, Some(&node_id), json!({"payload":null}), Value::Object(Map::new()), true, false);
                        }
                    }
                }
            });
        }
    }

    pub async fn stop_flow(&self, id: &str) -> CoreResult<()> {
        {
            let mut state = self.state.lock().unwrap();
            if !state.flows.contains_key(id) {
                for (run_id, flow_id) in &state.run_flows {
                    if flow_id == id {
                        if let Some(token) = state.run_tokens.get(run_id) {
                            token.cancel();
                        }
                    }
                }
                return Ok(());
            }
            let f = state.flows.get_mut(id).unwrap();
            f.enabled = false;
            f.trigger_cancel.cancel();
            if let Some((_, token)) = f.running.take() {
                token.cancel();
            }
        }
        self.emit(
            "flow.changed",
            Some(id),
            None,
            None,
            json!({"enabled":false,"busy":false}),
        );
        Ok(())
    }

    pub async fn trigger(
        self: &Arc<Self>,
        flow_id: &str,
        entry_node_id: Option<&str>,
        message: Value,
        inputs: Value,
    ) -> CoreResult<String> {
        self.trigger_internal(flow_id, entry_node_id, message, inputs, false, false)
    }

    pub async fn trigger_with_runtime_inputs(
        self: &Arc<Self>,
        flow_id: &str,
        entry_node_id: Option<&str>,
        message: Value,
        inputs: Value,
    ) -> CoreResult<String> {
        self.trigger_internal(flow_id, entry_node_id, message, inputs, false, true)
    }

    fn trigger_internal(
        self: &Arc<Self>,
        flow_id: &str,
        entry: Option<&str>,
        message: Value,
        inputs: Value,
        automatic: bool,
        prefer_runtime_inputs: bool,
    ) -> CoreResult<String> {
        check_message(&message)?;
        if !inputs.is_object() {
            return Err(CoreError::new("INPUTS", "控件输入快照必须为对象"));
        }
        let inputs = if prefer_runtime_inputs {
            let state = self.state.lock().unwrap();
            let mut merged = state.inputs.clone();
            for (key, value) in inputs.as_object().unwrap() {
                merged.entry(key.clone()).or_insert_with(|| value.clone());
            }
            Value::Object(merged.into_iter().collect())
        } else {
            inputs
        };
        if serde_json::to_vec(&inputs)?.len() > MAX_MESSAGE_BYTES {
            return Err(CoreError::new("MESSAGE_LIMIT", "输入快照超过 1 MiB"));
        }
        let project = self.project();
        let flow = project
            .flows
            .iter()
            .find(|f| f.id == flow_id)
            .ok_or_else(|| CoreError::new("FLOW_MISSING", "流程不存在"))?;
        let node = match entry {
            Some(id) => flow.nodes.iter().find(|n| {
                n.id == id
                    && if automatic {
                        n.kind.starts_with("trigger.")
                    } else {
                        n.kind == "trigger.ui"
                    }
            }),
            None => {
                let nodes: Vec<_> = flow
                    .nodes
                    .iter()
                    .filter(|n| n.kind == "trigger.ui")
                    .collect();
                if nodes.len() == 1 {
                    Some(nodes[0])
                } else {
                    None
                }
            }
        }
        .ok_or_else(|| CoreError::new("ENTRY_MISSING", "找不到唯一的有效流程入口"))?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let cancel = CancellationToken::new();
        let revision = self.project_revision.load(Ordering::Acquire);
        let admission = {
            let mut state = self.state.lock().unwrap();
            if !state.started {
                return Err(CoreError::new("NOT_RUNNING", "运行会话未启动"));
            }
            if state.revision_barrier {
                return Err(CoreError::new(
                    "REVISION_WAITING",
                    "资源更新等待当前流程和设备事务结束",
                ));
            }
            let f = state.flows.get_mut(flow_id).unwrap();
            if !f.enabled {
                return Err(CoreError::new("FLOW_DISABLED", "流程已停止"));
            }
            if automatic {
                if f.auto_paused {
                    return Err(CoreError::new("TRIGGERS_PAUSED", "自动触发已暂停"));
                }
                let now = Instant::now();
                while f
                    .auto_times
                    .front()
                    .is_some_and(|t| now.duration_since(*t) >= Duration::from_secs(1))
                {
                    f.auto_times.pop_front();
                }
                if f.auto_times.len() >= 10 {
                    f.auto_paused = true;
                    Err(CoreError::new(
                        "TRIGGER_RATE",
                        "自动触发超过每秒 10 次，已暂停；请手动恢复",
                    ))
                } else {
                    f.auto_times.push_back(now);
                    Ok(())
                }
            } else {
                Ok(())
            }
            .and_then(|()| {
                if f.running.is_some() {
                    f.dropped += 1;
                    Err(CoreError::new("FLOW_BUSY", "流程正在执行"))
                } else {
                    f.running = Some((run_id.clone(), cancel.clone()));
                    self.active_runs.fetch_add(1, Ordering::AcqRel);
                    Ok(())
                }
            })
        };
        if let Err(error) = admission {
            if error.code == "TRIGGER_RATE" {
                self.emit(
                    "flow.changed",
                    Some(flow_id),
                    None,
                    None,
                    json!({"autoPaused":true,"error":error}),
                );
            }
            return Err(error);
        }
        {
            let mut state = self.state.lock().unwrap();
            state.run_revisions.insert(run_id.clone(), revision);
            state.run_tokens.insert(run_id.clone(), cancel.clone());
            state.run_flows.insert(run_id.clone(), flow_id.to_owned());
        }
        let runtime = Arc::clone(self);
        let active_run = ActiveRun(Arc::clone(self));
        let flow = flow.clone();
        let project = project.clone();
        let node_id = node.id.clone();
        let id = run_id.clone();
        self.emit(
            "run.started",
            Some(flow_id),
            Some(&run_id),
            Some(&node_id),
            json!({"message":message}),
        );
        tokio::spawn(async move {
            let _active_run = active_run;
            let outcome = tokio::select! {
                biased;
                _ = cancel.cancelled() => Err(CoreError::cancelled()),
                _ = tokio::time::sleep(Duration::from_millis(flow.timeout_ms)) => Err(CoreError::new("RUN_TIMEOUT","流程达到总截止时间")),
                result = runtime.run(&project,&flow,&node_id,message,&inputs,&id,cancel.clone()) => result,
            };
            cancel.cancel();
            {
                let mut state = runtime.state.lock().unwrap();
                if let Some(f) = state.flows.get_mut(&flow.id) {
                    if f.running
                        .as_ref()
                        .is_some_and(|(current, _)| current == &id)
                    {
                        f.running = None;
                    }
                }
            }
            match outcome {
                Ok(()) => runtime.emit("run.completed", Some(&flow.id), Some(&id), None, json!({})),
                Err(e) => runtime.emit(
                    if e.code == "CANCELLED" {
                        "run.cancelled"
                    } else {
                        "run.failed"
                    },
                    Some(&flow.id),
                    Some(&id),
                    None,
                    json!({"error":e}),
                ),
            }
            let mut state = runtime.state.lock().unwrap();
            state.run_revisions.remove(&id);
            state.run_tokens.remove(&id);
            state.run_flows.remove(&id);
        });
        Ok(run_id)
    }

    pub async fn update_point(
        self: &Arc<Self>,
        connection_id: &str,
        key: &str,
        value: Value,
        origin_flow_id: Option<&str>,
    ) {
        let key = format!("{connection_id}.{key}");
        let project = self.project();
        let inputs = {
            let state = self.state.lock().unwrap();
            Value::Object(
                state
                    .inputs
                    .iter()
                    .map(|(id, value)| (id.clone(), value.clone()))
                    .collect(),
            )
        };
        let before_properties = self.capture_custom_property_values(&project, &json!({}), &inputs);
        let changed = {
            let mut state = self.state.lock().unwrap();
            let changed = state.points.get(&key) != Some(&value);
            state.points.insert(key.clone(), value.clone());
            changed
        };
        self.emit(
            "point.changed",
            None,
            None,
            None,
            json!({"key":key,"value":value,"valid":true}),
        );
        if changed {
            self.fire_custom_property_events(
                &project,
                before_properties,
                &json!({}),
                &inputs,
                origin_flow_id,
            )
            .await;
            for flow in &self.project().flows {
                if origin_flow_id == Some(flow.id.as_str()) {
                    continue;
                }
                for node in flow.nodes.iter().filter(|n| {
                    n.kind == "trigger.data"
                        && n.config.get("key").and_then(Value::as_str) == Some(&key)
                }) {
                    let _ = self.trigger_internal(
                        &flow.id,
                        Some(&node.id),
                        json!({"payload":value,"key":key}),
                        json!({}),
                        true,
                        false,
                    );
                }
            }
        }
    }

    pub async fn update_connection(&self, id: &str, value: Value) {
        let mut invalidated = Vec::new();
        {
            let mut state = self.state.lock().unwrap();
            state.connections.insert(id.to_owned(), value.clone());
            if value
                .get("status")
                .and_then(Value::as_str)
                .is_some_and(|v| ["fault", "closed"].contains(&v))
            {
                let prefix = format!("{id}.");
                for (key, value) in state
                    .points
                    .iter_mut()
                    .filter(|(k, _)| k.starts_with(&prefix))
                {
                    *value = Value::Null;
                    invalidated.push(key.clone());
                }
            }
        }
        self.emit(
            "connection.changed",
            None,
            None,
            None,
            json!({"id":id,"state":value}),
        );
        for key in invalidated {
            self.emit(
                "point.changed",
                None,
                None,
                None,
                json!({"key":key,"value":null,"valid":false}),
            );
        }
    }

    async fn run(
        self: &Arc<Self>,
        project: &Project,
        flow: &Flow,
        entry: &str,
        message: Value,
        inputs: &Value,
        run_id: &str,
        cancel: CancellationToken,
    ) -> CoreResult<()> {
        let mut queue = VecDeque::from([(entry.to_owned(), message)]);
        let mut steps = 0;
        let deadline = Instant::now() + Duration::from_millis(flow.timeout_ms);
        while let Some((id, mut msg)) = queue.pop_front() {
            if cancel.is_cancelled() {
                return Err(CoreError::cancelled());
            }
            if Instant::now() >= deadline {
                return Err(CoreError::new("RUN_TIMEOUT", "流程达到总截止时间"));
            }
            steps += 1;
            if steps > MAX_RUN_STEPS {
                return Err(CoreError::new("STEP_LIMIT", "流程超过 1,000 次节点执行"));
            }
            check_message(&msg)?;
            let node = flow.nodes.iter().find(|n| n.id == id).unwrap();
            self.emit(
                "node.started",
                Some(&flow.id),
                Some(run_id),
                Some(&id),
                json!({"input":msg,"step":steps}),
            );
            let started = Instant::now();
            let result = self
                .execute_node(
                    project,
                    node,
                    &mut msg,
                    inputs,
                    cancel.clone(),
                    &flow.id,
                    run_id,
                )
                .await
                .and_then(|port| check_message(&msg).map(|()| port));
            let port = match result {
                Ok(port) => {
                    self.emit("node.completed",Some(&flow.id),Some(run_id),Some(&id),json!({"output":msg,"port":port,"durationMs":started.elapsed().as_millis() as u64,"step":steps}));
                    port
                }
                Err(error) => {
                    self.emit(
                        "node.failed",
                        Some(&flow.id),
                        Some(run_id),
                        Some(&id),
                        json!({"error":error,"step":steps}),
                    );
                    if ["CANCELLED", "MESSAGE_LIMIT", "QUEUE_LIMIT", "STEP_LIMIT"]
                        .contains(&error.code.as_str())
                    {
                        return Err(error);
                    }
                    if !flow
                        .edges
                        .iter()
                        .any(|e| e.source == id && e.source_port == "error")
                    {
                        return Err(error);
                    }
                    msg["error"] = json!({"code":error.code,"message":error.message,"nodeId":id});
                    Some("error".into())
                }
            };
            if let Some(port) = port {
                for edge in flow
                    .edges
                    .iter()
                    .filter(|e| e.source == id && e.source_port == port)
                {
                    if queue.len() >= MAX_RUN_QUEUE {
                        return Err(CoreError::new("QUEUE_LIMIT", "流程待执行消息超过 256 条"));
                    }
                    queue.push_back((edge.target.clone(), msg.clone()));
                }
            }
        }
        Ok(())
    }

    fn resolve(
        &self,
        project: &Project,
        reference: &ValueRef,
        message: &Value,
        inputs: &Value,
    ) -> CoreResult<Value> {
        self.resolve_inner(
            project,
            reference,
            message,
            inputs,
            &mut std::collections::HashSet::new(),
        )
    }

    fn resolve_inner(
        &self,
        project: &Project,
        reference: &ValueRef,
        message: &Value,
        inputs: &Value,
        properties: &mut std::collections::HashSet<(String, String)>,
    ) -> CoreResult<Value> {
        let result = match reference {
            ValueRef::Literal { value } => Some(value.clone()),
            ValueRef::Message { path } => get_path(message, path).cloned(),
            ValueRef::Input { key } => inputs.get(key).cloned(),
            ValueRef::Variable { key } => self.state.lock().unwrap().variables.get(key).cloned(),
            ValueRef::Point { key } => self
                .state
                .lock()
                .unwrap()
                .points
                .get(key)
                .filter(|v| !v.is_null())
                .cloned(),
            ValueRef::Property { key, property } => {
                let widget = project
                    .pages
                    .iter()
                    .flat_map(|page| page.widgets.iter())
                    .find(|widget| &widget.id == key)
                    .ok_or_else(|| CoreError::new("WIDGET_MISSING", "界面组件不存在"))?;
                if !properties.insert((key.clone(), property.clone())) {
                    return Err(CoreError::new("PROPERTY_CYCLE", "界面属性绑定存在循环"));
                }
                let binding = widget
                    .custom_properties
                    .get(property)
                    .and_then(|entry| entry.binding.as_ref())
                    .or_else(|| widget.bindings.get(property));
                let resolved = if let Some(binding) = binding {
                    self.resolve_inner(project, binding, message, inputs, properties)
                } else if let Some(current) = self
                    .state
                    .lock()
                    .unwrap()
                    .ui
                    .get(&format!("{key}.{property}"))
                    .cloned()
                {
                    Ok(current)
                } else {
                    widget
                        .custom_properties
                        .get(property)
                        .map(|entry| entry.value.clone())
                        .or_else(|| widget.props.get(property).cloned())
                        .ok_or_else(|| CoreError::new("UI_PROPERTY_MISSING", "组件没有此属性"))
                };
                properties.remove(&(key.clone(), property.clone()));
                return resolved;
            }
        };
        result.ok_or_else(|| CoreError::new("VALUE_MISSING", "引用值不存在或数据点失效"))
    }

    fn capture_custom_property_values(
        &self,
        project: &Project,
        message: &Value,
        inputs: &Value,
    ) -> HashMap<(String, String), Option<Value>> {
        let mut values = HashMap::new();
        for widget in project.pages.iter().flat_map(|page| page.widgets.iter()) {
            for (name, property) in &widget.custom_properties {
                if property.event.is_some() {
                    let reference = ValueRef::Property {
                        key: widget.id.clone(),
                        property: name.clone(),
                    };
                    let value = self.resolve(project, &reference, message, inputs).ok();
                    values.insert((widget.id.clone(), name.clone()), value);
                }
            }
        }
        values
    }

    async fn fire_custom_property_events(
        self: &Arc<Self>,
        project: &Project,
        before: HashMap<(String, String), Option<Value>>,
        message: &Value,
        inputs: &Value,
        origin_flow_id: Option<&str>,
    ) {
        let after = self.capture_custom_property_values(project, message, inputs);
        for widget in project.pages.iter().flat_map(|page| page.widgets.iter()) {
            for (property_name, property) in &widget.custom_properties {
                let Some(action) = property.event.as_ref() else {
                    continue;
                };
                let key = (widget.id.clone(), property_name.clone());
                let old = before.get(&key).cloned().unwrap_or(None);
                let new = after.get(&key).cloned().unwrap_or(None);
                if old == new || new.is_none() || origin_flow_id == Some(action.flow_id.as_str()) {
                    continue;
                }
                let event_message = json!({"payload":{"widgetId":widget.id,"property":property_name,"oldValue":old,"value":new}});
                let latest_inputs = {
                    let state = self.state.lock().unwrap();
                    let mut merged = state.inputs.clone();
                    if let Some(values) = inputs.as_object() {
                        for (id, value) in values {
                            merged.entry(id.clone()).or_insert_with(|| value.clone());
                        }
                    }
                    Value::Object(merged.into_iter().collect())
                };
                let result = self.trigger_internal(
                    &action.flow_id,
                    action.entry_node_id.as_deref(),
                    event_message,
                    latest_inputs,
                    true,
                    true,
                );
                let _ = result;
            }
        }
    }

    pub async fn update_input_with_events(self: &Arc<Self>, id: String, value: Value) {
        let project = self.project();
        let inputs = {
            let state = self.state.lock().unwrap();
            Value::Object(
                state
                    .inputs
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        };
        let before = self.capture_custom_property_values(&project, &json!({}), &inputs);
        self.update_input(id, value);
        let after_inputs = {
            let state = self.state.lock().unwrap();
            Value::Object(
                state
                    .inputs
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            )
        };
        self.fire_custom_property_events(&project, before, &json!({}), &after_inputs, None)
            .await;
    }

    async fn execute_node(
        self: &Arc<Self>,
        project: &Project,
        node: &FlowNode,
        message: &mut Value,
        inputs: &Value,
        cancel: CancellationToken,
        flow_id: &str,
        run_id: &str,
    ) -> CoreResult<Option<String>> {
        let text = |key: &str| node.config.get(key).and_then(Value::as_str).unwrap_or("");
        let number = |key: &str, default| {
            node.config
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or(default)
        };
        let reference = |key: &str| -> CoreResult<Value> {
            let r: ValueRef =
                serde_json::from_value(node.config.get(key).cloned().unwrap_or(Value::Null))?;
            self.resolve(project, &r, message, inputs)
        };
        match node.kind.as_str() {
            k if k.starts_with("trigger.") => (),
            "connection.open" => self.host.open_connection(text("connectionId")).await?,
            "connection.close" => self.host.close_connection(text("connectionId")).await?,
            "protocol.command" => {
                let mut params = Map::new();
                if let Some(config) = node.config.get("params").and_then(Value::as_object) {
                    for (k,v) in config { params.insert(k.clone(),self.resolve(project,&serde_json::from_value(v.clone())?,message,inputs)?); }
                }
                message["payload"] = self.host.command_with_origin(text("connectionId"),text("command"),Value::Object(params),cancel,flow_id).await?;
            }
            "condition" => return Ok(Some(if compare(&reference("left")?,text("operator"),&reference("right")?)? {"true"} else {"false"}.into())),
            "assign" => { let value = reference("value")?; set_path(message,text("path"),value)?; }
            "variable.get" => {
                let value = self.state.lock().unwrap().variables.get(text("variableId")).cloned().ok_or_else(||CoreError::new("VARIABLE_MISSING","变量不存在"))?;
                set_path(message,node.config.get("path").and_then(Value::as_str).unwrap_or("payload"),value)?;
            }
            "variable.set" => {
                let value = reference("value")?; let id = text("variableId");
                let variable = project.variables.iter().find(|v|v.id == id).unwrap();
                if !variable.accepts(&value) { return Err(CoreError::new("VARIABLE_TYPE","值不符合变量类型")); }
                let before_properties = self.capture_custom_property_values(project, message, inputs);
                self.state.lock().unwrap().variables.insert(id.into(),value.clone());
                self.emit("variable.changed",Some(flow_id),Some(run_id),Some(&node.id),json!({"key":id,"value":value}));
                self.fire_custom_property_events(project, before_properties, message, inputs, Some(flow_id)).await;
            }
            "ui.setProperty" => {
                let widget_id = text("widgetId");
                let property = text("property");
                if property.is_empty() { return Err(CoreError::new("UI_PROPERTY", "界面属性名不能为空")); }
                let value = reference("value")?;
                let widget = project.pages.iter().flat_map(|page| page.widgets.iter())
                    .find(|widget| widget.id == widget_id)
                    .ok_or_else(|| CoreError::new("WIDGET_MISSING", "界面组件不存在"))?;
                if !std::ptr::eq(project, self.project().as_ref()) {
                    // A run admitted on an older revision may finish its business work,
                    // but its UI write must never overwrite the newer interface.
                    self.emit("ui.property.ignored",Some(flow_id),Some(run_id),Some(&node.id),json!({"widgetId":widget_id,"property":property,"revision":"stale"}));
                    return Ok(Some("out".into()));
                }
                let custom = widget.custom_properties.get(property);
                if widget.bindings.contains_key(property) || custom.is_some_and(|entry| entry.binding.is_some()) {
                    return Err(CoreError::new("UI_PROPERTY_BOUND", "已绑定属性应通过绑定变量更新"));
                }
                if !widget.props.contains_key(property) && custom.is_none() {
                    return Err(CoreError::new("UI_PROPERTY_MISSING", "组件没有此属性"));
                }
                if let Some(custom) = custom {
                    let matches_type = match custom.value_type.as_str() {
                        "text" | "string" => value.is_string(),
                        "number" => value.as_f64().is_some(),
                        "boolean" | "bool" => value.is_boolean(),
                        "enum" => value.as_str().is_some_and(|item| custom.options.iter().any(|option| option == item)),
                        "object" => value.is_object(),
                        "array" => value.is_array(),
                        _ => false,
                    };
                    if !matches_type { return Err(CoreError::new("UI_PROPERTY_TYPE", "值不符合界面属性类型")); }
                }
                let key = format!("{widget_id}.{property}");
                let previous = self.resolve(project, &ValueRef::Property { key: widget_id.into(), property: property.into() }, message, inputs).ok();
                if previous.as_ref() == Some(&value) { return Ok(Some("out".into())); }
                let before_properties = self.capture_custom_property_values(project, message, inputs);
                self.state.lock().unwrap().ui.insert(key.clone(), value.clone());
                self.emit("ui.property.changed",Some(flow_id),Some(run_id),Some(&node.id),json!({"widgetId":widget_id,"property":property,"value":value}));
                self.fire_custom_property_events(project, before_properties, message, inputs, Some(flow_id)).await;
            }
            "delay" => tokio::time::sleep(Duration::from_millis(number("durationMs",0))).await,
            "python" => match self.host.python(text("code"),message.clone(),number("timeoutMs",5000),number("memoryMb",256),cancel).await? {
                Some(output) => { check_message(&output)?; *message = output; }
                None => return Ok(None),
            },
            "page.navigate" => {
                let current = self.project();
                if std::ptr::eq(project, current.as_ref()) {
                    self.emit("page.changed",Some(flow_id),Some(run_id),Some(&node.id),json!({"pageId":text("pageId")}));
                }
            }
            "log" => self.emit("log",Some(flow_id),Some(run_id),Some(&node.id),json!({"level":node.config.get("level").and_then(Value::as_str).unwrap_or("info"),"value":reference("value")?})),
            _ => return Err(CoreError::new("NODE_KIND","不支持此节点类型")),
        }
        Ok(Some("out".into()))
    }
}

pub fn check_message(message: &Value) -> CoreResult<()> {
    if !message.is_object() {
        return Err(CoreError::new("MESSAGE_TYPE", "业务消息必须为 JSON 对象"));
    }
    if serde_json::to_vec(message)?.len() > MAX_MESSAGE_BYTES {
        return Err(CoreError::new("MESSAGE_LIMIT", "业务消息超过 1 MiB"));
    }
    Ok(())
}

pub fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value, |v, part| v.get(part))
}

pub fn set_path(value: &mut Value, path: &str, replacement: Value) -> CoreResult<()> {
    if !crate::valid_message_path(path) {
        return Err(CoreError::new("MESSAGE_PATH", "消息字段路径无效"));
    }
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = value;
    for part in &parts[..parts.len() - 1] {
        let object = current
            .as_object_mut()
            .ok_or_else(|| CoreError::new("MESSAGE_PATH", "中间字段不是对象"))?;
        current = object
            .entry((*part).to_owned())
            .or_insert_with(|| json!({}));
    }
    current
        .as_object_mut()
        .ok_or_else(|| CoreError::new("MESSAGE_PATH", "目标父字段不是对象"))?
        .insert(parts.last().unwrap().to_string(), replacement);
    Ok(())
}

fn compare(left: &Value, operator: &str, right: &Value) -> CoreResult<bool> {
    Ok(match operator {
        "eq" => left == right,
        "ne" => left != right,
        "contains" => match (left, right) {
            (Value::String(a), Value::String(b)) => a.contains(b),
            (Value::Array(a), b) => a.contains(b),
            _ => return Err(CoreError::new("CONDITION_TYPE", "contains 需要文本或数组")),
        },
        "gt" | "gte" | "lt" | "lte" => {
            let a = left
                .as_f64()
                .ok_or_else(|| CoreError::new("CONDITION_TYPE", "数值比较需要数字"))?;
            let b = right
                .as_f64()
                .ok_or_else(|| CoreError::new("CONDITION_TYPE", "数值比较需要数字"))?;
            match operator {
                "gt" => a > b,
                "gte" => a >= b,
                "lt" => a < b,
                _ => a <= b,
            }
        }
        _ => return Err(CoreError::new("CONDITION_OPERATOR", "不支持此比较符")),
    })
}
