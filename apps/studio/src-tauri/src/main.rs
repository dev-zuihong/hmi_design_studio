#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hmi_desktop::run(tauri::generate_context!(), hmi_desktop::AppMode::Studio);
}
