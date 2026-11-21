use crate::daemon::{DaemonBridge, DaemonCmd, DaemonEvent};
use crate::transcript::{
    append_text_delta, append_thinking_delta, build_items, finish_tool_call, format_expanded_body,
    settle_all, start_tool_call, CardVariant, ItemKind, TranscriptItem,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu as _, PopupMenuItem},
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState},
    v_flex, ActiveTheme as _, Icon, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode,
    TitleBar,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{ModelEntry, SessionRow, SessionStats};
use serde_json::Value;
use std::time::Duration;

pub struct PaApp {
    bridge: DaemonBridge,
    status: SharedString,
    notice: Option<SharedString>,
    sessions: Vec<SessionRow>,
    models: Vec<ModelEntry>,
    stats: Option<SessionStats>,
    queued_steer: Vec<String>,
    queued_follow_up: Vec<String>,
    active_session_id: Option<String>,
    renaming_path: Option<String>,
    items: Vec<TranscriptItem>,
    is_streaming: bool,
    sidebar_open: bool,
    dark_mode: bool,
    scroll_handle: ScrollHandle,
    search_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    composer: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
    _poll_task: Task<()>,
}

impl PaApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let bridge = DaemonBridge::spawn();
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Filter sessions…"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New session name…"));
        let composer = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Message Prime Agent… (/ for commands)")
        });

        let sub_composer = cx.subscribe_in(&composer, window, |this, _, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => {
                    this.send_current_prompt(false, window, cx);
                }
                InputEvent::Change => {
                    cx.notify();
                }
                _ => {}
            }
        });

        let sub_search = cx.subscribe_in(&search_input, window, |_this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change) {
                cx.notify();
            }
        });

        let sub_rename = cx.subscribe_in(&rename_input, window, |this, _, ev: &InputEvent, window, cx| {
            if matches!(
                ev,
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                }
            ) {
                this.commit_rename(window, cx);
            }
        });

        let poll_task = cx.spawn(async move |weak_self, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(24))
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
            notice: None,
            sessions: Vec::new(),
            models: Vec::new(),
            stats: None,
            queued_steer: Vec::new(),
            queued_follow_up: Vec::new(),
            active_session_id: None,
            renaming_path: None,
            items: Vec::new(),
            is_streaming: false,
            sidebar_open: true,
            dark_mode: true,
            scroll_handle: ScrollHandle::new(),
            search_input,
            rename_input,
            composer,
            _subscriptions: vec![sub_composer, sub_search, sub_rename],
            _poll_task: poll_task,
        }
    }

    fn is_near_bottom(&self) -> bool {
        let max_y = self.scroll_handle.max_offset().y;
        if max_y <= px(4.) {
            return true;
        }
        let cur_y = -self.scroll_handle.offset().y;
        (max_y - cur_y) <= px(120.)
    }

    fn scroll_if_sticky(&self) {
        if self.is_near_bottom() {
            self.scroll_handle.scroll_to_bottom();
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
                    self.status = reason.into();
                    self.is_streaming = false;
                    settle_all(&mut self.items);
                }
                DaemonEvent::Sessions(rows) => {
                    self.sessions = rows;
                    if self.active_session_id.is_none() {
                        if let Some(first) = self.sessions.first() {
                            self.active_session_id = Some(session_key(first));
                        }
                    }
                    if let Some(sid) = self.active_session_id.as_deref() {
                        if let Some(row) = self.sessions.iter().find(|s| session_key(s) == sid) {
                            self.is_streaming = row.is_streaming;
                        }
                    }
                }
                DaemonEvent::Messages { session, messages } => {
                    let switching = self.active_session_id.as_deref() != Some(session.as_str());
                    self.active_session_id = Some(session);
                    self.items = build_items(&messages, cx);
                    if switching || self.is_near_bottom() {
                        self.scroll_handle.scroll_to_bottom();
                    }
                }
                DaemonEvent::Models(models) => {
                    self.models = models;
                }
                DaemonEvent::Stats { session, stats } => {
                    if self.active_session_id.as_deref() == Some(session.as_str()) {
                        self.stats = Some(stats);
                    }
                }
                DaemonEvent::Queue {
                    session,
                    steer,
                    follow_up,
                } => {
                    if self.active_session_id.as_deref() == Some(session.as_str()) {
                        self.queued_steer = steer;
                        self.queued_follow_up = follow_up;
                    }
                }
                DaemonEvent::Notice(msg) => {
                    self.notice = Some(msg.into());
                }
                DaemonEvent::LiveFrame(frame) => {
                    self.handle_live_frame(&frame, cx);
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

    fn handle_live_frame(&mut self, frame: &Value, cx: &mut Context<Self>) {
        let frame_sid = frame
            .get("activeSessionId")
            .or_else(|| frame.get("sessionId"))
            .and_then(Value::as_str);
        let matches_active = match (frame_sid, self.active_session_id.as_deref()) {
            (Some(fs), Some(cur)) => fs == cur,
            _ => true,
        };
        if !matches_active {
            return;
        }

        let event = frame.get("event").filter(|v| v.is_object()).unwrap_or(frame);
        let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();

        match kind {
            "agent_start" | "turn_start" => {
                self.is_streaming = true;
            }
            "message_update" => {
                self.is_streaming = true;
                let delta_ev = event
                    .get("assistantMessageEvent")
                    .filter(|v| v.is_object())
                    .unwrap_or(event);
                let delta_kind = delta_ev
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(delta) = delta_ev.get("delta").and_then(Value::as_str) {
                    if delta_kind.starts_with("text_delta") {
                        append_text_delta(&mut self.items, delta, cx);
                        self.scroll_if_sticky();
                    } else if delta_kind.starts_with("thinking_delta") {
                        append_thinking_delta(&mut self.items, delta, cx);
                        self.scroll_if_sticky();
                    }
                }
            }
            "tool_execution_start" => {
                self.is_streaming = true;
                let cid = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = event
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let args = event.get("args").unwrap_or(&Value::Null);
                start_tool_call(&mut self.items, cid, name, args);
                self.scroll_if_sticky();
            }
            "tool_execution_end" => {
                let cid = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_err = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let out = event
                    .get("result")
                    .and_then(|r| {
                        r.as_str().map(str::to_string).or_else(|| {
                            r.get("content").and_then(Value::as_array).map(|arr| {
                                arr.iter()
                                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                        })
                    })
                    .unwrap_or_default();
                finish_tool_call(&mut self.items, cid, out, is_err, cx);
            }
            "agent_end" | "turn_end" => {
                self.is_streaming = false;
                settle_all(&mut self.items);
                if let Some(sid) = self.active_session_id.clone() {
                    let _ = self.bridge.tx.send(DaemonCmd::SelectSession(sid));
                }
                let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            }
            "session_updated" => {
                let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            }
            _ => {}
        }
    }

    fn send_current_prompt(&mut self, steer: bool, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(sid) = self.active_session_id.clone() else {
            return;
        };
        let next_id = self.items.last().map_or(0, |i| i.id + 1);
        let md = cx.new(|cx| TextViewState::markdown(&text, cx));
        self.items.push(TranscriptItem {
            id: next_id,
            call_id: None,
            kind: ItemKind::User,
            variant: CardVariant::None,
            verb: SharedString::default(),
            title: "You".into(),
            meta: SharedString::default(),
            body_md: Some(md),
            raw_body: text.clone(),
            output: None,
            expanded: true,
            active: false,
            failed: false,
        });
        self.scroll_handle.scroll_to_bottom();

        if steer {
            let _ = self.bridge.tx.send(DaemonCmd::SteerPrompt {
                session: sid,
                text,
            });
        } else {
            let _ = self.bridge.tx.send(DaemonCmd::SendPrompt {
                session: sid,
                text,
            });
        }
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn start_rename(&mut self, path: String, current_name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming_path = Some(path);
        self.rename_input.update(cx, |input, cx| {
            input.set_value(current_name, window, cx);
        });
        cx.notify();
    }

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.renaming_path.take() else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if !name.is_empty() {
            let _ = self.bridge.tx.send(DaemonCmd::RenameSaved {
                session_path: path,
                name,
            });
        }
        self.rename_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn toggle_item(&mut self, id: usize, cx: &mut Context<Self>) {
        if let Some(item) = self.items.iter_mut().find(|i| i.id == id) {
            item.expanded = !item.expanded;
            if item.expanded && item.body_md.is_none() && !item.raw_body.is_empty() {
                let formatted = format_expanded_body(item);
                item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
            }
            cx.notify();
        }
    }

    fn copy_text(&self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn active_session(&self) -> Option<&SessionRow> {
        let sid = self.active_session_id.as_deref()?;
        self.sessions.iter().find(|s| session_key(s) == sid)
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let active_title = self
            .active_session()
            .map(|s| {
                if !s.name.trim().is_empty() {
                    s.name.clone()
                } else {
                    session_key(s)
                }
            })
            .unwrap_or_else(|| "Prime Agent".to_string());

        let context_label = self
            .stats
            .as_ref()
            .filter(|st| st.context_window > 0)
            .map(|st| {
                format!(
                    "{:.0}k / {:.0}k ({:.0}%) · ${:.3}",
                    st.context_tokens as f64 / 1000.0,
                    st.context_window as f64 / 1000.0,
                    st.percent,
                    st.cost
                )
            })
            .unwrap_or_default();

        TitleBar::new().child(
            h_flex()
                .w_full()
                .h_full()
                .px_2()
                .items_center()
                .justify_between()
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            Button::new("toggle-sidebar")
                                .ghost()
                                .xsmall()
                                .icon(if self.sidebar_open {
                                    IconName::PanelLeftClose
                                } else {
                                    IconName::PanelLeftOpen
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sidebar_open = !this.sidebar_open;
                                    cx.notify();
                                })),
                        )
                        .when(self.is_streaming, |h| {
                            h.child(div().size(px(7.)).rounded_full().bg(theme.warning))
                        })
                        .child(
                            div()
                                .text_xs()
                                .font_semibold()
                                .text_color(theme.foreground)
                                .child(active_title),
                        ),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .pr_2()
                        .when(!context_label.is_empty(), |h| {
                            h.child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(context_label),
                            )
                        })
                        .when(self.active_session_id.is_some(), |h| {
                            h.child(
                                Button::new("export-html")
                                    .ghost()
                                    .xsmall()
                                    .label("Export")
                                    .on_click(cx.listener(|this, _, _, _| {
                                        if let Some(sid) = this.active_session_id.clone() {
                                            let _ = this
                                                .bridge
                                                .tx
                                                .send(DaemonCmd::ExportHtml { session: sid });
                                        }
                                    })),
                            )
                        })
                        .child(
                            Button::new("toggle-theme")
                                .ghost()
                                .xsmall()
                                .icon(if self.dark_mode {
                                    IconName::Sun
                                } else {
                                    IconName::Moon
                                })
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.dark_mode = !this.dark_mode;
                                    Theme::change(
                                        if this.dark_mode {
                                            ThemeMode::Dark
                                        } else {
                                            ThemeMode::Light
                                        },
                                        Some(window),
                                        cx,
                                    );
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let query = self.search_input.read(cx).value().trim().to_ascii_lowercase();
        let matches_query = |s: &&SessionRow| -> bool {
            if query.is_empty() {
                return true;
            }
            s.name.to_ascii_lowercase().contains(&query)
                || s.id.to_ascii_lowercase().contains(&query)
                || s.cwd.to_ascii_lowercase().contains(&query)
        };

        let active_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| s.is_resident())
            .filter(matches_query)
            .collect();
        let recent_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| !s.is_resident())
            .filter(matches_query)
            .take(50)
            .collect();

        v_flex()
            .w(px(244.))
            .h_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .p_3()
            .gap_2()
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
                            .child("Sessions"),
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
            .child(Input::new(&self.search_input).xsmall())
            .when(self.renaming_path.is_some(), |col| {
                col.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(div().flex_1().child(Input::new(&self.rename_input).xsmall()))
                        .child(
                            Button::new("save-rename")
                                .primary()
                                .xsmall()
                                .label("Save")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.commit_rename(window, cx);
                                })),
                        ),
                )
            })
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .gap_3()
                            .child(self.render_session_group("Active", &active_rows, true, cx))
                            .child(self.render_session_group("Recent", &recent_rows, false, cx)),
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
        is_active_group: bool,
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

        if rows.is_empty() && is_active_group {
            group = group.child(
                div()
                    .mx_1()
                    .px_2()
                    .py_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.border.opacity(0.6))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No active sessions"),
            );
            return group;
        }

        for (idx, row) in rows.iter().enumerate() {
            let key = session_key(row);
            let selected = self.active_session_id.as_deref() == Some(key.as_str());
            let label = if !row.name.trim().is_empty() {
                row.name.clone()
            } else {
                key.chars().take(12).collect()
            };
            let click_key = key.clone();
            let action_id = row.id.clone();
            let action_file = row.session_file.clone();
            let cur_name = label.clone();
            let streaming = row.is_streaming;
            let tx_clone = self.bridge.tx.clone();
            let view_entity = cx.entity().clone();

            group = group.child(
                h_flex()
                    .id((title, idx))
                    .px_2()
                    .py_1()
                    .gap_1()
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(theme.accent))
                    .hover(|d| d.bg(theme.accent.opacity(0.6)))
                    .when(streaming, |d| {
                        d.child(div().size(px(6.)).rounded_full().bg(theme.warning))
                    })
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(if selected {
                                theme.accent_foreground
                            } else {
                                theme.foreground
                            })
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(label),
                    )
                    .child(
                        Button::new((if is_active_group { "act-menu" } else { "rec-menu" }, idx))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                let tx = tx_clone.clone();
                                let path_ren = action_file.clone();
                                let name_ren = cur_name.clone();
                                let view = view_entity.clone();
                                let menu = if !path_ren.is_empty() {
                                    menu.item(PopupMenuItem::new("Rename").on_click(
                                        move |_, window, cx| {
                                            view.update(cx, |app, cx| {
                                                app.start_rename(
                                                    path_ren.clone(),
                                                    name_ren.clone(),
                                                    window,
                                                    cx,
                                                );
                                            });
                                        },
                                    ))
                                } else {
                                    menu
                                };
                                if is_active_group {
                                    let sid = action_id.clone();
                                    menu.item(PopupMenuItem::new("Stop / Archive").on_click(
                                        move |_, _, _| {
                                            let _ = tx.send(DaemonCmd::MarkDone {
                                                session: sid.clone(),
                                            });
                                        },
                                    ))
                                } else {
                                    let path = action_file.clone();
                                    menu.item(PopupMenuItem::new("Delete session").on_click(
                                        move |_, _, _| {
                                            let _ = tx.send(DaemonCmd::DeleteSaved {
                                                session_path: path.clone(),
                                            });
                                        },
                                    ))
                                }
                            }),
                    )
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

    fn render_slash_popup(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let val = self.composer.read(cx).value();
        let trimmed = val.trim_start();
        if !trimmed.starts_with('/') || trimmed.contains(' ') {
            return None;
        }
        let prefix = &trimmed[1..];
        let matches = pa_client::commands::filter(prefix);
        if matches.is_empty() {
            return None;
        }
        let theme = cx.theme().clone();
        let mut list = v_flex()
            .w_full()
            .max_w(px(860.))
            .mx_auto()
            .mb_1()
            .p_1()
            .rounded(px(8.))
            .border_1()
            .border_color(theme.border)
            .bg(theme.popover)
            .gap_0p5();

        for (i, cmd) in matches.into_iter().take(6).enumerate() {
            let cmd_name = cmd.name;
            let cmd_desc = cmd.description;
            list = list.child(
                h_flex()
                    .id(("slash-opt", i))
                    .px_2()
                    .py_1()
                    .gap_3()
                    .rounded(px(5.))
                    .cursor_pointer()
                    .hover(|d| d.bg(theme.accent))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child(format!("/{cmd_name}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(cmd_desc),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.composer.update(cx, |input, cx| {
                            input.set_value(format!("/{cmd_name} "), window, cx);
                        });
                        cx.notify();
                    })),
            );
        }

        Some(list.into_any_element())
    }

    fn render_queue_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let total = self.queued_steer.len() + self.queued_follow_up.len();
        if total == 0 {
            return None;
        }
        let theme = cx.theme().clone();
        Some(
            h_flex()
                .w_full()
                .max_w(px(860.))
                .mx_auto()
                .px_3()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary.opacity(0.6))
                .justify_between()
                .items_center()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!(
                            "Queued: {} steer · {} follow-up",
                            self.queued_steer.len(),
                            self.queued_follow_up.len()
                        )),
                )
                .child(
                    Button::new("clear-queue")
                        .ghost()
                        .xsmall()
                        .label("Clear queue")
                        .on_click(cx.listener(|this, _, _, _| {
                            if let Some(sid) = this.active_session_id.clone() {
                                let _ = this
                                    .bridge
                                    .tx
                                    .send(DaemonCmd::ClearQueue { session: sid });
                            }
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_composer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let cur_model_label = self
            .active_session()
            .map(|s| {
                if !s.model_name.is_empty() {
                    s.model_name.clone()
                } else if !s.model.is_empty() {
                    s.model.clone()
                } else {
                    "Model".to_string()
                }
            })
            .unwrap_or_else(|| "Model".to_string());

        let cur_thinking = self
            .active_session()
            .map(|s| s.thinking.clone())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| "medium".to_string());

        let cur_model_id = self
            .active_session()
            .map(|s| s.model.clone())
            .unwrap_or_default();

        let thinking_levels: Vec<String> = self
            .models
            .iter()
            .find(|m| m.id == cur_model_id)
            .map(|m| m.thinking_levels.clone())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                vec![
                    "off".into(),
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "max".into(),
                ]
            });

        let configured_models: Vec<(String, String, String)> = self
            .models
            .iter()
            .filter(|m| m.configured)
            .take(25)
            .map(|m| (m.provider.clone(), m.id.clone(), m.name.clone()))
            .collect();

        let sid_for_model = self.active_session_id.clone();
        let tx_for_model = self.bridge.tx.clone();
        let sid_for_think = self.active_session_id.clone();
        let tx_for_think = self.bridge.tx.clone();

        v_flex()
            .w_full()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .gap_2()
            .children(self.render_queue_strip(cx))
            .children(self.render_slash_popup(cx))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(860.))
                    .mx_auto()
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_end()
                            .child(div().flex_1().child(Input::new(&self.composer)))
                            .when(self.is_streaming, |h| {
                                h.child(
                                    Button::new("steer-btn")
                                        .outline()
                                        .label("Steer")
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.send_current_prompt(true, window, cx);
                                        })),
                                )
                                .child(
                                    Button::new("abort-btn")
                                        .danger()
                                        .label("Stop")
                                        .on_click(cx.listener(|this, _, _, _| {
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this
                                                    .bridge
                                                    .tx
                                                    .send(DaemonCmd::Abort { session: sid });
                                            }
                                        })),
                                )
                            })
                            .child(
                                Button::new("send-btn")
                                    .primary()
                                    .label("Send")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send_current_prompt(false, window, cx);
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .items_center()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        Button::new("model-picker")
                                            .ghost()
                                            .xsmall()
                                            .label(cur_model_label)
                                            .icon(IconName::ChevronDown)
                                            .dropdown_menu_with_anchor(
                                                Anchor::BottomLeft,
                                                move |mut menu, _, _| {
                                                    for (prov, mid, mname) in &configured_models {
                                                        let tx = tx_for_model.clone();
                                                        let sid = sid_for_model.clone();
                                                        let p = prov.clone();
                                                        let id = mid.clone();
                                                        menu = menu.item(
                                                            PopupMenuItem::new(format!(
                                                                "{mname} ({prov})"
                                                            ))
                                                            .on_click(move |_, _, _| {
                                                                if let Some(s) = sid.clone() {
                                                                    let _ = tx.send(
                                                                        DaemonCmd::SetModel {
                                                                            session: s,
                                                                            provider: p.clone(),
                                                                            model_id: id.clone(),
                                                                        },
                                                                    );
                                                                }
                                                            }),
                                                        );
                                                    }
                                                    menu
                                                },
                                            ),
                                    )
                                    .child(
                                        Button::new("thinking-picker")
                                            .ghost()
                                            .xsmall()
                                            .label(format!("Thinking: {cur_thinking}"))
                                            .icon(IconName::ChevronDown)
                                            .dropdown_menu_with_anchor(
                                                Anchor::BottomLeft,
                                                move |mut menu, _, _| {
                                                    for lvl in &thinking_levels {
                                                        let tx = tx_for_think.clone();
                                                        let sid = sid_for_think.clone();
                                                        let level = lvl.clone();
                                                        menu = menu.item(
                                                            PopupMenuItem::new(level.clone())
                                                                .on_click(move |_, _, _| {
                                                                    if let Some(s) = sid.clone() {
                                                                        let _ = tx.send(
                                                                            DaemonCmd::SetThinking {
                                                                                session: s,
                                                                                level: level.clone(),
                                                                            },
                                                                        );
                                                                    }
                                                                }),
                                                        );
                                                    }
                                                    menu
                                                },
                                            ),
                                    ),
                            )
                            .when_some(self.notice.clone(), |row, note| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(note),
                                )
                            }),
                    ),
            )
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
            ItemKind::Assistant => div()
                .w_full()
                .py_1()
                .children(item.body_md.as_ref().map(|md| {
                    TextView::new(md)
                        .selectable(true)
                        .stream_fade(item.active)
                }))
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
                let copy_payload = item.raw_body.clone();
                let icon = match (item.kind, item.variant) {
                    (ItemKind::ToolCall, CardVariant::Python) => IconName::File,
                    (ItemKind::ToolCall, CardVariant::Bash) => IconName::SquareTerminal,
                    (ItemKind::ToolCall, _) => IconName::SquareTerminal,
                    (ItemKind::Thinking, _) => IconName::Info,
                    (_, CardVariant::BashDone) => IconName::SquareTerminal,
                    (_, CardVariant::AgentMessage) => IconName::Bot,
                    (_, CardVariant::ProviderRetry) => IconName::Loader,
                    _ => IconName::Inbox,
                };
                let icon_color = if item.active {
                    theme.warning
                } else if item.failed {
                    theme.danger
                } else {
                    theme.muted_foreground
                };

                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if item.active {
                        theme.warning.opacity(0.55)
                    } else {
                        theme.border
                    })
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
                            .child(Icon::new(icon).xsmall().text_color(icon_color))
                            .when(!item.verb.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .font_medium()
                                        .text_color(if item.active {
                                            theme.warning
                                        } else {
                                            theme.muted_foreground
                                        })
                                        .child(item.verb.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .text_xs()
                                    .text_color(if item.failed {
                                        theme.danger_foreground
                                    } else {
                                        theme.foreground
                                    })
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
                                        .text_color(if item.failed {
                                            theme.danger_foreground
                                        } else {
                                            theme.muted_foreground
                                        })
                                        .child(item.meta.clone()),
                                )
                            })
                            .when(
                                item.kind == ItemKind::ToolCall && !item.raw_body.is_empty(),
                                |row| {
                                    row.child(
                                        Button::new(("copy-btn", id))
                                            .ghost()
                                            .xsmall()
                                            .label("Copy")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.copy_text(copy_payload.clone(), cx);
                                            })),
                                    )
                                },
                            )
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
        let mut rendered_items = Vec::with_capacity(self.items.len());
        for it in &self.items {
            rendered_items.push(self.render_item(it, cx));
        }

        v_flex()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(self.render_titlebar(cx))
            .child(
                h_flex()
                    .flex_1()
                    .w_full()
                    .min_h_0()
                    .when(self.sidebar_open, |row| row.child(self.render_sidebar(cx)))
                    .child(
                        v_flex()
                            .flex_1()
                            .h_full()
                            .min_w_0()
                            .child(
                                div()
                                    .id("transcript-scroll")
                                    .flex_1()
                                    .w_full()
                                    .track_scroll(&self.scroll_handle)
                                    .overflow_y_scrollbar()
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
                            .child(self.render_composer_bar(cx)),
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
