use boxpilot_core::{ProxyGroup, ProxyMember};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{App, Context, Entity, FontWeight, SharedString, SpringAnimation, Subscription, Window, div, px};

use super::anim;
use super::widgets::*;
use crate::state::AppState;
use crate::t;

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
                .max_w(px(260.))
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
            .w(px(236.))
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
                            .child(
                                div()
                                    .id(SharedString::from(format!("delay-{}-{idx}", group.name)))
                                    .flex_none()
                                    .px_1()
                                    .rounded(px(4.))
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
                    ),
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            .filter(|(_, m)| filter.is_empty() || m.name.to_lowercase().contains(&filter))
            .collect();
        let testable: Vec<String> = group
            .members
            .iter()
            .filter(|m| !m.is_group && m.name != "direct")
            .map(|m| m.name.clone())
            .collect();
        let testing = engine.any_testing();
        let e = engine.clone();
        let tabs = self.group_tabs(&groups, &current, cx).into_any_element();
        let t = cx.theme();

        let actions = vec![
            div().w(px(220.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
            Button::new("test-latency")
                .small()
                .icon(extra_icon(Lucide::Zap))
                .label(t!("btn.test_latency"))
                .loading(testing)
                .disabled(testable.is_empty())
                .on_click(move |_, _, _| e.test_delays(testable.clone()))
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
        .child(
            h_flex().gap_3().flex_wrap().children(
                members
                    .iter()
                    .enumerate()
                    .map(|(pos, (i, m))| {
                        anim::enter(
                            div().child(self.node_card(&group, *i, m, &groups, cx)),
                            SharedString::from(format!("enter-{}-{}", group.name, m.name)),
                            anim::stagger(pos.min(12)) * 0.5,
                        )
                    }),
            ),
        )
        .into_any_element()
    }
}
