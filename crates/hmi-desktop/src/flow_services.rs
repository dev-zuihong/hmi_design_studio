//! Built-in resources for portable flow logic. No application-specific protocol code.
use crate::{
    resources::{error, sha256},
    usb_printer::{self, DeviceInfo, DeviceIo},
};
use hmi_core::{
    backend::BackendResource, CoreError, CoreResult, Runtime, RuntimeEvent, TransportConfig,
};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex, RwLock, Weak,
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn fault(code: &str, message: impl std::fmt::Display) -> CoreError {
    CoreError::new(code, message.to_string())
}
fn io_error(error: impl std::fmt::Display) -> CoreError {
    fault("HOST_RESOURCE", error)
}
fn identifier(args: &Value, key: &str) -> CoreResult<String> {
    let value = args[key]
        .as_str()
        .ok_or_else(|| fault("HOST_ARGUMENT", format!("Missing {key}")))?;
    if !hmi_core::safe_identifier(value) {
        return Err(fault("HOST_ARGUMENT", format!("Invalid {key}")));
    }
    Ok(value.into())
}
fn atomic_write(path: &Path, content: &[u8]) -> CoreResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io_error("Missing parent directory"))?;
    std::fs::create_dir_all(parent).map_err(io_error)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(io_error)?;
    temp.write_all(content).map_err(io_error)?;
    temp.as_file().sync_all().map_err(io_error)?;
    temp.persist(path).map_err(io_error)?;
    Ok(())
}

