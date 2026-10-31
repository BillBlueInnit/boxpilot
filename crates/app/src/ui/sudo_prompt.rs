//! Linux without a polkit agent (common under tiling window managers):
//! BoxPilot asks for the password itself and grants the core TUN rights
//! through `sudo`. The password goes to sudo's stdin once and is not kept.

use std::cell::Cell;
use std::rc::Rc;

use boxpilot_core::model::Settings;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{App, ClipboardItem, Context, Entity, FocusHandle, Focusable, SharedString, Subscription, WeakEntity, Window, div};

use super::anim;
use super::layout::dp;
use super::subscriptions::field;
use crate::state::AppState;
use crate::t;

struct SudoPrompt {
    password: Entity<InputState>,
    error: Option<SharedString>,
    busy: bool,
    /// The same grant as a terminal command, for users who prefer that.
    hint: String,
    apply: Option<fn(&mut Settings)>,
    focus: FocusHandle,
    _enter: Subscription,
}

impl SudoPrompt {
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).value().to_string();
        if self.busy || password.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let engine = AppState::engine(cx);
        let task = engine.runtime().spawn({
            let engine = engine.clone();
            async move { engine.grant_tun_privilege_with_password(password).await }
        });
        let apply = self.apply;
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        super::after_grant(apply, &engine);
                        window.close_dialog(cx);
                    }
                    Err(e) => {
                        let wrong = e.to_string() == boxpilot_core::sys::WRONG_PASSWORD;
                        this.error = Some(if wrong { t!("admin.wrong_password").into() } else { format!("{e:#}").into() });
                        if wrong {
                            this.password.update(cx, |p, cx| p.set_value("", window, cx));
                        }
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
}

impl Focusable for SudoPrompt {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SudoPrompt {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let hint = self.hint.clone();
        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .child(div().text_sm().text_color(t.muted_foreground).child(t!("admin.body_password")))
            .child(field(t!("admin.password"), None, Input::new(&self.password).mask_toggle().disabled(self.busy), cx))
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
            .child(
                v_flex()
                    .gap_1()
                    .child(div().text_xs().text_color(t.muted_foreground).child(t!("admin.terminal")))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .p_2()
                                    .rounded(t.radius)
                                    .bg(t.muted)
                                    .text_xs()
                                    .font_family(t.mono_font_family.clone())
                                    .child(hint.clone()),
                            )
                            .child(
                                Button::new("sudo-copy")
                                    .ghost()
                                    .small()
                                    .icon(IconName::Copy)
                                    .tooltip(t!("btn.copy_command"))
                                    .on_click(move |_, window, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(hint.clone()));
                                        window.push_notification(Notification::success(t!("btn.copy_command")), cx);
                                    }),
                            ),
                    ),
            )
    }
}

/// Opens the password dialog; `apply` runs once the grant succeeds.
pub fn open(apply: Option<fn(&mut Settings)>, cx: &mut App) {
    super::show_main_window(cx);
    let Some(handle) = AppState::global(cx).read(cx).window else { return };
    let hint = AppState::engine(cx).tun_privilege_hint();
    let _ = handle.update(cx, move |_, window, cx| {
        let password = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder(t!("admin.password")));
        let form = cx.new(|cx| {
            let _enter = cx.subscribe_in(&password, window, |this: &mut SudoPrompt, _, e: &InputEvent, window, cx| {
                if matches!(e, InputEvent::PressEnter { .. }) {
                    this.submit(window, cx);
                }
            });
            SudoPrompt { password, error: None, busy: false, hint, apply, focus: cx.focus_handle(), _enter }
        });
        let first = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, window, cx| {
            if first.replace(false) {
                let focus = form.read(cx).password.read(cx).focus_handle(cx);
                focus.focus(window, cx);
            }
            let busy = form.read(cx).busy;
            let form_ok = form.clone();
            dialog
                .title(t!("admin.title"))
                .w(dp(520.))
                .child(form.clone())
                .footer(
                    DialogFooter::new().child(super::cancel_button()).child(
                        Button::new("sudo-grant")
                            .primary()
                            .label(t!("btn.grant"))
                            .loading(busy)
                            .on_click(move |_, window, cx| form_ok.update(cx, |f, cx| f.submit(window, cx))),
                    ),
                )
        });
    });
}
