mod accent;
pub mod anim;
pub mod layout;
pub mod assets;
mod chrome;
mod connections;
mod dashboard;
mod logs;
mod proxies;
mod rules;
mod settings;
pub mod smooth_scroll;
mod subscriptions;
pub mod widgets;

use boxpilot_core::CoreStatus;
use boxpilot_core::model::{Backdrop, Settings, ThemePref};
use gpui_kit::AnimationExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Theme, ThemeMode, TitleBar, WindowExt as _, h_flex, v_flex, window_border,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, App, Context, Entity, FontWeight, ScrollHandle, SpringAnimation, Subscription, Window,
    WindowBackgroundAppearance, WindowOptions, div,
};

use crate::state::{AppState, Page, status_text};
use crate::t;
use widgets::*;
use layout::dp;

pub fn init(_cx: &mut App) {}

pub fn apply_theme(window: Option<&mut Window>, cx: &mut App) {
    let pref = AppState::engine(cx).settings().theme;
    match window {
        Some(window) => {
            match pref {
                ThemePref::System => Theme::sync_system_appearance(Some(window), cx),
                ThemePref::Light => Theme::change(ThemeMode::Light, Some(window), cx),
                ThemePref::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
            }
            // Keep the native frame and Mica tint in step with the app theme.
            let dark = cx.theme().is_dark();
            crate::platform::set_dark_frame(window, dark);
            apply_backdrop(&AppState::engine(cx).settings(), dark, window);
        }
        None => match pref {
            ThemePref::System => Theme::sync_system_appearance(None, cx),
            ThemePref::Light => Theme::change(ThemeMode::Light, None, cx),
            ThemePref::Dark => Theme::change(ThemeMode::Dark, None, cx),
        },
    }
    let settings = AppState::engine(cx).settings();
    // Theme changes reset the colours; put the accent back on top.
    accent::apply(&settings, cx);
    layout::apply_scale(&settings, cx);
}

pub fn platform_mica_supported() -> bool {
    crate::platform::mica_supported()
}

/// Whether the window is translucent (Windows 11 22H2+ and enabled).
pub fn mica_active(settings: &Settings) -> bool {
    settings.mica && crate::platform::mica_supported()
}

fn backdrop(settings: &Settings) -> WindowBackgroundAppearance {
    if !mica_active(settings) {
        return WindowBackgroundAppearance::Opaque;
    }
    // macOS vibrancy / KWin blur: one material, whatever was picked on Windows.
    if crate::platform::translucency() == crate::platform::Translucency::Blur {
        return WindowBackgroundAppearance::Blurred;
    }
    match settings.backdrop {
        Backdrop::Mica => WindowBackgroundAppearance::MicaBackdrop,
        Backdrop::MicaAlt => WindowBackgroundAppearance::MicaAltBackdrop,
        // Switched to Acrylic through DWM right after; GPUI's own blur is an
        // untinted accent effect that also lingers over later materials.
        Backdrop::Acrylic => WindowBackgroundAppearance::MicaBackdrop,
    }
}

/// Applies the backdrop material, the frame it needs and surface opacity.
fn apply_backdrop(settings: &Settings, dark: bool, window: &mut Window) {
    let mut active = mica_active(settings);
    if active && cfg!(target_os = "macos") {
        // Vibrancy always follows the system appearance; under a forced theme
        // of the other kind it would clash with the content, so stay opaque.
        let system_dark = matches!(window.appearance(), gpui_kit::WindowAppearance::Dark | gpui_kit::WindowAppearance::VibrantDark);
        active = system_dark == dark;
    }
    // Going through Opaque first clears an accent blur left by an earlier
    // material or build; it would sit over the DWM backdrop and hide it.
    window.set_background_appearance(WindowBackgroundAppearance::Opaque);
    window.set_background_appearance(if active { backdrop(settings) } else { WindowBackgroundAppearance::Opaque });
    // DWM paints its backdrops only in the frame; extend it over the window.
    crate::platform::set_mica_frame(window, active);
    // Windows would swap the backdrop for a flat colour whenever the window
    // is not focused; keep it as it is.
    crate::platform::keep_backdrop_live(window, active);
    if active && crate::platform::translucency() == crate::platform::Translucency::WindowsMaterials {
        // Every material is a DWM system backdrop, tinted by Windows to match
        // light or dark mode (the dark-frame flag), so switching is clean.
        let kind = match settings.backdrop {
            Backdrop::Mica => 2,
            Backdrop::Acrylic => 3,
            Backdrop::MicaAlt => 4,
        };
        crate::platform::set_system_backdrop(window, kind);
    }
    let _ = dark;
    layout::set_surface_alpha(active, settings.transparency);
}