pub struct FlowServices {
    resources: RwLock<BTreeMap<String, BackendResource>>,
    origins: Mutex<BTreeMap<String, PathBuf>>,
    work: tempfile::TempDir,
    persistent: PathBuf,
    state_lock: Mutex<()>,
    pub transports: Arc<TransportManager>,
}
impl FlowServices {
    pub fn new(
        project_id: &str,
        configs: Vec<TransportConfig>,
        resources: BTreeMap<String, BackendResource>,
    ) -> Result<Arc<Self>, String> {
        let persistent = crate::session::user_directory(project_id)?.join("flow-data");
        std::fs::create_dir_all(&persistent).map_err(error)?;
        let work = tempfile::Builder::new()
            .prefix("hmi-flow-")
            .tempdir()
            .map_err(error)?;
        let guard_root = dirs::data_local_dir()
            .ok_or("Local data directory unavailable")?
            .join("HmiDesignStudio/transport-guards");
        Ok(Arc::new(Self {
            resources: RwLock::new(resources),
            origins: Mutex::new(BTreeMap::new()),
            work,
            persistent,
            state_lock: Mutex::new(()),
            transports: TransportManager::new(configs, guard_root),
        }))
    }
    pub fn supports(kind: &str) -> bool {
        kind.starts_with("transport.")
            || matches!(
                kind,
                "state.load" | "state.save" | "file.resolve" | "file.workspace" | "file.register"
            )
    }
    pub fn set_runtime(&self, runtime: Weak<Runtime>) {
        *self.transports.runtime.lock().unwrap() = runtime;
    }
    pub fn resource_map(&self) -> BTreeMap<String, BackendResource> {
        self.resources.read().unwrap().clone()
    }
    pub fn record_origin(&self, id: String, origin: PathBuf) -> CoreResult<()> {
        if !hmi_core::safe_identifier(&id) || !origin.is_absolute() {
            return Err(fault("HOST_ARGUMENT", "Invalid file resource origin"));
        }
        self.origins.lock().unwrap().insert(id, origin);
        Ok(())
    }
    pub fn get_origin(&self, id: &str) -> Option<PathBuf> {
        self.origins.lock().unwrap().get(id).cloned()
    }
    pub fn replace_resources(&self, resources: BTreeMap<String, BackendResource>) {
        *self.resources.write().unwrap() = resources;
    }
    pub fn register_resource(&self, id: String, resource: BackendResource) -> CoreResult<()> {
        if !hmi_core::safe_identifier(&id)
            || !resource.path.is_absolute()
            || !resource.path.is_file()
        {
            return Err(fault("HOST_RESOURCE", "Invalid registered file"));
        }
        let mut resources = self.resources.write().unwrap();
        if resources.get(&id).is_some_and(|old| old != &resource) {
            return Err(fault(
                "HOST_RESOURCE",
                "Immutable resource ID already exists",
            ));
        }
        resources.insert(id, resource);
        Ok(())
    }
    pub fn resource(&self, id: &str) -> CoreResult<BackendResource> {
        let resource = self
            .resources
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| fault("HOST_RESOURCE", "Unknown file resource"))?;
        let digest = sha256(&resource.path).map_err(io_error)?;
        if resource
            .sha256
            .as_ref()
            .is_some_and(|expected| *expected != digest)
            || resource.size.is_some_and(|size| {
                std::fs::metadata(&resource.path)
                    .map(|m| m.len() != size)
                    .unwrap_or(true)
            })
        {
            return Err(fault(
                "HOST_RESOURCE_CHANGED",
                "File snapshot changed; select the file again",
            ));
        }
        Ok(resource)
    }
    pub fn python_host(&self) -> Value {
        let resources: BTreeMap<_,_> = self.resources.read().unwrap().iter().map(|(id, resource)| (id.clone(), json!({"path":resource.path,"sha256":resource.sha256,"size":resource.size,"name":resource.name}))).collect();
        json!({"resources":resources,"workDirectory":self.work.path(),"persistentDirectory":self.persistent})
    }
    pub async fn action(
        self: &Arc<Self>,
        kind: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        if cancel.is_cancelled() {
            return Err(CoreError::cancelled());
        }
        if kind.starts_with("transport.") {
            let plan = if kind == "transport.exchangeMany" {
                Some(self.resource(&identifier(&args, "resourceId")?)?.path)
            } else {
                None
            };
            return self.transports.action(kind, args, plan, cancel).await;
        }
        match kind {
            "file.workspace" => {
                Ok(json!({"path":self.work.path(),"workDirectory":self.work.path()}))
            }
            "file.resolve" => {
                let id = args["resourceId"]
                    .as_str()
                    .or_else(|| args["resource"]["id"].as_str())
                    .ok_or_else(|| io_error("Missing resource ID"))?;
                let resource = self.resource(id)?;
                Ok(
                    json!({"id":id,"path":resource.path,"size":resource.size,"sha256":resource.sha256,"name":resource.name}),
                )
            }
            "file.register" => {
                let path = Path::new(
                    args["path"]
                        .as_str()
                        .ok_or_else(|| io_error("Missing generated file path"))?,
                )
                .canonicalize()
                .map_err(io_error)?;
                let work = self.work.path().canonicalize().map_err(io_error)?;
                if !path.starts_with(work) || !path.is_file() {
                    return Err(io_error(
                        "Generated files must be inside this session's workDirectory",
                    ));
                }
                let size = std::fs::metadata(&path).map_err(io_error)?.len();
                if size > 512 * 1024 * 1024 {
                    return Err(io_error("Generated file exceeds 512 MiB"));
                }
                let digest = sha256(&path).map_err(io_error)?;
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                use sha2::{Digest, Sha256};
                let id = format!(
                    "generated_{}_{:x}",
                    &digest[..20],
                    Sha256::digest(name.as_bytes())
                );
                let directory = self.persistent.join("files").join(&digest);
                std::fs::create_dir_all(&directory).map_err(io_error)?;
                let destination = directory.join(&name);
                if !destination.exists() {
                    let staged = tempfile::NamedTempFile::new_in(&directory).map_err(io_error)?;
                    std::fs::copy(&path, staged.path()).map_err(io_error)?;
                    if sha256(staged.path()).map_err(io_error)? != digest {
                        return Err(io_error("Generated file changed while copying"));
                    }
                    staged.persist(&destination).map_err(io_error)?;
                }
                let resource = BackendResource {
                    path: destination,
                    size: Some(size),
                    sha256: Some(digest.clone()),
                    name: Some(name.clone()),
                };
                self.register_resource(id.clone(), resource)?;
                Ok(json!({"id":id,"name":name,"size":size,"sha256":digest,"status":"ready"}))
            }
            "state.load" | "state.save" => {
                let key = identifier(&args, "key")?;
                let path = self.persistent.join("state").join(format!("{key}.json"));
                let _guard = self.state_lock.lock().unwrap();
                if kind == "state.save" {
                    let value = args
                        .get("value")
                        .ok_or_else(|| io_error("Missing state value"))?;
                    let content = serde_json::to_vec(value).map_err(io_error)?;
                    if content.len() > 4 * 1024 * 1024 {
                        return Err(io_error("Saved state exceeds 4 MiB"));
                    }
                    atomic_write(&path, &content)?;
                    Ok(json!({"saved":true,"key":key}))
                } else if path.exists() {
                    if std::fs::metadata(&path).map_err(io_error)?.len() > 4 * 1024 * 1024 {
                        return Err(io_error("Saved state exceeds 4 MiB"));
                    }
                    serde_json::from_slice(&std::fs::read(path).map_err(io_error)?)
                        .map_err(io_error)
                } else {
                    Ok(args.get("default").cloned().unwrap_or(Value::Null))
                }
            }
            _ => Err(fault("HOST_ACTION", "Unsupported built-in flow operation")),
        }
    }
}

