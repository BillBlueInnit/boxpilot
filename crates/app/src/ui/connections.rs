use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use boxpilot_core::Engine;
use boxpilot_core::clash_api::{Connection, ConnectionsSnapshot};
use boxpilot_core::rules::base_domain;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, WindowExt as _, h_flex};
use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    App, AsyncApp, ClipboardItem, Context, CursorStyle, DispatchPhase, Entity, FontWeight, MouseButton, MouseMoveEvent,
    MouseUpEvent, ScrollWheelEvent, SharedString, Subscription, Task, UniformListScrollHandle, WeakEntity, Window, canvas,
    div, uniform_list,
};

use super::widgets::*;
use crate::state::{AppState, Page};
use crate::t;
use super::layout::dp;

/// Host, process, rule, outbound, network, down, up, time.
const COLUMNS: usize = 8;
const DEFAULT_WIDTHS: [f32; COLUMNS] = [200., 90., 90., 100., 40., 64., 64., 44.];
const MIN_WIDTH: f32 = 40.;
const MAX_WIDTH: f32 = 1600.;
/// `px_3` and `gap_3` of a row, and the close button at its end.
const PAD: f32 = 12.;
const GAP: f32 = 12.;
const CLOSE: f32 = 24.;

/// What the pointer is dragging on the page.
#[derive(Clone, Copy)]
enum Drag {
    /// A column's right edge.
    Column { index: usize, start_x: f32, start_width: f32 },
    /// The horizontal scrollbar's thumb; `ratio` is content px per thumb px.
    Bar { start_x: f32, start_offset: f32, ratio: f32 },
}

#[derive(Clone)]
struct Row {
    conn: Connection,
    up_speed: u64,
    down_speed: u64,
    age: String,
}

pub struct ConnectionsPage {
    state: Entity<AppState>,
    search: Entity<InputState>,
    search_text: String,
    snapshot: ConnectionsSnapshot,
    rows: Rc<Vec<Row>>,
    prev: HashMap<String, (u64, u64)>,
    scroll: UniformListScrollHandle,
    /// Column widths as dragged; the host column also takes any spare room.
    widths: [f32; COLUMNS],
    /// How far the columns are scrolled sideways.
    h_offset: f32,
    drag: Option<Drag>,
    _poll: Task<()>,
    _subs: Vec<Subscription>,
}

