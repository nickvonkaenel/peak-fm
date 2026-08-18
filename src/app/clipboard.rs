//! Clipboard operations and platform-specific integrations

#![allow(unexpected_cfgs)]

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

use super::App;

impl App {
    /// Copy the selected file to the system clipboard
    pub fn copy_file_to_clipboard(&mut self) {
        let Some(path) = self.current.selected_path() else {
            self.set_status("No file selected");
            return;
        };

        if !path.exists() {
            self.set_status("File does not exist");
            return;
        }

        // Get absolute path
        let absolute_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path.clone())
        };

        let path_str = absolute_path.to_string_lossy().to_string();

        #[cfg(target_os = "macos")]
        let result = copy_to_clipboard_macos(&path_str);

        #[cfg(target_os = "windows")]
        let result = copy_to_clipboard_windows(&path_str);

        #[cfg(target_os = "linux")]
        let result = copy_to_clipboard_linux(&path_str);

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let result: Result<(), String> =
            Err("Clipboard not supported on this platform".to_string());

        match result {
            Ok(()) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.set_status(format!("Copied to clipboard: {}", name));
            }
            Err(e) => {
                self.set_status(format!("Clipboard error: {}", e));
            }
        }
    }

    /// Copy the selected entry's absolute path as plain text to the system clipboard
    pub fn copy_path_text_to_clipboard(&mut self) {
        let Some(path) = self.current.selected_path() else {
            self.set_status("No file selected");
            return;
        };

        // Get absolute path
        let absolute_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path.clone())
        };

        let path_str = absolute_path.to_string_lossy().to_string();
        // Remove Windows extended-length path prefix \\?\
        let path_str = path_str
            .strip_prefix(r"\\?\")
            .map(str::to_string)
            .unwrap_or(path_str);

        match copy_text_to_clipboard(&path_str) {
            Ok(()) => {
                self.set_status(format!("Copied path: {}", path_str));
            }
            Err(e) => {
                self.set_status(format!("Clipboard error: {}", e));
            }
        }
    }

    /// Copy the selected file to clipboard and activate Reaper
    pub fn copy_file_and_activate_reaper(&mut self) {
        let Some(path) = self.current.selected_path() else {
            self.set_status("No file selected");
            return;
        };

        if !path.exists() {
            self.set_status("File does not exist");
            return;
        }

        // Get absolute path
        let absolute_path = if path.is_absolute() {
            path.clone()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(&path))
                .unwrap_or(path.clone())
        };

        let path_str = absolute_path.to_string_lossy().to_string();

        #[cfg(target_os = "macos")]
        let clipboard_result = copy_to_clipboard_macos(&path_str);

        #[cfg(target_os = "windows")]
        let clipboard_result = copy_to_clipboard_windows(&path_str);

        #[cfg(target_os = "linux")]
        let clipboard_result = copy_to_clipboard_linux(&path_str);

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let clipboard_result: Result<(), String> =
            Err("Not supported on this platform".to_string());

        if let Err(e) = clipboard_result {
            self.set_status(format!("Clipboard error: {}", e));
            return;
        }

        #[cfg(target_os = "macos")]
        let reaper_result = activate_reaper_macos();

        #[cfg(target_os = "windows")]
        let reaper_result = activate_reaper_windows();

        #[cfg(target_os = "linux")]
        let reaper_result = activate_reaper_linux();

        #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
        let reaper_result: Result<(), String> = Err("Not supported".to_string());

        match reaper_result {
            Ok(()) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.set_status(format!("Copied & activated Reaper: {}", name));
            }
            Err(e) => {
                self.set_status(format!("Reaper error: {}", e));
            }
        }
    }
}

/// Copy a file path to the system clipboard (public API for use outside App)
pub fn copy_path_to_clipboard(path: &str) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    return copy_to_clipboard_macos(path);

    #[cfg(target_os = "windows")]
    return copy_to_clipboard_windows(path);

    #[cfg(target_os = "linux")]
    return copy_to_clipboard_linux(path);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Clipboard not supported on this platform".to_string());
}

/// Copy multiple file paths to the system clipboard
pub fn copy_paths_to_clipboard(paths: &[String]) -> std::result::Result<(), String> {
    if paths.is_empty() {
        return Err("No files to copy".to_string());
    }

    #[cfg(target_os = "macos")]
    return copy_paths_to_clipboard_macos(paths);

    #[cfg(target_os = "windows")]
    return copy_paths_to_clipboard_windows(paths);

    #[cfg(target_os = "linux")]
    return copy_paths_to_clipboard_linux(paths);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Clipboard not supported on this platform".to_string());
}