pub fn open_main_window(cx: &mut App, show: bool) {
    let settings = AppState::engine(cx).settings();
    // Fits the display it opens on; reopens where it was last time.
    let (bounds, min_size) = layout::initial_bounds(&settings, cx);
    let options = WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(min_size),
        show,
        focus: show,
        app_id: Some("BoxPilot".into()),
        // The OS title bar is hidden; the native frame (shadow, rounded
        // corners, resize borders, snap) is kept and our caption area maps
        // onto it. On macOS AppKit keeps the traffic lights and drag region.
        titlebar: Some(gpui_kit::TitlebarOptions {
            title: Some("BoxPilot".into()),
            appears_transparent: true,
            traffic_light_position: Some(gpui_kit::point(dp(14.), dp(14.))),
        }),
        app_owns_titlebar_drag: false,
        window_background: backdrop(&settings),
        ..TitleBar::window_options()
    };
    // Like gpui_kit::open_window, but the root surface is transparent: the
    // component layer fills it with the theme background, which would cover
    // the Mica backdrop. MainView paints everything itself when Mica is off.
    let result = cx.open_window(options, |window, cx| {
        apply_theme(Some(window), cx);
        window.on_window_should_close(cx, handle_close_request);
        let view = cx.new(|cx| MainView::new(window, cx));
        cx.new(|cx| gpui_kit::base::Root::new(view, window, cx).bg(gpui_kit::transparent_black()))
    });
    match result {
        Ok(handle) => {
            let state = AppState::global(cx);
            state.update(cx, |s, _| s.window = Some(handle.into()));
        }
        Err(e) => log::error!("failed to open window: {e:#}"),
    }
}

pub fn show_main_window(cx: &mut App) {
    let state = AppState::global(cx);
    let handle = state.read(cx).window;
    let shown = handle.is_some_and(|h| {
        h.update(cx, |_, window, cx| crate::platform::show_window(window, cx)).is_ok()
    });
    if !shown {
        // The window was closed (lightweight mode, Linux close-to-tray); open a fresh one.
        open_main_window(cx, true);
    }
}

/// Whether closing the window tears down the UI instead of hiding it.
pub fn lightweight(settings: &Settings) -> bool {
    settings.close_to_tray && settings.lightweight
}

/// The window's close button: hide to the tray or quit. Returns whether the
/// window should actually be destroyed.
pub fn handle_close_request(window: &mut Window, cx: &mut App) -> bool {
    let saved = layout::saved_window(window);
    AppState::engine(cx).update_settings(move |s| s.window = Some(saved));
    let settings = AppState::engine(cx).settings();
    if !settings.close_to_tray {
        quit(cx);
        false
    } else if lightweight(&settings) {
        enter_lightweight(cx);
        true
    } else {
        crate::platform::hide_to_tray(window, cx)
    }
}

/// Lightweight mode: the window, its renderer and every page are dropped;
/// the core, tray, scheduler and system proxy keep running. The tray or a
/// second launch opens a fresh window.
fn enter_lightweight(cx: &mut App) {
    log::info!("window closed: lightweight mode");
    AppState::global(cx).update(cx, |s, _| s.window = None);
    // Give the window time to be torn down before handing memory back.
    cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
        cx.background_executor().timer(std::time::Duration::from_secs(2)).await;
        let closed = cx.update(|cx| AppState::global(cx).read(cx).window.is_none());
        if closed {
            crate::platform::trim_memory();
        }
    })
    .detach();
}

/// The app icon as an image, drawn from the same shape as the .exe icon.
fn app_icon() -> std::sync::Arc<gpui_kit::Image> {
    static ICON: std::sync::OnceLock<std::sync::Arc<gpui_kit::Image>> = std::sync::OnceLock::new();
    ICON.get_or_init(|| {
        std::sync::Arc::new(gpui_kit::Image::from_bytes(gpui_kit::ImageFormat::Svg, crate::icon_art::SVG.as_bytes().to_vec()))
    })
    .clone()
}

