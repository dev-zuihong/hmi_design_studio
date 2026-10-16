use hmi_core::Project;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};
use zip::{write::FileOptions, CompressionMethod, ZipArchive, ZipWriter};

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageManifest {
    pub format: String,
    pub format_version: u32,
    pub project_schema_version: u32,
    pub files: BTreeMap<String, PackageFile>,
    #[serde(default)]
    pub missing_dependencies: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageFile {
    pub sha256: String,
    pub size: u64,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unix_mode: Option<u32>,
}

fn safe_relative(path: &str) -> Result<PathBuf> {
    if path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
        || path.starts_with('/')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("工程包包含不安全的文件路径".into());
    }
    let invalid_windows_segment = path.split('/').any(|part| {
        part.ends_with(' ')
            || part.ends_with('.')
            || part.chars().any(|character| {
                character.is_control() || ['<', '>', '"', '|', '?', '*'].contains(&character)
            })
            || {
                let basename = part.split('.').next().unwrap_or("").to_ascii_uppercase();
                matches!(basename.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                    || (1..=9).any(|number| {
                        basename == format!("COM{number}") || basename == format!("LPT{number}")
                    })
            }
    });
    if invalid_windows_segment {
        return Err("工程包路径不兼容 Windows 文件系统".into());
    }
    let path = Path::new(path);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("工程包包含不安全的文件路径".into());
    }
    Ok(path.to_path_buf())
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            if ty.is_symlink() {
                return Err(format!(
                    "工程资源不允许符号链接: {}",
                    entry.path().display()
                ));
            }
            if ty.is_dir() {
                walk(root, &entry.path(), out)?;
            } else if ty.is_file() {
                out.push(
                    entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|e| e.to_string())?
                        .to_path_buf(),
                );
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out)?;
    }
    out.sort();
    Ok(out)
}

fn file_record(bytes: &[u8], mode: &str, unix_mode: Option<u32>) -> PackageFile {
    PackageFile {
        sha256: digest(bytes),
        size: bytes.len() as u64,
        mode: mode.into(),
        unix_mode,
    }
}

/// Writes a self-contained ZIP package using project-relative archive entry names.
#[cfg(test)]
pub fn write_package(
    path: &Path,
    project: &Project,
    resource_root: &Path,
) -> Result<PackageManifest> {
    write_package_with_missing(path, project, resource_root, Vec::new())
}