/// Copy plain text to the system clipboard
pub fn copy_text_to_clipboard(text: &str) -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    return copy_text_to_clipboard_macos(text);

    #[cfg(target_os = "windows")]
    return copy_text_to_clipboard_windows(text);

    #[cfg(target_os = "linux")]
    return copy_text_to_clipboard_linux(text);

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Clipboard not supported on this platform".to_string());
}

/// Activate Reaper application
pub fn activate_reaper() -> std::result::Result<(), String> {
    #[cfg(target_os = "macos")]
    return activate_reaper_macos();

    #[cfg(target_os = "windows")]
    return activate_reaper_windows();

    #[cfg(target_os = "linux")]
    return activate_reaper_linux();

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Err("Reaper activation not supported on this platform".to_string());
}

// ============ macOS implementations ============

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn copy_to_clipboard_macos(path: &str) -> Result<(), String> {
    copy_paths_to_clipboard_macos(&[path.to_string()])
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn copy_paths_to_clipboard_macos(paths: &[String]) -> Result<(), String> {
    use cocoa::base::{id, nil};
    use cocoa::foundation::NSString;
    use objc::{class, msg_send, sel, sel_impl};

    if paths.is_empty() {
        return Err("No files to copy".to_string());
    }

    unsafe {
        let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
        let _: () = msg_send![pasteboard, clearContents];

        // Create array of file URLs
        let mut urls: Vec<id> = Vec::new();
        for path in paths {
            let ns_path = NSString::alloc(nil).init_str(path);
            let url: id = msg_send![class!(NSURL), fileURLWithPath: ns_path];
            urls.push(url);
        }

        // Create NSArray from URLs
        let objects: id =
            msg_send![class!(NSArray), arrayWithObjects:urls.as_ptr() count:urls.len()];
        let success: bool = msg_send![pasteboard, writeObjects: objects];

        if success {
            Ok(())
        } else {
            Err("Failed to write to pasteboard".to_string())
        }
    }
}

#[cfg(target_os = "macos")]
fn copy_text_to_clipboard_macos(text: &str) -> Result<(), String> {
    use std::io::Write;

    let mut child = Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(ref mut stdin) = child.stdin {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| e.to_string())?;
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("pbcopy failed".to_string())
    }
}

#[cfg(target_os = "macos")]
fn activate_reaper_macos() -> Result<(), String> {
    let script = r#"tell application "REAPER" to activate"#;
    let output = Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| e.to_string())?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(format!("AppleScript failed: {}", stderr))
    }
}

// ============ Windows implementations ============

#[cfg(target_os = "windows")]
fn copy_to_clipboard_windows(path: &str) -> std::result::Result<(), String> {
    copy_paths_to_clipboard_windows(&[path.to_string()])
}

