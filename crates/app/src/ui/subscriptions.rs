use std::cell::Cell;
use std::rc::Rc;

use boxpilot_core::model::{Subscription, UpdateVia, now_unix};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, ClipboardItem, Context, Entity, FocusHandle, Focusable, FontWeight, SharedString,
    Subscription as GpuiSub, WeakEntity, Window, div,
};

use super::anim;
use super::widgets::*;
use crate::state::AppState;
use crate::t;
use super::layout::dp;

pub struct SubscriptionsPage {
    state: Entity<AppState>,
    sel: Selection,
    _sub: GpuiSub,
}

impl SubscriptionsPage {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let _sub = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, sel: Selection::default(), _sub }
    }

    fn sub_card(&self, sub: &Subscription, idx: usize, this: WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let selecting = self.sel.active;
        let selected = self.sel.ids.contains(&sub.id);
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let updating = engine.is_updating(&sub.id);
        let remote = sub.is_remote();
        let now = now_unix();

        let mut meta = vec![t!("meta.nodes", sub.node_count)];
        meta.push(match sub.last_update {
            Some(ts) => t!("meta.updated", fmt_ago(ts)),
            None => t!("subs.never_updated").to_string(),
        });
        if remote {
            if sub.auto_update && sub.interval_min > 0 {
                meta.push(t!("meta.every", fmt_interval(sub.interval_min)));
                if let Some(due) = sub.next_due().filter(|_| sub.enabled) {
                    meta.push(if due <= now { t!("meta.due_now").to_string() } else { t!("meta.next_in", fmt_span(due - now)) });
                }
            } else {
                meta.push(t!("meta.auto_off").to_string());
            }
            if sub.update_on_start {
                meta.push(t!("meta.on_startup").to_string());
            }
        }

        let (e_toggle, e_update, e_delete, e_links) = (engine.clone(), engine.clone(), engine.clone(), engine.clone());
        let (id1, id2, id3, id4) = (sub.id.clone(), sub.id.clone(), sub.id.clone(), sub.id.clone());
        let edit_sub = sub.clone();
        let url = sub.url.clone();
        let name_for_delete = sub.name.clone();
        let pick_id = sub.id.clone();

        card(cx)
            .id(SharedString::from(format!("sub-card-{}", sub.id)))
            .gap_2()
            .when(!sub.enabled, |d| d.opacity(0.65))
            .when(selecting, |d| {
                d.cursor_pointer().on_click(move |_, _, cx| {
                    let _ = this.update(cx, |page, cx| {
                        page.sel.toggle(&pick_id);
                        cx.notify();
                    });
                })
            })
            .when(selected, |d| d.border_color(t.primary).bg(t.primary.opacity(0.05)))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .when(selecting, |d| {
                        // Display only: the whole card handles the click.
                        d.child(Checkbox::new(SharedString::from(format!("pick-{idx}"))).checked(selected))
                    })
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
                            .child(if remote { extra_icon(Lucide::Rss) } else { IconName::File.into() }),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .truncate()
                                            .child(if sub.name.is_empty() { t!("subs.untitled").to_string() } else { sub.name.clone() }),
                                    )
                                    .when(!remote, |d| d.child(pill(t!("subs.local"), t.muted_foreground, t.muted)))
                                    .when(updating, |d| d.child(pill(t!("subs.updating"), t.info, t.info.opacity(0.12))))
                                    .when(sub.last_error.is_some() && !updating, |d| {
                                        d.child(pill(t!("subs.update_failed"), t.danger, t.danger.opacity(0.12)))
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(t.muted_foreground)
                                    .truncate()
                                    .child(if remote { sub.url.clone() } else { t!("subs.pasted").to_string() }),
                            ),
                    )
                    .when(!selecting, |d| d.child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .flex_none()
                            .child(
                                Toggle::new(SharedString::from(format!("en-{idx}")))
                                    .checked(sub.enabled)
                                    .tooltip(t!("tip.enabled"))
                                    .on_change(move |v, _, _| e_toggle.set_subscription_enabled(&id1, *v)),
                            )
                            .when(remote, |d| {
                                d.child(
                                    Button::new(SharedString::from(format!("copy-{idx}")))
                                        .ghost()
                                        .small()
                                        .icon(IconName::Copy)
                                        .tooltip(t!("tip.copy_url"))
                                        .on_click(move |_, window, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                                            window.push_notification(t!("subs.url_copied"), cx);
                                        }),
                                )
                                .child(
                                    Button::new(SharedString::from(format!("upd-{idx}")))
                                        .ghost()
                                        .small()
                                        .icon(IconName::RefreshCw)
                                        .tooltip(t!("tip.update_now"))
                                        .loading(updating)
                                        .on_click(move |_, _, _| e_update.update_subscription(&id2)),
                                )
                            })
                            .child(
                                Button::new(SharedString::from(format!("links-{idx}")))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Share2))
                                    .tooltip(t!("tip.copy_links"))
                                    .disabled(sub.node_count == 0)
                                    .on_click(move |_, window, cx| {
                                        let nodes = e_links.nodes(&id4);
                                        super::local_nodes::copy_links(&nodes, window, cx);
                                    }),
                            )
                            .child(
                                Button::new(SharedString::from(format!("edit-{idx}")))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Pencil))
                                    .tooltip(t!("tip.edit"))
                                    .on_click(move |_, window, cx| open_editor(Some(edit_sub.clone()), None, window, cx)),
                            )
                            .child(
                                Button::new(SharedString::from(format!("del-{idx}")))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Trash))
                                    .tooltip(t!("tip.delete"))
                                    .on_click(move |_, window, cx| {
                                        let (engine, id) = (e_delete.clone(), id3.clone());
                                        confirm_delete(t!("subs.delete_title"), t!("subs.delete_body", name_for_delete), move |_| engine.remove_subscription(&id), window, cx);
                                    }),
                            ),
                    )),
            )
            .child(div().text_xs().text_color(t.muted_foreground).child(meta.join("  ·  ")))
            .when_some(sub.info.clone(), |d, info| {
                let pct = if info.total > 0 { info.used() as f32 / info.total as f32 * 100.0 } else { 0.0 };
                let mut text = if info.total > 0 {
                    t!("usage.of", fmt_bytes(info.used()), fmt_bytes(info.total))
                } else {
                    t!("usage.used", fmt_bytes(info.used()))
                };
                let mut expiring = false;
                if info.expire > 0 {
                    let left = info.expire - now;
                    expiring = left < 7 * 86400;
                    let rel = if left > 0 { t!("usage.left", fmt_span(left)) } else { t!("usage.expired").to_string() };
                    text.push_str("  ·  ");
                    text.push_str(&t!("usage.expires", fmt_date(info.expire), rel));
                }
                let bar_c = if pct > 90.0 { t.danger } else { t.primary };
                d.child(
                    v_flex()
                        .gap_1()
                        .when(info.total > 0, |d| d.child(progress_bar(("usage", idx), pct, bar_c, cx)))
                        .child(div().text_xs().text_color(if expiring { t.warning } else { t.muted_foreground }).child(text)),
                )
            })
            .when_some(sub.last_error.clone().filter(|_| !updating), |d, err| {
                d.child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .text_xs()
                        .text_color(t.danger)
                        .child(div().flex_none().child(IconName::TriangleAlert))
                        .child(div().flex_1().min_w_0().line_clamp(3).child(err)),
                )
            })
    }
}

