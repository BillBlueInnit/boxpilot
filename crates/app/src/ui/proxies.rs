use boxpilot_core::geo::{Country, Origin};
use boxpilot_core::model::AutoStrategy;
use boxpilot_core::{ProxyGroup, ProxyMember};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, FontWeight, SharedString, SpringAnimation, Subscription, Window, div};

use super::anim;
use super::widgets::*;
use crate::state::AppState;
use crate::t;
use super::layout::dp;

pub struct ProxiesPage {
    state: Entity<AppState>,
    group: Option<String>,
    search: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl ProxiesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("proxies.filter")));
        let _subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self { state, group: None, search, _subs }
    }

    fn group_tabs(&self, groups: &[ProxyGroup], current: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        h_flex().gap_2().flex_wrap().children(groups.iter().enumerate().map(|(i, g)| {
            let selected = g.name == current;
            let name = g.name.clone();
            let (primary, border, muted_bg) = (t.primary, t.border, t.muted);
            div()
                .id(("group", i))
                .px_3()
                .py_1p5()
                .rounded(t.radius)
                .border_1()
                .cursor_pointer()
                .text_sm()
                .max_w(dp(260.))
                .when(selected, |d| d.text_color(t.foreground).font_weight(FontWeight::MEDIUM))
                .when(!selected, |d| d.text_color(t.muted_foreground).hover(|d| d.bg(muted_bg)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.group = Some(name.clone());
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .child(div().min_w_0().truncate().child(outbound_label(&g.name)))
                        .child(div().flex_none().text_xs().text_color(t.muted_foreground).child(g.members.len().to_string())),
                )
                // The highlight eases in and out instead of snapping.
                .with_spring(
                    SharedString::from(format!("tab-{}", g.name)),
                    SpringAnimation::new(anim::SMOOTH).to(selected),
                    move |d, p| {
                        let p = p.0.clamp(0.0, 1.0);
                        d.border_color(p.interpolate_hsla(border, primary)).bg(primary.opacity(0.08 * p))
                    },
                )
        }))
    }

    fn node_card(&self, group: &ProxyGroup, idx: usize, member: &ProxyMember, groups: &[ProxyGroup], cx: &App) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let selected = member.name == group.now;
        let testing = engine.is_testing(&member.name);
        let delay = engine.delay(&member.name);
        let country = (!member.is_group).then(|| engine.country(&member.name)).flatten();
        let measurable = !member.is_group && member.name != "direct";
        let speed_testing = engine.is_speed_testing(&member.name);
        let speed = engine.speed(&member.name);
        let e_speed = engine.clone();
        let speed_name = member.name.clone();
        let subtitle = if member.is_group {
            groups
                .iter()
                .find(|g| g.name == member.name)
                .map(|g| format!("{} · {}", t!("proxies.group"), outbound_label(&g.now)))
                .unwrap_or_else(|| t!("proxies.group").to_string())
        } else if member.name == "direct" {
            "DIRECT".to_string()
        } else {
            member.protocol.to_uppercase()
        };
        let (delay_text, delay_c, delay_key) = match delay {
            _ if testing => ("…".to_string(), t.muted_foreground, "testing".to_string()),
            None => ("—".to_string(), t.muted_foreground, "none".to_string()),
            Some(None) => (t!("latency.timeout").to_string(), t.danger, "timeout".to_string()),
            Some(Some(d)) => (format!("{d} ms"), delay_color(Some(d), cx), d.to_string()),
        };
        let selectable = group.selectable();
        let (g, n) = (group.name.clone(), member.name.clone());
        let e_sel = engine.clone();
        let e_test = engine.clone();
        let test_name = member.name.clone();
        let (primary, border, bg) = (t.primary, t.border, t.background);
        div()
            .id(SharedString::from(format!("node-{}-{idx}", group.name)))
            .w_full()
            .p_3()
            .rounded(t.radius_lg)
            .border_1()
            .when(selectable, |d| {
                d.cursor_pointer().hover(|d| d.shadow_sm()).on_click(move |_, _, _| e_sel.select_proxy(&g, &n))
            })
            .child(
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .when_some(country, |d, c| {
                                d.child(country_badge(&c, SharedString::from(format!("cc-{}-{idx}", group.name)), cx))
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(if selected { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                                    .truncate()
                                    .child(outbound_label(&member.name)),
                            )
                            .when(selected, |d| {
                                d.child(div().flex_none().text_color(t.primary).child(IconName::Check))
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .justify_between()
                            .items_center()
                            .child(div().flex_1().min_w_0().text_xs().text_color(t.muted_foreground).truncate().child(subtitle))
                            .when(measurable, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("speed-{}-{idx}", group.name)))
                                        .flex_none()
                                        .px_1()
                                        .rounded(dp(4.))
                                        .text_xs()
                                        .text_color(t.muted_foreground)
                                        .cursor_pointer()
                                        .hover(|d| d.bg(t.muted).text_color(t.foreground))
                                        .tooltip(|window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(t!("tip.speed_one")).build(window, cx)
                                        })
                                        .on_click(move |_, _, cx| {
                                            cx.stop_propagation();
                                            e_speed.speed_test(vec![speed_name.clone()]);
                                        })
                                        .child(if speed_testing { "…".into_any_element() } else { extra_icon(Lucide::Gauge).into_any_element() }),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("delay-{}-{idx}", group.name)))
                                    .flex_none()
                                    .px_1()
                                    .rounded(dp(4.))
                                    .text_xs()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(delay_c)
                                    .cursor_pointer()
                                    .hover(|d| d.bg(t.muted))
                                    .on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        e_test.test_delays(vec![test_name.clone()]);
                                    })
                                    // A new result remounts the badge, which fades in.
                                    .child(
                                        div()
                                            .with_spring(
                                                SharedString::from(format!("d-{}-{}", member.name, delay_key)),
                                                SpringAnimation::new(anim::SNAPPY).to(1.0f32).from(0.0),
                                                |d, p| d.opacity(p.clamp(0.0, 1.0)),
                                            )
                                            .child(delay_text),
                                    ),
                            ),
                    )
                    // Speed test result: real latency over a fresh connection, then download speed.
                    .when_some(speed.filter(|_| !speed_testing), |d, r| {
                        d.child(match (r.latency_ms, r.bytes_per_sec) {
                            (None, _) => div().text_xs().text_color(t.danger).child(t!("speed.failed")),
                            (Some(ms), bps) => h_flex()
                                .gap_1()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(t!("speed.real"))
                                .child(div().text_color(delay_color(Some(ms), cx)).child(format!("{ms} ms")))
                                .child("·")
                                .child(
                                    div()
                                        .text_color(t.foreground)
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(bps.map_or_else(|| "—".to_string(), |b| format!("↓ {}", fmt_speed(b)))),
                                ),
                        })
                    }),
            )
            .with_spring(
                SharedString::from(format!("sel-{}-{}", group.name, member.name)),
                SpringAnimation::new(anim::SMOOTH).to(selected),
                move |d, p| {
                    let p = p.0.clamp(0.0, 1.0);
                    d.border_color(p.interpolate_hsla(border, primary)).bg(p.interpolate_hsla(bg, primary.opacity(0.07)))
                },
            )
    }
}

