use std::rc::Rc;

use boxpilot_core::process::{LogLevel, LogLine};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::component::Selectable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    Context, Entity, FontWeight, ScrollStrategy, SharedString, Subscription, UniformListScrollHandle, Window, div,
    px, uniform_list,
};

use super::widgets::*;
use crate::state::AppState;
use crate::t;

pub struct LogsPage {
    state: Entity<AppState>,
    search: Entity<InputState>,
    level: LogLevel,
    paused: bool,
    follow: bool,
    lines: Rc<Vec<LogLine>>,
    built_for: (u64, String, LogLevel),
    scroll: UniformListScrollHandle,
    _subs: Vec<Subscription>,
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
            paused: false,
            follow: true,
            lines: Rc::new(vec![]),
            built_for: (u64::MAX, String::new(), LogLevel::Info),
            scroll: UniformListScrollHandle::new(),
            _subs,
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let rev = self.state.read(cx).log_rev;
        let filter = self.search.read(cx).value().to_lowercase();
        let key = (rev, filter.clone(), self.level);
        if self.paused && self.built_for.1 == key.1 && self.built_for.2 == key.2 {
            return;
        }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh(cx);
        let t = cx.theme();
        let level = self.level;
        let this = cx.entity().downgrade();
        let (this2, this3, this4) = (this.clone(), this.clone(), this.clone());
        let engine = self.state.read(cx).engine.clone();
        let data_dir = engine.paths().root.clone();
        let actions = vec![
            div().w(px(220.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
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
            Button::new("pause")
                .small()
                .icon(if self.paused { IconName::Play } else { IconName::Pause })
                .label(if self.paused { t!("btn.resume") } else { t!("btn.pause") })
                .on_click(move |_, _, cx| {
                    let _ = this3.update(cx, |s, cx| {
                        s.paused = !s.paused;
                        cx.notify();
                    });
                })
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
        let list = uniform_list("log-list", count, move |range, _, _| {
            range
                .map(|i| {
                    let l = &lines[i];
                    let c = colors[l.level as usize];
                    h_flex()
                        .h(px(24.))
                        .px_3()
                        .gap_3()
                        .items_center()
                        .text_xs()
                        .font_family(mono.clone())
                        .child(div().w(px(60.)).flex_none().text_color(muted).child(l.time.clone()))
                        .child(
                            div()
                                .w(px(44.))
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
                                .child(l.message.clone()),
                        )
                })
                .collect()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        page(
            t!("nav.logs"),
            Some(if self.paused {
                format!("{} · {}", t!("logs.subtitle", count), t!("logs.paused"))
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
                .when(count > 0, |d| d.child(list)),
        )
    }
}
