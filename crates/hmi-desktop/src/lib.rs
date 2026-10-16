mod export;
mod flow_services;
mod preview_host;
mod project_package;
mod resources;
mod runtime_files;
mod runtime_log;
mod runtime_windows;
mod session;
mod startup;
pub mod tooling;
mod usb_printer;
mod window_settings;
mod workspace;

use hmi_core::{Project, RuntimeSnapshot, ValidationIssue};
use resources::*;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, PartialEq)]
pub enum AppMode {
    Studio,
    Runner,
}
pub struct DesktopState {
    mode: AppMode,
    runtime_windows: runtime_windows::RuntimeWindows,
    resources: Resources,
    project_path: Mutex<Option<PathBuf>>,
    current_project: Mutex<Option<Project>>,
    session: Mutex<Option<Arc<session::Session>>>,
    preview_process: Mutex<Option<Arc<preview_host::PreviewHostProcess>>>,
    preview_peer: Mutex<Option<Arc<preview_host::RpcPeer>>>,
    preview_locale: Mutex<Option<String>>,
    preview_host_mode: bool,
    workspace: Arc<workspace::WorkspaceStore>,
    preview_project: Mutex<Option<Project>>,
    preview_page: Arc<Mutex<Option<String>>>,
    preview_generation: Arc<AtomicU64>,
    preview_revision: AtomicU64,
    pending_preview_update: Mutex<Option<(u64, Project, Option<Resources>)>>,
    preview_update_worker: AtomicBool,
    preview_update_cancel: Mutex<CancellationToken>,
    preview_update_lock: Mutex<()>,
    preview_update_status: Mutex<String>,
    preview_closing: AtomicU64,
    main_close_pending: AtomicBool,
    transition: Mutex<()>,
    shutting_down: AtomicBool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewState {
    project: Option<Project>,
    running: bool,
    current_page: Option<String>,
    generation: u64,
    snapshot: RuntimeSnapshot,
    #[serde(default)]
    locale: Option<String>,
    revision: u64,
    pending_revision: Option<u64>,
    update_status: String,
    pid: Option<u32>,
}
impl DesktopState {
    fn workspace_resources(&self) -> Resources {
        Resources {
            root: self.workspace.root(),
            runner: false,
        }
    }

    async fn preview_rpc(&self, method: &str, payload: Value) -> Result<Value> {
        let mut payload = payload;
        let peer = if self.mode == AppMode::Studio {
            if let Some(expected) = payload.get("expectedGeneration").and_then(Value::as_u64) {
                if expected != self.preview_generation.load(Ordering::Acquire) {
                    return Err("预览会话已切换，请重新连接预览".into());
                }
            }
            if let Some(object) = payload.as_object_mut() {
                object.remove("expectedGeneration");
            }
            self.preview_process
                .lock()
                .await
                .as_ref()
                .map(|host| host.peer.clone())
        } else if self.preview_host_mode {
            self.preview_peer.lock().await.clone()
        } else {
            None
        }
        .ok_or("预览 host 未运行")?;
        peer.request(method, payload).await
    }

    async fn session_for(&self, expected_generation: Option<u64>) -> Result<Arc<session::Session>> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("应用正在退出".into());
        }
        let session = self.session.lock().await;
        if let Some(expected) = expected_generation {
            if (self.mode != AppMode::Studio && !self.preview_host_mode)
                || self.preview_generation.load(Ordering::Acquire) != expected
            {
                return Err("预览会话已切换，请重新连接预览".into());
            }
        }
        let active = session
            .clone()
            .ok_or_else(|| "请先启动预览或运行会话".to_string())?;
        if let Some(expected) = expected_generation {
            if self.preview_generation.load(Ordering::Acquire) != expected {
                return Err("预览会话已切换，请重新连接预览".into());
            }
        }
        Ok(active)
    }
    fn studio(&self) -> Result<()> {
        if self.mode == AppMode::Studio {
            Ok(())
        } else {
            Err("此操作仅在设计器可用".into())
        }
    }
}

#[tauri::command]
async fn get_environment(state: State<'_, DesktopState>) -> Result<Value> {
    let project_id = state
        .current_project
        .lock()
        .await
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_default();
    Ok(
        json!({"mode":if state.mode == AppMode::Studio {"studio"} else if state.preview_host_mode {"preview"} else {"runner"},
        "version":env!("CARGO_PKG_VERSION"), "pythonAvailable":state.resources.python(&project_id).join("runtime-manifest.json").is_file(),
        "platform":release_platform(),"target":hmi_core::host_target(),"pythonRuntimeId":hmi_python::runtime_id(),
        "runnerAvailable":state.resources.template().join(runner_relative()).is_file(),
        "webviewAvailable":cfg!(target_os = "macos") || state.resources.webview().join("msedgewebview2.exe").is_file(),
        "projectPath":state.project_path.lock().await.as_ref().map(|p|p.to_string_lossy().into_owned())}),
    )
}
fn load_project(path: &Path) -> Result<Project> {
    serde_json::from_value(read_json(path)?).map_err(error)
}

