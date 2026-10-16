use crate::{resources::*, AppMode};
#[cfg(windows)]
use std::process::Command;

#[cfg(windows)]
pub fn prepare_webview(resources: &Resources, mode: AppMode) -> Result<()> {
    let runtime = resources.webview();
    if !runtime.join("msedgewebview2.exe").exists() {
        // Dev studio can use the locally installed Evergreen runtime. Export is stricter.
        if mode == AppMode::Runner {
            return Err(format!(
                "缺少固定 WebView2 运行资源: {}。请完整解压运行包。",
                runtime.display()
            ));
        }
        return Ok(());
    }
    check_amd64(&runtime.join("msedgewebview2.exe"))?;
    let absolute = std::fs::canonicalize(&runtime).map_err(error)?;
    let path = absolute.to_string_lossy();
    if path.starts_with("\\\\") && !path.starts_with("\\\\?\\") || path.starts_with("\\\\?\\UNC\\")
    {
        return Err("固定 WebView2 需要从本地磁盘运行，请将运行包移动到本地目录。".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // Microsoft requires both AppContainer read/execute grants on Windows 10.
        // Applying the same grants on Windows 11 is harmless and avoids OS-version guessing.
        let status = Command::new("icacls.exe")
            .arg(&absolute)
            .args([
                "/grant",
                "*S-1-15-2-1:(OI)(CI)(RX)",
                "*S-1-15-2-2:(OI)(CI)(RX)",
                "/T",
                "/Q",
            ])
            .creation_flags(0x08000000)
            .output()
            .map_err(error)?;
        if !status.status.success() {
            return Err(format!(
                "无法设置固定 WebView2 所需目录权限，请使用当前用户可写的本地目录。\n{}",
                String::from_utf8_lossy(&status.stderr)
            ));
        }
    }
    std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", absolute);
    Ok(())
}
#[cfg(not(windows))]
pub fn prepare_webview(_resources: &Resources, _mode: AppMode) -> Result<()> {
    if cfg!(target_os = "macos") {
        // Tauri uses the operating system's WKWebView on macOS.
        Ok(())
    } else {
        Err("仅支持 Windows x64 和 macOS arm64/x64".into())
    }
}
pub fn fatal(message: &str) {
    eprintln!("{message}");
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<u16> = "HMI Studio / Runtime"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}
