//! Runtime file imports are immutable snapshots; flows carry IDs, never firmware bytes.
use crate::{resources::*, session, DesktopState};
use hmi_core::backend::BackendResource;
use serde_json::{json, Value};
use std::{path::Path, sync::Arc};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

async fn import_one(session: &Arc<session::Session>, path: &Path) -> Result<Value> {
    let source = path.canonicalize().map_err(error)?;
    let metadata = std::fs::metadata(&source).map_err(error)?;
    if !metadata.is_file() {
        return Err("请选择文件".into());
    }
    if metadata.len() > 512 * 1024 * 1024 {
        return Err("单个运行文件不能超过 512 MiB".into());
    }
    let digest = sha256(&source)?;
    let name = source
        .file_name()
        .ok_or("文件名无效")?
        .to_string_lossy()
        .into_owned();
    use sha2::{Digest, Sha256};
    let name_hash = format!("{:x}", Sha256::digest(name.as_bytes()));
    let id = format!("file_{}_{}", &digest[..20], &name_hash[..8]);
    let folder = session::user_directory(&session.runtime.project().id)?
        .join("files")
        .join(&digest);
    std::fs::create_dir_all(&folder).map_err(error)?;
    let destination = folder.join(&name);
    if !destination.is_file() || sha256(&destination)? != digest {
        let staged = tempfile::NamedTempFile::new_in(&folder).map_err(error)?;
        std::fs::copy(&source, staged.path()).map_err(error)?;
        if sha256(staged.path())? != digest {
            return Err("读取期间文件发生变化，请重新选择".into());
        }
        staged.persist(&destination).map_err(error)?;
    }
    session
        .backend_manager()
        .await
        .register_resource(
            id.clone(),
            BackendResource {
                path: destination,
                sha256: Some(digest.clone()),
                size: Some(metadata.len()),
                name: Some(name.clone()),
            },
        )
        .map_err(error)?;
    Ok(
        json!({"id":id,"name":name,"size":metadata.len(),"sha256":digest,"path":source.to_string_lossy(),"status":"ready"}),
    )
}

pub async fn import(session: &Arc<session::Session>, path: &Path) -> Result<Value> {
    let mut reference = import_one(session, path).await?;
    if path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("ptbcfg"))
    {
        if reference["size"].as_u64().unwrap_or(0) > 1024 * 1024 {
            return Err("配置文件不能超过 1 MiB".into());
        }
        let snapshot = session
            .backend_manager()
            .await
            .resource(reference["id"].as_str().unwrap())
            .map_err(error)?;
        let document = read_json(&snapshot.path)?;
        let mut files = Vec::new();
        if let Some(firmware) = document
            .get("firmware")
            .or_else(|| document.get("Firmware"))
            .and_then(Value::as_array)
        {
            files.extend(firmware.iter());
        }
        if let Some(firmware) = document
            .get("dutFirmware")
            .or_else(|| document.get("DutFirmware"))
        {
            files.push(firmware);
        }
        let mut references = serde_json::Map::new();
        let mut missing = Vec::new();
        for file in files {
            let Some(name) = file
                .get("filePath")
                .or_else(|| file.get("FilePath"))
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            if references.contains_key(name) {
                continue;
            }
            let selected = path.parent().unwrap_or(Path::new(".")).join(name);
            match import_one(session, &selected).await {
                Ok(resource) => {
                    references.insert(name.into(), resource);
                }
                Err(reason) => {
                    missing.push(json!({"path":name,"message":reason}));
                }
            }
        }
        reference["references"] = Value::Object(references);
        reference["missingResources"] = json!(missing);
    }
    Ok(reference)
}

#[tauri::command]
pub async fn runtime_import_file(
    path: String,
    state: tauri::State<'_, DesktopState>,
) -> Result<Value> {
    import(&state.session_for(None).await?, Path::new(&path)).await
}

