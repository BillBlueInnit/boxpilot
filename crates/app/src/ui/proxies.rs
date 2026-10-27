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
        let country = (!member.is_group).then(|| engine.country(&member.name)).flatten();
        let measurable = !member.is_group && member.name != "direct";
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

        // Three measurements, each tested on its own by clicking it.
        let latency = match engine.delay(&member.name) {
            _ if engine.is_testing(&member.name) => Metric::Testing,
            None => Metric::Untested,
            Some(None) => Metric::Failed,
            Some(Some(d)) => Metric::Ms(d),
        };
        let real = match engine.real_delay(&member.name) {
            _ if engine.is_real_testing(&member.name) || engine.is_speed_testing(&member.name) => Metric::Testing,
            None => Metric::Untested,
            Some(None) => Metric::Failed,
            Some(Some(d)) => Metric::Ms(d),
        };
        let speed = match engine.speed(&member.name) {
            _ if engine.is_speed_testing(&member.name) => Metric::Testing,
            None => Metric::Untested,
            Some(r) => r.bytes_per_sec.map_or(Metric::Failed, Metric::Speed),
        };
        let name = member.name.clone();
        let (e1, e2, e3) = (engine.clone(), engine.clone(), engine.clone());
        let (n1, n2, n3) = (name.clone(), name.clone(), name.clone());
        let id = |kind: &str| SharedString::from(format!("{kind}-{}-{idx}", group.name));
        let metrics = h_flex()
            .gap_1()
            .items_center()
            .child(metric_chip(id("lat"), Lucide::Zap, latency, t!("tip.chip_latency"), cx, move |_, _| {
                e1.test_delays(vec![n1.clone()])
            }))
            .child(metric_chip(id("real"), Lucide::Activity, real, t!("tip.chip_real"), cx, move |_, _| {
                e2.test_real_latency(vec![n2.clone()])
            }))
            .child(div().flex_1())
            .child(metric_chip(id("spd"), Lucide::Gauge, speed, t!("tip.speed_one"), cx, move |_, _| {
                e3.speed_test(vec![n3.clone()])
            }));

        let selectable = group.selectable();
        let (g, n) = (group.name.clone(), member.name.clone());
        let e_sel = engine.clone();
        let (primary, border, bg) = (t.primary, hairline(cx), surface(cx));
        div()
            .id(SharedString::from(format!("node-{}-{idx}", group.name)))
            .w_full()
            // Cards in a row share its height whatever they show.
            .h_full()
            .p_3()
            .rounded(t.radius_lg)
            .border_1()
            .when(selectable, |d| {
                // Hover lights the edge in the theme colour (a shadow would show
                // through the translucent card and grey it).
                d.cursor_pointer().hover(move |d| d.border_color(primary.opacity(0.5))).on_click(move |_, _, _| e_sel.select_proxy(&g, &n))
            })
            .child(
                v_flex()
                    .h_full()
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
                    .child(div().text_xs().text_color(t.muted_foreground).truncate().child(subtitle))
                    .child(div().flex_1())
                    .when(measurable, |d| d.child(metrics)),
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

/// The Auto group's line: how it picks, which region, and what it uses now.
fn auto_line(engine: &std::sync::Arc<boxpilot_core::Engine>, group: &ProxyGroup, cx: &App) -> impl IntoElement {
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
    let hint = auto_hint(s.auto_strategy);
    h_flex()
        .gap_2()
        .flex_wrap()
        .items_center()
        .min_w_0()
        .text_sm()
        .text_color(t.muted_foreground)
        .child(choice("auto-strategy", s.auto_strategy, strategies, move |v, _, _| e_mode.update_settings(|s| s.auto_strategy = v)))
        .child(choice("auto-region", current_region, region_options, move |i, _, _| {
            let code = region_codes.get(i).cloned().unwrap_or_default();
            e_region.update_settings(move |s| s.auto_region = code)
        }))
        .child(
            div()
                .id("auto-hint")
                .flex_none()
                .child(IconName::Info)
                .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(hint).build(window, cx)),
        )
        .child("·")
        .child(div().min_w_0().truncate().text_color(t.foreground).child(t!("proxies.current", outbound_label(&group.now))))
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
        let real_testing = engine.any_real_testing();
        let e = engine.clone();
        let e_speed = engine.clone();
        let e_real = engine.clone();
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
                .tooltip(t!("tip.url_latency"))
                .loading(testing)
                .disabled(testable.is_empty())
                .on_click(move |_, _, _| e.test_delays(testable.clone()))
                .into_any_element(),
            Button::new("real-latency")
                .small()
                .icon(extra_icon(Lucide::Activity))
                .label(t!("btn.real_latency"))
                .tooltip(t!("tip.real_latency"))
                .loading(real_testing)
                .disabled(speed_targets.is_empty())
                .on_click({
                    let targets = speed_targets.clone();
                    move |_, _, _| e_real.test_real_latency(targets.clone())
                })
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
        .child(if group.name == boxpilot_core::config::TAG_AUTO {
            auto_line(&engine, &group, cx).into_any_element()
        } else {
            h_flex()
                .gap_2()
                .items_center()
                .min_w_0()
                .text_sm()
                .text_color(t.muted_foreground)
                .child(div().flex_none().child(if group.selectable() { t!("proxies.selector") } else { t!("proxies.urltest") }))
                .child("·")
                .child(div().min_w_0().truncate().text_color(t.foreground).child(t!("proxies.current", outbound_label(&group.now))))
                .into_any_element()
        })
        .child(
            div().grid().grid_cols(cols).gap_3().children(
                members
                    .iter()
                    .enumerate()
                    .map(|(pos, (i, m))| {
                        anim::enter(
                            div().min_w_0().h_full().child(self.node_card(&group, *i, m, &groups, cx)),
                            SharedString::from(format!("enter-{}-{}", group.name, m.name)),
                            anim::stagger(pos.min(12)) * 0.5,
                        )
                    }),
            ),
        )
        .into_any_element()
    }
}
