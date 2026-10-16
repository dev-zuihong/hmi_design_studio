//! Project-local, one-shot CPython embeddable execution. Never searches PATH.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

#[cfg(windows)]
#[path = "process/windows.rs"]
mod process;
#[cfg(target_os = "macos")]
#[path = "process/macos.rs"]
mod process;

pub const PYTHON_VERSION: &str = "3.14.8";
#[cfg(windows)]
pub const ARCHIVE_SHA256: &str = "a93abe456ab01bd96d7a085b3cdb6566b3063f4241360d114142fbdb07f0a310";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub const ARCHIVE_SHA256: &str = "709f419d175eb015f810b68aa54bdae1d776c9824cb362aa701c93b0a9f2a9e7";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
pub const ARCHIVE_SHA256: &str = "e09bce050cf075e777d1a802c4a11b6d43d54291b5f76911887173b28ad9c9fd";
#[cfg(not(any(windows, target_os = "macos")))]
pub const ARCHIVE_SHA256: &str = "unsupported";
pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum PythonError {
    #[error("Python runtime unavailable: {0}")]
    Environment(String),
    #[error("Python queue is full (64 waiting calls)")]
    QueueFull,
    #[error("Python execution cancelled")]
    Cancelled,
    #[error("Python execution exceeded its wall-clock limit")]
    Timeout,
    #[error("Python process exceeded its configured memory limit")]
    MemoryLimit,
    #[error("Python process failed: {0}")]
    Process(String),
    #[error("Python code failed: {message}\n{logs}")]
    Code { message: String, logs: String },
    #[error("Invalid Python request: {0}")]
    InvalidInput(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    pub schema_version: u32,
    pub version: String,
    pub platform: String,
    pub abi: String,
    pub archive_sha256: String,
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct PythonEnvironment {
    root: PathBuf,
    pub manifest: RuntimeManifest,
    pub imports: Vec<String>,
}

impl PythonEnvironment {
    pub fn load(root: impl Into<PathBuf>) -> Result<Self, PythonError> {
        let root = fs::canonicalize(root.into()).map_err(environment_error)?;
        let manifest: RuntimeManifest = serde_json::from_slice(
            &fs::read(root.join("runtime-manifest.json")).map_err(environment_error)?,
        )
        .map_err(environment_error)?;
        #[cfg(windows)]
        let (expected_platform, _executable, required, lock_platform) = (
            "win_amd64",
            "python.exe",
            [
                "python.exe",
                "python314.dll",
                "python314.zip",
                "python314._pth",
                "executor/worker.py",
                "dependencies.lock.json",
            ],
            "win_amd64",
        );
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        let (expected_platform, _executable, required, lock_platform) = (
            "aarch64-apple-darwin",
            "bin/python3.14",
            [
                "bin/python3.14",
                "lib/libpython3.14.dylib",
                "lib/python3.14",
                "executor/worker.py",
                "dependencies.lock.json",
            ],
            "aarch64-apple-darwin",
        );
        #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
        let (expected_platform, _executable, required, lock_platform) = (
            "x86_64-apple-darwin",
            "bin/python3.14",
            [
                "bin/python3.14",
                "lib/libpython3.14.dylib",
                "lib/python3.14",
                "executor/worker.py",
                "dependencies.lock.json",
            ],
            "x86_64-apple-darwin",
        );
        #[cfg(not(any(windows, target_os = "macos")))]
        let (expected_platform, _executable, required, lock_platform) = (
            "unsupported",
            "python",
            [
                "python",
                "executor/worker.py",
                "dependencies.lock.json",
                "",
                "",
            ],
            "unsupported",
        );
        if manifest.schema_version != 1
            || manifest.version != PYTHON_VERSION
            || manifest.platform != expected_platform
            || manifest.abi != "cp314"
            || manifest.archive_sha256 != ARCHIVE_SHA256
        {
            return Err(PythonError::Environment(format!(
                "Expected the pinned CPython 3.14.8 / cp314 / {expected_platform} distribution"
            )));
        }
        for required in required {
            if !manifest.files.contains_key(required)
                && !manifest
                    .files
                    .keys()
                    .any(|name| name.starts_with(&format!("{required}/")))
            {
                return Err(PythonError::Environment(format!(
                    "Manifest missing {required}"
                )));
            }
        }
        for (relative, expected) in &manifest.files {
            let path = Path::new(relative);
            if relative.contains(':')
                || relative.contains('\\')
                || path
                    .components()
                    .any(|part| !matches!(part, Component::Normal(_)))
            {
                return Err(PythonError::Environment(format!(
                    "Invalid runtime path: {relative}"
                )));
            }
            let absolute = fs::canonicalize(root.join(path)).map_err(environment_error)?;
            if !absolute.starts_with(&root) {
                return Err(PythonError::Environment(format!(
                    "Runtime path escapes its directory: {relative}"
                )));
            }
            let mut file = fs::File::open(absolute).map_err(environment_error)?;
            let mut digest = Sha256::new();
            let mut buffer = [0_u8; 65536];
            loop {
                let count = file.read(&mut buffer).map_err(environment_error)?;
                if count == 0 {
                    break;
                }
                digest.update(&buffer[..count]);
            }
            if format!("{:x}", digest.finalize()) != *expected {
                return Err(PythonError::Environment(format!(
                    "SHA-256 mismatch: {relative}"
                )));
            }
            #[cfg(windows)]
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "exe" | "dll" | "pyd"
                    )
                })
            {
                verify_amd64(&mut file, relative)?;
            }
            #[cfg(target_os = "macos")]
            if path
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|ext| matches!(ext, "dylib" | "so"))
                || *relative == _executable
            {
                verify_macho(
                    &mut file,
                    relative,
                    if cfg!(target_arch = "aarch64") {
                        0x0100000c
                    } else {
                        0x01000007
                    },
                )?;
            }
        }
        let mut actual_files = BTreeSet::new();
        collect_files(&root, &root, &mut actual_files)?;
        actual_files.remove("runtime-manifest.json");
        if actual_files != manifest.files.keys().cloned().collect() {
            return Err(PythonError::Environment(
                "Runtime contains files outside its locked manifest".into(),
            ));
        }
        #[cfg(windows)]
        let paths = fs::read_to_string(root.join("python314._pth")).map_err(environment_error)?;
        #[cfg(windows)]
        let effective: Vec<_> = paths
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        #[cfg(windows)]
        if effective != ["python314.zip", ".", "executor", "dependencies"] {
            return Err(PythonError::Environment(
                "Unexpected Python search paths or site initialization".into(),
            ));
        }
        let lock: Value = serde_json::from_slice(
            &fs::read(root.join("dependencies.lock.json")).map_err(environment_error)?,
        )
        .map_err(environment_error)?;
        if lock["schemaVersion"] != 1
            || lock["pythonVersion"] != PYTHON_VERSION
            || lock["platform"] != lock_platform
            || lock["abi"] != "cp314"
            || !lock["packages"].is_array()
        {
            return Err(PythonError::Environment(
                "Dependency lock does not match the runtime".into(),
            ));
        }
        let imports: Vec<String> =
            serde_json::from_value(lock["imports"].clone()).map_err(environment_error)?;
        Ok(Self {
            root,
            manifest,
            imports,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn runtime_id(&self) -> String {
        format!("{}-{}", self.manifest.version, self.manifest.platform)
    }

    pub fn executable(&self) -> PathBuf {
        #[cfg(windows)]
        let relative = "python.exe";
        #[cfg(target_os = "macos")]
        let relative = "bin/python3.14";
        #[cfg(not(any(windows, target_os = "macos")))]
        let relative = "python";
        self.root.join(relative)
    }
}

pub fn runtime_platform() -> &'static str {
    #[cfg(windows)]
    {
        "win_amd64"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        "unsupported"
    }
}

