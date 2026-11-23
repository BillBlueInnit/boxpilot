//! Rules page: one-click profiles, top-to-bottom priority pipeline
//! (① Custom rules → ② Routing rule-sets → ③ Default fallback),
//! curated service library, inline rule match tester and collapsible advanced sources.

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use boxpilot_core::model::Settings;
use boxpilot_core::rules::{
    CustomRule, CustomRuleGroup, CustomRuleKind, RuleFormat, RuleMatchLayer, RuleProfile, RuleProvider,
    SERVICE_PRESETS, ServiceCategory, ServicePreset,
};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AsyncApp, Context, Div, Entity, FocusHandle, Focusable, FontWeight, PathPromptOptions, ScrollHandle,
    SharedString, Subscription, WeakEntity, Window, div,
};

use super::layout::dp;
use super::subscriptions::{confirm_delete, field};
use super::widgets::*;
use crate::state::AppState;
use crate::t;

const ACL4SSR_FULL: &str =
    "https://raw.githubusercontent.com/ACL4SSR/ACL4SSR/master/Clash/config/ACL4SSR_Online_Full.ini";

#[derive(Clone, Copy, PartialEq, Eq)]
enum CustomViewMode {
    Visual,
    Text,
}

#[derive(Clone)]
struct DraggedRule {
    line_index: usize,
    kind_label: String,
    value: String,
}

struct DragRulePreview(DraggedRule);

impl Render for DragRulePreview {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        h_flex()
            .gap_2()
            .px_2p5()
            .py_1p5()
            .rounded(t.radius)
            .border_1()
            .border_color(t.primary)
            .bg(t.popover)
            .items_center()
            .child(pill(self.0.kind_label.clone(), t.primary, t.primary.opacity(0.14)))
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.popover_foreground)
                    .child(self.0.value.clone()),
            )
    }
}