pub async fn action(
    app: &tauri::AppHandle,
    session: &Arc<session::Session>,
    kind: &str,
    args: Value,
) -> Result<Value> {
    if kind == "file.open" {
        let app = app.clone();
        let extensions: Vec<String> = args["extensions"]
            .as_array()
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let selected = tokio::task::spawn_blocking(move || {
            let mut dialog = app.dialog().file();
            if !extensions.is_empty() {
                dialog = dialog.add_filter(
                    "文件",
                    &extensions.iter().map(String::as_str).collect::<Vec<_>>(),
                );
            }
            dialog
                .blocking_pick_file()
                .map(|path| path.into_path().map_err(error))
                .transpose()
        })
        .await
        .map_err(error)??;
        return match selected {
            Some(path) => import(session, &path).await,
            None => Ok(Value::Null),
        };
    }
    if kind == "file.save" {
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or("保存文件需要 content 文本")?
            .to_owned();
        let name = args["name"].as_str().unwrap_or("export.txt").to_owned();
        let app = app.clone();
        let selected = tokio::task::spawn_blocking(move || {
            app.dialog()
                .file()
                .set_file_name(name)
                .blocking_save_file()
                .map(|path| path.into_path().map_err(error))
                .transpose()
        })
        .await
        .map_err(error)??;
        let Some(path) = selected else {
            return Ok(Value::Null);
        };
        return save_document(session, &path, &content, &args).await;
    }
    Err(format!("不支持的文件操作: {kind}"))
}