struct DeviceStatus {
    info: DeviceInfo,
    generation: u64,
    present: bool,
    open: bool,
    faulted: bool,
    effect: bool,
}
struct Device {
    status: Mutex<DeviceStatus>,
    operation: Arc<tokio::sync::Mutex<()>>,
    io: Mutex<Option<Box<dyn DeviceIo>>>,
    lease: Mutex<Option<File>>,
    cancel: Mutex<CancellationToken>,
    pending: AtomicUsize,
    marker: PathBuf,
    lease_path: PathBuf,
    packet_size: u32,
}
impl Device {
    fn snapshot(&self) -> Value {
        let state = self.status.lock().unwrap();
        json!({"deviceKey":state.info.device_key,"path":state.info.path,"location":state.info.location,"deviceGeneration":state.generation,"present":state.present,"open":state.open,"faulted":state.faulted,"resetRequired":state.effect || self.marker.exists(),"busy":self.pending.load(Ordering::Acquire)>0})
    }
    fn check(&self, generation: u64) -> CoreResult<()> {
        let state = self.status.lock().unwrap();
        if !state.present || state.generation != generation {
            return Err(fault(
                "TRANSPORT_STALE",
                "Device was removed or belongs to an old generation",
            ));
        }
        Ok(())
    }
    fn acquire_lease(&self) -> CoreResult<File> {
        std::fs::create_dir_all(self.lease_path.parent().unwrap()).map_err(io_error)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(0);
        }
        options.open(&self.lease_path).map_err(|e| {
            fault(
                "TRANSPORT_BUSY",
                format!("Device is owned by another application: {e}"),
            )
        })
    }
    fn close(&self) {
        self.io.lock().unwrap().take();
        self.lease.lock().unwrap().take();
        self.status.lock().unwrap().open = false;
    }
    fn close_if_removed_generation(&self, generation: u64) {
        let state = self.status.lock().unwrap();
        if state.present || state.generation != generation {
            return;
        }
        drop(state);
        self.close();
    }
    fn mark(&self) -> CoreResult<()> {
        atomic_write(
            &self.marker,
            b"Unconfirmed device effect; explicit physical reset confirmation required",
        )?;
        self.status.lock().unwrap().effect = true;
        Ok(())
    }
    fn clear_effect(&self) -> CoreResult<()> {
        if self.marker.exists() {
            std::fs::remove_file(&self.marker).map_err(io_error)?;
        }
        self.status.lock().unwrap().effect = false;
        Ok(())
    }
}

