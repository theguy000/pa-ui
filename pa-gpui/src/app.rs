use crate::daemon::{DaemonBridge, DaemonCmd, DaemonEvent};
use crate::transcript::{build_items, ItemKind, TranscriptItem};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    text::{TextView, TextViewState},
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::SessionRow;
use std::time::Duration;

pub struct PaApp {
    bridge: DaemonBridge,
    status: SharedString,
    sessions: Vec<SessionRow>,
    active_session_id: Option<String>,
    items: Vec<TranscriptItem>,
    scroll_handle: ScrollHandle,
    composer: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    _poll_task: Task<()>,
}

impl PaApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let bridge = DaemonBridge::spawn();
        let composer = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Message Prime Agent… (Enter to send)")
        });

        let sub = cx.subscribe_in(&composer, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(ev, InputEvent::PressEnter { secondary: false, shift: false }) {
                this.send_current_prompt(window, cx);
            }
        });

        let poll_task = cx.spawn(async move |weak_self, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(40))
                .await;
            let keep_going = weak_self
                .update(cx, |this, cx| {
                    this.drain_daemon(cx);
                })
                .is_ok();
            if !keep_going {
                break;
            }
        });

        Self {
            bridge,
            status: "Connecting to daemon…".into(),
            sessions: Vec::new(),
            active_session_id: None,
            items: Vec::new(),
            scroll_handle: ScrollHandle::new(),
            composer,
            _subscriptions: vec![sub],
            _poll_task: poll_task,
        }
    }

    fn drain_daemon(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(ev) = self.bridge.rx.try_recv() {
            changed = true;
            match ev {
                DaemonEvent::Connected => {
                    self.status = "Connected".into();
                }
                DaemonEvent::Disconnected(reason) => {
                    self.status = format!("Disconnected: {reason}").into();
                }
                DaemonEvent::Sessions(rows) => {
                    self.sessions = rows;
                    if self.active_session_id.is_none() {
                        if let Some(first) = self.sessions.first() {
                            self.active_session_id = Some(session_key(first));
                        }
                    }
                }
                DaemonEvent::Messages { session, messages } => {
                    self.active_session_id = Some(session);
                    self.items = build_items(&messages, cx);
                    self.scroll_handle.scroll_to_bottom();
                }
                DaemonEvent::LiveFrame(frame) => {
                    let ty = frame.get("type").and_then(|v| v.as_str()).unwrap_or_default();
                    if matches!(ty, "session_updated" | "message_end" | "turn_end" | "agent_end") {
                        if let Some(sid) = self.active_session_id.clone() {
                            let _ = self.bridge.tx.send(DaemonCmd::SelectSession(sid));
                        }
                        let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
                    }
                }
                DaemonEvent::Error(err) => {
                    self.status = format!("Error: {err}").into();
                }
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn send_current_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(sid) = self.active_session_id.clone() else {
            return;
        };
        let _ = self.bridge.tx.send(DaemonCmd::SendPrompt {
            session: sid,
            text,
        });
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn toggle_item(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.expanded = !item.expanded;
            if item.expanded && item.body_md.is_none() && !item.raw_body.is_empty() {
                let body = item.raw_body.clone();
                let formatted = if item.kind == ItemKind::ToolCall {
                    format!("```\n{body}\n```")
                } else {
                    body
                };
                item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
            }
            cx.notify();
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let active_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| s.is_resident())
            .collect();
        let recent_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| !s.is_resident())
            .take(40)
            .collect();

        v_flex()
            .w(px(244.))
            .h_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .p_3()
            .gap_3()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child("Prime Agent"),
                    )
                    .child(
                        Button::new("new-session")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|this, _, _, _| {
                                let _ = this.bridge.tx.send(DaemonCmd::CreateSession);
                            })),
                    ),
            )
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .gap_3()
                            .child(self.render_session_group("Active", &active_rows, cx))
                            .child(self.render_session_group("Recent", &recent_rows, cx)),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(self.status.clone()),
            )
    }

    fn render_session_group(
        &self,
        title: &'static str,
        rows: &[&SessionRow],
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mut group = v_flex().gap_1().child(
            div()
                .px_2()
                .text_xs()
                .font_medium()
                .text_color(theme.muted_foreground)
                .child(format!("{title} · {}", rows.len())),
        );

        for (idx, row) in rows.iter().enumerate() {
            let key = session_key(row);
            let selected = self.active_session_id.as_deref() == Some(key.as_str());
            let label = if !row.name.trim().is_empty() {
                row.name.clone()
            } else {
                key.chars().take(12).collect()
            };
            let click_key = key.clone();

            group = group.child(
                div()
                    .id((title, idx))
                    .px_2()
                    .py_1()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.accent))
                    .hover(|d| d.bg(theme.accent.opacity(0.6)))
                    .text_sm()
                    .text_color(if selected {
                        theme.accent_foreground
                    } else {
                        theme.foreground
                    })
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, _| {
                        let _ = this
                            .bridge
                            .tx
                            .send(DaemonCmd::SelectSession(click_key.clone()));
                    })),
            );
        }

        group
    }

    fn render_item(&self, item: &TranscriptItem, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match item.kind {
            ItemKind::User => div()
                .w_full()
                .rounded(px(10.))
                .bg(theme.secondary)
                .px_4()
                .py_3()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| TextView::new(md).selectable(true)),
                )
                .into_any_element(),
            ItemKind::Assistant => div().w_full().py_1().children(
                item.body_md
                    .as_ref()
                    .map(|md| TextView::new(md).selectable(true)),
            )
            .into_any_element(),
            ItemKind::Error => div()
                .w_full()
                .rounded(px(8.))
                .border_1()
                .border_color(theme.danger)
                .bg(theme.danger.opacity(0.08))
                .px_3()
                .py_2()
                .text_sm()
                .text_color(theme.danger_foreground)
                .child(item.raw_body.clone())
                .into_any_element(),
            ItemKind::Thinking | ItemKind::ToolCall | ItemKind::Summary | ItemKind::Custom => {
                let id = item.id;
                let icon = match item.kind {
                    ItemKind::ToolCall => IconName::SquareTerminal,
                    ItemKind::Summary => IconName::File,
                    ItemKind::Thinking => IconName::Info,
                    _ => IconName::Bot,
                };
                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.secondary.opacity(0.4))
                    .child(
                        h_flex()
                            .id(("card-hdr", id))
                            .h(px(30.))
                            .px_3()
                            .gap_2()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_item(id, cx);
                            }))
                            .child(Icon::new(icon).xsmall().text_color(theme.muted_foreground))
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(theme.foreground)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .child(item.title.clone()),
                            )
                            .when(!item.meta.is_empty(), |row| {
                                row.child(
                                    div()
                                        .px_2()
                                        .rounded(px(9.))
                                        .border_1()
                                        .border_color(theme.border)
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(item.meta.clone()),
                                )
                            })
                            .child(
                                Icon::new(if item.expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .xsmall()
                                .text_color(theme.muted_foreground),
                            ),
                    )
                    .when(item.expanded, |card| {
                        card.child(
                            div()
                                .border_t_1()
                                .border_color(theme.border)
                                .px_3()
                                .py_2()
                                .children(
                                    item.body_md
                                        .as_ref()
                                        .map(|md| TextView::new(md).selectable(true)),
                                ),
                        )
                    })
                    .into_any_element()
            }
        }
    }
}

