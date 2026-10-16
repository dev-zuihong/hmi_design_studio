use crate::project_package;
use hmi_core::Project;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tauri::{Emitter, State};
use tokio::sync::Mutex;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceOperation {
    pub path: Vec<String>,
    pub before_exists: bool,
    #[serde(default)]
    pub before: Value,
    pub after_exists: bool,
    #[serde(default)]
    pub after: Value,
    #[serde(default)]
    pub index: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceState {
    pub workspace_id: String,
    pub project: Value,
    pub revision: u64,
    pub saved_revision: u64,
    pub dirty: bool,
    pub path: Option<String>,
    pub can_undo: bool,
    pub can_redo: bool,
    pub histories: BTreeMap<String, HistoryState>,
    pub missing_dependencies: Vec<String>,
    pub recovered: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryState {
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct WindowHistory {
    undo: Vec<Vec<WorkspaceOperation>>,
    redo: Vec<Vec<WorkspaceOperation>>,
}

struct Inner {
    workspace_id: String,
    project: Value,
    revision: u64,
    saved_revision: u64,
    saved_fingerprint: String,
    path: Option<PathBuf>,
    histories: HashMap<String, WindowHistory>,
    applied_ops: HashSet<String>,
    missing_dependencies: Vec<String>,
    recovered: bool,
}

#[derive(Serialize, Deserialize)]
struct RecoveryImage {
    workspace_id: String,
    project: Value,
    revision: u64,
    saved_revision: u64,
    saved_fingerprint: String,
    path: Option<String>,
    histories: HashMap<String, WindowHistory>,
    missing_dependencies: Vec<String>,
}

/// One canonical project and operation log shared by all Studio windows.
pub struct WorkspaceStore {
    inner: Mutex<Inner>,
    resource_root: PathBuf,
    shared_resource_root: Option<PathBuf>,
    recovery_file: PathBuf,
    _active_lock: Option<ActiveLock>,
}

struct ActiveLock {
    path: PathBuf,
}
impl Drop for ActiveLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl From<&Inner> for RecoveryImage {
    fn from(inner: &Inner) -> Self {
        Self {
            workspace_id: inner.workspace_id.clone(),
            project: inner.project.clone(),
            revision: inner.revision,
            saved_revision: inner.saved_revision,
            saved_fingerprint: inner.saved_fingerprint.clone(),
            path: inner
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
            histories: inner.histories.clone(),
            missing_dependencies: inner.missing_dependencies.clone(),
        }
    }
}

fn recovery_file_for(root: &Path) -> PathBuf {
    let parent = root.parent().unwrap_or(Path::new("."));
    let name = root
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("active");
    parent.join(format!("{name}.recovery.json"))
}

pub fn recovered_project(root: &Path, requested_path: Option<&Path>) -> Option<Project> {
    let bytes = fs::read(recovery_file_for(root)).ok()?;
    let image: RecoveryImage = serde_json::from_slice(&bytes).ok()?;
    match (requested_path, image.path.as_deref()) {
        (Some(requested), Some(saved))
            if fs::canonicalize(requested)
                .unwrap_or_else(|_| requested.to_path_buf())
                .to_string_lossy()
                == saved => {}
        (None, _) => {}
        _ => return None,
    }
    serde_json::from_value(image.project).ok()
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path.parent().ok_or("恢复文件目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    temp.write_all(&serde_json::to_vec(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    temp.as_file().sync_all().map_err(|e| e.to_string())?;
    temp.persist(path).map_err(|e| e.error.to_string())?;
    if let Ok(directory) = fs::File::open(parent) {
        let _ = directory.sync_all();
    }
    Ok(())
}

fn persist_recovery(path: &Path, inner: &Inner) -> Result<()> {
    write_json_atomic(path, &RecoveryImage::from(inner))
}

fn acquire_active_lock(root: &Path) -> Result<ActiveLock> {
    let parent = root.parent().ok_or("工作目录无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let path = parent.join("active.lock");
    for _ in 0..3 {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                writeln!(file, "{}", std::process::id()).map_err(|e| e.to_string())?;
                file.sync_all().map_err(|e| e.to_string())?;
                return Ok(ActiveLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let pid = fs::read_to_string(&path)
                    .ok()
                    .and_then(|s| s.trim().parse::<u32>().ok());
                if pid.is_some_and(process_is_alive) {
                    return Err("已有设计器实例正在编辑活动工程".into());
                }
                fs::remove_file(&path).map_err(|e| e.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("无法取得活动工程锁".into())
}

fn process_is_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

impl WorkspaceStore {
    pub fn new(project: Project, path: Option<PathBuf>, workspace_root: PathBuf) -> Self {
        let path = path.map(|path| fs::canonicalize(&path).unwrap_or(path));
        let original_fingerprint =
            fingerprint(&serde_json::to_value(&project).expect("Project serializes to JSON"));
        let (project, migrated) = migrate_project(project);
        let value = serde_json::to_value(project).expect("Project serializes to JSON");
        let saved_fingerprint = if migrated {
            original_fingerprint
        } else {
            fingerprint(&value)
        };
        let recovery_file = recovery_file_for(&workspace_root);
        let initial_path = path.as_ref().map(|p| p.to_string_lossy().into_owned());
        let mut inner = Inner {
            workspace_id: uuid::Uuid::new_v4().to_string(),
            project: value,
            revision: 0,
            saved_revision: 0,
            saved_fingerprint: if path.is_some() {
                saved_fingerprint
            } else {
                String::new()
            },
            path,
            histories: HashMap::new(),
            applied_ops: HashSet::new(),
            missing_dependencies: Vec::new(),
            recovered: false,
        };
        if let Ok(bytes) = fs::read(&recovery_file) {
            if let Ok(recovery) = serde_json::from_slice::<RecoveryImage>(&bytes) {
                let path_matches = match (&initial_path, &recovery.path) {
                    (Some(current), Some(saved)) => current == saved,
                    (None, _) => true,
                    _ => false,
                };
                let valid_project =
                    serde_json::from_value::<Project>(recovery.project.clone()).is_ok();
                if path_matches && valid_project {
                    inner = Inner {
                        workspace_id: recovery.workspace_id,
                        project: recovery.project,
                        revision: recovery.revision,
                        saved_revision: recovery.saved_revision,
                        saved_fingerprint: recovery.saved_fingerprint,
                        path: recovery.path.map(PathBuf::from),
                        histories: recovery.histories,
                        applied_ops: HashSet::new(),
                        missing_dependencies: recovery.missing_dependencies,
                        recovered: true,
                    };
                }
            }
        }
        Self {
            resource_root: workspace_root,
            shared_resource_root: None,
            recovery_file,
            _active_lock: None,
            inner: Mutex::new(inner),
        }
    }

    /// Initialize a project work copy and copy already imported runtime resources into it.
    #[cfg(test)]
    pub fn with_shared_resources(
        project: Project,
        path: Option<PathBuf>,
        workspace_root: PathBuf,
        shared_resource_root: PathBuf,
    ) -> Result<Self> {
        Self::with_shared_resources_and_missing(
            project,
            path,
            workspace_root,
            shared_resource_root,
            Vec::new(),
        )
    }

    pub fn with_shared_resources_and_missing(
        project: Project,
        path: Option<PathBuf>,
        workspace_root: PathBuf,
        shared_resource_root: PathBuf,
        retained_missing: Vec<String>,
    ) -> Result<Self> {
        let mut store = Self::new(project, path, workspace_root);
        store.shared_resource_root = Some(shared_resource_root);
        if store
            .resource_root
            .file_name()
            .is_some_and(|name| name == "active")
        {
            store._active_lock = Some(acquire_active_lock(&store.resource_root)?);
        }
        // The empty list is meaningful: current host resources are present. Missing entries are
        // retained in the package manifest so the editor can still save a cross-target draft.
        let current_project = {
            let inner = store.inner.try_lock().map_err(|_| "工作区初始化状态冲突")?;
            serde_json::from_value::<Project>(inner.project.clone()).map_err(|e| e.to_string())?
        };
        let missing = stage_project_dependencies(
            &current_project,
            &store.resource_root,
            store.shared_resource_root.as_ref().unwrap(),
        )?;
        let mut combined = missing;
        let current_missing: HashSet<String> = combined.iter().cloned().collect();
        let retained_from_recovery: Vec<String> = store
            .inner
            .try_lock()
            .map_err(|_| "工作区初始化状态冲突")?
            .missing_dependencies
            .iter()
            .filter(|item| !is_known_dependency(item) || current_missing.contains(*item))
            .cloned()
            .collect();
        combined.extend(retained_from_recovery);
        combined.extend(
            retained_missing
                .into_iter()
                .filter(|item| !is_known_dependency(item) || current_missing.contains(item)),
        );
        combined.sort();
        combined.dedup();
        store
            .inner
            .try_lock()
            .map_err(|_| "工作区初始化状态冲突")?
            .missing_dependencies = combined;
        {
            let inner = store.inner.try_lock().map_err(|_| "工作区初始化状态冲突")?;
            persist_recovery(&store.recovery_file, &inner)?;
        }
        Ok(store)
    }

    pub fn root(&self) -> PathBuf {
        self.resource_root.clone()
    }

    pub async fn snapshot(&self) -> Project {
        let inner = self.inner.lock().await;
        serde_json::from_value(inner.project.clone())
            .expect("canonical workspace remains a Project")
    }

    pub async fn snapshot_with_revision(&self) -> (Project, u64) {
        let inner = self.inner.lock().await;
        (
            serde_json::from_value(inner.project.clone())
                .expect("canonical workspace remains a Project"),
            inner.revision,
        )
    }

    pub async fn state(&self, window_id: Option<&str>) -> WorkspaceState {
        let inner = self.inner.lock().await;
        make_state(&inner, window_id)
    }

    async fn mark_saved_snapshot(
        &self,
        workspace_id: &str,
        revision: u64,
        snapshot: &Value,
        path: PathBuf,
    ) -> Result<WorkspaceState> {
        let mut inner = self.inner.lock().await;
        if inner.workspace_id == workspace_id {
            inner.path = Some(path);
            inner.saved_revision = revision;
            inner.saved_fingerprint = fingerprint(snapshot);
        }
        persist_recovery(&self.recovery_file, &inner)?;
        Ok(make_state(&inner, None))
    }

    async fn replace(
        &self,
        project: Project,
        path: Option<PathBuf>,
        saved: bool,
        backup_existing: bool,
    ) -> Result<WorkspaceState> {
        if backup_existing && self.state(None).await.dirty {
            self.backup_recovery().await?;
        }
        let original_value = serde_json::to_value(&project).expect("Project serializes to JSON");
        let (project, migrated) = migrate_project(project);
        let missing = if let Some(shared) = &self.shared_resource_root {
            stage_project_dependencies(&project, &self.resource_root, shared)?
        } else {
            Vec::new()
        };
        let value = serde_json::to_value(project).expect("Project serializes to JSON");
        let mut inner = self.inner.lock().await;
        inner.workspace_id = uuid::Uuid::new_v4().to_string();
        inner.project = value.clone();
        inner.revision += 1;
        inner.saved_revision = if saved && !migrated {
            inner.revision
        } else {
            0
        };
        inner.saved_fingerprint = if saved && migrated {
            fingerprint(&original_value)
        } else if saved {
            fingerprint(&value)
        } else {
            String::new()
        };
        inner.path = path;
        inner.histories.clear();
        inner.applied_ops.clear();
        inner.missing_dependencies = missing;
        inner.recovered = false;
        persist_recovery(&self.recovery_file, &inner)?;
        Ok(make_state(&inner, None))
    }

    async fn backup_recovery(&self) -> Result<()> {
        let inner = self.inner.lock().await;
        let directory = self
            .resource_root
            .parent()
            .ok_or("工作目录无效")?
            .join("recovery-backups");
        fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let base = format!("{}-{}", inner.workspace_id, uuid::Uuid::new_v4());
        let recovery_image = RecoveryImage::from(&*inner);
        let recovery_path = directory.join(format!("{base}.json"));
        write_json_atomic(&recovery_path, &recovery_image)?;
        let project: Project =
            serde_json::from_value(inner.project.clone()).map_err(|e| e.to_string())?;
        let package_path = directory.join(format!("{base}.hmiproj"));
        project_package::write_package_with_missing(
            &package_path,
            &project,
            &self.resource_root,
            inner.missing_dependencies.clone(),
        )?;
        Ok(())
    }
}

fn migrate_project(mut project: Project) -> (Project, bool) {
    let mut migrated = project.schema_version < 2;
    for page in &mut project.pages {
        if page.layout.is_none() {
            page.layout = Some(hmi_core::LayoutSpec {
                kind: "absolute".into(),
                gap: None,
                padding: None,
                columns: None,
            });
            migrated = true;
        }
        if page.responsive.is_none() {
            page.responsive = Some(json!(false));
            migrated = true;
        }
        if page.guides.is_empty() {
            page.guides = Vec::new();
        }
    }
    if project.i18n.is_none() {
        project.i18n = Some(hmi_core::I18nConfig {
            default_locale: "zh-CN".into(),
            locales: vec!["zh-CN".into(), "en-US".into()],
            messages: BTreeMap::from([
                ("zh-CN".into(), BTreeMap::new()),
                ("en-US".into(), BTreeMap::new()),
            ]),
        });
        migrated = true;
    }
    if let Some(i18n) = project.i18n.as_mut() {
        if i18n.default_locale.is_empty() {
            i18n.default_locale = "zh-CN".into();
            migrated = true;
        }
        if i18n.locales.is_empty() {
            i18n.locales = vec![i18n.default_locale.clone(), "en-US".into()];
            i18n.locales.sort();
            i18n.locales.dedup();
            migrated = true;
        }
        for locale in &i18n.locales {
            if !i18n.messages.contains_key(locale) {
                i18n.messages.insert(locale.clone(), BTreeMap::new());
                migrated = true;
            }
        }
    }
    if project.actions.is_empty() {
        project.actions = Vec::new();
    }
    if project.schema_version < 2 {
        project.schema_version = 2;
        migrated = true;
    }
    (project, migrated)
}

fn fingerprint(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).expect("JSON value serializes"))
    )
}

fn make_state(inner: &Inner, window_id: Option<&str>) -> WorkspaceState {
    let histories: BTreeMap<String, HistoryState> = inner
        .histories
        .iter()
        .map(|(id, history)| {
            (
                id.clone(),
                HistoryState {
                    can_undo: !history.undo.is_empty(),
                    can_redo: !history.redo.is_empty(),
                },
            )
        })
        .collect();
    let selected = window_id.and_then(|id| inner.histories.get(id));
    WorkspaceState {
        workspace_id: inner.workspace_id.clone(),
        project: inner.project.clone(),
        revision: inner.revision,
        saved_revision: inner.saved_revision,
        dirty: fingerprint(&inner.project) != inner.saved_fingerprint,
        path: inner
            .path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        can_undo: selected.is_some_and(|h| !h.undo.is_empty()),
        can_redo: selected.is_some_and(|h| !h.redo.is_empty()),
        histories,
        missing_dependencies: inner.missing_dependencies.clone(),
        recovered: inner.recovered,
    }
}

fn object_id(value: &Value, id: &str) -> bool {
    value.get("id").and_then(Value::as_str) == Some(id)
}

fn same_value(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, value)| b.get(key).is_some_and(|other| same_value(value, other)))
        }
        _ => left == right,
    }
}

/// Resolve object keys normally and array entries by stable `id`, never by a moving array index.
fn lookup<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = root;
    for segment in path {
        current = if let Some(value) = current.get(segment) {
            value
        } else if let Some(array) = current.as_array() {
            array.iter().find(|value| object_id(value, segment))?
        } else {
            return None;
        };
    }
    Some(current)
}

