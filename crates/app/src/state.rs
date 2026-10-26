//! UI-side application state: page selection, traffic history and the bridge
//! between engine events and GPUI.

use std::collections::VecDeque;
use std::sync::Arc;

use boxpilot_core::clash_api::Traffic;
use boxpilot_core::model::Language;
use boxpilot_core::{CoreStatus, Engine, Event, Msg, NoticeKind};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::notification::Notification;
use gpui_kit::{AnyWindowHandle, App, Context, Entity, Global};

use crate::t;
use crate::tray::Tray;

pub const TRAFFIC_SAMPLES: usize = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Dashboard,
    Proxies,
    Subscriptions,
    Rules,
    Connections,
    Logs,
    Settings,
}

impl Page {
    pub const ALL: [Page; 7] = [
        Page::Dashboard,
        Page::Proxies,
        Page::Subscriptions,
        Page::Rules,
        Page::Connections,
        Page::Logs,
        Page::Settings,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Page::Dashboard => t!("nav.dashboard"),
            Page::Proxies => t!("nav.proxies"),
            Page::Subscriptions => t!("nav.subscriptions"),
            Page::Rules => t!("nav.rules"),
            Page::Connections => t!("nav.connections"),
            Page::Logs => t!("nav.logs"),
            Page::Settings => t!("nav.settings"),
        }
    }

    pub fn index(&self) -> usize {
        Self::ALL.iter().position(|p| p == self).unwrap_or(0)
    }
}

pub struct AppState {
    pub engine: Arc<Engine>,
    pub page: Page,
    pub traffic: VecDeque<Traffic>,
    pub session_up: u64,
    pub session_down: u64,
    /// Bumped whenever new log lines arrive.
    pub log_rev: u64,
    pub window: Option<AnyWindowHandle>,
    pub tray: Option<Tray>,
    pub language: Language,
}

struct GlobalAppState(Entity<AppState>);
impl Global for GlobalAppState {}

pub fn status_text(status: &CoreStatus) -> &'static str {
    match status {
        CoreStatus::Running => t!("status.connected"),
        CoreStatus::Starting => t!("status.connecting"),
        CoreStatus::Stopping => t!("status.disconnecting"),
        CoreStatus::Failed(_) => t!("status.error"),
        CoreStatus::Stopped => t!("status.disconnected"),
    }
}

pub fn notice_text(msg: &Msg) -> String {
    let template = crate::i18n::tr_dyn(msg.key);
    let args: Vec<&dyn std::fmt::Display> = msg.args.iter().map(|a| a as &dyn std::fmt::Display).collect();
    crate::i18n::fill(&template, &args)
}

impl AppState {
    pub fn new(engine: Arc<Engine>, language: Language) -> Self {
        Self {
            engine,
            page: Page::Dashboard,
            traffic: VecDeque::from(vec![Traffic::default(); TRAFFIC_SAMPLES]),
            session_up: 0,
            session_down: 0,
            log_rev: 0,
            window: None,
            tray: None,
            language,
        }
    }

    pub fn install(state: Entity<AppState>, cx: &mut App) {
        cx.set_global(GlobalAppState(state));
    }

    pub fn global(cx: &App) -> Entity<AppState> {
        cx.global::<GlobalAppState>().0.clone()
    }

    pub fn engine(cx: &App) -> Arc<Engine> {
        Self::global(cx).read(cx).engine.clone()
    }

    pub fn set_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            if page == Page::Proxies {
                self.engine.refresh_proxies();
            }
            cx.notify();
        }
    }

    pub fn handle_events(&mut self, events: Vec<Event>, cx: &mut Context<Self>) {
        let mut notices = Vec::new();
        let mut tray_dirty = false;
        for e in events {
            match e {
                Event::Traffic(t) => {
                    if self.traffic.len() >= TRAFFIC_SAMPLES {
                        self.traffic.pop_front();
                    }
                    self.traffic.push_back(t);
                    self.session_up += t.up;
                    self.session_down += t.down;
                }
                Event::Log(_) => self.log_rev += 1,
                Event::Notice(kind, msg) => notices.push((kind, msg)),
                Event::Status | Event::Settings => tray_dirty = true,
                _ => {}
            }
        }
        let language = self.engine.settings().language;
        if language != self.language {
            self.language = language;
            crate::i18n::set_language(language);
            if let Some(tray) = &mut self.tray {
                tray.relabel();
            }
            tray_dirty = true;
            if let Some(window) = self.window {
                let _ = window.update(cx, |_, window, _| window.refresh());
            }
        }
        if tray_dirty {
            self.sync_tray();
        }
        if let Some(window) = self.window {
            for (kind, msg) in notices {
                let text = notice_text(&msg);
                let note = match kind {
                    NoticeKind::Info => Notification::info(text),
                    NoticeKind::Success => Notification::success(text),
                    NoticeKind::Warning => Notification::warning(text),
                    NoticeKind::Error => Notification::error(text),
                };
                let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
            }
        }
        cx.notify();
    }

    pub fn sync_tray(&mut self) {
        let status = self.engine.status();
        let s = self.engine.settings();
        let tooltip = format!(
            "BoxPilot — {} · {}{}",
            status_text(&status),
            crate::ui::widgets::mode_label(s.mode),
            if s.system_proxy && status.is_running() {
                format!(" · {}", t!("tray.sysproxy"))
            } else {
                String::new()
            }
        );
        if let Some(tray) = &mut self.tray {
            tray.sync(status.is_running(), status.is_busy(), s.mode, s.system_proxy, s.tun_enabled, &tooltip);
        }
    }
}