/// Starts a fresh instance and exits (used to apply start-up settings).
pub fn restart_app(cx: &mut App) {
    remember_window(cx);
    AppState::engine(cx).shutdown_blocking();
    // After an in-place update this is the new executable.
    if let Some(exe) = boxpilot_core::update::restart_target() {
        if let Err(e) = std::process::Command::new(exe).arg("--restarted").spawn() {
            log::error!("restart failed: {e:#}");
            return;
        }
    }
    cx.quit();
}

/// Clears the chosen data and starts a fresh instance. Unlike a normal
/// restart nothing is saved on the way out, not even the window position.
pub fn reset_app(scope: boxpilot_core::ResetScope, cx: &mut App) {
    AppState::engine(cx).reset_blocking(scope);
    if let Some(exe) = boxpilot_core::update::restart_target() {
        if let Err(e) = std::process::Command::new(exe).arg("--restarted").spawn() {
            log::error!("restart after reset failed: {e:#}");
        }
    }
    cx.quit();
}

/// Saves the window's size and position for the next start.
fn remember_window(cx: &mut App) {
    let Some(handle) = AppState::global(cx).read(cx).window else { return };
    if let Ok(saved) = handle.update(cx, |_, window, _| layout::saved_window(window)) {
        AppState::engine(cx).update_settings(move |s| s.window = Some(saved));
    }
}