fn lookup_parent_mut<'a>(root: &'a mut Value, path: &[String]) -> Result<(&'a mut Value, String)> {
    if path.is_empty() {
        return Err("操作不能整体替换工程根对象".into());
    }
    let (last, parent_path) = path.split_last().unwrap();
    let mut current = root;
    for segment in parent_path {
        if current.is_object() {
            current = current
                .as_object_mut()
                .and_then(|object| object.get_mut(segment))
                .ok_or_else(|| format!("操作路径不存在: {segment}"))?;
        } else if current.is_array() {
            current = current
                .as_array_mut()
                .and_then(|array| array.iter_mut().find(|value| object_id(value, segment)))
                .ok_or_else(|| format!("操作路径中的对象 ID 不存在: {segment}"))?;
        } else {
            return Err(format!("操作路径不存在: {segment}"));
        }
    }
    Ok((current, last.clone()))
}

fn read_at(root: &Value, path: &[String]) -> Option<Value> {
    if path.last().is_some_and(|v| v == "$order") {
        let parent = lookup(root, &path[..path.len() - 1])?;
        return Some(Value::Array(
            parent
                .as_array()?
                .iter()
                .map(|v| v.get("id").cloned().unwrap_or(Value::Null))
                .collect(),
        ));
    }
    lookup(root, path).cloned()
}