#[tauri::command]
async fn get_initial_project(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<Project> {
    if let Some(project) = state.current_project.lock().await.as_ref() {
        return Ok(project.clone());
    }
    let project = if let Some(path) = state.project_path.lock().await.clone() {
        load_project(&path)?
    } else {
        serde_json::from_str(include_str!("../../../resources/default/project.json"))
            .map_err(error)?
    };
    let mut project = project;
    if state.mode == AppMode::Runner {
        session::apply_settings(&mut project)?;
    }
    if state.mode == AppMode::Runner {
        if let Some(window) = app.get_webview_window("main") {
            window.set_title(&project.name).map_err(error)?;
            // Invalid local preferences do not make an otherwise valid project unusable.
            let _ = window_settings::restore(&window, &project.id);
        }
    }
    *state.current_project.lock().await = Some(project.clone());
    Ok(project)
}
#[tauri::command]
async fn open_project(path: String, state: State<'_, DesktopState>) -> Result<Project> {
    state.studio()?;
    let path = std::fs::canonicalize(path).map_err(error)?;
    let project = load_project(&path)?;
    *state.project_path.lock().await = Some(path);
    *state.current_project.lock().await = Some(project.clone());
    Ok(project)
}
#[tauri::command]
async fn save_project(
    path: String,
    mut project: Project,
    state: State<'_, DesktopState>,
) -> Result<()> {
    state.studio()?;
    if project.schema_version != 1 {
        return Err("不支持此项目格式".into());
    }
    if !hmi_core::safe_identifier(&project.id) {
        return Err("项目 ID 无效".into());
    }
    session::pin_python(&state.resources, &mut project)?;
    session::pin_plugins(&state.resources, &mut project)?;
    let path = PathBuf::from(path);
    let destination_directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(destination_directory).map_err(error)?;
    if let Some(previous_directory) = state
        .project_path
        .lock()
        .await
        .as_ref()
        .and_then(|p| p.parent())
    {
        if previous_directory.canonicalize().map_err(error)?
            != destination_directory.canonicalize().map_err(error)?
        {
            for asset in &project.assets {
                let source = contained_file(previous_directory, &asset.path)?;
                let target = destination_directory.join(resources::safe_relative(&asset.path)?);
                if target.exists() {
                    if sha256(&target)? != sha256(&source)? {
                        return Err(format!(
                            "另存为目标资源已存在且内容不同: {}",
                            target.display()
                        ));
                    }
                } else {
                    std::fs::create_dir_all(target.parent().unwrap()).map_err(error)?;
                    std::fs::copy(source, target).map_err(error)?;
                }
            }
        }
    }
    write_json(&path, &project)?;
    *state.project_path.lock().await = Some(std::fs::canonicalize(path).map_err(error)?);
    *state.current_project.lock().await = Some(project);
    Ok(())
}
#[tauri::command]
async fn validate_project(
    project: Project,
    state: State<'_, DesktopState>,
) -> Result<Vec<ValidationIssue>> {
    let resources = if state.mode == AppMode::Studio {
        state.workspace_resources()
    } else {
        state.resources.clone()
    };
    Ok(session::validate(&resources, &project).await)
}
#[tauri::command]
fn list_serial_ports() -> Result<Value> {
    session::serial_ports()
}
#[tauri::command]
fn list_plugins(state: State<'_, DesktopState>) -> Result<Vec<Value>> {
    state.resources.plugin_manifests()
}
#[tauri::command]
async fn import_plugin(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    let path = PathBuf::from(path);
    let directory = if path.is_file() {
        path.parent().ok_or("无效插件路径")?.to_path_buf()
    } else {
        path
    };
    let manifest = read_json(&directory.join("manifest.json"))?;
    let id = manifest["id"].as_str().ok_or("缺少插件 ID")?;
    if !hmi_core::safe_identifier(id) {
        return Err("插件 ID 无效".into());
    }
    let target = manifest["target"]
        .as_str()
        .unwrap_or(hmi_core::host_target());
    let executable_relative = manifest["executable"]
        .as_str()
        .ok_or("缺少插件可执行文件")?;
    let executable = contained_file(&directory, executable_relative)?;
    resources::check_binary_for_target(&executable, target)?;
    let destination = state.resources.root.join("plugins").join(target).join(id);
    let workspace_destination = state.workspace.root().join("plugins").join(target).join(id);
    if destination.exists() || workspace_destination.exists() {
        return Err("同 ID 插件已存在；请使用新的版本 ID 导入".into());
    }
    let parent = destination.parent().ok_or("插件目录无效")?;
    std::fs::create_dir_all(&parent).map_err(error)?;
    let stage = tempfile::tempdir_in(&parent).map_err(error)?;
    copy_tree(&directory, stage.path())?;
    resources::check_binary_for_target(
        &contained_file(stage.path(), executable_relative)?,
        target,
    )?;
    std::fs::rename(stage.path(), &destination).map_err(error)?;
    let workspace_parent = workspace_destination.parent().ok_or("工程插件目录无效")?;
    std::fs::create_dir_all(workspace_parent).map_err(error)?;
    let workspace_stage = tempfile::tempdir_in(workspace_parent).map_err(error)?;
    copy_tree(&destination, workspace_stage.path())?;
    std::fs::rename(workspace_stage.path(), &workspace_destination).map_err(error)?;
    Ok(manifest)
}
#[tauri::command]
async fn import_python_runtime(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    import_python(path, &state, false).await?;
    let lock = read_json(
        &state
            .resources
            .root
            .join("python")
            .join(hmi_core::host_target())
            .join(hmi_python::runtime_id())
            .join("dependencies.lock.json"),
    )?;
    Ok(
        json!({"runtimeId":hmi_python::runtime_id(),"runtimeSha256":hmi_python::archive_sha256(),"dependencyLock":lock["packages"]}),
    )
}
#[tauri::command]
async fn import_backend(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    let path = PathBuf::from(path);
    let directory = if path.is_file() {
        path.parent().ok_or("无效后端路径")?.to_path_buf()
    } else {
        path
    };
    session::check_backend(&directory)?;
    let manifest = read_json(&directory.join("manifest.json"))?;
    let id = manifest["id"].as_str().ok_or("后端缺少 ID")?;
    if !hmi_core::safe_identifier(id) {
        return Err("后端 ID 无效".into());
    }
    let target = manifest["target"].as_str().ok_or("后端缺少目标平台")?;
    for root in [state.resources.root.clone(), state.workspace.root()] {
        let destination = root.join("backends").join(target).join(id);
        if destination.exists() {
            let current = read_json(&destination.join("manifest.json"))?;
            if current != manifest {
                return Err("同 ID 已存在不同版本后端，请使用新的资源 ID".into());
            }
            continue;
        }
        let parent = destination.parent().ok_or("后端路径无效")?;
        std::fs::create_dir_all(parent).map_err(error)?;
        let stage = tempfile::tempdir_in(parent).map_err(error)?;
        copy_tree(&directory, stage.path())?;
        session::check_backend(stage.path())?;
        std::fs::rename(stage.path(), &destination).map_err(error)?;
    }
    Ok(manifest)
}
#[tauri::command]
async fn import_python_dependencies(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    import_python(path, &state, true).await?;
    let project = state.workspace.snapshot().await;
    let lock = read_json(
        &state
            .workspace
            .root()
            .join("python")
            .join(hmi_core::host_target())
            .join("projects")
            .join(&project.id)
            .join("dependencies.lock.json"),
    )?;
    Ok(
        json!({"runtimeId":hmi_python::runtime_id(),"runtimeSha256":hmi_python::archive_sha256(),"dependencyLock":lock["packages"]}),
    )
}
async fn import_python(path: String, state: &DesktopState, project_specific: bool) -> Result<()> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    let env = hmi_python::PythonEnvironment::load(path).map_err(error)?;
    hmi_python::PythonExecutor::new(env.clone())
        .validate_imports_with_limits(hmi_python::PythonLimits {
            timeout: std::time::Duration::from_secs(15),
            memory_bytes: 512 * 1024 * 1024,
        })
        .await
        .map_err(error)?;
    let destination = if project_specific {
        let project = state.workspace.snapshot().await;
        let id = &project.id;
        if !hmi_core::safe_identifier(id) {
            return Err("项目 ID 无效".into());
        }
        state
            .workspace
            .root()
            .join("python")
            .join(hmi_core::host_target())
            .join("projects")
            .join(id)
    } else {
        state
            .resources
            .root
            .join("python")
            .join(hmi_core::host_target())
            .join(hmi_python::runtime_id())
    };
    if destination.exists() && std::fs::canonicalize(&destination).map_err(error)? == env.root() {
        return Ok(());
    }
    let parent = destination.parent().unwrap();
    std::fs::create_dir_all(parent).map_err(error)?;
    let stage = tempfile::tempdir_in(parent).map_err(error)?;
    copy_tree(env.root(), stage.path())?;
    hmi_python::PythonEnvironment::load(stage.path()).map_err(error)?;
    let backup = parent.join(format!("backup-{}", uuid::Uuid::new_v4()));
    if destination.exists() {
        std::fs::rename(&destination, &backup).map_err(error)?;
    }
    if let Err(e) = std::fs::rename(stage.path(), &destination) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, &destination);
        }
        return Err(error(e));
    }
    // Keep previous imported environments as local backups until explicitly removed.
    Ok(())
}
#[tauri::command]
async fn start_preview(
    project: Project,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<()> {
    if state.mode == AppMode::Studio {
        return open_preview_window(project, app, state).await;
    }
    let _transition = state.transition.lock().await;
    if state.shutting_down.load(Ordering::Acquire) {
        return Err("应用正在退出".into());
    }
    let issues = session::validate(&state.resources, &project).await;
    if let Some(issue) = issues.iter().find(|i| i.severity == "error") {
        return Err(format!("{}: {}", issue.path, issue.message));
    }
    if let Some(previous) = state.session.lock().await.take() {
        previous.stop().await;
    }
    session::save_settings(&project)?;
    let generation = state.preview_generation.load(Ordering::Acquire);
    let runtime = start_runtime_session(&app, &state, &project, generation).await?;
    *state.current_project.lock().await = Some(project);
    *state.session.lock().await = Some(runtime);
    Ok(())
}

#[tauri::command]
async fn open_preview_window(
    project: Project,
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<()> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    if state.shutting_down.load(Ordering::Acquire) {
        return Err("应用正在退出".into());
    }
    if state.preview_closing.load(Ordering::Acquire) != 0 {
        return Err("预览进程正在关闭，请稍后重试".into());
    }
    // Release the process-slot lock before checking/stopping an old host.
    let previous_host = state.preview_process.lock().await.clone();
    if let Some(host) = previous_host {
        if host.is_alive().await && !host.peer.closed_token().is_cancelled() {
            host.request("focus_preview", json!({})).await?;
            return Ok(());
        }
        stop_session_locked(&app, &state).await;
    }
    let issues = session::validate(&state.workspace_resources(), &project).await;
    if let Some(issue) = issues.iter().find(|i| i.severity == "error") {
        return Err(format!("{}: {}", issue.path, issue.message));
    }
    let (workspace_project, revision) = state.workspace.snapshot_with_revision().await;
    let project = if workspace_project.id == project.id {
        workspace_project
    } else {
        project
    };
    let runner = state.resources.template().join(runner_relative());
    check_native_binary(&runner)?;
    let generation = state.preview_generation.fetch_add(1, Ordering::AcqRel) + 1;
    let host = preview_host::PreviewHostProcess::launch(
        &runner,
        &state.resources.root,
        &state.workspace.root(),
        &project,
        revision,
    )
    .await?;
    let mut host_events = host.peer.subscribe();
    let disconnected = host.peer.closed_token();
    let event_app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let (name, mut payload) = tokio::select! {
                biased;
                _ = disconnected.cancelled() => break,
                event = host_events.recv() => match event {
                    Ok(event) => event,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            };
            if name == "runtime-event" {
                if let Some(object) = payload.as_object_mut() {
                    object.insert("generation".into(), json!(generation));
                }
                let _ = event_app.emit("runtime-event", payload);
            } else if name == "preview-state-changed" {
                let _ = event_app.emit(
                    "preview-state-changed",
                    json!({
                        "running":payload["running"],"generation":generation,
                        "revision":payload["revision"],"pendingRevision":payload["pendingRevision"],
                        "updateStatus":payload["updateStatus"],"pid":payload["pid"]
                    }),
                );
            } else if name == "preview-locale-changed" {
                let _ = event_app.emit(
                    "preview-locale-changed",
                    json!({"locale":payload["locale"],"generation":generation}),
                );
            } else if name == "preview-input-changed" {
                let _ = event_app.emit(
                    "preview-input-changed",
                    json!({"id":payload["id"],"value":payload["value"],"generation":generation}),
                );
            } else if name == "preview-update-applied" {
                if let Ok(project) = serde_json::from_value::<Project>(payload["project"].clone()) {
                    let app = event_app.clone();
                    let revision = payload["revision"].as_u64().unwrap_or_default();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<DesktopState>();
                        let _transition = state.transition.lock().await;
                        if state.preview_generation.load(Ordering::Acquire) != generation
                            || revision < state.preview_revision.load(Ordering::Acquire)
                        {
                            return;
                        }
                        *state.preview_project.lock().await = Some(project.clone());
                        *state.current_project.lock().await = Some(project);
                        state.preview_revision.store(revision, Ordering::Release);
                    });
                }
                let _ = event_app.emit(name.as_str(), payload);
            } else if name == "preview-update-failed" {
                let _ = event_app.emit(name.as_str(), payload);
            }
        }
        // A closed window or crashed host must also reset the editor controls.
        // An old host's disconnect must never stop a newer preview generation.
        let state = event_app.state::<DesktopState>();
        let _transition = state.transition.lock().await;
        if state.preview_generation.load(Ordering::Acquire) == generation {
            stop_session_locked(&event_app, &state).await;
        }
    });
    *state.preview_project.lock().await = Some(project);
    *state.preview_process.lock().await = Some(host);
    state.preview_revision.store(revision, Ordering::Release);
    *state.preview_update_status.lock().await = "applied".into();
    emit_preview_state(&app, true, generation);
    Ok(())
}

