use boxpilot_core::engine::TUN_NEEDS_ADMIN;
use boxpilot_core::model::ProxyMode;
use boxpilot_core::{CoreDownloadState, CoreStatus};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, Context, ElementId, Entity, FontWeight, Hsla, SharedString, SpringAnimation, Subscription, Window, div,
    relative,
};

use super::anim;
use super::widgets::*;
use crate::state::{AppState, Page};
use crate::t;
use super::layout::dp;

pub struct DashboardPage {
    state: Entity<AppState>,
    _sub: Subscription,
}

impl DashboardPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let _sub = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, _sub }
    }

    fn current_node(&self, cx: &App) -> Option<String> {
        let groups = self.state.read(cx).engine.groups();
        let proxy = groups.iter().find(|g| g.name == "Proxy")?;
        let mut label = outbound_label(&proxy.now);
        if let Some(inner) = groups.iter().find(|g| g.name == proxy.now) {
            if !inner.now.is_empty() {
                label = format!("{} → {}", outbound_label(&inner.name), outbound_label(&inner.now));
            }
        }
        Some(label)
    }

    fn hero(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let status = engine.status();
        let installed = engine.core_installed();
        let nodes = engine.total_nodes();
        let settings = engine.settings();
        let t = cx.theme();
        let (color, title) = match &status {
            CoreStatus::Running => (t.success, t!("status.connected")),
            CoreStatus::Starting => (t.warning, t!("status.connecting")),
            CoreStatus::Stopping => (t.warning, t!("status.disconnecting")),
            CoreStatus::Failed(_) => (t.danger, t!("status.failed")),
            CoreStatus::Stopped => (t.muted_foreground, t!("status.disconnected")),
        };
        let mode = mode_label(settings.mode);
        let detail = match &status {
            CoreStatus::Running => {
                let node = self.current_node(cx).unwrap_or_default();
                let via = if settings.tun_enabled {
                    "TUN".to_string()
                } else if settings.system_proxy {
                    t!("dash.via_sysproxy", settings.mixed_port)
                } else {
                    format!("127.0.0.1:{}", settings.mixed_port)
                };
                t!("dash.detail_running", mode, node, via)
            }
            _ if !installed => t!("dash.core_missing").to_string(),
            _ if nodes == 0 => t!("dash.no_nodes").to_string(),
            CoreStatus::Failed(_) => t!("dash.see_details").to_string(),
            _ => t!("dash.ready", nodes, mode),
        };

        let action = if !installed {
            let downloading = !matches!(
                engine.download_state(),
                CoreDownloadState::Idle
                    | CoreDownloadState::Failed(_)
                    | CoreDownloadState::UpToDate(_)
                    | CoreDownloadState::Available(_)
            );
            let engine = engine.clone();
            Button::new("hero-download")
                .primary()
                .large()
                .icon(extra_icon(Lucide::Download))
                .label(if downloading { t!("btn.downloading") } else { t!("btn.download_core") })
                .loading(downloading)
                .on_click(move |_, _, _| engine.install_core())
                .into_any_element()
        } else if nodes == 0 {
            let state = self.state.clone();
            Button::new("hero-add")
                .primary()
                .large()
                .icon(IconName::Plus)
                .label(t!("btn.add_subscription"))
                .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_page(Page::Subscriptions, cx)))
                .into_any_element()
        } else {
            let running = status.is_running();
            let engine_restart = engine.clone();
            let engine = engine.clone();
            let mut b = Button::new("hero-toggle")
                .large()
                .icon(extra_icon(Lucide::Power))
                .label(if running { t!("btn.disconnect") } else { t!("btn.connect") })
                .loading(status.is_busy())
                .disabled(status.is_busy())
                .on_click(move |_, _, _| {
                    if engine.status().is_running() {
                        engine.stop();
                    } else {
                        engine.start();
                    }
                });
            b = if running { b.outline() } else { b.primary() };
            if running {
                // Restart without a disconnect/connect round trip (applies a stuck core or new files).
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("hero-restart")
                            .large()
                            .outline()
                            .icon(IconName::RefreshCw)
                            .tooltip(t!("tip.restart_core"))
                            .disabled(status.is_busy())
                            .on_click(move |_, _, _| engine_restart.restart()),
                    )
                    .child(b)
                    .into_any_element()
            } else {
                b.into_any_element()
            }
        };

        // The ring's colour eases between states on a critically damped spring.
        let ring = div()
            .size(dp(52.))
            .flex_none()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .with_spring(
                "hero-ring",
                SpringAnimation::new(anim::SMOOTH).to(1.0f32).from(0.0),
                move |d, p| {
                    let p = p.clamp(0.0, 1.0);
                    d.bg(color.opacity(0.15 * p)).text_color(color.opacity(0.4 + 0.6 * p))
                },
            )
            .child(extra_icon(Lucide::ShieldCheck).size(dp(26.)));
        // Re-mount the ring when the status colour changes so the spring replays.
        let ring = div().id(ElementId::from(SharedString::from(format!("ring-{:?}", status_key(&status))))).child(ring);

        let mut c = card(cx).child(
            h_flex()
                .gap_4()
                .items_center()
                .flex_wrap()
                .child(ring)
                .child(
                    v_flex()
                        .flex_1()
                        .min_w(dp(200.))
                        .gap_1()
                        .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_sm().text_color(t.muted_foreground).truncate().child(detail)),
                )
                .child(div().flex_none().child(action)),
        );
        if let CoreStatus::Failed(msg) = &status {
            let needs_admin = msg.contains(TUN_NEEDS_ADMIN);
            c = c.child(anim::enter(
                h_flex()
                    .gap_3()
                    .p_3()
                    .rounded(t.radius)
                    .bg(t.danger.opacity(0.08))
                    .border_1()
                    .border_color(t.danger.opacity(0.3))
                    .items_start()
                    .child(div().flex_none().text_color(t.danger).child(IconName::CircleX))
                    .child(div().flex_1().min_w_0().text_sm().text_color(t.foreground).child(core_error_text(msg)))
                    .when(needs_admin, |d| {
                        d.child(
                            Button::new("admin")
                                .small()
                                .primary()
                                .label(if cfg!(windows) { t!("btn.restart_admin_short") } else { t!("btn.grant_tun") })
                                .on_click(|_, _, cx| super::toggle_tun(true, cx)),
                        )
                    }),
                "hero-error",
                0.0,
            ));
        }
        c
    }

    /// A statistic whose number rolls toward its new value on a spring.
    fn stat(
        id: &'static str,
        icon: impl Into<gpui_kit::component::Icon>,
        label: &str,
        value: f32,
        format: fn(f32) -> String,
        sub: String,
        color: Hsla,
        cx: &App,
    ) -> impl IntoElement {
        let t = cx.theme();
        card(cx)
            .flex_1()
            .min_w(dp(180.))
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .child(div().flex_none().text_color(color).child(icon.into().size(dp(15.))))
                    .child(div().min_w_0().truncate().child(label.to_string())),
            )
            .child(
                div()
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .truncate()
                    .with_spring(id, SpringAnimation::new(anim::SMOOTH).to(value), move |d, v| {
                        d.child(format(v.max(0.0)))
                    }),
            )
            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(sub))
    }

    fn chart(&self, cx: &App) -> impl IntoElement {
        let st = self.state.read(cx);
        let t = cx.theme();
        let max = st.traffic.iter().map(|s| s.up + s.down).max().unwrap_or(0).max(64 * 1024);
        let (down_c, up_c) = (t.chart_1, t.chart_2);
        card(cx)
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .flex_wrap()
                    .gap_2()
                    .child(section_title(t!("chart.traffic"), cx))
                    .child(
                        h_flex()
                            .gap_4()
                            .text_xs()
                            .text_color(t.muted_foreground)
                            .child(h_flex().gap_1p5().items_center().child(dot(down_c)).child(t!("stat.download")))
                            .child(h_flex().gap_1p5().items_center().child(dot(up_c)).child(t!("stat.upload")))
                            .child(t!("chart.peak", fmt_speed(max))),
                    ),
            )
            .child(
                h_flex()
                    .h(dp(120.))
                    .w_full()
                    .items_end()
                    .gap(dp(2.))
                    .border_b_1()
                    .border_color(t.border)
                    .children(st.traffic.iter().enumerate().map(|(i, s)| {
                        // Each bar eases to its new height as samples shift left.
                        let down = s.down as f32 / max as f32;
                        let up = s.up as f32 / max as f32;
                        v_flex()
                            .flex_1()
                            .h_full()
                            .justify_end()
                            .child(div().w_full().bg(up_c).rounded_t(dp(2.)).with_spring(
                                ("bar-up", i),
                                SpringAnimation::new(anim::SMOOTH).to(up),
                                |d, v| d.h(relative(v.clamp(0.0, 1.0))),
                            ))
                            .child(div().w_full().bg(down_c).with_spring(
                                ("bar-down", i),
                                SpringAnimation::new(anim::SMOOTH).to(down),
                                |d, v| d.h(relative(v.clamp(0.0, 1.0))),
                            ))
                    })),
            )
    }

    fn controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let s = engine.settings();
        let tun_ok = engine.tun_permitted();
        let (e1, e2, e3) = (engine.clone(), engine.clone(), engine.clone());
        card(cx)
            .flex_1()
            .min_w(dp(340.))
            .gap_1()
            .child(section_title(t!("quick.title"), cx))
            .child(setting_row(
                t!("quick.mode"),
                Some(t!("quick.mode_desc")),
                segmented(
                    "mode",
                    &[
                        (t!("mode.rule"), ProxyMode::Rule),
                        (t!("mode.global"), ProxyMode::Global),
                        (t!("mode.direct"), ProxyMode::Direct),
                    ],
                    s.mode,
                    move |m, _, _| e1.set_mode(m),
                    cx,
                ),
                cx,
            ))
            .child(divider(cx))
            .child(setting_row(
                t!("quick.sysproxy"),
                Some(t!("quick.sysproxy_desc")),
                Toggle::new("sysproxy").checked(s.system_proxy).on_change(move |v, _, _| {
                    let v = *v;
                    e2.update_settings(|s| s.system_proxy = v)
                }),
                cx,
            ))
            .child(divider(cx))
            .child(setting_row(
                t!("quick.tun"),
                Some(if tun_ok { t!("quick.tun_desc") } else if cfg!(windows) { t!("quick.tun_desc_admin") } else { t!("quick.tun_desc_grant") }),
                Toggle::new("tun").checked(s.tun_enabled).on_change(|v, _, cx| super::toggle_tun(*v, cx)),
                cx,
            ))
            .child(divider(cx))
            .child(setting_row(
                t!("quick.lan"),
                Some(t!("quick.lan_desc")),
                Toggle::new("lan").checked(s.allow_lan).on_change(move |v, _, _| {
                    let v = *v;
                    e3.update_settings(|s| s.allow_lan = v)
                }),
                cx,
            ))
    }

    /// The node traffic actually leaves through, followed from Proxy through
    /// any groups, with the same quick tests as the Proxies page.
    fn now_using(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let groups = engine.groups();
        let mut path: Vec<String> = Vec::new();
        let mut now = groups.iter().find(|g| g.name == "Proxy").map(|g| g.now.clone()).unwrap_or_default();
        while let Some(g) = groups.iter().find(|g| g.name == now && !g.now.is_empty()) {
            if path.len() > 4 {
                break;
            }
            path.push(outbound_label(&g.name));
            now = g.now.clone();
        }
        let protocol = groups
            .iter()
            .flat_map(|g| g.members.iter())
            .find(|m| m.name == now)
            .map(|m| m.protocol.to_uppercase())
            .unwrap_or_default();
        let state = self.state.clone();
        let header = h_flex()
            .justify_between()
            .items_center()
            .gap_2()
            .child(section_title(t!("dash.now_using"), cx))
            .child(
                Button::new("dash-change-node")
                    .small()
                    .ghost()
                    .icon(extra_icon(Lucide::Earth))
                    .label(t!("btn.change_node"))
                    .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_page(Page::Proxies, cx))),
            );
        let card = card(cx).gap_2().child(header);
        if !engine.status().is_running() || now.is_empty() {
            return card.child(div().text_sm().text_color(t.muted_foreground).child(t!("status.disconnected")));
        }
        let latency = match engine.delay(&now) {
            _ if engine.is_testing(&now) => Metric::Testing,
            None => Metric::Untested,
            Some(None) => Metric::Failed,
            Some(Some(d)) => Metric::Ms(d),
        };
        let real = match engine.real_delay(&now) {
            _ if engine.is_real_testing(&now) || engine.is_speed_testing(&now) => Metric::Testing,
            None => Metric::Untested,
            Some(None) => Metric::Failed,
            Some(Some(d)) => Metric::Ms(d),
        };
        let speed = match engine.speed(&now) {
            _ if engine.is_speed_testing(&now) => Metric::Testing,
            None => Metric::Untested,
            Some(r) => r.bytes_per_sec.map_or(Metric::Failed, Metric::Speed),
        };
        let (e1, e2, e3) = (engine.clone(), engine.clone(), engine.clone());
        let (n1, n2, n3) = (now.clone(), now.clone(), now.clone());
        let route = if protocol.is_empty() { path.join(" → ") } else { format!("{} · {protocol}", path.join(" → ")) };
        card.child(
            h_flex()
                .gap_2()
                .items_center()
                .min_w_0()
                .when_some(engine.country(&now), |d, c| d.child(country_badge(&c, "dash-cc".into(), cx)))
                .child(div().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD).child(outbound_label(&now))),
        )
        .when(!route.is_empty(), |d| d.child(div().text_xs().text_color(t.muted_foreground).truncate().child(route)))
        .when(now != "direct", |d| {
            d.child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(metric_chip("dash-lat".into(), Lucide::Zap, latency, t!("tip.chip_latency"), cx, move |_, _| {
                        e1.test_delays(vec![n1.clone()])
                    }))
                    .child(metric_chip("dash-real".into(), Lucide::Activity, real, t!("tip.chip_real"), cx, move |_, _| {
                        e2.test_real_latency(vec![n2.clone()])
                    }))
                    .child(metric_chip("dash-spd".into(), Lucide::Gauge, speed, t!("tip.speed_one"), cx, move |_, _| {
                        e3.speed_test(vec![n3.clone()])
                    })),
            )
        })
    }

    fn subscriptions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let subs = engine.subscriptions();
        let t = cx.theme();
        let updating = engine.any_updating();
        let e = engine.clone();
        let state = self.state.clone();
        card(cx)
            .flex_1()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(section_title(t!("nav.subscriptions"), cx))
                    .child(
                        Button::new("dash-update-all")
                            .small()
                            .ghost()
                            .icon(IconName::RefreshCw)
                            .label(t!("btn.update_all"))
                            .loading(updating)
                            .disabled(subs.is_empty())
                            .on_click(move |_, _, _| e.update_all()),
                    ),
            )
            .when(subs.is_empty(), |d| {
                d.child(div().text_sm().text_color(t.muted_foreground).child(t!("subs.none_yet")))
            })
            .children(subs.iter().take(5).enumerate().map(|(i, sub)| {
                let usage = sub.info.as_ref().filter(|x| x.total > 0).map(|x| {
                    (
                        x.used() as f32 / x.total as f32 * 100.0,
                        format!("{} / {}", fmt_bytes(x.used()), fmt_bytes(x.total)),
                    )
                });
                let updated = sub.last_update.map(fmt_ago).unwrap_or_else(|| t!("subs.never_updated").to_string());
                let error = sub.last_error.is_some();
                let dot_c = if !sub.enabled {
                    t.muted_foreground
                } else if error {
                    t.danger
                } else {
                    t.success
                };
                v_flex()
                    .gap_1()
                    .py_1()
                    .child(
                        h_flex()
                            .justify_between()
                            .gap_2()
                            .child(
                                h_flex()
                                    .flex_1()
                                    .gap_2()
                                    .items_center()
                                    .min_w_0()
                                    .child(dot(dot_c))
                                    .child(div().text_sm().min_w_0().truncate().child(sub.name.clone())),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(t.muted_foreground)
                                    .flex_shrink_0()
                                    .max_w(relative(0.6))
                                    .truncate()
                                    .child(t!("subs.nodes_updated", sub.node_count, updated)),
                            ),
                    )
                    .when_some(usage, |d, (pct, text)| {
                        let bar_c = if pct > 90.0 { t.danger } else { t.primary };
                        d.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().flex_1().child(progress_bar(("dash-usage", i), pct, bar_c, cx)))
                                .child(div().flex_none().text_xs().text_color(t.muted_foreground).child(text)),
                        )
                    })
            }))
            .when(subs.len() > 5, |d| {
                d.child(
                    Button::new("dash-more")
                        .small()
                        .ghost()
                        .label(t!("btn.view_all", subs.len()))
                        .on_click(move |_, _, cx| state.update(cx, |s, cx| s.set_page(Page::Subscriptions, cx))),
                )
            })
    }
}

