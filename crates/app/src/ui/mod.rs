pub mod anim;
pub mod assets;
mod connections;
mod dashboard;
mod logs;
mod proxies;
mod rules;
mod settings;
mod subscriptions;
pub mod widgets;

use boxpilot_core::CoreStatus;
use boxpilot_core::model::{Settings, ThemePref};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Theme, ThemeMode, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, App, Bounds, Context, Entity, FontWeight, SharedString, SpringAnimation, Subscription, Window,
    WindowBounds, WindowOptions, div, px, size,
};

use crate::state::{AppState, Page, status_text};
use crate::t;
use widgets::*;

pub fn init(_cx: &mut App) {}

pub fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    match AppState::engine(cx).settings().theme {
        ThemePref::System => Theme::sync_system_appearance(window, cx),
        ThemePref::Light => Theme::change(ThemeMode::Light, window, cx),
        ThemePref::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

pub fn open_main_window(cx: &mut App, show: bool) {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1140.), px(760.)), cx))),
        window_min_size: Some(size(px(900.), px(600.))),
        show,
        focus: show,
        app_id: Some("BoxPilot".into()),
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("BoxPilot".into()),
            ..TitleBar::title_bar_options()
        }),
        ..TitleBar::window_options()
    };
    let result = gpui_kit::open_window(options, cx, |window, cx| {
        apply_theme(Some(window), cx);
        window.on_window_should_close(cx, |window, cx| {
            if AppState::engine(cx).settings().close_to_tray {
                crate::platform::hide_window(window);
            } else {
                quit(cx);
            }
            false
        });
        cx.new(|cx| MainView::new(window, cx))
    });
    match result {
        Ok((handle, _)) => {
            let state = AppState::global(cx);
            state.update(cx, |s, _| s.window = Some(handle));
        }
        Err(e) => log::error!("failed to open window: {e:#}"),
    }
}

pub fn show_main_window(cx: &mut App) {
    let state = AppState::global(cx);
    match state.read(cx).window {
        Some(handle) => {
            let _ = handle.update(cx, |_, window, _| crate::platform::show_window(window));
        }
        None => open_main_window(cx, true),
    }
}

/// Stops the core, restores the system proxy and exits.
pub fn quit(cx: &mut App) {
    AppState::engine(cx).shutdown_blocking();
    cx.quit();
}

/// Asks to restart elevated; `apply` is saved first so it takes effect after the restart.
fn confirm_elevation(body: &'static str, apply: fn(&mut Settings), cx: &mut App) {
    show_main_window(cx);
    let Some(handle) = AppState::global(cx).read(cx).window else { return };
    let _ = handle.update(cx, |_, window, cx| {
        window.open_dialog(cx, move |dialog, _, cx| {
            let muted = cx.theme().muted_foreground;
            dialog
                .title(t!("admin.title"))
                .w(px(480.))
                .child(div().text_sm().text_color(muted).child(body))
                .footer(
                    DialogFooter::new()
                        .child(cancel_button())
                        .child(
                            Button::new("restart-admin")
                                .primary()
                                .label(t!("btn.restart_admin"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    restart_elevated(apply, cx);
                                }),
                        ),
                )
        });
    });
}

/// Enables or disables TUN mode, offering to restart elevated when needed.
pub fn toggle_tun(enable: bool, cx: &mut App) {
    let engine = AppState::engine(cx);
    if !enable || boxpilot_core::sys::is_elevated() {
        engine.update_settings(|s| s.tun_enabled = enable);
        return;
    }
    confirm_elevation(t!("admin.body_tun"), |s| s.tun_enabled = true, cx);
}

pub fn toggle_run_as_admin(enable: bool, cx: &mut App) {
    let engine = AppState::engine(cx);
    if !enable || boxpilot_core::sys::is_elevated() {
        engine.update_settings(|s| s.run_as_admin = enable);
        return;
    }
    confirm_elevation(t!("admin.body_run"), |s| s.run_as_admin = true, cx);
}

fn restart_elevated(apply: fn(&mut Settings), cx: &mut App) {
    let engine = AppState::engine(cx);
    let before = engine.settings();
    engine.update_settings(apply);
    engine.shutdown_blocking();
    match boxpilot_core::sys::relaunch_elevated("--restarted") {
        Ok(()) => cx.quit(),
        Err(e) => {
            engine.update_settings(move |s| *s = before);
            engine.start();
            if let Some(handle) = AppState::global(cx).read(cx).window {
                let _ = handle.update(cx, |_, window, cx| {
                    window.push_notification(
                        gpui_kit::component::notification::Notification::warning(format!("{e:#}")),
                        cx,
                    )
                });
            }
        }
    }
}