#[tauri::command]
async fn get_preview_state(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
) -> Result<PreviewState> {
    let _transition = state.transition.lock().await;
    if state.mode == AppMode::Studio {
        let host = state.preview_process.lock().await.clone();
        if let Some(host) = host {
            if host.is_alive().await && !host.peer.closed_token().is_cancelled() {
                match host.request("get_preview_state", json!({})).await {
                    Ok(mut value) => {
                        value["generation"] =
                            json!(state.preview_generation.load(Ordering::Acquire));
                        return serde_json::from_value(value).map_err(error);
                    }
                    Err(problem) => {
                        // Only a confirmed exit/disconnect is a stopped preview;
                        // errors from a live host must still reach the caller.
                        if host.is_alive().await && !host.peer.closed_token().is_cancelled() {
                            return Err(problem);
                        }
                    }
                }
            }
            stop_session_locked(&app, &state).await;
        }
        // The editor queries state before opening its first preview and after
        // stopping it. Absence of a host is a normal lifecycle state.
        return Ok(PreviewState {
            project: None,
            running: false,
            current_page: None,
            generation: state.preview_generation.load(Ordering::Acquire),
            snapshot: RuntimeSnapshot::default(),
            locale: state.preview_locale.lock().await.clone(),
            revision: state.preview_revision.load(Ordering::Acquire),
            pending_revision: None,
            update_status: "applied".into(),
            pid: None,
        });
    }
    if !state.preview_host_mode {
        return Err("当前运行程序没有预览 host".into());
    }
    let project = state.preview_project.lock().await.clone();
    let current_page = state.preview_page.lock().await.clone();
    let generation = state.preview_generation.load(Ordering::Acquire);
    let runtime = state.session.lock().await.clone();
    let running = project.is_some() && runtime.is_some();
    let snapshot = match runtime {
        Some(session) if running => session.runtime.snapshot().await,
        _ => RuntimeSnapshot::default(),
    };
    let pending_revision = state
        .pending_preview_update
        .lock()
        .await
        .as_ref()
        .map(|(revision, _, _)| *revision);
    Ok(PreviewState {
        project,
        running,
        current_page,
        generation,
        snapshot,
        locale: state.preview_locale.lock().await.clone(),
        revision: state.preview_revision.load(Ordering::Acquire),
        pending_revision,
        update_status: state.preview_update_status.lock().await.clone(),
        pid: Some(std::process::id()),
    })
}

async fn start_runtime_session(
    app: &tauri::AppHandle,
    state: &DesktopState,
    project: &Project,
    generation: u64,
) -> Result<Arc<session::Session>> {
    let log_directory = session::user_directory(&project.id)?;
    let runtime = session::Session::new(project.clone(), state.resources.clone()).await?;
    runtime_files::attach(app, &runtime).await;
    let mut events = runtime.runtime.subscribe();
    let cancel = runtime.cancel.clone();
    let page = state.preview_page.clone();
    let generation_source = state.preview_generation.clone();
    let app = app.clone();
    let forwarder = tokio::spawn(async move {
        let mut log = runtime_log::RuntimeLog::new(log_directory.join("runtime.jsonl")).await;
        let mut log_warned = false;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    loop { match events.try_recv() {
                        Ok(event) => forward_runtime_event(&app, &page, &generation_source, generation, &mut log, &mut log_warned, &event).await,
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                            forward_runtime_event(&app, &page, &generation_source, generation, &mut log, &mut log_warned, &hmi_core::RuntimeEvent::new("log", json!({"level":"error","message":format!("运行事件队列溢出，丢失 {count} 条事件")}))).await;
                        },
                        Err(_) => break,
                    }}
                    break;
                },
                event = events.recv() => match event {
                    Ok(event) => forward_runtime_event(&app, &page, &generation_source, generation, &mut log, &mut log_warned, &event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        forward_runtime_event(&app, &page, &generation_source, generation, &mut log, &mut log_warned, &hmi_core::RuntimeEvent::new("log", json!({"level":"error","message":format!("运行事件队列溢出，丢失 {count} 条事件")}))).await;
                    },
                    Err(_) => break,
                }
            }
        }
        if let Ok(log) = &mut log {
            if let Err(error) = log.flush().await {
                let mut payload = serde_json::to_value(hmi_core::RuntimeEvent::new(
                    "log",
                    json!({"level":"error","message":format!("日志保存失败: {error}")}),
                ))
                .unwrap_or_else(|_| json!({}));
                if let Some(payload) = payload.as_object_mut() {
                    payload.insert("generation".into(), json!(generation));
                }
                let _ = app.emit("runtime-event", payload);
            }
        }
    });
    runtime.add_event_forwarder(forwarder).await;
    if let Err(start_error) = runtime.runtime.start().await {
        runtime.stop().await;
        return Err(error(start_error));
    }
    *state.session.lock().await = Some(runtime.clone());
    Ok(runtime)
}

fn emit_preview_state(app: &tauri::AppHandle, running: bool, generation: u64) {
    let payload = json!({"running":running,"generation":generation});
    let _ = app.emit("preview-state-changed", payload);
}