fn status_key(s: &CoreStatus) -> u8 {
    match s {
        CoreStatus::Stopped => 0,
        CoreStatus::Starting => 1,
        CoreStatus::Running => 2,
        CoreStatus::Stopping => 3,
        CoreStatus::Failed(_) => 4,
    }
}

impl Render for DashboardPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let st = self.state.read(cx);
        let engine = st.engine.clone();
        let last = st.traffic.back().copied().unwrap_or_default();
        let (sup, sdown) = (st.session_up, st.session_down);
        let t = cx.theme();
        let (c_up, c_down, c_total, c_nodes) = (t.chart_2, t.chart_1, t.info, t.success);
        let subs = engine.subscriptions().len();
        let speed = |v: f32| fmt_speed(v as u64);
        let bytes = |v: f32| fmt_bytes(v as u64);
        let count = |v: f32| format!("{}", v.round() as u64);

        page(t!("nav.dashboard"), None, vec![], cx)
            .child(anim::enter(div().child(self.hero(cx)), "dash-hero", 0.0))
            .child(anim::enter(
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .child(Self::stat("stat-up", IconName::ArrowUp, t!("stat.upload"), last.up as f32, speed, t!("stat.current").into(), c_up, cx))
                    .child(Self::stat("stat-down", IconName::ArrowDown, t!("stat.download"), last.down as f32, speed, t!("stat.current").into(), c_down, cx))
                    .child(Self::stat(
                        "stat-session",
                        extra_icon(Lucide::ArrowDownUp),
                        t!("stat.session"),
                        (sup + sdown) as f32,
                        bytes,
                        format!("↑ {}  ↓ {}", fmt_bytes(sup), fmt_bytes(sdown)),
                        c_total,
                        cx,
                    ))
                    .child(Self::stat(
                        "stat-nodes",
                        extra_icon(Lucide::Earth),
                        t!("stat.nodes"),
                        engine.total_nodes() as f32,
                        count,
                        t!("stat.from_subs", subs),
                        c_nodes,
                        cx,
                    )),
                "dash-stats",
                anim::stagger(1),
            ))
            .child(anim::enter(div().child(self.chart(cx)), "dash-chart", anim::stagger(2)))
            .child(anim::enter(
                // Columns share the row height, so neither leaves a hole.
                h_flex()
                    .gap_4()
                    .flex_wrap()
                    .items_stretch()
                    .child(self.controls(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w(dp(340.))
                            .gap_4()
                            .child(self.now_using(cx))
                            .child(self.subscriptions(cx)),
                    ),
                "dash-bottom",
                anim::stagger(3),
            ))
    }
}
