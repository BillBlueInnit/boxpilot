//! Small building blocks shared by the pages.

use std::rc::Rc;

use boxpilot_core::geo::{Country, Origin};
use gpui_kit::assets::IconName as Lucide;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, Hsla, SharedString, SpringAnimation, Window, div, relative,
};

use super::anim;
use crate::t;
use super::layout::dp;

pub fn extra_icon(name: gpui_kit::assets::IconName) -> Icon {
    Icon::new(name)
}

pub fn card(cx: &App) -> Div {
    let t = cx.theme();
    v_flex()
        .min_w_0()
        .gap_3()
        .p_4()
        .rounded(t.radius_lg)
        .border_1()
        .border_color(hairline(cx))
        .bg(surface(cx))
        .shadow_xs()
}

/// Edge colour for cards and the page sheet. An opaque window keeps the theme
/// border. A translucent one uses Fluent's card stroke: black at low opacity,
/// which reads as a soft edge over any backdrop instead of a bright grey line.
pub fn hairline(cx: &App) -> Hsla {
    let t = cx.theme();
    if !super::layout::translucent() {
        return t.border;
    }
    gpui_kit::black().opacity(if t.is_dark() { 0.1 } else { 0.06 })
}

/// Page header with title, optional subtitle and right-aligned actions that
/// wrap below the title when space runs out (long translations).
pub fn page(title: &str, subtitle: Option<String>, actions: Vec<AnyElement>, cx: &App) -> Div {
    let t = cx.theme();
    v_flex().gap_4().w_full().min_w_0().child(
        h_flex()
            .w_full()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_3()
            .child(
                v_flex()
                    .flex_1()
                    .min_w(dp(220.))
                    .gap_0p5()
                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
                    .when_some(subtitle, |d, s| {
                        d.child(div().text_sm().text_color(t.muted_foreground).child(s))
                    }),
            )
            .child(
                h_flex()
                    .flex_wrap()
                    .justify_end()
                    .gap_2()
                    .items_center()
                    .children(actions),
            ),
    )
}

pub fn section_title(title: &str, cx: &App) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().foreground)
        .child(title.to_string())
}

/// A settings row: title + description on the left, control on the right.
/// The text column shrinks and wraps; the control keeps its size.
pub fn setting_row(title: &str, description: Option<&str>, control: impl IntoElement, cx: &App) -> Div {
    let t = cx.theme();
    // When the control and a readable text column don't fit side by side
    // (long translations, wide controls), the control wraps onto its own line.
    h_flex()
        .w_full()
        .flex_wrap()
        .gap_x_4()
        .gap_y_2()
        .py_2()
        .justify_between()
        .items_center()
        .child(
            v_flex()
                .flex_1()
                .min_w(dp(220.))
                .gap_0p5()
                .child(div().text_sm().child(title.to_string()))
                .when_some(description, |d, s| {
                    d.child(div().text_xs().text_color(t.muted_foreground).child(s.to_string()))
                }),
        )
        .child(div().flex_shrink_0().ml_auto().child(control))
}

pub fn divider(cx: &App) -> Div {
    div().h(dp(1.)).w_full().bg(cx.theme().border)
}

/// A dropdown button that shows `current` and lets the user pick one of `options`.
pub fn choice<T: Copy + PartialEq + 'static>(
    id: impl Into<SharedString>,
    current: T,
    options: Vec<(SharedString, T)>,
    on_select: impl Fn(T, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id: SharedString = id.into();
    let label = options
        .iter()
        .find(|(_, v)| *v == current)
        .map(|(l, _)| l.clone())
        .unwrap_or_default();
    let on_select = Rc::new(on_select);
    Button::new(id)
        .outline()
        .small()
        .label(label)
        .dropdown_caret(true)
        .dropdown_menu(move |mut menu, _, _| {
            for (label, value) in options.iter().cloned() {
                let on_select = on_select.clone();
                menu = menu.item(
                    PopupMenuItem::new(label)
                        .checked(value == current)
                        .on_click(move |_, window, cx| on_select(value, window, cx)),
                );
            }
            menu
        })
}

