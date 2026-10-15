use hmi_core::Project;
use hmi_python::{PythonEnvironment, PythonExecutor};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

pub type Result<T> = std::result::Result<T, String>;
pub fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Clone)]
pub struct Resources {
    pub root: PathBuf,
    pub runner: bool,
}
impl Resources {
    pub fn discover(runner: bool) -> Self {
        let executable = std::env::current_exe().unwrap_or_default();
        let directory = executable.parent().unwrap_or(Path::new(".")).to_path_buf();
        let root = if runner && directory.join("project.json").exists() {
            directory
        } else if let Some(path) = std::env::var_os("HMI_RESOURCE_ROOT") {
            PathBuf::from(path)
        } else if directory.join("resources").is_dir() {
            directory.join("resources")
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.cache")
        };
        Self { root, runner }
    }
    pub fn plugins(&self) -> PathBuf {
        self.root.join("plugins")
    }
    pub fn template(&self) -> PathBuf {
        self.root.join("templates/windows-x64")
    }
    pub fn webview(&self) -> PathBuf {
        self.root.join("webview2")
    }
    pub fn python(&self, project_id: &str) -> PathBuf {
        if self.runner {
            return self.root.join("runtime/python");
        }
        let custom = self.root.join("python/projects").join(project_id);
        if custom.exists() {
            custom
        } else {
            self.root.join("python/3.14.8-win_amd64")
        }
    }
    pub fn plugin_manifests(&self) -> Result<Vec<Value>> {
        if !self.plugins().exists() {
            return Ok(vec![]);
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(self.plugins()).map_err(error)? {
            let path = entry.map_err(error)?.path().join("manifest.json");
            if path.is_file() {
                manifests.push(read_json(&path)?);
            }
        }
        Ok(manifests)
    }
}

pub fn has_python(project: &Project) -> bool {
    project
        .flows
        .iter()
        .flat_map(|f| &f.nodes)
        .any(|n| n.kind == "python")
}
pub async fn check_python(
    resources: &Resources,
    project: &Project,
) -> Result<Option<PythonExecutor>> {
    if !has_python(project) {
        return Ok(None);
    }
    let config = project.python.as_ref().ok_or("未指定 Python 运行资源")?;
    if config.runtime_id != "3.14.8-win_amd64" {
        return Err("Python 资源必须为 3.14.8-win_amd64".into());
    }
    if config
        .runtime_sha256
        .as_deref()
        .is_some_and(|v| v != hmi_python::ARCHIVE_SHA256)
    {
        return Err("项目 Python SHA-256 与固定运行资源不一致".into());
    }
    let env = PythonEnvironment::load(resources.python(&project.id)).map_err(error)?;
    let lock = read_json(&env.root().join("dependencies.lock.json"))?;
    if lock["packages"] != json!(config.dependency_lock) {
        return Err("Python 依赖锁与项目不一致，请重新导入对应资源".into());
    }
    let executor = PythonExecutor::new(env);
    let memory_mb = project
        .flows
        .iter()
        .flat_map(|f| &f.nodes)
        .filter(|n| n.kind == "python")
        .map(|n| {
            n.config
                .get("memoryMb")
                .and_then(Value::as_u64)
                .unwrap_or(256)
        })
        .max()
        .unwrap_or(256);
    let timeout_ms = project
        .flows
        .iter()
        .flat_map(|f| &f.nodes)
        .filter(|n| n.kind == "python")
        .map(|n| {
            n.config
                .get("timeoutMs")
                .and_then(Value::as_u64)
                .unwrap_or(5000)
        })
        .max()
        .unwrap_or(5000);
    executor
        .validate_imports_with_limits(hmi_python::PythonLimits {
            timeout: std::time::Duration::from_millis(timeout_ms),
            memory_bytes: (memory_mb * 1024 * 1024) as usize,
        })
        .await
        .map_err(error)?;
    for flow in &project.flows {
        for node in flow.nodes.iter().filter(|n| n.kind == "python") {
            executor
                .validate_source(
                    node.config
                        .get("code")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                )
                .await
                .map_err(|e| format!("流程 {} / 节点 {}: {e}", flow.name, node.label))?;
        }
    }
    Ok(Some(executor))
}
pub fn read_json(path: &Path) -> Result<Value> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(error)?
        .take(32 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(error)?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("JSON 文件超过 32 MiB".into());
    }
    serde_json::from_slice(bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes)).map_err(error)
}
pub fn safe_relative(path: &str) -> Result<PathBuf> {
    if path.is_empty() || path.contains(':') || path.contains('\\') {
        return Err(format!("非法资源路径: {path}"));
    }
    let p = Path::new(path);
    if p.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("资源路径必须位于项目内: {path}"));
    }
    Ok(p.to_path_buf())
}
pub fn contained_file(root: &Path, path: &str) -> Result<PathBuf> {
    let root = fs::canonicalize(root).map_err(error)?;
    let file = fs::canonicalize(root.join(safe_relative(path)?)).map_err(error)?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(format!("资源越界或不存在: {path}"));
    }
    Ok(file)
}
pub fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    if !source.is_dir() {
        return Err(format!("资源目录不存在: {}", source.display()));
    }
    reject_reparse(source)?;
    fs::create_dir_all(destination).map_err(error)?;
    reject_reparse(destination)?;
    for entry in walkdir::WalkDir::new(source).follow_links(false) {
        let entry = entry.map_err(error)?;
        reject_reparse(entry.path())?;
        let relative = entry.path().strip_prefix(source).map_err(error)?;
        let target = destination.join(relative);
        if target.exists() {
            reject_reparse(&target)?;
        }
        if entry.file_type().is_dir() {
            fs::create_dir_all(target).map_err(error)?;
        } else if entry.file_type().is_file() {
            fs::copy(entry.path(), target).map_err(error)?;
        }
    }
    Ok(())
}