pub struct RulesPage {
    state: Entity<AppState>,
    // Custom rules: visual quick-add + groups + text editor
    custom_mode: CustomViewMode,
    quick_kind: CustomRuleKind,
    quick_value: Entity<InputState>,
    quick_target: String,
    quick_group: Option<String>,
    quick_error: Option<String>,
    collapsed_groups: HashSet<String>,
    custom_text: Entity<TextareaState>,
    /// The custom rules last synced into the text editor.
    custom_seen: String,
    // Rule-sets bulk selection
    sel: Selection,
    // Advanced collapsible section
    advanced_open: bool,
    geosite: Entity<InputState>,
    geoip: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl RulesPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let s = state.read(cx).engine.settings();
        let quick_value = cx.new(|cx| {
            InputState::new(window, cx).placeholder(quick_placeholder(CustomRuleKind::DomainSuffix))
        });
        let custom_text = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(7)
                .placeholder(
                    "DOMAIN-SUFFIX,openai.com,proxy\nGEOSITE,netflix,proxy\nIP-CIDR,192.168.0.0/16,direct\nPROCESS-NAME,steam.exe,direct",
                )
                .default_value(s.custom_rules.clone())
        });
        let geosite = cx.new(|cx| InputState::new(window, cx).default_value(s.geosite_url.clone()));
        let geoip = cx.new(|cx| InputState::new(window, cx).default_value(s.geoip_url.clone()));

        let commit = |get: fn(&Settings) -> String, set: fn(&mut Settings, String)| {
            move |this: &mut Self,
                  input: &Entity<InputState>,
                  e: &InputEvent,
                  window: &mut Window,
                  cx: &mut Context<Self>| {
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
            cx.subscribe_in(&quick_value, window, |this, _, e: &InputEvent, window, cx| {
                match e {
                    InputEvent::PressEnter { .. } => this.submit_quick_rule(window, cx),
                    InputEvent::Change if this.quick_error.is_some() => {
                        this.quick_error = None;
                        cx.notify();
                    }
                    _ => {}
                }
            }),
            cx.subscribe_in(&geosite, window, commit(|s| s.geosite_url.clone(), |s, v| s.geosite_url = v)),
            cx.subscribe_in(&geoip, window, commit(|s| s.geoip_url.clone(), |s, v| s.geoip_url = v)),
        ];

        Self {
            state,
            custom_mode: CustomViewMode::Visual,
            quick_kind: CustomRuleKind::DomainSuffix,
            quick_value,
            quick_target: "proxy".to_string(),
            quick_group: None,
            quick_error: None,
            collapsed_groups: HashSet::new(),
            custom_text,
            custom_seen: s.custom_rules.clone(),
            sel: Selection::default(),
            advanced_open: false,
            geosite,
            geoip,
            _subs,
        }
    }

    fn submit_quick_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.quick_value.read(cx).value().to_string();
        if raw.trim().is_empty() {
            return;
        }
        let engine = self.state.read(cx).engine.clone();
        match engine.add_structured_custom_rule_in_group(
            self.quick_kind,
            &raw,
            &self.quick_target,
            self.quick_group.as_deref(),
        ) {
            Ok(true) => {
                self.quick_error = None;
                self.quick_value.update(cx, |i, cx| i.set_value(String::new(), window, cx));
                window.push_notification(Notification::success(t!("rules.custom_added")), cx);
                cx.notify();
            }
            Ok(false) => {
                self.quick_error = None;
                window.push_notification(t!("rules.custom_exists"), cx);
            }
            Err(e) => {
                self.quick_error = Some(format!("{e:#}"));
                cx.notify();
            }
        }
    }

    fn pick_quick_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let kind = self.quick_kind;
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = path.display().to_string();
            let value = if kind == CustomRuleKind::ProcessName {
                boxpilot_core::rules::normalize_custom_value(CustomRuleKind::ProcessName, &text)
            } else {
                text
            };
            let _ = this.update_in(cx, |page, window, cx| {
                page.quick_error = None;
                page.quick_value.update(cx, |i, cx| i.set_value(value, window, cx));
                cx.notify();
            });
        })
        .detach();
    }

    /// Available routing targets for custom rules: Proxy, Direct, Reject + groups.
    fn custom_targets(&self, cx: &App) -> Vec<String> {
        let engine = self.state.read(cx).engine.clone();
        collect_targets(&engine)
    }

    fn render_profiles(&self, window: &Window, cx: &App) -> Div {
        let engine = self.state.read(cx).engine.clone();
        let active = engine.active_rule_profile();
        let providers = engine.rule_providers();
        let t = cx.theme();
        let cols = super::layout::grid_columns(super::layout::metrics(window).content - 32.0, 210.0, 10.0).min(4);

        let cards = RuleProfile::ALL.into_iter().enumerate().map(|(i, profile)| {
            let is_active = active == Some(profile);
            let title = profile_title(profile);
            let desc = profile_desc(profile);
            let e = engine.clone();
            let has_presets = providers.iter().any(|p| p.preset.is_some());
            let border = if is_active { t.primary } else { hairline(cx) };
            let bg = if is_active { t.primary.opacity(0.08) } else { surface(cx) };
            let primary_hover = t.primary.opacity(0.5);
            v_flex()
                .id(("profile-card", i))
                .h_full()
                .p_3()
                .gap_1()
                .rounded(t.radius)
                .border_1()
                .border_color(border)
                .bg(bg)
                .cursor_pointer()
                .hover(move |d| d.border_color(primary_hover))
                .on_click(move |_, window, cx| {
                    if is_active {
                        return;
                    }
                    let e = e.clone();
                    if has_presets {
                        confirm_apply_profile(profile, e, window, cx);
                    } else {
                        e.apply_rule_profile(profile);
                        window.push_notification(Notification::success(t!("rules.profile_applied", title)), cx);
                    }
                })
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .min_w_0()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .truncate()
                                .child(title),
                        )
                        .when(is_active, |d| {
                            d.child(pill(t!("rules.profile_active"), t.primary, t.primary.opacity(0.14)))
                        }),
                )
                .child(div().text_xs().text_color(t.muted_foreground).line_clamp(2).child(desc))
        });

        card(cx)
            .gap_2p5()
            .child(
                v_flex()
                    .gap_0p5()
                    .child(section_title(t!("rules.profiles_title"), cx))
                    .child(div().text_xs().text_color(t.muted_foreground).child(t!("rules.profiles_hint"))),
            )
            .child(div().grid().grid_cols(cols).gap_2p5().children(cards))
    }

    fn render_custom_section(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        sync_placeholder(&self.quick_value, quick_placeholder(self.quick_kind), window, cx);
        let engine = self.state.read(cx).engine.clone();
        let rules = engine.custom_rules_parsed();
        let custom_groups = engine.custom_rule_groups();
        let t = cx.theme();
        let this = cx.entity().downgrade();
        let targets = self.custom_targets(cx);
        if !targets.iter().any(|x| x.eq_ignore_ascii_case(&self.quick_target)) {
            self.quick_target = "proxy".to_string();
        }
        if let Some(gid) = &self.quick_group {
            if !custom_groups.iter().any(|g| g.id == *gid) {
                self.quick_group = None;
            }
        }
        let target_idx = targets
            .iter()
            .position(|x| x.eq_ignore_ascii_case(&self.quick_target))
            .unwrap_or(0);
        let target_opts: Vec<(SharedString, usize)> = targets
            .iter()
            .enumerate()
            .map(|(i, x)| (SharedString::from(target_label(x)), i))
            .collect();

        let (file_engine, apply_engine) = (engine.clone(), engine.clone());
        let text_state = self.custom_text.clone();
        let this_mode = this.clone();
        let header = h_flex()
            .gap_2()
            .flex_wrap()
            .items_center()
            .justify_between()
            .child(step_header("1", t!("rules.step_custom"), t!("rules.step_custom_badge"), rules.len(), cx))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("new-custom-group")
                            .small()
                            .icon(IconName::Plus)
                            .label(t!("rules.new_group"))
                            .on_click(|_, window, cx| open_custom_group_editor(None, window, cx)),
                    )
                    .child(segmented(
                        "custom-mode",
                        &[
                            (t!("rules.mode_visual"), CustomViewMode::Visual),
                            (t!("rules.mode_text"), CustomViewMode::Text),
                        ],
                        self.custom_mode,
                        move |mode, _, cx| {
                            let _ = this_mode.update(cx, |page, cx| {
                                page.custom_mode = mode;
                                cx.notify();
                            });
                        },
                        cx,
                    ))
                    .child(
                        Button::new("edit-custom-file")
                            .ghost()
                            .small()
                            .icon(extra_icon(Lucide::FilePen))
                            .tooltip(t!("tip.edit_custom_file"))
                            .on_click(move |_, window, cx| match file_engine.custom_rules_file() {
                                Ok(path) => crate::platform::open_in_text_editor(&path),
                                Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                            }),
                    ),
            );

        if self.custom_mode == CustomViewMode::Text {
            return card(cx)
                .gap_2p5()
                .child(header)
                .child(div().text_xs().text_color(t.muted_foreground).child(t!("rules.custom_help")))
                .child(blur_on_outside_click(&self.custom_text, Textarea::new(&self.custom_text).h(dp(190.)), cx))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().min_w_0().text_xs().text_color(t.muted_foreground).child(t!("rules.custom_file")))
                        .child(
                            Button::new("apply-rules")
                                .small()
                                .primary()
                                .label(t!("btn.apply_rules"))
                                .on_click(move |_, window, cx| {
                                    let text = text_state.read(cx).value().to_string();
                                    match apply_engine.set_custom_rules(text) {
                                        Ok(()) => window.push_notification(Notification::success(t!("rules.applied")), cx),
                                        Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
                                    }
                                }),
                        ),
                );
        }

        // Visual mode: inline quick-add bar + Ungrouped drop zone + Custom Rule Groups.
        let kind_opts: Vec<(SharedString, CustomRuleKind)> = CustomRuleKind::QUICK
            .into_iter()
            .map(|k| (SharedString::from(kind_label(k)), k))
            .collect();
        let (this_kind, this_target, this_group, this_pick, this_add) = (
            this.clone(),
            this.clone(),
            this.clone(),
            this.clone(),
            this.clone(),
        );
        let targets_for_quick = targets.clone();
        let show_pick_app = matches!(self.quick_kind, CustomRuleKind::ProcessName | CustomRuleKind::ProcessPath);

        // Optional destination group selector when custom groups exist.
        let mut group_ids: Vec<Option<String>> = vec![None];
        let mut group_opts: Vec<(SharedString, usize)> = vec![(SharedString::from(t!("rules.ungrouped")), 0)];
        for (i, g) in custom_groups.iter().enumerate() {
            group_ids.push(Some(g.id.clone()));
            group_opts.push((SharedString::from(g.name.clone()), i + 1));
        }
        let quick_group_idx = match &self.quick_group {
            None => 0,
            Some(gid) => group_ids.iter().position(|id| id.as_deref() == Some(gid.as_str())).unwrap_or(0),
        };
        let active_quick_group = self
            .quick_group
            .as_deref()
            .and_then(|gid| custom_groups.iter().find(|g| g.id == gid));

        let quick_bar = h_flex()
            .gap_2()
            .flex_wrap()
            .items_center()
            .child(choice("quick-kind", self.quick_kind, kind_opts, move |k, _, cx| {
                let _ = this_kind.update(cx, |page, cx| {
                    page.quick_kind = k;
                    page.quick_error = None;
                    cx.notify();
                });
            }))
            .child(div().flex_1().min_w(dp(190.)).child(blur_on_outside_click(&self.quick_value, Input::new(&self.quick_value).small(), cx)))
            .when(show_pick_app, |d| {
                d.child(
                    Button::new("quick-pick-app")
                        .small()
                        .icon(extra_icon(Lucide::FolderOpen))
                        .label(t!("rules.pick_app"))
                        .on_click(move |_, window, cx| {
                            let _ = this_pick.update(cx, |page, cx| page.pick_quick_app(window, cx));
                        }),
                )
            })
            .when(!custom_groups.is_empty(), |d| {
                let group_ids = group_ids.clone();
                d.child(choice("quick-group", quick_group_idx, group_opts, move |i, _, cx| {
                    if let Some(dest) = group_ids.get(i).cloned() {
                        let _ = this_group.update(cx, |page, cx| {
                            page.quick_group = dest;
                            cx.notify();
                        });
                    }
                }))
            })
            .when(active_quick_group.is_none(), |d| {
                d.child(choice("quick-target", target_idx, target_opts, move |i, _, cx| {
                    if let Some(target) = targets_for_quick.get(i).cloned() {
                        let _ = this_target.update(cx, |page, cx| {
                            page.quick_target = target;
                            cx.notify();
                        });
                    }
                }))
            })
            .when_some(active_quick_group, |d, g| {
                let (fg, bg) = target_colors(g.display_target(), cx);
                d.child(pill(format!("→ {}", target_label(g.display_target())), fg, bg))
            })
            .child(
                Button::new("quick-add-rule")
                    .small()
                    .primary()
                    .icon(IconName::Plus)
                    .label(t!("btn.add"))
                    .on_click(move |_, window, cx| {
                        let _ = this_add.update(cx, |page, cx| page.submit_quick_rule(window, cx));
                    }),
            );

        // Partition rules into Ungrouped (plus any ad-hoc `#!group` not registered) and each CustomRuleGroup.
        let ungrouped: Vec<&CustomRule> = rules
            .iter()
            .filter(|r| {
                r.group.as_deref().is_none_or(|gn| {
                    !custom_groups.iter().any(|g| g.name.eq_ignore_ascii_case(gn))
                })
            })
            .collect();
        let has_groups = !custom_groups.is_empty();
        let primary = t.primary;
        let e_drop_ungrouped = engine.clone();

        card(cx)
            .gap_2p5()
            .child(header)
            .child(quick_bar)
            .when_some(self.quick_error.clone(), |d, err| {
                d.child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .px_2p5()
                        .py_1p5()
                        .rounded(t.radius)
                        .bg(t.danger.opacity(0.1))
                        .text_xs()
                        .text_color(t.danger)
                        .child(IconName::CircleX)
                        .child(err),
                )
            })
            .when(rules.is_empty() && !has_groups, |d| {
                d.child(empty_state(
                    extra_icon(Lucide::Route),
                    t!("rules.custom_empty"),
                    t!("rules.custom_empty_hint"),
                    None,
                    cx,
                ))
            })
            .when(!has_groups && !ungrouped.is_empty(), |d| {
                let total = ungrouped.len();
                let rows: Vec<gpui_kit::AnyElement> = ungrouped
                    .iter()
                    .enumerate()
                    .map(|(idx, r)| {
                        let row = self.custom_rule_row(r, idx, total, &targets, false, cx);
                        v_flex().when(idx > 0, |d| d.child(divider(cx))).child(row).into_any_element()
                    })
                    .collect();
                d.child(v_flex().gap_0().children(rows))
            })
            .when(has_groups, |d| {
                let ungrouped_total = ungrouped.len();
                let ungrouped_rows: Vec<gpui_kit::AnyElement> = ungrouped
                    .iter()
                    .enumerate()
                    .map(|(idx, r)| {
                        let row = self.custom_rule_row(r, idx, ungrouped_total, &targets, false, cx);
                        v_flex().when(idx > 0, |d| d.child(divider(cx))).child(row).into_any_element()
                    })
                    .collect();
                let ungrouped_zone = v_flex()
                    .id("custom-ungrouped-zone")
                    .gap_1()
                    .p_2p5()
                    .rounded(t.radius)
                    .border_1()
                    .border_color(hairline(cx))
                    .bg(t.muted.opacity(0.22))
                    .drag_over::<DraggedRule>(move |style, _, _, _| {
                        style.border_color(primary).bg(primary.opacity(0.08))
                    })
                    .on_drop::<DraggedRule>(move |dragged, window, cx| {
                        match e_drop_ungrouped.move_custom_rule_to_group(dragged.line_index, None) {
                            Ok(()) => {
                                window.push_notification(Notification::success(t!("rules.moved_ungrouped")), cx);
                            }
                            Err(e) => {
                                window.push_notification(Notification::error(format!("{e:#}")), cx);
                            }
                        }
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).child(t!("rules.ungrouped")))
                            .when(ungrouped_total > 0, |d| {
                                d.child(pill(ungrouped_total.to_string(), t.muted_foreground, t.muted))
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(t.muted_foreground)
                                    .child(format!("·  {}", t!("rules.ungrouped_hint"))),
                            ),
                    )
                    .when(ungrouped_rows.is_empty(), |d| {
                        d.child(
                            div()
                                .py_1()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(t!("rules.ungrouped_drop_hint")),
                        )
                    })
                    .when(!ungrouped_rows.is_empty(), |d| d.child(v_flex().gap_0().children(ungrouped_rows)));

                let group_total = custom_groups.len();
                let group_cards: Vec<gpui_kit::AnyElement> = custom_groups
                    .iter()
                    .enumerate()
                    .map(|(gidx, g)| {
                        let members: Vec<&CustomRule> = rules
                            .iter()
                            .filter(|r| r.group.as_deref().is_some_and(|gn| gn.eq_ignore_ascii_case(&g.name)))
                            .collect();
                        self.custom_group_card(g, gidx, group_total, &members, &targets, this.clone(), cx)
                    })
                    .collect();

                d.child(v_flex().gap_2p5().child(ungrouped_zone).children(group_cards))
            })
    }

    fn custom_group_card(
        &self,
        g: &CustomRuleGroup,
        gidx: usize,
        gtotal: usize,
        members: &[&CustomRule],
        targets: &[String],
        this: WeakEntity<Self>,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let primary = t.primary;
        let collapsed = self.collapsed_groups.contains(&g.id);
        let own_active = g.own_group && g.can_own_group();

        // Inline target dropdown identical to rule-set rows: Proxy / Direct / Reject / Own group / other groups.
        const OWN_SENTINEL: &str = "__own_group__";
        let mut target_values: Vec<String> = vec!["proxy".into(), "direct".into(), "reject".into()];
        if g.can_own_group() {
            target_values.push(OWN_SENTINEL.into());
        }
        for pg in engine.groups() {
            let is_self = engine.custom_rule_group_for_tag(&pg.name).is_some_and(|cg| cg.id == g.id);
            if !is_self
                && pg.name != boxpilot_core::config::TAG_PROXY
                && pg.name != boxpilot_core::config::TAG_AUTO
                && !target_values.contains(&pg.name)
            {
                target_values.push(pg.name.clone());
            }
        }
        if !own_active && !target_values.iter().any(|x| x.eq_ignore_ascii_case(&g.target)) {
            target_values.push(g.target.clone());
        }
        let current_target_idx = if own_active {
            target_values.iter().position(|x| x == OWN_SENTINEL).unwrap_or(0)
        } else {
            target_values
                .iter()
                .position(|x| x.eq_ignore_ascii_case(&g.target))
                .unwrap_or(0)
        };
        let target_opts: Vec<(SharedString, usize)> = target_values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = if v == OWN_SENTINEL {
                    t!("rules.own_group_option").to_string()
                } else {
                    target_label(v)
                };
                (SharedString::from(label), i)
            })
            .collect();

        let (e_up, e_dn, e_tgt, e_en, e_del, e_drop) = (
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
        );
        let (id_up, id_dn, id_tgt, id_en, id_del, id_drop, id_col) = (
            g.id.clone(),
            g.id.clone(),
            g.id.clone(),
            g.id.clone(),
            g.id.clone(),
            g.id.clone(),
            g.id.clone(),
        );
        let prev_target = if g.can_own_group() { g.target.clone() } else { "proxy".to_string() };
        let group_tag = own_active.then(|| engine.custom_rule_group_tag_for(&g.id).unwrap_or_else(|| g.name.clone()));
        let state_jump = self.state.clone();
        let g_rename = g.clone();
        let g_name_del = g.name.clone();
        let g_name_drop = g.name.clone();
        let member_count = members.len();

        let member_rows: Vec<gpui_kit::AnyElement> = if collapsed {
            Vec::new()
        } else {
            members
                .iter()
                .enumerate()
                .map(|(idx, r)| {
                    let row = self.custom_rule_row(r, idx, member_count, targets, true, cx);
                    v_flex().when(idx > 0, |d| d.child(divider(cx))).child(row).into_any_element()
                })
                .collect()
        };

        v_flex()
            .id(SharedString::from(format!("cg-card-{}", g.id)))
            .gap_1p5()
            .p_2p5()
            .rounded(t.radius)
            .border_1()
            .border_color(hairline(cx))
            .bg(t.muted.opacity(0.32))
            .when(!g.enabled, |d| d.opacity(0.58))
            .drag_over::<DraggedRule>(move |style, _, _, _| {
                style.border_color(primary).bg(primary.opacity(0.08))
            })
            .on_drop::<DraggedRule>(move |dragged, window, cx| {
                match e_drop.move_custom_rule_to_group(dragged.line_index, Some(&id_drop)) {
                    Ok(()) => {
                        window.push_notification(
                            Notification::success(t!("rules.moved_to_group", g_name_drop)),
                            cx,
                        );
                    }
                    Err(e) => {
                        window.push_notification(Notification::error(format!("{e:#}")), cx);
                    }
                }
            })
            .child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .gap_1p5()
                            .items_center()
                            .min_w_0()
                            .child(
                                v_flex()
                                    .flex_none()
                                    .child(
                                        Button::new(("cg-up", gidx))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::ChevronUp)
                                            .tooltip(t!("tip.move_up"))
                                            .disabled(gidx == 0)
                                            .on_click(move |_, _, _| e_up.move_custom_rule_group(&id_up, -1)),
                                    )
                                    .child(
                                        Button::new(("cg-dn", gidx))
                                            .ghost()
                                            .xsmall()
                                            .icon(IconName::ChevronDown)
                                            .tooltip(t!("tip.move_down"))
                                            .disabled(gidx + 1 == gtotal)
                                            .on_click(move |_, _, _| e_dn.move_custom_rule_group(&id_dn, 1)),
                                    ),
                            )
                            .child(
                                Button::new(("cg-fold", gidx))
                                    .ghost()
                                    .xsmall()
                                    .icon(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown })
                                    .on_click(move |_, _, cx| {
                                        let _ = this.update(cx, |page, cx| {
                                            if !page.collapsed_groups.remove(&id_col) {
                                                page.collapsed_groups.insert(id_col.clone());
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .truncate()
                                    .child(g.name.clone()),
                            )
                            .child(pill(t!("rules.group_count", member_count), t.muted_foreground, t.muted)),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1p5()
                            .items_center()
                            .child(choice(
                                SharedString::from(format!("cg-tgt-{}", g.id)),
                                current_target_idx,
                                target_opts,
                                move |i, _, _| {
                                    let Some(v) = target_values.get(i) else { return };
                                    if v == OWN_SENTINEL {
                                        e_tgt.set_custom_rule_group_target(&id_tgt, prev_target.clone(), true);
                                    } else {
                                        e_tgt.set_custom_rule_group_target(&id_tgt, v.clone(), false);
                                    }
                                },
                            ))
                            .when_some(group_tag.filter(|_| g.enabled), |d, gtag| {
                                d.child(
                                    Button::new(("cg-jump", gidx))
                                        .ghost()
                                        .xsmall()
                                        .label(t!("rules.goto_proxies"))
                                        .on_click(move |_, _, cx| {
                                            state_jump.update(cx, |s, cx| s.open_proxy_group(gtag.clone(), cx));
                                        }),
                                )
                            })
                            .child(
                                Toggle::new(("cg-en", gidx))
                                    .checked(g.enabled)
                                    .tooltip(t!("tip.enabled"))
                                    .on_change(move |v, _, _| e_en.set_custom_rule_group_enabled(&id_en, *v)),
                            )
                            .child(
                                Button::new(("cg-edit", gidx))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Pencil))
                                    .tooltip(t!("rules.rename_group"))
                                    .on_click(move |_, window, cx| {
                                        open_custom_group_editor(Some(g_rename.clone()), window, cx)
                                    }),
                            )
                            .child(
                                Button::new(("cg-del", gidx))
                                    .ghost()
                                    .small()
                                    .icon(extra_icon(Lucide::Trash))
                                    .tooltip(t!("tip.delete"))
                                    .on_click(move |_, window, cx| {
                                        let (engine, id) = (e_del.clone(), id_del.clone());
                                        confirm_delete(
                                            t!("rules.delete_group_title"),
                                            t!("rules.delete_group_body", g_name_del),
                                            move |_| engine.remove_custom_rule_group(&id),
                                            window,
                                            cx,
                                        );
                                    }),
                            ),
                    ),
            )
            .when(!collapsed, |d| {
                d.child(divider(cx))
                    .when(member_rows.is_empty(), |d| {
                        d.child(
                            div()
                                .py_2()
                                .px_2()
                                .text_xs()
                                .text_color(t.muted_foreground)
                                .child(t!("rules.group_drop_hint")),
                        )
                    })
                    .when(!member_rows.is_empty(), |d| d.child(v_flex().gap_0().children(member_rows)))
            })
            .into_any_element()
    }

    fn custom_rule_row(
        &self,
        r: &CustomRule,
        idx: usize,
        total: usize,
        targets: &[String],
        in_group: bool,
        cx: &App,
    ) -> gpui_kit::AnyElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let line = r.line_index;
        let kind_text = r.kind.map(kind_label).unwrap_or(&r.kind_raw).to_string();
        let mono = matches!(
            r.kind,
            Some(CustomRuleKind::IpCidr | CustomRuleKind::SrcIpCidr | CustomRuleKind::DstPort | CustomRuleKind::DomainRegex | CustomRuleKind::ProcessPath)
        );
        let mut row_targets = targets.to_vec();
        if !r.target.is_empty() && !row_targets.iter().any(|x| x.eq_ignore_ascii_case(&r.target)) {
            row_targets.push(r.target.clone());
        }
        let target_idx = row_targets
            .iter()
            .position(|x| x.eq_ignore_ascii_case(&r.target))
            .unwrap_or(0);
        let target_opts: Vec<(SharedString, usize)> = row_targets
            .iter()
            .enumerate()
            .map(|(i, x)| (SharedString::from(target_label(x)), i))
            .collect();
        let (e_up, e_dn, e_tgt, e_en, e_del) = (
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
        );
        let rule_clone = r.clone();
        let value_for_tgt = r.value.clone();
        let kind_for_tgt = r.kind;
        let enabled_for_tgt = r.enabled;
        let drag_payload = DraggedRule {
            line_index: line,
            kind_label: kind_text.clone(),
            value: r.value.clone(),
        };
        let grip_dot_color = t.muted_foreground.opacity(0.55);
        let grip_col = || {
            v_flex()
                .gap(dp(2.5))
                .child(div().w(dp(2.5)).h(dp(2.5)).rounded_full().bg(grip_dot_color))
                .child(div().w(dp(2.5)).h(dp(2.5)).rounded_full().bg(grip_dot_color))
                .child(div().w(dp(2.5)).h(dp(2.5)).rounded_full().bg(grip_dot_color))
        };

        v_flex()
            .id(("cr-row", line))
            .py_1p5()
            .px_1()
            .gap_1()
            .rounded(t.radius)
            .cursor_grab()
            .hover(|d| d.bg(t.list_hover))
            .on_drag(drag_payload, |d, _, _, cx| cx.new(|_| DragRulePreview(d.clone())))
            .when(!r.enabled, |d| d.opacity(0.55))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        h_flex()
                            .id(("cr-grip", line))
                            .flex_none()
                            .px_1()
                            .py_1()
                            .gap(dp(2.5))
                            .items_center()
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(t!("rules.drag_hint")).build(window, cx)
                            })
                            .child(grip_col())
                            .child(grip_col()),
                    )
                    .child(
                        v_flex()
                            .flex_none()
                            .child(
                                Button::new(("cr-up", line))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronUp)
                                    .tooltip(t!("tip.move_up"))
                                    .disabled(idx == 0)
                                    .on_click(move |_, _, _| {
                                        let _ = e_up.move_custom_rule(line, -1);
                                    }),
                            )
                            .child(
                                Button::new(("cr-dn", line))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ChevronDown)
                                    .tooltip(t!("tip.move_down"))
                                    .disabled(idx + 1 == total)
                                    .on_click(move |_, _, _| {
                                        let _ = e_dn.move_custom_rule(line, 1);
                                    }),
                            ),
                    )
                    .child(pill(kind_text, t.muted_foreground, t.muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .when(mono, |d| d.font_family(t.mono_font_family.clone()))
                            .truncate()
                            .child(r.value.clone()),
                    )
                    .when(!in_group && r.kind.is_some(), |d| {
                        let row_targets = row_targets.clone();
                        d.child(choice(
                            SharedString::from(format!("cr-tgt-{line}")),
                            target_idx,
                            target_opts,
                            move |i, window, cx| {
                                let Some(k) = kind_for_tgt else { return };
                                let Some(new_target) = row_targets.get(i) else { return };
                                if let Err(e) =
                                    e_tgt.update_custom_rule(line, k, &value_for_tgt, new_target, enabled_for_tgt)
                                {
                                    window.push_notification(Notification::error(format!("{e:#}")), cx);
                                }
                            },
                        ))
                    })
                    .child(
                        Toggle::new(("cr-en", line))
                            .checked(r.enabled)
                            .tooltip(t!("tip.enabled"))
                            .on_change(move |v, _, _| {
                                let _ = e_en.set_custom_rule_enabled(line, *v);
                            }),
                    )
                    .child(
                        Button::new(("cr-edit", line))
                            .ghost()
                            .small()
                            .icon(extra_icon(Lucide::Pencil))
                            .tooltip(t!("tip.edit"))
                            .on_click(move |_, window, cx| open_custom_rule_editor(rule_clone.clone(), window, cx)),
                    )
                    .child(
                        Button::new(("cr-del", line))
                            .ghost()
                            .small()
                            .icon(extra_icon(Lucide::Trash))
                            .tooltip(t!("tip.delete"))
                            .on_click(move |_, _, _| {
                                let _ = e_del.remove_custom_rule(line);
                            }),
                    ),
            )
            .when_some(r.error.clone(), |d, err| {
                d.child(
                    h_flex()
                        .pl(dp(36.))
                        .gap_1p5()
                        .text_xs()
                        .text_color(t.danger)
                        .child(IconName::TriangleAlert)
                        .child(err),
                )
            })
            .into_any_element()
    }

    fn provider_row(
        &self,
        p: &RuleProvider,
        idx: usize,
        total: usize,
        this: WeakEntity<Self>,
        cx: &App,
    ) -> impl IntoElement {
        let selecting = self.sel.active;
        let selected = self.sel.ids.contains(&p.id);
        let pick_id = p.id.clone();
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let updating = engine.is_updating(&p.id);
        let own_active = p.own_group && p.can_own_group();

        // Friendly title & 1-line clean subtitle (no raw 100-char GitHub URL wall).
        let display_name = p
            .preset
            .as_deref()
            .and_then(service_label)
            .map(String::from)
            .unwrap_or_else(|| p.name.clone());
        let mut meta = Vec::new();
        if let Some(desc) = p.preset.as_deref().and_then(service_desc) {
            meta.push(desc.to_string());
        } else {
            meta.push(short_source(&p.source));
        }
        if p.rule_count > 0 {
            meta.push(t!("rules.count", p.rule_count));
        }
        meta.push(match p.last_update {
            Some(ts) => t!("meta.updated", fmt_ago(ts)),
            None => t!("subs.never_updated").to_string(),
        });

        // Inline target dropdown merges Proxy / Direct / Reject / Own node group / other groups.
        const OWN_SENTINEL: &str = "__own_group__";
        let mut target_values: Vec<String> = vec!["proxy".into(), "direct".into(), "reject".into()];
        if p.can_own_group() {
            target_values.push(OWN_SENTINEL.into());
        }
        for g in engine.groups() {
            let is_self = engine.rule_group(&g.name).is_some_and(|rp| rp.id == p.id);
            if !is_self
                && g.name != boxpilot_core::config::TAG_PROXY
                && g.name != boxpilot_core::config::TAG_AUTO
                && !target_values.contains(&g.name)
            {
                target_values.push(g.name.clone());
            }
        }
        if !own_active && !target_values.iter().any(|x| x.eq_ignore_ascii_case(&p.target)) {
            target_values.push(p.target.clone());
        }
        let current_target_idx = if own_active {
            target_values.iter().position(|x| x == OWN_SENTINEL).unwrap_or(0)
        } else {
            target_values
                .iter()
                .position(|x| x.eq_ignore_ascii_case(&p.target))
                .unwrap_or(0)
        };
        let target_opts: Vec<(SharedString, usize)> = target_values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = if v == OWN_SENTINEL {
                    t!("rules.own_group_option").to_string()
                } else {
                    target_label(v)
                };
                (SharedString::from(label), i)
            })
            .collect();

        let (e1, e2, e3, e4, e5, e_tgt, e_file) = (
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
            engine.clone(),
        );
        let (id1, id2, id3, id4, id5, id_tgt, id_file) = (
            p.id.clone(),
            p.id.clone(),
            p.id.clone(),
            p.id.clone(),
            p.id.clone(),
            p.id.clone(),
            p.id.clone(),
        );
        let prev_target = if p.can_own_group() { p.target.clone() } else { "proxy".to_string() };
        let text_file = engine.rule_provider_text_file(&p.id);
        let edit = p.clone();
        let name_del = display_name.clone();
        let group_tag = own_active.then(|| engine.rule_group_tag_for(&p.id).unwrap_or_else(|| p.name.clone()));
        let state_jump = self.state.clone();

        v_flex()
            .id(SharedString::from(format!("rp-row-{}", p.id)))
            .gap_1()
            .py_2()
            .rounded(t.radius)
            .when(!p.enabled, |d| d.opacity(0.55))
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
                    .gap_2p5()
                    .items_center()
                    .when(selecting, |d| {
                        d.child(
                            div()
                                .w(dp(28.))
                                .flex_none()
                                .flex()
                                .justify_center()
                                .child(Checkbox::new(("rp-pick", idx)).checked(selected)),
                        )
                    })
                    .when(!selecting, |d| {
                        d.child(
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
                        )
                    })
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
                                    .child(div().min_w_0().text_sm().font_weight(FontWeight::MEDIUM).truncate().child(display_name))
                                    .when(updating, |d| d.child(pill(t!("subs.updating"), t.info, t.info.opacity(0.12))))
                                    .when(p.last_error.is_some() && !updating, |d| {
                                        d.child(pill(t!("subs.update_failed"), t.danger, t.danger.opacity(0.12)))
                                    }),
                            )
                            .child(div().text_xs().text_color(t.muted_foreground).truncate().child(meta.join("  ·  "))),
                    )
                    .when(!selecting, |d| {
                        d.child(
                            h_flex()
                                .flex_none()
                                .gap_1p5()
                                .items_center()
                                .child(choice(
                                    SharedString::from(format!("rp-tgt-{}", p.id)),
                                    current_target_idx,
                                    target_opts,
                                    move |i, _, _| {
                                        let Some(v) = target_values.get(i) else { return };
                                        if v == OWN_SENTINEL {
                                            e_tgt.set_rule_provider_target(&id_tgt, prev_target.clone(), true);
                                        } else {
                                            e_tgt.set_rule_provider_target(&id_tgt, v.clone(), false);
                                        }
                                    },
                                ))
                                .when_some(group_tag.filter(|_| p.enabled), |d, gtag| {
                                    d.child(
                                        Button::new(("rp-jump", idx))
                                            .ghost()
                                            .xsmall()
                                            .label(t!("rules.goto_proxies"))
                                            .on_click(move |_, _, cx| {
                                                state_jump.update(cx, |s, cx| s.open_proxy_group(gtag.clone(), cx));
                                            }),
                                    )
                                })
                                .child(
                                    Toggle::new(("rp-en", idx))
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
                                    Button::new(("rp-file", idx))
                                        .ghost()
                                        .small()
                                        .icon(extra_icon(Lucide::FilePen))
                                        .tooltip(if text_file.is_some() {
                                            t!("tip.edit_file")
                                        } else {
                                            t!("tip.edit_file_binary")
                                        })
                                        .disabled(text_file.is_none())
                                        .on_click(move |_, window, cx| {
                                            let Some((path, local)) = e_file.rule_provider_text_file(&id_file) else {
                                                return;
                                            };
                                            crate::platform::open_in_text_editor(&path);
                                            e_file.watch_rule_provider_edits(&id_file, path, local);
                                            window.push_notification(
                                                if local { t!("rules.edit_local") } else { t!("rules.edit_cached") },
                                                cx,
                                            );
                                        }),
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
                                                t!("rules.delete_body", name_del),
                                                move |_| engine.remove_rule_provider(&id),
                                                window,
                                                cx,
                                            );
                                        }),
                                ),
                        )
                    }),
            )
            .when_some(p.last_error.clone().filter(|_| !updating), |d, err| {
                d.child(
                    h_flex()
                        .pl(dp(36.))
                        .gap_2()
                        .text_xs()
                        .text_color(t.danger)
                        .child(div().flex_none().child(IconName::TriangleAlert))
                        .child(div().flex_1().min_w_0().line_clamp(2).child(err)),
                )
            })
    }
}