impl ConnectionsPage {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = AppState::global(cx);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("conn.filter")));
        let _subs = vec![
            cx.subscribe(&search, |this, input, e: &InputEvent, cx| {
                if matches!(e, InputEvent::Change) {
                    this.search_text = input.read(cx).value().to_string();
                    this.rebuild_rows(None);
                    cx.notify();
                }
            }),
            cx.observe(&state, |this, state, cx| {
                if !state.read(cx).engine.status().is_running() && !this.rows.is_empty() {
                    this.snapshot = ConnectionsSnapshot::default();
                    this.prev.clear();
                    this.rows = Rc::new(vec![]);
                    cx.notify();
                }
            }),
        ];
        let _poll = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let Ok(Some(engine)) = this.update(cx, |this, cx| {
                    let st = this.state.read(cx);
                    (st.page == Page::Connections && st.engine.status().is_running()).then(|| st.engine.clone())
                }) else {
                    if this.upgrade().is_none() {
                        break;
                    }
                    continue;
                };
                let api = engine.api();
                let snap = engine.runtime().spawn(async move { api.connections().await }).await;
                if let Ok(Ok(snap)) = snap {
                    let _ = this.update(cx, |this, cx| {
                        this.rebuild_rows(Some(snap));
                        cx.notify();
                    });
                }
            }
        });
        let saved = state.read(cx).engine.settings().conn_columns;
        let widths = <[f32; COLUMNS]>::try_from(saved.as_slice()).unwrap_or(DEFAULT_WIDTHS);
        Self {
            state,
            search,
            search_text: String::new(),
            snapshot: ConnectionsSnapshot::default(),
            rows: Rc::new(vec![]),
            prev: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
            widths,
            h_offset: 0.0,
            drag: None,
            _poll,
            _subs,
        }
    }

    fn rebuild_rows(&mut self, snap: Option<ConnectionsSnapshot>) {
        let fresh = snap.is_some();
        if let Some(s) = snap {
            self.snapshot = s;
        }
        let now = chrono::Utc::now();
        let mut next_prev = HashMap::new();
        let filter = self.search_text.to_lowercase();
        let mut rows: Vec<Row> = self
            .snapshot
            .connections
            .iter()
            .flatten()
            .map(|c| {
                let (pu, pd) = self.prev.get(&c.id).copied().unwrap_or((c.upload, c.download));
                next_prev.insert(c.id.clone(), (c.upload, c.download));
                let age = chrono::DateTime::parse_from_rfc3339(&c.start)
                    .map(|s| fmt_span((now - s.with_timezone(&chrono::Utc)).num_seconds()))
                    .unwrap_or_default();
                Row {
                    conn: c.clone(),
                    up_speed: c.upload.saturating_sub(pu),
                    down_speed: c.download.saturating_sub(pd),
                    age,
                }
            })
            .filter(|r| {
                filter.is_empty()
                    || r.conn.target().to_lowercase().contains(&filter)
                    || r.conn.process().to_lowercase().contains(&filter)
                    || r.conn.rule.to_lowercase().contains(&filter)
                    || r.conn.chains.iter().any(|c| c.to_lowercase().contains(&filter))
            })
            .collect();
        rows.sort_by(|a, b| (b.down_speed + b.up_speed).cmp(&(a.down_speed + a.up_speed)).then(b.conn.start.cmp(&a.conn.start)));
        if fresh {
            self.prev = next_prev;
        }
        self.rows = Rc::new(rows);
    }

    /// The widths to draw at, and the width of a whole row: spare room goes to
    /// the host, so the columns always fill the card.
    fn laid_out(&self, viewport: f32) -> ([f32; COLUMNS], f32) {
        let mut widths = self.widths;
        let total = PAD * 2.0 + widths.iter().sum::<f32>() + GAP * COLUMNS as f32 + CLOSE;
        if total < viewport {
            widths[0] += viewport - total;
        }
        (widths, total.max(viewport))
    }

    fn save_widths(&self, cx: &App) {
        let widths = self.widths.to_vec();
        self.state.read(cx).engine.update_settings(move |s| s.conn_columns = widths);
    }

    /// Double-clicking a column's edge fits it to what it shows now.
    fn fit_column(&mut self, index: usize, cx: &mut Context<Self>) {
        let label = text_width(column_title(index));
        let widest = self.rows.iter().map(|r| text_width(&cell_text(r, index))).fold(label, f32::max);
        self.widths[index] = (widest.ceil() + 6.0).clamp(MIN_WIDTH, MAX_WIDTH);
        self.save_widths(cx);
        cx.notify();
    }
}

fn column_title(index: usize) -> &'static str {
    match index {
        0 => t!("col.host"),
        1 => t!("col.process"),
        2 => t!("col.rule"),
        3 => t!("col.outbound"),
        4 => t!("col.net"),
        5 => t!("col.down"),
        6 => t!("col.up"),
        _ => t!("col.time"),
    }
}

fn cell_text(r: &Row, index: usize) -> String {
    let c = &r.conn;
    match index {
        0 => c.target(),
        1 => c.process().to_string(),
        2 if c.rule_payload.is_empty() => c.rule.clone(),
        2 => format!("{} {}", c.rule, c.rule_payload),
        3 => outbound_label(c.outbound()),
        4 => c.metadata.network.to_uppercase(),
        5 => fmt_speed(r.down_speed),
        6 => fmt_speed(r.up_speed),
        _ => r.age.clone(),
    }
}

/// Adds a rule to `CustomRule.list` and says what happened.
fn add_rule(engine: &Arc<Engine>, rule: &str, window: &mut Window, cx: &mut App) {
    match engine.add_custom_rule(rule) {
        Ok(true) => window.push_notification(Notification::success(t!("conn.rule_added", rule)), cx),
        Ok(false) => window.push_notification(t!("conn.rule_exists"), cx),
        Err(e) => window.push_notification(Notification::error(format!("{e:#}")), cx),
    }
}

