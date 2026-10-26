//! Small building blocks shared by the pages.

use std::rc::Rc;

use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::AnimationExt as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    AnyElement, App, Div, ElementId, FontWeight, Hsla, SharedString, SpringAnimation, Window, div, px, relative,
};

use super::anim;
use crate::t;

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
        .border_color(t.border)
        .bg(t.background)
        .shadow_xs()
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
                    .min_w(px(220.))
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
                .min_w(px(220.))
                .gap_0p5()
                .child(div().text_sm().child(title.to_string()))
                .when_some(description, |d, s| {
                    d.child(div().text_xs().text_color(t.muted_foreground).child(s.to_string()))
                }),
        )
        .child(div().flex_shrink_0().ml_auto().child(control))
}

pub fn divider(cx: &App) -> Div {
    div().h(px(1.)).w_full().bg(cx.theme().border)
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
        .h(px(32.))
        .w(px(seg_w * options.len() as f32 + pad * 2.0))
        .rounded(t.radius)
        .bg(t.muted)
        .child(
            div()
                .absolute()
                .top(px(pad))
                .bottom(px(pad))
                .w(px(seg_w))
                .rounded(t.radius)
                .bg(t.background)
                .shadow_xs()
                .with_spring(
                    ElementId::from(SharedString::from(format!("{id}-thumb"))),
                    SpringAnimation::new(anim::BOUNCY).to(px(thumb_x)),
                    |d, x| d.left(x),
                ),
        )
        .child(
            h_flex()
                .absolute()
                .left(px(pad))
                .top_0()
                .bottom_0()
                .children(options.iter().enumerate().map(|(i, (label, value))| {
                    let selected = i == idx;
                    let value = *value;
                    let on_select = on_select.clone();
                    div()
                        .id(SharedString::from(format!("{id}-{i}")))
                        .w(px(seg_w))
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
        .h(px(6.))
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
    div().size(px(8.)).rounded_full().bg(color).flex_none()
}

pub fn pill(text: impl Into<SharedString>, fg: Hsla, bg: Hsla) -> Div {
    div()
        .flex_none()
        .px_2()
        .py(px(1.))
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
                .child(icon.into().size(px(22.))),
        )
        .child(div().font_weight(FontWeight::MEDIUM).text_center().child(title.to_string()))
        .when(!hint.is_empty(), |d| {
            d.child(
                div()
                    .text_sm()
                    .text_color(t.muted_foreground)
                    .max_w(px(460.))
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
        other => other.to_string(),
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