fn step_header(step: &'static str, title: &str, hint: &str, count: usize, cx: &App) -> impl IntoElement {
    let t = cx.theme();
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .w(dp(20.))
                .h(dp(20.))
                .rounded_full()
                .bg(t.primary.opacity(0.14))
                .text_xs()
                .line_height(gpui_kit::relative(1.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.primary)
                .flex()
                .items_center()
                .justify_center()
                .child(step),
        )
        .child(section_title(title, cx))
        .when(count > 0, |d| {
            d.child(pill(count.to_string(), t.muted_foreground, t.muted))
        })
        .child(div().text_xs().text_color(t.muted_foreground).child(format!("·  {hint}")))
}

fn switch_row(
    id: &'static str,
    title: &str,
    desc: &str,
    checked: bool,
    set: fn(&mut Settings, bool),
    cx: &App,
) -> impl IntoElement {
    let engine = AppState::engine(cx);
    setting_row(
        title,
        Some(desc),
        Toggle::new(id).checked(checked).on_change(move |v, _, _| {
            let v = *v;
            engine.update_settings(move |s| set(s, v))
        }),
        cx,
    )
}

impl Render for RulesPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let s = engine.settings();
        if self.custom_seen != s.custom_rules {
            self.custom_seen = s.custom_rules.clone();
            let text = s.custom_rules.clone();
            self.custom_text.update(cx, |i, cx| i.set_value(text, window, cx));
        }

        let providers = engine.rule_providers();
        let t = cx.theme();
        let any_updating = providers.iter().any(|p| engine.is_updating(&p.id));
        let order: Vec<String> = providers.iter().map(|p| p.id.clone()).collect();
        self.sel.retain(&order);
        if providers.is_empty() {
            self.sel.active = false;
        }
        let this = cx.entity().downgrade();

        let actions = vec![
            Button::new("open-tester")
                .small()
                .icon(IconName::Search)
                .label(t!("btn.test_rules"))
                .on_click(|_, window, cx| open_rule_tester(window, cx))
                .into_any_element(),
            Button::new("open-library")
                .small()
                .icon(IconName::Star)
                .label(t!("btn.service_library"))
                .on_click(|_, window, cx| open_service_library(window, cx))
                .into_any_element(),
            Button::new("import-acl")
                .small()
                .icon(extra_icon(Lucide::Download))
                .label(t!("btn.import_acl"))
                .on_click(|_, window, cx| open_acl_import(window, cx))
                .into_any_element(),
            Button::new("add-ruleset")
                .small()
                .primary()
                .icon(IconName::Plus)
                .label(t!("btn.add_ruleset"))
                .on_click(|_, window, cx| open_rule_editor(None, window, cx))
                .into_any_element(),
        ];

        // Section ②: Routing rule-sets
        let ids = Rc::new(order.clone());
        let (e_on, e_off, e_all, ids_on, ids_off) = (engine.clone(), engine.clone(), engine.clone(), ids.clone(), ids);
        let all_on = providers.iter().all(|p| p.enabled);
        let all_off = providers.iter().all(|p| !p.enabled);
        let this_sel = this.clone();
        let bulk_bar = self.sel.active.then(|| {
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
        });
        let rulesets_card = card(cx)
            .gap_0()
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .flex_wrap()
                    .gap_2()
                    .pb_2()
                    .child(step_header("2", t!("rules.step_sets"), t!("rules.step_sets_badge"), providers.len(), cx))
                    .when_some(bulk_bar, |d, items| {
                        d.child(h_flex().gap_1().items_center().flex_wrap().children(items))
                    })
                    .when(!self.sel.active, |d| {
                        d.child(
                            h_flex()
                                .gap_1()
                                .flex_none()
                                .when(!providers.is_empty(), |d| {
                                    d.child(
                                        Button::new("rules-enable-all")
                                            .ghost()
                                            .small()
                                            .label(t!("btn.enable_all"))
                                            .disabled(all_on)
                                            .on_click(move |_, _, _| e_on.set_rule_providers_enabled(&ids_on, true)),
                                    )
                                    .child(
                                        Button::new("rules-disable-all")
                                            .ghost()
                                            .small()
                                            .label(t!("btn.disable_all"))
                                            .disabled(all_off)
                                            .on_click(move |_, _, _| e_off.set_rule_providers_enabled(&ids_off, false)),
                                    )
                                    .child(
                                        Button::new("rules-update-all")
                                            .ghost()
                                            .small()
                                            .icon(IconName::RefreshCw)
                                            .tooltip(t!("btn.update_all"))
                                            .loading(any_updating)
                                            .on_click(move |_, _, _| e_all.update_all_rule_providers()),
                                    )
                                    .child(
                                        Button::new("select-rules")
                                            .ghost()
                                            .small()
                                            .icon(extra_icon(Lucide::ListChecks))
                                            .tooltip(t!("btn.select"))
                                            .on_click(move |_, _, cx| {
                                                let _ = this_sel.update(cx, |page, cx| {
                                                    page.sel.active = true;
                                                    cx.notify();
                                                });
                                            }),
                                    )
                                }),
                        )
                    }),
            )
            .when(providers.is_empty(), |d| {
                let cta = Button::new("empty-open-lib")
                    .small()
                    .primary()
                    .icon(IconName::Star)
                    .label(t!("btn.service_library"))
                    .on_click(|_, window, cx| open_service_library(window, cx))
                    .into_any_element();
                d.child(empty_state(
                    extra_icon(Lucide::ListFilter),
                    t!("rules.empty_title"),
                    t!("rules.empty_hint"),
                    Some(cta),
                    cx,
                ))
            })
            .children(providers.iter().enumerate().map(|(i, p)| {
                v_flex()
                    .when(i > 0, |d| d.child(divider(cx)))
                    .child(self.provider_row(p, i, providers.len(), this.clone(), cx))
            }));

        // Section ③: Default fallback
        let e_final = engine.clone();
        let fallback_card = card(cx)
            .gap_1()
            .child(step_header("3", t!("rules.step_fallback"), t!("rules.step_fallback_badge"), 0, cx))
            .child(switch_row(
                "cn",
                t!("rules.bypass_cn"),
                t!("rules.bypass_cn_desc"),
                s.bypass_cn,
                |s, v| s.bypass_cn = v,
                cx,
            ))
            .child(switch_row(
                "ads",
                t!("rules.block_ads"),
                t!("rules.block_ads_desc"),
                s.block_ads,
                |s, v| s.block_ads = v,
                cx,
            ))
            .child(setting_row(
                t!("rules.final_outbound"),
                Some(t!("rules.final_outbound_desc")),
                choice(
                    "final-outbound",
                    s.final_direct,
                    vec![
                        (SharedString::from(t!("target.proxy")), false),
                        (SharedString::from(t!("target.direct")), true),
                    ],
                    move |direct, _, _| e_final.update_settings(move |s| s.final_direct = direct),
                ),
                cx,
            ));

        // Collapsible advanced rule sources
        let this_adv = this.clone();
        let adv_open = self.advanced_open;
        let advanced_card = card(cx)
            .gap_2()
            .child(
                h_flex()
                    .id("rules-adv-toggle")
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = this_adv.update(cx, |page, cx| {
                            page.advanced_open = !page.advanced_open;
                            cx.notify();
                        });
                    })
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(section_title(t!("rules.advanced"), cx))
                            .child(div().text_xs().text_color(t.muted_foreground).child(t!("rules.advanced_desc"))),
                    )
                    .child(
                        div()
                            .text_color(t.muted_foreground)
                            .child(if adv_open { IconName::ChevronDown } else { IconName::ChevronRight }),
                    ),
            )
            .when(adv_open, |d| {
                d.child(divider(cx))
                    .child(switch_row(
                        "rsproxy",
                        t!("rules.via_proxy"),
                        t!("rules.via_proxy_desc"),
                        s.ruleset_via_proxy,
                        |s, v| s.ruleset_via_proxy = v,
                        cx,
                    ))
                    .child(setting_row(
                        t!("rules.geosite_url"),
                        Some(t!("rules.template_desc")),
                        div().w(dp(340.)).child(blur_on_outside_click(&self.geosite, Input::new(&self.geosite).small(), cx)),
                        cx,
                    ))
                    .child(setting_row(
                        t!("rules.geoip_url"),
                        Some(t!("rules.template_desc")),
                        div().w(dp(340.)).child(blur_on_outside_click(&self.geoip, Input::new(&self.geoip).small(), cx)),
                        cx,
                    ))
            });

        let profiles_card = self.render_profiles(window, cx);
        let custom_card = self.render_custom_section(window, cx);

        page(t!("nav.rules"), Some(t!("rules.pipeline_subtitle").to_string()), actions, cx).child(
            v_flex()
                .gap_4()
                .child(profiles_card)
                .child(custom_card)
                .child(rulesets_card)
                .child(fallback_card)
                .child(advanced_card),
        )
    }
}

