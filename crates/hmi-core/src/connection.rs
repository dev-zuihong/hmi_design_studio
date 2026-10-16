use crate::{
    protocol::{load_plugin_manifest, RpcClient},
    runtime::RuntimeEvent,
    ConnectionConfig, CoreError, CoreResult,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Serialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex as StdMutex, RwLock as StdRwLock,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{broadcast, mpsc, oneshot, Mutex},
};
use tokio_serial::SerialPortBuilderExt;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Serialize)]
pub struct SerialPortInfo {
    pub name: String,
    pub description: String,
}

pub fn list_serial_ports() -> CoreResult<Vec<SerialPortInfo>> {
    tokio_serial::available_ports()
        .map(|ports| {
            ports
                .into_iter()
                .map(|p| {
                    let description = match p.port_type {
                        tokio_serial::SerialPortType::UsbPort(info) => {
                            [info.manufacturer, info.product, info.serial_number]
                                .into_iter()
                                .flatten()
                                .collect::<Vec<_>>()
                                .join(" ")
                        }
                        tokio_serial::SerialPortType::BluetoothPort => "Bluetooth".into(),
                        _ => "Serial port".into(),
                    };
                    SerialPortInfo {
                        name: p.port_name,
                        description,
                    }
                })
                .collect()
        })
        .map_err(|e| CoreError::new("SERIAL_ENUMERATE", e.to_string()))
}

struct Request {
    name: String,
    params: Value,
    cancel: CancellationToken,
    origin_flow_id: Option<String>,
    reply: oneshot::Sender<CoreResult<Value>>,
}

struct Session {
    rpc: Arc<RpcClient>,
    cancel: CancellationToken,
    sender: mpsc::Sender<Request>,
    faulted: AtomicBool,
}

struct Registry {
    configs: HashMap<String, ConnectionConfig>,
    slots: HashMap<String, Arc<Mutex<Option<Arc<Session>>>>>,
    generations: HashMap<String, Arc<AtomicU64>>,
    root: PathBuf,
}

pub struct ConnectionManager {
    registry: StdRwLock<Registry>,
    ports: StdMutex<HashSet<String>>,
    events: broadcast::Sender<RuntimeEvent>,
    maintenance: Mutex<()>,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        // Orderly shutdown uses close_all. This fallback also releases idle plugin
        // tasks if a caller drops the manager after an earlier setup failure.
        let Ok(registry) = self.registry.try_read() else {
            return;
        };
        for slot in registry.slots.values() {
            if let Ok(mut session) = slot.try_lock() {
                if let Some(session) = session.take() {
                    session.cancel.cancel();
                }
            }
        }
    }
}

