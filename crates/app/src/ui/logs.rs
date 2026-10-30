use std::rc::Rc;
use std::sync::Arc;

use boxpilot_core::process::{LogLevel, LogLine};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex, v_flex};
use gpui_kit::component::Selectable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    ClipboardItem, Context, Entity, FontWeight, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window,
    div, uniform_list,
};

use super::widgets::*;
use crate::state::AppState;
use crate::t;
use super::layout::dp;

pub struct LogsPage {
    state: Entity<AppState>,
    search: Entity<InputState>,
    level: LogLevel,
    follow: bool,
    lines: Rc<Vec<LogLine>>,
    built_for: (u64, String, LogLevel),
    scroll: UniformListScrollHandle,
    /// The line shown in full under the list (rows are one line high).
    selected: Option<LogLine>,
    _subs: Vec<Subscription>,
}

fn same_line(a: &LogLine, b: &LogLine) -> bool {
    a.time == b.time && Arc::ptr_eq(&a.message, &b.message)
}

fn copy_line(l: &LogLine, window: &mut Window, cx: &mut gpui_kit::App) {
    cx.write_to_clipboard(ClipboardItem::new_string(format!("{} {} {}", l.clock(), l.level.label(), l.message)));
    window.push_notification(t!("logs.copied"), cx);
}

impl LogsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("logs.search")));
        let _subs = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe(&search, |_, _, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            state,
            search,
            level: LogLevel::Info,
            follow: true,
            lines: Rc::new(vec![]),
            built_for: (u64::MAX, String::new(), LogLevel::Info),
            scroll: UniformListScrollHandle::new(),
            selected: None,
            _subs,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let rev = self.state.read(cx).log_rev;
        let filter = self.search.read(cx).value().to_lowercase();
        let key = (rev, filter.clone(), self.level);
        if key == self.built_for {
            return;
        }
        let lines: Vec<LogLine> = self
            .state
            .read(cx)
            .engine
            .logs()
            .into_iter()
            .filter(|l| l.level >= self.level)
            .filter(|l| filter.is_empty() || l.message.to_lowercase().contains(&filter))
            .collect();
        let grew = lines.len() != self.lines.len() || key.0 != self.built_for.0;
        self.lines = Rc::new(lines);
        self.built_for = key;
        if self.follow && grew && !self.lines.is_empty() {
            self.scroll.scroll_to_item(self.lines.len() - 1, ScrollStrategy::Bottom);
        }
    }
}

