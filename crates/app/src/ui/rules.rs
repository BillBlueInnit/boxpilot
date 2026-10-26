//! Rules page: built-in rule toggles, imported rule-sets (sing-box .srs/.json,
//! Clash / ACL4SSR lists, ACL4SSR .ini import) and custom rules.

use std::cell::Cell;
use std::rc::Rc;

use boxpilot_core::model::Settings;
use boxpilot_core::rules::{RuleFormat, RuleProvider};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AsyncApp, Context, Entity, FocusHandle, Focusable, FontWeight, PathPromptOptions, SharedString, Subscription,
    WeakEntity, Window, div,
};

use super::anim;
use super::subscriptions::{confirm_delete, field};
use super::widgets::*;
use crate::state::AppState;
use crate::t;
use super::layout::dp;

const ACL4SSR_FULL: &str =
    "https://raw.githubusercontent.com/ACL4SSR/ACL4SSR/master/Clash/config/ACL4SSR_Online_Full.ini";

pub struct RulesPage {
    state: Entity<AppState>,
    custom: Entity<TextareaState>,
    geosite: Entity<InputState>,
    geoip: Entity<InputState>,
    sel: Selection,
    _subs: Vec<Subscription>,
}

impl RulesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let s = state.read(cx).engine.settings();
        let custom = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(6)
                .placeholder("DOMAIN-SUFFIX,openai.com,proxy\nGEOSITE,netflix,proxy\nIP-CIDR,192.168.0.0/16,direct\nPROCESS-NAME,steam.exe,direct")
                .default_value(s.custom_rules.clone())
        });
        let geosite = cx.new(|cx| InputState::new(window, cx).default_value(s.geosite_url.clone()));
        let geoip = cx.new(|cx| InputState::new(window, cx).default_value(s.geoip_url.clone()));
        let commit = |get: fn(&Settings) -> String, set: fn(&mut Settings, String)| {
            move |this: &mut Self, input: &Entity<InputState>, e: &InputEvent, window: &mut Window, cx: &mut Context<Self>| {
                if !matches!(e, InputEvent::Blur | InputEvent::PressEnter { .. }) {
                    return;
                }
                let engine = this.state.read(cx).engine.clone();
                let value = input.read(cx).value().trim().to_string();
                if value == get(&engine.settings()) {
                    return;
                }
                if !value.contains("{name}") {
                    window.push_notification(Notification::error(t!("err.template")), cx);
                    let old = get(&engine.settings());
                    input.update(cx, |i, cx| i.set_value(old, window, cx));
                    return;
                }
                engine.update_settings(move |s| set(s, value));
            }
        };
        let _subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(&geosite, window, commit(|s| s.geosite_url.clone(), |s, v| s.geosite_url = v)),
            cx.subscribe_in(&geoip, window, commit(|s| s.geoip_url.clone(), |s, v| s.geoip_url = v)),
        ];
        Self { state, custom, geosite, geoip, sel: Selection::default(), _subs }
    }

    fn provider_row(&self, p: &RuleProvider, idx: usize, total: usize, this: WeakEntity<Self>, cx: &App) -> impl IntoElement {
        let selecting = self.sel.active;
        let selected = self.sel.ids.contains(&p.id);
        let pick_id = p.id.clone();
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let updating = engine.is_updating(&p.id);
        let (target_fg, target_bg) = match p.target.to_ascii_lowercase().as_str() {
            "direct" => (t.success, t.success.opacity(0.12)),
            "reject" | "block" => (t.danger, t.danger.opacity(0.12)),
            _ => (t.primary, t.primary.opacity(0.12)),
        };
        let mut meta = Vec::new();
        if p.rule_count > 0 {
            meta.push(t!("rules.count", p.rule_count));
        }
        meta.push(
            match p.stored.unwrap_or(p.format) {
                RuleFormat::SingBoxBinary => "SRS",
                RuleFormat::SingBoxSource if p.format != RuleFormat::SingBoxSource && p.format != RuleFormat::Auto => "Clash → sing-box",
                RuleFormat::SingBoxSource => "JSON",
                RuleFormat::Clash | RuleFormat::ClashDomain | RuleFormat::ClashIpCidr => "Clash",
                RuleFormat::Auto => "—",
            }
            .to_string(),
        );
        meta.push(match p.last_update {
            Some(ts) => t!("meta.updated", fmt_ago(ts)),
            None => t!("subs.never_updated").to_string(),
        });
        if p.interval_hours > 0 && p.is_remote() {
            meta.push(t!("meta.every", fmt_interval(p.interval_hours * 60)));
        }
        let (e1, e2, e3, e4, e5) = (engine.clone(), engine.clone(), engine.clone(), engine.clone(), engine.clone());
        let (id1, id2, id3, id4, id5) = (p.id.clone(), p.id.clone(), p.id.clone(), p.id.clone(), p.id.clone());
        let edit = p.clone();
        let name = p.name.clone();
        v_flex()
            .id(SharedString::from(format!("rp-row-{}", p.id)))
            .gap_1()
            .py_2()
            .rounded(t.radius)
            .when(!p.enabled, |d| d.opacity(0.6))
            .when(selecting, |d| {
                d.cursor_pointer().on_click(move |_, _, cx| {
                    let _ = this.update(cx, |page, cx| {
                        page.sel.toggle(&pick_id);
                        cx.notify();
                    });
                })
            })
            .when(selected, |d| d.bg(t.primary.opacity(0.06)))
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .when(selecting, |d| {
                        // Display only: the whole row handles the click.
                        d.child(
                            div()
                                .w(dp(28.))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .child(Checkbox::new(("rp-pick", idx)).checked(selected)),
                        )
                    })
                    .when(!selecting, |d| d.child(
                        v_flex()
                            .flex_none()
                            .child(
                                Button::new(("up", idx))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronUp)
                                    .tooltip(t!("tip.move_up"))
                                    .disabled(idx == 0)
                                    .on_click(move |_, _, _| e1.move_rule_provider(&id1, -1)),
                            )
                            .child(
                                Button::new(("down", idx))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronDown)
                                    .tooltip(t!("tip.move_down"))
                                    .disabled(idx + 1 == total)
                                    .on_click(move |_, _, _| e2.move_rule_provider(&id2, 1)),
                            ),
                    ))
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
                                    .child(div().min_w_0().font_weight(FontWeight::MEDIUM).truncate().child(p.name.clone()))
                                    .child(pill(target_label(&p.target), target_fg, target_bg))
                                    .when(updating, |d| d.child(pill(t!("subs.updating"), t.info, t.info.opacity(0.12))))
                                    .when(p.last_error.is_some() && !updating, |d| {
                                        d.child(pill(t!("subs.update_failed"), t.danger, t.danger.opacity(0.12)))
                                    }),
                            )
                            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(p.source.clone()))
                            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(meta.join("  ·  "))),
                    )
                    .when(!selecting, |d| d.child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .items_center()
                            .child(
                                Switch::new(("rp-en", idx))
                                    .checked(p.enabled)
                                    .tooltip(t!("tip.enabled"))
                                    .on_change(move |v, _, _| e3.set_rule_provider_enabled(&id3, *v)),
                            )
                            .child(
                                Button::new(("rp-upd", idx))
                                    .ghost()
                                    .small()
                                    .icon(IconName::RefreshCw)
                                    .tooltip(t!("tip.update_now"))
                                    .loading(updating)
                                    .on_click(move |_, _, _| e4.update_rule_provider(&id4)),
                            )
                            .child(
                                Button::new(("rp-edit", idx))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Pencil))
                                    .tooltip(t!("tip.edit"))
                                    .on_click(move |_, window, cx| open_rule_editor(Some(edit.clone()), window, cx)),
                            )
                            .child(
                                Button::new(("rp-del", idx))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Trash))
                                    .tooltip(t!("tip.delete"))
                                    .on_click(move |_, window, cx| {
                                        let (engine, id) = (e5.clone(), id5.clone());
                                        confirm_delete(
                                            t!("rules.delete_title"),
                                            t!("rules.delete_body", name),
                                            move |_| engine.remove_rule_provider(&id),
                                            window,
                                            cx,
                                        );
                                    }),
                            ),
                    )),
            )
            .when_some(p.last_error.clone().filter(|_| !updating), |d, err| {
                d.child(
                    h_flex()
                        .pl(dp(40.))
                        .gap_2()
                        .text_xs()
                        .text_color(t.danger)
                        .child(div().flex_none().child(IconName::TriangleAlert))
                        .child(div().flex_1().min_w_0().line_clamp(2).child(err)),
                )
            })
    }
}