/// Stops the core, restores the system proxy and exits.
pub fn quit(cx: &mut App) {
    remember_window(cx);
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
                .w(dp(480.))
                .child(div().text_sm().text_color(muted).child(body))
                .footer(
                    DialogFooter::new().child(cancel_button()).child(
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

/// Enables or disables TUN mode. On Windows TUN needs an elevated app; on
/// macOS / Linux the core binary is granted the privilege instead.
pub fn toggle_tun(enable: bool, cx: &mut App) {
    let engine = AppState::engine(cx);
    if !enable || engine.tun_permitted() {
        engine.update_settings(|s| s.tun_enabled = enable);
        return;
    }
    if cfg!(windows) {
        confirm_elevation(t!("admin.body_tun"), |s| s.tun_enabled = true, cx);
    } else {
        grant_core_privilege(Some(|s: &mut Settings| s.tun_enabled = true), cx);
    }
}

pub fn toggle_run_as_admin(enable: bool, cx: &mut App) {
    let engine = AppState::engine(cx);
    if !cfg!(windows) {
        // macOS / Linux: "administrator" means a privileged core binary.
        if enable && !engine.tun_permitted() {
            grant_core_privilege(Some(|s: &mut Settings| s.run_as_admin = true), cx);
        } else {
            engine.update_settings(|s| s.run_as_admin = enable);
        }
        return;
    }
    if !enable || boxpilot_core::sys::is_elevated() {
        engine.update_settings(|s| s.run_as_admin = enable);
        return;
    }
    confirm_elevation(t!("admin.body_run"), |s| s.run_as_admin = true, cx);
}

/// macOS / Linux: asks the system for an administrator password to let the
/// core create TUN interfaces (setuid on macOS, capabilities on Linux).
fn grant_core_privilege(apply: Option<fn(&mut Settings)>, cx: &mut App) {
    let engine = AppState::engine(cx);
    let window = AppState::global(cx).read(cx).window;
    let task = engine.runtime().spawn({
        let engine = engine.clone();
        async move { engine.grant_tun_privilege().await }
    });
    cx.spawn(async move |cx: &mut gpui_kit::AsyncApp| {
        let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
        cx.update(|cx| match result {
            Ok(()) => {
                if let Some(apply) = apply {
                    engine.update_settings(apply);
                }
            }
            Err(e) => {
                if let Some(handle) = window {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.push_notification(
                            gpui_kit::component::notification::Notification::warning(format!("{e:#}")),
                            cx,
                        )
                    });
                }
            }
        });
    })
    .detach();
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
    /// One scroll position per page, kept while switching pages.
    scroll: Vec<ScrollHandle>,
    last_theme: ThemePref,
    last_accent: boxpilot_core::model::Accent,
    /// Translucency, material and transparency last applied to the window.
    last_look: (bool, Backdrop, u32),
    last_page: Page,
    moving_down: bool,
    _subscriptions: Vec<Subscription>,
}

impl MainView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let settings = state.read(cx).engine.settings();
        let subs = vec![
            cx.observe_in(&state, window, |this: &mut Self, state, window, cx| {
                let settings = state.read(cx).engine.settings();
                if settings.theme != this.last_theme || settings.accent != this.last_accent {
                    this.last_theme = settings.theme;
                    this.last_accent = settings.accent;
                    apply_theme(Some(window), cx);
                }
                if layout::apply_scale(&settings, cx) {
                    window.refresh();
                }
                let look = (mica_active(&settings), settings.backdrop, settings.transparency);
                if look != this.last_look {
                    this.last_look = look;
                    apply_backdrop(&settings, cx.theme().is_dark(), window);
                    // Text boxes are restyled for (or back from) a translucent window.
                    accent::apply(&settings, cx);
                    window.refresh();
                }
                cx.notify();
            }),
            // The OS text size may have changed while the window was in the background.
            cx.observe_window_activation(window, |this: &mut Self, window, cx| {
                if window.is_window_active() {
                    layout::refresh_system_scale();
                    let settings = this.state.read(cx).engine.settings();
                    if layout::apply_scale(&settings, cx) {
                        window.refresh();
                    }
                }
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
            scroll: Page::ALL.iter().map(|_| ScrollHandle::new()).collect(),
            state,
            last_theme: settings.theme,
            last_accent: settings.accent,
            last_look: (mica_active(&settings), settings.backdrop, settings.transparency),
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

    fn render_sidebar(&self, page: Page, mica: bool, m: layout::Metrics, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = m.compact_nav;
        // Linux desktops may put the window buttons on the left (button-layout).
        let left_controls = chrome::window_controls(chrome::Side::Left, window, cx);
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
        let accent = if mica { t.sidebar_accent.opacity(0.85) } else { t.sidebar_accent };
        let indicator = div()
            .absolute()
            .left_0()
            .right_0()
            .rounded(t.radius)
            .bg(accent)
            .with_spring("nav-top", SpringAnimation::new(top_cfg).to(dp(top)), |d, v| d.top(v))
            .with_spring("nav-bottom", SpringAnimation::new(bottom_cfg).to(dp(bottom)), |e, v| {
                e.map_element(|d| d.bottom(v))
            });
        let bar = div()
            .absolute()
            .left_0()
            .w(dp(3.))
            .h(dp(16.))
            .rounded_full()
            .bg(t.primary)
            .with_spring(
                "nav-bar",
                SpringAnimation::new(anim::BOUNCY).to(dp(top + (NAV_ITEM_H - 16.0) / 2.0)),
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
            let title = p.title();
            h_flex()
                .id(("nav", p.index()))
                .h(dp(NAV_ITEM_H))
                .px_3()
                .gap_2p5()
                .items_center()
                .when(compact, |d| {
                    d.justify_center()
                        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(title).build(window, cx))
                })
                .rounded(t.radius)
                .cursor_pointer()
                .text_sm()
                .when(active, |d| d.text_color(t.sidebar_accent_foreground).font_weight(FontWeight::MEDIUM))
                .when(!active, |d| {
                    d.text_color(t.muted_foreground)
                        .hover(|d| d.text_color(t.sidebar_foreground).bg(t.sidebar_accent.opacity(0.4)))
                })
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_page(p, cx)))
                .child(Self::page_icon(p).size(dp(16.)))
                .when(!compact, |d| d.child(div().flex_1().min_w_0().truncate().child(title)))
                .when_some(badge.filter(|_| !compact), |d, n| {
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
            .w(dp(m.sidebar))
            .h_full()
            .flex_none()
            .when(!mica, |d| d.bg(t.sidebar))
            .when_some(left_controls, |d, controls| {
                d.child(
                    h_flex()
                        .h(gpui_kit::px(chrome::CAPTION_H))
                        .child(controls)
                        .child(chrome::drag_region("drag-left-controls").flex_1().h_full()),
                )
            })
            // The logo block doubles as a window drag handle (it has no controls).
            .child(
                chrome::drag_region("drag-sidebar")
                    .pt(gpui_kit::px(chrome::SIDEBAR_TOP_INSET))
                    .child(
                        h_flex()
                            .px_4()
                            .when(compact, |d| d.px_0().justify_center())
                            .pt_3()
                            .pb_4()
                            .gap_2p5()
                            .items_center()
                            // The app icon itself, as on the taskbar and the .exe.
                            .child(gpui_kit::img(app_icon()).size(dp(30.)).flex_none())
                            .when(!compact, |d| d.child(
                                v_flex()
                                    .min_w_0()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child("BoxPilot"))
                                    .child(div().text_xs().text_color(t.muted_foreground).truncate().child(version)),
                            )),
                    ),
            )
            .child(
                div().px_2().child(
                    v_flex()
                        .relative()
                        .gap(dp(NAV_GAP))
                        .child(indicator)
                        .child(bar)
                        .children(items),
                ),
            )
            .child(chrome::drag_region("drag-sidebar-fill").flex_1())
            .child(
                h_flex()
                    .mx_2()
                    .mb_3()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .rounded(t.radius)
                    .bg(accent.opacity(0.5))
                    .when(compact, |d| d.justify_center().px_0())
                    .child(status_dot)
                    .when(!compact, |d| d.child(div().text_sm().min_w_0().truncate().child(status_text(&status)))),
            )
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = self.state.read(cx).page;
        if page != self.last_page {
            self.moving_down = page.index() > self.last_page.index();
            self.last_page = page;
        }
        // What was applied (macOS may fall back to opaque), not just the setting.
        let mica = layout::translucent();
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
        let (bg, fg, sidebar_bg) = (t.background, t.foreground, t.sidebar);
        let sheet_edge = hairline(cx);
        let layer_fill = layer(cx);
        // Acrylic (and the single blur elsewhere) is drawn as one pane; Mica
        // keeps Windows 11's content layer.
        let glass = mica
            && (self.last_look.1 == Backdrop::Acrylic
                || crate::platform::translucency() == crate::platform::Translucency::Blur);
        // Floors keep text readable: KWin's blur has no tint of its own, and a
        // light pane over dark windows behind would otherwise turn grey.
        let glass_alpha = if cfg!(target_os = "linux") {
            layout::surface_alpha().max(0.6)
        } else if !t.is_dark() {
            layout::surface_alpha().max(0.5)
        } else {
            layout::surface_alpha()
        };

        let m = layout::metrics(window);
        let top_pad = 20.0;
        // Each page mounts under its own id, so switching pages replays the
        // entrance spring; the scroll position of each page is kept.
        let body = if full_height {
            anim::enter(
                v_flex().flex_1().min_h_0().w_full().px(dp(m.inset)).pb_5().pt(dp(top_pad)).child(content),
                ("page", page.index()),
                0.0,
            )
            .into_any_element()
        } else {
            let handle = self.scroll[page.index()].clone();
            smooth_scroll::with_scrollbar(
                ("page-scrollbar", page.index()),
                &handle,
                smooth_scroll::smooth_scroll(
                    ("smooth", page.index()),
                    &handle,
                    div()
                        .id(("content", page.index()))
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&handle)
                        .child(anim::enter(
                            div().w_full().px(dp(m.inset)).pb_5().pt(dp(top_pad)).child(content),
                            ("page", page.index()),
                            0.0,
                        )),
                ),
            )
            .size_full()
            .into_any_element()
        };

        let caption = chrome::caption_bar(window, cx);
        window_border().child(
            h_flex()
                .size_full()
                .text_color(fg)
                .when(!mica, |d| d.bg(sidebar_bg))
                // Acrylic and the macOS/KDE blur are one even pane: Transparency
                // sets a single tint over the whole window (KWin's blur has no
                // tint of its own, so it keeps a floor for readable text).
                .when(glass, |d| d.bg(bg.opacity(glass_alpha)))
                .child(self.render_sidebar(page, mica, m, window, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(caption)
                        .child(
                            v_flex()
                                .flex_1()
                                .min_h_0()
                                // The content sits on a raised layer, as in Windows 11
                                // apps; on glass (Acrylic, macOS, KDE) it is one pane.
                                .when(!glass, |d| {
                                    d.bg(layer_fill)
                                        .border_l_1()
                                        .border_t_1()
                                        .border_color(sheet_edge)
                                        .rounded_tl(dp(8.))
                                })
                                .child(body),
                        ),
                ),
        )
    }
}