/// Right-click on a connection: always route what it matches one way, by
/// site, host, process or (without a host) address.
fn connection_menu(
    menu: PopupMenu,
    c: &Connection,
    engine: &Arc<Engine>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let host = c.metadata.host.clone();
    let mut matches: Vec<(String, String)> = Vec::new();
    if let Some(base) = base_domain(&host) {
        if base != host {
            matches.push((t!("conn.match_host", host), format!("DOMAIN,{host}")));
        }
        matches.push((t!("conn.match_site", base), format!("DOMAIN-SUFFIX,{base}")));
    } else if !host.is_empty() {
        matches.push((t!("conn.match_host", host), format!("DOMAIN,{host}")));
    }
    let ip = c.metadata.destination_ip.clone();
    if host.is_empty() && !ip.is_empty() {
        let bits = if ip.contains(':') { 128 } else { 32 };
        matches.push((t!("conn.match_ip", ip), format!("IP-CIDR,{ip}/{bits}")));
    }
    let process = c.process().to_string();
    if !process.is_empty() {
        matches.push((t!("conn.match_process", process), format!("PROCESS-NAME,{process}")));
    }
    let matches = Rc::new(matches);
    let mut menu = menu.label(c.target());
    for (label, target) in [(t!("conn.always_direct"), "direct"), (t!("conn.always_proxy"), "proxy"), (t!("conn.always_block"), "reject")] {
        let (matches, engine) = (matches.clone(), engine.clone());
        menu = menu.submenu(label, window, cx, move |mut sub, _, _| {
            for (label, key) in matches.iter() {
                let (rule, engine) = (format!("{key},{target}"), engine.clone());
                sub = sub.item(
                    PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| add_rule(&engine, &rule, window, cx)),
                );
            }
            sub
        });
    }
    let target = c.target();
    let (id, e_close) = (c.id.clone(), engine.clone());
    menu.separator()
        .item(PopupMenuItem::new(t!("conn.copy_address")).on_click(move |_, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(target.clone()));
            window.push_notification(t!("conn.copied"), cx);
        }))
        .item(PopupMenuItem::new(t!("conn.close")).on_click(move |_, _, _| {
            let (api, id) = (e_close.api(), id.clone());
            e_close.runtime().spawn(async move {
                let _ = api.close_connection(&id).await;
            });
        }))
}

