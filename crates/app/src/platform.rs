//! Native window visibility, single-instance handling and other OS glue.

use gpui_kit::{App, Window};

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as _),
        _ => None,
    }
}

/// Puts the window away for "close to tray". Returns whether the platform
/// should go ahead and destroy the window.
pub fn hide_to_tray(window: &mut Window, cx: &mut App) -> bool {
    #[cfg(windows)]
    {
        let _ = cx;
        if let Some(h) = hwnd(window) {
            use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindowAsync};
            unsafe { ShowWindowAsync(h, SW_HIDE) };
            return false;
        }
        window.minimize_window();
        false
    }
    #[cfg(target_os = "macos")]
    {
        // Like native macOS apps: hide, and come back from the Dock or menu bar.
        let _ = window;
        cx.hide();
        false
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // X11 / Wayland have no portable "hide": close the window and reopen it
        // from the tray. All state lives outside the window.
        let _ = window;
        let state = crate::state::AppState::global(cx);
        state.update(cx, |s, _| s.window = None);
        true
    }
}

pub fn show_window(window: &mut Window, cx: &mut App) {
    #[cfg(windows)]
    {
        let _ = cx;
        if let Some(h) = hwnd(window) {
            use windows_sys::Win32::UI::WindowsAndMessaging::*;
            unsafe {
                if IsIconic(h) != 0 {
                    ShowWindow(h, SW_RESTORE);
                } else {
                    ShowWindow(h, SW_SHOW);
                }
                SetForegroundWindow(h);
            }
            return;
        }
    }
    #[cfg(not(windows))]
    cx.activate(true);
    window.activate_window();
}

/// Returns memory freed by a closed window to the OS.
pub fn trim_memory() {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        // (-1, -1) empties the working set; pages fault back in as needed.
        SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX);
    }
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Maximizes, or restores when already maximized.
pub fn toggle_maximize(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        // Async: a synchronous resize would re-enter GPUI's message handler
        // while it is still dispatching this click.
        use windows_sys::Win32::UI::WindowsAndMessaging::{IsZoomed, SW_MAXIMIZE, SW_RESTORE, ShowWindowAsync};
        unsafe { ShowWindowAsync(h, if IsZoomed(h) != 0 { SW_RESTORE } else { SW_MAXIMIZE }) };
        return;
    }
    window.zoom_window();
}

/// Mica needs the system backdrop API added in Windows 11 22H2 (build 22621).
pub fn mica_supported() -> bool {
    cfg!(windows) && boxpilot_core::sys::windows_build() >= 22621
}

/// Tells DWM whether the frame (and the Mica tint) should be dark, so the
/// native backdrop follows the app theme rather than the system one.
pub fn set_dark_frame(window: &Window, dark: bool) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        use windows_sys::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
        let value: i32 = dark as i32;
        unsafe {
            DwmSetWindowAttribute(
                h,
                DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
                &value as *const i32 as *const _,
                std::mem::size_of::<i32>() as u32,
            );
        }
    }
    let _ = (window, dark);
}

/// Ensures only one instance runs. Returns `None` when another instance was
/// found (and asked to show itself); otherwise a receiver that fires whenever a
/// later instance asks this one to come to the front.
pub fn single_instance() -> Option<tokio::sync::mpsc::UnboundedReceiver<()>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, SetEvent, WaitForSingleObject};
        let name: Vec<u16> = "Local\\BoxPilot.SingleInstance.Show".encode_utf16().chain(Some(0)).collect();
        // After an elevated restart the previous instance may still be exiting.
        let restarted = std::env::args().any(|a| a == "--restarted");
        let mut attempts = if restarted { 50 } else { 1 };
        let event = loop {
            let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
            if event.is_null() {
                return Some(rx);
            }
            if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
                break event;
            }
            attempts -= 1;
            if attempts == 0 {
                unsafe { SetEvent(event) };
                return None;
            }
            unsafe { CloseHandle(event) };
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
        let event = event as usize;
        std::thread::spawn(move || {
            loop {
                unsafe { WaitForSingleObject(event as _, INFINITE) };
                if tx.send(()).is_err() {
                    break;
                }
            }
        });
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::net::{UnixListener, UnixStream};
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = dir.join(format!("boxpilot-{}.sock", unsafe { libc::getuid() }));
        // A live instance answers on the socket: ask it to show itself. After
        // a restart the previous instance may still be shutting down.
        let restarted = std::env::args().any(|a| a == "--restarted");
        for attempt in 0..if restarted { 50 } else { 1 } {
            match UnixStream::connect(&path) {
                Ok(_) if restarted && attempt < 49 => std::thread::sleep(std::time::Duration::from_millis(100)),
                Ok(mut stream) => {
                    let _ = stream.write_all(b"show");
                    return None;
                }
                Err(_) => break,
            }
        }
        let _ = std::fs::remove_file(&path);
        if let Ok(listener) = UnixListener::bind(&path) {
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stream.is_ok() && tx.send(()).is_err() {
                        break;
                    }
                }
            });
        }
    }
    Some(rx)
}

pub fn open_path(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