fn apply_one(root: &mut Value, operation: &WorkspaceOperation) -> Result<()> {
    if operation.path.last().is_some_and(|v| v == "$order") {
        let parent_path = &operation.path[..operation.path.len() - 1];
        let parent = lookup_mut(root, parent_path)?;
        let array = parent.as_array_mut().ok_or("$order 只能用于数组")?;
        let ordered_ids: Vec<String> = operation
            .after
            .as_array()
            .ok_or("$order after 必须是 ID 数组")?
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "$order ID 必须是字符串".to_string())
            })
            .collect::<Result<_>>()?;
        if ordered_ids.len() != array.len() {
            return Err("重排不能丢失或新增数组项".into());
        }
        let mut reordered = Vec::with_capacity(array.len());
        for id in ordered_ids {
            let index = array
                .iter()
                .position(|value| object_id(value, &id))
                .ok_or(format!("重排目标不存在: {id}"))?;
            reordered.push(array.remove(index));
        }
        *array = reordered;
        return Ok(());
    }
    let (parent, key) = lookup_parent_mut(root, &operation.path)?;
    if let Some(array) = parent.as_array_mut() {
        if let Some(index) = array.iter().position(|value| object_id(value, &key)) {
            if operation.after_exists {
                array[index] = operation.after.clone();
            } else {
                array.remove(index);
            }
        } else if operation.after_exists {
            let value = operation.after.clone();
            if !object_id(&value, &key) {
                return Err("新增数组对象的 id 必须与路径一致".into());
            }
            let index = operation.index.unwrap_or(array.len()).min(array.len());
            array.insert(index, value);
        } else {
            return Err("删除目标数组对象不存在".into());
        }
    } else if let Some(object) = parent.as_object_mut() {
        if operation.after_exists {
            object.insert(key, operation.after.clone());
        } else if object.remove(&key).is_none() {
            return Err("删除目标字段不存在".into());
        }
    } else {
        return Err("操作路径的父级不是对象或数组".into());
    }
    Ok(())
}