async fn forward_runtime_event(
    app: &tauri::AppHandle,
    preview_page: &Mutex<Option<String>>,
    preview_generation: &AtomicU64,
    generation: u64,
    log: &mut std::io::Result<runtime_log::RuntimeLog>,
    warned: &mut bool,
    event: &hmi_core::RuntimeEvent,
) {
    if event.kind == "page.changed" && preview_generation.load(Ordering::Acquire) == generation {
        if let Some(page_id) = event.data.get("pageId").and_then(Value::as_str) {
            *preview_page.lock().await = Some(page_id.to_owned());
        }
    }
    let mut payload = serde_json::to_value(event).unwrap_or_else(|_| json!({}));
    if let Some(payload) = payload.as_object_mut() {
        payload.insert("generation".into(), json!(generation));
    }
    let _ = app.emit("runtime-event", payload);
    if event.kind == "transport.devices" {
        if preview_generation.load(Ordering::Acquire) == generation {
            app.state::<DesktopState>()
                .runtime_windows
                .reconcile_transport(app, &event.data)
                .await;
        }
    }
    if event.kind == "backend.event" && event.data["kind"] == "snapshot" {
        if preview_generation.load(Ordering::Acquire) == generation {
            let state = app.state::<DesktopState>();
            state
                .runtime_windows
                .reconcile_devices(
                    app,
                    event.data["backendId"].as_str().unwrap_or(""),
                    &event.data["data"],
                )
                .await;
        }
        return;
    }
    let failure = match log {
        Ok(log) => log.write(event).await.err().map(|e| e.to_string()),
        Err(error) => Some(error.to_string()),
    };
    if let Some(error) = failure {
        if !*warned {
            *warned = true;
            let _ = app.emit(
                "runtime-event",
                hmi_core::RuntimeEvent::new(
                    "log",
                    json!({"level":"error","message":format!("日志保存失败: {error}")}),
                ),
            );
        }
    }
}
#[tauri::command]
async fn stop_preview(
    app: tauri::AppHandle,
    state: State<'_, DesktopState>,
    expected_generation: Option<u64>,
) -> Result<()> {
    let _transition = state.transition.lock().await;
    if let Some(expected) = expected_generation {
        if state.mode != AppMode::Studio && !state.preview_host_mode
            || state.preview_generation.load(Ordering::Acquire) != expected
        {
            return Err("预览会话已切换，请重新连接预览".into());
        }
    }
    if state.mode == AppMode::Runner && state.preview_host_mode {
        let generation = state.preview_generation.load(Ordering::Acquire);
        let _ = stop_session_locked(&app, &state).await;
        app.exit(0);
        emit_preview_state(&app, false, generation.saturating_add(1));
        return Ok(());
    }
    let (window, generation) = stop_session_locked(&app, &state).await;
    if let (Some(window), Some(generation)) = (window, generation) {
        if state
            .preview_closing
            .compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            schedule_preview_destroy(app, window, generation);
        }
    }
    Ok(())
}

#[tauri::command]
async fn apply_preview_update(
    app: tauri::AppHandle,
    revision: u64,
    project: Project,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<Value> {
    if state.mode == AppMode::Studio {
        let host = state
            .preview_process
            .lock()
            .await
            .clone()
            .ok_or("预览 host 未运行")?;
        let previous = state.preview_project.lock().await.clone();
        let resource_change = previous
            .as_ref()
            .is_none_or(|old| preview_resources_changed(old, &project));
        let candidate_root = if resource_change {
            Some(host.stage_revision_resources(&state.workspace.root(), &project, revision)?)
        } else {
            None
        };
        let result = state.preview_rpc("apply_preview_update", json!({
            "revision":revision, "project":project, "expectedGeneration":expected_generation,
            "resourceRoot":candidate_root,
        })).await?;
        return Ok(result);
    }
    if !state.preview_host_mode {
        return Err("只有预览 host 支持热更新".into());
    }
    apply_preview_revision(app, &state, revision, project, expected_generation, None).await
}

fn preview_resources_changed(previous: &Project, next: &Project) -> bool {
    let resources = |project: &Project| {
        json!({
            "connections":project.connections,
            "python":project.python,
            "variables":project.variables.iter().map(|v| (&v.id,&v.value_type)).collect::<Vec<_>>(),
            "assets":project.assets,
        })
    };
    resources(previous) != resources(next)
}

async fn apply_preview_revision(
    app: tauri::AppHandle,
    state: &DesktopState,
    revision: u64,
    project: Project,
    expected_generation: Option<u64>,
    candidate_resources: Option<Resources>,
) -> Result<Value> {
    let _revision_guard = state.preview_update_lock.lock().await;
    if expected_generation
        .is_some_and(|expected| expected != state.preview_generation.load(Ordering::Acquire))
    {
        return Err("预览会话已切换，请重新连接预览".into());
    }
    let previous_revision = state.preview_revision.load(Ordering::Acquire);
    if revision <= previous_revision {
        return Ok(json!({"status":"stale","revision":previous_revision,"issues":[]}));
    }
    let active = state
        .session
        .lock()
        .await
        .clone()
        .ok_or("预览运行会话不存在")?;
    let resources = match candidate_resources {
        Some(resources) => resources,
        None => active.current_resources().await,
    };
    let issues = session::validate(&resources, &project).await;
    if issues.iter().any(|issue| issue.severity == "error") {
        if state.pending_preview_update.lock().await.take().is_some() {
            active.runtime.finish_revision_barrier();
        }
        *state.preview_update_status.lock().await = "invalid".into();
        let payload = json!({"running":true,"generation":state.preview_generation.load(Ordering::Acquire),"revision":previous_revision,"updateStatus":"invalid"});
        let _ = app.emit("preview-state-changed", payload.clone());
        if let Some(peer) = state.preview_peer.lock().await.as_ref() {
            let _ = peer.event("preview-state-changed", payload);
        }
        return Ok(json!({"status":"invalid","revision":revision,"issues":issues}));
    }
    let resource_barrier = active
        .requires_resource_barrier_with_resources(&project, &resources)
        .await?;
    let had_pending = state.pending_preview_update.lock().await.is_some();
    if resource_barrier && active.resource_busy().await {
        active.runtime.begin_revision_barrier();
        *state.pending_preview_update.lock().await = Some((revision, project, Some(resources)));
        *state.preview_update_status.lock().await = "waiting".into();
        let waiting = json!({"running":true,"generation":state.preview_generation.load(Ordering::Acquire),"revision":state.preview_revision.load(Ordering::Acquire),"pendingRevision":revision,"updateStatus":"waiting"});
        let _ = app.emit("preview-state-changed", waiting.clone());
        if let Some(peer) = state.preview_peer.lock().await.as_ref() {
            let _ = peer.event("preview-state-changed", waiting);
        }
        if !state.preview_update_worker.swap(true, Ordering::AcqRel) {
            let cancel = state.preview_update_cancel.lock().await.clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let state = app.state::<DesktopState>();
                    let active = match state.session.lock().await.clone() {
                        Some(active) => active,
                        None => break,
                    };
                    tokio::select! {
                        _ = cancel.cancelled() => break,
                        _ = active.wait_for_resource_idle() => {}
                    }
                    if cancel.is_cancelled() {
                        break;
                    }
                    let _update_lock = state.preview_update_lock.lock().await;
                    let next = state.pending_preview_update.lock().await.take();
                    let Some((revision, project, resources)) = next else {
                        state.preview_update_worker.store(false, Ordering::Release);
                        if state.pending_preview_update.lock().await.is_some()
                            && !state.preview_update_worker.swap(true, Ordering::AcqRel)
                        {
                            continue;
                        }
                        active.runtime.finish_revision_barrier();
                        break;
                    };
                    let result = apply_preview_revision_now_locked(
                        &app, &state, revision, project, resources,
                    )
                    .await;
                    if let Err(message) = result {
                        active.runtime.finish_revision_barrier();
                        state.preview_update_worker.store(false, Ordering::Release);
                        *state.preview_update_status.lock().await = "invalid".into();
                        if let Some(peer) = state.preview_peer.lock().await.clone() {
                            let _ = peer.event(
                                "preview-update-failed",
                                json!({"revision":revision,"message":message}),
                            );
                        }
                        break;
                    }
                }
                let state = app.state::<DesktopState>();
                state.preview_update_worker.store(false, Ordering::Release);
            });
        }
        return Ok(json!({"status":"waiting","revision":revision,"issues":[]}));
    }
    if had_pending {
        state.pending_preview_update.lock().await.take();
        active.runtime.finish_revision_barrier();
    }
    apply_preview_revision_now_locked(&app, state, revision, project, Some(resources)).await?;
    Ok(json!({"status":"applied","revision":revision,"issues":[]}))
}

