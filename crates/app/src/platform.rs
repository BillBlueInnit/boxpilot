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
/// What the window can show behind translucent surfaces on this system.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Translucency {
    /// Opaque only: either nothing is available, or only unblurred
    /// transparency (GNOME, X11), which would leave text unreadable.
    None,
    /// Windows 11 22H2+ DWM backdrops: Mica, Mica Alt, Acrylic.
    WindowsMaterials,
    /// One system blur: macOS vibrancy, or KWin's blur on KDE Plasma (Wayland).
    Blur,
}

pub fn translucency() -> Translucency {
    if cfg!(windows) {
        if boxpilot_core::sys::windows_build() >= 22621 { Translucency::WindowsMaterials } else { Translucency::None }
    } else if cfg!(target_os = "macos") {
        Translucency::Blur
    } else {
        // GPUI blurs through org_kde_kwin_blur, which only KWin offers.
        let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
        if wayland && kde { Translucency::Blur } else { Translucency::None }
    }
}

pub fn mica_supported() -> bool {
    translucency() != Translucency::None
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

/// Lets the Mica backdrop show through the client area. DWM paints it in the
/// window frame only, and the custom title bar leaves no visible frame, so the
/// frame is extended over the whole window (content painted on top hides it).
pub fn set_mica_frame(window: &Window, on: bool) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
        use windows_sys::Win32::UI::Controls::MARGINS;
        let m = if on { -1 } else { 0 };
        let margins = MARGINS { cxLeftWidth: m, cxRightWidth: m, cyTopHeight: m, cyBottomHeight: m };
        unsafe { DwmExtendFrameIntoClientArea(h, &margins) };
    }
    let _ = (window, on);
}

/// Picks the DWM system backdrop: 2 Mica, 3 Acrylic, 4 Mica Alt. Windows
/// tints it for light or dark mode itself, following the dark-frame flag.
pub fn set_system_backdrop(window: &Window, kind: u32) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        use windows_sys::Win32::Graphics::Dwm::{DWMWA_SYSTEMBACKDROP_TYPE, DwmSetWindowAttribute};
        unsafe {
            DwmSetWindowAttribute(
                h,
                DWMWA_SYSTEMBACKDROP_TYPE as u32,
                &kind as *const u32 as *const _,
                std::mem::size_of::<u32>() as u32,
            );
        }
    }
    let _ = (window, kind);
}

/// Whether the backdrop should stay as it is on an inactive window.
#[cfg(windows)]
static KEEP_BACKDROP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Windows draws Mica and Acrylic only while a window is active and a flat
/// colour otherwise, deciding from WM_NCACTIVATE. Answering that message as
/// "still active" keeps the backdrop live. GPUI tracks focus from WM_ACTIVATE,
/// so input focus and the dimmed caption buttons are unaffected.
pub fn keep_backdrop_live(window: &Window, on: bool) {
    #[cfg(windows)]
    if let Some(h) = hwnd(window) {
        use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
        use windows_sys::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
        use windows_sys::Win32::UI::WindowsAndMessaging::WM_NCACTIVATE;
        unsafe extern "system" fn subclass(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
            if msg == WM_NCACTIVATE && KEEP_BACKDROP.load(std::sync::atomic::Ordering::Relaxed) {
                return unsafe { DefSubclassProc(hwnd, msg, 1, lparam) };
            }
            unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
        }
        KEEP_BACKDROP.store(on, std::sync::atomic::Ordering::Relaxed);
        // Installing again with the same id only updates the existing hook.
        unsafe { SetWindowSubclass(h, Some(subclass), 0xB0C5, 0) };
    }
    let _ = (window, on);
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
        // BOXPILOT_INSTANCE gives a development copy its own lock.
        let suffix = std::env::var("BOXPILOT_INSTANCE").map(|s| format!(".{s}")).unwrap_or_default();
        let name: Vec<u16> = format!("Local\\BoxPilot.SingleInstance.Show{suffix}").encode_utf16().chain(Some(0)).collect();
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

/// Opens a file in the user's default text editor, whatever its extension.
pub fn open_in_text_editor(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{SEE_MASK_CLASSNAME, SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file, class) = (wide("open"), wide(&path.to_string_lossy()), wide(".txt"));
        // Opened as a .txt, so it goes to the editor the user picked for text.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_CLASSNAME;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpClass = class.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            let _ = std::process::Command::new("notepad.exe").arg(path).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-t").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The desktop's text/plain handler; xdg-open would pick the JSON viewer.
        let handler = std::process::Command::new("xdg-mime")
            .args(["query", "default", "text/plain"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_end_matches(".desktop").to_string())
            .filter(|h| !h.is_empty());
        let launched = handler.is_some_and(|h| std::process::Command::new("gtk-launch").arg(h).arg(path).spawn().is_ok());
        if !launched {
            let _ = std::process::Command::new("xdg-open").arg(path).spawn();
        }
    }
}

pub fn open_path(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}
