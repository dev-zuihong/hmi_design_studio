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
    package_with_project_resources(
        resources,
        resources,
        project,
        project_directory,
        destination,
    )
}

pub fn package_with_project_resources(
    resources: &Resources,
    project_resources: &Resources,
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
    let platform = crate::resources::release_platform();
    let is_macos = platform.starts_with("macos-");
    if !is_macos && platform != "windows-x64" {
        return Err(format!("不支持导出平台: {platform}"));
    }
    let template = resources.template();
    let runner_source = template.join(crate::resources::runner_relative());
    crate::resources::check_native_binary(&runner_source)?;
    let staging = tempfile::tempdir().map_err(error)?;
    let root = staging.path();
    // Windows ships the loose executable and fixed WebView2 runtime. macOS
    // ships the complete native app bundle and uses the system WKWebView.
    let resource_root = if is_macos {
        let app_source = template.join("HmiRunner.app");
        let app_resources = app_source.join("Contents/Resources");
        reject_stale_app_resources(&app_resources)?;
        let app_target = root.join("HmiRunner.app");
        copy_app_bundle(&app_source, &app_target)?;
        let copied_runner = root.join(crate::resources::runner_relative());
        crate::resources::check_native_binary(&copied_runner)?;
        app_target.join("Contents/Resources")
    } else {
        check_webview(&resources.webview())?;
        let runner_source = contained_file(&template, crate::resources::runner_relative())?;
        reject_reparse(&runner_source)?;
        let runner_target = root.join("HmiRunner.exe");
        fs::copy(&runner_source, &runner_target).map_err(error)?;
        if sha256(&runner_source)? != sha256(&runner_target)? {
            return Err("Runner 复制后校验失败".into());
        }
        copy_tree(&resources.webview(), &root.join("webview2"))?;
        check_webview(&root.join("webview2"))?;
        root.to_path_buf()
    };
    // Copy the whole declared plugin package, including its private Python dependencies.
    let protocols: BTreeSet<_> = project
        .connections
        .iter()
        .map(|c| c.protocol_id.as_str())
        .collect();
    for id in protocols.iter().copied() {
        if !hmi_core::safe_identifier(id) {
            return Err("协议 ID 无效".into());
        }
        copy_tree(
            &project_resources.plugins().join(id),
            &resource_root.join("plugins").join(id),
        )?;
        crate::session::check_plugin(&resource_root.join("plugins").join(id))?;
    }
    let mut export_project = project.clone();
    let providers: BTreeSet<_> = project
        .backends
        .iter()
        .map(|backend| backend.provider_id.as_str())
        .collect();
    for id in providers {
        if !hmi_core::safe_identifier(id) {
            return Err("后端 ID 无效".into());
        }
        let destination = resource_root.join("backends").join(id);
        copy_tree(&project_resources.backends().join(id), &destination)?;
        crate::session::check_backend(&destination)?;
    }
    crate::session::pin_plugins(project_resources, &mut export_project)?;
    for asset in &mut export_project.assets {
        if !hmi_core::safe_identifier(&asset.id) {
            return Err("资源 ID 无效".into());
        }
        let source = contained_file(project_directory, &asset.path)?;
        let extension = source.extension().and_then(|s| s.to_str()).unwrap_or("bin");
        let relative = format!("assets/{}.{}", asset.id, extension);
        let target = resource_root.join(&relative);
        fs::create_dir_all(target.parent().unwrap()).map_err(error)?;
        fs::copy(&source, &target).map_err(error)?;
        if sha256(&source)? != sha256(&target)? {
            return Err(format!("资源复制后校验失败: {}", asset.id));
        }
        asset.path = relative;
    }
    let python = has_python(project);
    if python {
        if let Some(config) = &project.python {
            if config.runtime_id != hmi_python::runtime_id() {
                return Err("项目 Python 运行资源与当前导出平台不匹配".into());
            }
            if config
                .runtime_sha256
                .as_deref()
                .is_some_and(|hash| hash != hmi_python::archive_sha256())
            {
                return Err("项目 Python 校验和不一致".into());
            }
        }
    }
    if python {
        let source = project_resources.python(&project.id);
        let env = hmi_python::PythonEnvironment::load(&source).map_err(error)?;
        let python_root = resource_root.join("runtime/python");
        copy_tree(env.root(), &python_root)?;
        hmi_python::PythonEnvironment::load(&python_root).map_err(error)?;
        let config = export_project
            .python
            .as_mut()
            .ok_or("未指定 Python 运行资源")?;
        let copied_lock = read_json(&python_root.join("dependencies.lock.json"))?;
        if json!(config.dependency_lock) != copied_lock["packages"] {
            return Err("复制后的 Python 依赖与项目锁定清单不一致".into());
        }
        config.runtime_sha256 = Some(hmi_python::archive_sha256().into());
    } else {
        export_project.python = None;
    }
    write_json(&resource_root.join("project.json"), &export_project)?;
    if is_macos {
        let app = root.join("HmiRunner.app");
        sign_macos_app(&app)?;
        if python {
            hmi_python::PythonEnvironment::load(&resource_root.join("runtime/python"))
                .map_err(|e| format!("签名后 Python 运行资源校验失败: {e}"))?;
        }
        for id in protocols.iter().copied() {
            crate::session::check_plugin(&resource_root.join("plugins").join(id))?;
        }
        fs::write(root.join("README.txt"), "HMI Runtime — macOS 14 or later\nOpen HmiRunner.app to run the project. The app uses the system WKWebView.\nThis app is ad-hoc signed and is not notarized. macOS may require approval in Privacy & Security before first launch.\nNo Python, Node.js or compiler installation is required. Device drivers remain a system prerequisite.\nSettings and logs are stored under the current user's application support directory.\nPython and protocol plugins run as the current user; use trusted projects.\n").map_err(error)?;
    } else {
        fs::write(
            root.join("START.cmd"),
            "@echo off\r\nstart \"\" \"%~dp0HmiRunner.exe\" --project \"%~dp0project.json\"\r\n",
        )
        .map_err(error)?;
        fs::write(root.join("README.txt"), "HMI Runtime — Windows 10/11 x64\r\nExtract the complete ZIP to a local disk, then launch HmiRunner.exe or START.cmd.\r\nNo Python, Node.js or compiler installation is required. Device drivers remain a system prerequisite.\r\nSettings and logs are stored under the current user's local application data directory.\r\nPython and protocol plugins run as the current user; use trusted projects.\r\n").map_err(error)?;
    }
    let mut files = std::collections::BTreeMap::new();
    for entry in walkdir::WalkDir::new(root).sort_by_file_name() {
        let entry = entry.map_err(error)?;
        if entry.file_type().is_file() || entry.file_type().is_symlink() {
            files.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(error)?
                    .to_string_lossy()
                    .replace('\\', "/"),
                release_file_sha256(entry.path())?,
            );
        }
    }
    write_json(
        &root.join("release-manifest.json"),
        &json!({
            "schemaVersion":1,"applicationVersion":env!("CARGO_PKG_VERSION"),"platform":platform,
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
                    zip.add_directory(format!("{relative}/"), directory_zip_options(options))
                        .map_err(error)?;
                }
                continue;
            }
            if !entry.file_type().is_file() && !entry.file_type().is_symlink() {
                continue;
            }
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(error)?
                .to_string_lossy()
                .replace('\\', "/");
            if entry.file_type().is_symlink() {
                let link = fs::read_link(entry.path()).map_err(error)?;
                zip.add_symlink(relative, link.to_string_lossy(), options)
                    .map_err(error)?;
                continue;
            }
            zip.start_file(relative, file_zip_options(options, entry.path())?)
                .map_err(error)?;
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

fn reject_stale_app_resources(resources: &Path) -> Result<()> {
    if !resources.is_dir() {
        return Err(format!(
            "Runner app 缺少 Contents/Resources: {}",
            resources.display()
        ));
    }
    const FORBIDDEN: &[&str] = &[
        "project.json",
        "plugins",
        "runtime",
        "assets",
        "templates",
        "python",
    ];
    for entry in fs::read_dir(resources).map_err(error)? {
        let entry = entry.map_err(error)?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if FORBIDDEN.iter().any(|item| name == *item) {
            return Err(format!(
                "Runner app 模板 Resources 包含过期项目或运行资源: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub(crate) fn copy_app_bundle(source: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::symlink;

    let source = fs::canonicalize(source).map_err(error)?;
    if !source.is_dir() {
        return Err("macOS Runner 模板不是 .app 目录".into());
    }
    fs::create_dir_all(destination).map_err(error)?;
    for entry in walkdir::WalkDir::new(&source).follow_links(false) {
        let entry = entry.map_err(error)?;
        let relative = entry.path().strip_prefix(&source).map_err(error)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = destination.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(target).map_err(error)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(error)?;
            }
            fs::copy(entry.path(), target).map_err(error)?;
        } else if entry.file_type().is_symlink() {
            let link = fs::read_link(entry.path()).map_err(error)?;
            if link.is_absolute() {
                return Err(format!(
                    "Runner app 包含绝对符号链接: {}",
                    entry.path().display()
                ));
            }
            let resolved = fs::canonicalize(entry.path().parent().unwrap_or(&source).join(&link))
                .map_err(error)?;
            if !resolved.starts_with(&source) {
                return Err(format!(
                    "Runner app 符号链接越界: {}",
                    entry.path().display()
                ));
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(error)?;
            }
            symlink(link, target).map_err(error)?;
        } else {
            return Err(format!(
                "Runner app 包含特殊文件: {}",
                entry.path().display()
            ));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn copy_app_bundle(_source: &Path, _destination: &Path) -> Result<()> {
    Err("macOS .app 只能在 macOS 上组装和签名".into())
}

#[cfg(target_os = "macos")]
fn sign_macos_app(app: &Path) -> Result<()> {
    use std::process::Command;

    let signed = Command::new("/usr/bin/codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(app)
        .status()
        .map_err(error)?;
    if !signed.success() {
        return Err(format!("Runner app ad-hoc 签名失败: {}", app.display()));
    }
    let verified = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(app)
        .status()
        .map_err(error)?;
    if !verified.success() {
        return Err(format!("Runner app 签名校验失败: {}", app.display()));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn sign_macos_app(_app: &Path) -> Result<()> {
    Err("macOS .app 只能在 macOS 上签名".into())
}

#[cfg(unix)]
fn directory_zip_options(options: zip::write::SimpleFileOptions) -> zip::write::SimpleFileOptions {
    options.unix_permissions(0o755)
}

#[cfg(not(unix))]
fn directory_zip_options(options: zip::write::SimpleFileOptions) -> zip::write::SimpleFileOptions {
    options
}

#[cfg(unix)]
fn file_zip_options(
    options: zip::write::SimpleFileOptions,
    path: &Path,
) -> Result<zip::write::SimpleFileOptions> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::symlink_metadata(path).map_err(error)?;
    let mode = if metadata.file_type().is_symlink() {
        0o120777
    } else {
        metadata.permissions().mode() & 0o777
    };
    Ok(options.unix_permissions(mode))
}

#[cfg(not(unix))]
fn file_zip_options(
    options: zip::write::SimpleFileOptions,
    _path: &Path,
) -> Result<zip::write::SimpleFileOptions> {
    Ok(options)
}

fn release_file_sha256(path: &Path) -> Result<String> {
    if fs::symlink_metadata(path)
        .map_err(error)?
        .file_type()
        .is_symlink()
    {
        let link = fs::read_link(path).map_err(error)?;
        let bytes = link.to_string_lossy();
        use sha2::{Digest, Sha256};
        return Ok(format!("{:x}", Sha256::digest(bytes.as_bytes())));
    }
    sha256(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    use crate::resources::tests::fake_pe;
    use crate::resources::tests::webview_fixture;
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

    fn write_executable(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        #[cfg(target_os = "macos")]
        {
            fs::copy(std::env::current_exe().unwrap(), path).unwrap();
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        #[cfg(windows)]
        fs::write(path, fake_pe()).unwrap();
        #[cfg(not(any(target_os = "macos", windows)))]
        panic!("export fixtures require a supported native target");
    }

    fn resources_fixture(root: &Path) -> Resources {
        let resources = Resources {
            root: root.join("resources"),
            runner: false,
        };
        let template = resources.template();
        if crate::resources::release_platform().starts_with("macos-") {
            let app = template.join("HmiRunner.app");
            write_executable(&app.join("Contents/MacOS/hmi-runner"));
            fs::create_dir_all(app.join("Contents/Resources")).unwrap();
            fs::write(
                app.join("Contents/Info.plist"),
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>CFBundleExecutable</key><string>hmi-runner</string><key>CFBundleIdentifier</key><string>org.example.HmiRunner</string><key>CFBundleName</key><string>HmiRunner</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleVersion</key><string>0.1.0</string><key>LSMinimumSystemVersion</key><string>11.0</string></dict></plist>",
            )
            .unwrap();
            fs::write(
                app.join("Contents/Resources/template-marker.txt"),
                "template",
            )
            .unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(
                "template-marker.txt",
                app.join("Contents/Resources/template-marker-link.txt"),
            )
            .unwrap();
        } else {
            write_executable(&template.join("HmiRunner.exe"));
            fs::create_dir_all(template.join("runtime/python")).unwrap();
            fs::write(template.join("runtime/python/stale.py"), "must not ship").unwrap();
            webview_fixture(&resources.webview());
        }
        resources
    }

    fn project_fixture() -> Project {
        serde_json::from_value(json!({
            "schemaVersion":1,"id":"test","name":"test","width":1280,"height":720,"pages":[],
            "assets":[{"id":"picture","path":"assets/picture.png"}],
            "connections":[{"id":"device","name":"Device","port":"sim://temperature","baudRate":115200,
                "dataBits":8,"stopBits":1,"parity":"none","protocolId":"test-protocol"}],
            "python":{"runtimeId":"3.14.8-win_amd64","dependencyLock":[]}
        }))
        .unwrap()
    }

    fn plugin_fixture(resources: &Resources) {
        let plugin = resources.plugins().join("test-protocol");
        fs::create_dir_all(plugin.join("private-python")).unwrap();
        fs::create_dir_all(plugin.join("empty-state-directory")).unwrap();
        let executable = if crate::resources::release_platform() == "windows-x64" {
            "test-protocol.exe"
        } else {
            "test-protocol"
        };
        write_executable(&plugin.join(executable));
        fs::write(
            plugin.join("private-python/required.dll"),
            "plugin-owned-dependency",
        )
        .unwrap();
        fs::write(
            plugin.join("manifest.json"),
            serde_json::to_vec(&json!({
                "id":"test-protocol","name":"Test protocol","version":"1.0.0","apiVersion":1,
                "target":hmi_core::host_target(),"executable":executable,
                "configSchema":{"type":"object"},"commands":[],"points":[]
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn release_is_atomic_platform_native_and_self_contained() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let resources = resources_fixture(root);
        plugin_fixture(&resources);
        fs::create_dir_all(root.join("assets")).unwrap();
        fs::write(root.join("assets/picture.png"), "image-content").unwrap();
        let project = project_fixture();
        let destination = root.join("runtime.zip");
        let result = package(&resources, &project, root, &destination).unwrap();
        assert!(!result.includes_python);
        let mut archive = zip::ZipArchive::new(fs::File::open(&destination).unwrap()).unwrap();
        let names: Vec<_> = archive.file_names().map(str::to_owned).collect();
        let platform = crate::resources::release_platform();
        let (runner, resources_prefix) = if platform.starts_with("macos-") {
            (
                "HmiRunner.app/Contents/MacOS/hmi-runner",
                "HmiRunner.app/Contents/Resources/",
            )
        } else {
            ("HmiRunner.exe", "")
        };
        let runner_file = archive.by_name(runner).unwrap();
        if platform.starts_with("macos-") {
            assert!(runner_file.unix_mode().unwrap() & 0o111 != 0);
        }
        drop(runner_file);
        let plugin_prefix = format!("{resources_prefix}plugins/test-protocol/");
        assert!(names
            .iter()
            .any(|name| name == &format!("{plugin_prefix}empty-state-directory/")));
        assert!(!names.iter().any(|name| name.contains("runtime/python/")));
        assert!(names.iter().any(|name| name
            == &format!("{resources_prefix}plugins/test-protocol/private-python/required.dll")));
        assert!(names
            .iter()
            .any(|name| name == &format!("{resources_prefix}assets/picture.png")));
        assert!(names.iter().any(|name| name == "README.txt"));
        if platform.starts_with("macos-") {
            assert!(!names.iter().any(|name| name.starts_with("webview2/")));
            assert!(!names.iter().any(|name| name == "START.cmd"));
            assert!(names
                .iter()
                .any(|name| name == "HmiRunner.app/Contents/Resources/template-marker.txt"));
            let link = archive
                .by_name("HmiRunner.app/Contents/Resources/template-marker-link.txt")
                .unwrap();
            assert_eq!(link.unix_mode().unwrap() & 0o170000, 0o120000);
            drop(link);
        } else {
            assert!(names
                .iter()
                .any(|name| name == "webview2/msedgewebview2.exe"));
            assert!(names.iter().any(|name| name == "webview2/msedge.dll"));
            assert!(names.iter().any(|name| name == "START.cmd"));
        }
        let manifest: Value =
            serde_json::from_reader(archive.by_name("release-manifest.json").unwrap()).unwrap();
        assert_eq!(manifest["platform"], platform);
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
                expected.as_str().unwrap(),
                "{path}"
            );
        }
        let exported_project: Value = serde_json::from_reader(
            archive
                .by_name(&format!("{resources_prefix}project.json"))
                .unwrap(),
        )
        .unwrap();
        assert!(exported_project.get("python").is_none());
        drop(archive);
        let original = sha256(&destination).unwrap();
        assert!(package(&resources, &project, root, &destination).is_err());
        assert_eq!(sha256(&destination).unwrap(), original);

        let mut python_project = project.clone();
        python_project.flows = serde_json::from_value(json!([{
            "id":"python_flow","name":"Python","enabled":false,"timeoutMs":30000,
            "nodes":[{"id":"python_node","kind":"python","label":"Python","position":{"x":0,"y":0},"config":{"code":"def transform(msg):\n return msg"}}],"edges":[]
        }])).unwrap();
        let missing_python = root.join("missing-python.zip");
        assert!(package(&resources, &python_project, root, &missing_python).is_err());
        assert!(!missing_python.exists());
        #[cfg(windows)]
        {
            fs::write(resources.webview().join("icudtl.dat"), "tampered").unwrap();
            let failed = root.join("failed.zip");
            assert!(package(&resources, &project, root, &failed).is_err());
            assert!(!failed.exists());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_export_rejects_template_runtime_contents_and_foreign_python_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let resources = resources_fixture(root);
        let project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"test","name":"test","width":1280,"height":720,"pages":[]
        }))
        .unwrap();
        let stale = resources
            .template()
            .join("HmiRunner.app/Contents/Resources/python");
        fs::create_dir_all(&stale).unwrap();
        fs::write(stale.join("stale.py"), "must be rejected").unwrap();
        let stale_export = root.join("stale.zip");
        assert!(package(&resources, &project, root, &stale_export).is_err());
        assert!(!stale_export.exists());
        fs::remove_dir_all(stale).unwrap();

        let mut mismatched = project;
        mismatched.flows = serde_json::from_value(json!([{
            "id":"python_flow","name":"Python","enabled":false,"timeoutMs":30000,
            "nodes":[{"id":"python_node","kind":"python","label":"Python","position":{"x":0,"y":0},"config":{}}],"edges":[]
        }])).unwrap();
        mismatched.python = Some(hmi_core::PythonConfig {
            runtime_id: "3.14.8-win_amd64".into(),
            runtime_sha256: None,
            dependency_lock: vec![],
        });
        let mismatch_export = root.join("mismatch.zip");
        assert!(package(&resources, &mismatched, root, &mismatch_export).is_err());
        assert!(!mismatch_export.exists());
    }
}