fn lookup_mut<'a>(root: &'a mut Value, path: &[String]) -> Result<&'a mut Value> {
    let mut current = root;
    for segment in path {
        if current.is_object() {
            current = current
                .as_object_mut()
                .and_then(|object| object.get_mut(segment))
                .ok_or_else(|| format!("操作路径不存在: {segment}"))?;
        } else if current.is_array() {
            current = current
                .as_array_mut()
                .and_then(|array| array.iter_mut().find(|value| object_id(value, segment)))
                .ok_or_else(|| format!("操作路径中的对象 ID 不存在: {segment}"))?;
        } else {
            return Err(format!("操作路径不存在: {segment}"));
        }
    }
    Ok(current)
}

fn inverse(ops: &[WorkspaceOperation]) -> Vec<WorkspaceOperation> {
    ops.iter()
        .rev()
        .map(|op| WorkspaceOperation {
            path: op.path.clone(),
            before_exists: op.after_exists,
            before: op.after.clone(),
            after_exists: op.before_exists,
            after: op.before.clone(),
            index: op.index,
        })
        .collect()
}

fn validate_operation(root: &Value, op: &WorkspaceOperation) -> Result<()> {
    if op.path.is_empty() || op.path.iter().any(|s| s.is_empty()) {
        return Err("操作路径无效".into());
    }
    if op.path.last().is_some_and(|s| s == "$order") {
        let actual = read_at(root, &op.path);
        if op.before_exists != actual.is_some()
            || actual.as_ref().is_some_and(|v| !same_value(v, &op.before))
        {
            return Err("数组已被其他窗口修改，无法应用排序".into());
        }
        return Ok(());
    }
    let actual = lookup(root, &op.path);
    if op.before_exists != actual.is_some() || actual.is_some_and(|v| !same_value(v, &op.before)) {
        return Err(format!(
            "字段已被其他窗口修改，操作冲突: {}",
            op.path.join(".")
        ));
    }
    Ok(())
}