impl Render for LogsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        sync_placeholder(&self.search, t!("logs.search"), window, cx);
        self.refresh(cx);
        let t = cx.theme();
        let level = self.level;
        let this = cx.entity().downgrade();
        let (this2, this4) = (this.clone(), this.clone());
        let engine = self.state.read(cx).engine.clone();
        let data_dir = engine.paths().root.clone();
        let recording = engine.settings().record_logs;
        let e_rec = engine.clone();
        let actions = vec![
            div().w(dp(220.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
            choice(
                "log-level",
                level,
                LogLevel::ALL.iter().map(|l| (SharedString::from(format!("≥ {}", l.label())), *l)).collect(),
                move |l, _, cx| {
                    let _ = this.update(cx, |s, cx| {
                        s.level = l;
                        cx.notify();
                    });
                },
            )
            .into_any_element(),
            Button::new("follow")
                .small()
                .icon(IconName::ArrowDown)
                .label(t!("btn.follow"))
                .selected(self.follow)
                .on_click(move |_, _, cx| {
                    let _ = this2.update(cx, |s, cx| {
                        s.follow = !s.follow;
                        cx.notify();
                    });
                })
                .into_any_element(),
            // Stopping also freezes the list, which makes long output readable.
            Button::new("record")
                .small()
                .icon(extra_icon(if recording { Lucide::CircleStop } else { Lucide::CirclePlay }))
                .label(if recording { t!("btn.record_stop") } else { t!("btn.record_start") })
                .tooltip(t!("tip.record"))
                .on_click(move |_, _, _| e_rec.update_settings(move |s| s.record_logs = !recording))
                .into_any_element(),
            Button::new("clear-logs")
                .small()
                .ghost()
                .icon(extra_icon(Lucide::Trash))
                .tooltip(t!("tip.clear"))
                .on_click(move |_, _, cx| {
                    engine.clear_logs();
                    let _ = this4.update(cx, |s, cx| {
                        s.built_for.0 = u64::MAX;
                        cx.notify();
                    });
                })
                .into_any_element(),
            Button::new("open-logs")
                .small()
                .ghost()
                .icon(extra_icon(Lucide::FolderOpen))
                .tooltip(t!("tip.open_folder"))
                .on_click(move |_, _, _| crate::platform::open_path(&data_dir))
                .into_any_element(),
        ];

        let lines = self.lines.clone();
        let count = lines.len();
        let (muted, fg, border) = (t.muted_foreground, t.foreground, t.border);
        let colors = [t.muted_foreground, t.muted_foreground, t.info, t.warning, t.danger];
        let mono = t.mono_font_family.clone();
        let detail_mono = mono.clone();
        let base = self.scroll.0.borrow().base_handle.clone();
        let selected = self.selected.clone();
        let (hover_bg, picked_bg) = (t.muted, t.primary.opacity(0.1));
        let this_pick = cx.entity().downgrade();
        let list = uniform_list("log-list", count, move |range, _, _| {
            range
                .map(|i| {
                    let l = &lines[i];
                    let c = colors[l.level as usize];
                    let picked = selected.as_ref().is_some_and(|s| same_line(s, l));
                    let (this, line) = (this_pick.clone(), l.clone());
                    h_flex()
                        .id(("log-line", i))
                        .w_full()
                        .h(dp(24.))
                        .px_3()
                        .gap_3()
                        .items_center()
                        .text_xs()
                        .font_family(mono.clone())
                        // Click shows the whole line below; again hides it.
                        .cursor_pointer()
                        .hover(move |d| d.bg(hover_bg))
                        .when(picked, move |d| d.bg(picked_bg))
                        .on_click(move |_, _, cx| {
                            let line = line.clone();
                            let _ = this.update(cx, |s, cx| {
                                let again = s.selected.as_ref().is_some_and(|x| same_line(x, &line));
                                s.selected = (!again).then_some(line);
                                cx.notify();
                            });
                        })
                        .child(div().w(dp(60.)).flex_none().text_color(muted).child(l.clock()))
                        .child(
                            div()
                                .w(dp(44.))
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(c)
                                .child(l.level.label()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(if l.level >= LogLevel::Warn { c } else { fg })
                                .child(SharedString::from(l.message.clone())),
                        )
                })
                .collect()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        // The selected line in full, wrapped, with copy and close.
        let detail = self.selected.clone().map(|l| {
            let c = colors[l.level as usize];
            let (copy, this_close) = (l.clone(), cx.entity().downgrade());
            v_flex()
                .flex_none()
                .gap_1()
                .px_3()
                .py_2()
                .border_t_1()
                .border_color(border)
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .text_xs()
                        .font_family(detail_mono.clone())
                        .child(div().text_color(muted).child(l.clock()))
                        .child(div().font_weight(FontWeight::SEMIBOLD).text_color(c).child(l.level.label()))
                        .child(div().flex_1())
                        .child(
                            Button::new("log-copy")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .label(t!("btn.copy"))
                                .on_click(move |_, window, cx| copy_line(&copy, window, cx)),
                        )
                        .child(
                            Button::new("log-close")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip(t!("btn.close"))
                                .on_click(move |_, _, cx| {
                                    let _ = this_close.update(cx, |s, cx| {
                                        s.selected = None;
                                        cx.notify();
                                    });
                                }),
                        ),
                )
                .child(
                    div()
                        .id("log-detail")
                        .max_h(dp(160.))
                        .overflow_y_scroll()
                        .text_xs()
                        .font_family(detail_mono.clone())
                        .text_color(if l.level >= LogLevel::Warn { c } else { fg })
                        .child(SharedString::from(l.message.to_string())),
                )
        });

        page(
            t!("nav.logs"),
            Some(if !recording {
                format!("{} · {}", t!("logs.subtitle", count), t!("logs.stopped"))
            } else {
                t!("logs.subtitle", count)
            }),
            actions,
            cx,
        )
        .flex_1()
        .min_h_0()
        .child(
            card(cx)
                .p_0()
                .py_1()
                .gap_0()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .border_color(border)
                .when(count == 0, |d| {
                    d.child(empty_state(extra_icon(Lucide::ScrollText), t!("logs.empty"), t!("logs.empty_hint"), None, cx))
                })
                .when(count > 0, |d| {
                    d.child(
                        super::smooth_scroll::with_scrollbar(
                            "log-scrollbar",
                            &self.scroll,
                            super::smooth_scroll::smooth_scroll("log-smooth", &base, list),
                        )
                        .flex_1()
                        .min_h_0(),
                    )
                })
                .when_some(detail, |d, detail| d.child(detail)),
        )
    }
}