impl ConnectionManager {
    pub fn new(connections: Vec<ConnectionConfig>, plugins_root: impl Into<PathBuf>) -> Arc<Self> {
        let (events, _) = broadcast::channel(1024);
        let slots = connections
            .iter()
            .map(|c| (c.id.clone(), Arc::new(Mutex::new(None))))
            .collect();
        let generations = connections
            .iter()
            .map(|c| (c.id.clone(), Arc::new(AtomicU64::new(0))))
            .collect();
        Arc::new(Self {
            registry: StdRwLock::new(Registry {
                configs: connections.into_iter().map(|c| (c.id.clone(), c)).collect(),
                slots,
                generations,
                root: plugins_root.into(),
            }),
            ports: StdMutex::new(HashSet::new()),
            events,
            maintenance: Mutex::new(()),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub fn configs(&self) -> Vec<ConnectionConfig> {
        let mut configs: Vec<_> = self
            .registry
            .read()
            .unwrap()
            .configs
            .values()
            .cloned()
            .collect();
        configs.sort_by(|a, b| a.id.cmp(&b.id));
        configs
    }

    fn emit(&self, kind: &str, data: Value) {
        let _ = self.events.send(RuntimeEvent::new(kind, data));
    }

    fn state(&self, id: &str, status: &str, simulated: bool, error: Option<CoreError>) {
        self.emit(
            "connection.changed",
            json!({"id":id,"state":{"status":status,"simulated":simulated,"error":error}}),
        );
    }

    pub async fn open(self: &Arc<Self>, id: &str) -> CoreResult<()> {
        // Opening has ownership outside any one flow. Dropping a flow future cannot leak
        // a half-open plugin or leave a port without a registered session.
        let generation = self
            .registry
            .read()
            .unwrap()
            .generations
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?;
        let expected_generation = generation.load(Ordering::Acquire);
        let manager = Arc::clone(self);
        let id = id.to_owned();
        tokio::spawn(async move {
            manager
                .open_inner(&id, generation, expected_generation)
                .await
        })
        .await
        .map_err(|e| CoreError::new("CONNECTION_TASK", e.to_string()))?
    }

    async fn open_inner(
        self: &Arc<Self>,
        id: &str,
        generation: Arc<AtomicU64>,
        expected_generation: u64,
    ) -> CoreResult<()> {
        let (config, slot) = {
            let registry = self.registry.read().unwrap();
            let config = registry
                .configs
                .get(id)
                .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?
                .clone();
            let slot = registry.slots.get(id).cloned().unwrap();
            if !registry
                .generations
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, &generation))
            {
                return Err(CoreError::cancelled());
            }
            (config, slot)
        };
        let mut guard = slot.lock().await;
        if !self.is_current_generation(id, &generation, expected_generation) {
            return Err(CoreError::cancelled());
        }
        if let Some(session) = guard.as_ref() {
            if session.faulted.load(Ordering::Acquire) {
                return Err(CoreError::new(
                    "CONNECTION_FAULT",
                    "连接故障，请关闭后重新打开",
                ));
            }
            return Ok(());
        }
        let simulated = config.port == "sim://temperature";
        let port_key = config.port.to_uppercase();
        if !simulated && !self.ports.lock().unwrap().insert(port_key.clone()) {
            return Err(CoreError::new("PORT_IN_USE", "另一个连接已占用此串口"));
        }
        self.state(id, "opening", simulated, None);
        let result = self.build_session(config.clone()).await;
        match result {
            Ok(session) => {
                if !self.is_current_generation(id, &generation, expected_generation) {
                    session.cancel.cancel();
                    session.rpc.shutdown().await;
                    if !simulated {
                        self.ports.lock().unwrap().remove(&port_key);
                    }
                    return Err(CoreError::cancelled());
                }
                *guard = Some(session);
                self.state(id, "open", simulated, None);
                Ok(())
            }
            Err(error) => {
                if !simulated {
                    self.ports.lock().unwrap().remove(&port_key);
                }
                if !self.is_current_generation(id, &generation, expected_generation) {
                    return Err(CoreError::cancelled());
                }
                self.state(id, "fault", simulated, Some(error.clone()));
                Err(error)
            }
        }
    }

    fn is_current_generation(&self, id: &str, generation: &Arc<AtomicU64>, expected: u64) -> bool {
        generation.load(Ordering::Acquire) == expected
            && self
                .registry
                .read()
                .unwrap()
                .generations
                .get(id)
                .is_some_and(|current| Arc::ptr_eq(current, generation))
    }

    async fn build_session(self: &Arc<Self>, config: ConnectionConfig) -> CoreResult<Arc<Session>> {
        let root = self.registry.read().unwrap().root.clone();
        let directory = root
            .join(&config.protocol_id)
            .canonicalize()
            .map_err(|e| CoreError::new("PLUGIN_MISSING", e.to_string()))?;
        let manifest = load_plugin_manifest(&directory.join("manifest.json"))?;
        crate::protocol::validate_schema(
            &manifest.config_schema,
            &serde_json::to_value(&config.protocol_config)?,
        )
        .map_err(|error| CoreError::new("PROTOCOL_CONFIG", error.message))?;
        if manifest.id != config.protocol_id {
            return Err(CoreError::new("PLUGIN_ID", "插件 ID 与目录不一致"));
        }
        if config
            .protocol_version
            .as_ref()
            .is_some_and(|v| v != &manifest.version)
        {
            return Err(CoreError::new(
                "PLUGIN_VERSION",
                "协议插件版本与项目锁定版本不匹配",
            ));
        }
        let executable = directory
            .join(&manifest.executable)
            .canonicalize()
            .map_err(|e| CoreError::new("PLUGIN_MISSING", e.to_string()))?;
        if !executable.starts_with(&directory) {
            return Err(CoreError::new("PLUGIN_PATH", "插件路径超出插件目录"));
        }
        let simulated = config.port == "sim://temperature";
        let serial = if simulated {
            None
        } else {
            let bits = match config.data_bits {
                5 => tokio_serial::DataBits::Five,
                6 => tokio_serial::DataBits::Six,
                7 => tokio_serial::DataBits::Seven,
                8 => tokio_serial::DataBits::Eight,
                _ => return Err(CoreError::new("SERIAL_CONFIG", "无效数据位")),
            };
            let parity = match config.parity.as_str() {
                "none" => tokio_serial::Parity::None,
                "odd" => tokio_serial::Parity::Odd,
                "even" => tokio_serial::Parity::Even,
                _ => return Err(CoreError::new("SERIAL_CONFIG", "无效校验位")),
            };
            let stop = match config.stop_bits {
                1 => tokio_serial::StopBits::One,
                2 => tokio_serial::StopBits::Two,
                _ => return Err(CoreError::new("SERIAL_CONFIG", "无效停止位")),
            };
            Some(
                tokio_serial::new(&config.port, config.baud_rate)
                    .data_bits(bits)
                    .parity(parity)
                    .stop_bits(stop)
                    .open_native_async()
                    .map_err(|e| CoreError::new("SERIAL_OPEN", e.to_string()))?,
            )
        };
        let (rpc, mut inbound) = RpcClient::spawn(executable, &manifest.args, &directory).await?;
        let (sender, mut requests) = mpsc::channel::<Request>(64);
        let session = Arc::new(Session {
            rpc: rpc.clone(),
            cancel: CancellationToken::new(),
            sender,
            faulted: AtomicBool::new(false),
        });
        let (reader, writer) = match serial {
            Some(serial) => {
                let (r, w) = tokio::io::split(serial);
                (Some(r), Some(w))
            }
            None => (None, None),
        };
        let weak_manager = Arc::downgrade(self);
        let weak_session = Arc::downgrade(&session);
        let id = config.id.clone();
        let cancel = session.cancel.clone();
        let rpc_events = rpc.clone();
        let point_types: HashMap<_, _> = manifest
            .points
            .iter()
            .map(|p| (p.key.clone(), p.value_type.clone()))
            .collect();
        tokio::spawn(async move {
            let mut writer = writer;
            loop {
                let event = tokio::select! { biased; _=cancel.cancelled()=>break, event=inbound.recv()=>match event {Some(e)=>e,None=>break} };
                let Some(manager) = weak_manager.upgrade() else {
                    break;
                };
                let result = match event.method.as_str() {
                    "transport.write" => {
                        match event
                            .params
                            .get("bytes")
                            .and_then(Value::as_str)
                            .and_then(|s| BASE64.decode(s).ok())
                        {
                            Some(bytes) if bytes.len() <= 64 * 1024 => {
                                if let Some(writer) = writer.as_mut() {
                                    match tokio::time::timeout(Duration::from_secs(2), async {
                                        writer.write_all(&bytes).await?;
                                        writer.flush().await
                                    })
                                    .await
                                    {
                                        Ok(Ok(())) => Ok(json!({"written":bytes.len()})),
                                        Ok(Err(e)) => {
                                            Err(CoreError::new("SERIAL_WRITE", e.to_string()))
                                        }
                                        Err(_) => {
                                            Err(CoreError::new("SERIAL_WRITE", "串口写入超时"))
                                        }
                                    }
                                } else {
                                    Err(CoreError::new(
                                        "SIMULATION_TRANSPORT",
                                        "模拟连接不发送真实字节",
                                    ))
                                }
                            }
                            _ => Err(CoreError::new(
                                "TRANSPORT_BYTES",
                                "字节必须为不超过64KiB的Base64",
                            )),
                        }
                    }
                    "point.publish" => {
                        if let (Some(key), Some(value)) = (
                            event.params.get("key").and_then(Value::as_str),
                            event.params.get("value"),
                        ) {
                            let valid = point_types.get(key).is_some_and(|t| match t.as_str() {
                                "number" => value.is_number(),
                                "boolean" => value.is_boolean(),
                                "string" | "enum" => value.is_string(),
                                _ => false,
                            });
                            if valid {
                                manager.emit("point.changed",json!({"connectionId":id,"key":key,"value":value,"simulated":simulated,"originFlowId":event.params.get("originFlowId")}));
                            }
                        }
                        Ok(Value::Null)
                    }
                    "log" => {
                        manager.emit(
                            "log",
                            json!({"connectionId":id,"source":"plugin","detail":event.params}),
                        );
                        Ok(Value::Null)
                    }
                    "process.exited" => {
                        if let Some(session) = weak_session.upgrade() {
                            session.faulted.store(true, Ordering::Release);
                        }
                        manager.state(
                            &id,
                            "fault",
                            simulated,
                            Some(CoreError::new("PLUGIN_EXITED", "协议插件已退出")),
                        );
                        break;
                    }
                    _ => Err(CoreError::new("PLUGIN_METHOD", "不支持的插件宿主调用")),
                };
                if let Some(id) = event.id {
                    rpc_events.respond(id, result).await;
                } else if let Err(error) = result {
                    manager.emit(
                        "log",
                        json!({"connectionId":id,"level":"error","error":error}),
                    );
                }
            }
        });
        if let Some(mut reader) = reader {
            let weak_manager = Arc::downgrade(self);
            let weak_session = Arc::downgrade(&session);
            let id = config.id.clone();
            let cancel = session.cancel.clone();
            let rpc = rpc.clone();
            tokio::spawn(async move {
                let mut bytes = [0u8; 4096];
                loop {
                    let result = tokio::select! {biased;_=cancel.cancelled()=>break,result=reader.read(&mut bytes)=>result};
                    match result {
                        Ok(n) if n > 0 => {
                            if rpc
                                .notify(
                                    "transport.data",
                                    json!({"bytes":BASE64.encode(&bytes[..n])}),
                                )
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                        outcome => {
                            if let Some(session) = weak_session.upgrade() {
                                session.faulted.store(true, Ordering::Release);
                            }
                            if let Some(manager) = weak_manager.upgrade() {
                                manager.state(
                                    &id,
                                    "fault",
                                    false,
                                    Some(CoreError::new(
                                        "SERIAL_READ",
                                        outcome
                                            .err()
                                            .map(|e| e.to_string())
                                            .unwrap_or_else(|| "串口已断开".into()),
                                    )),
                                );
                            }
                            break;
                        }
                    }
                }
            });
        }
        let initialized=rpc.call("initialize",json!({"apiVersion":1,"protocolId":manifest.id,"config":config.protocol_config,"simulation":simulated}),Duration::from_secs(5)).await;
        if initialized
            .as_ref()
            .ok()
            .and_then(|v| v.get("apiVersion"))
            .and_then(Value::as_u64)
            != Some(1)
        {
            session.cancel.cancel();
            rpc.shutdown().await;
            return Err(initialized
                .err()
                .unwrap_or_else(|| CoreError::new("PLUGIN_API", "插件 API 版本不兼容")));
        }
        let safe_timeout = initialized
            .as_ref()
            .ok()
            .and_then(|v| v.get("lateResponsesDiscarded"))
            .and_then(Value::as_bool)
            == Some(true);
        let commands: HashMap<_, _> = manifest
            .commands
            .iter()
            .map(|c| (c.name.clone(), c.params.clone()))
            .collect();
        let weak_manager = Arc::downgrade(self);
        let weak_session = Arc::downgrade(&session);
        let id = config.id.clone();
        let cancel = session.cancel.clone();
        let timeout_ms = config
            .protocol_config
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .unwrap_or(2000)
            .clamp(10, 60_000);
        tokio::spawn(async move {
            loop {
                let request = tokio::select! {biased;_=cancel.cancelled()=>break,request=requests.recv()=>match request{Some(r)=>r,None=>break}};
                if request.cancel.is_cancelled() || request.reply.is_closed() {
                    let _ = request.reply.send(Err(CoreError::cancelled()));
                    continue;
                }
                let Some(session) = weak_session.upgrade() else {
                    break;
                };
                if session.faulted.load(Ordering::Acquire) {
                    let _ = request.reply.send(Err(CoreError::new(
                        "CONNECTION_FAULT",
                        "连接故障，请手动恢复",
                    )));
                    continue;
                }
                if !commands.contains_key(&request.name) {
                    let _ = request
                        .reply
                        .send(Err(CoreError::new("COMMAND_MISSING", "插件未声明此命令")));
                    continue;
                }
                if let Err(error) =
                    crate::protocol::validate_schema(&commands[&request.name], &request.params)
                {
                    let _ = request
                        .reply
                        .send(Err(CoreError::new("PARAMETERS", error.message)));
                    continue;
                }
                // Deliberately no per-flow cancellation selection here. After dispatch,
                // this actor owns the transaction until response/timeout/resynchronization.
                let result=rpc.call("command.execute",json!({"command":request.name,"params":request.params,"timeoutMs":timeout_ms,"originFlowId":request.origin_flow_id}),Duration::from_millis(timeout_ms+1000)).await;
                let fault = result.as_ref().err().is_some_and(|e| {
                    ["PLUGIN_TIMEOUT", "PLUGIN_EXITED", "SERIAL_WRITE"].contains(&e.code.as_str())
                        || e.code == "DEVICE_TIMEOUT" && !safe_timeout
                });
                if fault && !cancel.is_cancelled() {
                    session.faulted.store(true, Ordering::Release);
                    if let Some(manager) = weak_manager.upgrade() {
                        manager.state(&id, "fault", simulated, result.as_ref().err().cloned());
                    }
                }
                let _ = request.reply.send(result);
            }
            while let Ok(request) = requests.try_recv() {
                let _ = request
                    .reply
                    .send(Err(CoreError::new("CONNECTION_CLOSED", "连接已关闭")));
            }
        });
        Ok(session)
    }

    pub async fn command(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        self.command_with_origin(id, name, params, cancel, None)
            .await
    }

    pub async fn command_with_origin(
        &self,
        id: &str,
        name: &str,
        params: Value,
        cancel: CancellationToken,
        origin_flow_id: Option<&str>,
    ) -> CoreResult<Value> {
        let slot = self
            .registry
            .read()
            .unwrap()
            .slots
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?;
        let session = slot
            .lock()
            .await
            .clone()
            .ok_or_else(|| CoreError::new("CONNECTION_CLOSED", "请先打开串口连接"))?;
        if session.faulted.load(Ordering::Acquire) {
            return Err(CoreError::new("CONNECTION_FAULT", "连接故障，请手动恢复"));
        }
        if cancel.is_cancelled() {
            return Err(CoreError::cancelled());
        }
        let (reply, receive) = oneshot::channel();
        session
            .sender
            .try_send(Request {
                name: name.into(),
                params,
                cancel: cancel.clone(),
                origin_flow_id: origin_flow_id.map(str::to_owned),
                reply,
            })
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => {
                    CoreError::new("COMMAND_QUEUE_FULL", "连接命令队列已满（64）")
                }
                _ => CoreError::new("CONNECTION_CLOSED", "连接已关闭"),
            })?;
        tokio::select! {biased;_=cancel.cancelled()=>Err(CoreError::cancelled()),result=receive=>result.map_err(|_|CoreError::new("CONNECTION_CLOSED","连接命令执行器已退出"))?}
    }

