use crate::{resources::*, session};
use serde_json::json;
use tauri::{PhysicalPosition, PhysicalSize, WebviewWindow};

pub fn restore(window: &WebviewWindow, project_id: &str) -> Result<()> {
    let file = session::user_directory(project_id)?.join("window.json");
    if !file.exists() {
        return Ok(());
    }
    let value = read_json(&file)?;
    if let (Some(width), Some(height)) = (value["width"].as_u64(), value["height"].as_u64()) {
        if (640..=16384).contains(&width) && (480..=16384).contains(&height) {
            window
                .set_size(PhysicalSize::new(width as u32, height as u32))
                .map_err(error)?;
        }
    }
    if let (Some(x), Some(y)) = (value["x"].as_i64(), value["y"].as_i64()) {
        // Never restore a title bar outside all connected monitors.
        let visible = window
            .available_monitors()
            .map_err(error)?
            .iter()
            .any(|monitor| {
                let position = monitor.position();
                let size = monitor.size();
                x >= position.x as i64
                    && y >= position.y as i64
                    && x + 100 <= position.x as i64 + size.width as i64
                    && y + 60 <= position.y as i64 + size.height as i64
            });
        if visible {
            window
                .set_position(PhysicalPosition::new(x as i32, y as i32))
                .map_err(error)?;
        }
    }
    if value["maximized"] == true {
        window.maximize().map_err(error)?;
    }
    Ok(())
}

pub fn save(window: &WebviewWindow, project_id: &str) -> Result<()> {
    if window.is_minimized().map_err(error)? {
        return Ok(());
    }
    let size = window.inner_size().map_err(error)?;
    let position = window.outer_position().map_err(error)?;
    write_json(
        &session::user_directory(project_id)?.join("window.json"),
        &json!({
            "width":size.width,"height":size.height,"x":position.x,"y":position.y,
            "maximized":window.is_maximized().map_err(error)?,
        }),
    )
}