/// Footer button that closes the active dialog.
pub(crate) fn cancel_button() -> Button {
    Button::new("dialog-cancel")
        .label(t!("btn.cancel"))
        .on_click(|_, window, cx| window.close_dialog(cx))
}

const NAV_ITEM_H: f32 = 36.0;
const NAV_GAP: f32 = 2.0;

pub struct MainView {
    state: Entity<AppState>,
    dashboard: Entity<dashboard::DashboardPage>,
    proxies: Entity<proxies::ProxiesPage>,
    subscriptions: Entity<subscriptions::SubscriptionsPage>,
    rules: Entity<rules::RulesPage>,
    connections: Entity<connections::ConnectionsPage>,
    logs: Entity<logs::LogsPage>,
    settings: Entity<settings::SettingsPage>,
    last_theme: ThemePref,
    last_page: Page,
    moving_down: bool,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let last_theme = state.read(cx).engine.settings().theme;
        let subs = vec![
            cx.observe_in(&state, window, |this: &mut Self, state, window, cx| {
                let theme = state.read(cx).engine.settings().theme;
                if theme != this.last_theme {
                    this.last_theme = theme;
                    apply_theme(Some(window), cx);
                }
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this: &mut Self, window, cx| {
                if this.last_theme == ThemePref::System {
                    apply_theme(Some(window), cx);
                }
            }),
        ];
        Self {
            dashboard: cx.new(|cx| dashboard::DashboardPage::new(window, cx)),
            proxies: cx.new(|cx| proxies::ProxiesPage::new(window, cx)),
            subscriptions: cx.new(|cx| subscriptions::SubscriptionsPage::new(window, cx)),
            rules: cx.new(|cx| rules::RulesPage::new(window, cx)),
            connections: cx.new(|cx| connections::ConnectionsPage::new(window, cx)),
            logs: cx.new(|cx| logs::LogsPage::new(window, cx)),
            settings: cx.new(|cx| settings::SettingsPage::new(window, cx)),
            state,
            last_theme,
            last_page: Page::Dashboard,
            moving_down: true,
            _subscriptions: subs,
        }
    }

    fn page_icon(p: Page) -> Icon {
        match p {
            Page::Dashboard => IconName::LayoutDashboard.into(),
            Page::Proxies => extra_icon(Lucide::Earth),
            Page::Subscriptions => extra_icon(Lucide::Rss),
            Page::Rules => extra_icon(Lucide::Route),
            Page::Connections => extra_icon(Lucide::Cable),
            Page::Logs => extra_icon(Lucide::ScrollText),
            Page::Settings => IconName::Settings.into(),
        }
    }

