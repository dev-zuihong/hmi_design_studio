use crate::{validate_project, CoreError, CoreResult, Flow, FlowNode, Project, ValueRef};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{Arc, Mutex},
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
        }
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub variables: BTreeMap<String, Value>,
    pub points: BTreeMap<String, Value>,
    pub connections: BTreeMap<String, Value>,
    pub flows: BTreeMap<String, Value>,
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
    variables: BTreeMap<String, Value>,
    points: BTreeMap<String, Value>,
    connections: BTreeMap<String, Value>,
    flows: HashMap<String, FlowState>,
    logs: VecDeque<RuntimeEvent>,
}

pub struct Runtime {
    project: Arc<Project>,
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
            variables: project
                .variables
                .iter()
                .map(|v| (v.id.clone(), v.initial.clone()))
                .collect(),
            points: BTreeMap::new(),
            connections: BTreeMap::new(),
            logs: VecDeque::new(),
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
            project: Arc::new(project),
            host,
            state: Mutex::new(state),
            events,
            active_runs: AtomicUsize::new(0),
            runs_finished: Notify::new(),
        }))
    }

    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub async fn snapshot(&self) -> RuntimeSnapshot {
        let state = self.state.lock().unwrap();
        RuntimeSnapshot {
            variables: state.variables.clone(), points: state.points.clone(), connections: state.connections.clone(),
            flows: state.flows.iter().map(|(id,f)|(id.clone(),json!({
                "enabled": f.enabled, "busy": f.running.is_some(), "autoPaused": f.auto_paused,
                "dropped": f.dropped, "runId": f.running.as_ref().map(|(id,_)|id),
            }))).collect(),
            logs: state.logs.iter().cloned().collect(),
        }
    }

    pub fn publish_event(&self, event: RuntimeEvent) {
        {
            let mut state = self.state.lock().unwrap();
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
        for f in &self.project.flows {
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
        let ids: Vec<String> = self.project.flows.iter().map(|f| f.id.clone()).collect();
        for id in ids {
            let _ = self.stop_flow(&id).await;
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
        let flow = self
            .project
            .flows
            .iter()
            .find(|f| f.id == id)
            .ok_or_else(|| CoreError::new("FLOW_MISSING", "流程不存在"))?;
        let token = {
            let mut state = self.state.lock().unwrap();
            if !state.started {
                return Err(CoreError::new("NOT_RUNNING", "请先启动运行会话"));
            }
            let f = state.flows.get_mut(id).unwrap();
            f.trigger_cancel.cancel();
            f.trigger_cancel = CancellationToken::new();
            f.enabled = true;
            f.auto_paused = false;
            f.auto_times.clear();
            f.trigger_cancel.clone()
        };
        for node in flow.nodes.iter().filter(|n| n.kind == "trigger.timer") {
            let weak = Arc::downgrade(self);
            let cancel = token.clone();
            let flow_id = id.to_owned();
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
                            let _ = runtime.trigger_internal(&flow_id, Some(&node_id), json!({"payload":null}), Value::Object(Map::new()), true);
                        }
                    }
                }
            });
        }
        self.emit(
            "flow.changed",
            Some(id),
            None,
            None,
            json!({"enabled":true,"autoPaused":false}),
        );
        Ok(())
    }

    pub async fn stop_flow(&self, id: &str) -> CoreResult<()> {
        {
            let mut state = self.state.lock().unwrap();
            let f = state
                .flows
                .get_mut(id)
                .ok_or_else(|| CoreError::new("FLOW_MISSING", "流程不存在"))?;
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
        self.trigger_internal(flow_id, entry_node_id, message, inputs, false)
    }

    fn trigger_internal(
        self: &Arc<Self>,
        flow_id: &str,
        entry: Option<&str>,
        message: Value,
        inputs: Value,
        automatic: bool,
    ) -> CoreResult<String> {
        check_message(&message)?;
        if !inputs.is_object() {
            return Err(CoreError::new("INPUTS", "控件输入快照必须为对象"));
        }
        if serde_json::to_vec(&inputs)?.len() > MAX_MESSAGE_BYTES {
            return Err(CoreError::new("MESSAGE_LIMIT", "输入快照超过 1 MiB"));
        }
        let flow = self
            .project
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
        let admission = {
            let mut state = self.state.lock().unwrap();
            if !state.started {
                return Err(CoreError::new("NOT_RUNNING", "运行会话未启动"));
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
        let runtime = Arc::clone(self);
        let active_run = ActiveRun(Arc::clone(self));
        let flow = flow.clone();
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
                result = runtime.run(&flow,&node_id,message,&inputs,&id,cancel.clone()) => result,
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
            for flow in &self.project.flows {
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
        &self,
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
                .execute_node(node, &mut msg, inputs, cancel.clone(), &flow.id, run_id)
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

    fn resolve(&self, reference: &ValueRef, message: &Value, inputs: &Value) -> CoreResult<Value> {
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
        };
        result.ok_or_else(|| CoreError::new("VALUE_MISSING", "引用值不存在或数据点失效"))
    }

    async fn execute_node(
        &self,
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
            self.resolve(&r, message, inputs)
        };
        match node.kind.as_str() {
            k if k.starts_with("trigger.") => (),
            "connection.open" => self.host.open_connection(text("connectionId")).await?,
            "connection.close" => self.host.close_connection(text("connectionId")).await?,
            "protocol.command" => {
                let mut params = Map::new();
                if let Some(config) = node.config.get("params").and_then(Value::as_object) {
                    for (k,v) in config { params.insert(k.clone(),self.resolve(&serde_json::from_value(v.clone())?,message,inputs)?); }
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
                let variable = self.project.variables.iter().find(|v|v.id == id).unwrap();
                if !variable.accepts(&value) { return Err(CoreError::new("VARIABLE_TYPE","值不符合变量类型")); }
                self.state.lock().unwrap().variables.insert(id.into(),value.clone());
                self.emit("variable.changed",Some(flow_id),Some(run_id),Some(&node.id),json!({"key":id,"value":value}));
            }
            "delay" => tokio::time::sleep(Duration::from_millis(number("durationMs",0))).await,
            "python" => match self.host.python(text("code"),message.clone(),number("timeoutMs",5000),number("memoryMb",256),cancel).await? {
                Some(output) => { check_message(&output)?; *message = output; }
                None => return Ok(None),
            },
            "page.navigate" => self.emit("page.changed",Some(flow_id),Some(run_id),Some(&node.id),json!({"pageId":text("pageId")})),
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