    pub async fn close(self: &Arc<Self>, id: &str) -> CoreResult<()> {
        let generation = self
            .registry
            .read()
            .unwrap()
            .generations
            .get(id)
            .cloned()
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?;
        let manager = Arc::clone(self);
        let id = id.to_owned();
        tokio::spawn(async move {
            let _maintenance = manager.maintenance.lock().await;
            let is_current = manager
                .registry
                .read()
                .unwrap()
                .generations
                .get(&id)
                .is_some_and(|current| Arc::ptr_eq(current, &generation));
            if !is_current {
                return Err(CoreError::cancelled());
            }
            generation.fetch_add(1, Ordering::AcqRel);
            manager.close_inner(&id).await
        })
        .await
        .map_err(|e| CoreError::new("CONNECTION_TASK", e.to_string()))?
    }

    async fn close_inner(&self, id: &str) -> CoreResult<()> {
        let (slot, config) = {
            let registry = self.registry.read().unwrap();
            (
                registry
                    .slots
                    .get(id)
                    .cloned()
                    .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?,
                registry.configs.get(id).cloned(),
            )
        };
        let mut guard = slot.lock().await;
        if let Some(session) = guard.take() {
            session.cancel.cancel();
            session.rpc.shutdown().await;
        }
        if let Some(config) = config {
            self.ports
                .lock()
                .unwrap()
                .remove(&config.port.to_uppercase());
            self.state(id, "closed", config.port == "sim://temperature", None);
        }
        Ok(())
    }

