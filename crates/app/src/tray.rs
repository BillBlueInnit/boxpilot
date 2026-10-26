//! System tray icon and menu.

use boxpilot_core::model::ProxyMode;
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::icon_art::render_icon;
use crate::t;
use crate::ui::widgets::mode_label;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    ToggleCore,
    Mode(ProxyMode),
    ToggleSystemProxy,
    ToggleTun,
    UpdateSubscriptions,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    show: MenuItem,
    toggle_core: MenuItem,
    mode_menu: Submenu,
    modes: Vec<(ProxyMode, CheckMenuItem)>,
    system_proxy: CheckMenuItem,
    tun: CheckMenuItem,
    update: MenuItem,
    quit: MenuItem,
    active: Option<bool>,
    running: bool,
}

fn icon(active: bool) -> Option<Icon> {
    Icon::from_rgba(render_icon(32, active), 32, 32).ok()
}

impl Tray {
    pub fn new(on_action: impl Fn(TrayAction) + Send + Sync + 'static) -> anyhow::Result<Self> {
        let show = MenuItem::new(t!("tray.show"), true, None);
        let toggle_core = MenuItem::new(t!("tray.start"), true, None);
        let mode_menu = Submenu::new(t!("tray.mode"), true);
        let modes: Vec<(ProxyMode, CheckMenuItem)> = ProxyMode::ALL
            .into_iter()
            .map(|m| (m, CheckMenuItem::new(mode_label(m), true, false, None)))
            .collect();
        for (_, item) in &modes {
            mode_menu.append(item)?;
        }
        let system_proxy = CheckMenuItem::new(t!("tray.sysproxy"), true, false, None);
        let tun = CheckMenuItem::new(t!("tray.tun"), true, false, None);
        let update = MenuItem::new(t!("tray.update_subs"), true, None);
        let quit = MenuItem::new(t!("tray.quit"), true, None);

        let menu = Menu::new();
        menu.append_items(&[
            &show,
            &PredefinedMenuItem::separator(),
            &toggle_core,
            &mode_menu,
            &system_proxy,
            &tun,
            &PredefinedMenuItem::separator(),
            &update,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let mut ids = vec![
            (show.id().clone(), TrayAction::Show),
            (toggle_core.id().clone(), TrayAction::ToggleCore),
            (system_proxy.id().clone(), TrayAction::ToggleSystemProxy),
            (tun.id().clone(), TrayAction::ToggleTun),
            (update.id().clone(), TrayAction::UpdateSubscriptions),
            (quit.id().clone(), TrayAction::Quit),
        ];
        for (m, item) in &modes {
            ids.push((item.id().clone(), TrayAction::Mode(*m)));
        }

        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("BoxPilot")
            .with_icon(icon(false).ok_or_else(|| anyhow::anyhow!("tray icon"))?)
            .build()?;

        let on_action = std::sync::Arc::new(on_action);
        let menu_action = on_action.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, a)) = ids.iter().find(|(id, _)| *id == e.id) {
                menu_action(*a);
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            let show = match e {
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => true,
                TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => true,
                _ => false,
            };
            if show {
                on_action(TrayAction::Show);
            }
        }));

        Ok(Self {
            icon,
            show,
            toggle_core,
            mode_menu,
            modes,
            system_proxy,
            tun,
            update,
            quit,
            active: None,
            running: false,
        })
    }

    /// Re-applies menu labels after a language change.
    pub fn relabel(&mut self) {
        self.show.set_text(t!("tray.show"));
        self.toggle_core.set_text(if self.running { t!("tray.stop") } else { t!("tray.start") });
        self.mode_menu.set_text(t!("tray.mode"));
        for (m, item) in &self.modes {
            item.set_text(mode_label(*m));
        }
        self.system_proxy.set_text(t!("tray.sysproxy"));
        self.tun.set_text(t!("tray.tun"));
        self.update.set_text(t!("tray.update_subs"));
        self.quit.set_text(t!("tray.quit"));
    }

    pub fn sync(&mut self, running: bool, busy: bool, mode: ProxyMode, system_proxy: bool, tun: bool, tooltip: &str) {
        self.running = running;
        self.toggle_core.set_text(if running { t!("tray.stop") } else { t!("tray.start") });
        self.toggle_core.set_enabled(!busy);
        for (m, item) in &self.modes {
            item.set_checked(*m == mode);
        }
        self.system_proxy.set_checked(system_proxy);
        self.tun.set_checked(tun);
        let _ = self.icon.set_tooltip(Some(tooltip));
        if self.active != Some(running) {
            self.active = Some(running);
            let _ = self.icon.set_icon(icon(running));
        }
    }
}

/// Owns the tray from the UI thread. On Linux the indicator must live on a
/// GTK main loop, so it runs on its own thread and receives updates.
pub struct TrayHandle {
    #[cfg(not(target_os = "linux"))]
    tray: Tray,
    #[cfg(target_os = "linux")]
    tx: std::sync::mpsc::Sender<TrayMsg>,
}

#[cfg(target_os = "linux")]
enum TrayMsg {
    Relabel,
    Sync(bool, bool, ProxyMode, bool, bool, String),
}

impl TrayHandle {
    pub fn new(on_action: impl Fn(TrayAction) + Send + Sync + 'static) -> anyhow::Result<Self> {
        #[cfg(not(target_os = "linux"))]
        {
            Ok(Self { tray: Tray::new(on_action)? })
        }
        #[cfg(target_os = "linux")]
        {
            let (tx, rx) = std::sync::mpsc::channel::<TrayMsg>();
            std::thread::Builder::new().name("tray".into()).spawn(move || {
                if let Err(e) = gtk::init() {
                    log::error!("tray unavailable (GTK): {e}");
                    return;
                }
                let mut tray = match Tray::new(on_action) {
                    Ok(t) => t,
                    Err(e) => {
                        log::error!("tray unavailable: {e:#}");
                        return;
                    }
                };
                gtk::glib::timeout_add_local(std::time::Duration::from_millis(120), move || {
                    while let Ok(msg) = rx.try_recv() {
                        match msg {
                            TrayMsg::Relabel => tray.relabel(),
                            TrayMsg::Sync(r, b, m, p, t, tip) => tray.sync(r, b, m, p, t, &tip),
                        }
                    }
                    gtk::glib::ControlFlow::Continue
                });
                gtk::main();
            })?;
            Ok(Self { tx })
        }
    }

    pub fn relabel(&mut self) {
        #[cfg(not(target_os = "linux"))]
        self.tray.relabel();
        #[cfg(target_os = "linux")]
        let _ = self.tx.send(TrayMsg::Relabel);
    }

    pub fn sync(&mut self, running: bool, busy: bool, mode: ProxyMode, system_proxy: bool, tun: bool, tooltip: &str) {
        #[cfg(not(target_os = "linux"))]
        self.tray.sync(running, busy, mode, system_proxy, tun, tooltip);
        #[cfg(target_os = "linux")]
        let _ = self.tx.send(TrayMsg::Sync(running, busy, mode, system_proxy, tun, tooltip.to_string()));
    }
}
