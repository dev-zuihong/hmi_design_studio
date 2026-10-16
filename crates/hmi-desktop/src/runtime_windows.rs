//! Project-defined runtime windows share one session and keep their input drafts separate.
use crate::{
    resources::{error, Result},
    DesktopState,
};
use hmi_core::Project;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tauri::Manager;
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowInstance {
    pub id: String,
    pub window_id: String,
    pub page_id: String,
    pub context: Value,
    pub inputs: serde_json::Map<String, Value>,
}
#[derive(Default)]
pub struct RuntimeWindows {
    instances: Mutex<HashMap<String, WindowInstance>>,
}
impl RuntimeWindows {
    pub async fn get(&self, label: &str) -> Option<WindowInstance> {
        self.instances.lock().await.get(label).cloned()
    }
    pub async fn set_input(&self, label: &str, id: String, value: Value) -> Result<()> {
        let mut instances = self.instances.lock().await;
        instances
            .get_mut(label)
            .ok_or("运行窗口已关闭")?
            .inputs
            .insert(id, value);
        Ok(())
    }
    pub async fn remove(&self, label: &str) {
        self.instances.lock().await.remove(label);
    }
    pub async fn close_all(&self, app: &tauri::AppHandle) {
        let labels: Vec<_> = self
            .instances
            .lock()
            .await
            .drain()
            .map(|(key, _)| key)
            .collect();
        for label in labels {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    }
    pub async fn reconcile_devices(
        &self,
        app: &tauri::AppHandle,
        backend_id: &str,
        snapshot: &Value,
    ) {
        let stale: Vec<_> = self
            .instances
            .lock()
            .await
            .iter()
            .filter(|(_, instance)| {
                instance.context["backendId"].as_str() == Some(backend_id)
                    && device_context_stale(&instance.context, snapshot)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for label in stale {
            self.remove(&label).await;
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    }
    pub async fn reconcile_project(&self, app: &tauri::AppHandle, project: &Project) {
        let stale: Vec<_> = self
            .instances
            .lock()
            .await
            .iter()
            .filter(|(_, instance)| {
                !project.windows.iter().any(|template| {
                    template.id == instance.window_id && template.page_id == instance.page_id
                }) || !project.pages.iter().any(|page| page.id == instance.page_id)
                    || instance.context["backendId"]
                        .as_str()
                        .is_some_and(|id| !project.backends.iter().any(|backend| backend.id == id))
                    || instance.context["transportId"].as_str().is_some_and(|id| {
                        !project
                            .transports
                            .iter()
                            .any(|transport| transport.id == id)
                    })
            })
            .map(|(label, _)| label.clone())
            .collect();
        for label in stale {
            self.remove(&label).await;
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    }
    pub async fn reconcile_transport(&self, app: &tauri::AppHandle, snapshot: &Value) {
        let Some(id) = snapshot["transportId"].as_str() else {
            return;
        };
        let stale: Vec<_> = self
            .instances
            .lock()
            .await
            .iter()
            .filter(|(_, instance)| {
                instance.context["transportId"] == id
                    && !snapshot["devices"].as_array().is_some_and(|devices| {
                        devices.iter().any(|device| {
                            device["deviceKey"] == instance.context["deviceKey"]
                                && device["present"] == true
                                && device["deviceGeneration"]
                                    == instance.context["deviceGeneration"]
                        })
                    })
            })
            .map(|(label, _)| label.clone())
            .collect();
        for label in stale {
            self.remove(&label).await;
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.destroy();
            }
        }
    }
    pub async fn action(
        &self,
        app: &tauri::AppHandle,
        project: &Project,
        kind: &str,
        args: Value,
        context: Value,
    ) -> Result<Value> {
        let mut next_context = context.as_object().cloned().unwrap_or_default();
        if let Some(values) = args.get("context").and_then(Value::as_object) {
            next_context.extend(values.clone());
        }
        for key in [
            "deviceKey",
            "backendId",
            "generation",
            "transportId",
            "deviceGeneration",
        ] {
            if let Some(value) = args.get(key) {
                next_context.insert(key.into(), value.clone());
            }
        }
        if kind == "window.close" {
            let label = args
                .get("windowInstanceId")
                .or_else(|| context.get("windowInstanceId"))
                .and_then(Value::as_str)
                .ok_or("缺少窗口实例")?;
            if let Some(window) = app.get_webview_window(label) {
                window.close().map_err(error)?;
            }
            self.remove(label).await;
            return Ok(Value::Null);
        }
        let id = args["windowId"].as_str().ok_or("缺少窗口模板 ID")?;
        let template = project
            .windows
            .iter()
            .find(|window| window.id == id)
            .ok_or("窗口模板不存在")?;
        let key_field = template.instance_key.as_deref().unwrap_or("deviceKey");
        let instance_key = format!(
            "{}:{}:{}:{}:{}:{}",
            id,
            next_context.get("backendId").unwrap_or(&Value::Null),
            next_context.get(key_field).unwrap_or(&Value::Null),
            next_context.get("generation").unwrap_or(&Value::Null),
            next_context.get("transportId").unwrap_or(&Value::Null),
            next_context.get("deviceGeneration").unwrap_or(&Value::Null)
        );
        use sha2::{Digest, Sha256};
        let hash = format!("{:x}", Sha256::digest(instance_key.as_bytes()));
        let label = format!("runtime-{}", &hash[..24]);
        if let Some(window) = app.get_webview_window(&label) {
            window.unminimize().map_err(error)?;
            window.show().map_err(error)?;
            window.set_focus().map_err(error)?;
            return Ok(json!({"windowInstanceId":label}));
        }
        if kind == "window.focus" {
            return Err("运行窗口尚未打开".into());
        }
        next_context.insert("windowInstanceId".into(), json!(label));
        let title = if let Some(key) = next_context.get("deviceKey").and_then(Value::as_str) {
            format!("{} · {}", template.title, key)
        } else {
            template.title.clone()
        };
        let instance = WindowInstance {
            id: label.clone(),
            window_id: id.into(),
            page_id: template.page_id.clone(),
            context: Value::Object(next_context),
            inputs: Default::default(),
        };
        self.instances.lock().await.insert(label.clone(), instance);
        let result = tauri::WebviewWindowBuilder::new(
            app,
            &label,
            tauri::WebviewUrl::App("runner.html?view=runtime-window".into()),
        )
        .title(title)
        .inner_size(template.width, template.height)
        .min_inner_size(
            template.min_width.unwrap_or(400.0),
            template.min_height.unwrap_or(300.0),
        )
        .build();
        match result {
            Ok(window) => {
                let app = app.clone();
                let closed_label = label.clone();
                window.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Destroyed) {
                        let app = app.clone();
                        let label = closed_label.clone();
                        tauri::async_runtime::spawn(async move {
                            let state = app.state::<DesktopState>();
                            state.runtime_windows.remove(&label).await;
                            if let Ok(session) = state.session_for(None).await {
                                session.runtime.clear_window(&label);
                            }
                        });
                    }
                });
                Ok(json!({"windowInstanceId":label}))
            }
            Err(reason) => {
                self.remove(&label).await;
                Err(error(reason))
            }
        }
    }
}

fn device_context_stale(context: &Value, snapshot: &Value) -> bool {
    if snapshot["providerStatus"] == "failed" || context["generation"] != snapshot["generation"] {
        return true;
    }
    context["deviceKey"].as_str().is_some_and(|key| {
        snapshot["devices"].as_array().is_some_and(|devices| {
            !devices.iter().any(|device| {
                device["deviceKey"]
                    .as_str()
                    .or_else(|| device["id"].as_str())
                    == Some(key)
                    && device["present"] != false
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_window_lives_through_recovery_but_closes_on_removal_or_provider_failure() {
        let context = json!({"deviceKey":"ptb1","generation":"g1"});
        let snapshot = json!({"generation":"g1","devices":[{"deviceKey":"ptb1","present":true,"state":"resetRequired"}]});
        assert!(!device_context_stale(&context, &snapshot));
        let mut unplugged = snapshot.clone();
        unplugged["devices"][0]["present"] = json!(false);
        assert!(device_context_stale(&context, &unplugged));
        let mut failed = snapshot.clone();
        failed["providerStatus"] = json!("failed");
        assert!(device_context_stale(&context, &failed));
        let mut next = snapshot.clone();
        next["generation"] = json!("g2");
        assert!(device_context_stale(&context, &next));
        assert!(device_context_stale(
            &context,
            &json!({"generation":"g1","devices":[]})
        ));
    }
}

#[tauri::command]
pub async fn get_runtime_window_state(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, DesktopState>,
) -> Result<Value> {
    let instance = state
        .runtime_windows
        .get(window.label())
        .await
        .ok_or("运行窗口不存在")?;
    let session = state.session_for(None).await?;
    Ok(
        json!({"instance":instance,"project":session.runtime.project(),"snapshot":session.runtime.snapshot().await,
        "generation":if state.preview_host_mode {Some(state.preview_generation.load(std::sync::atomic::Ordering::Acquire))} else {None},
        "revision":state.preview_revision.load(std::sync::atomic::Ordering::Acquire)}),
    )
}

#[tauri::command]
pub async fn update_runtime_window_input(
    window: tauri::WebviewWindow,
    id: String,
    value: Value,
    state: tauri::State<'_, DesktopState>,
) -> Result<()> {
    state
        .runtime_windows
        .set_input(window.label(), id, value)
        .await
}