/// How the Auto group picks its node, and which region it draws from.
fn auto_bar(engine: &std::sync::Arc<boxpilot_core::Engine>, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let s = engine.settings();
    let (e_mode, e_region) = (engine.clone(), engine.clone());
    let strategies: Vec<(SharedString, AutoStrategy)> =
        AutoStrategy::ALL.iter().map(|a| (SharedString::from(auto_label(*a)), *a)).collect();
    // (label, code); the dropdown carries the index, since codes are strings.
    let mut regions: Vec<(SharedString, String)> = vec![(t!("auto.region_any").into(), String::new())];
    regions.extend(engine.country_counts().into_iter().map(|(code, n)| (SharedString::from(format!("{code} · {n}")), code)));
    if !s.auto_region.is_empty() && !regions.iter().any(|(_, c)| *c == s.auto_region) {
        regions.push((SharedString::from(s.auto_region.clone()), s.auto_region.clone()));
    }
    let current_region = regions.iter().position(|(_, c)| *c == s.auto_region).unwrap_or(0);
    let region_codes: Vec<String> = regions.iter().map(|(_, c)| c.clone()).collect();
    let region_options: Vec<(SharedString, usize)> = regions.into_iter().enumerate().map(|(i, (label, _))| (label, i)).collect();
    card(cx)
        .gap_2()
        .p_3()
        .child(
            h_flex()
                .gap_3()
                .flex_wrap()
                .items_center()
                .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(t!("auto.title")))
                .child(choice("auto-strategy", s.auto_strategy, strategies, move |v, _, _| {
                    e_mode.update_settings(|s| s.auto_strategy = v)
                }))
                .child(choice("auto-region", current_region, region_options, move |i, _, _| {
                    let code = region_codes.get(i).cloned().unwrap_or_default();
                    e_region.update_settings(move |s| s.auto_region = code)
                })),
        )
        .child(div().text_xs().text_color(t.muted_foreground).child(auto_hint(s.auto_strategy)))
}

