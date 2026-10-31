use boxpilot_core::CoreDownloadState;
use boxpilot_core::model::{Accent, Backdrop, GpuPreference, Language, Settings, ThemePref, TunStack, UpdateMode};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Context, Entity, Focusable as _, FontWeight, SharedString, Subscription, Window, div};

use super::anim;
use super::widgets::*;
use crate::state::AppState;
use crate::t;
use super::layout::dp;

type Getter = fn(&Settings) -> String;
type Setter = fn(&mut Settings, &str) -> Result<(), &'static str>;

struct Field {
    input: Entity<InputState>,
    get: Getter,
    set: Setter,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum F {
    CorePath,
    Mirror,
    MixedPort,
    Bypass,
    TunMtu,
    DnsRemote,
    DnsDirect,
    UserAgent,
    SubInterval,
    SubTimeout,
    ApiPort,
    TestUrl,
    TestInterval,
}

const FIELDS: [F; 13] = [
    F::CorePath,
    F::Mirror,
    F::MixedPort,
    F::Bypass,
    F::TunMtu,
    F::DnsRemote,
    F::DnsDirect,
    F::UserAgent,
    F::SubInterval,
    F::SubTimeout,
    F::ApiPort,
    F::TestUrl,
    F::TestInterval,
];

fn parse_port(v: &str) -> Result<u16, &'static str> {
    v.trim().parse::<u16>().ok().filter(|p| *p >= 1).ok_or(t!("err.port"))
}

fn parse_num<T: std::str::FromStr + PartialOrd>(v: &str, min: T, max: T) -> Result<T, &'static str> {
    v.trim().parse::<T>().ok().filter(|n| *n >= min && *n <= max).ok_or(t!("err.invalid"))
}

fn required(v: &str) -> Result<String, &'static str> {
    let v = v.trim();
    if v.is_empty() { Err(t!("err.required")) } else { Ok(v.to_string()) }
}

fn spec(f: F) -> (Getter, Setter, String) {
    match f {
        F::CorePath => (|s| s.core_path.clone(), |s, v| { s.core_path = v.trim().trim_matches('"').into(); Ok(()) }, t!("set.core_path_ph").into()),
        F::Mirror => (|s| s.github_mirror.clone(), |s, v| { s.github_mirror = v.trim().into(); Ok(()) }, "https://ghfast.top/".into()),
        F::MixedPort => (|s| s.mixed_port.to_string(), |s, v| {
            let p = parse_port(v)?;
            if p == s.clash_api_port { return Err(t!("err.port_api")); }
            s.mixed_port = p;
            Ok(())
        }, "7890".into()),
        F::Bypass => (|s| s.sysproxy_bypass.clone(), |s, v| { s.sysproxy_bypass = v.trim().into(); Ok(()) }, "localhost;127.*;<local>".into()),
        F::TunMtu => (|s| s.tun_mtu.to_string(), |s, v| { s.tun_mtu = parse_num(v, 1280u32, 65535)?; Ok(()) }, "9000".into()),
        F::DnsRemote => (|s| s.dns_remote.clone(), |s, v| { s.dns_remote = required(v)?; Ok(()) }, "https://1.1.1.1/dns-query".into()),
        F::DnsDirect => (|s| s.dns_direct.clone(), |s, v| { s.dns_direct = required(v)?; Ok(()) }, "https://223.5.5.5/dns-query".into()),
        F::UserAgent => (|s| s.sub_user_agent.clone(), |s, v| {
            s.sub_user_agent = if v.trim().is_empty() { "clash.meta".into() } else { v.trim().into() };
            Ok(())
        }, "clash.meta".into()),
        F::SubInterval => (|s| format!("{}", s.sub_default_interval_min as f64 / 60.0).trim_end_matches(".0").to_string(), |s, v| {
            let h: f64 = parse_num(v, 0.0, 24.0 * 30.0)?;
            s.sub_default_interval_min = (h * 60.0).round() as u32;
            Ok(())
        }, "12".into()),
        F::SubTimeout => (|s| s.sub_timeout_secs.to_string(), |s, v| { s.sub_timeout_secs = parse_num(v, 5u64, 300)?; Ok(()) }, "30".into()),
        F::ApiPort => (|s| s.clash_api_port.to_string(), |s, v| {
            let p = parse_port(v)?;
            if p == s.mixed_port { return Err(t!("err.port_proxy")); }
            s.clash_api_port = p;
            Ok(())
        }, "9090".into()),
        F::TestUrl => (|s| s.urltest_url.clone(), |s, v| {
            if !v.trim().starts_with("http") { return Err(t!("err.http_url")); }
            s.urltest_url = v.trim().into();
            Ok(())
        }, "https://www.gstatic.com/generate_204".into()),
        F::TestInterval => (|s| s.urltest_interval_min.to_string(), |s, v| { s.urltest_interval_min = parse_num(v, 1u32, 1440)?; Ok(()) }, "10".into()),
    }
}

