use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use boxpilot_core::clash_api::{Connection, ConnectionsSnapshot};
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::component::Disableable as _;
use gpui_kit::prelude::*;
use gpui_kit::{
    UniformListScrollHandle, AsyncApp, Context, Entity, FontWeight, SharedString, Subscription, Task, WeakEntity, Window, div, px,
    uniform_list,
};

use super::widgets::*;
use crate::state::{AppState, Page};
use crate::t;

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
        Self {
            state,
            search,
            search_text: String::new(),
            snapshot: ConnectionsSnapshot::default(),
            rows: Rc::new(vec![]),
            prev: HashMap::new(),
            scroll: UniformListScrollHandle::new(),
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
}

impl Render for ConnectionsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let engine = self.state.read(cx).engine.clone();
        let running = engine.status().is_running();
        let t = cx.theme();
        let total = self.snapshot.connections.as_ref().map_or(0, Vec::len);
        let e_all = engine.clone();
        let actions = vec![
            div().w(px(240.)).child(Input::new(&self.search).small().cleanable(true)).into_any_element(),
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

        let head = |label: &'static str, w: Option<f32>| {
            let d = div().text_xs().font_weight(FontWeight::MEDIUM).text_color(t.muted_foreground).truncate().child(label);
            match w {
                Some(w) => d.w(px(w)).flex_none(),
                None => d.flex_1().min_w_0(),
            }
        };
        let rows = self.rows.clone();
        let (fg, muted, border, hover, danger) = (t.foreground, t.muted_foreground, t.border, t.muted, t.danger);
        let row_count = rows.len();
        let base = self.scroll.0.borrow().base_handle.clone();
        let list = uniform_list("conn-list", row_count, move |range, _, _| {
            range
                .map(|i| {
                    let r = &rows[i];
                    let c = &r.conn;
                    let id = c.id.clone();
                    let engine = engine.clone();
                    h_flex()
                        .id(("conn", i))
                        .h(px(36.))
                        .px_3()
                        .gap_3()
                        .items_center()
                        .border_b_1()
                        .border_color(border)
                        .text_sm()
                        .hover(|d| d.bg(hover))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(fg)
                                .child(c.target()),
                        )
                        .child(div().w(px(130.)).flex_none().truncate().text_color(muted).child(c.process().to_string()))
                        .child(
                            div()
                                .w(px(150.))
                                .flex_none()
                                .truncate()
                                .text_color(muted)
                                .child(if c.rule_payload.is_empty() { c.rule.clone() } else { format!("{} {}", c.rule, c.rule_payload) }),
                        )
                        .child(div().w(px(150.)).flex_none().truncate().child(outbound_label(c.outbound())))
                        .child(div().w(px(44.)).flex_none().text_xs().text_color(muted).child(c.metadata.network.to_uppercase()))
                        .child(div().w(px(84.)).flex_none().text_xs().child(fmt_speed(r.down_speed)))
                        .child(div().w(px(84.)).flex_none().text_xs().child(fmt_speed(r.up_speed)))
                        .child(div().w(px(56.)).flex_none().text_xs().text_color(muted).child(r.age.clone()))
                        .child(
                            div()
                                .id(SharedString::from(format!("x-{id}")))
                                .w(px(24.))
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
                })
                .collect()
        })
        .track_scroll(&self.scroll)
        .flex_1()
        .min_h_0();

        header
            .flex_1()
            .min_h_0()
            .child(
                card(cx)
                    .p_0()
                    .gap_0()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(
                        h_flex()
                            .h(px(34.))
                            .px_3()
                            .gap_3()
                            .items_center()
                            .border_b_1()
                            .border_color(border)
                            .bg(t.muted.opacity(0.5))
                            .child(head(t!("col.host"), None))
                            .child(head(t!("col.process"), Some(130.)))
                            .child(head(t!("col.rule"), Some(150.)))
                            .child(head(t!("col.outbound"), Some(150.)))
                            .child(head(t!("col.net"), Some(44.)))
                            .child(head(t!("col.down"), Some(84.)))
                            .child(head(t!("col.up"), Some(84.)))
                            .child(head(t!("col.time"), Some(56.)))
                            .child(head("", Some(24.))),
                    )
                    .when(row_count == 0, |d| {
                        d.child(empty_state(extra_icon(Lucide::Cable), t!("conn.none"), t!("conn.none_hint"), None, cx))
                    })
                    .when(row_count > 0, |d| d.child(super::smooth_scroll::smooth_scroll("conn-smooth", &base, list))),
            )
            .into_any_element()
    }
}