pub fn write_package_with_missing(
    path: &Path,
    project: &Project,
    resource_root: &Path,
    missing_dependencies: Vec<String>,
) -> Result<PackageManifest> {
    if path.extension().and_then(|v| v.to_str()) != Some("hmiproj") {
        return Err("工程包扩展名必须为 .hmiproj".into());
    }
    let mut resources = collect_files(resource_root)?;
    let mut translations = BTreeMap::<PathBuf, Vec<u8>>::new();
    if let Some(i18n) = &project.i18n {
        for (locale, messages) in &i18n.messages {
            let relative = safe_relative(&format!("i18n/{locale}.json"))?;
            translations.insert(
                relative,
                serde_json::to_vec_pretty(messages).map_err(|e| e.to_string())?,
            );
        }
    }
    resources.retain(|relative| {
        let path = relative.to_string_lossy().replace('\\', "/");
        !(path.starts_with("i18n/") && path.ends_with(".json"))
    });
    resources.extend(translations.keys().cloned());
    resources.sort();
    resources.dedup();
    let resource_paths: HashSet<String> = resources
        .iter()
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    for asset in &project.assets {
        let relative = safe_relative(&asset.path)?;
        let path = relative.to_string_lossy().replace('\\', "/");
        if !hmi_core::safe_identifier(&asset.id) || !resource_paths.contains(&path) {
            return Err(format!("工程资源缺失或引用路径无效: {}", asset.path));
        }
    }
    let parent = path.parent().ok_or("工程包目标目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    let mut records = BTreeMap::new();
    {
        let mut zip = ZipWriter::new(temp.reopen().map_err(|e| e.to_string())?);
        let options = FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);
        let project_bytes = serde_json::to_vec_pretty(project).map_err(|e| e.to_string())?;
        records.insert(
            "project.json".into(),
            file_record(&project_bytes, "file", None),
        );
        zip.start_file("project.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(&project_bytes).map_err(|e| e.to_string())?;
        for relative in resources {
            let archive_path = relative.to_string_lossy().replace('\\', "/");
            let safe = safe_relative(&archive_path)?;
            if archive_path == "manifest.json" || archive_path == "project.json" {
                return Err(format!("工程资源占用了保留路径: {archive_path}"));
            }
            let source = resource_root.join(&relative);
            let bytes = if let Some(bytes) = translations.get(&relative) {
                bytes.clone()
            } else {
                fs::read(&source)
                    .map_err(|e| format!("读取工程资源失败 {}: {e}", source.display()))?
            };
            let unix_mode = if translations.contains_key(&relative) {
                None
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    Some(
                        fs::metadata(&source)
                            .map_err(|e| e.to_string())?
                            .permissions()
                            .mode()
                            & 0o777,
                    )
                }
                #[cfg(not(unix))]
                {
                    None
                }
            };
            let executable = unix_mode.is_some_and(|m| m & 0o111 != 0);
            records.insert(
                archive_path.clone(),
                file_record(
                    &bytes,
                    if executable { "executable" } else { "file" },
                    unix_mode,
                ),
            );
            let mut entry_options = options;
            #[cfg(unix)]
            if let Some(mode) = unix_mode {
                entry_options = entry_options.unix_permissions(mode);
            }
            zip.start_file(safe.to_string_lossy(), entry_options)
                .map_err(|e| e.to_string())?;
            zip.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        let manifest = PackageManifest {
            format: "hmi-project".into(),
            format_version: 1,
            project_schema_version: project.schema_version,
            files: records.clone(),
            missing_dependencies: missing_dependencies.clone(),
        };
        zip.start_file("manifest.json", options)
            .map_err(|e| e.to_string())?;
        zip.write_all(&serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        zip.finish()
            .map_err(|e| e.to_string())?
            .sync_all()
            .map_err(|e| e.to_string())?;
    }
    temp.persist(path).map_err(|e| e.error.to_string())?;
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
    Ok(PackageManifest {
        format: "hmi-project".into(),
        format_version: 1,
        project_schema_version: project.schema_version,
        files: records,
        missing_dependencies,
    })
}

/// Verifies every file before extracting into a fresh, private workspace directory.
pub fn read_package(path: &Path, workspace_root: &Path) -> Result<(Project, PackageManifest)> {
    let mut zip = ZipArchive::new(File::open(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("无效的 hmiproj 工程包: {e}"))?;
    if zip.len() > 100_000 {
        return Err("工程包文件数量超限".into());
    }
    let (manifest, manifest_size): (PackageManifest, u64) = {
        let mut entry = zip
            .by_name("manifest.json")
            .map_err(|_| "工程包缺少 manifest.json")?;
        if entry.size() > 16 * 1024 * 1024 {
            return Err("工程包清单大小超限".into());
        }
        let size = entry.size();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        (
            serde_json::from_slice(&bytes).map_err(|e| format!("工程包清单格式无效: {e}"))?,
            size,
        )
    };
    if manifest.format != "hmi-project" || manifest.format_version != 1 {
        return Err("不支持此工程包格式".into());
    }
    if manifest.files.len() > 99_999 {
        return Err("工程包文件数量超限".into());
    }
    let mut archive_names = HashSet::new();
    for index in 0..zip.len() {
        let entry = zip.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().to_owned();
        if entry.is_dir() {
            return Err(format!("工程包不能包含未登记的目录项: {name}"));
        }
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            if kind == 0o120000 || kind != 0 && kind != 0o100000 {
                return Err(format!("工程包不能包含符号链接或特殊文件: {name}"));
            }
        }
        if !archive_names.insert(name.clone()) {
            return Err(format!("工程包含重复文件路径: {name}"));
        }
        if name != "manifest.json" && !manifest.files.contains_key(&name) {
            return Err(format!("工程包含未登记文件: {name}"));
        }
    }
    let mut entries = HashSet::new();
    let mut contents = Vec::with_capacity(manifest.files.len());
    let mut expanded = manifest_size;
    for (name, record) in &manifest.files {
        let relative = safe_relative(name)?;
        if !["file", "executable"].contains(&record.mode.as_str())
            || record.unix_mode.is_some_and(|mode| {
                mode > 0o777 || (mode & 0o111 != 0) != (record.mode == "executable")
            })
        {
            return Err(format!("工程包权限声明无效: {name}"));
        }
        if !entries.insert(name.clone()) {
            return Err("工程包清单含重复路径".into());
        }
        let mut entry = zip
            .by_name(name)
            .map_err(|_| format!("工程包缺少文件: {name}"))?;
        if entry.is_dir()
            || entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(format!("工程包文件类型无效: {name}"));
        }
        if let (Some(expected), Some(actual)) = (record.unix_mode, entry.unix_mode()) {
            if actual & 0o777 != expected {
                return Err(format!("工程包权限与清单不一致: {name}"));
            }
        }
        if entry.size() != record.size || entry.size() > 1_073_741_824 {
            return Err(format!("工程包文件大小不符或超过限制: {name}"));
        }
        expanded = expanded
            .checked_add(entry.size())
            .filter(|v| *v <= 4_294_967_296)
            .ok_or("工程包解压总量超过限制")?;
        let mut bytes = Vec::with_capacity(record.size as usize);
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if digest(&bytes) != record.sha256 {
            return Err(format!("工程包文件摘要不符: {name}"));
        }
        contents.push((relative, bytes, record.unix_mode));
    }
    if !entries.contains("project.json") {
        return Err("工程包清单没有 project.json".into());
    }
    let project_bytes = contents
        .iter()
        .find(|(p, _, _)| p == Path::new("project.json"))
        .unwrap()
        .1
        .clone();
    let project: Project =
        serde_json::from_slice(&project_bytes).map_err(|e| format!("工程配置无效: {e}"))?;
    if project.schema_version != manifest.project_schema_version {
        return Err("工程格式版本与清单不一致".into());
    }
    for asset in &project.assets {
        let path = safe_relative(&asset.path)?
            .to_string_lossy()
            .replace('\\', "/");
        if !hmi_core::safe_identifier(&asset.id) || !manifest.files.contains_key(&path) {
            return Err(format!("工程资源引用缺失或无效: {}", asset.path));
        }
    }
    let parent = workspace_root.parent().ok_or("工作目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = tempfile::tempdir_in(parent).map_err(|e| e.to_string())?;
    for (relative, bytes, unix_mode) in contents {
        if relative == Path::new("project.json") {
            continue;
        }
        let destination = stage.path().join(relative);
        fs::create_dir_all(destination.parent().ok_or("工程资源目标路径无效")?)
            .map_err(|e| e.to_string())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|e| e.to_string())?;
        output.write_all(&bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = unix_mode {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode & 0o777))
                .map_err(|e| e.to_string())?;
        }
    }
    let stage_path = stage.keep();
    let backup = workspace_root.with_extension(format!("backup-{}", uuid::Uuid::new_v4()));
    let had_old = workspace_root.exists();
    if had_old {
        fs::rename(workspace_root, &backup).map_err(|e| e.to_string())?;
    }
    if let Err(error) = fs::rename(&stage_path, workspace_root) {
        if had_old {
            let _ = fs::rename(&backup, workspace_root);
        }
        let _ = fs::remove_dir_all(stage_path);
        return Err(error.to_string());
    }
    if had_old {
        let _ = fs::remove_dir_all(backup);
    }
    Ok((project, manifest))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use serde_json::Value;
    #[test]
    fn package_round_trip_keeps_same_plugin_id_for_multiple_targets() {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("work");
        fs::create_dir_all(resources.join("assets")).unwrap();
        fs::write(resources.join("assets/a.bin"), b"asset-data").unwrap();
        fs::create_dir_all(resources.join("i18n")).unwrap();
        fs::write(resources.join("i18n/en-US.json"), b"stale translation").unwrap();
        fs::write(resources.join("i18n/fr-FR.json"), b"removed locale").unwrap();
        for (target, bytes) in [
            ("x86_64-pc-windows-msvc", b"win-plugin".as_slice()),
            ("aarch64-apple-darwin", b"mac-plugin".as_slice()),
        ] {
            let plugin = resources.join("plugins").join(target).join("demo-protocol");
            fs::create_dir_all(&plugin).unwrap();
            fs::write(plugin.join("protocol.bin"), bytes).unwrap();
        }
        for (target, bytes) in [
            ("x86_64-pc-windows-msvc", b"win-python".as_slice()),
            ("aarch64-apple-darwin", b"mac-python".as_slice()),
        ] {
            let runtime = resources.join("python").join(target).join("projects/p");
            fs::create_dir_all(&runtime).unwrap();
            fs::write(runtime.join("environment.bin"), bytes).unwrap();
        }
        let project: Project = serde_json::from_value(json!({"schemaVersion":1,"id":"p","name":"P","width":800,"height":600,"pages":[{"id":"page","name":"Page","widgets":[]}],"assets":[{"id":"a","path":"assets/a.bin"}],"connections":[{"id":"c","name":"Device","port":"COM1","baudRate":9600,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"demo-protocol"}],"i18n":{"defaultLocale":"zh-CN","locales":["zh-CN","en-US"],"messages":{"en-US":{"title":"Fresh translation"},"zh-CN":{}}}})).unwrap();
        let archive = dir.path().join("p.hmiproj");
        let manifest = write_package(&archive, &project, &resources).unwrap();
        assert_eq!(manifest.format, "hmi-project");
        let reopened = dir.path().join("reopened");
        fs::create_dir_all(&reopened).unwrap();
        let (loaded, _) = read_package(&archive, &reopened).unwrap();
        assert_eq!(loaded.id, project.id);
        assert_eq!(
            fs::read(reopened.join("assets/a.bin")).unwrap(),
            b"asset-data"
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(reopened.join("i18n/en-US.json")).unwrap())
                .unwrap(),
            json!({"title":"Fresh translation"})
        );
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(reopened.join("i18n/zh-CN.json")).unwrap())
                .unwrap(),
            json!({})
        );
        assert!(!reopened.join("i18n/fr-FR.json").exists());
        assert_eq!(
            fs::read(reopened.join("plugins/x86_64-pc-windows-msvc/demo-protocol/protocol.bin"))
                .unwrap(),
            b"win-plugin"
        );
        assert_eq!(
            fs::read(reopened.join("plugins/aarch64-apple-darwin/demo-protocol/protocol.bin"))
                .unwrap(),
            b"mac-plugin"
        );
        assert_eq!(
            fs::read(reopened.join("python/x86_64-pc-windows-msvc/projects/p/environment.bin"))
                .unwrap(),
            b"win-python"
        );
        assert_eq!(
            fs::read(reopened.join("python/aarch64-apple-darwin/projects/p/environment.bin"))
                .unwrap(),
            b"mac-python"
        );
    }
    #[test]
    fn rejects_traversal() {
        assert!(safe_relative("../escape").is_err());
        assert!(safe_relative("/absolute").is_err());
        assert!(safe_relative("C:/escape").is_err());
        assert!(safe_relative("assets\\escape.png").is_err());
        assert!(safe_relative("assets/./escape.png").is_err());
        assert!(safe_relative("assets/\0escape.png").is_err());
        assert!(safe_relative("assets/CON.txt").is_err());
        assert!(safe_relative("assets/trailing.").is_err());
    }

    #[test]
    fn refuses_to_save_project_with_missing_asset_reference() {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("work");
        fs::create_dir_all(&resources).unwrap();
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"p","name":"P","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[]}],
            "assets":[{"id":"missing","path":"assets/missing.png"}]
        }))
        .unwrap();
        let archive = dir.path().join("missing.hmiproj");
        assert!(write_package(&archive, &project, &resources).is_err());
        assert!(!archive.exists());
    }

    #[test]
    fn emit_example_cross_language_fixture_when_requested() {
        let Some(output) = std::env::var_os("HMI_CROSS_PACKAGE_OUT").map(PathBuf::from) else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("resources");
        let asset_path = resources.join("assets/tiny.png");
        fs::create_dir_all(asset_path.parent().unwrap()).unwrap();
        fs::write(
            &asset_path,
            [
                0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
                b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08,
                0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
                0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, b'B', 0x60, 0x82,
            ],
        )
        .unwrap();
        let mut project: Project =
            serde_json::from_str(include_str!("../../../resources/example/project.json")).unwrap();
        project.schema_version = 2;
        project.assets = vec![hmi_core::Asset {
            id: "tiny-fixture".into(),
            path: "assets/tiny.png".into(),
        }];
        project.i18n = Some(hmi_core::I18nConfig {
            default_locale: "zh-CN".into(),
            locales: vec!["zh-CN".into(), "en-US".into()],
            messages: BTreeMap::from([
                (
                    "zh-CN".into(),
                    BTreeMap::from([("fixture.title".into(), "互通测试".into())]),
                ),
                (
                    "en-US".into(),
                    BTreeMap::from([("fixture.title".into(), "Interop fixture".into())]),
                ),
            ]),
        });
        project.actions = Vec::new();
        for page in &mut project.pages {
            page.layout = Some(hmi_core::LayoutSpec {
                kind: "absolute".into(),
                gap: None,
                padding: None,
                columns: None,
            });
            page.responsive = Some(json!(false));
        }
        let foreign = resources.join("plugins/x86_64-pc-windows-msvc/demo-temperature/fixture.bin");
        fs::create_dir_all(foreign.parent().unwrap()).unwrap();
        fs::write(foreign, b"opaque foreign plugin variant").unwrap();
        let python = resources.join(format!(
            "python/x86_64-pc-windows-msvc/projects/{}/fixture.bin",
            project.id
        ));
        fs::create_dir_all(python.parent().unwrap()).unwrap();
        fs::write(python, b"opaque foreign python variant").unwrap();
        write_package(&output, &project, &resources).unwrap();
    }

    #[test]
    fn verify_browser_package_and_reexport_when_requested() {
        let Some(input) = std::env::var_os("HMI_BROWSER_PACKAGE_IN").map(PathBuf::from) else {
            return;
        };
        let output = std::env::var_os("HMI_BROWSER_PACKAGE_OUT").map(PathBuf::from);
        let dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().join("workspace");
        let (project, manifest) = read_package(&input, &workspace).unwrap();
        assert_eq!(project.schema_version, 2);
        assert_eq!(project.id, "temperature-demo");
        assert_eq!(
            fs::read(workspace.join("assets/tiny.png")).unwrap(),
            [
                0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
                b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08,
                0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
                0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, b'B', 0x60, 0x82,
            ]
        );
        assert_eq!(
            fs::read(workspace.join("plugins/x86_64-pc-windows-msvc/demo-temperature/fixture.bin"))
                .unwrap(),
            b"opaque foreign plugin variant"
        );
        assert_eq!(
            fs::read(workspace.join(format!(
                "python/x86_64-pc-windows-msvc/projects/{}/fixture.bin",
                project.id
            )))
            .unwrap(),
            b"opaque foreign python variant"
        );
        assert_eq!(
            fs::read(workspace.join("i18n/en-US.json")).unwrap(),
            serde_json::to_vec_pretty(&project.i18n.as_ref().unwrap().messages["en-US"]).unwrap()
        );
        if let Some(output) = output {
            write_package_with_missing(
                &output,
                &project,
                &workspace,
                manifest.missing_dependencies,
            )
            .unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "requires the locally imported host plugin and Python runtime; used for CLI export acceptance"]
    async fn emit_actual_resource_package_when_requested() {
        let Some(output) = std::env::var_os("HMI_ACTUAL_PACKAGE_OUT").map(PathBuf::from) else {
            return;
        };
        let shared_root = std::env::var_os("HMI_PACKAGE_RESOURCES")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(".cache"));
        let target = hmi_core::host_target();
        let runtime_id = hmi_python::runtime_id();
        let targeted_runtime_root = shared_root.join("python").join(target).join(&runtime_id);
        let runtime_root = if targeted_runtime_root.is_dir() {
            targeted_runtime_root
        } else {
            shared_root.join("python").join(&runtime_id)
        };
        assert!(
            runtime_root.join("runtime-manifest.json").is_file(),
            "host Python runtime cache is missing"
        );
        let dependencies =
            crate::resources::read_json(&runtime_root.join("dependencies.lock.json")).unwrap();
        let mut project: Project =
            serde_json::from_str(include_str!("../../../resources/example/project.json")).unwrap();
        project.flows.push(hmi_core::Flow {
            id: "package_fixture_python".into(),
            name: "Package fixture Python".into(),
            enabled: false,
            timeout_ms: 10_000,
            nodes: vec![hmi_core::FlowNode {
                id: "package_fixture_python_node".into(),
                kind: "python".into(),
                label: "Python".into(),
                position: hmi_core::Position { x: 10.0, y: 10.0 },
                config: BTreeMap::from([
                    ("code".into(), json!("def transform(msg): return msg")),
                    ("timeoutMs".into(), json!(5_000)),
                    ("memoryMb".into(), json!(256)),
                ]),
            }],
            edges: Vec::new(),
        });
        project.python = Some(hmi_core::PythonConfig {
            runtime_id: runtime_id.into(),
            runtime_sha256: Some(hmi_python::archive_sha256().into()),
            dependency_lock: dependencies["packages"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        });
        let workspace = tempfile::tempdir().unwrap();
        let store = crate::workspace::WorkspaceStore::with_shared_resources(
            project,
            None,
            workspace.path().join("work"),
            shared_root.clone(),
        )
        .unwrap();
        let resources_root = store.root();
        let asset = resources_root.join("assets/tiny.png");
        fs::create_dir_all(asset.parent().unwrap()).unwrap();
        fs::write(
            &asset,
            [
                0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
                b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
                0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0d, b'I', b'D', b'A', b'T', 0x08,
                0xd7, 0x63, 0xf8, 0xcf, 0xc0, 0xf0, 0x1f, 0x00, 0x05, 0x00, 0x01, 0xff, 0x89, 0x99,
                0x3d, 0x1d, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xae, b'B', 0x60, 0x82,
            ],
        )
        .unwrap();
        let mut project = store.snapshot().await;
        project.assets.push(hmi_core::Asset {
            id: "tiny-fixture".into(),
            path: "assets/tiny.png".into(),
        });
        project
            .i18n
            .as_mut()
            .unwrap()
            .messages
            .get_mut("en-US")
            .unwrap()
            .insert("fixture.title".into(), "Real resource package".into());
        let _ = crate::session::validate(
            &crate::resources::Resources {
                root: resources_root.clone(),
                runner: false,
            },
            &project,
        )
        .await
        .into_iter()
        .find(|issue| issue.severity == "error")
        .map_or((), |issue| panic!("{}: {}", issue.path, issue.message));
        write_package_with_missing(&output, &project, &resources_root, Vec::new()).unwrap();
    }
}
