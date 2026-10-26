//! Native window visibility and single-instance handling.

use gpui_kit::Window;

#[cfg(windows)]
fn hwnd(window: &Window) -> Option<windows_sys::Win32::Foundation::HWND> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get() as _),
        _ => None,
    }
}

/// Hides the window without destroying it (used for "close to tray").
pub fn hide_window(window: &Window) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindow};
        unsafe { ShowWindow(h, SW_HIDE) };
        return;
    }
    window.minimize_window();
}

pub fn show_window(window: &Window) {
    #[cfg(windows)]
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
    window.activate_window();
}

/// Ensures only one instance runs. Returns `None` when another instance was
/// found (and asked to show itself); otherwise a receiver that fires whenever a
/// later instance asks this one to come to the front.
pub fn single_instance() -> Option<tokio::sync::mpsc::UnboundedReceiver<()>> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
        use windows_sys::Win32::System::Threading::{
            CreateEventW, INFINITE, SetEvent, WaitForSingleObject,
        };
        let name: Vec<u16> = "Local\\BoxPilot.SingleInstance.Show"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        use windows_sys::Win32::Foundation::CloseHandle;
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
    #[cfg(not(windows))]
    drop(tx);
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