pub struct SettingsPage {
    state: Entity<AppState>,
    fields: Vec<(F, Field)>,
    /// Text input width for the current window size (dp).
    input_w: f32,
    _subs: Vec<Subscription>,
}

impl SettingsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let settings = state.read(cx).engine.settings();
        let mut subs = vec![cx.observe_in(&state, window, |this: &mut Self, _, window, cx| {
            this.sync_inputs(window, cx);
            cx.notify();
        })];
        let mut fields = Vec::new();
        for f in FIELDS {
            let (get, set, placeholder) = spec(f);
            let value = get(&settings);
            let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value));
            subs.push(cx.subscribe_in(&input, window, move |this: &mut Self, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    this.commit(f, window, cx);
                }
            }));
            fields.push((f, Field { input, get, set }));
        }
        Self { state, fields, input_w: 300.0, _subs: subs }
    }

    fn field(&self, f: F) -> &Field {
        &self.fields.iter().find(|(k, _)| *k == f).unwrap().1
    }

    fn commit(&mut self, f: F, window: &mut Window, cx: &mut Context<Self>) {
        let engine = self.state.read(cx).engine.clone();
        let current = engine.settings();
        let field = self.field(f);
        let value = field.input.read(cx).value().to_string();
        if value == (field.get)(&current) {
            return;
        }
        let mut next = current.clone();
        match (field.set)(&mut next, &value) {
            Ok(()) => engine.update_settings(move |s| *s = next),
            Err(msg) => {
                window.push_notification(Notification::error(msg), cx);
                let old = (field.get)(&current);
                field.input.update(cx, |i, cx| i.set_value(old, window, cx));
            }
        }
    }

    /// Reflects settings changed elsewhere in inputs that are not being edited.
    fn sync_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.state.read(cx).engine.settings();
        for (f, field) in &self.fields {
            let focused = field.input.read(cx).focus_handle(cx).is_focused(window);
            let want = (field.get)(&settings);
            if !focused && field.input.read(cx).value() != want.as_str() {
                field.input.update(cx, |i, cx| i.set_value(want, window, cx));
            }
            // The hint follows the language too.
            let hint = spec(*f).2;
            if field.input.read(cx).presentation().placeholder().as_ref() != hint {
                field.input.update(cx, |i, cx| i.set_placeholder(hint, window, cx));
            }
        }
    }

    fn input(&self, f: F) -> impl IntoElement {
        div().w(dp(self.input_w)).child(Input::new(&self.field(f).input).small())
    }

    fn core_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let version = engine.core_version();
        let installed = engine.core_installed();
        let state = engine.download_state();
        let settings = engine.settings();
        let busy = matches!(
            state,
            CoreDownloadState::Checking | CoreDownloadState::Downloading { .. } | CoreDownloadState::Installing
        );
        let (e1, e2, e3) = (engine.clone(), engine.clone(), engine.clone());
        let status_line: String = match (&state, version) {
            (CoreDownloadState::Checking, _) => t!("core.checking").into(),
            (CoreDownloadState::Available(v), _) => t!("core.available", v),
            (CoreDownloadState::UpToDate(v), _) => t!("core.uptodate", v),
            (CoreDownloadState::Downloading { done, total }, _) => t!("core.downloading", fmt_bytes(*done), fmt_bytes(*total)),
            (CoreDownloadState::Installing, _) => t!("core.installing").into(),
            (CoreDownloadState::Failed(e), _) => e.clone(),
            (CoreDownloadState::Idle, Some(_)) => t!("core.installed").into(),
            (CoreDownloadState::Idle, None) => t!("core.not_installed_long").into(),
        };
        let failed = matches!(state, CoreDownloadState::Failed(_));
        let install_label = match (&state, installed) {
            (CoreDownloadState::Available(v), true) => t!("btn.update_to", v),
            (_, true) => t!("btn.reinstall").into(),
            (_, false) => t!("btn.download_latest").into(),
        };
        let core_dir = engine.paths().core_dir.clone();
        let progress = match state {
            CoreDownloadState::Downloading { done, total } if total > 0 => Some(done as f32 / total as f32 * 100.0),
            _ => None,
        };
        card(cx)
            .gap_1()
            .child(section_title(t!("set.core"), cx))
            .child(
                h_flex()
                    .gap_3()
                    .py_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        div()
                            .size(dp(40.))
                            .flex_none()
                            .rounded(t.radius)
                            .bg(if installed { t.success.opacity(0.12) } else { t.warning.opacity(0.12) })
                            .text_color(if installed { t.success } else { t.warning })
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(IconName::Cpu),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(dp(180.))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(version.map(|v| format!("sing-box {v}")).unwrap_or_else(|| t!("core.not_installed").into())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(if failed { t.danger } else { t.muted_foreground })
                                    .line_clamp(2)
                                    .child(status_line),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_2()
                            .child(
                                Button::new("check-core")
                                    .small()
                                    .label(t!("btn.check_updates"))
                                    .disabled(busy)
                                    .on_click(move |_, _, _| e1.check_core_update()),
                            )
                            .child(
                                Button::new("install-core")
                                    .small()
                                    .icon(extra_icon(Lucide::Download))
                                    .label(install_label)
                                    .loading(busy)
                                    .disabled(busy)
                                    .on_click(move |_, _, _| e2.install_core()),
                            ),
                    ),
            )
            .when_some(progress, |d, pct| {
                let c = cx.theme().primary;
                d.child(progress_bar("core-progress", pct, c, cx))
            })
            .child(divider(cx))
            .child(setting_row(
                t!("set.prerelease"),
                Some(t!("set.prerelease_desc")),
                Toggle::new("prerelease").checked(settings.core_prerelease).on_change(move |v, _, _| {
                    let v = *v;
                    e3.update_settings(|s| s.core_prerelease = v)
                }),
                cx,
            ))
            .child(setting_row(t!("set.mirror"), Some(t!("set.mirror_desc")), self.input(F::Mirror), cx))
            .child(setting_row(t!("set.core_path"), Some(t!("set.core_path_desc")), self.input(F::CorePath), cx))
            .child(setting_row(
                t!("set.core_folder"),
                None,
                Button::new("open-core")
                    .small()
                    .icon(extra_icon(Lucide::FolderOpen))
                    .label(t!("btn.open"))
                    .on_click(move |_, _, _| crate::platform::open_path(&core_dir)),
                cx,
            ))
    }
}