fn switch_row(id: &'static str, title: &str, desc: &str, checked: bool, set: fn(&mut Settings, bool), cx: &App) -> impl IntoElement {
    let engine = AppState::engine(cx);
    setting_row(
        title,
        Some(desc),
        Switch::new(id).checked(checked).on_change(move |v, _, _| {
            let v = *v;
            engine.update_settings(move |s| set(s, v))
        }),
        cx,
    )
}

impl Render for RulesPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let s = engine.settings();
        let providers = engine.rule_providers();
        let t = cx.theme();
        let muted = t.muted_foreground;
        let e_all = engine.clone();
        let any_updating = providers.iter().any(|p| engine.is_updating(&p.id));
        let rules = self.custom.clone();
        let rules_engine = engine.clone();
        let order: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        self.sel.retain(&order);
        if providers.is_empty() {
            self.sel.active = false;
        }
        let this = cx.entity().downgrade();

        let actions = if self.sel.active {
            let chosen = Rc::new(self.sel.ordered(&order));
            let all = !order.is_empty() && chosen.len() == order.len();
            let (this_all, this_del, this_done) = (this.clone(), this.clone(), this.clone());
            let (e_en, e_upd, e_del) = (engine.clone(), engine.clone(), engine.clone());
            let (c_en, c_upd, c_del) = (chosen.clone(), chosen.clone(), chosen.clone());
            let order = order.clone();
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
                    enable: Rc::new(move |on, _, _| e_en.set_rule_providers_enabled(&c_en, on)),
                    update: Rc::new(move |_, _| e_upd.update_rule_providers(&c_upd)),
                    delete: Rc::new(move |window, cx| {
                        let (engine, ids, this) = (e_del.clone(), c_del.clone(), this_del.clone());
                        confirm_delete(
                            t!("bulk.delete_title"),
                            t!("bulk.delete_body", ids.len()),
                            move |cx| {
                                engine.remove_rule_providers(&ids);
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
            Button::new("select-rules")
                .small()
                .ghost()
                .icon(extra_icon(Lucide::ListChecks))
                .label(t!("btn.select"))
                .disabled(providers.is_empty())
                .on_click(move |_, _, cx| {
                    let _ = this_sel.update(cx, |page, cx| {
                        page.sel.active = true;
                        cx.notify();
                    });
                })
                .into_any_element(),
            Button::new("import-acl")
                .small()
                .icon(extra_icon(Lucide::Download))
                .label(t!("btn.import_acl"))
                .on_click(|_, window, cx| open_acl_import(window, cx))
                .into_any_element(),
            Button::new("rules-update-all")
                .small()
                .icon(IconName::RefreshCw)
                .label(t!("btn.update_all"))
                .loading(any_updating)
                .disabled(providers.is_empty())
                .on_click(move |_, _, _| e_all.update_all_rule_providers())
                .into_any_element(),
            Button::new("add-ruleset")
                .small()
                .primary()
                .icon(IconName::Plus)
                .label(t!("btn.add_ruleset"))
                .on_click(|_, window, cx| open_rule_editor(None, window, cx))
                .into_any_element(),
            ]
        };

        let list = card(cx)
            .gap_0()
            .child(section_title(t!("rules.sets"), cx))
            .when(providers.is_empty(), |d| {
                d.child(empty_state(extra_icon(Lucide::ListFilter), t!("rules.empty_title"), t!("rules.empty_hint"), None, cx))
            })
            .children(providers.iter().enumerate().map(|(i, p)| {
                v_flex()
                    .when(i > 0, |d| d.child(divider(cx)))
                    .child(anim::enter(
                        div().child(self.provider_row(p, i, providers.len(), this.clone(), cx)),
                        SharedString::from(format!("rp-{}", p.id)),
                        anim::stagger(i) * 0.5,
                    ))
            }));

        page(t!("nav.rules"), Some(t!("rules.subtitle", providers.len())), actions, cx).child(
            v_flex()
                .gap_4()
                .max_w(dp(900.))
                .child(anim::enter(list, "rules-list", 0.0))
                .child(anim::enter(
                    card(cx)
                        .gap_1()
                        .child(section_title(t!("rules.builtin"), cx))
                        .child(switch_row("cn", t!("rules.bypass_cn"), t!("rules.bypass_cn_desc"), s.bypass_cn, |s, v| s.bypass_cn = v, cx))
                        .child(switch_row("ads", t!("rules.block_ads"), t!("rules.block_ads_desc"), s.block_ads, |s, v| s.block_ads = v, cx))
                        .child(switch_row("rsproxy", t!("rules.via_proxy"), t!("rules.via_proxy_desc"), s.ruleset_via_proxy, |s, v| s.ruleset_via_proxy = v, cx))
                        .child(setting_row(t!("rules.geosite_url"), Some(t!("rules.template_desc")), div().w(dp(340.)).child(Input::new(&self.geosite).small()), cx))
                        .child(setting_row(t!("rules.geoip_url"), Some(t!("rules.template_desc")), div().w(dp(340.)).child(Input::new(&self.geoip).small()), cx)),
                    "rules-builtin",
                    anim::stagger(1),
                ))
                .child(anim::enter(
                    card(cx)
                        .gap_2()
                        .child(section_title(t!("rules.custom"), cx))
                        .child(div().text_xs().text_color(muted).child(t!("rules.custom_help")))
                        .child(Textarea::new(&self.custom).h(dp(150.)))
                        .child(
                            h_flex().justify_end().child(
                                Button::new("apply-rules")
                                    .small()
                                    .primary()
                                    .label(t!("btn.apply_rules"))
                                    .on_click(move |_, window, cx| {
                                        let text = rules.read(cx).value().to_string();
                                        rules_engine.update_settings(|s| s.custom_rules = text);
                                        window.push_notification(Notification::success(t!("rules.applied")), cx);
                                    }),
                            ),
                        ),
                    "rules-custom",
                    anim::stagger(2),
                )),
        )
    }
}

/// Form for adding or editing one rule-set.
pub struct RuleEditor {
    original: Option<RuleProvider>,
    name: Entity<InputState>,
    source: Entity<InputState>,
    interval: Entity<InputState>,
    format: RuleFormat,
    targets: Vec<String>,
    target: usize,
    error: Option<String>,
    focus: FocusHandle,
}

impl RuleEditor {
    fn new(original: Option<RuleProvider>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = AppState::engine(cx);
        let base = original.clone().unwrap_or_default();
        // Targets: the three actions plus any group/node names the user may route to.
        let mut targets = vec!["proxy".to_string(), "direct".to_string(), "reject".to_string()];
        for g in engine.groups() {
            if g.name != "Proxy" && !targets.contains(&g.name) {
                targets.push(g.name.clone());
            }
        }
        if !targets.iter().any(|t| t.eq_ignore_ascii_case(&base.target)) {
            targets.push(base.target.clone());
        }
        let target = targets.iter().position(|t| t.eq_ignore_ascii_case(&base.target)).unwrap_or(0);
        Self {
            name: cx.new(|cx| InputState::new(window, cx).placeholder("Google").default_value(base.name.clone())),
            source: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("https://…/Google.list  ·  D:\\rules\\geosite-google.srs")
                    .default_value(base.source.clone())
            }),
            interval: cx.new(|cx| InputState::new(window, cx).placeholder("24").default_value(base.interval_hours.to_string())),
            format: base.format,
            targets,
            target,
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

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let engine = AppState::engine(cx);
        let mut p = self.original.clone().unwrap_or_default();
        let source = self.source.read(cx).value().trim().trim_matches('"').to_string();
        let remote = source.starts_with("http://") || source.starts_with("https://");
        if !remote && !std::path::Path::new(&source).is_file() {
            return self.fail(t!("rules.err_source"), cx);
        }
        let interval = self.interval.read(cx).value().trim().to_string();
        let Ok(hours) = (if interval.is_empty() { Ok(24) } else { interval.parse::<u32>() }) else {
            return self.fail(t!("editor.err_interval"), cx);
        };
        let mut name = self.name.read(cx).value().trim().to_string();
        if name.is_empty() {
            name = source
                .rsplit(['/', '\\'])
                .next()
                .and_then(|f| f.split(['.', '?']).next())
                .unwrap_or("rule-set")
                .to_string();
        }
        p.name = name;
        p.source = source;
        p.format = self.format;
        p.target = self.targets[self.target].clone();
        p.interval_hours = hours;
        engine.save_rule_provider(p);
        true
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let _ = this.update_in(cx, |e, window, cx| {
                let path = path.display().to_string();
                e.source.update(cx, |i, cx| i.set_value(path, window, cx));
            });
        })
        .detach();
    }
}