async fn save_document(
    session: &Arc<session::Session>,
    path: &Path,
    content: &str,
    args: &Value,
) -> Result<Value> {
    let parent = path.parent().ok_or("保存路径无效")?;
    // Copy only resources previously registered with this session. Refuse
    // conflicting companions instead of overwriting a different firmware.
    if let Some(companions) = args.get("resources").and_then(Value::as_array) {
        let manager = session.backend_manager().await;
        let mut copies = Vec::new();
        for companion in companions {
            let id = companion["id"].as_str().ok_or("缺少资源 ID")?;
            let relative = crate::project_package::safe_relative(
                companion["relativePath"]
                    .as_str()
                    .ok_or("缺少伴随文件路径")?,
            )?;
            let resource = manager.resource(id).map_err(error)?;
            let destination = parent.join(relative);
            if destination == path {
                return Err("伴随文件与配置文件同名".into());
            }
            let digest = sha256(&resource.path)?;
            if resource
                .sha256
                .as_ref()
                .is_some_and(|expected| expected != &digest)
            {
                return Err("运行文件快照已改变，请重新选择文件".into());
            }
            if destination.exists() && sha256(&destination)? != digest {
                return Err(format!(
                    "伴随文件已存在且内容不同: {}",
                    destination.display()
                ));
            }
            if copies
                .iter()
                .any(|(_, existing, _)| existing == &destination)
            {
                return Err("伴随文件路径重复".into());
            }
            copies.push((resource.path, destination, digest));
        }
        for (source, destination, digest) in copies {
            if !destination.exists() {
                std::fs::create_dir_all(destination.parent().unwrap()).map_err(error)?;
                std::fs::copy(&source, &destination).map_err(error)?;
                if sha256(&destination)? != digest {
                    return Err("伴随文件复制校验失败".into());
                }
            }
        }
    }
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(error)?;
    use std::io::Write;
    temp.write_all(content.as_bytes()).map_err(error)?;
    temp.persist(path).map_err(error)?;
    Ok(json!({"path":path.to_string_lossy(),"size":content.len()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn configuration_companions_roundtrip_preserves_snapshots_and_refuses_conflicts() {
        let directory = tempfile::tempdir().unwrap();
        let mut project: hmi_core::Project =
            serde_json::from_str(include_str!("../../../examples/ing-ptb/project.json")).unwrap();
        project.id = format!(
            "ptb-files-test-{}",
            directory.path().file_name().unwrap().to_string_lossy()
        );
        project.backends.clear();
        project.flows.clear();
        project.assets.clear();
        project.actions.clear();
        project.windows.clear();
        for page in &mut project.pages {
            page.widgets.clear();
        }
        let session = session::Session::new(
            project,
            Resources {
                root: directory.path().to_path_buf(),
                runner: false,
            },
        )
        .await
        .unwrap();
        let source = directory.path().join("firmware 测试.bin");
        std::fs::write(&source, [1, 2, 3, 4]).unwrap();
        let configuration = directory.path().join("source.ptbcfg");
        let document = json!({"Firmware":[{"FilePath":"firmware 测试.bin"}],"DutFirmware":{"FilePath":"missing.bin"}});
        std::fs::write(&configuration, document.to_string()).unwrap();
        let reference = import(&session, &configuration).await.unwrap();
        assert_eq!(reference["missingResources"][0]["path"], "missing.bin");
        let firmware = &reference["references"]["firmware 测试.bin"];
        let manager = session.backend_manager().await;
        let snapshot = manager.resource(firmware["id"].as_str().unwrap()).unwrap();
        std::fs::write(&source, [9, 9]).unwrap();
        assert_eq!(std::fs::read(&snapshot.path).unwrap(), [1, 2, 3, 4]);

        let export = directory.path().join("export");
        std::fs::create_dir(&export).unwrap();
        let destination = export.join("roundtrip.ptbcfg");
        let content = json!({"firmware":[{"filePath":"firmware/part.bin"}]}).to_string();
        let args = json!({"resources":[{"id":firmware["id"],"relativePath":"firmware/part.bin"}]});
        save_document(&session, &destination, &content, &args)
            .await
            .unwrap();
        let reopened = import(&session, &destination).await.unwrap();
        assert_eq!(reopened["missingResources"], json!([]));
        assert_eq!(
            reopened["references"]["firmware/part.bin"]["sha256"],
            firmware["sha256"]
        );
        std::fs::write(export.join("firmware/part.bin"), [8]).unwrap();
        assert!(save_document(&session, &destination, "changed", &args)
            .await
            .unwrap_err()
            .contains("内容不同"));
        assert_eq!(std::fs::read_to_string(&destination).unwrap(), content);
        let unsafe_args =
            json!({"resources":[{"id":firmware["id"],"relativePath":"../escape.bin"}]});
        assert!(
            save_document(&session, &destination, "changed", &unsafe_args)
                .await
                .is_err()
        );
        assert!(!directory.path().join("escape.bin").exists());
        session.stop().await;
        let cache = session::user_directory(&session.runtime.project().id).unwrap();
        assert!(cache
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("ptb-files-test-"));
        std::fs::remove_dir_all(cache).unwrap();
    }
}

pub async fn attach(app: &tauri::AppHandle, session: &Arc<session::Session>) {
    let mut receiver = session.attach_host().await;
    let app = app.clone();
    let weak = Arc::downgrade(session);
    let cancel = session.cancel.clone();
    let task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                request = receiver.recv() => {
                    let Some(request) = request else { break };
                    if request.reply.is_closed() { continue; }
                    let Some(session) = weak.upgrade() else { break };
                    let dispatch = async {
                        if request.kind.starts_with("window.") {
                            let state = app.state::<DesktopState>();
                            let mut args = request.args;
                            if let Some(backend_id) = args["backendId"].as_str().or_else(||request.context["backendId"].as_str()) {
                                let snapshot = session.runtime.snapshot().await;
                                let backend = snapshot.backends.get(backend_id).ok_or("后端实例已移除")?;
                                if request.context.get("generation").is_some_and(|generation| generation != &backend["generation"]) {
                                    return Err("设备窗口属于旧会话，请重新打开".into());
                                }
                                args["generation"] = backend["generation"].clone();
                            }
                            state.runtime_windows.action(&app, &session.runtime.project(), &request.kind, args, request.context).await
                        } else { action(&app, &session, &request.kind, request.args).await }
                    };
                    let result = tokio::select! { _ = cancel.cancelled() => break, result = dispatch => result };
                    let _ = request.reply.send(result.map_err(|message|hmi_core::CoreError::new("HOST_ACTION", message)));
                }
            }
        }
    });
    session.add_event_forwarder(task).await;
}