pub struct TransportManager {
    configs: RwLock<Vec<TransportConfig>>,
    devices: Mutex<BTreeMap<(String, String), Arc<Device>>>,
    generation: AtomicU64,
    closing: AtomicBool,
    guards: PathBuf,
    runtime: Mutex<Weak<Runtime>>,
}
struct Pending(Arc<Device>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}
impl TransportManager {
    fn new(configs: Vec<TransportConfig>, guards: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            configs: RwLock::new(configs),
            devices: Mutex::new(BTreeMap::new()),
            generation: AtomicU64::new(1),
            closing: AtomicBool::new(false),
            guards,
            runtime: Mutex::new(Weak::new()),
        })
    }
    pub fn is_busy(&self) -> bool {
        self.devices
            .lock()
            .unwrap()
            .values()
            .any(|d| d.pending.load(Ordering::Acquire) > 0)
    }
    pub fn snapshot(&self, transport_id: &str) -> Value {
        let devices: Vec<_> = self
            .devices
            .lock()
            .unwrap()
            .iter()
            .filter(|((id, _), _)| id == transport_id)
            .map(|(_, d)| d.snapshot())
            .collect();
        json!({"transportId":transport_id,"devices":devices})
    }
    fn emit(&self, kind: &str, data: Value) {
        if let Some(runtime) = self.runtime.lock().unwrap().upgrade() {
            runtime.publish_event(RuntimeEvent::new(kind, data));
        }
    }
    pub async fn reconfigure(&self, configs: Vec<TransportConfig>) {
        self.close_devices().await;
        *self.configs.write().unwrap() = configs;
    }
    async fn close_devices(&self) {
        let devices: Vec<_> = self.devices.lock().unwrap().values().cloned().collect();
        for device in &devices {
            device.cancel.lock().unwrap().cancel();
        }
        for device in devices {
            let guard = device.operation.clone().lock_owned().await;
            let _ = tokio::task::spawn_blocking(move || {
                let _guard = guard;
                device.close();
            })
            .await;
        }
        self.devices.lock().unwrap().clear();
    }
    pub async fn shutdown(&self) {
        self.closing.store(true, Ordering::Release);
        self.close_devices().await;
    }

    async fn enumerate(self: &Arc<Self>, transport_id: &str) -> CoreResult<Value> {
        let config = self
            .configs
            .read()
            .unwrap()
            .iter()
            .find(|c| c.id == transport_id)
            .cloned()
            .ok_or_else(|| fault("TRANSPORT_MISSING", "Transport resource does not exist"))?;
        let packet_size = config.packet_size;
        let found = tokio::task::spawn_blocking(move || usb_printer::enumerate(&config))
            .await
            .map_err(io_error)??;
        let mut devices = self.devices.lock().unwrap();
        for ((id, key), device) in devices.iter().filter(|((id, _), _)| id == transport_id) {
            if !found.iter().any(|info| info.device_key == *key) {
                let generation = {
                    let mut status = device.status.lock().unwrap();
                    status.present = false;
                    status.generation
                };
                device.cancel.lock().unwrap().cancel();
                let device = device.clone();
                tokio::spawn(async move {
                    let guard = device.operation.clone().lock_owned().await;
                    let _ = tokio::task::spawn_blocking(move || {
                        let _guard = guard;
                        device.close_if_removed_generation(generation);
                    })
                    .await;
                });
            }
            let _ = id;
        }
        for info in found {
            let key = (transport_id.to_owned(), info.device_key.clone());
            if let Some(device) = devices.get(&key) {
                let mut status = device.status.lock().unwrap();
                if !status.present {
                    status.generation = self.generation.fetch_add(1, Ordering::AcqRel);
                    status.present = true;
                    *device.cancel.lock().unwrap() = CancellationToken::new();
                }
                status.info = info;
            } else {
                let marker = self.guards.join(format!("{}.state", info.device_key));
                let lease_path = self.guards.join(format!("{}.lease", info.device_key));
                devices.insert(
                    key,
                    Arc::new(Device {
                        status: Mutex::new(DeviceStatus {
                            info,
                            generation: self.generation.fetch_add(1, Ordering::AcqRel),
                            present: true,
                            open: false,
                            faulted: false,
                            effect: marker.exists(),
                        }),
                        operation: Arc::new(tokio::sync::Mutex::new(())),
                        io: Mutex::new(None),
                        lease: Mutex::new(None),
                        cancel: Mutex::new(CancellationToken::new()),
                        pending: AtomicUsize::new(0),
                        marker,
                        lease_path,
                        packet_size,
                    }),
                );
            }
        }
        drop(devices);
        let snapshot = self.snapshot(transport_id);
        self.emit("transport.devices", snapshot.clone());
        Ok(snapshot)
    }

    pub async fn action(
        self: &Arc<Self>,
        kind: &str,
        args: Value,
        plan: Option<PathBuf>,
        cancel: CancellationToken,
    ) -> CoreResult<Value> {
        if self.closing.load(Ordering::Acquire) {
            return Err(fault("TRANSPORT_CLOSED", "Session is stopping"));
        }
        let transport_id = identifier(&args, "transportId")?;
        if kind == "transport.enumerate" {
            return self.enumerate(&transport_id).await;
        }
        let key = identifier(&args, "deviceKey")?;
        let generation = args["deviceGeneration"]
            .as_u64()
            .ok_or_else(|| fault("TRANSPORT_STALE", "Missing deviceGeneration"))?;
        let device = self
            .devices
            .lock()
            .unwrap()
            .get(&(transport_id.clone(), key.clone()))
            .cloned()
            .ok_or_else(|| {
                fault(
                    "TRANSPORT_MISSING",
                    "Enumerate the device before opening it",
                )
            })?;
        device.check(generation)?;
        if kind == "transport.cancel" {
            let mut token = device.cancel.lock().unwrap();
            token.cancel();
            *token = CancellationToken::new();
            return Ok(json!({"cancelled":true,"deviceKey":key,"deviceGeneration":generation}));
        }
        if device.pending.fetch_add(1, Ordering::AcqRel) >= 64 {
            device.pending.fetch_sub(1, Ordering::AcqRel);
            return Err(fault("TRANSPORT_QUEUE_FULL", "Device queue is full"));
        }
        let pending = Pending(device.clone());
        let device_cancel = device.cancel.lock().unwrap().clone();
        let request_cancel = cancel.child_token();
        let _cancel_on_drop = request_cancel.clone().drop_guard();
        let kind = kind.to_owned();
        let manager = self.clone();
        // This task outlives its caller, so cancellation cannot drop an issued I/O transaction.
        tokio::spawn(async move {
            let _pending=pending;
            let guard=tokio::select!{biased;_=request_cancel.cancelled()=>return Err(CoreError::cancelled()),_=device_cancel.cancelled()=>return Err(CoreError::cancelled()),guard=device.operation.clone().lock_owned()=>guard};
            tokio::task::spawn_blocking(move || {
                let _guard=guard;
                device.check(generation)?;
                if request_cancel.is_cancelled()||device_cancel.is_cancelled(){return Err(CoreError::cancelled());}
                let cancelled=||request_cancel.is_cancelled()||device_cancel.is_cancelled();
                let result=match kind.as_str(){
                    "transport.open"=>{
                        if device.io.lock().unwrap().is_none(){
                            let lease=device.acquire_lease()?;
                            if device.marker.exists(){return Err(fault("DEVICE_RESET_REQUIRED","Unconfirmed device effect; physically reset and confirm recovery"));}
                            let path=device.status.lock().unwrap().info.path.clone();
                            *device.io.lock().unwrap()=Some(usb_printer::open(&path,device.packet_size)?);
                            *device.lease.lock().unwrap()=Some(lease);
                            let mut status=device.status.lock().unwrap();status.open=true;status.faulted=false;status.effect=false;
                        }
                        Ok(device.snapshot())
                    },
                    "transport.close"=>{device.close();Ok(device.snapshot())},
                    "transport.recover"=>{
                        if args["confirmed"]!=true{return Err(fault("DEVICE_RESET_REQUIRED","Physical device reset confirmation is required"));}
                        device.io.lock().unwrap().take();
                        let lease=match device.lease.lock().unwrap().take(){Some(lease)=>lease,None=>device.acquire_lease()?};
                        device.clear_effect()?;{let mut status=device.status.lock().unwrap();status.open=false;status.faulted=false;}
                        drop(lease);Ok(device.snapshot())
                    },
                    "transport.completeEffect"=>{
                        if device.lease.lock().unwrap().is_none()||device.status.lock().unwrap().faulted{return Err(fault("DEVICE_RESET_REQUIRED","Cannot confirm a closed or faulted device"));}
                        device.clear_effect()?;Ok(device.snapshot())
                    },
                    "transport.exchange"|"transport.exchangeMany"=>{
                        if device.status.lock().unwrap().faulted{return Err(fault("TRANSPORT_FAULT","Device must be reconnected or recovered"));}
                        if device.marker.exists()&&args["readOnly"]!=true{return Err(fault("DEVICE_RESET_REQUIRED","Previous device effect has not completed"));}
                        if device.io.lock().unwrap().is_none(){return Err(fault("TRANSPORT_CLOSED","Open the device before exchanging frames"));}
                        if let Some(plan)=plan{
                            // Validate the entire immutable plan before admitting its first frame.
                            let count=validate_plan(&plan,&args)?;
                            let reader=BufReader::new(File::open(&plan).map_err(io_error)?);
                            let mut last=String::new();let mut completed=0;let mut notified=Instant::now();
                            for line in reader.lines(){
                                if cancelled(){return Err(CoreError::cancelled());}
                                let record:Value=serde_json::from_str(&line.map_err(io_error)?).map_err(io_error)?;
                                let options=merge_record(&args,&record)?;
                                if completed==0&&args["effect"]==true{device.mark()?;}
                                last=exchange(&device,&options)?;completed+=1;
                                if completed==count||notified.elapsed()>=Duration::from_millis(100){
                                    manager.emit("transport.progress",json!({"transportId":transport_id,"deviceKey":key,"deviceGeneration":generation,"completedFrames":completed,"totalFrames":count,"progress":completed as f64*100.0/count as f64}));notified=Instant::now();
                                }
                            }
                            Ok(json!({"lastResponseHex":last,"frameCount":count,"deviceKey":key,"deviceGeneration":generation}))
                        }else{
                            validate_request(&args)?;
                            if args["effect"]==true{device.mark()?;}
                            let response=exchange(&device,&args)?;
                            Ok(json!({"responseHex":response,"deviceKey":key,"deviceGeneration":generation}))
                        }
                    },
                    _=>Err(fault("TRANSPORT_OPERATION","Unsupported transport operation")),
                };
                result
            }).await.map_err(io_error)?
        }).await.map_err(io_error)?
    }
}

