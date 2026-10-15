mod export;
mod resources;
mod runtime_log;
mod session;
mod startup;
pub mod tooling;
mod window_settings;

use hmi_core::{Project, RuntimeSnapshot, ValidationIssue};
use resources::*;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

#[derive(Clone, Copy, PartialEq)]
pub enum AppMode {
    Studio,
    Runner,
}
pub struct DesktopState {
    mode: AppMode,
    resources: Resources,
    project_path: Mutex<Option<PathBuf>>,
    current_project: Mutex<Option<Project>>,
    session: Mutex<Option<Arc<session::Session>>>,
    transition: Mutex<()>,
    shutting_down: AtomicBool,
}
impl DesktopState {
    async fn session(&self) -> Result<Arc<session::Session>> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("应用正在退出".into());
        }
        self.session
            .lock()
            .await
            .clone()
            .ok_or_else(|| "请先启动预览或运行会话".into())
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
        json!({"mode":if state.mode == AppMode::Studio {"studio"} else {"runner"},
        "version":env!("CARGO_PKG_VERSION"), "pythonAvailable":state.resources.python(&project_id).join("runtime-manifest.json").is_file(),
        "runnerAvailable":state.resources.template().join("HmiRunner.exe").is_file(),
        "webviewAvailable":state.resources.webview().join("msedgewebview2.exe").is_file(),
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
        serde_json::from_str(include_str!("../../../resources/example/project.json"))
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
    Ok(session::validate(&state.resources, &project).await)
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
    session::check_plugin(&directory)?;
    let id = manifest["id"].as_str().ok_or("缺少插件 ID")?;
    let destination = state.resources.plugins().join(id);
    if destination.exists() {
        return Err("同 ID 插件已存在；请使用新的版本 ID 导入".into());
    }
    let parent = state.resources.plugins();
    std::fs::create_dir_all(&parent).map_err(error)?;
    let stage = tempfile::tempdir_in(&parent).map_err(error)?;
    copy_tree(&directory, stage.path())?;
    session::check_plugin(stage.path())?;
    std::fs::rename(stage.path(), &destination).map_err(error)?;
    Ok(manifest)
}
#[tauri::command]
async fn import_python_runtime(path: String, state: State<'_, DesktopState>) -> Result<()> {
    import_python(path, &state, false).await
}
#[tauri::command]
async fn import_python_dependencies(path: String, state: State<'_, DesktopState>) -> Result<Value> {
    import_python(path, &state, true).await?;
    let mut current = state.current_project.lock().await;
    let project = current.as_mut().ok_or("请先保存项目")?;
    let lock = read_json(
        &state
            .resources
            .python(&project.id)
            .join("dependencies.lock.json"),
    )?;
    let config = json!({"runtimeId":"3.14.8-win_amd64","runtimeSha256":hmi_python::ARCHIVE_SHA256,"dependencyLock":lock["packages"]});
    project.python = Some(serde_json::from_value(config.clone()).map_err(error)?);
    Ok(config)
}
async fn import_python(path: String, state: &DesktopState, project_specific: bool) -> Result<()> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    if state.session.lock().await.is_some() {
        return Err("请停止预览后导入 Python 资源".into());
    }
    let env = hmi_python::PythonEnvironment::load(path).map_err(error)?;
    hmi_python::PythonExecutor::new(env.clone())
        .validate_imports_with_limits(hmi_python::PythonLimits {
            timeout: std::time::Duration::from_secs(15),
            memory_bytes: 512 * 1024 * 1024,
        })
        .await
        .map_err(error)?;
    let destination = if project_specific {
        let project = state.current_project.lock().await;
        let id = &project.as_ref().ok_or("请先保存当前项目")?.id;
        if !hmi_core::safe_identifier(id) {
            return Err("项目 ID 无效".into());
        }
        state.resources.root.join("python/projects").join(id)
    } else {
        state.resources.root.join("python/3.14.8-win_amd64")
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
    if state.mode == AppMode::Runner {
        session::save_settings(&project)?;
    }
    let runtime = session::Session::new(project.clone(), state.resources.clone()).await?;
    let mut events = runtime.runtime.subscribe();
    let cancel = runtime.cancel.clone();
    let log_directory = session::user_directory(&project.id)?;
    let forwarder = tokio::spawn(async move {
        let mut log = runtime_log::RuntimeLog::new(log_directory.join("runtime.jsonl")).await;
        let mut log_warned = false;
        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    loop { match events.try_recv() {
                        Ok(event) => forward_runtime_event(&app, &mut log, &mut log_warned, &event).await,
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(count)) => {
                            forward_runtime_event(&app, &mut log, &mut log_warned, &hmi_core::RuntimeEvent::new("log", json!({"level":"error","message":format!("运行事件队列溢出，丢失 {count} 条事件")}))).await;
                        },
                        Err(_) => break,
                    }}
                    break;
                },
                event = events.recv() => match event {
                    Ok(event) => forward_runtime_event(&app, &mut log, &mut log_warned, &event).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(count)) => {
                        forward_runtime_event(&app, &mut log, &mut log_warned, &hmi_core::RuntimeEvent::new("log", json!({"level":"error","message":format!("运行事件队列溢出，丢失 {count} 条事件")}))).await;
                    },
                    Err(_) => break,
                }
            }
        }
        if let Ok(log) = &mut log {
            if let Err(error) = log.flush().await {
                let _ = app.emit(
                    "runtime-event",
                    hmi_core::RuntimeEvent::new(
                        "log",
                        json!({"level":"error","message":format!("日志保存失败: {error}")}),
                    ),
                );
            }
        }
    });
    runtime.add_event_forwarder(forwarder).await;
    runtime.runtime.start().await.map_err(error)?;
    *state.current_project.lock().await = Some(project);
    *state.session.lock().await = Some(runtime);
    Ok(())
}
async fn forward_runtime_event(
    app: &tauri::AppHandle,
    log: &mut std::io::Result<runtime_log::RuntimeLog>,
    warned: &mut bool,
    event: &hmi_core::RuntimeEvent,
) {
    let _ = app.emit("runtime-event", event);
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
async fn stop_preview(state: State<'_, DesktopState>) -> Result<()> {
    let _transition = state.transition.lock().await;
    if let Some(session) = state.session.lock().await.take() {
        session.stop().await;
    }
    Ok(())
}
#[tauri::command]
async fn trigger_flow(
    flow_id: String,
    entry_node_id: Option<String>,
    message: Value,
    inputs: Value,
    state: State<'_, DesktopState>,
) -> Result<String> {
    state
        .session()
        .await?
        .runtime
        .trigger(&flow_id, entry_node_id.as_deref(), message, inputs)
        .await
        .map_err(error)
}
#[tauri::command]
async fn start_flow(flow_id: String, state: State<'_, DesktopState>) -> Result<()> {
    state
        .session()
        .await?
        .runtime
        .start_flow(&flow_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn stop_flow(flow_id: String, state: State<'_, DesktopState>) -> Result<()> {
    state
        .session()
        .await?
        .runtime
        .stop_flow(&flow_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn runtime_snapshot(state: State<'_, DesktopState>) -> Result<RuntimeSnapshot> {
    match state.session.lock().await.clone() {
        Some(s) => Ok(s.runtime.snapshot().await),
        None => Ok(RuntimeSnapshot::default()),
    }
}
#[tauri::command]
async fn open_connection(connection_id: String, state: State<'_, DesktopState>) -> Result<()> {
    let _transition = state.transition.lock().await;
    state
        .session()
        .await?
        .connections
        .open(&connection_id)
        .await
        .map_err(error)
}
#[tauri::command]
async fn close_connection(connection_id: String, state: State<'_, DesktopState>) -> Result<()> {
    let _transition = state.transition.lock().await;
    state
        .session()
        .await?
        .connections
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
    project: Project,
    destination: String,
    state: State<'_, DesktopState>,
) -> Result<export::ExportResult> {
    state.studio()?;
    let _transition = state.transition.lock().await;
    let issues = session::validate(&state.resources, &project).await;
    if let Some(issue) = issues.iter().find(|i| i.severity == "error") {
        return Err(format!("{}: {}", issue.path, issue.message));
    }
    let directory = state
        .project_path
        .lock()
        .await
        .as_ref()
        .and_then(|p| p.parent())
        .map(Path::to_path_buf)
        .unwrap_or(std::env::current_dir().map_err(error)?);
    let resources = state.resources.clone();
    tokio::task::spawn_blocking(move || {
        export::package(&resources, &project, &directory, Path::new(&destination))
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
        "png", "jpg", "jpeg", "webp", "gif", "woff", "woff2", "ttf", "otf",
    ]
    .contains(&extension.as_str())
    {
        return Err("支持 PNG/JPEG/WebP/GIF 图片及 WOFF/WOFF2/TTF/OTF 字体".into());
    }
    if std::fs::metadata(&source).map_err(error)?.len() > 64 * 1024 * 1024 {
        return Err("单个资源最大 64 MiB".into());
    }
    let current = state.project_path.lock().await;
    let directory = current
        .as_ref()
        .and_then(|p| p.parent())
        .ok_or("请先保存项目后导入资源")?;
    let id = format!("asset_{}", uuid::Uuid::new_v4().simple());
    let relative = format!("assets/{id}.{extension}");
    let destination = directory.join(&relative);
    std::fs::create_dir_all(destination.parent().unwrap()).map_err(error)?;
    std::fs::copy(source, &destination).map_err(error)?;
    Ok(json!({"id":id,"path":relative,"absolutePath":destination}))
}
#[tauri::command]
async fn resolve_assets(project: Project, state: State<'_, DesktopState>) -> Result<Value> {
    let current = state.project_path.lock().await;
    let directory = current.as_ref().and_then(|p| p.parent());
    let mut assets = serde_json::Map::new();
    for asset in &project.assets {
        let directory = directory.ok_or("请先保存项目，以解析本地资源")?;
        assets.insert(
            asset.id.clone(),
            json!(contained_file(directory, &asset.path)?),
        );
    }
    Ok(Value::Object(assets))
}

pub fn run(context: tauri::Context<tauri::Wry>, mode: AppMode) {
    let resources = Resources::discover(mode == AppMode::Runner);
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
    let state = DesktopState {
        mode,
        resources,
        project_path: Mutex::new(project_path),
        current_project: Mutex::new(None),
        session: Mutex::new(None),
        transition: Mutex::new(()),
        shutting_down: AtomicBool::new(false),
    };
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_environment,
            get_initial_project,
            open_project,
            save_project,
            validate_project,
            list_serial_ports,
            list_plugins,
            import_plugin,
            import_python_runtime,
            import_python_dependencies,
            start_preview,
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
            resolve_assets
        ])
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let app = window.app_handle();
                let state = app.state::<DesktopState>();
                if state.mode == AppMode::Runner {
                    if let Ok(project) = state.current_project.try_lock() {
                        if let (Some(project), Some(webview)) =
                            (project.as_ref(), app.get_webview_window("main"))
                        {
                            let _ = window_settings::save(&webview, &project.id);
                        }
                    }
                }
            }
        })
        .build(context)
        .expect("Cannot build HMI application");
    app.run(|app, event| {
        if let tauri::RunEvent::Exit = event {
            tauri::async_runtime::block_on(async {
                let state = app.state::<DesktopState>();
                state.shutting_down.store(true, Ordering::Release);
                let _transition = state.transition.lock().await;
                let session = state.session.lock().await.take();
                if let Some(session) = session {
                    session.stop().await;
                }
            });
        }
    });
}