fn auto_label(a: AutoStrategy) -> &'static str {
    match a {
        AutoStrategy::Quickest => t!("auto.quickest"),
        AutoStrategy::Steady => t!("auto.steady"),
        AutoStrategy::Ordered => t!("auto.ordered"),
        AutoStrategy::Bandwidth => t!("auto.bandwidth"),
    }
}

fn auto_hint(a: AutoStrategy) -> &'static str {
    match a {
        AutoStrategy::Quickest => t!("auto.quickest_hint"),
        AutoStrategy::Steady => t!("auto.steady_hint"),
        AutoStrategy::Ordered => t!("auto.ordered_hint"),
        AutoStrategy::Bandwidth => t!("auto.bandwidth_hint"),
    }
}

/// A two-letter country code; hovering tells where it came from.
fn country_badge(c: &Country, id: SharedString, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    let tip: SharedString = match &c.origin {
        Origin::Name => t!("geo.from_name").into(),
        Origin::Ip(ip) => t!("geo.from_ip", ip).into(),
        Origin::Exit(ip) => t!("geo.from_exit", ip).into(),
    };
    div()
        .id(id)
        .flex_none()
        .px_1()
        .rounded(dp(4.))
        .border_1()
        .border_color(t.border)
        .bg(t.muted)
        .text_xs()
        .font_family(t.mono_font_family.clone())
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted_foreground)
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx))
        .child(c.code.clone())
}

/// Colour interpolation on an animation phase.
trait PhaseExt {
    fn interpolate_hsla(self, a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> gpui_kit::Hsla;
}

impl PhaseExt for f32 {
    fn interpolate_hsla(self, a: gpui_kit::Hsla, b: gpui_kit::Hsla) -> gpui_kit::Hsla {
        gpui_kit::AnimationPhase(self).interpolate(a, b)
    }
}

impl Render for ProxiesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Cards share each row evenly at any window width.
        let cols = super::layout::grid_columns(super::layout::metrics(window).content, 236.0, 12.0);
        let engine = self.state.read(cx).engine.clone();
        let status = engine.status();
        if !status.is_running() {
            let e = engine.clone();
            let action = (engine.total_nodes() > 0 && engine.core_installed()).then(|| {
                Button::new("proxies-connect")
                    .primary()
                    .icon(extra_icon(Lucide::Power))
                    .label(t!("btn.connect"))
                    .loading(status.is_busy())
                    .on_click(move |_, _, _| e.start())
                    .into_any_element()
            });
            return page(t!("nav.proxies"), None, vec![], cx)
                .child(card(cx).child(empty_state(
                    extra_icon(Lucide::Earth),
                    t!("proxies.not_connected"),
                    t!("proxies.not_connected_hint"),
                    action,
                    cx,
                )))
                .into_any_element();
        }