impl Render for ConnectionsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        sync_placeholder(&self.search, t!("conn.filter"), window, cx);
        let engine = self.state.read(cx).engine.clone();
        let running = engine.status().is_running();
        let t = cx.theme();
        let total = self.snapshot.connections.as_ref().map_or(0, Vec::len);
        let e_all = engine.clone();
        let actions = vec![
            div().w(dp(240.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
            Button::new("close-all")
                .small()
                .icon(extra_icon(Lucide::Unplug))
                .label(t!("btn.close_all"))
                .disabled(total == 0)
                .on_click(move |_, _, _| {
                    let api = e_all.api();
                    e_all.runtime().spawn(async move {
                        let _ = api.close_all_connections().await;
                    });
                })
                .into_any_element(),
        ];
        let subtitle = t!(
            "conn.subtitle",
            total,
            fmt_bytes(self.snapshot.upload_total),
            fmt_bytes(self.snapshot.download_total)
        );
        let header = page(t!("nav.connections"), Some(subtitle), actions, cx);
        if !running {
            return header
                .child(card(cx).child(empty_state(
                    extra_icon(Lucide::Cable),
                    t!("proxies.not_connected"),
                    t!("conn.offline_hint"),
                    None,
                    cx,
                )))
                .into_any_element();
        }

        // The card's inside; its border takes a pixel on each side.
        let viewport = super::layout::metrics(window).content - 2.0;
        let (widths, total) = self.laid_out(viewport);
        let max_offset = (total - viewport).max(0.0);
        self.h_offset = self.h_offset.clamp(0.0, max_offset);
        let offset = self.h_offset;

        let rows = self.rows.clone();
        let (fg, muted, border, hover, danger, edge, primary) =
            (t.foreground, t.muted_foreground, t.border, t.muted, t.danger, hairline(cx), t.primary);
        let row_count = rows.len();
        let base = self.scroll.0.borrow().base_handle.clone();
        let list = uniform_list("conn-list", row_count, move |range, _, _| {
            range
                .map(|i| {
                    let r = &rows[i];
                    let c = &r.conn;
                    let id = c.id.clone();
                    let (e_menu, c_menu, engine) = (engine.clone(), c.clone(), engine.clone());
                    // Host, process, rule and outbound read as text; the rest as small figures.
                    let cell = |index: usize| {
                        div()
                            .w(dp(widths[index]))
                            .flex_none()
                            .truncate()
                            .when(index >= 4, |d| d.text_xs())
                            .when(matches!(index, 1 | 2 | 4 | 7), |d| d.text_color(muted))
                            .when(index == 0, |d| d.text_color(fg))
                            .child(cell_text(r, index))
                    };
                    // List items are laid out as roots: without a width they
                    // shrink to their content and the columns drift, and their
                    // own margin is ignored, so the row scrolls inside a box.
                    div().w(dp(viewport)).h(dp(36.)).child(
                        h_flex()
                            .id(("conn", i))
                            .w(dp(total))
                            .flex_none()
                            .ml(dp(-offset))
                            .h(dp(36.))
                            .px(dp(PAD))
                            .gap(dp(GAP))
                            .items_center()
                            .border_b_1()
                            .border_color(border)
                            .text_sm()
                            .hover(|d| d.bg(hover))
                            .children((0..COLUMNS).map(cell))
                            .child(
                                div()
                                    .id(SharedString::from(format!("x-{id}")))
                                    .w(dp(CLOSE))
                                    .flex_none()
                                    .text_color(muted)
                                    .cursor_pointer()
                                    .hover(|d| d.text_color(danger))
                                    .on_click(move |_, _, _| {
                                        let api = engine.api();
                                        let id = id.clone();
                                        engine.runtime().spawn(async move {
                                            let _ = api.close_connection(&id).await;
                                        });
                                    })
                                    .child(IconName::Close),
                            )
                            .context_menu(move |menu, window, cx| connection_menu(menu, &c_menu, &e_menu, window, cx)),
                    )
                })
                .collect::<Vec<_>>()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        // Column titles, each with a draggable edge in the gap after it.
        let this = cx.entity().downgrade();
        let title = |index: usize| {
            let this = this.clone();
            let start_width = self.widths[index];
            div()
                .relative()
                .w(dp(widths[index]))
                .flex_none()
                .h_full()
                .flex()
                .items_center()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted)
                        .child(column_title(index)),
                )
                .child(
                    div()
                        .id(("col-edge", index))
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right(dp(-(GAP / 2.0 + 5.0)))
                        .w(dp(10.))
                        .flex()
                        .justify_center()
                        .cursor(CursorStyle::ResizeColumn)
                        .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                            cx.stop_propagation();
                            let (count, x) = (e.click_count, f32::from(e.position.x));
                            let _ = this.update(cx, |page, cx| {
                                if count >= 2 {
                                    page.fit_column(index, cx);
                                } else {
                                    page.drag = Some(Drag::Column { index, start_x: x, start_width });
                                    cx.notify();
                                }
                            });
                        })
                        .child(div().id(("col-line", index)).w(dp(1.)).h_full().my_2().bg(edge).hover(move |d| d.bg(primary))),
                )
        };
        let header_row = h_flex()
            .w(dp(total))
            .flex_none()
            .ml(dp(-offset))
            .h(dp(34.))
            .px(dp(PAD))
            .gap(dp(GAP))
            .items_center()
            .children((0..COLUMNS).map(title))
            .child(div().w(dp(CLOSE)).flex_none());

        // A scrollbar along the bottom when the columns outgrow the card.
        let bar = (max_offset > 0.0).then(|| {
            let thumb = (viewport * viewport / total).max(40.0);
            let ratio = max_offset / (viewport - thumb).max(1.0);
            let this = this.clone();
            div()
                .flex_none()
                .relative()
                .h(dp(12.))
                .w_full()
                .child(
                    div()
                        .id("conn-hbar")
                        .absolute()
                        .top(dp(3.))
                        .left(dp(offset / ratio))
                        .w(dp(thumb))
                        .h(dp(6.))
                        .rounded_full()
                        .bg(muted.opacity(0.35))
                        .hover(move |d| d.bg(muted.opacity(0.6)))
                        .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                            cx.stop_propagation();
                            let x = f32::from(e.position.x);
                            let _ = this.update(cx, |page, cx| {
                                page.drag = Some(Drag::Bar { start_x: x, start_offset: page.h_offset, ratio });
                                cx.notify();
                            });
                        }),
                )
        });

        // Sideways wheel, Shift + wheel and drags, wherever the pointer goes.
        let (this_events, dragging) = (this.clone(), self.drag.is_some());
        let events = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let this = this_events.clone();
                window.on_mouse_event(move |e: &ScrollWheelEvent, phase, window, cx| {
                    if phase != DispatchPhase::Capture || !bounds.contains(&e.position) {
                        return;
                    }
                    let delta = e.delta.pixel_delta(window.line_height());
                    let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
                    let sideways = if dx != 0.0 { dx } else if e.modifiers.shift { dy } else { 0.0 };
                    if sideways == 0.0 {
                        return;
                    }
                    let _ = this.update(cx, |page, cx| {
                        page.h_offset = (page.h_offset - sideways).clamp(0.0, max_offset);
                        cx.notify();
                    });
                    if e.modifiers.shift {
                        cx.stop_propagation();
                    }
                });
                if !dragging {
                    return;
                }
                let this = this_events.clone();
                window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let x = f32::from(e.position.x);
                    let held = e.pressed_button == Some(MouseButton::Left);
                    let _ = this.update(cx, |page, cx| {
                        match page.drag {
                            // Let go outside the window: the release never came.
                            Some(Drag::Column { .. }) if !held => {
                                page.drag = None;
                                page.save_widths(cx);
                            }
                            Some(_) if !held => page.drag = None,
                            Some(Drag::Column { index, start_x, start_width }) => {
                                page.widths[index] = (start_width + x - start_x).clamp(MIN_WIDTH, MAX_WIDTH);
                            }
                            Some(Drag::Bar { start_x, start_offset, ratio }) => {
                                page.h_offset = (start_offset + (x - start_x) * ratio).clamp(0.0, max_offset);
                            }
                            None => {}
                        }
                        cx.notify();
                    });
                });
                let this = this_events.clone();
                window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture {
                        return;
                    }
                    let _ = this.update(cx, |page, cx| {
                        if matches!(page.drag.take(), Some(Drag::Column { .. })) {
                            page.save_widths(cx);
                        }
                        cx.notify();
                    });
                });
            },
        )
        .absolute()
        .inset_0();

        header
            .flex_1()
            .min_h_0()
            .child(
                card(cx)
                    .relative()
                    .p_0()
                    .gap_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex_none()
                            .w_full()
                            .overflow_hidden()
                            .border_b_1()
                            .border_color(border)
                            .bg(t.muted.opacity(0.5))
                            .child(header_row),
                    )
                    .when(row_count == 0, |d| {
                        d.child(empty_state(extra_icon(Lucide::Cable), t!("conn.none"), t!("conn.none_hint"), None, cx))
                    })
                    .when(row_count > 0, |d| {
                        d.child(
                            super::smooth_scroll::with_scrollbar(
                                "conn-scrollbar",
                                &self.scroll,
                                super::smooth_scroll::smooth_scroll("conn-smooth", &base, list),
                            )
                            .flex_1()
                            .min_h_0(),
                        )
                    })
                    .children(bar)
                    .child(events),
            )
            .into_any_element()
    }
}