/// Approximate rendered width of `s` at `text_sm`, for equal-width segments.
fn text_width(s: &str) -> f32 {
    s.chars()
        .map(|c| if c.is_ascii() { 7.4 } else if (c as u32) < 0x2E80 { 8.2 } else { 14.0 })
        .sum()
}

/// Segmented control whose selection thumb slides on a spring.
pub fn segmented<T: Copy + PartialEq + 'static, F: Fn(T, &mut Window, &mut App) + 'static>(
    id: &str,
    options: &[(&'static str, T)],
    current: T,
    on_select: F,
    cx: &App,
) -> impl IntoElement + use<T, F> {
    let t = cx.theme();
    let on_select = Rc::new(on_select);
    let widest = options.iter().map(|(l, _)| text_width(l)).fold(0.0f32, f32::max);
    let seg_w = (widest + 26.0).clamp(56.0, 170.0);
    let pad = 3.0;
    let idx = options.iter().position(|(_, v)| *v == current).unwrap_or(0);
    let thumb_x = pad + idx as f32 * seg_w;
    h_flex()
        .relative()
        .flex_none()
        .h(dp(32.))
        .w(dp(seg_w * options.len() as f32 + pad * 2.0))
        .rounded(t.radius)
        .bg(t.muted)
        .child(
            div()
                .absolute()
                .top(dp(pad))
                .bottom(dp(pad))
                .w(dp(seg_w))
                .rounded(t.radius)
                .bg(t.background)
                .shadow_xs()
                .with_spring(
                    ElementId::from(SharedString::from(format!("{id}-thumb"))),
                    SpringAnimation::new(anim::BOUNCY).to(dp(thumb_x)),
                    |d, x| d.left(x),
                ),
        )
        .child(
            h_flex()
                .absolute()
                .left(dp(pad))
                .top_0()
                .bottom_0()
                .children(options.iter().enumerate().map(|(i, (label, value))| {
                    let selected = i == idx;
                    let value = *value;
                    let on_select = on_select.clone();
                    div()
                        .id(SharedString::from(format!("{id}-{i}")))
                        .w(dp(seg_w))
                        .h_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_sm()
                        .truncate()
                        .cursor_pointer()
                        .when(selected, |d| d.font_weight(FontWeight::MEDIUM).text_color(t.foreground))
                        .when(!selected, |d| {
                            d.text_color(t.muted_foreground).hover(|d| d.text_color(t.foreground))
                        })
                        .on_click(move |_, window, cx| on_select(value, window, cx))
                        .child(*label)
                })),
        )
}

/// A thin progress bar whose fill grows from zero on a spring.
pub fn progress_bar(id: impl Into<ElementId>, pct: f32, color: Hsla, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    div()
        .w_full()
        .h(dp(6.))
        .rounded_full()
        .bg(t.muted)
        .overflow_hidden()
        .child(
            div()
                .h_full()
                .rounded_full()
                .bg(color)
                .with_spring(
                    id,
                    SpringAnimation::new(anim::GENTLE).to(pct / 100.0).from(0.0),
                    |d, v| d.w(relative(v.clamp(0.0, 1.0))),
                ),
        )
}

pub fn dot(color: Hsla) -> Div {
    div().size(dp(8.)).rounded_full().bg(color).flex_none()
}

pub fn pill(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> Div {
    div()
        .flex_none()
        .px_2()
        .py(dp(1.))
        .rounded_full()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap()
        .text_color(fg)
        .bg(bg)
        .child(text.into())
}

pub fn delay_color(delay: Option<u32>, cx: &App) -> Hsla {
    let t = cx.theme();
    match delay {
        None => t.danger,
        Some(d) if d < 300 => t.success,
        Some(d) if d < 800 => t.warning,
        Some(_) => t.danger,
    }
}

pub fn empty_state(icon: impl Into<Icon>, title: &str, hint: &str, action: Option<AnyElement>, cx: &App) -> Div {
    let t = cx.theme();
    v_flex()
        .w_full()
        .py_12()
        .px_4()
        .gap_2()
        .items_center()
        .justify_center()
        .child(
            div()
                .p_3()
                .rounded_full()
                .bg(t.muted)
                .text_color(t.muted_foreground)
                .child(icon.into().size(dp(22.))),
        )
        .child(div().font_weight(FontWeight::MEDIUM).text_center().child(title.to_string()))
        .when(!hint.is_empty(), |d| {
            d.child(
                div()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .max_w(dp(460.))
                    .text_center()
                    .child(hint.to_string()),
            )
        })
        .when_some(action, |d, a| d.child(div().pt_2().child(a)))
}

pub fn fmt_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{b} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn fmt_speed(b: u64) -> String {
    format!("{}/s", fmt_bytes(b))
}

pub fn fmt_span(secs: i64) -> String {
    let s = secs.abs();
    if s < 60 {
        t!("time.s", s)
    } else if s < 3600 {
        t!("time.m", s / 60)
    } else if s < 86400 {
        let (h, m) = (s / 3600, s % 3600 / 60);
        if m == 0 { t!("time.h", h) } else { t!("time.hm", h, m) }
    } else {
        t!("time.d", s / 86400)
    }
}

pub fn fmt_ago(ts: i64) -> String {
    let d = boxpilot_core::model::now_unix() - ts;
    if d < 45 { t!("time.just_now").to_string() } else { t!("time.ago", fmt_span(d)) }
}

pub fn fmt_date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

pub fn fmt_interval(min: u32) -> String {
    if min == 0 { t!("time.manual").to_string() } else { fmt_span(min as i64 * 60) }
}

pub fn mode_label(mode: boxpilot_core::model::ProxyMode) -> &'static str {
    use boxpilot_core::model::ProxyMode;
    match mode {
        ProxyMode::Rule => t!("mode.rule"),
        ProxyMode::Global => t!("mode.global"),
        ProxyMode::Direct => t!("mode.direct"),
    }
}

/// Display name for built-in outbound tags.
pub fn outbound_label(tag: &str) -> String {
    match tag {
        "Proxy" => t!("group.proxy").to_string(),
        "Auto" => t!("group.auto").to_string(),
        "direct" => t!("group.direct").to_string(),
        other => strip_flags(other),
    }
}

/// Removes flag emoji (regional-indicator pairs): Windows draws them as bare
/// letters, and the country badge already says where a node is.
fn strip_flags(name: &str) -> String {
    let is_flag = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
    if !name.chars().any(is_flag) {
        return name.to_string();
    }
    let cleaned: String = name.chars().filter(|c| !is_flag(*c)).collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() { name.to_string() } else { cleaned }
}

/// Fill for cards. On a translucent window it is a faint veil over the
/// backdrop (light in light mode), so cards read as raised, as in Windows 11.
pub fn surface(cx: &App) -> Hsla {
    let t = cx.theme();
    if super::layout::surface_alpha() < 1.0 {
        let white = gpui_kit::white();
        // Fluent's card fills over Mica/Acrylic.
        if t.is_dark() { white.opacity(0.05) } else { white.opacity(0.7) }
    } else {
        t.background
    }
}

/// Human readable rule target (`proxy` / `direct` / `reject` / group name).
pub fn target_label(target: &str) -> String {
    match target.to_ascii_lowercase().as_str() {
        "proxy" => t!("target.proxy").to_string(),
        "direct" => t!("target.direct").to_string(),
        "reject" | "block" => t!("target.reject").to_string(),
        _ => outbound_label(target),
    }
}

/// Translates well-known core errors; others are shown as reported.
pub fn core_error_text(msg: &str) -> String {
    if msg.contains(boxpilot_core::engine::TUN_NEEDS_ADMIN) {
        t!("err.tun_admin").to_string()
    } else if msg.starts_with(boxpilot_core::engine::CORE_MISSING) {
        t!("err.core_missing").to_string()
    } else {
        msg.to_string()
    }
}

/// Multi-select state for list pages (subscriptions, rule-sets).
#[derive(Default)]
pub struct Selection {
    pub active: bool,
    pub ids: std::collections::HashSet<String>,
}

impl Selection {
    pub fn toggle(&mut self, id: &str) {
        if !self.ids.remove(id) {
            self.ids.insert(id.to_string());
        }
    }

    /// Drops ids that no longer exist.
    pub fn retain(&mut self, existing: &[String]) {
        self.ids.retain(|id| existing.contains(id));
    }

    /// Selected ids in display order.
    pub fn ordered(&self, order: &[String]) -> Vec<String> {
        order.iter().filter(|id| self.ids.contains(*id)).cloned().collect()
    }
}

type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// What the selection toolbar does; each closure acts on the current selection.
pub struct BulkOps {
    pub select_all: Rc<dyn Fn(bool, &mut Window, &mut App)>,
    pub enable: Rc<dyn Fn(bool, &mut Window, &mut App)>,
    pub update: Action,
    pub delete: Action,
    pub done: Action,
}

/// Page-header actions shown while selecting.
pub fn bulk_actions(count: usize, all: bool, ops: BulkOps, cx: &App) -> Vec<AnyElement> {
    use gpui_kit::component::Disableable as _;
    let muted = cx.theme().muted_foreground;
    let BulkOps { select_all, enable, update, delete, done } = ops;
    let disable = enable.clone();
    vec![
        div().text_sm().text_color(muted).child(t!("bulk.selected", count)).into_any_element(),
        Button::new("bulk-all")
            .small()
            .ghost()
            .label(if all { t!("btn.select_none") } else { t!("btn.select_all") })
            .on_click(move |_, window, cx| select_all(!all, window, cx))
            .into_any_element(),
        Button::new("bulk-enable")
            .small()
            .label(t!("btn.enable"))
            .disabled(count == 0)
            .on_click(move |_, window, cx| enable(true, window, cx))
            .into_any_element(),
        Button::new("bulk-disable")
            .small()
            .label(t!("btn.disable"))
            .disabled(count == 0)
            .on_click(move |_, window, cx| disable(false, window, cx))
            .into_any_element(),
        Button::new("bulk-update")
            .small()
            .icon(gpui_kit::component::IconName::RefreshCw)
            .label(t!("btn.update"))
            .disabled(count == 0)
            .on_click(move |_, window, cx| update(window, cx))
            .into_any_element(),
        Button::new("bulk-delete")
            .small()
            .danger()
            .icon(extra_icon(gpui_kit::assets::IconName::Trash))
            .label(t!("btn.delete"))
            .disabled(count == 0)
            .on_click(move |_, window, cx| delete(window, cx))
            .into_any_element(),
        Button::new("bulk-done")
            .small()
            .primary()
            .label(t!("btn.done"))
            .on_click(move |_, window, cx| done(window, cx))
            .into_any_element(),
    ]
}

/// One measurement on a node card.
#[derive(Clone, Copy)]
pub enum Metric {
    Untested,
    Testing,
    Failed,
    Ms(u32),
    Speed(u64),
}

/// A small clickable value: shows the result, runs its test on click.
pub fn metric_chip(
    id: SharedString,
    icon: Lucide,
    value: Metric,
    tip: &'static str,
    cx: &App,
    on_test: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let t = cx.theme();
    let (text, color) = match value {
        Metric::Untested => ("—".to_string(), t.muted_foreground),
        Metric::Testing => ("…".to_string(), t.muted_foreground),
        Metric::Failed => (t!("latency.timeout").to_string(), t.danger),
        Metric::Ms(ms) => (format!("{ms} ms"), delay_color(Some(ms), cx)),
        Metric::Speed(b) => (fmt_speed(b), t.foreground),
    };
    let key = SharedString::from(format!("{id}-{text}"));
    div()
        .id(id)
        .flex_none()
        .px_1()
        .py_0p5()
        .rounded(dp(4.))
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .cursor_pointer()
        .hover(|d| d.bg(t.muted))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx))
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            on_test(window, cx);
        })
        .child(
            h_flex()
                .gap_1()
                .items_center()
                .child(div().text_color(t.muted_foreground).child(extra_icon(icon).xsmall()))
                // A new result remounts the value, which fades in.
                .child(
                    div()
                        .with_spring(key, SpringAnimation::new(anim::SNAPPY).to(1.0f32).from(0.0), |d, p| d.opacity(p.clamp(0.0, 1.0)))
                        .child(text),
                ),
        )
}

/// A two-letter country code; hovering tells where it came from.
pub fn country_badge(c: &Country, id: SharedString, cx: &App) -> impl IntoElement {
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
