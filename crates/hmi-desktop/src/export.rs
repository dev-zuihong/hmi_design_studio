use crate::resources::*;
use hmi_core::Project;
use serde::Serialize;
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    io::{Read, Write},
    path::Path,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub files: usize,
    pub includes_python: bool,
}

pub fn package(
    resources: &Resources,
    project: &Project,
    project_directory: &Path,
    destination: &Path,
) -> Result<ExportResult> {
    if destination.exists() {
        return Err("目标文件已存在，请选择新文件名".into());
    }
    if destination
        .extension()
        .and_then(|v| v.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
        != Some("zip")
    {
        return Err("导出目标必须为 .zip 文件".into());
    }
    let template = resources.template();
    check_amd64(&template.join("HmiRunner.exe"))?;
    check_webview(&resources.webview())?;
    let staging = tempfile::tempdir().map_err(error)?;
    let root = staging.path();
    // The first-version template contract is one self-contained Tauri executable.
    // Never copy previous projects, Python environments or unrelated cache files
    // accidentally placed in a template directory.
    let runner_source = contained_file(&template, "HmiRunner.exe")?;
    reject_reparse(&template.join("HmiRunner.exe"))?;
    fs::copy(&runner_source, root.join("HmiRunner.exe")).map_err(error)?;
    if sha256(&runner_source)? != sha256(&root.join("HmiRunner.exe"))? {
        return Err("Runner 复制后校验失败".into());
    }
    copy_tree(&resources.webview(), &root.join("webview2"))?;
    check_webview(&root.join("webview2"))?;
    // Copy the whole declared plugin package, including its private Python dependencies.
    let protocols: BTreeSet<_> = project
        .connections
        .iter()
        .map(|c| c.protocol_id.as_str())
        .collect();
    for id in protocols {
        if !hmi_core::safe_identifier(id) {
            return Err("协议 ID 无效".into());
        }
        copy_tree(
            &resources.plugins().join(id),
            &root.join("plugins").join(id),
        )?;
        crate::session::check_plugin(&root.join("plugins").join(id))?;
    }
    let mut export_project = project.clone();
    crate::session::pin_plugins(resources, &mut export_project)?;
    for asset in &mut export_project.assets {
        if !hmi_core::safe_identifier(&asset.id) {
            return Err("资源 ID 无效".into());
        }
        let source = contained_file(project_directory, &asset.path)?;
        let extension = source.extension().and_then(|s| s.to_str()).unwrap_or("bin");
        let relative = format!("assets/{}.{}", asset.id, extension);
        let target = root.join(&relative);
        fs::create_dir_all(target.parent().unwrap()).map_err(error)?;
        fs::copy(&source, &target).map_err(error)?;
        if sha256(&source)? != sha256(&target)? {
            return Err(format!("资源复制后校验失败: {}", asset.id));
        }
        asset.path = relative;
    }
    let python = has_python(project);
    if python {
        let source = resources.python(&project.id);
        let env = hmi_python::PythonEnvironment::load(&source).map_err(error)?;
        copy_tree(env.root(), &root.join("runtime/python"))?;
        hmi_python::PythonEnvironment::load(root.join("runtime/python")).map_err(error)?;
        let config = export_project
            .python
            .as_mut()
            .ok_or("未指定 Python 运行资源")?;
        let copied_lock = read_json(&root.join("runtime/python/dependencies.lock.json"))?;
        if json!(config.dependency_lock) != copied_lock["packages"] {
            return Err("复制后的 Python 依赖与项目锁定清单不一致".into());
        }
        if config
            .runtime_sha256
            .as_deref()
            .is_some_and(|hash| hash != hmi_python::ARCHIVE_SHA256)
        {
            return Err("项目 Python 校验和不一致".into());
        }
        config.runtime_sha256 = Some(hmi_python::ARCHIVE_SHA256.into());
    } else {
        export_project.python = None;
    }
    write_json(&root.join("project.json"), &export_project)?;
    fs::write(
        root.join("START.cmd"),
        "@echo off\r\nstart \"\" \"%~dp0HmiRunner.exe\" --project \"%~dp0project.json\"\r\n",
    )
    .map_err(error)?;
    fs::write(root.join("README.txt"), "HMI Runtime — Windows 10/11 x64\r\nExtract the complete ZIP to a local disk, then launch HmiRunner.exe or START.cmd.\r\nNo Python, Node.js or compiler installation is required. Device drivers remain a system prerequisite.\r\nSettings and logs are stored under the current user's local application data directory.\r\nPython and protocol plugins run as the current user; use trusted projects.\r\n").map_err(error)?;
    let mut files = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(error)?;
        if entry.file_type().is_file() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                sha256(entry.path())?,
            );
        }
    }
    write_json(
        &root.join("release-manifest.json"),
        &json!({
            "schemaVersion":1,"applicationVersion":env!("CARGO_PKG_VERSION"),"platform":"windows-x64",
            "projectId":project.id,"includesPython":python,"files":files,
        }),
    )?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(error)?;
    let mut archive_file = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    {
        let mut zip = zip::ZipWriter::new(archive_file.as_file_mut());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
            let entry = entry.map_err(error)?;
            if entry.file_type().is_dir() {
                let relative = entry
                    .path()
                    .strip_prefix(root)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/");
                if !relative.is_empty() {
                    zip.add_directory(format!("{relative}/"), options)
                        .map_err(error)?;
                }
                continue;
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(error)?
                .to_string_lossy()
                .replace('\\', "/");
            zip.start_file(relative, options).map_err(error)?;
            let mut file = fs::File::open(entry.path()).map_err(error)?;
            let mut buffer = [0u8; 65536];
            loop {
                let n = file.read(&mut buffer).map_err(error)?;
                if n == 0 {
                    break;
                }
                zip.write_all(&buffer[..n]).map_err(error)?;
            }
        }
        zip.finish().map_err(error)?;
    }
    archive_file.as_file().sync_all().map_err(error)?;
    archive_file.persist_noclobber(destination).map_err(error)?;
    Ok(ExportResult {
        path: destination.to_string_lossy().into_owned(),
        files: files.len() + 1,
        includes_python: python,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::tests::{fake_pe, webview_fixture};
    use serde_json::Value;
    #[test]
    fn resources_cannot_escape_project() {
        for path in [
            "../secret",
            "/absolute",
            "C:/secret",
            "a/../../secret",
            "a\\b",
            "",
        ] {
            assert!(safe_relative(path).is_err(), "{path}");
        }
        assert!(safe_relative("assets/font.woff2").is_ok());
    }
    #[test]
    fn disabled_python_nodes_still_require_runtime() {
        let mut project: Project = serde_json::from_value(json!({"schemaVersion":1,"id":"test","name":"test","width":1280,"height":720,"pages":[],"flows":[{"id":"f","name":"f","enabled":false,"timeoutMs":30000,"nodes":[{"id":"p","kind":"python","label":"p","position":{"x":0,"y":0},"config":{}}],"edges":[]}]})).unwrap();
        assert!(has_python(&project));
        project.flows.clear();
        assert!(!has_python(&project));
    }

    #[test]
    fn release_is_atomic_self_contained_and_python_is_only_added_on_demand() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let resources = Resources {
            root: root.join("resources"),
            runner: false,
        };
        fs::create_dir_all(resources.template().join("runtime/python")).unwrap();
        fs::write(resources.template().join("HmiRunner.exe"), fake_pe()).unwrap();
        fs::write(
            resources.template().join("runtime/python/stale.py"),
            "must not ship",
        )
        .unwrap();
        webview_fixture(&resources.webview());
        let plugin = resources.plugins().join("demo-temperature");
        fs::create_dir_all(plugin.join("private-python")).unwrap();
        fs::create_dir_all(plugin.join("empty-state-directory")).unwrap();
        fs::write(plugin.join("demo-protocol.exe"), fake_pe()).unwrap();
        fs::write(
            plugin.join("private-python/required.dll"),
            "plugin-owned-dependency",
        )
        .unwrap();
        fs::write(
            plugin.join("manifest.json"),
            include_str!("../../demo-protocol/package/manifest.json"),
        )
        .unwrap();
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/picture.png"), "image-content").unwrap();
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"test","name":"test","width":1280,"height":720,"pages":[],
            "assets":[{"id":"picture","path":"assets/picture.png"}],
            "connections":[{"id":"device","name":"Device","port":"sim://temperature","baudRate":115200,
                "dataBits":8,"stopBits":1,"parity":"none","protocolId":"demo-temperature"}],
            "python":{"runtimeId":"3.14.8-win_amd64","dependencyLock":[]}
        })).unwrap();
        let destination = root.join("runtime.zip");
        let result = package(&resources, &project, root, &destination).unwrap();
        assert!(!result.includes_python);
        let mut archive = zip::ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
        let names: Vec<_> = archive.file_names().map(str::to_owned).collect();
        assert!(names
            .iter()
            .any(|name| name == "plugins/demo-temperature/empty-state-directory/"));
        assert!(!names.iter().any(|name| name.starts_with("runtime/python/")));
        for required in [
            "HmiRunner.exe",
            "webview2/msedgewebview2.exe",
            "webview2/msedge.dll",
            "plugins/demo-temperature/private-python/required.dll",
            "assets/picture.png",
            "START.cmd",
        ] {
            assert!(names.iter().any(|name| name == required), "{required}");
        }
        let manifest: Value =
            serde_json::from_reader(archive.by_name("release-manifest.json").unwrap()).unwrap();
        assert!(manifest["files"].get("release-manifest.json").is_none());
        assert_eq!(
            manifest["files"].as_object().unwrap().len() + 1,
            names.iter().filter(|name| !name.ends_with('/')).count()
        );
        for (path, expected) in manifest["files"].as_object().unwrap() {
            let mut bytes = Vec::new();
            archive
                .by_name(path)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            use sha2::{Digest, Sha256};
            assert_eq!(
                format!("{:x}", Sha256::digest(bytes)),
                expected.as_str().unwrap()
            );
        }
        let exported_project: Value =
            serde_json::from_reader(archive.by_name("project.json").unwrap()).unwrap();
        assert!(exported_project.get("python").is_none());
        drop(archive);
        let original = sha256(&destination).unwrap();
        assert!(package(&resources, &project, root, &destination).is_err());
        assert_eq!(sha256(&destination).unwrap(), original);
        let mut python_project = project.clone();
        python_project.flows = serde_json::from_value(json!([{
            "id":"python_flow","name":"Python","enabled":false,"timeoutMs":30000,
            "nodes":[{"id":"python_node","kind":"python","label":"Python","position":{"x":0,"y":0},
                "config":{"code":"def transform(msg):\n return msg"}}],"edges":[]
        }]))
        .unwrap();
        let missing_python = root.join("missing-python.zip");
        assert!(package(&resources, &python_project, root, &missing_python).is_err());
        assert!(!missing_python.exists());
        fs::write(resources.webview().join("icudtl.dat"), "tampered").unwrap();
        let failed = root.join("failed.zip");
        assert!(package(&resources, &project, root, &failed).is_err());
        assert!(!failed.exists());
    }
}