// ------------------------------------------------ dialogs

fn confirm_apply_profile(
    profile: RuleProfile,
    engine: std::sync::Arc<boxpilot_core::Engine>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = profile_title(profile);
    window.open_dialog(cx, move |dialog, _, cx| {
        let engine = engine.clone();
        dialog
            .title(t!("rules.profile_confirm_title", title))
            .w(dp(440.))
            .child(div().text_sm().text_color(cx.theme().muted_foreground).child(t!("rules.profile_confirm_body")))
            .footer(
                DialogFooter::new().child(super::cancel_button()).child(
                    Button::new("apply-profile-ok")
                        .primary()
                        .label(t!("btn.apply_rules"))
                        .on_click(move |_, window, cx| {
                            engine.apply_rule_profile(profile);
                            window.close_dialog(cx);
                            window.push_notification(Notification::success(t!("rules.profile_applied", title)), cx);
                        }),
                ),
            )
    });
}

struct RuleTesterView {
    state: Entity<AppState>,
    input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl RuleTesterView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(t!("rules.tester_placeholder")));
        let _subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&input, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self { state, input, _subs }
    }
}

impl Render for RuleTesterView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        sync_placeholder(&self.input, t!("rules.tester_placeholder"), window, cx);
        let engine = self.state.read(cx).engine.clone();
        let query = self.input.read(cx).value().to_string();
        let result = engine.test_rule_match(&query);
        let t = cx.theme();

        v_flex()
            .gap_3()
            .child(div().text_xs().text_color(t.muted_foreground).child(t!("rules.pipeline_subtitle")))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_none().text_color(t.muted_foreground).child(IconName::Search))
                    .child(div().flex_1().min_w_0().child(Input::new(&self.input).small().cleanable(true))),
            )
            .when_some(result, |d, m| {
                let (fg, bg) = target_colors(&m.target, cx);
                let layer_label = match &m.layer {
                    RuleMatchLayer::PrivateIp => t!("rules.match_private").to_string(),
                    RuleMatchLayer::CustomRule { line_index } => t!("rules.match_custom", line_index + 1),
                    RuleMatchLayer::CustomGroup { name, .. } => t!("rules.match_custom_group", name),
                    RuleMatchLayer::RuleSet { name, .. } => t!("rules.match_ruleset", outbound_label(name)),
                    RuleMatchLayer::BuiltIn { .. } => t!("rules.match_builtin").to_string(),
                    RuleMatchLayer::Fallback => t!("rules.match_fallback").to_string(),
                };
                d.child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .items_center()
                        .px_3()
                        .py_2p5()
                        .rounded(t.radius)
                        .border_1()
                        .border_color(hairline(cx))
                        .bg(t.muted.opacity(0.35))
                        .text_xs()
                        .child(pill(layer_label, t.foreground, t.muted))
                        .child(
                            div()
                                .font_family(t.mono_font_family.clone())
                                .text_color(t.muted_foreground)
                                .truncate()
                                .child(m.rule),
                        )
                        .child(div().text_color(t.muted_foreground).child("→"))
                        .child(pill(target_label(&m.target), fg, bg))
                        .when(m.approximate, |d| {
                            d.child(pill(t!("rules.match_approx"), t.muted_foreground, t.muted))
                        }),
                )
            })
    }
}