fn gpu_label(g: GpuPreference) -> &'static str {
    match g {
        GpuPreference::Auto => t!("gpu.auto"),
        GpuPreference::PowerSaving => t!("gpu.power"),
        GpuPreference::HighPerformance => t!("gpu.perf"),
        GpuPreference::Software => t!("gpu.software"),
    }
}

impl SettingsPage {
    /// Interface size, hardware acceleration and window material.
    fn updates_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let s = engine.settings();
        let info = engine.update_info();
        let t = cx.theme();
        let (muted, danger) = (t.muted_foreground, t.danger);
        let status = if let Some(v) = &info.installed {
            t!("upd.installed", v)
        } else if let Some(r) = &info.latest {
            if info.installing {
                t!("upd.installing", r.version)
            } else if boxpilot_core::update::package_managed() {
                format!("{} {}", t!("upd.available", r.version), t!("upd.package_manager"))
            } else if r.asset.is_none() {
                format!("{} {}", t!("upd.available", r.version), t!("upd.no_asset"))
            } else {
                t!("upd.available", r.version)
            }
        } else if info.checking {
            t!("upd.checking").to_string()
        } else if s.last_update_check > 0 {
            format!("{} {}", t!("upd.latest"), t!("upd.checked", fmt_ago(s.last_update_check)))
        } else {
            t!("upd.never").to_string()
        };
        let mut actions = h_flex().gap_2().flex_wrap().justify_end();
        if info.installed.is_some() {
            actions = actions.child(
                Button::new("upd-restart")
                    .small()
                    .label(t!("btn.restart_apply"))
                    .on_click(|_, _, cx| super::restart_app(cx)),
            );
        } else if let Some(r) = info.latest.clone() {
            let page = r.page.clone();
            actions = actions.child(
                Button::new("upd-notes")
                    .small()
                    .label(t!("btn.release_notes"))
                    .on_click(move |_, _, cx| cx.open_url(&page)),
            );
            if r.asset.is_some() {
                let e = engine.clone();
                actions = actions.child(
                    Button::new("upd-install")
                        .small()
                        .icon(extra_icon(Lucide::Download))
                        .label(t!("btn.update_now"))
                        .loading(info.installing)
                        .on_click(move |_, _, _| e.install_update()),
                );
            }
        } else {
            let e = engine.clone();
            actions = actions.child(
                Button::new("upd-check")
                    .small()
                    .icon(IconName::RefreshCw)
                    .label(t!("btn.check_now"))
                    .loading(info.checking)
                    .on_click(move |_, _, _| e.check_for_update(true)),
            );
        }
        let e_mode = engine.clone();
        let modes: Vec<(SharedString, UpdateMode)> = vec![
            (t!("upd.mode_auto").into(), UpdateMode::Auto),
            (t!("upd.mode_notify").into(), UpdateMode::Notify),
            (t!("upd.mode_off").into(), UpdateMode::Off),
        ];
        card(cx)
            .gap_1()
            .child(section_title(t!("set.updates"), cx))
            .child(setting_row(&format!("BoxPilot {}", boxpilot_core::update::CURRENT), Some(&status), actions, cx))
            .when_some(info.error.clone().filter(|_| !info.checking && !info.installing), |d, err| {
                d.child(div().text_xs().text_color(danger).line_clamp(2).child(err))
            })
            .child(setting_row(
                t!("set.update_mode"),
                Some(t!("set.update_mode_desc")),
                choice("update-mode", s.update_mode, modes, move |v, _, _| e_mode.update_settings(|s| s.update_mode = v)),
                cx,
            ))
            .child(div().text_xs().text_color(muted).child(format!("github.com/{}", boxpilot_core::update::REPO)))
    }

    fn graphics_section(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let s = engine.settings();
        let muted = cx.theme().muted_foreground;
        let (e_scale, e_gpu, e_mica) = (engine.clone(), engine.clone(), engine.clone());
        let system_pct = (super::layout::system_scale() * 100.0).round() as u32;
        let scale_options: Vec<(SharedString, u32)> = super::layout::SCALES
            .iter()
            .map(|v| {
                let label = if *v == 0 { t!("scale.system", system_pct) } else { format!("{v}%") };
                (SharedString::from(label), *v)
            })
            .collect();
        let gpu_options: Vec<(SharedString, GpuPreference)> =
            crate::gpu::choices().iter().map(|g| (SharedString::from(gpu_label(*g)), *g)).collect();
        // What the running renderer actually uses.
        let renderer = window.gpu_specs().map(|g| {
            let kind = if g.is_software_emulated { t!("gpu.cpu") } else { t!("gpu.hardware") };
            t!("gpu.current", format!("{} · {kind}", g.device_name))
        });
        let needs_restart = s.gpu != crate::gpu::startup_preference();
        card(cx)
            .gap_1()
            .child(section_title(t!("set.graphics"), cx))
            .child(setting_row(
                t!("set.scale"),
                Some(t!("set.scale_desc")),
                choice("ui-scale", s.ui_scale, scale_options, move |v, _, _| e_scale.update_settings(|s| s.ui_scale = v)),
                cx,
            ))
            .when(!gpu_options.is_empty(), |d| {
                d.child(setting_row(
                    t!("set.gpu"),
                    Some(t!("set.gpu_desc")),
                    choice("gpu", s.gpu, gpu_options, move |v, _, _| e_gpu.update_settings(|s| s.gpu = v)),
                    cx,
                ))
            })
            .when(crate::gpu::choices().is_empty(), |d| d.child(setting_row(t!("set.gpu"), Some(t!("gpu.mac")), div(), cx)))
            .when_some(renderer, |d, r| d.child(div().text_xs().text_color(muted).pb_1().child(r)))
            .when_some(crate::gpu::missing_display_driver(), |d, (maker, packages)| {
                d.child(div().text_xs().text_color(cx.theme().warning).pb_1().child(t!("gpu.no_display_driver", maker, packages)))
            })
            .when(needs_restart, |d| {
                d.child(
                    h_flex().justify_end().child(
                        Button::new("restart-gpu")
                            .small()
                            .label(t!("btn.restart_apply"))
                            .on_click(|_, _, cx| super::restart_app(cx)),
                    ),
                )
            })
            .when(super::platform_mica_supported(), |d| {
                let (e_kind, e_glass) = (e_mica.clone(), e_mica.clone());
                let kinds: Vec<(SharedString, Backdrop)> = Backdrop::ALL
                    .iter()
                    .map(|b| {
                        let label = match b {
                            Backdrop::Mica => t!("backdrop.mica"),
                            Backdrop::MicaAlt => t!("backdrop.mica_alt"),
                            Backdrop::Acrylic => t!("backdrop.acrylic"),
                        };
                        (SharedString::from(label), *b)
                    })
                    .collect();
                let levels: Vec<(SharedString, u32)> =
                    [0u32, 20, 40, 60, 80].iter().map(|v| (SharedString::from(format!("{v}%")), *v)).collect();
                // Only Windows has several materials; elsewhere it is the system blur.
                let materials = crate::platform::translucency() == crate::platform::Translucency::WindowsMaterials;
                d.child(switch_row("mica", t!("set.mica"), t!("set.mica_desc"), s.mica, &e_mica, |s, v| s.mica = v, cx))
                    .when(s.mica && materials, |d| {
                        d.child(setting_row(
                            t!("set.backdrop"),
                            Some(t!("set.backdrop_desc")),
                            choice("backdrop", s.backdrop, kinds, move |v, _, _| e_kind.update_settings(|s| s.backdrop = v)),
                            cx,
                        ))
                    })
                    .when(s.mica, |d| {
                        d.child(setting_row(
                            t!("set.transparency"),
                            Some(t!("set.transparency_desc")),
                            choice("transparency", s.transparency, levels, move |v, _, _| {
                                e_glass.update_settings(|s| s.transparency = v)
                            }),
                            cx,
                        ))
                    })
            })
    }
}