        let groups = engine.groups();
        let current = self
            .group
            .clone()
            .filter(|g| groups.iter().any(|x| &x.name == g))
            .or_else(|| groups.first().map(|g| g.name.clone()))
            .unwrap_or_default();
        let Some(group) = groups.iter().find(|g| g.name == current).cloned() else {
            return page(t!("nav.proxies"), None, vec![], cx)
                .child(card(cx).child(empty_state(IconName::Inbox, t!("proxies.loading"), "", None, cx)))
                .into_any_element();
        };
        let filter = self.search.read(cx).value().to_lowercase();
        let members: Vec<(usize, ProxyMember)> = group
            .members
            .iter()
            .cloned()
            .enumerate()
            .filter(|(_, m)| {
                filter.is_empty()
                    || m.name.to_lowercase().contains(&filter)
                    || engine.country(&m.name).is_some_and(|c| c.code.eq_ignore_ascii_case(&filter))
            })
            .collect();
        let testable: Vec<String> = group
            .members
            .iter()
            .filter(|m| !m.is_group && m.name != "direct")
            .map(|m| m.name.clone())
            .collect();
        let testing = engine.any_testing();
        let speed_testing = engine.any_speed_testing();
        let e = engine.clone();
        let e_speed = engine.clone();
        // Speed tests download data, so only what is on screen.
        let speed_targets: Vec<String> =
            members.iter().filter(|(_, m)| !m.is_group && m.name != "direct").map(|(_, m)| m.name.clone()).collect();
        let tabs = self.group_tabs(&groups, &current, cx).into_any_element();
        let t = cx.theme();

        let actions = vec![
            div().w(dp(220.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
            Button::new("test-latency")
                .small()
                .icon(extra_icon(Lucide::Zap))
                .label(t!("btn.test_latency"))
                .loading(testing)
                .disabled(testable.is_empty())
                .on_click(move |_, _, _| e.test_delays(testable.clone()))
                .into_any_element(),
            Button::new("speed-test")
                .small()
                .icon(extra_icon(Lucide::Gauge))
                .label(t!("btn.speed_test"))
                .tooltip(t!("tip.speed_test"))
                .loading(speed_testing)
                .disabled(speed_targets.is_empty())
                .on_click(move |_, _, _| e_speed.speed_test(speed_targets.clone()))
                .into_any_element(),
        ];

        page(
            t!("nav.proxies"),
            Some(t!("proxies.subtitle", groups.len(), mode_label(engine.settings().mode))),
            actions,
            cx,
        )
        .child(tabs)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .min_w_0()
                .text_sm()
                .text_color(t.muted_foreground)
                .child(div().flex_none().child(if group.selectable() { t!("proxies.selector") } else { t!("proxies.urltest") }))
                .child("·")
                .child(div().min_w_0().truncate().text_color(t.foreground).child(t!("proxies.current", outbound_label(&group.now)))),
        )
        .when(group.name == boxpilot_core::config::TAG_AUTO, |d| d.child(auto_bar(&engine, cx)))
        .child(
            div().grid().grid_cols(cols).gap_3().children(
                members
                    .iter()
                    .enumerate()
                    .map(|(pos, (i, m))| {
                        anim::enter(
                            div().min_w_0().child(self.node_card(&group, *i, m, &groups, cx)),
                            SharedString::from(format!("enter-{}-{}", group.name, m.name)),
                            anim::stagger(pos.min(12)) * 0.5,
                        )
                    }),
            ),
        )
        .into_any_element()
    }
}
