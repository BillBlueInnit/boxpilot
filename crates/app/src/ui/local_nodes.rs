//! Nodes added one by one from share links. They live in their own group,
//! "Local nodes", next to the subscriptions.

use std::cell::Cell;
use std::rc::Rc;

use boxpilot_core::convert::{parse_uri, share_link, share_links};
use boxpilot_core::model::Node;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString, Window, div};

use super::anim;
use super::layout::dp;
use super::subscriptions::{confirm_delete, field};
use super::widgets::*;
use crate::state::AppState;
use crate::t;

/// True when every line of `text` is a node link (so Paste adds nodes, not a subscription).
pub fn is_node_links(text: &str) -> bool {
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty()).peekable();
    lines.peek().is_some() && lines.all(|l| parse_uri(l).is_ok())
}

/// Copies share links to the clipboard and says how it went.
pub fn copy_links(nodes: &[Node], window: &mut Window, cx: &mut App) {
    let (text, skipped) = share_links(nodes);
    if text.is_empty() {
        window.push_notification(Notification::error(t!("share.none")), cx);
        return;
    }
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    let copied = nodes.len() - skipped.len();
    let mut msg = t!("share.copied_n", copied);
    if !skipped.is_empty() {
        msg.push_str(&t!("share.skipped", skipped.len()));
    }
    window.push_notification(msg, cx);
}

/// Copies one link, or reports why the node has none.
pub fn copy_link(link: Result<String, String>, window: &mut Window, cx: &mut App) {
    match link {
        Ok(link) => {
            cx.write_to_clipboard(ClipboardItem::new_string(link));
            window.push_notification(t!("share.copied"), cx);
        }
        Err(e) => window.push_notification(Notification::error(format!("{}: {e}", t!("share.failed"))), cx),
    }
}

/// The Local nodes card, when there are any.
pub fn card_view(cx: &App) -> Option<impl IntoElement> {
    let engine = AppState::engine(cx);
    let group = engine.local_group()?;
    let nodes = engine.nodes(&group.id);
    let t = cx.theme();
    let (e_toggle, id) = (engine.clone(), group.id.clone());
    let all = nodes.clone();
    let rows = nodes.iter().enumerate().map(|(i, node)| {
        let (e_del, name) = (engine.clone(), node.name.clone());
        let shared = node.clone();
        h_flex()
            .id(SharedString::from(format!("local-{i}")))
            .gap_3()
            .items_center()
            .py_1p5()
            .when(i > 0, |d| d.border_t_1().border_color(hairline(cx)))
            .child(
                div()
                    .w(dp(84.))
                    .flex_none()
                    .child(pill(node.protocol.to_uppercase(), t.muted_foreground, t.muted)),
            )
            .child(div().flex_1().min_w_0().text_sm().truncate().child(outbound_label(&node.name)))
            .child(
                div()
                    .flex_none()
                    .max_w(dp(260.))
                    .text_xs()
                    .text_color(t.muted_foreground)
                    .truncate()
                    .child(format!("{}:{}", node.server, node.port)),
            )
            .child(
                Button::new(SharedString::from(format!("local-share-{i}")))
                    .ghost()
                    .small()
                    .icon(extra_icon(Lucide::Share2))
                    .tooltip(t!("tip.share"))
                    .on_click(move |_, window, cx| copy_link(share_link(&shared), window, cx)),
            )
            .child(
                Button::new(SharedString::from(format!("local-del-{i}")))
                    .ghost()
                    .small()
                    .icon(extra_icon(Lucide::Trash))
                    .tooltip(t!("tip.delete"))
                    .on_click(move |_, window, cx| {
                        let engine = e_del.clone();
                        confirm_delete(
                            t!("local.delete_title"),
                            t!("local.delete_body", name),
                            move |_| {
                                let _ = engine.remove_local_node(i);
                            },
                            window,
                            cx,
                        );
                    }),
            )
    });
    Some(
        card(cx)
            .gap_2()
            .when(!group.enabled, |d| d.opacity(0.65))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .size(dp(34.))
                            .rounded(t.radius)
                            .bg(t.primary.opacity(0.1))
                            .text_color(t.primary)
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .child(extra_icon(Lucide::ServerPlus)),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(t!("local.title")))
                            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(t!("local.subtitle"))),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .flex_none()
                            .child(
                                Toggle::new("local-enabled")
                                    .checked(group.enabled)
                                    .tooltip(t!("tip.enabled"))
                                    .on_change(move |v, _, _| e_toggle.set_subscription_enabled(&id, *v)),
                            )
                            .child(
                                Button::new("local-share-all")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .tooltip(t!("tip.copy_links"))
                                    .on_click(move |_, window, cx| copy_links(&all, window, cx)),
                            )
                            .child(
                                Button::new("local-add")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Plus)
                                    .tooltip(t!("btn.add_node"))
                                    .on_click(|_, window, cx| open_add_node(None, window, cx)),
                            ),
                    ),
            )
            .child(div().text_xs().text_color(t.muted_foreground).child(t!("meta.nodes", nodes.len())))
            .child(v_flex().children(rows)),
    )
}

/// Form state for adding nodes from links.
struct AddNode {
    links: Entity<TextareaState>,
    error: Option<String>,
    focus: FocusHandle,
}

impl AddNode {
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.links.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.error = Some(t!("local.err_empty").into());
            cx.notify();
            return false;
        }
        match AppState::engine(cx).add_local_nodes(&text) {
            Ok((added, skipped)) => {
                let mut msg = t!("local.added", added);
                if !skipped.is_empty() {
                    msg.push_str(&t!("share.skipped", skipped.len()));
                }
                window.push_notification(Notification::success(msg), cx);
                true
            }
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
                false
            }
        }
    }
}

impl Focusable for AddNode {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AddNode {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .child(field(t!("local.links"), Some(t!("local.hint")), Textarea::new(&self.links).h(dp(150.)), cx))
            .when_some(self.error.clone(), |d, e| {
                d.child(anim::enter(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .rounded(t.radius)
                        .bg(t.danger.opacity(0.1))
                        .text_sm()
                        .text_color(t.danger)
                        .child(div().flex_none().child(IconName::CircleX))
                        .child(div().flex_1().min_w_0().child(e.clone())),
                    SharedString::from(format!("err-{e}")),
                    0.0,
                ))
            })
    }
}

/// Opens the Add node dialog, optionally filled with pasted links.
pub fn open_add_node(prefill: Option<String>, window: &mut Window, cx: &mut App) {
    let form = cx.new(|cx| AddNode {
        links: cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(6)
                .placeholder("vless://…\nhysteria2://…\nss://…")
                .default_value(prefill.unwrap_or_default())
        }),
        error: None,
        focus: cx.focus_handle(),
    });
    let first = Rc::new(Cell::new(true));
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = form.read(cx).links.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        let form_ok = form.clone();
        dialog
            .title(t!("btn.add_node"))
            .w(dp(600.))
            .child(form.clone())
            .footer(
                DialogFooter::new()
                    .child(super::cancel_button())
                    .child(Button::new("save-node").primary().label(t!("btn.add")).on_click(move |_, window, cx| {
                        if form_ok.update(cx, |f, cx| f.save(window, cx)) {
                            window.close_dialog(cx);
                        }
                    })),
            )
    });
}