pub fn runtime_id() -> String {
    format!("{PYTHON_VERSION}-{}", runtime_platform())
}

pub fn archive_sha256() -> &'static str {
    ARCHIVE_SHA256
}

#[cfg(target_os = "macos")]
fn verify_macho(file: &mut fs::File, name: &str, expected_cpu: u32) -> Result<(), PythonError> {
    file.seek(SeekFrom::Start(0)).map_err(environment_error)?;
    let mut header = [0_u8; 8];
    file.read_exact(&mut header).map_err(environment_error)?;
    let magic_be = u32::from_be_bytes(header[..4].try_into().expect("four bytes"));
    if matches!(magic_be, 0xcafebabe | 0xcafebabf | 0xbebafeca | 0xbfbafeca) {
        let little = matches!(magic_be, 0xbebafeca | 0xbfbafeca);
        let count = if little {
            u32::from_le_bytes(header[4..8].try_into().expect("four bytes"))
        } else {
            u32::from_be_bytes(header[4..8].try_into().expect("four bytes"))
        };
        let entry_size = if matches!(magic_be, 0xcafebabf | 0xbfbafeca) {
            32_u64
        } else {
            20_u64
        };
        if count > 64 {
            return Err(PythonError::Environment(format!(
                "Invalid universal Mach-O binary: {name}"
            )));
        }
        let mut found = false;
        for index in 0..count {
            file.seek(SeekFrom::Start(8 + u64::from(index) * entry_size))
                .map_err(environment_error)?;
            let mut cpu_bytes = [0_u8; 4];
            file.read_exact(&mut cpu_bytes).map_err(environment_error)?;
            let cpu = if little {
                u32::from_le_bytes(cpu_bytes)
            } else {
                u32::from_be_bytes(cpu_bytes)
            };
            found |= cpu == expected_cpu;
        }
        if found {
            return Ok(());
        }
        return Err(PythonError::Environment(format!(
            "Unexpected universal Mach-O architecture: {name}"
        )));
    }
    let little_endian = matches!(magic_be, 0xcefaedfe | 0xcffaedfe);
    let magic = if little_endian {
        u32::from_le_bytes(header[..4].try_into().expect("four bytes"))
    } else {
        magic_be
    };
    let cpu = if little_endian {
        u32::from_le_bytes(header[4..8].try_into().expect("four bytes"))
    } else {
        u32::from_be_bytes(header[4..8].try_into().expect("four bytes"))
    };
    if !matches!(magic, 0xfeedface | 0xfeedfacf) || cpu != expected_cpu {
        return Err(PythonError::Environment(format!(
            "Unexpected Mach-O architecture: {name}"
        )));
    }
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    result: &mut BTreeSet<String>,
) -> Result<(), PythonError> {
    for entry in fs::read_dir(directory).map_err(environment_error)? {
        let entry = entry.map_err(environment_error)?;
        let kind = entry.file_type().map_err(environment_error)?;
        if kind.is_symlink() {
            return Err(PythonError::Environment(
                "Runtime symlinks/reparse points are not supported".into(),
            ));
        }
        if kind.is_dir() {
            if entry.file_name() == "__pycache__" {
                continue;
            }
            collect_files(root, &entry.path(), result)?;
        } else {
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("pyc") {
                continue;
            }
            result.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(environment_error)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn verify_amd64(file: &mut fs::File, name: &str) -> Result<(), PythonError> {
    file.seek(SeekFrom::Start(0)).map_err(environment_error)?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos).map_err(environment_error)?;
    if &dos[..2] != b"MZ" {
        return Err(PythonError::Environment(format!(
            "Not a Windows PE binary: {name}"
        )));
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().expect("four bytes"));
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(environment_error)?;
    let mut pe = [0_u8; 6];
    file.read_exact(&mut pe).map_err(environment_error)?;
    if &pe[..4] != b"PE\0\0" || u16::from_le_bytes([pe[4], pe[5]]) != 0x8664 {
        return Err(PythonError::Environment(format!(
            "Expected Windows x64 binary: {name}"
        )));
    }
    Ok(())
}

fn environment_error(error: impl std::fmt::Display) -> PythonError {
    PythonError::Environment(error.to_string())
}

#[derive(Debug, Clone, Copy)]
pub struct PythonLimits {
    pub timeout: Duration,
    /// Windows Job committed memory. macOS samples aggregate process-group RSS; this is approximate, not a hard limit.
    pub memory_bytes: usize,
}

impl Default for PythonLimits {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(5),
            memory_bytes: 256 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PythonResult {
    pub message: Option<Value>,
    pub logs: String,
}

/// Construct once per project session and clone it to share the FIFO task queue.
#[derive(Clone)]
pub struct PythonExecutor {
    environment: Arc<PythonEnvironment>,
    capacity: Arc<Semaphore>,
    serial: Arc<Semaphore>,
}

impl PythonExecutor {
    pub fn new(environment: PythonEnvironment) -> Self {
        Self {
            environment: Arc::new(environment),
            capacity: Arc::new(Semaphore::new(65)),
            serial: Arc::new(Semaphore::new(1)),
        }
    }

    pub async fn execute(
        &self,
        source: &str,
        message: Value,
        limits: PythonLimits,
        cancel: CancellationToken,
    ) -> Result<PythonResult, PythonError> {
        if !message.is_object()
            || serde_json::to_vec(&message)
                .map_err(environment_error)?
                .len()
                > MAX_MESSAGE_BYTES
        {
            return Err(PythonError::InvalidInput(
                "Message must be an object of at most 1 MiB".into(),
            ));
        }
        let (result, logs) = self
            .request(
                "execute",
                json!({"source":source,"message":message}),
                limits,
                cancel,
            )
            .await?;
        let value = result
            .get("message")
            .ok_or_else(|| PythonError::Process("Missing message result".into()))?;
        if !value.is_null() && !value.is_object() {
            return Err(PythonError::Process("Non-object message result".into()));
        }
        Ok(PythonResult {
            message: (!value.is_null()).then(|| value.clone()),
            logs,
        })
    }

    /// AST and compile checks only. User module imports and top-level code are not executed.
    pub async fn validate_source(&self, source: &str) -> Result<(), PythonError> {
        self.request(
            "validate",
            json!({"source":source}),
            PythonLimits::default(),
            CancellationToken::new(),
        )
        .await?;
        Ok(())
    }

    /// Import declared dependency modules inside the same limited environment used for execution.
    pub async fn validate_imports(&self) -> Result<(), PythonError> {
        self.validate_imports_with_limits(PythonLimits::default())
            .await
    }

    pub async fn validate_imports_with_limits(
        &self,
        limits: PythonLimits,
    ) -> Result<(), PythonError> {
        // Probe each declared import in a fresh process, avoiding artificial
        // accumulation of modules which may be used by different code nodes.
        for module in &self.environment.imports {
            self.request(
                "imports",
                json!({"modules":[module]}),
                limits,
                CancellationToken::new(),
            )
            .await?;
        }
        Ok(())
    }

    async fn request(
        &self,
        method: &str,
        params: Value,
        limits: PythonLimits,
        cancel: CancellationToken,
    ) -> Result<(Value, String), PythonError> {
        if limits.timeout.is_zero() || limits.memory_bytes < 16 * 1024 * 1024 {
            return Err(PythonError::InvalidInput(
                "Timeout must be positive and memory limit at least 16 MiB".into(),
            ));
        }
        let capacity = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| PythonError::QueueFull)?;
        let cancel = cancel.child_token();
        // Dropping an in-flight future also terminates the blocking worker's Job.
        let _cancel_on_drop = cancel.clone().drop_guard();
        let serial = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err(PythonError::Cancelled),
            permit = self.serial.clone().acquire_owned() => permit.map_err(|_| PythonError::Cancelled)?,
        };
        let mut request =
            serde_json::to_vec(&json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}))
                .map_err(environment_error)?;
        if request.len() > 4 * MAX_MESSAGE_BYTES {
            return Err(PythonError::InvalidInput("Request exceeds 4 MiB".into()));
        }
        request.push(b'\n');
        let environment = self.environment.clone();
        let output = tokio::task::spawn_blocking(move || {
            let _capacity = capacity;
            let _serial = serial;
            #[cfg(windows)]
            {
                process::run(&environment, request, limits, cancel)
            }
            #[cfg(target_os = "macos")]
            {
                process::run(&environment, request, limits, cancel)
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            {
                let _ = (environment, request, limits, cancel);
                Err(PythonError::Environment(
                    "Only Windows x64 is supported".into(),
                ))
            }
        })
        .await
        .map_err(|e| PythonError::Process(e.to_string()))??;
        let response: Value = serde_json::from_slice(&output.0).map_err(|error| {
            PythonError::Process(format!("Invalid worker protocol: {error}; {}", output.1))
        })?;
        if response["jsonrpc"] != "2.0" || response["id"] != 1 {
            return Err(PythonError::Process(
                "Invalid JSON-RPC response identity".into(),
            ));
        }
        if let Some(error) = response.get("error") {
            return Err(PythonError::Code {
                message: error["message"]
                    .as_str()
                    .unwrap_or("Unknown Python error")
                    .to_owned(),
                logs: output.1,
            });
        }
        Ok((
            response
                .get("result")
                .cloned()
                .ok_or_else(|| PythonError::Process("Missing worker result".into()))?,
            output.1,
        ))
    }
}