fn merge_record(defaults: &Value, record: &Value) -> CoreResult<Value> {
    let mut merged = defaults
        .as_object()
        .cloned()
        .ok_or_else(|| fault("TRANSPORT_PLAN", "Invalid transfer defaults"))?;
    let object = record
        .as_object()
        .ok_or_else(|| fault("TRANSPORT_PLAN", "Transfer record must be an object"))?;
    for (key, value) in object {
        if !["requestHex", "framing", "match", "ack", "timeoutMs"].contains(&key.as_str()) {
            return Err(fault(
                "TRANSPORT_PLAN",
                format!("Unsupported record field: {key}"),
            ));
        }
        merged.insert(key.clone(), value.clone());
    }
    Ok(Value::Object(merged))
}
fn validate_plan(path: &Path, defaults: &Value) -> CoreResult<usize> {
    let mut reader = BufReader::new(File::open(path).map_err(io_error)?);
    let mut bytes = Vec::new();
    let mut count = 0;
    loop {
        bytes.clear();
        let read = reader
            .by_ref()
            .take(1024 * 1024 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(io_error)?;
        if read == 0 {
            break;
        }
        if bytes.len() > 1024 * 1024 {
            return Err(fault("TRANSPORT_PLAN", "Transfer record exceeds 1 MiB"));
        }
        let record: Value = serde_json::from_slice(&bytes).map_err(io_error)?;
        validate_request(&merge_record(defaults, &record)?)?;
        count += 1;
        if count > 2_000_000 {
            return Err(fault("TRANSPORT_PLAN", "Transfer exceeds record limit"));
        }
    }
    if count == 0 {
        return Err(fault("TRANSPORT_PLAN", "Transfer plan is empty"));
    }
    Ok(count)
}
fn decode_hex(value: &str) -> CoreResult<Vec<u8>> {
    if value.len() % 2 != 0 || value.len() > 2 * 1024 * 1024 {
        return Err(fault("TRANSPORT_FRAME", "Invalid hex length"));
    }
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(io_error)?;
            u8::from_str_radix(text, 16).map_err(|_| fault("TRANSPORT_FRAME", "Invalid hex byte"))
        })
        .collect()
}
fn encode_hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02X}")).collect()
}
fn validate_request(args: &Value) -> CoreResult<()> {
    let request = decode_hex(
        args["requestHex"]
            .as_str()
            .ok_or_else(|| fault("TRANSPORT_FRAME", "Missing requestHex"))?,
    )?;
    if request.is_empty() || request.len() > 65536 {
        return Err(fault(
            "TRANSPORT_FRAME",
            "Request must contain 1..65536 bytes",
        ));
    }
    if args.get("retry").is_some_and(|retry| retry != "never") {
        return Err(fault(
            "TRANSPORT_RETRY",
            "Automatic retries are not supported",
        ));
    }
    let timeout = args["timeoutMs"].as_u64().unwrap_or(2000);
    if !(1..=15000).contains(&timeout) {
        return Err(fault(
            "TRANSPORT_TIMEOUT",
            "Frame timeout must be 1..15000 ms",
        ));
    }
    FrameDecoder::new(&args["framing"])?;
    for name in ["match", "ack"] {
        if let Some(rules) = args.get(name) {
            validate_rules(rules)?;
        }
    }
    Ok(())
}
fn validate_rules(value: &Value) -> CoreResult<()> {
    let rules = value
        .as_array()
        .ok_or_else(|| fault("TRANSPORT_MATCH", "Frame rules must be an array"))?;
    for rule in rules {
        if rule["offset"].as_u64().is_none_or(|n| n > 65535)
            || rule["value"].as_u64().is_none_or(|n| n > 255)
            || rule
                .get("mask")
                .is_some_and(|v| v.as_u64().is_none_or(|n| n > 255))
        {
            return Err(fault("TRANSPORT_MATCH", "Invalid byte matching rule"));
        }
    }
    Ok(())
}
fn matches_rules(frame: &[u8], rules: &Value) -> bool {
    rules.as_array().is_none_or(|rules| {
        rules.iter().all(|rule| {
            frame
                .get(rule["offset"].as_u64().unwrap_or(u64::MAX) as usize)
                .is_some_and(|byte| {
                    (*byte & rule["mask"].as_u64().unwrap_or(255) as u8)
                        == rule["value"].as_u64().unwrap_or(256) as u8
                })
        })
    })
}
fn exchange(device: &Device, args: &Value) -> CoreResult<String> {
    let result = (|| {
        let request = decode_hex(args["requestHex"].as_str().unwrap_or(""))?;
        let mut decoder = FrameDecoder::new(&args["framing"])?;
        let deadline =
            Instant::now() + Duration::from_millis(args["timeoutMs"].as_u64().unwrap_or(2000));
        let mut guard = device.io.lock().unwrap();
        let io = guard
            .as_mut()
            .ok_or_else(|| fault("TRANSPORT_CLOSED", "Device is closed"))?;
        io.write(&request, deadline)?;
        loop {
            if Instant::now() >= deadline {
                return Err(fault(
                    "TRANSPORT_TIMEOUT",
                    "Response deadline reached; request was not retried",
                ));
            }
            for frame in decoder.append(&io.read(deadline)?)? {
                if !matches_rules(&frame, &args["match"]) {
                    continue;
                }
                if !matches_rules(&frame, &args["ack"]) {
                    return Err(fault(
                        "TRANSPORT_ACK",
                        format!("Response rejected: {}", encode_hex(&frame)),
                    ));
                }
                return Ok(encode_hex(&frame));
            }
        }
    })();
    if result.is_err() {
        device.status.lock().unwrap().faulted = true;
        device.close();
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    struct FakeDeviceIo {
        writes: Arc<AtomicUsize>,
        cancel: CancellationToken,
        response: Vec<u8>,
    }
    impl DeviceIo for FakeDeviceIo {
        fn write(&mut self, bytes: &[u8], _: Instant) -> CoreResult<()> {
            assert_eq!(bytes, [0xA5, 0x01]);
            self.writes.fetch_add(1, Ordering::AcqRel);
            // Model a caller stopping immediately after the request has reached
            // the device. The response read must still finish before returning.
            self.cancel.cancel();
            Ok(())
        }
        fn read(&mut self, _: Instant) -> CoreResult<Vec<u8>> {
            Ok(self.response.clone())
        }
    }

    fn framing() -> Value {
        json!({"headerHex":"AA","trailerHex":"55","maxFrameBytes":16,
            "length":{"offset":1,"width":1,"endian":"big","totalAdjustment":0}})
    }
    fn fake_device(directory: &Path, cancel: CancellationToken) -> (Device, Arc<AtomicUsize>) {
        let writes = Arc::new(AtomicUsize::new(0));
        let fake = FakeDeviceIo {
            writes: writes.clone(),
            cancel,
            response: vec![0x00, 0xAA, 0x04, 0x10, 0x55],
        };
        let info = DeviceInfo {
            device_key: "fake".into(),
            path: "fake://device".into(),
            location: "test".into(),
        };
        (
            Device {
                status: Mutex::new(DeviceStatus {
                    info,
                    generation: 1,
                    present: true,
                    open: true,
                    faulted: false,
                    effect: false,
                }),
                operation: Arc::new(tokio::sync::Mutex::new(())),
                io: Mutex::new(Some(Box::new(fake))),
                lease: Mutex::new(Some(
                    OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(directory.join("lease"))
                        .unwrap(),
                )),
                cancel: Mutex::new(CancellationToken::new()),
                pending: AtomicUsize::new(0),
                marker: directory.join("effect.state"),
                lease_path: directory.join("lease"),
                packet_size: 64,
            },
            writes,
        )
    }

    #[test]
    fn issued_exchange_reads_response_after_caller_cancellation_and_never_retries() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let (device, writes) = fake_device(directory.path(), cancel.clone());
        let result = exchange(
            &device,
            &json!({"requestHex":"A501","timeoutMs":1000,"framing":framing()}),
        )
        .unwrap();
        assert!(cancel.is_cancelled());
        assert_eq!(result, "AA041055");
        assert_eq!(writes.load(Ordering::Acquire), 1);
        assert!(device.status.lock().unwrap().open);
    }

    #[test]
    fn effect_guard_survives_reopening_manager_state_until_explicit_clear() {
        let directory = tempfile::tempdir().unwrap();
        let cancel = CancellationToken::new();
        let (device, _) = fake_device(directory.path(), cancel);
        device.mark().unwrap();
        assert!(device.marker.exists());
        assert_eq!(device.snapshot()["resetRequired"], true);
        device.clear_effect().unwrap();
        assert!(!device.marker.exists());
        assert_eq!(device.snapshot()["resetRequired"], false);
    }

    #[test]
    fn delayed_unplug_close_cannot_close_a_reinserted_generation() {
        let directory = tempfile::tempdir().unwrap();
        let (device, _) = fake_device(directory.path(), CancellationToken::new());
        {
            let mut status = device.status.lock().unwrap();
            status.present = true;
            status.generation = 2;
        }
        device.close_if_removed_generation(1);
        assert!(device.io.lock().unwrap().is_some());
        assert!(device.lease.lock().unwrap().is_some());
        assert_eq!(device.snapshot()["open"], true);
    }
}