impl Focusable for RuleEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for RuleEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let weak = cx.entity().downgrade();
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak);
        let formats: Vec<(SharedString, RuleFormat)> = RuleFormat::ALL
            .iter()
            .map(|f| {
                let label: SharedString = if *f == RuleFormat::Auto { t!("rules.format_auto").into() } else { f.label().into() };
                (label, *f)
            })
            .collect();
        let targets: Vec<(SharedString, usize)> = self
            .targets
            .iter()
            .enumerate()
            .map(|(i, t)| (SharedString::from(target_label(t)), i))
            .collect();
        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .child(field(t!("editor.name"), None, Input::new(&self.name), cx))
            .child(field(
                t!("rules.source"),
                Some(t!("rules.source_hint")),
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.source)))
                    .child(
                        Button::new("browse")
                            .label(t!("btn.browse"))
                            .on_click(move |_, window, cx| {
                                let _ = w1.update(cx, |e, cx| e.browse(window, cx));
                            }),
                    ),
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(field(
                        t!("rules.format"),
                        None,
                        choice("rule-format", self.format, formats, move |f, _, cx| {
                            let _ = w2.update(cx, |e, cx| {
                                e.format = f;
                                cx.notify();
                            });
                        }),
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().child(field(
                        t!("rules.target"),
                        Some(t!("rules.target_hint")),
                        choice("rule-target", self.target, targets, move |i, _, cx| {
                            let _ = w3.update(cx, |e, cx| {
                                e.target = i;
                                cx.notify();
                            });
                        }),
                        cx,
                    )))
                    .child(div().w(dp(150.)).flex_none().child(field(t!("editor.interval"), None, Input::new(&self.interval), cx))),
            )
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    h_flex()
                        .gap_2()
                        .p_2()
                        .rounded(t.radius)
                        .bg(t.danger.opacity(0.1))
                        .text_sm()
                        .text_color(t.danger)
                        .child(div().flex_none().child(IconName::CircleX))
                        .child(div().flex_1().min_w_0().child(e)),
                )
            })
    }
}