/// Asks what to reset (everything is ticked), then wipes it and restarts.
fn open_reset_dialog(window: &mut Window, cx: &mut App) {
    use boxpilot_core::ResetScope;
    use gpui_kit::component::checkbox::Checkbox;
    use gpui_kit::component::dialog::DialogFooter;
    use std::cell::Cell;
    use std::rc::Rc;
    let scope = Rc::new(Cell::new(ResetScope::ALL));
    window.open_dialog(cx, move |dialog, _, cx| {
        let muted = cx.theme().muted_foreground;
        let current = scope.get();
        let item = |id: &'static str, label: &'static str, desc: &'static str, checked: bool, set: fn(&mut ResetScope, bool)| {
            let scope = scope.clone();
            v_flex()
                .gap_0p5()
                .child(Checkbox::new(id).label(label).checked(checked).on_click(move |v: &bool, window, _| {
                    let mut s = scope.get();
                    set(&mut s, *v);
                    scope.set(s);
                    window.refresh();
                }))
                .child(div().pl(dp(24.)).text_xs().text_color(muted).child(desc))
        };
        let confirm = scope.clone();
        dialog
            .title(t!("reset.title"))
            .w(dp(480.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(muted).child(t!("reset.body")))
                    .child(item("reset-settings", t!("reset.settings"), t!("reset.settings_desc"), current.settings, |s, v| s.settings = v))
                    .child(item("reset-subs", t!("reset.subs"), t!("reset.subs_desc"), current.subscriptions, |s, v| s.subscriptions = v))
                    .child(item("reset-rules", t!("reset.rules"), t!("reset.rules_desc"), current.rule_sets, |s, v| s.rule_sets = v))
                    .child(item("reset-caches", t!("reset.caches"), t!("reset.caches_desc"), current.caches, |s, v| s.caches = v))
                    .child(div().text_xs().text_color(muted).child(t!("reset.keeps_core"))),
            )
            .footer(
                DialogFooter::new().child(super::cancel_button()).child(
                    Button::new("confirm-reset")
                        .danger()
                        .label(t!("btn.reset_restart"))
                        .disabled(current.is_empty())
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            super::reset_app(confirm.get(), cx);
                        }),
                ),
            )
    });
}