struct FrameDecoder {
    config: Value,
    header: Vec<u8>,
    trailer: Vec<u8>,
    buffer: Vec<u8>,
    maximum: usize,
}
impl FrameDecoder {
    fn new(config: &Value) -> CoreResult<Self> {
        let header = decode_hex(config["headerHex"].as_str().unwrap_or(""))?;
        let trailer = decode_hex(config["trailerHex"].as_str().unwrap_or(""))?;
        let maximum = config["maxFrameBytes"].as_u64().unwrap_or(65536) as usize;
        let length = &config["length"];
        let width = length["width"].as_u64().unwrap_or(0);
        let offset = length["offset"].as_u64().unwrap_or(u64::MAX);
        if header.is_empty()
            || header.len() > 64
            || trailer.len() > 64
            || !(1..=4).contains(&width)
            || maximum > 65536
            || maximum < header.len() + trailer.len() + width as usize
            || offset
                .checked_add(width)
                .is_none_or(|end| end > maximum as u64)
            || length["totalAdjustment"].as_u64().is_none()
        {
            return Err(fault(
                "TRANSPORT_FRAMING",
                "Invalid length-prefixed framing description",
            ));
        }
        if let Some(checksum) = config.get("checksum") {
            if checksum["algorithm"] != "crc16"
                || checksum["reflected"] != true
                || checksum["polynomial"].as_u64().is_none_or(|n| n > 65535)
                || checksum["initial"].as_u64().is_none_or(|n| n > 65535)
                || !matches!(checksum["wireOrder"].as_str(), Some("little" | "big"))
                || checksum["offsetFromEnd"]
                    .as_u64()
                    .is_none_or(|n| n < 2 || n > maximum as u64)
                || checksum["excludeTailBytes"]
                    .as_u64()
                    .is_none_or(|n| n < 2 || n > maximum as u64)
            {
                return Err(fault(
                    "TRANSPORT_FRAMING",
                    "Unsupported checksum description",
                ));
            }
        }
        Ok(Self {
            config: config.clone(),
            header,
            trailer,
            buffer: Vec::new(),
            maximum,
        })
    }
    fn append(&mut self, bytes: &[u8]) -> CoreResult<Vec<Vec<u8>>> {
        self.buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        loop {
            let Some(start) = self
                .buffer
                .windows(self.header.len())
                .position(|window| window == self.header)
            else {
                let retain = self.header.len().saturating_sub(1).min(self.buffer.len());
                self.buffer.drain(..self.buffer.len() - retain);
                break;
            };
            self.buffer.drain(..start);
            let length = &self.config["length"];
            let offset = length["offset"].as_u64().unwrap() as usize;
            let width = length["width"].as_u64().unwrap() as usize;
            if self.buffer.len() < offset + width {
                break;
            }
            let mut count = 0usize;
            let field = &self.buffer[offset..offset + width];
            if length["endian"] == "big" {
                for byte in field {
                    count = count * 256 + *byte as usize;
                }
            } else {
                for byte in field.iter().rev() {
                    count = count * 256 + *byte as usize;
                }
            }
            let Some(count) =
                count.checked_add(length["totalAdjustment"].as_u64().unwrap() as usize)
            else {
                return Err(fault("TRANSPORT_FRAMING", "Frame length overflow"));
            };
            if count < offset + width
                || count < self.header.len() + self.trailer.len()
                || count > self.maximum
            {
                self.buffer.remove(0);
                continue;
            }
            if self.buffer.len() < count {
                break;
            }
            let frame = &self.buffer[..count];
            let mut valid = frame.ends_with(&self.trailer);
            if let Some(checksum) = self.config.get("checksum") {
                let trailing = checksum["excludeTailBytes"].as_u64().unwrap() as usize;
                let from_end = checksum["offsetFromEnd"].as_u64().unwrap() as usize;
                if count < trailing || count < from_end || from_end < 2 {
                    valid = false;
                } else {
                    let mut crc = checksum["initial"].as_u64().unwrap() as u16;
                    let polynomial = checksum["polynomial"].as_u64().unwrap() as u16;
                    for byte in &frame[..count - trailing] {
                        crc ^= *byte as u16;
                        for _ in 0..8 {
                            crc = if crc & 1 != 0 {
                                (crc >> 1) ^ polynomial
                            } else {
                                crc >> 1
                            };
                        }
                    }
                    let index = count - from_end;
                    let actual = if checksum["wireOrder"] == "big" {
                        u16::from_be_bytes([frame[index], frame[index + 1]])
                    } else {
                        u16::from_le_bytes([frame[index], frame[index + 1]])
                    };
                    valid &= crc == actual;
                }
            }
            if valid {
                output.push(self.buffer.drain(..count).collect());
            } else {
                self.buffer.remove(0);
            }
        }
        if self.buffer.len() > self.maximum + 4096 {
            return Err(fault(
                "TRANSPORT_FRAMING",
                "Receive buffer exceeded its limit",
            ));
        }
        Ok(output)
    }
}
