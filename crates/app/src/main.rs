#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gpu;
mod i18n;
mod icon_art;
mod logger;
mod platform;
mod state;
mod tray;
mod ui;

use std::time::Duration;

use boxpilot_core::Engine;
use gpui_kit::{App, AppContext as _, AsyncApp};

use crate::state::AppState;
use crate::tray::{TrayAction, TrayHandle};

fn main() {
    let autostart = std::env::args().any(|a| a == "--autostart");
    // CI: start the full UI, check the window came up, then exit.
    let smoke_test = std::env::args().any(|a| a == "--smoke-test");
    // The renderer picks its GPU at start-up; apply the preference before
    // GPUI (or any other thread) starts.
    if let Ok(store) = boxpilot_core::store::Store::new(boxpilot_core::store::Paths::detect()) {
        gpu::apply_at_startup(store.load_settings().gpu);
    }
    let Some(mut show_requests) = platform::single_instance() else {
        // Another instance is running and has been asked to show itself.
        return;
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("boxpilot-worker")
        .enable_all()
        .build()
        .expect("tokio runtime");
    let (engine, mut events) = match Engine::new(runtime.handle().clone()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("BoxPilot failed to start: {e:#}");
            return;
        }
    };
    logger::init(&engine.paths().root);
    log::info!("BoxPilot {} starting", env!("CARGO_PKG_VERSION"));

    let settings = engine.settings();
    // Windows only: elsewhere "administrator" means a privileged core binary.
    if cfg!(windows) && settings.run_as_admin && !boxpilot_core::sys::is_elevated() {
        // Hand over to an elevated instance; it waits for this one to exit.
        let mut args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--restarted").collect();
        args.push("--restarted".into());
        match boxpilot_core::sys::relaunch_elevated(&args.join(" ")) {
            Ok(()) => return,
            Err(e) => log::warn!("continuing without elevation: {e:#}"),
        }
    }
    i18n::set_language(settings.language);

    let app = gpui_kit::application().with_assets(ui::assets::AppAssets);
    // macOS: clicking the Dock icon brings the window back.
    app.on_reopen(ui::show_main_window);
    app.run(move |cx: &mut App| {
            gpui_kit::init(cx);
            ui::init(cx);
            ui::layout::refresh_system_scale();

            let state = cx.new(|_| AppState::new(engine.clone(), settings.language));
            AppState::install(state.clone(), cx);

            let (tray_tx, mut tray_rx) = tokio::sync::mpsc::unbounded_channel();
            match TrayHandle::new(move |a| {
                let _ = tray_tx.send(a);
            }) {
                Ok(tray) => state.update(cx, |s, _| {
                    s.tray = Some(tray);
                    s.sync_tray();
                }),
                Err(e) => log::error!("tray unavailable: {e:#}"),
            }

            // Engine events -> UI, coalesced to keep rendering cheap under log floods.
            let pump_state = state.clone();
            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Some(first) = events.recv().await {
                    let mut batch = vec![first];
                    while let Ok(e) = events.try_recv() {
                        batch.push(e);
                    }
                    pump_state.update(cx, |s, cx| s.handle_events(batch, cx));
                    cx.background_executor().timer(Duration::from_millis(60)).await;
                }
            })
            .detach();

            cx.spawn(async move |cx: &mut AsyncApp| {
                while let Some(action) = tray_rx.recv().await {
                    cx.update(|cx| handle_tray_action(action, cx));
                }
            })
            .detach();

            cx.spawn(async move |cx: &mut AsyncApp| {
                while show_requests.recv().await.is_some() {
                    cx.update(ui::show_main_window);
                }
            })
            .detach();

            ui::open_main_window(cx, !(autostart && settings.start_minimized));
            engine.startup();
            if smoke_test {
                cx.spawn(async move |cx: &mut AsyncApp| {
                    cx.background_executor().timer(Duration::from_secs(6)).await;
                    cx.update(|cx| {
                        let ok = AppState::global(cx).read(cx).window.is_some();
                        println!("ui smoke test: window {}", if ok { "open" } else { "missing" });
                        AppState::engine(cx).shutdown_blocking();
                        std::process::exit(if ok { 0 } else { 1 });
                    });
                })
                .detach();
            }
        });
    drop(runtime);
}

fn handle_tray_action(action: TrayAction, cx: &mut App) {
    let engine = AppState::engine(cx);
    match action {
        TrayAction::Show => ui::show_main_window(cx),
        TrayAction::ToggleCore => {
            if engine.status().is_running() {
                engine.stop();
            } else {
                engine.start();
            }
        }
        TrayAction::Mode(mode) => engine.set_mode(mode),
        TrayAction::ToggleSystemProxy => engine.update_settings(|s| s.system_proxy = !s.system_proxy),
        TrayAction::ToggleTun => ui::toggle_tun(!engine.settings().tun_enabled, cx),
        TrayAction::UpdateSubscriptions => engine.update_all(),
        TrayAction::Quit => ui::quit(cx),
    }
}