/// Theme colour picker: one swatch per accent, in the current mode's shade.
fn accent_swatches(current: Accent, engine: &std::sync::Arc<boxpilot_core::Engine>, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let dark = t.is_dark();
    let (ring, edge) = (t.foreground, t.border);
    h_flex().gap_1().flex_wrap().children(Accent::ALL.iter().map(|a| {
        let a = *a;
        let e = engine.clone();
        let label = accent_label(a);
        let fill = super::accent::color(a, dark);
        div()
            .id(SharedString::from(format!("accent-{a:?}")))
            .size(dp(28.))
            .p(dp(3.))
            .rounded_full()
            .border_2()
            .border_color(if a == current { ring } else { gpui_kit::transparent_black() })
            .cursor_pointer()
            .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(label).build(window, cx))
            .on_click(move |_, _, _| e.update_settings(|s| s.accent = a))
            .child(
                div()
                    .size_full()
                    .rounded_full()
                    .border_1()
                    .border_color(edge)
                    .bg(fill)
                    .flex()
                    .items_center()
                    .justify_center()
                    // System is marked so it reads as "follows the OS", not one more colour.
                    .when(a == Accent::System, |d| {
                        d.text_color(super::accent::text_on(fill)).child(gpui_kit::component::Icon::new(IconName::Settings).xsmall())
                    }),
            )
    }))
}

