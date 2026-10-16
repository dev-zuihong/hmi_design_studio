//! CLI entry points use exactly the same validation, sessions and packager as the GUI.
use crate::{export, resources::*, session};
use hmi_core::Project;
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub async fn execute(args: Vec<String>) -> Result<Value> {
    let option = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let operation = args.first().map(String::as_str).unwrap_or("help");
    if operation == "help" {
        return Ok(
            json!({"usage":"hmi-cli <validate|smoke|export> --project project.json [--resources .cache] [--destination out.zip] [--flow flowId]"}),
        );
    }
    let path = PathBuf::from(option("--project").ok_or("--project is required")?)
        .canonicalize()
        .map_err(error)?;
    let project: Project = serde_json::from_value(read_json(&path)?).map_err(error)?;
    let resources = Resources {
        root: PathBuf::from(option("--resources").unwrap_or_else(|| ".cache".into()))
            .canonicalize()
            .map_err(error)?,
        runner: args.iter().any(|a| a == "--packaged"),
    };
    let issues = session::validate(&resources, &project).await;
    if operation == "validate" {
        return Ok(json!({"valid":issues.iter().all(|i|i.severity != "error"),"issues":issues}));
    }
    if let Some(issue) = issues.iter().find(|i| i.severity == "error") {
        return Err(format!("{}: {}", issue.path, issue.message));
    }
    if operation == "export" {
        let destination = option("--destination").ok_or("--destination is required")?;
        return serde_json::to_value(export::package(
            &resources,
            &project,
            path.parent().unwrap(),
            Path::new(&destination),
        )?)
        .map_err(error);
    }
    if operation != "smoke" {
        return Err("Unknown operation".into());
    }
    if project
        .connections
        .iter()
        .any(|c| c.port != "sim://temperature")
    {
        return Err("Automated smoke only allows the explicit simulation transport".into());
    }
    let session = session::Session::new(project.clone(), resources).await?;
    let result = async {
        session.runtime.start().await.map_err(error)?;
        let connections = session.connection_manager().await;
        for connection in &project.connections {
            connections.open(&connection.id).await.map_err(error)?;
        }
        let flow_id = option("--flow").unwrap_or_else(|| "flow_read".into());
        let mut events = session.runtime.subscribe();
        let run_id = session
            .runtime
            .trigger(&flow_id, None, json!({"payload":null}), json!({}))
            .await
            .map_err(error)?;
        let terminal = tokio::time::timeout(Duration::from_secs(35), async {
            loop {
                let event = events.recv().await.map_err(error)?;
                if event.run_id.as_deref() == Some(&run_id)
                    && ["run.completed", "run.failed", "run.cancelled"]
                        .contains(&event.kind.as_str())
                {
                    return Ok::<_, String>(event);
                }
            }
        })
        .await
        .map_err(error)??;
        if terminal.kind != "run.completed" {
            return Err(serde_json::to_string(&terminal).map_err(error)?);
        }
        Ok(json!({"success":true,"runId":run_id,"snapshot":session.runtime.snapshot().await}))
    }
    .await;
    session.stop().await;
    result
}