/// A small confirmation dialog for destructive actions.
pub fn confirm_delete(
    title: &'static str,
    body: String,
    on_confirm: impl Fn(&mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_confirm = Rc::new(on_confirm);
    window.open_dialog(cx, move |dialog, _, cx| {
        let on_confirm = on_confirm.clone();
        dialog
            .title(title)
            .w(dp(440.))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(body.clone()))
            .footer(
                DialogFooter::new()
                    .child(super::cancel_button())
                    .child(
                        Button::new("confirm-delete")
                            .danger()
                            .label(t!("btn.delete"))
                            .on_click(move |_, window, cx| {
                                on_confirm(cx);
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
}

impl Render for SubscriptionsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let subs = engine.subscriptions();
        let updating = engine.any_updating();
        let e = engine.clone();
        let remote = subs.iter().filter(|s| s.is_remote() && s.enabled).count();
        let order: Vec<String> = subs.iter().map(|s| s.id.clone()).collect();
        self.sel.retain(&order);
        if subs.is_empty() {
            self.sel.active = false;
        }
        let this = cx.entity().downgrade();
        let actions = if self.sel.active {
            let chosen = Rc::new(self.sel.ordered(&order));
            let all = !order.is_empty() && chosen.len() == order.len();
            let (this_all, this_del, this_done) = (this.clone(), this.clone(), this.clone());
            let (e_en, e_upd, e_del) = (engine.clone(), engine.clone(), engine.clone());
            let (c_en, c_upd, c_del) = (chosen.clone(), chosen.clone(), chosen.clone());
            bulk_actions(
                chosen.len(),
                all,
                BulkOps {
                    select_all: Rc::new(move |all, _, cx| {
                        let order = order.clone();
                        let _ = this_all.update(cx, |page, cx| {
                            page.sel.ids = if all { order.into_iter().collect() } else { Default::default() };
                            cx.notify();
                        });
                    }),
                    enable: Rc::new(move |on, _, _| e_en.set_subscriptions_enabled(&c_en, on)),
                    update: Rc::new(move |_, _| e_upd.update_subscriptions(&c_upd)),
                    delete: Rc::new(move |window, cx| {
                        let (engine, ids, this) = (e_del.clone(), c_del.clone(), this_del.clone());
                        confirm_delete(
                            t!("bulk.delete_title"),
                            t!("bulk.delete_body", ids.len()),
                            move |cx| {
                                engine.remove_subscriptions(&ids);
                                let _ = this.update(cx, |page, cx| {
                                    page.sel = Selection::default();
                                    cx.notify();
                                });
                            },
                            window,
                            cx,
                        );
                    }),
                    done: Rc::new(move |_, cx| {
                        let _ = this_done.update(cx, |page, cx| {
                            page.sel = Selection::default();
                            cx.notify();
                        });
                    }),
                },
                cx,
            )
        } else {
            let this_sel = this.clone();
            vec![
            Button::new("select-subs")
                .small()
                .ghost()
                .icon(extra_icon(Lucide::ListChecks))
                .label(t!("btn.select"))
                .disabled(subs.is_empty())
                .on_click(move |_, _, cx| {
                    let _ = this_sel.update(cx, |page, cx| {
                        page.sel.active = true;
                        cx.notify();
                    });
                })
                .into_any_element(),
            Button::new("paste-sub")
                .small()
                .icon(extra_icon(Lucide::ClipboardPaste))
                .label(t!("btn.paste"))
                .tooltip(t!("tip.paste"))
                .on_click(|_, window, cx| {
                    let text = cx.read_from_clipboard().and_then(|c| c.text()).unwrap_or_default();
                    let text = text.trim().to_string();
                    let node_links = super::local_nodes::is_node_links(&text);
                    let url = !text.contains('\n') && (text.starts_with("http://") || text.starts_with("https://"));
                    // A lone http(s) link with a #name is an HTTP proxy, not a subscription.
                    if url && !(node_links && text.contains('#')) {
                        open_editor(None, Some(Prefill::Url(text)), window, cx);
                    } else if node_links {
                        super::local_nodes::open_add_node(Some(text), window, cx);
                    } else if !text.is_empty() {
                        open_editor(None, Some(Prefill::Content(text)), window, cx);
                    } else {
                        window.push_notification(t!("subs.clipboard_empty"), cx);
                    }
                })
                .into_any_element(),
            Button::new("update-all")
                .small()
                .icon(IconName::RefreshCw)
                .label(t!("btn.update_all"))
                .loading(updating)
                .disabled(remote == 0)
                .on_click(move |_, _, _| e.update_all())
                .into_any_element(),
            Button::new("add-sub")
                .small()
                .primary()
                .icon(IconName::Plus)
                .label(t!("btn.add"))
                .dropdown_caret(true)
                .dropdown_menu(|menu, _, _| {
                    menu.item(
                        PopupMenuItem::new(t!("btn.add_subscription"))
                            .icon(extra_icon(Lucide::Rss))
                            .on_click(|_, window, cx| open_editor(None, None, window, cx)),
                    )
                    .item(
                        PopupMenuItem::new(t!("btn.add_node"))
                            .icon(extra_icon(Lucide::ServerPlus))
                            .on_click(|_, window, cx| super::local_nodes::open_add_node(None, window, cx)),
                    )
                })
                .into_any_element(),
            ]
        };
        let subtitle = t!("subs.subtitle", subs.len(), engine.total_nodes());
        let local = super::local_nodes::card_view(cx).map(|c| c.into_any_element());
        let has_local = local.is_some();
        page(t!("nav.subscriptions"), Some(subtitle), actions, cx)
            .when(subs.is_empty() && !has_local, |d| {
                d.child(card(cx).child(empty_state(
                    extra_icon(Lucide::Rss),
                    t!("subs.empty_title"),
                    t!("subs.empty_hint"),
                    Some(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("empty-add")
                                    .primary()
                                    .icon(IconName::Plus)
                                    .label(t!("btn.add_subscription"))
                                    .on_click(|_, window, cx| open_editor(None, None, window, cx)),
                            )
                            .child(
                                Button::new("empty-add-node")
                                    .icon(extra_icon(Lucide::ServerPlus))
                                    .label(t!("btn.add_node"))
                                    .on_click(|_, window, cx| super::local_nodes::open_add_node(None, window, cx)),
                            )
                            .into_any_element(),
                    ),
                    cx,
                )))
            })
            .child(v_flex().gap_3().children(subs.iter().enumerate().map(|(i, s)| {
                anim::enter(div().child(self.sub_card(s, i, this.clone(), cx)), SharedString::from(format!("sub-{}", s.id)), anim::stagger(i))
            })))
            .when_some(local, |d, local| {
                d.child(anim::enter(div().child(local), "local-nodes", anim::stagger(subs.len())))
            })
    }
}

pub enum Prefill {
    Url(String),
    Content(String),
}

/// Form state for adding or editing a subscription.
pub struct SubEditor {
    original: Option<Subscription>,
    local: bool,
    name: Entity<InputState>,
    url: Entity<InputState>,
    content: Entity<TextareaState>,
    interval_hours: Entity<InputState>,
    user_agent: Entity<InputState>,
    include: Entity<InputState>,
    exclude: Entity<InputState>,
    auto_update: bool,
    update_on_start: bool,
    update_via: UpdateVia,
    error: Option<String>,
    focus: FocusHandle,
}

impl SubEditor {
    fn new(original: Option<Subscription>, prefill: Option<Prefill>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = AppState::engine(cx);
        let settings = engine.settings();
        let base = original.clone().unwrap_or_else(|| Subscription {
            interval_min: settings.sub_default_interval_min,
            ..Default::default()
        });
        let local = original.as_ref().is_some_and(|s| !s.is_remote()) || matches!(prefill, Some(Prefill::Content(_)));
        let content_text = match (&prefill, &original) {
            (Some(Prefill::Content(c)), _) => c.clone(),
            (_, Some(s)) if !s.is_remote() => std::fs::read_to_string(engine.paths().subs_dir.join(format!("{}.raw", s.id))).unwrap_or_default(),
            _ => String::new(),
        };
        let url_text = match &prefill {
            Some(Prefill::Url(u)) => u.clone(),
            _ => base.url.clone(),
        };
        let input = |cx: &mut Context<Self>, window: &mut Window, value: String, placeholder: String| {
            cx.new(|cx| InputState::new(window, cx).placeholder(placeholder).default_value(value))
        };
        Self {
            name: input(cx, window, base.name.clone(), t!("editor.name_ph").into()),
            url: input(cx, window, url_text, "https://example.com/api/v1/client/subscribe?token=…".into()),
            content: cx.new(|cx| {
                TextareaState::new(window, cx)
                    .rows(8)
                    .placeholder(t!("editor.content_ph"))
                    .default_value(content_text)
            }),
            interval_hours: input(
                cx,
                window,
                format!("{}", base.interval_min as f64 / 60.0).trim_end_matches(".0").to_string(),
                "12".into(),
            ),
            user_agent: input(cx, window, base.user_agent.clone(), t!("editor.ua_ph", settings.sub_user_agent)),
            include: input(cx, window, base.include.clone(), "HK|SG|JP".into()),
            exclude: input(cx, window, base.exclude.clone(), "expire|traffic|官网".into()),
            auto_update: base.auto_update,
            update_on_start: base.update_on_start,
            update_via: base.update_via,
            local,
            original,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    fn fail(&mut self, msg: impl Into<String>, cx: &mut Context<Self>) -> bool {
        self.error = Some(msg.into());
        cx.notify();
        false
    }

    /// Validates the form and saves it. Returns false to keep the dialog open.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let engine = AppState::engine(cx);
        let mut sub = self.original.clone().unwrap_or_default();
        sub.name = self.name.read(cx).value().trim().to_string();
        sub.include = self.include.read(cx).value().trim().to_string();
        sub.exclude = self.exclude.read(cx).value().trim().to_string();
        sub.user_agent = self.user_agent.read(cx).value().trim().to_string();
        sub.auto_update = self.auto_update;
        sub.update_on_start = self.update_on_start;
        sub.update_via = self.update_via;
        let hours_text = self.interval_hours.read(cx).value().trim().to_string();
        let hours: f64 = if hours_text.is_empty() {
            12.0
        } else {
            match hours_text.parse() {
                Ok(h) if h >= 0.0 => h,
                _ => return self.fail(t!("editor.err_interval"), cx),
            }
        };
        sub.interval_min = (hours * 60.0).round() as u32;
        if sub.auto_update && sub.interval_min > 0 && sub.interval_min < 5 {
            sub.interval_min = 5;
        }
        for re in [&sub.include, &sub.exclude] {
            if !re.is_empty() {
                if let Err(e) = boxpilot_core::convert::filter_nodes(vec![], re, "") {
                    return self.fail(e, cx);
                }
            }
        }
        let content = if self.local {
            sub.url.clear();
            let c = self.content.read(cx).value().to_string();
            if c.trim().is_empty() {
                return self.fail(t!("editor.err_content"), cx);
            }
            if sub.name.is_empty() {
                sub.name = t!("subs.local").to_string();
            }
            Some(c)
        } else {
            let url = self.url.read(cx).value().trim().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return self.fail(t!("editor.err_url"), cx);
            }
            sub.url = url;
            None
        };
        let result = match &self.original {
            Some(_) => engine.save_subscription(sub, content),
            None => engine.add_subscription(sub, content).map(|_| ()),
        };
        match result {
            Ok(()) => true,
            Err(e) => self.fail(format!("{e:#}"), cx),
        }
    }
}

pub fn field(label: &str, hint: Option<&str>, control: impl IntoElement, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    v_flex()
        .min_w_0()
        .gap_1()
        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label.to_string()))
        .child(control)
        .when_some(hint, |d, h| d.child(div().text_xs().text_color(t.muted_foreground).child(h.to_string())))
}

impl Focusable for SubEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SubEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let editing = self.original.is_some();
        let local = self.local;
        let via = self.update_via;
        let weak = cx.entity().downgrade();
        let source_toggle = (!editing).then(|| {
            let weak = weak.clone();
            segmented(
                "source",
                &[(t!("editor.remote"), false), (t!("editor.paste"), true)],
                local,
                move |v, _, cx| {
                    let _ = weak.update(cx, |s, cx| {
                        s.local = v;
                        s.error = None;
                        cx.notify();
                    });
                },
                cx,
            )
        });
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak);
        let via_options: Vec<(SharedString, UpdateVia)> = vec![
            (t!("via.auto").into(), UpdateVia::Auto),
            (t!("via.direct").into(), UpdateVia::Direct),
            (t!("via.proxy").into(), UpdateVia::Proxy),
        ];

        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .when_some(source_toggle, |d, s| d.child(s))
            .child(field(t!("editor.name"), None, Input::new(&self.name), cx))
            .when(!local, |d| {
                d.child(field(t!("editor.url"), Some(t!("editor.url_hint")), Input::new(&self.url), cx))
            })
            .when(local, |d| d.child(field(t!("editor.content"), None, Textarea::new(&self.content).h(dp(150.)), cx)))
            .when(!local, |d| {
                d.child(
                    h_flex()
                        .gap_3()
                        .items_start()
                        .child(div().flex_1().min_w_0().child(field(t!("editor.interval"), Some(t!("editor.interval_hint")), Input::new(&self.interval_hours), cx)))
                        .child(div().flex_1().min_w_0().child(field(
                            t!("editor.via"),
                            Some(t!("editor.via_hint")),
                            choice("via", via, via_options, move |v, _, cx| {
                                let _ = w1.update(cx, |s, cx| {
                                    s.update_via = v;
                                    cx.notify();
                                });
                            }),
                            cx,
                        ))),
                )
                .child(setting_row(
                    t!("editor.auto_update"),
                    Some(t!("editor.auto_update_desc")),
                    Toggle::new("auto-update").checked(self.auto_update).on_change(move |v, _, cx| {
                        let v = *v;
                        let _ = w2.update(cx, |s, cx| {
                            s.auto_update = v;
                            cx.notify();
                        });
                    }),
                    cx,
                ))
                .child(setting_row(
                    t!("editor.update_start"),
                    Some(t!("editor.update_start_desc")),
                    Toggle::new("update-start").checked(self.update_on_start).on_change(move |v, _, cx| {
                        let v = *v;
                        let _ = w3.update(cx, |s, cx| {
                            s.update_on_start = v;
                            cx.notify();
                        });
                    }),
                    cx,
                ))
                .child(field(t!("editor.ua"), Some(t!("editor.ua_hint")), Input::new(&self.user_agent), cx))
            })
            .child(
                h_flex()
                    .gap_3()
                    .child(div().flex_1().min_w_0().child(field(t!("editor.include"), None, Input::new(&self.include), cx)))
                    .child(div().flex_1().min_w_0().child(field(t!("editor.exclude"), None, Input::new(&self.exclude), cx))),
            )
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

pub fn open_editor(original: Option<Subscription>, prefill: Option<Prefill>, window: &mut Window, cx: &mut App) {
    let editing = original.is_some();
    let editor = cx.new(|cx| SubEditor::new(original, prefill, window, cx));
    let first = Rc::new(Cell::new(true));
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = editor.read(cx).url.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        let editor_ok = editor.clone();
        dialog
            .title(if editing { t!("editor.edit_title") } else { t!("btn.add_subscription") })
            .w(dp(600.))
            .child(editor.clone())
            .footer(
                DialogFooter::new()
                    .child(super::cancel_button())
                    .child(
                        Button::new("save-sub")
                            .primary()
                            .label(if editing { t!("btn.save") } else { t!("btn.add") })
                            .on_click(move |_, window, cx| {
                                if editor_ok.update(cx, |e, cx| e.save(cx)) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}