/// Auto-focused rule match tester dialog (no layout shift on the Rules page).
pub fn open_rule_tester(window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| RuleTesterView::new(window, cx));
    let dialog_w = (window.viewport_size().width.as_f32() / super::layout::scale() - 48.0).clamp(360.0, 560.0);
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = view.read(cx).input.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        dialog
            .title(t!("btn.test_rules"))
            .w(dp(dialog_w))
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("tester-done")
                        .primary()
                        .label(t!("btn.done"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

struct ServiceLibraryView {
    state: Entity<AppState>,
    scroll: ScrollHandle,
    _sub: Subscription,
}

impl ServiceLibraryView {
    fn new(cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let _sub = cx.observe(&state, |_, _, cx| cx.notify());
        Self { state, scroll: ScrollHandle::new(), _sub }
    }
}

impl Render for ServiceLibraryView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let t = cx.theme();
        let mut sections = v_flex().gap_3();
        for cat in ServiceCategory::ALL {
            let items: Vec<&ServicePreset> = SERVICE_PRESETS.iter().filter(|p| p.category == cat).collect();
            if items.is_empty() {
                continue;
            }
            let mut list = v_flex().gap_1p5();
            for sp in items {
                let added = engine.has_service_preset(sp.id);
                let e = engine.clone();
                let id = sp.id;
                let name = service_label(sp.id).unwrap_or(sp.name);
                let desc = service_desc(sp.id).unwrap_or("");
                let (fg, bg) = target_colors(sp.default_target, cx);
                list = list.child(
                    h_flex()
                        .gap_3()
                        .px_3()
                        .py_2()
                        .rounded(t.radius)
                        .border_1()
                        .border_color(if added { t.primary.opacity(0.45) } else { hairline(cx) })
                        .bg(if added { t.primary.opacity(0.06) } else { t.muted.opacity(0.35) })
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .items_center()
                                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(name))
                                        .child(pill(target_label(sp.default_target), fg, bg)),
                                )
                                .child(div().text_xs().text_color(t.muted_foreground).truncate().child(desc)),
                        )
                        .child(if added {
                            Button::new(SharedString::from(format!("svc-rm-{id}")))
                                .small()
                                .icon(IconName::Check)
                                .label(t!("rules.added_tag"))
                                .on_click(move |_, window, _| {
                                    e.remove_service_preset(id);
                                    window.refresh();
                                })
                        } else {
                            Button::new(SharedString::from(format!("svc-add-{id}")))
                                .small()
                                .primary()
                                .icon(IconName::Plus)
                                .label(t!("btn.add"))
                                .on_click(move |_, window, _| {
                                    e.add_service_preset(id);
                                    window.refresh();
                                })
                        }),
                );
            }
            sections = sections.child(
                v_flex()
                    .gap_1p5()
                    .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(t.muted_foreground).child(category_label(cat)))
                    .child(list),
            );
        }

        let avail_h = (window.viewport_size().height.as_f32() / super::layout::scale() - 250.0).clamp(220.0, 460.0);
        let list_scroll = super::smooth_scroll::with_scrollbar(
            "service-lib-scrollbar",
            &self.scroll,
            super::smooth_scroll::smooth_scroll(
                "service-lib-smooth",
                &self.scroll,
                div()
                    .id("service-lib-list")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pr_3()
                    .pb_2()
                    .child(sections),
            ),
        )
        .w_full()
        .h(dp(avail_h));

        v_flex()
            .gap_3()
            .child(div().text_sm().text_color(t.muted_foreground).child(t!("rules.library_desc")))
            .child(list_scroll)
    }
}