    pub async fn close_all(self: &Arc<Self>) {
        let manager = Arc::clone(self);
        let _ = tokio::spawn(async move {
            let _maintenance = manager.maintenance.lock().await;
            let ids: Vec<_> = manager
                .registry
                .read()
                .unwrap()
                .configs
                .keys()
                .cloned()
                .collect();
            for id in ids {
                if let Some(generation) = manager.registry.read().unwrap().generations.get(&id) {
                    generation.fetch_add(1, Ordering::AcqRel);
                }
                let _ = manager.close_inner(&id).await;
            }
        })
        .await;
    }

    /// Reconcile project resources while preserving the actor for every byte-for-byte
    /// unchanged connection. The plugin root is switched for future opens only.
    pub async fn reconfigure(
        self: &Arc<Self>,
        connections: Vec<ConnectionConfig>,
        plugins_root: PathBuf,
    ) -> CoreResult<Vec<String>> {
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let _maintenance = manager.maintenance.lock().await;
            manager.reconfigure_inner(connections, plugins_root).await
        })
        .await
        .map_err(|e| CoreError::new("CONNECTION_TASK", e.to_string()))?
    }

    async fn reconfigure_inner(
        self: &Arc<Self>,
        connections: Vec<ConnectionConfig>,
        plugins_root: PathBuf,
    ) -> CoreResult<Vec<String>> {
        let mut new_configs = HashMap::with_capacity(connections.len());
        let mut used_ports = HashSet::new();
        for config in connections {
            if !crate::safe_identifier(&config.id)
                || !crate::safe_identifier(&config.protocol_id)
                || config.port.trim().is_empty()
                || config.baud_rate == 0
                || !(5..=8).contains(&config.data_bits)
                || ![1, 2].contains(&config.stop_bits)
                || !["none", "odd", "even"].contains(&config.parity.as_str())
                || new_configs.contains_key(&config.id)
            {
                return Err(CoreError::new(
                    "CONNECTION_CONFIG",
                    "连接 ID 重复或串口参数无效",
                ));
            }
            if !config.port.starts_with("sim://") && !used_ports.insert(config.port.to_uppercase())
            {
                return Err(CoreError::new(
                    "PORT_IN_USE",
                    format!("多个连接配置占用串口 {}", config.port),
                ));
            }
            new_configs.insert(config.id.clone(), config);
        }

        let (old_configs, old_slots, old_generations) = {
            let registry = self.registry.read().unwrap();
            (
                registry.configs.clone(),
                registry.slots.clone(),
                registry.generations.clone(),
            )
        };
        let mut ids: Vec<String> = old_configs
            .keys()
            .chain(new_configs.keys())
            .cloned()
            .collect();
        ids.sort();
        ids.dedup();
        let changed: Vec<String> = ids
            .into_iter()
            .filter(|id| match (old_configs.get(id), new_configs.get(id)) {
                (Some(old), Some(new)) => !same_connection_config(old, new),
                (None, None) => false,
                _ => true,
            })
            .collect();

        let mut reopen = Vec::new();
        for id in &changed {
            if let Some(slot) = old_slots.get(id) {
                if slot.lock().await.is_some() {
                    reopen.push(id.clone());
                }
            }
        }
        let mut closed: Vec<String> = Vec::new();
        for id in &changed {
            if let Some(generation) = old_generations.get(id) {
                generation.fetch_add(1, Ordering::AcqRel);
                if let Err(error) = self.close_inner(id).await {
                    for restore_id in &closed {
                        if let Some(generation) = old_generations.get(restore_id) {
                            let _ = self
                                .open_inner(
                                    restore_id,
                                    generation.clone(),
                                    generation.load(Ordering::Acquire),
                                )
                                .await;
                        }
                    }
                    return Err(error);
                }
                closed.push(id.clone());
            }
        }

        let mut slots = HashMap::with_capacity(new_configs.len());
        let mut generations = HashMap::with_capacity(new_configs.len());
        for (id, config) in &new_configs {
            let unchanged = old_configs
                .get(id)
                .is_some_and(|old| same_connection_config(old, config));
            if unchanged {
                slots.insert(id.clone(), old_slots[id].clone());
                generations.insert(id.clone(), old_generations[id].clone());
            } else {
                slots.insert(id.clone(), Arc::new(Mutex::new(None)));
                generations.insert(id.clone(), Arc::new(AtomicU64::new(0)));
            }
        }
        *self.registry.write().unwrap() = Registry {
            configs: new_configs,
            slots,
            generations,
            root: plugins_root,
        };
        Ok(changed)
    }
}

