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
        Arc, Mutex as StdMutex,
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

pub struct ConnectionManager {
    configs: HashMap<String, ConnectionConfig>,
    slots: HashMap<String, Arc<Mutex<Option<Arc<Session>>>>>,
    generations: HashMap<String, AtomicU64>,
    ports: StdMutex<HashSet<String>>,
    root: PathBuf,
    events: broadcast::Sender<RuntimeEvent>,
}

impl Drop for ConnectionManager {
    fn drop(&mut self) {
        // Orderly shutdown uses close_all. This fallback also releases idle plugin
        // tasks if a caller drops the manager after an earlier setup failure.
        for slot in self.slots.values() {
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
            .map(|c| (c.id.clone(), AtomicU64::new(0)))
            .collect();
        Arc::new(Self {
            configs: connections.into_iter().map(|c| (c.id.clone(), c)).collect(),
            slots,
            generations,
            ports: StdMutex::new(HashSet::new()),
            root: plugins_root.into(),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RuntimeEvent> {
        self.events.subscribe()
    }

    pub fn configs(&self) -> Vec<ConnectionConfig> {
        self.configs.values().cloned().collect()
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
            .generations
            .get(id)
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?
            .load(Ordering::Acquire);
        let manager = Arc::clone(self);
        let id = id.to_owned();
        tokio::spawn(async move { manager.open_inner(&id, generation).await })
            .await
            .map_err(|e| CoreError::new("CONNECTION_TASK", e.to_string()))?
    }

    async fn open_inner(self: &Arc<Self>, id: &str, generation: u64) -> CoreResult<()> {
        let config = self
            .configs
            .get(id)
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?
            .clone();
        let slot = self.slots.get(id).unwrap();
        let mut guard = slot.lock().await;
        if self.generations[id].load(Ordering::Acquire) != generation {
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
                if self.generations[id].load(Ordering::Acquire) != generation {
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
                self.state(id, "fault", simulated, Some(error.clone()));
                Err(error)
            }
        }
    }

    async fn build_session(self: &Arc<Self>, config: ConnectionConfig) -> CoreResult<Arc<Session>> {
        let directory = self
            .root
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
            .slots
            .get(id)
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
        self.generations
            .get(id)
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?
            .fetch_add(1, Ordering::AcqRel);
        let manager = Arc::clone(self);
        let id = id.to_owned();
        tokio::spawn(async move { manager.close_inner(&id).await })
            .await
            .map_err(|e| CoreError::new("CONNECTION_TASK", e.to_string()))?
    }

    async fn close_inner(&self, id: &str) -> CoreResult<()> {
        let slot = self
            .slots
            .get(id)
            .ok_or_else(|| CoreError::new("CONNECTION_MISSING", "串口资源不存在"))?;
        let mut guard = slot.lock().await;
        if let Some(session) = guard.take() {
            session.cancel.cancel();
            session.rpc.shutdown().await;
        }
        if let Some(config) = self.configs.get(id) {
            self.ports
                .lock()
                .unwrap()
                .remove(&config.port.to_uppercase());
            self.state(id, "closed", config.port == "sim://temperature", None);
        }
        Ok(())
    }

    pub async fn close_all(self: &Arc<Self>) {
        for id in self.configs.keys() {
            let _ = self.close(id).await;
        }
    }
}