pub fn open_rule_editor(original: Option<RuleProvider>, window: &mut Window, cx: &mut App) {
    let editing = original.is_some();
    let editor = cx.new(|cx| RuleEditor::new(original, window, cx));
    let first = Rc::new(Cell::new(true));
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = editor.read(cx).source.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        let ok = editor.clone();
        dialog
            .title(if editing { t!("rules.edit_title") } else { t!("btn.add_ruleset") })
            .w(dp(640.))
            .child(editor.clone())
            .footer(
                DialogFooter::new()
                    .child(super::cancel_button())
                    .child(
                        Button::new("save-rule")
                            .primary()
                            .label(if editing { t!("btn.save") } else { t!("btn.add") })
                            .on_click(move |_, window, cx| {
                                if ok.update(cx, |e, cx| e.save(cx)) {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
}

fn open_acl_import(window: &mut Window, cx: &mut App) {
    let input = cx.new(|cx| InputState::new(window, cx).placeholder("https://…/ACL4SSR_Online_Full.ini"));
    let busy = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, _, cx| {
        let t = cx.theme();
        let (input_ok, input_preset) = (input.clone(), input.clone());
        let busy = busy.clone();
        dialog
            .title(t!("acl.title"))
            .w(dp(580.))
            .child(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().text_color(t.muted_foreground).child(t!("acl.desc")))
                    .child(Input::new(&input))
                    .child(
                        h_flex().child(
                            Button::new("acl-preset")
                                .small()
                                .ghost()
                                .icon(IconName::Star)
                                .label(t!("acl.preset"))
                                .on_click(move |_, window, cx| {
                                    input_preset.update(cx, |i, cx| i.set_value(ACL4SSR_FULL, window, cx));
                                }),
                        ),
                    ),
            )
            .footer(
                DialogFooter::new()
                    .child(super::cancel_button())
                    .child(
                        Button::new("acl-import")
                            .primary()
                            .label(t!("btn.import"))
                            .loading(busy.get())
                            .on_click(move |_, window, cx| {
                                let source = input_ok.read(cx).value().trim().to_string();
                                if source.is_empty() || busy.get() {
                                    return;
                                }
                                busy.set(true);
                                let engine = AppState::engine(cx);
                                let handle = window.window_handle();
                                let task = engine.runtime().spawn({
                                    let engine = engine.clone();
                                    async move { engine.import_acl_ini(&source).await }
                                });
                                let busy = busy.clone();
                                cx.spawn(async move |cx: &mut AsyncApp| {
                                    let result = task.await.map_err(anyhow::Error::from).and_then(|r| r);
                                    busy.set(false);
                                    let _ = handle.update(cx, |_, window, cx| match result {
                                        Ok(n) => {
                                            window.close_dialog(cx);
                                            window.push_notification(Notification::success(t!("acl.imported", n)), cx);
                                        }
                                        Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                                    });
                                })
                                .detach();
                            }),
                    ),
            )
    });
}