fn same_connection_config(left: &ConnectionConfig, right: &ConnectionConfig) -> bool {
    serde_json::to_value(left).ok() == serde_json::to_value(right).ok()
}

#[cfg(test)]
mod reconfigure_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn config(id: &str, port: &str) -> ConnectionConfig {
        ConnectionConfig {
            id: id.into(),
            name: id.into(),
            port: port.into(),
            baud_rate: 115_200,
            data_bits: 8,
            stop_bits: 1,
            parity: "none".into(),
            protocol_id: "test-protocol".into(),
            protocol_version: Some("1.0".into()),
            protocol_config: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn reconfigure_preserves_unchanged_actor_slots_and_only_releases_changed_ports() {
        let manager = ConnectionManager::new(
            vec![
                config("keep", "COM1"),
                config("change", "COM2"),
                config("remove", "COM4"),
            ],
            "/plugins/old",
        );
        let (keep_slot, change_slot, keep_generation, change_generation) = {
            let registry = manager.registry.read().unwrap();
            (
                registry.slots["keep"].clone(),
                registry.slots["change"].clone(),
                registry.generations["keep"].clone(),
                registry.generations["change"].clone(),
            )
        };
        manager
            .ports
            .lock()
            .unwrap()
            .extend(["COM1".into(), "COM2".into(), "COM4".into()]);
        let changed = manager
            .reconfigure(
                vec![
                    config("keep", "COM1"),
                    config("change", "COM3"),
                    config("added", "COM5"),
                ],
                PathBuf::from("/plugins/new"),
            )
            .await
            .unwrap();
        assert_eq!(changed, vec!["added", "change", "remove"]);
        {
            let registry = manager.registry.read().unwrap();
            assert!(Arc::ptr_eq(&keep_slot, &registry.slots["keep"]));
            assert!(Arc::ptr_eq(&keep_generation, &registry.generations["keep"]));
            assert!(!Arc::ptr_eq(&change_slot, &registry.slots["change"]));
            assert!(!Arc::ptr_eq(
                &change_generation,
                &registry.generations["change"]
            ));
            assert!(!registry.slots.contains_key("remove"));
            assert_eq!(registry.root, PathBuf::from("/plugins/new"));
        }
        assert_eq!(
            *manager.ports.lock().unwrap(),
            HashSet::from(["COM1".into()])
        );

        let switched = manager
            .reconfigure(
                vec![
                    config("keep", "COM1"),
                    config("change", "COM3"),
                    config("added", "COM5"),
                ],
                PathBuf::from("/plugins/next"),
            )
            .await
            .unwrap();
        assert!(
            switched.is_empty(),
            "changing plugin root alone must not interrupt actors"
        );
        assert!(Arc::ptr_eq(
            &keep_slot,
            &manager.registry.read().unwrap().slots["keep"]
        ));
        assert_eq!(
            manager.registry.read().unwrap().root,
            PathBuf::from("/plugins/next")
        );
    }

    #[tokio::test]
    async fn duplicate_ports_are_rejected_before_any_connection_is_closed() {
        let manager = ConnectionManager::new(
            vec![config("one", "COM1"), config("two", "COM2")],
            "/plugins",
        );
        let one_slot = manager.registry.read().unwrap().slots["one"].clone();
        manager.ports.lock().unwrap().insert("COM1".into());
        assert!(manager
            .reconfigure(
                vec![config("one", "COM1"), config("two", "COM1")],
                PathBuf::from("/other")
            )
            .await
            .is_err());
        assert!(Arc::ptr_eq(
            &one_slot,
            &manager.registry.read().unwrap().slots["one"]
        ));
        assert_eq!(
            manager.registry.read().unwrap().root,
            PathBuf::from("/plugins")
        );
        assert_eq!(
            *manager.ports.lock().unwrap(),
            HashSet::from(["COM1".into()])
        );
    }

    #[tokio::test]
    async fn delayed_open_from_replaced_generation_cannot_attach_to_detached_slot() {
        let manager = ConnectionManager::new(vec![config("device", "COM1")], "/plugins/old");
        let old_generation = manager.registry.read().unwrap().generations["device"].clone();
        let old_generation_value = old_generation.load(Ordering::Acquire);
        manager
            .reconfigure(
                vec![config("device", "COM2")],
                PathBuf::from("/plugins/new"),
            )
            .await
            .unwrap();
        let error = manager
            .open_inner("device", old_generation, old_generation_value)
            .await
            .unwrap_err();
        assert_eq!(error.code, "CANCELLED");
        assert!(manager.ports.lock().unwrap().is_empty());
        let slot = manager.registry.read().unwrap().slots["device"].clone();
        assert!(slot.lock().await.is_none());
    }
}