#[cfg(target_os = "windows")]
fn copy_paths_to_clipboard_windows(paths: &[String]) -> std::result::Result<(), String> {
    use clipboard_win::{formats, Clipboard, Setter};

    if paths.is_empty() {
        return Err("No files to copy".to_string());
    }

    // Remove Windows extended-length path prefix \\?\ from all paths
    let clean_paths: Vec<String> = paths
        .iter()
        .map(|path| {
            path.strip_prefix(r"\\?\")
                .unwrap_or(path.as_str())
                .to_string()
        })
        .collect();

    let _clip = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;

    // Empty clipboard to clear all formats (including custom Reaper formats)
    unsafe {
        if EmptyClipboard().is_err() {
            return Err("Failed to empty clipboard".to_string());
        }
    }

    formats::FileList
        .write_clipboard(&clean_paths)
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn copy_text_to_clipboard_windows(text: &str) -> std::result::Result<(), String> {
    use clipboard_win::{formats, Clipboard, Setter};

    let _clip = Clipboard::new_attempts(10).map_err(|e| e.to_string())?;

    // Empty clipboard to clear all formats (including custom Reaper formats)
    unsafe {
        if EmptyClipboard().is_err() {
            return Err("Failed to empty clipboard".to_string());
        }
    }

    formats::Unicode
        .write_clipboard(&text)
        .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(target_os = "windows")]
use windows::{
    core::*, Win32::Foundation::*, Win32::System::DataExchange::EmptyClipboard,
    Win32::System::Threading::*, Win32::UI::WindowsAndMessaging::*,
};

#[cfg(target_os = "windows")]
fn activate_reaper_windows() -> std::result::Result<(), String> {
    unsafe {
        let mut reaper_hwnd: HWND = HWND::default();

        // Find Reaper window by enumerating all windows
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut reaper_hwnd as *mut HWND as isize),
        );

        if reaper_hwnd.0.is_null() {
            return Err("REAPER window not found".to_string());
        }

        // Bring window to foreground
        if SetForegroundWindow(reaper_hwnd).as_bool() {
            Ok(())
        } else {
            // Try alternative method
            let _ = ShowWindow(reaper_hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(reaper_hwnd);
            Ok(())
        }
    }
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    let mut process_id: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut process_id));

    if process_id == 0 {
        return BOOL(1); // Continue enumeration
    }

    // Get process handle and check executable name
    if let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) {
        let mut buffer = [0u16; 260];
        let mut size = buffer.len() as u32;

        if QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
        .is_ok()
        {
            let path = OsString::from_wide(&buffer[..size as usize]);
            let path_str = path.to_string_lossy().to_lowercase();

            if path_str.ends_with("reaper.exe") {
                // Check if window is visible
                if IsWindowVisible(hwnd).as_bool() {
                    let result_ptr = lparam.0 as *mut HWND;
                    *result_ptr = hwnd;
                    let _ = CloseHandle(process);
                    return BOOL(0); // Stop enumeration
                }
            }
        }
        let _ = CloseHandle(process);
    }

    BOOL(1) // Continue enumeration
}

// ============ Linux implementations ============

#[cfg(target_os = "linux")]
fn copy_to_clipboard_linux(path: &str) -> Result<(), String> {
    copy_paths_to_clipboard_linux(&[path.to_string()])
}

#[cfg(target_os = "linux")]
fn copy_paths_to_clipboard_linux(paths: &[String]) -> Result<(), String> {
    use std::io::Write;

    if paths.is_empty() {
        return Err("No files to copy".to_string());
    }

    // Use gnome-copied-files format (works with Nautilus, most file managers)
    // Format: "copy\nfile://path1\nfile://path2\n..."
    let mut content = String::from("copy\n");
    for path in paths {
        content.push_str(&format!("file://{}\n", path));
    }

    // Try xclip first (X11)
    let xclip_result = Command::new("xclip")
        .args([
            "-selection",
            "clipboard",
            "-t",
            "x-special/gnome-copied-files",
        ])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(content.as_bytes())?;
            }
            child.wait()
        });

    if xclip_result.is_ok() {
        return Ok(());
    }

    // If xclip fails, try wl-copy (Wayland)
    let wl_result = Command::new("wl-copy")
        .args(["--type", "x-special/gnome-copied-files"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(content.as_bytes())?;
            }
            child.wait()
        });

    if wl_result.is_ok() {
        return Ok(());
    }

    Err("Neither xclip nor wl-copy available".to_string())
}

#[cfg(target_os = "linux")]
fn copy_text_to_clipboard_linux(text: &str) -> Result<(), String> {
    use std::io::Write;

    // Try xclip first (X11)
    let xclip_result = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });

    if xclip_result.is_ok() {
        return Ok(());
    }

    // If xclip fails, try wl-copy (Wayland)
    let wl_result = Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                stdin.write_all(text.as_bytes())?;
            }
            child.wait()
        });

    if wl_result.is_ok() {
        return Ok(());
    }

    Err("Neither xclip nor wl-copy available".to_string())
}

#[cfg(target_os = "linux")]
fn activate_reaper_linux() -> Result<(), String> {
    // Try wmctrl first (most reliable)
    let wmctrl_result = Command::new("wmctrl").args(["-a", "REAPER"]).output();

    if let Ok(output) = wmctrl_result {
        if output.status.success() {
            return Ok(());
        }
    }

    // Try xdotool as fallback
    let xdotool_result = Command::new("xdotool")
        .args(["search", "--name", "REAPER", "windowactivate"])
        .output();

    if let Ok(output) = xdotool_result {
        if output.status.success() {
            return Ok(());
        }
    }

    Err("Could not activate REAPER (install wmctrl or xdotool)".to_string())
}