fn accent_label(a: Accent) -> &'static str {
    match a {
        Accent::System => t!("accent.system"),
        Accent::Blue => t!("accent.blue"),
        Accent::Teal => t!("accent.teal"),
        Accent::Green => t!("accent.green"),
        Accent::Violet => t!("accent.violet"),
        Accent::Pink => t!("accent.pink"),
        Accent::Orange => t!("accent.orange"),
        Accent::Graphite => t!("accent.graphite"),
    }
}

fn switch_row(
    id: &'static str,
    title: &str,
    desc: &str,
    checked: bool,
    engine: &std::sync::Arc<boxpilot_core::Engine>,
    set: fn(&mut Settings, bool),
    cx: &App,
) -> impl IntoElement {
    let engine = engine.clone();
    setting_row(
        title,
        Some(desc),
        Toggle::new(id).checked(checked).on_change(move |v, _, _| {
            let v = *v;
            engine.update_settings(move |s| set(s, v))
        }),
        cx,
    )
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let m = super::layout::metrics(window);
        self.input_w = if m.narrow { 200.0 } else if m.compact_nav { 250.0 } else { 300.0 };
        let engine = self.state.read(cx).engine.clone();
        let graphics = self.graphics_section(window, cx).into_any_element();
        let s = engine.settings();
        let e = &engine;
        let (et, es, el, elang) = (engine.clone(), engine.clone(), engine.clone(), engine.clone());
        let export_engine = engine.clone();
        let data_dir = engine.paths().root.clone();
        let muted = cx.theme().muted_foreground;
        let theme_options: Vec<(SharedString, ThemePref)> = vec![
            (t!("theme.system").into(), ThemePref::System),
            (t!("theme.light").into(), ThemePref::Light),
            (t!("theme.dark").into(), ThemePref::Dark),
        ];
        let language_options: Vec<(SharedString, Language)> = Language::ALL
            .iter()
            .map(|l| {
                let label: SharedString = if *l == Language::System { t!("lang.system").into() } else { l.native_name().into() };
                (label, *l)
            })
            .collect();
        let levels: Vec<(SharedString, &'static str)> =
            ["trace", "debug", "info", "warn", "error"].into_iter().map(|l| (SharedString::from(l), l)).collect();
        let current_level: &'static str =
            ["trace", "debug", "info", "warn", "error"].into_iter().find(|l| *l == s.log_level).unwrap_or("info");

        let sections = vec![
            card(cx)
                .gap_1()
                .child(section_title(t!("set.general"), cx))
                .child(setting_row(
                    t!("set.language"),
                    None,
                    choice("language", s.language, language_options, move |v, _, _| elang.update_settings(|s| s.language = v)),
                    cx,
                ))
                .child(setting_row(
                    t!("set.theme"),
                    None,
                    choice("theme", s.theme, theme_options, move |v, _, _| et.update_settings(|s| s.theme = v)),
                    cx,
                ))
                .child(setting_row(t!("set.accent"), Some(t!("set.accent_desc")), accent_swatches(s.accent, e, cx), cx))
                .child(switch_row("login", t!("set.launch_login"), t!("set.launch_login_desc"), s.launch_at_login, e, |s, v| s.launch_at_login = v, cx))
                // macOS / Linux grant TUN to the core binary from the TUN switch instead.
                .when(cfg!(windows), |d| {
                    d.child(setting_row(
                        t!("set.run_admin"),
                        Some(t!("set.run_admin_desc")),
                        Toggle::new("run-admin").checked(s.run_as_admin).on_change(|v, _, cx| super::toggle_run_as_admin(*v, cx)),
                        cx,
                    ))
                })
                .child(switch_row("minimized", t!("set.start_min"), t!("set.start_min_desc"), s.start_minimized, e, |s, v| s.start_minimized = v, cx))
                .child(switch_row("autoconnect", t!("set.autoconnect"), t!("set.autoconnect_desc"), s.auto_start_core, e, |s, v| s.auto_start_core = v, cx))
                .child(switch_row("tray", t!("set.close_tray"), t!("set.close_tray_desc"), s.close_to_tray, e, |s, v| s.close_to_tray = v, cx))
                .when(s.close_to_tray, |d| {
                    d.child(switch_row("lightweight", t!("set.lightweight"), t!("set.lightweight_desc"), s.lightweight, e, |s, v| s.lightweight = v, cx))
                })
                .into_any_element(),
            graphics,
            self.updates_section(cx).into_any_element(),
            self.core_section(cx).into_any_element(),
            card(cx)
                .gap_1()
                .child(section_title(t!("set.proxy"), cx))
                .child(setting_row(t!("set.mixed_port"), Some(t!("set.mixed_port_desc")), self.input(F::MixedPort), cx))
                .child(switch_row("lan", t!("set.lan"), t!("set.lan_desc"), s.allow_lan, e, |s, v| s.allow_lan = v, cx))
                .child(switch_row("udp", t!("set.proxy_udp"), t!("set.proxy_udp_desc"), s.proxy_udp, e, |s, v| s.proxy_udp = v, cx))
                .child(switch_row("quic", t!("set.block_quic"), t!("set.block_quic_desc"), s.block_quic, e, |s, v| s.block_quic = v, cx))
                .child(switch_row("sysproxy2", t!("set.sysproxy"), t!("set.sysproxy_desc"), s.system_proxy, e, |s, v| s.system_proxy = v, cx))
                .child(setting_row(t!("set.bypass"), Some(t!("set.bypass_desc")), self.input(F::Bypass), cx))
                .into_any_element(),
            card(cx)
                .gap_1()
                .child(section_title(t!("set.tun"), cx))
                .child(setting_row(
                    t!("set.tun_enable"),
                    Some(if engine.tun_permitted() { t!("set.tun_enable_desc") } else if cfg!(windows) { t!("set.tun_enable_desc_admin") } else { t!("set.tun_enable_desc_grant") }),
                    Toggle::new("tun2").checked(s.tun_enabled).on_change(|v, _, cx| super::toggle_tun(*v, cx)),
                    cx,
                ))
                .child(setting_row(
                    t!("set.stack"),
                    Some(t!("set.stack_desc")),
                    choice(
                        "stack",
                        s.tun_stack,
                        TunStack::ALL.iter().map(|v| (SharedString::from(v.as_str()), *v)).collect(),
                        move |v, _, _| es.update_settings(|s| s.tun_stack = v),
                    ),
                    cx,
                ))
                .child(switch_row("strict", t!("set.strict"), t!("set.strict_desc"), s.tun_strict_route, e, |s, v| s.tun_strict_route = v, cx))
                .child(setting_row(t!("set.mtu"), None, self.input(F::TunMtu), cx))
                .into_any_element(),
            card(cx)
                .gap_1()
                .child(section_title(t!("set.dns"), cx))
                .child(setting_row(t!("set.dns_remote"), Some(t!("set.dns_remote_desc")), self.input(F::DnsRemote), cx))
                .child(setting_row(t!("set.dns_direct"), Some(t!("set.dns_direct_desc")), self.input(F::DnsDirect), cx))
                .child(switch_row("fakeip", t!("set.fakeip"), t!("set.fakeip_desc"), s.fake_ip, e, |s, v| s.fake_ip = v, cx))
                .child(switch_row("ipv6", t!("set.ipv6"), t!("set.ipv6_desc"), s.ipv6, e, |s, v| s.ipv6 = v, cx))
                .into_any_element(),
            card(cx)
                .gap_1()
                .child(section_title(t!("set.subs"), cx))
                .child(setting_row(t!("set.ua"), Some(t!("set.ua_desc")), self.input(F::UserAgent), cx))
                .child(setting_row(t!("set.default_interval"), Some(t!("set.default_interval_desc")), self.input(F::SubInterval), cx))
                .child(setting_row(t!("set.timeout"), None, self.input(F::SubTimeout), cx))
                .child(switch_row("reload", t!("set.reload"), t!("set.reload_desc"), s.sub_reload_after_update, e, |s, v| s.sub_reload_after_update = v, cx))
                .into_any_element(),
            card(cx)
                .gap_1()
                .child(section_title(t!("set.advanced"), cx))
                .child(setting_row(
                    t!("set.log_level"),
                    None,
                    choice("loglevel", current_level, levels, move |v, _, _| el.update_settings(|s| s.log_level = v.to_string())),
                    cx,
                ))
                .child(setting_row(t!("set.api_port"), Some(t!("set.api_port_desc")), self.input(F::ApiPort), cx))
                .child(setting_row(t!("set.test_url"), None, self.input(F::TestUrl), cx))
                .child(switch_row("auto-test", t!("set.auto_test"), t!("set.auto_test_desc"), s.auto_test_nodes, e, |s, v| s.auto_test_nodes = v, cx))
                .child(setting_row(t!("set.test_interval"), None, self.input(F::TestInterval), cx))
                .child(setting_row(
                    t!("set.export"),
                    Some(t!("set.export_desc")),
                    Button::new("export")
                        .small()
                        .icon(IconName::Copy)
                        .label(t!("btn.copy_json"))
                        .on_click(move |_, window, cx| {
                            let json = serde_json::to_string_pretty(&export_engine.preview_config()).unwrap_or_default();
                            cx.write_to_clipboard(ClipboardItem::new_string(json));
                            window.push_notification(Notification::success(t!("set.config_copied")), cx);
                        }),
                    cx,
                ))
                .child(setting_row(
                    t!("set.reset"),
                    Some(t!("set.reset_desc")),
                    Button::new("reset-app")
                        .small()
                        .icon(extra_icon(Lucide::Trash))
                        .label(t!("btn.reset"))
                        .on_click(|_, window, cx| open_reset_dialog(window, cx)),
                    cx,
                ))
                .child(setting_row(
                    t!("set.data_folder"),
                    Some(&data_dir.display().to_string()),
                    Button::new("open-data")
                        .small()
                        .icon(extra_icon(Lucide::FolderOpen))
                        .label(t!("btn.open"))
                        .on_click({
                            let d = data_dir.clone();
                            move |_, _, _| crate::platform::open_path(&d)
                        }),
                    cx,
                ))
                .into_any_element(),
        ];

        let sections: Vec<gpui_kit::AnyElement> = sections
            .into_iter()
            .enumerate()
            .map(|(i, el)| anim::enter(div().child(el), ("settings-section", i), anim::stagger(i) * 0.6).into_any_element())
            .collect();
        // Wide windows get two columns instead of a narrow strip with empty space beside it.
        let body = if m.content >= 1180.0 {
            let (mut left, mut right) = (Vec::new(), Vec::new());
            for (i, el) in sections.into_iter().enumerate() {
                if i % 2 == 0 { left.push(el) } else { right.push(el) }
            }
            h_flex()
                .items_start()
                .gap_4()
                .child(v_flex().flex_1().min_w_0().gap_4().children(left))
                .child(v_flex().flex_1().min_w_0().gap_4().children(right))
                .into_any_element()
        } else {
            v_flex().gap_4().children(sections).into_any_element()
        };

        page(t!("nav.settings"), Some(t!("settings.subtitle").into()), vec![], cx).child(
            v_flex()
                .gap_4()
                .child(body)
                .child(
                    div()
                        .text_xs()
                        .text_color(muted)
                        .pb_4()
                        .child(t!("set.about", env!("CARGO_PKG_VERSION"))),
                ),
        )
    }
}