async fn apply_preview_revision_now_locked(
    app: &tauri::AppHandle,
    state: &DesktopState,
    revision: u64,
    project: Project,
    resources: Option<Resources>,
) -> Result<()> {
    if revision <= state.preview_revision.load(Ordering::Acquire) {
        return Ok(());
    }
    let active = state
        .session
        .lock()
        .await
        .clone()
        .ok_or("预览运行会话不存在")?;
    let resources = match resources {
        Some(resources) => resources,
        None => active.current_resources().await,
    };
    active
        .apply_revision(project.clone(), resources.clone())
        .await?;
    state.runtime_windows.reconcile_project(app, &project).await;
    state.preview_revision.store(revision, Ordering::Release);
    *state.current_project.lock().await = Some(project.clone());
    *state.preview_project.lock().await = Some(project.clone());
    *state.project_path.lock().await = Some(resources.root.join("project.json"));
    if state
        .pending_preview_update
        .lock()
        .await
        .as_ref()
        .is_some_and(|(pending, _, _)| *pending == revision)
    {
        state.pending_preview_update.lock().await.take();
    }
    *state.preview_update_status.lock().await = "applied".into();
    let payload = json!({"running":true,"generation":state.preview_generation.load(Ordering::Acquire),"revision":revision,"updateStatus":"applied"});
    if let Some(peer) = state.preview_peer.lock().await.as_ref() {
        let _ = peer.event("preview-state-changed", payload.clone());
        let _ = peer.event(
            "preview-update-applied",
            json!({"revision":revision,"project":project}),
        );
    }
    let _ = app.emit("preview-state-changed", payload);
    Ok(())
}