/// Curated service library dialog: one-click add/remove popular services with zero URLs.
pub fn open_service_library(window: &mut Window, cx: &mut App) {
    let view = cx.new(ServiceLibraryView::new);
    let dialog_w = (window.viewport_size().width.as_f32() / super::layout::scale() - 48.0).clamp(380.0, 600.0);
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .title(t!("rules.library_title"))
            .w(dp(dialog_w))
            .child(view.clone())
            .footer(
                DialogFooter::new().child(
                    Button::new("lib-done")
                        .primary()
                        .label(t!("btn.done"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}

/// Modal editor for creating or renaming a custom rule group.
struct CustomGroupEditor {
    existing: Option<CustomRuleGroup>,
    name: Entity<InputState>,
    /// Index into `target_values` (only shown when creating a new group).
    target_idx: usize,
    target_values: Vec<String>,
    error: Option<String>,
    focus: FocusHandle,
}

impl CustomGroupEditor {
    const OWN_SENTINEL: &'static str = "__own_group__";

    fn new(existing: Option<CustomRuleGroup>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let initial_name = existing.as_ref().map(|g| g.name.clone()).unwrap_or_default();
        let target_values = vec![
            Self::OWN_SENTINEL.to_string(),
            "proxy".to_string(),
            "direct".to_string(),
            "reject".to_string(),
        ];
        Self {
            existing,
            name: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(t!("rules.group_name_placeholder"))
                    .default_value(initial_name)
            }),
            target_idx: 0, // Default to Own Node Group (shown on Proxies page)
            target_values,
            error: None,
            focus: cx.focus_handle(),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let engine = AppState::engine(cx);
        let name = self.name.read(cx).value().trim().to_string();
        let res = if let Some(g) = &self.existing {
            engine.rename_custom_rule_group(&g.id, name)
        } else {
            let v = self.target_values.get(self.target_idx).map(String::as_str).unwrap_or(Self::OWN_SENTINEL);
            let (target, own_group) = if v == Self::OWN_SENTINEL {
                ("proxy".to_string(), true)
            } else {
                (v.to_string(), false)
            };
            engine.create_custom_rule_group(name, target, own_group).map(|_| ())
        };
        match res {
            Ok(()) => true,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
                false
            }
        }
    }
}

impl Focusable for CustomGroupEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CustomGroupEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.theme();
        let weak = cx.entity().downgrade();
        let creating = self.existing.is_none();
        let target_opts: Vec<(SharedString, usize)> = self
            .target_values
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let label = if v == Self::OWN_SENTINEL {
                    t!("rules.own_group_option").to_string()
                } else {
                    target_label(v)
                };
                (SharedString::from(label), i)
            })
            .collect();

        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .child(field(
                t!("rules.group_name"),
                Some(t!("rules.group_name_hint")),
                Input::new(&self.name),
                cx,
            ))
            .when(creating, |d| {
                d.child(field(
                    t!("rules.group_target"),
                    Some(t!("rules.group_target_hint")),
                    choice("cg-new-target", self.target_idx, target_opts, move |i, _, cx| {
                        let _ = weak.update(cx, |e, cx| {
                            e.target_idx = i;
                            cx.notify();
                        });
                    }),
                    cx,
                ))
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .p_2()
                        .rounded(t.radius)
                        .bg(t.danger.opacity(0.1))
                        .text_sm()
                        .text_color(t.danger)
                        .child(IconName::CircleX)
                        .child(div().flex_1().min_w_0().child(e)),
                )
            })
    }
}