    fn render_sidebar(&self, page: Page, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let status = engine.status();
        let t = cx.theme();
        let status_color = match &status {
            CoreStatus::Running => t.success,
            CoreStatus::Starting | CoreStatus::Stopping => t.warning,
            CoreStatus::Failed(_) => t.danger,
            CoreStatus::Stopped => t.muted_foreground,
        };
        let version = engine
            .core_version()
            .map(|v| format!("sing-box {v}"))
            .unwrap_or_else(|| t!("core.not_installed_short").to_string());
        let sub_count = engine.subscriptions().len();
        let rule_count = engine.rule_providers().len();

        // The selection indicator is a box whose top and bottom edges are two
        // springs. The leading edge is stiffer than the trailing one, so the box
        // stretches in the direction of travel and relaxes as it settles.
        let n = Page::ALL.len();
        let idx = page.index();
        let step = NAV_ITEM_H + NAV_GAP;
        let top = idx as f32 * step;
        let bottom = (n - 1 - idx) as f32 * step;
        let (top_cfg, bottom_cfg) = if self.moving_down {
            (anim::TRAILING, anim::SNAPPY)
        } else {
            (anim::SNAPPY, anim::TRAILING)
        };
        let indicator = div()
            .absolute()
            .left_0()
            .right_0()
            .rounded(t.radius)
            .bg(t.sidebar_accent)
            .with_spring("nav-top", SpringAnimation::new(top_cfg).to(px(top)), |d, v| d.top(v))
            .with_spring("nav-bottom", SpringAnimation::new(bottom_cfg).to(px(bottom)), |e, v| {
                e.map_element(|d| d.bottom(v))
            });
        let bar = div()
            .absolute()
            .left_0()
            .w(px(3.))
            .h(px(16.))
            .rounded_full()
            .bg(t.primary)
            .with_spring(
                "nav-bar",
                SpringAnimation::new(anim::BOUNCY).to(px(top + (NAV_ITEM_H - 16.0) / 2.0)),
                |d, v| d.top(v),
            );

        let items = Page::ALL.map(|p| {
            let active = p == page;
            let state = self.state.clone();
            let badge = match p {
                Page::Subscriptions if sub_count > 0 => Some(sub_count),
                Page::Rules if rule_count > 0 => Some(rule_count),
                _ => None,
            };
            h_flex()
                .id(("nav", p.index()))
                .h(px(NAV_ITEM_H))
                .px_3()
                .gap_2p5()
                .items_center()
                .rounded(t.radius)
                .cursor_pointer()
                .text_sm()
                .when(active, |d| d.text_color(t.sidebar_accent_foreground).font_weight(FontWeight::MEDIUM))
                .when(!active, |d| {
                    d.text_color(t.muted_foreground)
                        .hover(|d| d.text_color(t.sidebar_foreground).bg(t.sidebar_accent.opacity(0.4)))
                })
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_page(p, cx)))
                .child(Self::page_icon(p).size(px(16.)))
                .child(div().flex_1().min_w_0().truncate().child(p.title()))
                .when_some(badge, |d, n| {
                    d.child(div().flex_none().text_xs().text_color(t.muted_foreground).child(n.to_string()))
                })
        });

        let busy = status.is_busy();
        let status_dot = if busy {
            // Breathing dot: simple harmonic motion while the core is changing state.
            dot(status_color)
                .with_animation(
                    "status-breathe",
                    Animation::new(std::time::Duration::from_millis(1200))
                        .repeat()
                        .with_easing(|t| 0.5 - 0.5 * (t * std::f32::consts::TAU).cos()),
                    |d, v| d.opacity(0.35 + 0.65 * v),
                )
                .into_any_element()
        } else {
            dot(status_color).into_any_element()
        };

        v_flex()
            .w(px(216.))
            .h_full()
            .flex_none()
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.sidebar_border)
            .child(
                h_flex()
                    .px_4()
                    .pt_3()
                    .pb_4()
                    .gap_2p5()
                    .items_center()
                    .child(
                        div()
                            .size(px(30.))
                            .flex_none()
                            .rounded(px(8.))
                            .bg(t.primary)
                            .text_color(t.primary_foreground)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(extra_icon(Lucide::Waypoints).size(px(16.))),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("BoxPilot"))
                            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(version)),
                    ),
            )
            .child(
                div().px_2().child(
                    v_flex()
                        .relative()
                        .gap(px(NAV_GAP))
                        .child(indicator)
                        .child(bar)
                        .children(items),
                ),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .mx_2()
                    .mb_3()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .rounded(t.radius)
                    .bg(t.sidebar_accent.opacity(0.5))
                    .child(status_dot)
                    .child(div().text_sm().min_w_0().truncate().child(status_text(&status))),
            )
    }
}

impl Render for MainView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.state.read(cx).page;
        if page != self.last_page {
            self.moving_down = page.index() > self.last_page.index();
            self.last_page = page;
        }
        let t = cx.theme();
        let content: gpui_kit::AnyView = match page {
            Page::Dashboard => self.dashboard.clone().into(),
            Page::Proxies => self.proxies.clone().into(),
            Page::Subscriptions => self.subscriptions.clone().into(),
            Page::Rules => self.rules.clone().into(),
            Page::Connections => self.connections.clone().into(),
            Page::Logs => self.logs.clone().into(),
            Page::Settings => self.settings.clone().into(),
        };
        let full_height = matches!(page, Page::Connections | Page::Logs);
        let title_fg = t.muted_foreground;
        let bg = t.background;
        let fg = t.foreground;

        // Each page mounts under its own id, so switching pages replays the
        // entrance spring.
        let body = if full_height {
            anim::enter(
                v_flex().flex_1().min_h_0().w_full().px_6().py_5().child(content),
                ("page", page.index()),
                0.0,
            )
            .into_any_element()
        } else {
            div()
                .id(("content", page.index()))
                .size_full()
                .overflow_y_scrollbar()
                .child(anim::enter(
                    div().w_full().px_6().py_5().max_w(px(1180.)).child(content),
                    ("page", page.index()),
                    0.0,
                ))
                .into_any_element()
        };

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .text_color(title_fg)
                        .child(SharedString::from(page.title())),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .child(self.render_sidebar(page, cx))
                    .child(v_flex().flex_1().min_w_0().h_full().child(body)),
            )
    }
}