#[tauri::command]
async fn update_preview_input(
    app: tauri::AppHandle,
    id: String,
    value: Value,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "update_preview_input",
                json!({"id":id,"value":value,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    let session = state.session_for(expected_generation).await?;
    session
        .runtime
        .update_input_with_events(id.clone(), value.clone())
        .await;
    let payload = json!({"id":id,"value":value,"generation":state.preview_generation.load(Ordering::Acquire)});
    let _ = app.emit("preview-input-changed", payload.clone());
    if let Some(peer) = state.preview_peer.lock().await.as_ref() {
        peer.event("preview-input-changed", payload)?;
    }
    Ok(())
}

#[tauri::command]
async fn set_preview_locale(
    app: tauri::AppHandle,
    locale: String,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "set_preview_locale",
                json!({"locale":locale,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    if expected_generation
        .is_some_and(|expected| expected != state.preview_generation.load(Ordering::Acquire))
    {
        return Err("预览会话已切换，请重新连接预览".into());
    }
    let project = state
        .preview_project
        .lock()
        .await
        .clone()
        .ok_or("预览工程不存在")?;
    let config = project.i18n.as_ref().ok_or("项目尚未配置多语言")?;
    if locale != config.default_locale && !config.locales.contains(&locale) {
        return Err("项目未配置此语言".into());
    }
    *state.preview_locale.lock().await = Some(locale.clone());
    let payload =
        json!({"locale":locale,"generation":state.preview_generation.load(Ordering::Acquire)});
    let _ = app.emit("preview-locale-changed", payload.clone());
    if let Some(peer) = state.preview_peer.lock().await.as_ref() {
        let _ = peer.event("preview-locale-changed", payload);
    }
    Ok(())
}

#[tauri::command]
async fn open_logic_window(
    app: tauri::AppHandle,
    flow_id: Option<String>,
    node_id: Option<String>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    state.studio()?;
    if flow_id
        .as_deref()
        .is_some_and(|id| !hmi_core::safe_identifier(id))
        || node_id
            .as_deref()
            .is_some_and(|id| !hmi_core::safe_identifier(id))
    {
        return Err("逻辑节点或流程 ID 无效".into());
    }
    let url = format!(
        "index.html?view=logic{}{}",
        flow_id
            .as_ref()
            .map(|id| format!("&flowId={id}"))
            .unwrap_or_default(),
        node_id
            .as_ref()
            .map(|id| format!("&nodeId={id}"))
            .unwrap_or_default()
    );
    if let Some(window) = app.get_webview_window("logic") {
        window.unminimize().map_err(error)?;
        window.show().map_err(error)?;
        window.set_focus().map_err(error)?;
        let _ = app.emit_to(
            "logic",
            "logic-focus",
            json!({"flowId":flow_id,"nodeId":node_id}),
        );
        return Ok(());
    }
    tauri::WebviewWindowBuilder::new(&app, "logic", tauri::WebviewUrl::App(url.into()))
        .title("逻辑流程")
        .inner_size(1180.0, 820.0)
        .min_inner_size(800.0, 600.0)
        .build()
        .map_err(error)?;
    Ok(())
}

async fn stop_session_locked(
    app: &tauri::AppHandle,
    state: &DesktopState,
) -> (Option<tauri::WebviewWindow>, Option<u64>) {
    state.runtime_windows.close_all(app).await;
    state.preview_update_cancel.lock().await.cancel();
    state.pending_preview_update.lock().await.take();
    state.preview_update_worker.store(false, Ordering::Release);
    if let Some(session) = state.session.lock().await.as_ref() {
        session.runtime.finish_revision_barrier();
    }
    let had_preview = state.preview_project.lock().await.take().is_some();
    let window = app.get_webview_window("preview");
    let host = state.preview_process.lock().await.take();
    let old_generation = state.preview_generation.load(Ordering::Acquire);
    let close_generation = if had_preview || window.is_some() || host.is_some() {
        state.preview_generation.fetch_add(1, Ordering::AcqRel);
        *state.preview_page.lock().await = None;
        Some(old_generation)
    } else {
        None
    };
    if let Some(session) = state.session.lock().await.take() {
        session.stop().await;
    }
    if let Some(host) = host {
        host.stop().await;
    }
    if close_generation.is_some() {
        emit_preview_state(app, false, state.preview_generation.load(Ordering::Acquire));
    }
    (window, close_generation)
}

fn schedule_preview_destroy(app: tauri::AppHandle, window: tauri::WebviewWindow, generation: u64) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let state = app.state::<DesktopState>();
        if state.preview_closing.load(Ordering::Acquire) == generation {
            if let Err(close_error) = window.destroy() {
                let _ = state.preview_closing.compare_exchange(
                    generation,
                    0,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                emit_preview_window_error(&app, close_error);
            }
        }
    });
}

fn emit_preview_window_error(app: &tauri::AppHandle, close_error: impl std::fmt::Display) {
    let generation = app
        .state::<DesktopState>()
        .preview_generation
        .load(Ordering::Acquire);
    let mut event = serde_json::to_value(hmi_core::RuntimeEvent::new(
        "log",
        json!({"level":"error","message":format!("关闭预览窗口失败: {close_error}")}),
    ))
    .unwrap_or_else(|_| json!({}));
    if let Some(event) = event.as_object_mut() {
        event.insert("generation".into(), json!(generation));
    }
    let _ = app.emit_to("main", "runtime-event", event.clone());
    let _ = app.emit_to("preview", "runtime-event", event);
}

fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    let app = window.app_handle();
    let state = app.state::<DesktopState>();
    match event {
        tauri::WindowEvent::Destroyed if window.label() == "preview" => {
            state.preview_closing.store(0, Ordering::Release);
        }
        tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "preview" => {
            api.prevent_close();
            let generation = state.preview_generation.load(Ordering::Acquire);
            if state
                .preview_closing
                .compare_exchange(0, generation, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                return;
            }
            let app = app.clone();
            let Some(closing_window) = app.get_webview_window("preview") else {
                return;
            };
            tauri::async_runtime::spawn(async move {
                let state = app.state::<DesktopState>();
                let _transition = state.transition.lock().await;
                if state.preview_generation.load(Ordering::Acquire) == generation {
                    let _ = stop_session_locked(&app, &state).await;
                }
                if let Err(close_error) = closing_window.destroy() {
                    let _ = state.preview_closing.compare_exchange(
                        generation,
                        0,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    );
                    emit_preview_window_error(&app, close_error);
                }
            });
        }
        tauri::WindowEvent::CloseRequested { api, .. } if window.label() == "main" => {
            if state.mode == AppMode::Runner {
                if let Ok(project) = state.current_project.try_lock() {
                    if let (Some(project), Some(webview)) =
                        (project.as_ref(), app.get_webview_window("main"))
                    {
                        let _ = window_settings::save(&webview, &project.id);
                    }
                }
            }
            let session_active = state
                .session
                .try_lock()
                .map(|session| session.is_some())
                .unwrap_or(true);
            let preview_active = state
                .preview_project
                .try_lock()
                .map(|project| project.is_some())
                .unwrap_or(true);
            let transition_active = state.transition.try_lock().is_err();
            if state.mode != AppMode::Runner
                && !session_active
                && !preview_active
                && !transition_active
                && app.get_webview_window("preview").is_none()
            {
                return;
            }
            api.prevent_close();
            if state.main_close_pending.swap(true, Ordering::AcqRel) {
                return;
            }
            state.shutting_down.store(true, Ordering::Release);
            let app = app.clone();
            let main_window = window.clone();
            tauri::async_runtime::spawn(async move {
                let state = app.state::<DesktopState>();
                let _transition = state.transition.lock().await;
                let (preview_window, generation) = stop_session_locked(&app, &state).await;
                if let Some(logic) = app.get_webview_window("logic") {
                    let _ = logic.destroy();
                }
                if let Some(preview_window) = preview_window {
                    if let Some(generation) = generation {
                        state.preview_closing.store(generation, Ordering::Release);
                    }
                    if let Err(close_error) = preview_window.destroy() {
                        if let Some(generation) = generation {
                            let _ = state.preview_closing.compare_exchange(
                                generation,
                                0,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        }
                        state.main_close_pending.store(false, Ordering::Release);
                        state.shutting_down.store(false, Ordering::Release);
                        emit_preview_window_error(&app, close_error);
                        return;
                    }
                }
                if let Err(close_error) = main_window.destroy() {
                    state.main_close_pending.store(false, Ordering::Release);
                    state.shutting_down.store(false, Ordering::Release);
                    emit_preview_window_error(&app, close_error);
                }
            });
        }
        _ => {}
    }
}

#[tauri::command]
async fn trigger_flow(
    flow_id: String,
    entry_node_id: Option<String>,
    message: Value,
    inputs: Value,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<String> {
    if state.mode == AppMode::Studio {
        let value = state.preview_rpc("trigger_flow", json!({"flowId":flow_id,"entryNodeId":entry_node_id,"message":message,"inputs":inputs,"expectedGeneration":expected_generation,"preferRuntimeInputs":true})).await?;
        return serde_json::from_value(value).map_err(error);
    }
    if state.preview_host_mode && message.pointer("/context/windowInstanceId").is_none() {
        let run_id = state
            .session_for(expected_generation)
            .await?
            .runtime
            .trigger_with_runtime_inputs(&flow_id, entry_node_id.as_deref(), message, inputs)
            .await
            .map_err(error)?;
        return Ok(run_id);
    }
    state
        .session_for(expected_generation)
        .await?
        .runtime
        .trigger(&flow_id, entry_node_id.as_deref(), message, inputs)
        .await
        .map_err(error)
}
#[tauri::command]
async fn start_flow(
    flow_id: String,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "start_flow",
                json!({"flowId":flow_id,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    state
        .session_for(expected_generation)
        .await?
        .runtime
        .start_flow(&flow_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn stop_flow(
    flow_id: String,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "stop_flow",
                json!({"flowId":flow_id,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    state
        .session_for(expected_generation)
        .await?
        .runtime
        .stop_flow(&flow_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn runtime_snapshot(
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<RuntimeSnapshot> {
    if state.mode == AppMode::Studio {
        let value = state
            .preview_rpc(
                "runtime_snapshot",
                json!({"expectedGeneration":expected_generation}),
            )
            .await?;
        return serde_json::from_value(value).map_err(error);
    }
    match state.session_for(expected_generation).await {
        Ok(session) => Ok(session.runtime.snapshot().await),
        Err(_) if expected_generation.is_none() => Ok(RuntimeSnapshot::default()),
        Err(error) => Err(error),
    }
}
#[tauri::command]
async fn open_connection(
    connection_id: String,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    let _transition = state.transition.lock().await;
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "open_connection",
                json!({"connectionId":connection_id,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    let session = state.session_for(expected_generation).await?;
    session
        .connection_manager()
        .await
        .open(&connection_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn close_connection(
    connection_id: String,
    expected_generation: Option<u64>,
    state: State<'_, DesktopState>,
) -> Result<()> {
    let _transition = state.transition.lock().await;
    if state.mode == AppMode::Studio {
        state
            .preview_rpc(
                "close_connection",
                json!({"connectionId":connection_id,"expectedGeneration":expected_generation}),
            )
            .await?;
        return Ok(());
    }
    let session = state.session_for(expected_generation).await?;
    session
        .connection_manager()
        .await
        .close(&connection_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn save_runtime_settings(project: Project, state: State<'_, DesktopState>) -> Result<()> {
    if state.mode != AppMode::Runner {
        return Err("此操作仅在 Runner 可用".into());
    }
    if state.session.lock().await.is_some() {
        return Err("请停止运行后修改串口设置".into());
    }
    session::save_settings(&project)
}
#[tauri::command]
async fn export_project(
    _project: Project,
    destination: String,
    state: State<'_, DesktopState>,
) -> Result<export::ExportResult> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    let project = state.workspace.snapshot().await;
    let project_resources = state.workspace_resources();
    let issues = session::validate(&project_resources, &project).await;
    if let Some(issue) = issues.iter().find(|i| i.severity == "error") {
        return Err(format!("{}: {}", issue.path, issue.message));
    }
    let directory = state.workspace.root();
    let resources = state.resources.clone();
    let project_resources = project_resources.clone();
    tokio::task::spawn_blocking(move || {
        export::package_with_project_resources(
            &resources,
            &project_resources,
            &project,
            &directory,
            Path::new(&destination),
        )
    })
    .await
    .map_err(error)?
}

#[tauri::command]
async fn import_asset(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    state.studio()?;
    let source = PathBuf::from(path).canonicalize().map_err(error)?;
    let extension = source
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ![
        "png", "jpg", "jpeg", "webp", "gif", "woff", "woff2", "ttf", "otf", "bin", "hex", "ihx",
        "ptbcfg", "json",
    ]
    .contains(&extension.as_str())
    {
        return Err("支持图片、字体、BIN/HEX/IHX 固件及 PTBCFG/JSON 配置文件".into());
    }
    if std::fs::metadata(&source).map_err(error)?.len() > 64 * 1024 * 1024 {
        return Err("单个资源最大 64 MiB".into());
    }
    let id = format!("asset_{}", uuid::Uuid::new_v4().simple());
    let relative = format!("assets/{id}.{extension}");
    let destination = state.workspace.root().join(&relative);
    std::fs::create_dir_all(destination.parent().unwrap()).map_err(error)?;
    let checksum = sha256(&source)?;
    let name = source.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::copy(&source, &destination).map_err(error)?;
    if sha256(&destination)? != checksum {
        return Err("资源复制期间发生变化".into());
    }
    Ok(
        json!({"id":id,"path":relative,"absolutePath":destination,"name":name,"size":std::fs::metadata(&destination).map_err(error)?.len(),"sha256":checksum}),
    )
}
#[tauri::command]
async fn import_asset_bytes(
    name: String,
    bytes_base64: String,
    state: State<'_, DesktopState>,
) -> Result<Value> {
    state.studio()?;
    use base64::Engine;
    if bytes_base64.len() > 90 * 1024 * 1024 {
        return Err("单个资源最大 64 MiB".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64)
        .map_err(error)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("单个资源最大 64 MiB".into());
    }
    let suffix = Path::new(&name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ![
        "png", "jpg", "jpeg", "webp", "gif", "woff", "woff2", "ttf", "otf",
    ]
    .contains(&suffix.as_str())
    {
        return Err("设计库资源仅支持图片和字体".into());
    }
    let id = format!("asset_{}", uuid::Uuid::new_v4().simple());
    let relative = format!("assets/{id}.{suffix}");
    let destination = state.workspace.root().join(&relative);
    std::fs::create_dir_all(destination.parent().unwrap()).map_err(error)?;
    let mut staged =
        tempfile::NamedTempFile::new_in(destination.parent().unwrap()).map_err(error)?;
    use std::io::Write;
    staged.write_all(&bytes).map_err(error)?;
    staged.persist(&destination).map_err(error)?;
    Ok(
        json!({"id":id,"path":relative,"absolutePath":destination,"name":name,"size":bytes.len(),"sha256":sha256(&destination)?}),
    )
}
#[tauri::command]
async fn resolve_assets(project: Project, state: State<'_, DesktopState>) -> Result<Value> {
    let directory = if state.mode == AppMode::Studio {
        Some(state.workspace.root())
    } else {
        state
            .project_path
            .lock()
            .await
            .as_ref()
            .and_then(|p| p.parent().map(Path::to_path_buf))
    };
    let mut assets = serde_json::Map::new();
    for asset in &project.assets {
        let directory = directory.as_ref().ok_or("项目资源目录不可用")?;
        assets.insert(
            asset.id.clone(),
            json!(contained_file(directory, &asset.path)?),
        );
    }
    Ok(Value::Object(assets))
}

async fn handle_preview_rpc(
    app: tauri::AppHandle,
    method: String,
    payload: Value,
) -> Result<Value> {
    let state = app.state::<DesktopState>();
    match method.as_str() {
        "get_preview_state" => {
            serde_json::to_value(get_preview_state(app.clone(), state).await?).map_err(error)
        }
        "runtime_snapshot" => {
            let generation = payload.get("expectedGeneration").and_then(Value::as_u64);
            serde_json::to_value(
                state
                    .session_for(generation)
                    .await?
                    .runtime
                    .snapshot()
                    .await,
            )
            .map_err(error)
        }
        "trigger_flow" => {
            let session = state
                .session_for(payload.get("expectedGeneration").and_then(Value::as_u64))
                .await?;
            let flow_id = payload["flowId"].as_str().ok_or("缺少 flowId")?;
            let entry = payload["entryNodeId"].as_str();
            let message = payload
                .get("message")
                .cloned()
                .unwrap_or_else(|| json!({"payload":null}));
            let inputs = payload.get("inputs").cloned().unwrap_or_else(|| json!({}));
            let run_id = if payload["preferRuntimeInputs"] == true {
                session
                    .runtime
                    .trigger_with_runtime_inputs(flow_id, entry, message, inputs)
                    .await
                    .map_err(error)?
            } else {
                session
                    .runtime
                    .trigger(flow_id, entry, message, inputs)
                    .await
                    .map_err(error)?
            };
            Ok(json!(run_id))
        }
        "start_flow" | "stop_flow" => {
            let session = state
                .session_for(payload.get("expectedGeneration").and_then(Value::as_u64))
                .await?;
            let id = payload["flowId"].as_str().ok_or("缺少 flowId")?;
            if method == "start_flow" {
                session.runtime.start_flow(id).await.map_err(error)?;
            } else {
                session.runtime.stop_flow(id).await.map_err(error)?;
            }
            Ok(Value::Null)
        }
        "open_connection" | "close_connection" => {
            let session = state
                .session_for(payload.get("expectedGeneration").and_then(Value::as_u64))
                .await?;
            let manager = session.connection_manager().await;
            let id = payload["connectionId"]
                .as_str()
                .ok_or("缺少 connectionId")?;
            if method == "open_connection" {
                manager.open(id).await.map_err(error)?;
            } else {
                manager.close(id).await.map_err(error)?;
            }
            Ok(Value::Null)
        }
        "update_preview_input" => {
            let session = state
                .session_for(payload.get("expectedGeneration").and_then(Value::as_u64))
                .await?;
            let id = payload["id"].as_str().ok_or("缺少 id")?.to_owned();
            let value = payload.get("value").cloned().unwrap_or(Value::Null);
            session
                .runtime
                .update_input_with_events(id.clone(), value.clone())
                .await;
            let event = json!({"id":id,"value":value,"generation":state.preview_generation.load(Ordering::Acquire)});
            let _ = app.emit("preview-input-changed", event.clone());
            if let Some(peer) = state.preview_peer.lock().await.as_ref() {
                peer.event("preview-input-changed", event)?;
            }
            Ok(Value::Null)
        }
        "set_preview_locale" => {
            let locale = payload["locale"].as_str().ok_or("缺少 locale")?;
            let project = state
                .preview_project
                .lock()
                .await
                .clone()
                .ok_or("预览工程不存在")?;
            let config = project.i18n.as_ref().ok_or("项目尚未配置多语言")?;
            if locale != config.default_locale && !config.locales.iter().any(|item| item == locale)
            {
                return Err("项目未配置此语言".into());
            }
            *state.preview_locale.lock().await = Some(locale.into());
            let event = json!({"locale":locale,"generation":state.preview_generation.load(Ordering::Acquire)});
            let _ = app.emit("preview-locale-changed", event.clone());
            if let Some(peer) = state.preview_peer.lock().await.as_ref() {
                let _ = peer.event("preview-locale-changed", event);
            }
            Ok(Value::Null)
        }
        "apply_preview_update" => {
            let project: Project =
                serde_json::from_value(payload.get("project").cloned().ok_or("缺少 project")?)
                    .map_err(error)?;
            let revision = payload["revision"].as_u64().ok_or("缺少 revision")?;
            let resources = payload
                .get("resourceRoot")
                .and_then(Value::as_str)
                .map(|path| Resources {
                    root: PathBuf::from(path),
                    runner: true,
                });
            apply_preview_revision(
                app.clone(),
                &state,
                revision,
                project,
                payload.get("expectedGeneration").and_then(Value::as_u64),
                resources,
            )
            .await
        }
        "focus_preview" => {
            if let Some(window) = app.get_webview_window("main") {
                window.unminimize().map_err(error)?;
                window.show().map_err(error)?;
                window.set_focus().map_err(error)?;
            }
            Ok(Value::Null)
        }
        "shutdown" => {
            state.shutting_down.store(true, Ordering::Release);
            if let Some(session) = state.session.lock().await.take() {
                session.stop().await;
            }
            app.exit(0);
            Ok(Value::Null)
        }
        "stop_preview" => {
            app.exit(0);
            Ok(Value::Null)
        }
        _ => Err(format!("不支持的预览 RPC: {method}")),
    }
}

pub fn run(context: tauri::Context<tauri::Wry>, mode: AppMode) {
    #[cfg(not(feature = "native-preview-smoke"))]
    if mode == AppMode::Studio && std::env::var_os("HMI_NATIVE_WORKSPACE_ROOT").is_some() {
        eprintln!("HMI_NATIVE_WORKSPACE_ROOT requires a native-preview-smoke build");
        return;
    }
    let resources = match Resources::discover(mode == AppMode::Runner) {
        Ok(resources) => resources,
        Err(error) => {
            startup::fatal(&error);
            return;
        }
    };
    if let Err(e) = startup::prepare_webview(&resources, mode) {
        startup::fatal(&e);
        return;
    }
    let args: Vec<_> = std::env::args_os().collect();
    let project_path = args
        .iter()
        .position(|a| a == "--project")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| (mode == AppMode::Runner).then(|| resources.root.join("project.json")));
    let preview_host_mode =
        mode == AppMode::Runner && std::env::var_os("HMI_PREVIEW_HOST").is_some();
    let workspace_resource_root = dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("HmiStudio/workspaces")
        .join("active");
    // Native acceptance runs must not replace or lock the user's active work.
    #[cfg(feature = "native-preview-smoke")]
    let workspace_resource_root = std::env::var_os("HMI_NATIVE_WORKSPACE_ROOT")
        .map(PathBuf::from)
        .unwrap_or(workspace_resource_root);
    let recovered = if mode == AppMode::Studio {
        workspace::recovered_project(&workspace_resource_root, project_path.as_deref())
    } else {
        None
    };
    let mut retained_missing = Vec::new();
    let mut initial_project = if let Some(project) = recovered {
        project
    } else if let Some(path) = project_path.as_ref().filter(|p| p.is_file()) {
        if path.extension().and_then(|v| v.to_str()) == Some("hmiproj") {
            match project_package::read_package(path, &workspace_resource_root) {
                Ok((project, manifest)) => {
                    retained_missing = manifest.missing_dependencies;
                    project
                }
                Err(problem) => {
                    startup::fatal(&problem);
                    return;
                }
            }
        } else {
            match load_project(path) {
                Ok(project) => project,
                Err(problem) => {
                    startup::fatal(&problem);
                    return;
                }
            }
        }
    } else {
        match serde_json::from_str(include_str!("../../../resources/default/project.json")) {
            Ok(project) => project,
            Err(problem) => {
                startup::fatal(&problem.to_string());
                return;
            }
        }
    };
    if mode == AppMode::Runner && !preview_host_mode {
        if let Err(problem) = session::apply_settings(&mut initial_project) {
            startup::fatal(&problem);
            return;
        }
    }
    let workspace = if mode == AppMode::Studio {
        match workspace::WorkspaceStore::with_shared_resources_and_missing(
            initial_project.clone(),
            project_path.clone(),
            workspace_resource_root,
            resources.root.clone(),
            retained_missing,
        ) {
            Ok(store) => Arc::new(store),
            Err(problem) => {
                startup::fatal(&problem);
                return;
            }
        }
    } else {
        Arc::new(workspace::WorkspaceStore::new(
            initial_project.clone(),
            project_path.clone(),
            workspace_resource_root,
        ))
    };
    initial_project = tauri::async_runtime::block_on(workspace.snapshot());
    let initial_generation = std::env::var("HMI_PREVIEW_REVISION")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    let workspace_state = workspace.clone();
    let state = DesktopState {
        mode,
        runtime_windows: runtime_windows::RuntimeWindows::default(),
        resources,
        project_path: Mutex::new(project_path),
        current_project: Mutex::new(Some(initial_project.clone())),
        session: Mutex::new(None),
        preview_process: Mutex::new(None),
        preview_peer: Mutex::new(None),
        preview_locale: Mutex::new(
            initial_project
                .i18n
                .as_ref()
                .map(|i18n| i18n.default_locale.clone()),
        ),
        preview_host_mode,
        workspace,
        preview_project: Mutex::new(preview_host_mode.then_some(initial_project.clone())),
        preview_page: Arc::new(Mutex::new(
            initial_project.pages.first().map(|page| page.id.clone()),
        )),
        preview_generation: Arc::new(AtomicU64::new(if preview_host_mode { 1 } else { 0 })),
        preview_revision: AtomicU64::new(initial_generation),
        pending_preview_update: Mutex::new(None),
        preview_update_worker: AtomicBool::new(false),
        preview_update_cancel: Mutex::new(CancellationToken::new()),
        preview_update_lock: Mutex::new(()),
        preview_update_status: Mutex::new("applied".into()),
        preview_closing: AtomicU64::new(0),
        main_close_pending: AtomicBool::new(false),
        transition: Mutex::new(()),
        shutting_down: AtomicBool::new(false),
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .manage(workspace_state)
        .invoke_handler(tauri::generate_handler![
            get_environment,
            runtime_windows::get_runtime_window_state,
            runtime_windows::update_runtime_window_input,
            runtime_files::runtime_import_file,
            get_initial_project,
            open_project,
            save_project,
            validate_project,
            list_serial_ports,
            list_plugins,
            import_plugin,
            import_backend,
            import_python_runtime,
            import_python_dependencies,
            start_preview,
            open_preview_window,
            get_preview_state,
            stop_preview,
            trigger_flow,
            start_flow,
            stop_flow,
            runtime_snapshot,
            open_connection,
            close_connection,
            save_runtime_settings,
            export_project,
            import_asset,
            import_asset_bytes,
            resolve_assets,
            workspace::workspace_state,
            workspace::apply_workspace_operations,
            workspace::undo_workspace,
            workspace::redo_workspace,
            workspace::replace_workspace,
            workspace::open_workspace_package,
            workspace::save_workspace_package,
            workspace::import_legacy_project,
            apply_preview_update,
            update_preview_input,
            set_preview_locale,
            open_logic_window
        ])
        .setup(|app| {
            let state = app.state::<DesktopState>();
            if state.preview_host_mode {
                let app_handle = app.handle().clone();
                let result = tauri::async_runtime::block_on(async {
                    let project = state
                        .current_project
                        .lock()
                        .await
                        .clone()
                        .ok_or("预览工程未加载")?;
                    let generation = state.preview_generation.load(Ordering::Acquire);
                    let runtime_session =
                        start_runtime_session(&app_handle, &state, &project, generation).await?;
                    *state.session.lock().await = Some(runtime_session.clone());
                    let endpoint = std::env::var("HMI_PREVIEW_ENDPOINT").map_err(error)?;
                    let token = std::env::var("HMI_PREVIEW_TOKEN").map_err(error)?;
                    let handler_app = app_handle.clone();
                    #[cfg(unix)]
                    let endpoint_path = PathBuf::from(endpoint);
                    let peer = loop {
                        #[cfg(unix)]
                        let connected =
                            preview_host::local::connect_with_handler(&endpoint_path, &token, {
                                let handler_app = handler_app.clone();
                                move |method, payload| {
                                    let app = handler_app.clone();
                                    async move { handle_preview_rpc(app, method, payload).await }
                                }
                            })
                            .await;
                        #[cfg(windows)]
                        let connected =
                            preview_host::local::connect_with_handler(&endpoint, &token, {
                                let handler_app = handler_app.clone();
                                move |method, payload| {
                                    let app = handler_app.clone();
                                    async move { handle_preview_rpc(app, method, payload).await }
                                }
                            })
                            .await;
                        match connected {
                            Ok(peer) => break Arc::new(peer),
                            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                        }
                    };
                    *state.preview_peer.lock().await = Some(peer.clone());
                    let disconnect = peer.closed_token();
                    let supervisor_app = app_handle.clone();
                    tokio::spawn(async move {
                        disconnect.cancelled().await;
                        let state = supervisor_app.state::<DesktopState>();
                        if state.shutting_down.swap(true, Ordering::AcqRel) {
                            return;
                        }
                        let _transition = state.transition.lock().await;
                        let _ = stop_session_locked(&supervisor_app, &state).await;
                        supervisor_app.exit(0);
                    });
                    let mut events = runtime_session.runtime.subscribe();
                    let generation = state.preview_generation.load(Ordering::Acquire);
                    tokio::spawn(async move {
                        while let Ok(event) = events.recv().await {
                            let mut payload =
                                serde_json::to_value(event).unwrap_or_else(|_| json!({}));
                            if let Some(object) = payload.as_object_mut() {
                                object.insert("generation".into(), json!(generation));
                            }
                            let _ = peer.event("runtime-event", payload);
                        }
                    });
                    Ok::<(), String>(())
                });
                result.map_err(|message| {
                    Box::<dyn std::error::Error>::from(std::io::Error::other(message))
                })?;
            }
            Ok(())
        })
        .on_window_event(handle_window_event)
        .build(context)
        .expect("Cannot build HMI application");

    app.run(move |app, event| {
        if let tauri::RunEvent::Exit = event {
            tauri::async_runtime::block_on(async {
                let state = app.state::<DesktopState>();
                state.shutting_down.store(true, Ordering::Release);
                let _transition = state.transition.lock().await;
                let (preview_window, generation) = stop_session_locked(&app, &state).await;
                if let Some(logic) = app.get_webview_window("logic") {
                    let _ = logic.destroy();
                }
                if let Some(preview_window) = preview_window {
                    if let Some(generation) = generation {
                        state.preview_closing.store(generation, Ordering::Release);
                    }
                    let _ = preview_window.destroy();
                }
            });
        }
    });
}