#[cfg(test)]
async fn apply_batch(
    store: &WorkspaceStore,
    window_id: &str,
    base_revision: u64,
    op_id: &str,
    operations: Vec<WorkspaceOperation>,
    record_history: bool,
) -> Result<WorkspaceState> {
    apply_batch_guarded(
        store,
        None,
        window_id,
        base_revision,
        op_id,
        operations,
        record_history,
    )
    .await
}

async fn apply_batch_guarded(
    store: &WorkspaceStore,
    expected_workspace_id: Option<&str>,
    window_id: &str,
    base_revision: u64,
    op_id: &str,
    operations: Vec<WorkspaceOperation>,
    record_history: bool,
) -> Result<WorkspaceState> {
    if window_id.is_empty() || window_id.len() > 128 || op_id.is_empty() || op_id.len() > 128 {
        return Err("窗口或操作 ID 无效".into());
    }
    if operations.is_empty() {
        return Err("操作列表不能为空".into());
    }
    let mut inner = store.inner.lock().await;
    if expected_workspace_id.is_some_and(|expected| inner.workspace_id != expected) {
        return Err("工程已切换，拒绝应用旧窗口操作".into());
    }
    let applied_key = format!("{window_id}:{op_id}");
    if inner.applied_ops.contains(&applied_key) {
        return Ok(make_state(&inner, Some(window_id)));
    }
    if base_revision > inner.revision {
        return Err("基础修订号来自未来".into());
    }
    let mut candidate = inner.project.clone();
    for operation in &operations {
        validate_operation(&candidate, operation)?;
        apply_one(&mut candidate, operation)?;
    }
    let project: Project =
        serde_json::from_value(candidate.clone()).map_err(|e| format!("项目结构无效: {e}"))?;
    if ![1, 2].contains(&project.schema_version) {
        return Err("不支持此项目格式版本".into());
    }
    inner.project = candidate;
    inner.revision += 1;
    inner.applied_ops.insert(applied_key);
    if record_history {
        let history = inner.histories.entry(window_id.to_owned()).or_default();
        history.undo.push(operations);
        history.redo.clear();
    }
    inner.recovered = false;
    persist_recovery(&store.recovery_file, &inner)?;
    Ok(make_state(&inner, Some(window_id)))
}

#[tauri::command]
pub async fn workspace_state(
    window_id: Option<String>,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    Ok(store.state(window_id.as_deref()).await)
}