pub fn reject_reparse(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if metadata.file_type().is_symlink() || reparse {
        return Err(format!(
            "资源不能包含符号链接或 NTFS 重解析点: {}",
            path.display()
        ));
    }
    Ok(())
}

/// Validate the complete pinned fixed runtime, including files after copying.
pub fn check_webview(directory: &Path) -> Result<()> {
    let source: Value =
        serde_json::from_str(include_str!("../../../resources/webview2-source.json"))
            .map_err(error)?;
    let manifest = read_json(&directory.join("webview2-manifest.json"))?;
    if manifest["schemaVersion"] != 1
        || manifest["version"] != source["version"]
        || manifest["platform"] != source["platform"]
        || manifest["archiveSha256"] != source["sha256"]
    {
        return Err("WebView2 固定版本、架构或发布校验和不一致".into());
    }
    let files = manifest["files"]
        .as_object()
        .ok_or("WebView2 文件清单缺失")?;
    if files.contains_key("webview2-manifest.json")
        || !files.contains_key("msedgewebview2.exe")
        || !files.contains_key("msedge.dll")
        || !files.contains_key("icudtl.dat")
    {
        return Err("WebView2 文件清单缺少必要文件或错误包含清单自身".into());
    }
    let mut actual = std::collections::BTreeSet::new();
    for entry in walkdir::WalkDir::new(directory).follow_links(false) {
        let entry = entry.map_err(error)?;
        reject_reparse(entry.path())?;
        if entry.file_type().is_file() {
            actual.insert(
                entry
                    .path()
                    .strip_prefix(directory)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    actual.remove("webview2-manifest.json");
    if actual != files.keys().cloned().collect() {
        return Err("WebView2 文件与锁定清单不一致".into());
    }
    for (relative, expected) in files {
        let path = contained_file(directory, relative)?;
        if sha256(&path)? != expected.as_str().ok_or("WebView2 文件校验和无效")? {
            return Err(format!("WebView2 文件 SHA-256 不匹配: {relative}"));
        }
    }
    check_amd64(&directory.join("msedgewebview2.exe"))
}
pub fn sha256(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path).map_err(error)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(error)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(error)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    temporary
        .write_all(&serde_json::to_vec_pretty(value).map_err(error)?)
        .map_err(error)?;
    temporary.as_file().sync_all().map_err(error)?;
    temporary.persist(path).map_err(error)?;
    Ok(())
}

pub fn check_amd64(path: &Path) -> Result<()> {
    use std::io::{Seek, SeekFrom};
    let mut f = fs::File::open(path).map_err(error)?;
    let mut header = [0u8; 64];
    f.read_exact(&mut header).map_err(error)?;
    if &header[..2] != b"MZ" {
        return Err(format!("不是 Windows 程序: {}", path.display()));
    }
    let offset = u32::from_le_bytes(header[60..64].try_into().unwrap());
    f.seek(SeekFrom::Start(offset as u64)).map_err(error)?;
    let mut pe = [0u8; 6];
    f.read_exact(&mut pe).map_err(error)?;
    if &pe[..4] != b"PE\0\0" || u16::from_le_bytes([pe[4], pe[5]]) != 0x8664 {
        return Err(format!("程序必须为 Windows x64: {}", path.display()));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn fake_pe() -> Vec<u8> {
        let mut data = vec![0_u8; 128];
        data[..2].copy_from_slice(b"MZ");
        data[60..64].copy_from_slice(&64_u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&0x8664_u16.to_le_bytes());
        data
    }

    pub fn webview_fixture(directory: &Path) {
        fs::create_dir_all(directory).unwrap();
        fs::write(directory.join("msedgewebview2.exe"), fake_pe()).unwrap();
        fs::write(directory.join("msedge.dll"), fake_pe()).unwrap();
        fs::write(directory.join("icudtl.dat"), "fixture").unwrap();
        let source: Value =
            serde_json::from_str(include_str!("../../../resources/webview2-source.json")).unwrap();
        let mut files = serde_json::Map::new();
        for name in ["msedgewebview2.exe", "msedge.dll", "icudtl.dat"] {
            files.insert(name.into(), json!(sha256(&directory.join(name)).unwrap()));
        }
        write_json(
            &directory.join("webview2-manifest.json"),
            &json!({
                "schemaVersion":1,"version":source["version"],"platform":source["platform"],
                "archiveSha256":source["sha256"],"files":files,
            }),
        )
        .unwrap();
    }

    #[test]
    fn webview_rejects_tampering_unknown_files_and_wrong_release() {
        let root = tempfile::tempdir().unwrap();
        webview_fixture(root.path());
        check_webview(root.path()).unwrap();
        fs::write(root.path().join("unlocked.dll"), "unexpected").unwrap();
        assert!(check_webview(root.path()).is_err());
        fs::remove_file(root.path().join("unlocked.dll")).unwrap();
        fs::write(root.path().join("icudtl.dat"), "changed").unwrap();
        assert!(check_webview(root.path()).is_err());
        webview_fixture(root.path());
        let mut manifest = read_json(&root.path().join("webview2-manifest.json")).unwrap();
        manifest["version"] = json!("1.0.0.0");
        write_json(&root.path().join("webview2-manifest.json"), &manifest).unwrap();
        assert!(check_webview(root.path()).is_err());
        webview_fixture(root.path());
        let mut manifest = read_json(&root.path().join("webview2-manifest.json")).unwrap();
        manifest["files"]["webview2-manifest.json"] = json!("self-reference");
        write_json(&root.path().join("webview2-manifest.json"), &manifest).unwrap();
        assert!(check_webview(root.path()).is_err());
    }

    #[test]
    fn copied_runtime_is_verified_after_source_is_gone() {
        let source = tempfile::tempdir().unwrap();
        let destination = tempfile::tempdir().unwrap();
        webview_fixture(source.path());
        copy_tree(source.path(), destination.path()).unwrap();
        source.close().unwrap();
        check_webview(destination.path()).unwrap();
    }
}