fn open_custom_group_editor(existing: Option<CustomRuleGroup>, window: &mut Window, cx: &mut App) {
    let renaming = existing.is_some();
    let editor = cx.new(|cx| CustomGroupEditor::new(existing, window, cx));
    let first = Rc::new(Cell::new(true));
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = editor.read(cx).name.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        let ok = editor.clone();
        dialog
            .title(if renaming { t!("rules.rename_group") } else { t!("rules.new_group") })
            .w(dp(460.))
            .child(editor.clone())
            .footer(
                DialogFooter::new().child(super::cancel_button()).child(
                    Button::new("save-custom-group")
                        .primary()
                        .label(if renaming { t!("btn.save") } else { t!("btn.add") })
                        .on_click(move |_, window, cx| {
                            if ok.update(cx, |e, cx| e.save(cx)) {
                                window.close_dialog(cx);
                            }
                        }),
                ),
            )
    });
}

/// Modal editor for a single custom rule (exposes all 13 rule types + app picker).
struct CustomRuleEditor {
    line_index: usize,
    enabled: bool,
    in_group: Option<String>,
    kind: CustomRuleKind,
    value: Entity<InputState>,
    targets: Vec<String>,
    target: usize,
    error: Option<String>,
    focus: FocusHandle,
}

impl CustomRuleEditor {
    fn new(rule: CustomRule, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = AppState::engine(cx);
        let kind = rule.kind.unwrap_or(CustomRuleKind::DomainSuffix);
        let mut targets = collect_targets(&engine);
        if !rule.target.is_empty() && !targets.iter().any(|x| x.eq_ignore_ascii_case(&rule.target)) {
            targets.push(rule.target.clone());
        }
        let target = targets.iter().position(|x| x.eq_ignore_ascii_case(&rule.target)).unwrap_or(0);
        Self {
            line_index: rule.line_index,
            enabled: rule.enabled,
            in_group: rule.group,
            kind,
            value: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(quick_placeholder(kind))
                    .default_value(rule.value)
            }),
            targets,
            target,
            error: rule.error,
            focus: cx.focus_handle(),
        }
    }

    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let engine = AppState::engine(cx);
        let val = self.value.read(cx).value().to_string();
        let target = &self.targets[self.target];
        match engine.update_custom_rule(self.line_index, self.kind, &val, target, self.enabled) {
            Ok(()) => true,
            Err(e) => {
                self.error = Some(format!("{e:#}"));
                cx.notify();
                false
            }
        }
    }

    fn pick_app(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let kind = self.kind;
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx| {
            let Ok(Ok(Some(paths))) = rx.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let text = path.display().to_string();
            let value = if kind == CustomRuleKind::ProcessName {
                boxpilot_core::rules::normalize_custom_value(CustomRuleKind::ProcessName, &text)
            } else {
                text
            };
            let _ = this.update_in(cx, |e, window, cx| {
                e.value.update(cx, |i, cx| i.set_value(value, window, cx));
            });
        })
        .detach();
    }
}

impl Focusable for CustomRuleEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for CustomRuleEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        sync_placeholder(&self.value, quick_placeholder(self.kind), window, cx);
        let t = cx.theme();
        let weak = cx.entity().downgrade();
        let (w1, w2, w3) = (weak.clone(), weak.clone(), weak);
        let kinds: Vec<(SharedString, CustomRuleKind)> = CustomRuleKind::ALL
            .into_iter()
            .map(|k| (SharedString::from(kind_label(k)), k))
            .collect();
        let targets: Vec<(SharedString, usize)> = self
            .targets
            .iter()
            .enumerate()
            .map(|(i, x)| (SharedString::from(target_label(x)), i))
            .collect();
        let show_pick = matches!(self.kind, CustomRuleKind::ProcessName | CustomRuleKind::ProcessPath);

        v_flex()
            .gap_3()
            .track_focus(&self.focus)
            .child(
                h_flex()
                    .gap_3()
                    .items_start()
                    .child(div().flex_1().min_w_0().child(field(
                        t!("rules.custom_type"),
                        None,
                        choice("cr-edit-kind", self.kind, kinds, move |k, _, cx| {
                            let _ = w1.update(cx, |e, cx| {
                                e.kind = k;
                                e.error = None;
                                cx.notify();
                            });
                        }),
                        cx,
                    )))
                    .when(self.in_group.is_none(), |d| {
                        d.child(div().flex_1().min_w_0().child(field(
                            t!("rules.target"),
                            None,
                            choice("cr-edit-target", self.target, targets, move |i, _, cx| {
                                let _ = w2.update(cx, |e, cx| {
                                    e.target = i;
                                    cx.notify();
                                });
                            }),
                            cx,
                        )))
                    }),
            )
            .child(field(
                t!("rules.custom_value"),
                None,
                h_flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.value)))
                    .when(show_pick, |d| {
                        d.child(
                            Button::new("cr-edit-pick")
                                .icon(extra_icon(Lucide::FolderOpen))
                                .label(t!("rules.pick_app"))
                                .on_click(move |_, window, cx| {
                                    let _ = w3.update(cx, |e, cx| e.pick_app(window, cx));
                                }),
                        )
                    }),
                cx,
            ))
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .p_2()
                        .rounded(t.radius)
                        .bg(t.danger.opacity(0.1))
                        .text_sm()
                        .text_color(t.danger)
                        .child(IconName::CircleX)
                        .child(div().flex_1().min_w_0().child(e)),
                )
            })
    }
}

fn open_custom_rule_editor(rule: CustomRule, window: &mut Window, cx: &mut App) {
    let editor = cx.new(|cx| CustomRuleEditor::new(rule, window, cx));
    let first = Rc::new(Cell::new(true));
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            let focus = editor.read(cx).value.read(cx).focus_handle(cx);
            focus.focus(window, cx);
        }
        let ok = editor.clone();
        dialog
            .title(t!("rules.edit_custom_title"))
            .w(dp(540.))
            .child(editor.clone())
            .footer(
                DialogFooter::new().child(super::cancel_button()).child(
                    Button::new("save-custom-rule")
                        .primary()
                        .label(t!("btn.save"))
                        .on_click(move |_, window, cx| {
                            if ok.update(cx, |e, cx| e.save(cx)) {
                                window.close_dialog(cx);
                            }
                        }),
                ),
            )
    });
}