#[tauri::command]
pub async fn apply_workspace_operations(
    app: tauri::AppHandle,
    window_id: String,
    workspace_id: Option<String>,
    base_revision: u64,
    op_id: String,
    operations: Vec<WorkspaceOperation>,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let state = apply_batch_guarded(
        &store,
        workspace_id.as_deref(),
        &window_id,
        base_revision,
        &op_id,
        operations,
        true,
    )
    .await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

#[tauri::command]
pub async fn undo_workspace(
    app: tauri::AppHandle,
    window_id: String,
    workspace_id: Option<String>,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let state = change_history(&store, &window_id, workspace_id.as_deref(), true).await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

#[tauri::command]
pub async fn redo_workspace(
    app: tauri::AppHandle,
    window_id: String,
    workspace_id: Option<String>,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let state = change_history(&store, &window_id, workspace_id.as_deref(), false).await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

async fn change_history(
    store: &WorkspaceStore,
    window_id: &str,
    expected_workspace_id: Option<&str>,
    undo: bool,
) -> Result<WorkspaceState> {
    let (ops, revision) = {
        let mut inner = store.inner.lock().await;
        if expected_workspace_id.is_some_and(|expected| inner.workspace_id != expected) {
            return Err("工程已切换，拒绝应用旧窗口历史".into());
        }
        let history = inner
            .histories
            .get_mut(window_id)
            .ok_or("此窗口没有可用的编辑历史")?;
        let ops = if undo {
            history.undo.pop()
        } else {
            history.redo.pop()
        }
        .ok_or(if undo {
            "没有可撤销的操作"
        } else {
            "没有可重做的操作"
        })?;
        (ops, inner.revision)
    };
    let id = format!(
        "history:{}:{}",
        if undo { "undo" } else { "redo" },
        uuid::Uuid::new_v4()
    );
    let to_apply = if undo { inverse(&ops) } else { ops.clone() };
    let state = apply_batch_guarded(
        store,
        expected_workspace_id,
        window_id,
        revision,
        &id,
        to_apply,
        false,
    )
    .await;
    match state {
        Ok(_state) => {
            let mut inner = store.inner.lock().await;
            let history = inner.histories.entry(window_id.to_owned()).or_default();
            if undo {
                history.redo.push(ops);
            } else {
                history.undo.push(ops);
            }
            inner.recovered = false;
            persist_recovery(&store.recovery_file, &inner)?;
            Ok(make_state(&inner, Some(window_id)))
        }
        Err(error) => {
            let mut inner = store.inner.lock().await;
            if expected_workspace_id.map_or(true, |expected| inner.workspace_id == expected) {
                let history = inner.histories.entry(window_id.to_owned()).or_default();
                if undo {
                    history.undo.push(ops);
                } else {
                    history.redo.push(ops);
                }
            }
            Err(error)
        }
    }
}

#[tauri::command]
pub async fn replace_workspace(
    app: tauri::AppHandle,
    project: Value,
    path: Option<String>,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let project: Project =
        serde_json::from_value(project).map_err(|e| format!("项目结构无效: {e}"))?;
    if ![1, 2].contains(&project.schema_version) {
        return Err("不支持此项目格式版本".into());
    }
    let path = path.map(PathBuf::from);
    let state = store
        .replace(project, path.clone(), path.is_some(), true)
        .await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

#[tauri::command]
pub async fn open_workspace_package(
    app: tauri::AppHandle,
    path: String,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if store.state(None).await.dirty {
        store.backup_recovery().await?;
    }
    let (project, manifest) = project_package::read_package(&path, &store.resource_root)?;
    store.replace(project, Some(path), true, false).await?;
    {
        let mut inner = store.inner.lock().await;
        let current_missing: HashSet<String> = inner.missing_dependencies.iter().cloned().collect();
        inner.missing_dependencies.extend(
            manifest
                .missing_dependencies
                .into_iter()
                .filter(|item| !is_known_dependency(item) || current_missing.contains(item)),
        );
        inner.missing_dependencies.sort();
        inner.missing_dependencies.dedup();
        inner.recovered = false;
        persist_recovery(&store.recovery_file, &inner)?;
    }
    let state = store.state(None).await;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

#[tauri::command]
pub async fn save_workspace_package(
    app: tauri::AppHandle,
    path: String,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let path = PathBuf::from(path);
    let (project, revision, snapshot, root, missing_dependencies, saved_workspace_id) = {
        let inner = store.inner.lock().await;
        let project: Project =
            serde_json::from_value(inner.project.clone()).map_err(|e| e.to_string())?;
        (
            project,
            inner.revision,
            inner.project.clone(),
            store.resource_root.clone(),
            inner.missing_dependencies.clone(),
            inner.workspace_id.clone(),
        )
    };
    project_package::write_package_with_missing(&path, &project, &root, missing_dependencies)?;
    let state = store
        .mark_saved_snapshot(&saved_workspace_id, revision, &snapshot, path)
        .await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

#[tauri::command]
pub async fn import_legacy_project(
    app: tauri::AppHandle,
    path: String,
    store: State<'_, Arc<WorkspaceStore>>,
) -> Result<WorkspaceState> {
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    if path.extension().and_then(|v| v.to_str()) != Some("json") {
        return Err("旧版项目必须是 JSON 文件".into());
    }
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let project: Project =
        serde_json::from_slice(&bytes).map_err(|e| format!("旧项目格式无效: {e}"))?;
    if store.state(None).await.dirty {
        store.backup_recovery().await?;
    }
    let root = &store.resource_root;
    fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let sibling_assets = path.parent().unwrap_or(Path::new(".")).join("assets");
    if sibling_assets.is_dir() {
        copy_tree_safe(&sibling_assets, &root.join("assets"))?;
    }
    let state = store.replace(project, None, false, false).await?;
    let _ = app.emit("workspace-changed", &state);
    Ok(state)
}

fn copy_tree_safe(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            return Err("导入的旧项目资源不能包含符号链接".into());
        }
        let target = destination.join(entry.file_name());
        if ty.is_dir() {
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            copy_tree_safe(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::create_dir_all(destination).map_err(|e| e.to_string())?;
            fs::copy(entry.path(), target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn stage_project_dependencies(
    project: &Project,
    workspace_root: &Path,
    shared_root: &Path,
) -> Result<Vec<String>> {
    fs::create_dir_all(workspace_root).map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    let target = hmi_core::host_target();
    let mut plugin_ids = HashSet::new();
    for connection in &project.connections {
        if !plugin_ids.insert(connection.protocol_id.as_str()) {
            continue;
        }
        if !hmi_core::safe_identifier(&connection.protocol_id) {
            missing.push(format!("plugin:{}", connection.protocol_id));
            continue;
        }
        let target_source = shared_root
            .join("plugins")
            .join(target)
            .join(&connection.protocol_id);
        let legacy_source = shared_root.join("plugins").join(&connection.protocol_id);
        let source = if target_source.is_dir() {
            target_source
        } else {
            legacy_source
        };
        let destination = workspace_root
            .join("plugins")
            .join(target)
            .join(&connection.protocol_id);
        if destination.is_dir() {
            continue;
        }
        if source.is_dir() {
            copy_tree_safe(&source, &destination)?;
        } else {
            missing.push(format!("plugin:{}", connection.protocol_id));
        }
    }
    let requires_python = project
        .flows
        .iter()
        .any(|flow| flow.nodes.iter().any(|node| node.kind == "python"));
    if requires_python {
        if let Some(python) = &project.python {
            if !hmi_core::safe_identifier(&python.runtime_id) {
                missing.push(format!("python:{}", python.runtime_id));
                return Ok(missing);
            }
            let target_project = shared_root
                .join("python")
                .join(target)
                .join("projects")
                .join(&project.id);
            let legacy_project = shared_root.join("python/projects").join(&project.id);
            let target_runtime = shared_root
                .join("python")
                .join(target)
                .join(&python.runtime_id);
            let legacy_runtime = shared_root.join("python").join(&python.runtime_id);
            let target_project_destination = workspace_root
                .join("python")
                .join(target)
                .join("projects")
                .join(&project.id);
            let target_runtime_destination = workspace_root
                .join("python")
                .join(target)
                .join(&python.runtime_id);
            let project_source = if target_project.is_dir() {
                target_project
            } else {
                legacy_project
            };
            let runtime_source = if target_runtime.is_dir() {
                target_runtime
            } else {
                legacy_runtime
            };
            let mut available =
                target_project_destination.is_dir() || target_runtime_destination.is_dir();
            if project_source.is_dir() {
                available = true;
                if !target_project_destination.is_dir() {
                    copy_tree_safe(&project_source, &target_project_destination)?;
                }
            }
            if runtime_source.is_dir() {
                available = true;
                if !target_runtime_destination.is_dir() {
                    copy_tree_safe(&runtime_source, &target_runtime_destination)?;
                }
            }
            if !available {
                missing.push(format!("python:{}", python.runtime_id));
            }
        } else {
            missing.push("python:runtime-not-configured".into());
        }
    }
    missing.sort();
    missing.dedup();
    Ok(missing)
}

fn is_known_dependency(item: &str) -> bool {
    item.starts_with("plugin:") || item.starts_with("python:")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Project {
        serde_json::from_value(json!({"schemaVersion":1,"id":"p","name":"P","width":800,"height":600,"pages":[{"id":"page","name":"Page","widgets":[]}]})).unwrap()
    }
    fn op(path: &[&str], before: Value, after: Value) -> WorkspaceOperation {
        WorkspaceOperation {
            path: path.iter().map(|v| (*v).into()).collect(),
            before_exists: true,
            before,
            after_exists: true,
            after,
            index: None,
        }
    }
    #[tokio::test]
    async fn v1_projects_migrate_in_memory_and_remain_dirty_until_package_save() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(
            fixture(),
            Some(dir.path().join("legacy.json")),
            dir.path().join("workspace"),
        );
        let state = store.state(Some("design")).await;
        assert_eq!(state.project["schemaVersion"], 2);
        assert_eq!(state.project["pages"][0]["layout"]["type"], "absolute");
        assert_eq!(state.project["pages"][0]["responsive"], false);
        assert_eq!(state.project["i18n"]["defaultLocale"], "zh-CN");
        assert_eq!(state.project["i18n"]["locales"], json!(["zh-CN", "en-US"]));
        assert!(state.dirty);
    }
    #[tokio::test]
    async fn recovery_restores_project_revision_and_independent_history() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("active");
        let store = WorkspaceStore::new(fixture(), None, root.clone());
        apply_batch(
            &store,
            "design",
            0,
            "edit",
            vec![op(&["name"], json!("P"), json!("Draft"))],
            true,
        )
        .await
        .unwrap();
        drop(store);

        let reopened = WorkspaceStore::new(fixture(), None, root);
        let state = reopened.state(Some("design")).await;
        assert!(state.recovered);
        assert_eq!(state.project["name"], "Draft");
        assert_eq!(state.revision, 1);
        assert!(state.can_undo);
        assert!(state.dirty);
    }
    #[test]
    fn active_workspace_lock_rejects_a_second_live_studio_instance() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("active");
        let shared = dir.path().join("shared");
        let first =
            WorkspaceStore::with_shared_resources(fixture(), None, root.clone(), shared.clone())
                .unwrap();
        assert!(WorkspaceStore::with_shared_resources(
            fixture(),
            None,
            root.clone(),
            shared.clone()
        )
        .is_err());
        drop(first);
        assert!(WorkspaceStore::with_shared_resources(fixture(), None, root, shared).is_ok());
    }
    #[test]
    fn staging_host_plugin_keeps_foreign_target_variant_in_engineering_package() {
        let dir = tempfile::tempdir().unwrap();
        let target = hmi_core::host_target();
        let foreign_target = if target == "x86_64-pc-windows-msvc" {
            "aarch64-apple-darwin"
        } else {
            "x86_64-pc-windows-msvc"
        };
        let shared = dir.path().join("shared");
        let workspace = dir.path().join("workspace");
        for (root, platform, content) in [
            (&shared, target, b"host-plugin".as_slice()),
            (&workspace, foreign_target, b"foreign-plugin".as_slice()),
        ] {
            let path = root.join("plugins").join(platform).join("demo-protocol");
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("plugin.bin"), content).unwrap();
        }
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"p","name":"P","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[]}],
            "connections":[{"id":"device","name":"Device","port":"COM1","baudRate":9600,"dataBits":8,"stopBits":1,"parity":"none","protocolId":"demo-protocol"}]
        })).unwrap();
        assert!(stage_project_dependencies(&project, &workspace, &shared)
            .unwrap()
            .is_empty());
        let archive = dir.path().join("p.hmiproj");
        project_package::write_package(&archive, &project, &workspace).unwrap();
        let reopened = dir.path().join("reopened");
        project_package::read_package(&archive, &reopened).unwrap();
        assert_eq!(
            fs::read(
                reopened
                    .join("plugins")
                    .join(target)
                    .join("demo-protocol/plugin.bin")
            )
            .unwrap(),
            b"host-plugin"
        );
        assert_eq!(
            fs::read(
                reopened
                    .join("plugins")
                    .join(foreign_target)
                    .join("demo-protocol/plugin.bin")
            )
            .unwrap(),
            b"foreign-plugin"
        );
    }
    #[test]
    fn existing_target_project_python_environment_satisfies_dependency_staging() {
        let dir = tempfile::tempdir().unwrap();
        let target = hmi_core::host_target();
        let shared = dir.path().join("shared");
        let workspace = dir.path().join("workspace");
        let project_python = workspace.join("python").join(target).join("projects/p");
        fs::create_dir_all(&project_python).unwrap();
        fs::write(project_python.join("python.exe"), b"python-runtime").unwrap();
        let project: Project = serde_json::from_value(json!({
            "schemaVersion":1,"id":"p","name":"P","width":800,"height":600,
            "pages":[{"id":"page","name":"Page","widgets":[]}],
            "flows":[{"id":"f","name":"Python","enabled":true,"timeoutMs":1000,"nodes":[{"id":"py","kind":"python","label":"Python","position":{"x":0,"y":0},"config":{"code":"def transform(msg): return msg"}}],"edges":[]}],
            "python":{"runtimeId":"3.14.8-test"}
        })).unwrap();
        assert!(stage_project_dependencies(&project, &workspace, &shared)
            .unwrap()
            .is_empty());
    }
    #[tokio::test]
    async fn saving_revision_n_keeps_later_edits_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(fixture(), None, dir.path().join("workspace"));
        let (snapshot, revision) = store.snapshot_with_revision().await;
        let snapshot_value = serde_json::to_value(snapshot).unwrap();
        let workspace_id = store.state(None).await.workspace_id;
        apply_batch(
            &store,
            "design",
            0,
            "after-snapshot",
            vec![op(&["name"], json!("P"), json!("Edited during save"))],
            true,
        )
        .await
        .unwrap();
        let saved = store
            .mark_saved_snapshot(
                &workspace_id,
                revision,
                &snapshot_value,
                dir.path().join("project.hmiproj"),
            )
            .await
            .unwrap();
        assert!(saved.dirty);
        assert_eq!(saved.saved_revision, 0);
        assert_eq!(saved.revision, 1);
    }
    #[tokio::test]
    async fn disjoint_window_edits_merge_and_same_field_conflicts_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(WorkspaceStore::new(
            fixture(),
            None,
            dir.path().join("workspace"),
        ));
        apply_batch(
            &store,
            "design",
            0,
            "1",
            vec![op(&["name"], json!("P"), json!("Changed"))],
            true,
        )
        .await
        .unwrap();
        apply_batch(
            &store,
            "logic",
            0,
            "2",
            vec![op(&["width"], json!(800), json!(900))],
            true,
        )
        .await
        .unwrap();
        let before = store.state(Some("logic")).await;
        assert!(apply_batch(
            &store,
            "logic",
            0,
            "3",
            vec![op(&["name"], json!("P"), json!("Conflict"))],
            true
        )
        .await
        .is_err());
        assert_eq!(store.state(Some("logic")).await.revision, before.revision);
        assert_eq!(store.snapshot().await.width, 900.0);
        assert!(apply_batch(
            &store,
            "design",
            1,
            "atomic",
            vec![
                op(&["height"], json!(600), json!(700)),
                op(&["name"], json!("P"), json!("stale")),
            ],
            true,
        )
        .await
        .is_err());
        assert_eq!(store.snapshot().await.height, 600.0);
    }
    #[tokio::test]
    async fn stale_workspace_session_is_rejected_even_when_revision_matches() {
        let dir = tempfile::tempdir().unwrap();
        let store = WorkspaceStore::new(fixture(), None, dir.path().join("workspace"));
        let old_workspace = store.state(None).await.workspace_id;
        store.inner.lock().await.workspace_id = "new-session".into();
        let result = apply_batch_guarded(
            &store,
            Some(&old_workspace),
            "design",
            0,
            "stale",
            vec![op(&["name"], json!("P"), json!("stale write"))],
            true,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(store.snapshot().await.name, "P");
        assert_eq!(store.state(None).await.revision, 0);
    }
    #[tokio::test]
    async fn undo_is_window_local_and_remote_edits_keep_redo() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(WorkspaceStore::new(
            fixture(),
            Some(dir.path().join("saved.hmiproj")),
            dir.path().join("workspace"),
        ));
        {
            let mut inner = store.inner.lock().await;
            inner.saved_fingerprint = fingerprint(&inner.project);
            inner.saved_revision = inner.revision;
        }
        apply_batch(
            &store,
            "design",
            0,
            "1",
            vec![op(&["name"], json!("P"), json!("Q"))],
            true,
        )
        .await
        .unwrap();
        change_history(&store, "design", None, true).await.unwrap();
        assert!(!store.state(Some("design")).await.dirty);
        apply_batch(
            &store,
            "logic",
            1,
            "2",
            vec![op(&["width"], json!(800), json!(900))],
            true,
        )
        .await
        .unwrap();
        assert!(store.state(Some("design")).await.can_redo);
        change_history(&store, "design", None, false).await.unwrap();
        let project = store.snapshot().await;
        assert_eq!(project.name, "Q");
        assert_eq!(project.width, 900.0);
    }
}
