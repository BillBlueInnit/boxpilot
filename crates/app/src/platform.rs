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
    } else if compositor_blur().is_some() {
        Translucency::Blur
    } else {
        Translucency::None
    }
}

/// How a Wayland compositor blurs behind a translucent window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CompositorBlur {
    /// org_kde_kwin_blur (KWin), which GPUI asks for itself.
    Kde,
    /// ext-background-effect-v1 (niri…), asked for by [`set_background_blur`].
    Effect,
}

/// The compositor's blur, asked of it directly rather than guessed from the
/// desktop's name; `None` off Wayland or where nothing blurs.
pub fn compositor_blur() -> Option<CompositorBlur> {
    #[cfg(target_os = "linux")]
    {
        static BLUR: std::sync::LazyLock<Option<CompositorBlur>> = std::sync::LazyLock::new(|| {
            let blur = blur::detect();
            log::info!("compositor blur: {blur:?}");
            blur
        });
        *BLUR
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// Where the compositor blurs through ext-background-effect-v1: blurs the
/// background behind the window inside its frame (not the frame's shadow
/// margin), or stops. Called on every render; the compositor only hears
/// about changes, which GPUI commits with the next frame.
pub fn set_background_blur(window: &mut Window, on: bool) {
    #[cfg(target_os = "linux")]
    {
        use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
        if compositor_blur() != Some(CompositorBlur::Effect) {
            return;
        }
        let display = HasDisplayHandle::display_handle(window).ok().map(|h| h.as_raw());
        let surface = HasWindowHandle::window_handle(window).ok().map(|h| h.as_raw());
        let (Some(RawDisplayHandle::Wayland(display)), Some(RawWindowHandle::Wayland(surface))) = (display, surface) else {
            return;
        };
        // The frame's shadow margin is the client inset on each edge that is not tiled.
        let area = on.then(|| {
            let inset = f32::from(window.client_inset().unwrap_or_default());
            let tiling = match window.window_decorations() {
                gpui_kit::Decorations::Client { tiling } => tiling,
                gpui_kit::Decorations::Server => gpui_kit::Tiling { top: true, left: true, right: true, bottom: true },
            };
            let side = |tiled: bool| if tiled { 0.0 } else { inset };
            let size = window.viewport_size();
            let (left, top) = (side(tiling.left), side(tiling.top));
            let width = f32::from(size.width) - left - side(tiling.right);
            let height = f32::from(size.height) - top - side(tiling.bottom);
            [left, top, width, height].map(|v| v.round() as i32)
        });
        let id = Window::window_handle(window).window_id().as_u64();
        match blur::set(display.display.as_ptr(), surface.surface.as_ptr(), id, area) {
            Ok(true) => window.refresh(),
            Ok(false) => {}
            Err(e) => log::warn!("background blur: {e:#}"),
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, on);
}

#[cfg(target_os = "linux")]
mod blur {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use anyhow::Context as _;
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::protocol::wl_compositor::WlCompositor;
    use wayland_client::protocol::wl_region::WlRegion;
    use wayland_client::protocol::wl_registry::{self, WlRegistry};
    use wayland_client::protocol::wl_surface::WlSurface;
    use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum, delegate_noop};
    use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::{
        self as manager, ExtBackgroundEffectManagerV1,
    };
    use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

    use super::CompositorBlur;

    /// What the registry lists, and whether the effect manager can blur.
    #[derive(Default)]
    struct Globals {
        kde: bool,
        effect: Option<u32>,
        compositor: Option<(u32, u32)>,
        can_blur: bool,
    }

    impl Dispatch<WlRegistry, ()> for Globals {
        fn event(g: &mut Self, _: &WlRegistry, event: wl_registry::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
            if let wl_registry::Event::Global { name, interface, version } = event {
                match interface.as_str() {
                    "org_kde_kwin_blur_manager" => g.kde = true,
                    "ext_background_effect_manager_v1" => g.effect = Some(name),
                    "wl_compositor" => g.compositor = Some((name, version)),
                    _ => {}
                }
            }
        }
    }

    impl Dispatch<ExtBackgroundEffectManagerV1, ()> for Globals {
        fn event(
            g: &mut Self,
            _: &ExtBackgroundEffectManagerV1,
            event: manager::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let manager::Event::Capabilities { flags: WEnum::Value(flags) } = event {
                g.can_blur = flags.contains(manager::Capability::Blur);
            }
        }
    }

    delegate_noop!(Globals: ignore WlCompositor);
    delegate_noop!(Globals: WlRegion);
    delegate_noop!(Globals: ExtBackgroundEffectSurfaceV1);

    /// Lists the globals and binds the effect manager, whose capabilities
    /// arrive on the second roundtrip.
    fn probe(conn: &Connection) -> Option<(EventQueue<Globals>, Globals, WlRegistry, Option<ExtBackgroundEffectManagerV1>)> {
        let mut queue = conn.new_event_queue();
        let qh = queue.handle();
        let registry = conn.display().get_registry(&qh, ());
        let mut globals = Globals::default();
        queue.roundtrip(&mut globals).ok()?;
        let manager = globals.effect.map(|name| registry.bind::<ExtBackgroundEffectManagerV1, _, _>(name, 1, &qh, ()));
        if manager.is_some() {
            queue.roundtrip(&mut globals).ok()?;
        }
        Some((queue, globals, registry, manager))
    }

    /// ext-background-effect where offered: its region keeps the blur
    /// inside the frame, which GPUI's KWin blur (the whole surface) does not.
    pub fn detect() -> Option<CompositorBlur> {
        let conn = Connection::connect_to_env().ok()?;
        let (_, globals, _, _) = probe(&conn)?;
        if globals.can_blur {
            Some(CompositorBlur::Effect)
        } else if globals.kde {
            Some(CompositorBlur::Kde)
        } else {
            None
        }
    }

    /// The effect objects, made on GPUI's own connection (the surfaces live
    /// there), one per window. Kept for the app's lifetime with their queue.
    struct Effects {
        conn: Connection,
        queue: EventQueue<Globals>,
        compositor: WlCompositor,
        manager: ExtBackgroundEffectManagerV1,
        /// Each window's effect and the blur area last sent for it.
        windows: HashMap<u64, (ExtBackgroundEffectSurfaceV1, Option<[i32; 4]>)>,
    }

    thread_local! {
        static EFFECTS: RefCell<Option<Effects>> = const { RefCell::new(None) };
    }

    /// `display` and `surface`: GPUI's wl_display and the window's wl_surface;
    /// `area`: x, y, width, height to blur, or none. Whether anything was sent.
    pub fn set(
        display: *mut std::ffi::c_void,
        surface: *mut std::ffi::c_void,
        window: u64,
        area: Option<[i32; 4]>,
    ) -> anyhow::Result<bool> {
        EFFECTS.with_borrow_mut(|slot| {
            if slot.is_none() {
                // SAFETY: GPUI's display outlives every window, and so this.
                let conn = Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
                let (queue, globals, registry, manager) = probe(&conn).context("wayland roundtrip failed")?;
                let manager = manager.filter(|_| globals.can_blur).context("the compositor does not blur")?;
                let (name, version) = globals.compositor.context("no wl_compositor")?;
                let compositor = registry.bind::<WlCompositor, _, _>(name, version.min(4), &queue.handle(), ());
                *slot = Some(Effects { conn, queue, compositor, manager, windows: HashMap::new() });
            }
            let fx = slot.as_mut().unwrap();
            let qh = fx.queue.handle();
            let (effect, sent) = match fx.windows.get_mut(&window) {
                Some(entry) => entry,
                None => {
                    if area.is_none() {
                        return Ok(false);
                    }
                    // SAFETY: the window's surface, alive while it is.
                    let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }?;
                    let surface = WlSurface::from_id(&fx.conn, id)?;
                    let effect = fx.manager.get_background_effect(&surface, &qh, ());
                    fx.windows.entry(window).or_insert((effect, None))
                }
            };
            if *sent == area {
                return Ok(false);
            }
            match area {
                Some([x, y, width, height]) => {
                    let region = fx.compositor.create_region(&qh, ());
                    region.add(x, y, width, height);
                    effect.set_blur_region(Some(&region));
                    region.destroy();
                }
                None => effect.set_blur_region(None),
            }
            *sent = area;
            fx.conn.flush()?;
            Ok(true)
        })
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
        // BOXPILOT_INSTANCE gives a development copy its own lock.
        let suffix = std::env::var("BOXPILOT_INSTANCE").map(|s| format!(".{s}")).unwrap_or_default();
        let path = dir.join(format!("boxpilot-{}{suffix}.sock", unsafe { libc::getuid() }));
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

/// Runs a shell call on a thread of its own and hands back its result.
///
/// The shell can pump window messages while it works (COM waits, a UAC
/// prompt, the editor taking focus). On the UI thread that re-enters the
/// window in the middle of an event and the app aborts; on its own thread
/// nothing reaches the UI. Windows gets the single-threaded COM apartment
/// the shell expects there.
pub fn shell_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> tokio::sync::oneshot::Receiver<T> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    let spawned = std::thread::Builder::new().name("shell".into()).spawn(move || {
        #[cfg(windows)]
        let com = {
            use windows_sys::Win32::System::Com::{COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx};
            unsafe { CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32) >= 0 }
        };
        let _ = tx.send(f());
        #[cfg(windows)]
        if com {
            unsafe { windows_sys::Win32::System::Com::CoUninitialize() };
        }
    });
    if let Err(e) = spawned {
        log::error!("shell thread: {e}");
    }
    rx
}

/// Opens a file in the user's default text editor, whatever its extension.
pub fn open_in_text_editor(path: &std::path::Path) {
    let path = path.to_path_buf();
    drop(shell_thread(move || open_editor_blocking(&path)));
}

fn open_editor_blocking(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{SEE_MASK_CLASSNAME, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW};
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        let (verb, file, class) = (wide("open"), wide(&path.to_string_lossy()), wide(".txt"));
        // Opened as a .txt, so it goes to the editor the user picked for text.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        // NOASYNC: this thread ends right after, so the shell must finish here.
        info.fMask = SEE_MASK_CLASSNAME | SEE_MASK_NOASYNC;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpClass = class.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        if unsafe { ShellExecuteExW(&mut info) } == 0 {
            log::warn!("no text editor for .txt; using Notepad");
            let _ = std::process::Command::new("notepad.exe").arg(path).spawn();
        }
    }
    #[cfg(target_os = "macos")]
    if let Err(e) = std::process::Command::new("open").arg("-t").arg(path).status() {
        log::warn!("open -t: {e}");
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        // The desktop's text/plain handler; xdg-open would pick the JSON viewer.
        let handler = std::process::Command::new("xdg-mime")
            .args(["query", "default", "text/plain"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_end_matches(".desktop").to_string())
            .filter(|h| !h.is_empty());
        let launched = handler.is_some_and(|h| {
            std::process::Command::new("gtk-launch").arg(h).arg(path).status().is_ok_and(|s| s.success())
        });
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