impl Render for PaApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let active_title = self
            .active_session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| session_key(s) == id))
            .map(|s| {
                if !s.name.trim().is_empty() {
                    s.name.clone()
                } else {
                    session_key(s)
                }
            })
            .unwrap_or_else(|| "Prime Agent (GPUI Spike)".to_string());

        let active_model = self
            .active_session_id
            .as_deref()
            .and_then(|id| self.sessions.iter().find(|s| session_key(s) == id))
            .map(|s| format!("{} · {}", s.model_name, s.thinking))
            .unwrap_or_default();

        let mut rendered_items = Vec::with_capacity(self.items.len());
        for it in &self.items {
            rendered_items.push(self.render_item(it, cx));
        }

        h_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .h_full()
                    .child(
                        h_flex()
                            .h(px(44.))
                            .px_5()
                            .border_b_1()
                            .border_color(theme.border)
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_semibold()
                                    .child(active_title),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(active_model),
                            ),
                    )
                    .child(
                        div()
                            .id("transcript-scroll")
                            .flex_1()
                            .w_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll_handle)
                            .px_6()
                            .py_4()
                            .child(
                                v_flex()
                                    .w_full()
                                    .max_w(px(860.))
                                    .mx_auto()
                                    .gap_3()
                                    .children(rendered_items),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .px_6()
                            .py_3()
                            .border_t_1()
                            .border_color(theme.border)
                            .gap_2()
                            .items_end()
                            .child(div().flex_1().child(Input::new(&self.composer)))
                            .child(
                                Button::new("send-btn")
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_current_prompt(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("abort-btn")
                                    .outline()
                                    .label("Stop")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        if let Some(sid) = this.active_session_id.clone() {
                                            let _ = this
                                                .bridge
                                                .tx
                                                .send(DaemonCmd::Abort { session: sid });
                                        }
                                    })),
                            ),
                    ),
            )
    }
}

fn session_key(row: &SessionRow) -> String {
    if !row.id.is_empty() {
        row.id.clone()
    } else {
        row.session_file.clone()
    }
}