/// Form for adding or editing one external rule-set.
pub struct RuleEditor {
    original: Option<RuleProvider>,
    name: Entity<InputState>,
    source: Entity<InputState>,
    interval: Entity<InputState>,
    format: RuleFormat,
    targets: Vec<String>,
    target: usize,
    own_group: bool,
    error: Option<String>,
    focus: FocusHandle,
}

impl RuleEditor {
    fn new(original: Option<RuleProvider>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let engine = AppState::engine(cx);
        let base = original.clone().unwrap_or_default();
        let mut targets = collect_targets(&engine);
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
            interval: cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("24")
                    .default_value(base.interval_hours.to_string())
            }),
            format: base.format,
            targets,
            target,
            own_group: base.own_group,
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
        p.own_group = self.own_group && p.can_own_group();
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
        let (w1, w2, w3, w4) = (weak.clone(), weak.clone(), weak.clone(), weak);
        let formats: Vec<(SharedString, RuleFormat)> = RuleFormat::ALL
            .iter()
            .map(|f| {
                let label: SharedString =
                    if *f == RuleFormat::Auto { t!("rules.format_auto").into() } else { f.label().into() };
                (label, *f)
            })
            .collect();
        let targets: Vec<(SharedString, usize)> = self
            .targets
            .iter()
            .enumerate()
            .map(|(i, t)| (SharedString::from(target_label(t)), i))
            .collect();
        let can_group = !matches!(self.targets[self.target].to_ascii_lowercase().as_str(), "reject" | "block");
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
                    .child(Button::new("browse").label(t!("btn.browse")).on_click(move |_, window, cx| {
                        let _ = w1.update(cx, |e, cx| e.browse(window, cx));
                    })),
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
                    .child(
                        div()
                            .w(dp(150.))
                            .flex_none()
                            .child(field(t!("editor.interval"), None, Input::new(&self.interval), cx)),
                    ),
            )
            .when(can_group, |d| {
                d.child(setting_row(
                    t!("rules.own_group"),
                    None,
                    Toggle::new("edit-own-group").checked(self.own_group).on_change(move |v, _, cx| {
                        let v = *v;
                        let _ = w4.update(cx, |e, cx| {
                            e.own_group = v;
                            cx.notify();
                        });
                    }),
                    cx,
                ))
            })
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    h_flex()
                        .gap_2()
                        .items_center()
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
                DialogFooter::new().child(super::cancel_button()).child(
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
                DialogFooter::new().child(super::cancel_button()).child(
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
                                    Err(e) => {
                                        window.push_notification(Notification::error(format!("{e:#}")), cx);
                                    }
                                });
                            })
                            .detach();
                        }),
                ),
            )
    });
}

// ------------------------------------------------ helpers & labels

fn collect_targets(engine: &boxpilot_core::Engine) -> Vec<String> {
    let mut targets = vec!["proxy".to_string(), "direct".to_string(), "reject".to_string()];
    for g in engine.groups() {
        if g.name != boxpilot_core::config::TAG_PROXY
            && g.name != boxpilot_core::config::TAG_AUTO
            && !targets.contains(&g.name)
        {
            targets.push(g.name.clone());
        }
    }
    // Include own-group custom rule groups & rule-set names even while core is stopped.
    for g in engine.custom_rule_groups() {
        if g.enabled && g.own_group && g.can_own_group() && !g.name.trim().is_empty() && !targets.contains(&g.name) {
            targets.push(g.name);
        }
    }
    for p in engine.rule_providers() {
        if p.enabled && p.own_group && p.can_own_group() && !p.name.trim().is_empty() && !targets.contains(&p.name) {
            targets.push(p.name);
        }
    }
    targets
}

fn target_colors(target: &str, cx: &App) -> (gpui_kit::Hsla, gpui_kit::Hsla) {
    let t = cx.theme();
    match target.to_ascii_lowercase().as_str() {
        "direct" => (t.success, t.success.opacity(0.12)),
        "reject" | "block" => (t.danger, t.danger.opacity(0.12)),
        _ => (t.primary, t.primary.opacity(0.12)),
    }
}

fn short_source(source: &str) -> String {
    let s = source.trim();
    if let Some(rest) = s.strip_prefix("https://").or_else(|| s.strip_prefix("http://")) {
        let host = rest.split('/').next().unwrap_or(rest);
        let file = rest.rsplit('/').next().unwrap_or("").split('?').next().unwrap_or("");
        if !file.is_empty() && file != host {
            format!("{host} · {file}")
        } else {
            host.to_string()
        }
    } else {
        s.rsplit(['/', '\\']).next().unwrap_or(s).to_string()
    }
}

fn profile_title(p: RuleProfile) -> &'static str {
    match p {
        RuleProfile::Daily => t!("profile.daily"),
        RuleProfile::MinimalBypassCn => t!("profile.minimal"),
        RuleProfile::StreamingGaming => t!("profile.streaming"),
        RuleProfile::DirectFirst => t!("profile.direct_first"),
    }
}

fn profile_desc(p: RuleProfile) -> &'static str {
    match p {
        RuleProfile::Daily => t!("profile.daily_desc"),
        RuleProfile::MinimalBypassCn => t!("profile.minimal_desc"),
        RuleProfile::StreamingGaming => t!("profile.streaming_desc"),
        RuleProfile::DirectFirst => t!("profile.direct_first_desc"),
    }
}

fn kind_label(k: CustomRuleKind) -> &'static str {
    match k {
        CustomRuleKind::DomainSuffix => t!("kind.domain_suffix"),
        CustomRuleKind::Domain => t!("kind.domain"),
        CustomRuleKind::DomainKeyword => t!("kind.domain_keyword"),
        CustomRuleKind::ProcessName => t!("kind.process_name"),
        CustomRuleKind::IpCidr => t!("kind.ip_cidr"),
        CustomRuleKind::DstPort => t!("kind.dst_port"),
        CustomRuleKind::GeoSite => t!("kind.geosite"),
        CustomRuleKind::GeoIp => t!("kind.geoip"),
        CustomRuleKind::RuleSet => t!("kind.rule_set"),
        CustomRuleKind::DomainRegex => t!("kind.domain_regex"),
        CustomRuleKind::ProcessPath => t!("kind.process_path"),
        CustomRuleKind::SrcIpCidr => t!("kind.src_ip_cidr"),
        CustomRuleKind::Network => t!("kind.network"),
    }
}

fn quick_placeholder(k: CustomRuleKind) -> &'static str {
    match k {
        CustomRuleKind::DomainSuffix | CustomRuleKind::Domain => t!("rules.quick_placeholder_domain"),
        CustomRuleKind::DomainKeyword => t!("rules.quick_placeholder_keyword"),
        CustomRuleKind::ProcessName | CustomRuleKind::ProcessPath => t!("rules.quick_placeholder_process"),
        CustomRuleKind::IpCidr | CustomRuleKind::SrcIpCidr => t!("rules.quick_placeholder_ip"),
        CustomRuleKind::DstPort => t!("rules.quick_placeholder_port"),
        CustomRuleKind::GeoSite | CustomRuleKind::GeoIp => t!("rules.quick_placeholder_geo"),
        CustomRuleKind::DomainRegex | CustomRuleKind::Network | CustomRuleKind::RuleSet => t!("rules.quick_placeholder_other"),
    }
}

fn category_label(c: ServiceCategory) -> &'static str {
    match c {
        ServiceCategory::AiDev => t!("cat.ai_dev"),
        ServiceCategory::Streaming => t!("cat.streaming"),
        ServiceCategory::SocialGaming => t!("cat.social_gaming"),
        ServiceCategory::DomesticPlatforms => t!("cat.domestic"),
        ServiceCategory::General => t!("cat.general"),
    }
}

fn service_label(id: &str) -> Option<&'static str> {
    Some(match id {
        "ai" => t!("svc.ai"),
        "github" => t!("svc.github"),
        "google" => t!("svc.google"),
        "youtube" => t!("svc.youtube"),
        "netflix" => t!("svc.netflix"),
        "disney" => t!("svc.disney"),
        "spotify" => t!("svc.spotify"),
        "tiktok" => t!("svc.tiktok"),
        "telegram" => t!("svc.telegram"),
        "steam" => t!("svc.steam"),
        "bilibili" => t!("svc.bilibili"),
        "apple" => t!("svc.apple"),
        "microsoft" => t!("svc.microsoft"),
        "overseas" => t!("svc.overseas"),
        _ => return None,
    })
}

fn service_desc(id: &str) -> Option<&'static str> {
    Some(match id {
        "ai" => t!("svc.ai_desc"),
        "github" => t!("svc.github_desc"),
        "google" => t!("svc.google_desc"),
        "youtube" => t!("svc.youtube_desc"),
        "netflix" => t!("svc.netflix_desc"),
        "disney" => t!("svc.disney_desc"),
        "spotify" => t!("svc.spotify_desc"),
        "tiktok" => t!("svc.tiktok_desc"),
        "telegram" => t!("svc.telegram_desc"),
        "steam" => t!("svc.steam_desc"),
        "bilibili" => t!("svc.bilibili_desc"),
        "apple" => t!("svc.apple_desc"),
        "microsoft" => t!("svc.microsoft_desc"),
        "overseas" => t!("svc.overseas_desc"),
        _ => return None,
    })
}
