use crate::assets::{
    ICON_BASH_PATH, ICON_HEARTBEAT_PATH, ICON_PYTHON_PATH, ICON_SEND_PATH, ICON_SHELLS_PATH,
    ICON_SUBAGENTS_PATH, ICON_THINKING_PATH, PRIME_AGENT_MARK_DARK_PATH,
    PRIME_AGENT_MARK_LIGHT_PATH,
};
use crate::daemon::{DaemonBridge, DaemonCmd, DaemonEvent};
use crate::settings::AppSettings;
use crate::theme::PaPalette;
use crate::transcript::{
    append_text_delta, append_thinking_delta, build_items, finish_tool_call, format_expanded_body,
    settle_all, start_tool_call, CardVariant, ItemKind, TranscriptItem,
};
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState},
    v_flex, Icon, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode, TitleBar,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{ModelEntry, SessionRow, SessionStats};
use serde_json::Value;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarGroup {
    Active,
    Idle,
    Done,
}

#[derive(Clone, PartialEq, Eq)]
enum OpenPopup {
    None,
    ModelMenu,
    SettingsMenu,
    ContextCard,
    QueuePanel,
    SessionMenu {
        key: String,
        id: String,
        file: String,
        name: String,
        is_resident: bool,
    },
}

fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let ra: Rgba = a.into();
    let rb: Rgba = b.into();
    Rgba {
        r: ra.r + (rb.r - ra.r) * t,
        g: ra.g + (rb.g - ra.g) * t,
        b: ra.b + (rb.b - ra.b) * t,
        a: ra.a + (rb.a - ra.a) * t,
    }
    .into()
}

fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

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
    creating_session: bool,
    rail_mark_hovered: bool,
    // Smooth Send <-> Stop morph state (0.0 = Send arrow, 1.0 = Stop square)
    stop_mix: f32,
    stop_target: f32,
    last_anim_tick: Instant,
    // Interactive composer drag-resize state
    composer_height: Option<f32>,
    dragging_composer: bool,
    drag_start_y: f32,
    drag_start_height: f32,
    handle_hovered: bool,
    settings: AppSettings,
    subagent_count: usize,
    shell_count: usize,
    heartbeat_count: usize,
    search_open: bool,
    find_open: bool,
    find_index: usize,
    open_popup: OpenPopup,
    list_state: ListState,
    search_input: Entity<InputState>,
    find_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    composer: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
    _daemon_task: Task<()>,
}

impl PaApp {
    pub fn new(settings: AppSettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (bridge, ev_rx) = DaemonBridge::spawn();
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in transcript…"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New session name…"));
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask anything… (/ for commands · Shift+Enter for newline)")
                .soft_wrap(true)
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });

        let sub_composer = cx.subscribe_in(&composer, window, |this, _, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter {
                    secondary,
                    shift: false,
                } => {
                    // Plain Enter sends (or steers if streaming); Ctrl+Enter queues follow-up when streaming.
                    let steer = this.is_streaming && !*secondary;
                    this.send_current_prompt(steer, window, cx);
                }
                InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {
                    this.sync_stop_target(cx);
                    cx.notify();
                }
                _ => {}
            }
        });

        let sub_search = cx.subscribe_in(&search_input, window, |_this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change | InputEvent::Focus | InputEvent::Blur) {
                cx.notify();
            }
        });

        let sub_find = cx.subscribe_in(&find_input, window, |this, _, ev: &InputEvent, _, cx| {
            match ev {
                InputEvent::Change => {
                    this.find_index = 0;
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => {
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.step_find(1, cx);
                }
            }
        });

        let sub_rename = cx.subscribe_in(&rename_input, window, |this, _, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => {
                    this.commit_rename(window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {
                    cx.notify();
                }
                _ => {}
            }
        });

        let daemon_task = cx.spawn(async move |weak_self, cx| {
            while let Ok(first) = ev_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(ev) = ev_rx.try_recv() {
                    batch.push(ev);
                }
                let keep_going = weak_self
                    .update(cx, |this, cx| {
                        for ev in batch {
                            this.handle_daemon_event(ev, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        });

        // Top alignment keeps short transcripts anchored to the top of the pane while
        // FollowMode::Tail pins overflowing and streaming transcripts to the bottom.
        let list_state = ListState::new(0, ListAlignment::Top, px(300.));
        list_state.set_follow_mode(FollowMode::Tail);

        Self {
            bridge,
            status: "Connecting…".into(),
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
            creating_session: false,
            rail_mark_hovered: false,
            stop_mix: 0.0,
            stop_target: 0.0,
            last_anim_tick: Instant::now(),
            composer_height: None,
            dragging_composer: false,
            drag_start_y: 0.0,
            drag_start_height: 54.0,
            handle_hovered: false,
            settings,
            subagent_count: 0,
            shell_count: 0,
            heartbeat_count: 0,
            search_open: false,
            find_open: false,
            find_index: 0,
            open_popup: OpenPopup::None,
            list_state,
            search_input,
            find_input,
            rename_input,
            composer,
            _subscriptions: vec![sub_composer, sub_search, sub_find, sub_rename],
            _daemon_task: daemon_task,
        }
    }

    fn palette(&self) -> PaPalette {
        PaPalette::for_mode(self.settings.dark_mode)
    }

    fn sync_stop_target(&mut self, cx: &Context<Self>) {
        let has_text = !self.composer.read(cx).value().trim().is_empty();
        let next = if self.is_streaming && !has_text {
            1.0
        } else {
            0.0
        };
        if next != self.stop_target {
            self.stop_target = next;
            self.last_anim_tick = Instant::now();
        }
    }

    fn advance_stop_anim(&mut self, window: &Window) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_anim_tick)
            .as_secs_f32();
        self.last_anim_tick = now;
        if (self.stop_mix - self.stop_target).abs() > 0.002 {
            let step = (dt / 0.22).min(0.25);
            if self.stop_mix < self.stop_target {
                self.stop_mix = (self.stop_mix + step).min(self.stop_target);
            } else {
                self.stop_mix = (self.stop_mix - step).max(self.stop_target);
            }
            window.request_animation_frame();
        } else if self.stop_mix != self.stop_target {
            self.stop_mix = self.stop_target;
        }
    }

    fn request_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating_session {
            return;
        }
        self.open_popup = OpenPopup::None;
        self.creating_session = true;
        self.is_streaming = false;
        self.items.clear();
        self.list_state.reset(0);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.stats = None;
        self.queued_steer.clear();
        self.queued_follow_up.clear();
        self.subagent_count = 0;
        self.shell_count = 0;
        self.heartbeat_count = 0;
        self.notice = Some("Starting new session…".into());
        self.sync_stop_target(cx);
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        let _ = self.bridge.tx.send(DaemonCmd::CreateSession);
        cx.notify();
    }

    fn is_scrolled_up(&self) -> bool {
        if self.items.is_empty() || self.list_state.is_following_tail() {
            return false;
        }
        let max_y = self.list_state.max_offset_for_scrollbar().y;
        if max_y <= px(4.) {
            return false;
        }
        let cur_y = -self.list_state.scroll_px_offset_for_scrollbar().y;
        (max_y - cur_y) > px(80.)
    }

    fn sync_list_after_tail_update(&self, old_len: usize) {
        let new_len = self.items.len();
        if new_len > old_len {
            self.list_state.splice(old_len..old_len, new_len - old_len);
        } else if new_len > 0 {
            self.list_state.remeasure_items(new_len - 1..new_len);
        }
    }

    fn handle_daemon_event(&mut self, ev: DaemonEvent, cx: &mut Context<Self>) {
        match ev {
            DaemonEvent::Connected => {
                self.status = "Connected".into();
            }
            DaemonEvent::Disconnected(reason) => {
                self.status = reason.into();
                self.is_streaming = false;
                self.creating_session = false;
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
            }
            DaemonEvent::SessionCreated(new_sid) => {
                self.creating_session = false;
                self.active_session_id = Some(new_sid);
                self.is_streaming = false;
                self.items.clear();
                self.list_state.reset(0);
                self.list_state.set_follow_mode(FollowMode::Tail);
                self.notice = None;
                self.sync_stop_target(cx);
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
                self.sync_stop_target(cx);
            }
            DaemonEvent::Messages { session, messages } => {
                let switching = self.active_session_id.as_deref() != Some(session.as_str());
                let follow_bottom = switching || !self.is_scrolled_up();
                let prev_anchor = self.list_state.logical_scroll_top();
                self.active_session_id = Some(session);
                self.creating_session = false;
                self.items = build_items(&messages, cx);
                self.list_state
                    .reset_with_uniform_height(self.items.len(), px(48.));
                if follow_bottom {
                    self.list_state.set_follow_mode(FollowMode::Tail);
                } else {
                    self.list_state.scroll_to(prev_anchor);
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
                    if self.queued_steer.is_empty()
                        && self.queued_follow_up.is_empty()
                        && self.open_popup == OpenPopup::QueuePanel
                    {
                        self.open_popup = OpenPopup::None;
                    }
                }
            }
            DaemonEvent::Activity {
                session,
                subagents,
                shells,
                heartbeats,
            } => {
                if self.active_session_id.as_deref() == Some(session.as_str()) {
                    self.subagent_count = subagents;
                    self.shell_count = shells;
                    self.heartbeat_count = heartbeats;
                }
            }
            DaemonEvent::Notice(msg) => {
                self.notice = Some(msg.into());
            }
            DaemonEvent::LiveFrame(frame) => {
                self.handle_live_frame(&frame, cx);
            }
            DaemonEvent::Error(err) => {
                self.creating_session = false;
                self.status = format!("Error: {err}").into();
                self.notice = Some(format!("Error: {err}").into());
            }
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
                self.sync_stop_target(cx);
            }
            "message_update" => {
                self.is_streaming = true;
                self.sync_stop_target(cx);
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
                        let old_len = self.items.len();
                        append_text_delta(&mut self.items, delta, cx);
                        self.sync_list_after_tail_update(old_len);
                    } else if delta_kind.starts_with("thinking_delta") {
                        let old_len = self.items.len();
                        append_thinking_delta(&mut self.items, delta, cx);
                        self.sync_list_after_tail_update(old_len);
                    }
                }
            }
            "tool_execution_start" => {
                self.is_streaming = true;
                self.sync_stop_target(cx);
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
                let old_len = self.items.len();
                start_tool_call(&mut self.items, cid, name, args);
                self.sync_list_after_tail_update(old_len);
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
                let details = event
                    .get("result")
                    .and_then(|r| r.get("details"))
                    .or_else(|| event.get("details"));
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
                if let Some(ix) = finish_tool_call(&mut self.items, cid, out, is_err, details, cx) {
                    self.list_state.remeasure_items(ix..ix + 1);
                }
            }
            "agent_end" | "turn_end" => {
                self.is_streaming = false;
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
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
        self.open_popup = OpenPopup::None;
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let Some(sid) = self.active_session_id.clone() else {
            return;
        };

        if !self.is_streaming {
            let old_len = self.items.len();
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
                images: Vec::new(),
                expanded: true,
                active: false,
                failed: false,
            });
            self.list_state.splice(old_len..old_len, 1);
            self.list_state.set_follow_mode(FollowMode::Tail);
        }

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
        self.sync_stop_target(cx);
        cx.notify();
    }

    fn start_rename(
        &mut self,
        path: String,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_popup = OpenPopup::None;
        self.renaming_path = Some(path);
        self.rename_input.update(cx, |input, cx| {
            input.set_value(current_name, window, cx);
        });
        let focus = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
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

    fn toggle_group(&mut self, group: SidebarGroup, cx: &mut Context<Self>) {
        match group {
            SidebarGroup::Active => self.settings.active_closed = !self.settings.active_closed,
            SidebarGroup::Idle => self.settings.idle_closed = !self.settings.idle_closed,
            SidebarGroup::Done => self.settings.done_closed = !self.settings.done_closed,
        }
        self.settings.save();
        cx.notify();
    }

    fn toggle_item(&mut self, id: usize, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        if let Some((ix, item)) = self
            .items
            .iter_mut()
            .enumerate()
            .find(|(_, i)| i.id == id)
        {
            item.expanded = !item.expanded;
            if item.expanded && item.body_md.is_none() && !item.raw_body.is_empty() {
                let formatted = format_expanded_body(item);
                item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
            }
            self.list_state.remeasure_items(ix..ix + 1);
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

    fn find_matches(&self, cx: &Context<Self>) -> Vec<usize> {
        let q = self.find_input.read(cx).value().trim().to_ascii_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|it| {
                it.raw_body.to_ascii_lowercase().contains(&q)
                    || it.title.to_ascii_lowercase().contains(&q)
                    || it.output
                        .as_deref()
                        .is_some_and(|o| o.to_ascii_lowercase().contains(&q))
            })
            .map(|it| it.id)
            .collect()
    }

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        let matches = self.find_matches(cx);
        if matches.is_empty() {
            self.find_index = 0;
            return;
        }
        let len = matches.len() as isize;
        self.find_index = ((self.find_index as isize + delta).rem_euclid(len)) as usize;
        if let Some(ix) = self
            .items
            .iter()
            .position(|it| it.id == matches[self.find_index])
        {
            self.list_state.scroll_to_reveal_item(ix);
        }
        cx.notify();
    }

    fn render_prime_mark(&self) -> impl IntoElement {
        let path = if self.settings.dark_mode {
            PRIME_AGENT_MARK_DARK_PATH
        } else {
            PRIME_AGENT_MARK_LIGHT_PATH
        };
        img(path).size(px(18.)).object_fit(ObjectFit::Contain)
    }

    fn render_slim_input(
        &self,
        state: &Entity<InputState>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let p = self.palette();
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        div()
            .w_full()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(7.))
            .border_1()
            .border_color(if focused {
                p.composer_edge_focus
            } else {
                p.composer_edge
            })
            .bg(p.composer_surface)
            .child(
                Input::new(state)
                    .xsmall()
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
            )
    }

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let settings_open = self.open_popup == OpenPopup::SettingsMenu;

        TitleBar::new()
            .h(px(32.))
            .bg(p.window_surface)
            .border_b_0()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .pr(px(8.))
                    .child(
                        div()
                            .id("tb-find")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(self.find_open, |d| d.bg(p.sidebar_row_selected))
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(Icon::new(IconName::Search).size(px(14.)).text_color(p.quiet_ink))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.find_open = !this.find_open;
                                if this.find_open {
                                    let focus = this.find_input.read(cx).focus_handle(cx);
                                    window.focus(&focus, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("tb-settings")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(settings_open, |d| d.bg(p.sidebar_row_selected))
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(Icon::new(IconName::Settings).size(px(14.)).text_color(p.quiet_ink))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_popup = if this.open_popup == OpenPopup::SettingsMenu {
                                    OpenPopup::None
                                } else {
                                    OpenPopup::SettingsMenu
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("tb-right-col")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(self.settings.right_open, |d| d.bg(p.sidebar_row_selected))
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(
                                Icon::new(if self.settings.right_open {
                                    IconName::PanelRightClose
                                } else {
                                    IconName::PanelRightOpen
                                })
                                .size(px(14.))
                                .text_color(p.quiet_ink),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.settings.right_open = !this.settings.right_open;
                                this.settings.save();
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_collapsed_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        v_flex()
            .w(px(44.))
            .h_full()
            .bg(p.sidebar_surface)
            .pt(px(4.))
            .gap(px(6.))
            .items_center()
            .child(
                div()
                    .id("rail-open-sidebar")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.rail_mark_hovered = *hovered;
                        cx.notify();
                    }))
                    .child(if self.rail_mark_hovered {
                        Icon::new(IconName::PanelLeftOpen)
                            .size(px(16.))
                            .text_color(p.ink)
                            .into_any_element()
                    } else {
                        self.render_prime_mark().into_any_element()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.settings.sidebar_open = true;
                        this.rail_mark_hovered = false;
                        this.settings.save();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("rail-new-session")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(Icon::new(IconName::Plus).size(px(15.)).text_color(p.quiet_ink))
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_new_session(window, cx);
                    })),
            )
            .child(
                div()
                    .id("rail-search")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(Icon::new(IconName::Search).size(px(14.)).text_color(p.quiet_ink))
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.settings.sidebar_open = true;
                        this.search_open = true;
                        this.settings.save();
                        let focus = this.search_input.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                        cx.notify();
                    })),
            )
    }

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.settings.sidebar_open {
            return self.render_collapsed_rail(cx).into_any_element();
        }
        let p = self.palette();
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
            .filter(|s| s.roster_status() == "Running")
            .filter(matches_query)
            .collect();
        let idle_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| s.roster_status() == "Idle")
            .filter(matches_query)
            .collect();
        let done_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(|s| !matches!(s.roster_status(), "Running" | "Idle"))
            .filter(matches_query)
            .take(60)
            .collect();

        v_flex()
            .w(px(244.))
            .h_full()
            .bg(p.sidebar_surface)
            .px(px(10.))
            .pt(px(2.))
            .pb(px(8.))
            .gap(px(6.))
            .child(
                h_flex()
                    .h(px(36.))
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .pl(px(6.))
                            .gap(px(8.))
                            .items_center()
                            .child(self.render_prime_mark())
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_semibold()
                                    .text_color(p.ink)
                                    .child("Prime Agent"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .items_center()
                            .child(
                                div()
                                    .id("sb-search-btn")
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .when(self.search_open, |d| d.bg(p.sidebar_row_selected))
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(Icon::new(IconName::Search).size(px(14.)).text_color(p.quiet_ink))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.search_open = !this.search_open;
                                        if this.search_open {
                                            let focus = this.search_input.read(cx).focus_handle(cx);
                                            window.focus(&focus, cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("sb-collapse-btn")
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        Icon::new(IconName::PanelLeftClose)
                                            .size(px(14.))
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.settings.sidebar_open = false;
                                        this.rail_mark_hovered = false;
                                        this.settings.save();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .id("sb-new-session")
                    .h(px(34.))
                    .w_full()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(p.sidebar_new_edge)
                    .bg(p.sidebar_new_fill)
                    .hover(|d| d.bg(p.sidebar_new_fill_hover))
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Plus).size(px(14.)).text_color(p.ink))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(p.ink)
                            .child(if self.creating_session {
                                "Creating…"
                            } else {
                                "New Session"
                            }),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_new_session(window, cx);
                    })),
            )
            .when(self.search_open, |col| {
                col.child(self.render_slim_input(&self.search_input, window, cx))
            })
            .when(self.renaming_path.is_some(), |col| {
                col.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(self.render_slim_input(&self.rename_input, window, cx)),
                        )
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
                            .gap(px(8.))
                            .pt(px(4.))
                            .child(self.render_session_group(
                                SidebarGroup::Active,
                                "ACTIVE",
                                &active_rows,
                                self.settings.active_closed,
                                cx,
                            ))
                            .child(self.render_session_group(
                                SidebarGroup::Idle,
                                "IDLE",
                                &idle_rows,
                                self.settings.idle_closed,
                                cx,
                            ))
                            .child(self.render_session_group(
                                SidebarGroup::Done,
                                "DONE",
                                &done_rows,
                                self.settings.done_closed,
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    fn render_session_group(
        &self,
        group_kind: SidebarGroup,
        title: &'static str,
        rows: &[&SessionRow],
        closed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let empty = rows.is_empty();
        let mut card = v_flex()
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(if empty {
                p.sidebar_group_empty_edge
            } else {
                p.sidebar_group_edge
            })
            .bg(if empty {
                gpui_kit::transparent_black()
            } else {
                p.sidebar_group_surface
            })
            .p(px(4.))
            .gap(px(2.))
            .child(
                h_flex()
                    .id(SharedString::from(format!("grp-hdr-{title}")))
                    .h(px(26.))
                    .w_full()
                    .px(px(6.))
                    .rounded(px(6.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_group(group_kind, cx);
                    }))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                Icon::new(if closed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(px(11.))
                                .text_color(if empty {
                                    p.sidebar_group_empty_ink
                                } else {
                                    p.sidebar_group_ink
                                }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_semibold()
                                    .text_color(if empty {
                                        p.sidebar_group_empty_ink
                                    } else {
                                        p.sidebar_group_ink
                                    })
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_medium()
                            .text_color(if empty {
                                p.sidebar_group_empty_ink
                            } else {
                                p.sidebar_group_ink
                            })
                            .child(rows.len().to_string()),
                    ),
            );

        if closed || empty {
            return card;
        }

        for (idx, row) in rows.iter().enumerate() {
            let key = session_key(row);
            let selected = self.active_session_id.as_deref() == Some(key.as_str());
            let label = {
                let single: String = row
                    .name
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if !single.is_empty() {
                    single
                } else {
                    key.chars().take(12).collect()
                }
            };
            let click_key = key.clone();
            let menu_key = key.clone();
            let menu_id = row.id.clone();
            let menu_file = row.session_file.clone();
            let menu_name = label.clone();
            let is_resident = row.is_resident();
            let dot_color = match group_kind {
                SidebarGroup::Active => Some(p.working),
                SidebarGroup::Idle => Some(p.tag_idle),
                SidebarGroup::Done => None,
            };

            card = card.child(
                h_flex()
                    .id((title, idx))
                    .h(px(28.))
                    .px(px(7.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(p.sidebar_row_selected))
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .when_some(dot_color, |d, c| {
                        d.child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(c))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .text_color(if selected { p.ink } else { p.quiet_ink })
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("sess-dots-{title}-{idx}")))
                            .size(px(22.))
                            .flex_shrink_0()
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(p.sidebar_row_selected))
                            .child(
                                Icon::new(IconName::Ellipsis)
                                    .size(px(12.))
                                    .text_color(p.sidebar_group_empty_ink),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let next = OpenPopup::SessionMenu {
                                    key: menu_key.clone(),
                                    id: menu_id.clone(),
                                    file: menu_file.clone(),
                                    name: menu_name.clone(),
                                    is_resident,
                                };
                                this.open_popup = if this.open_popup == next {
                                    OpenPopup::None
                                } else {
                                    next
                                };
                                cx.notify();
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_popup = OpenPopup::None;
                        let _ = this
                            .bridge
                            .tx
                            .send(DaemonCmd::SelectSession(click_key.clone()));
                        cx.notify();
                    })),
            );
        }

        card
    }

    fn render_find_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open {
            return None;
        }
        let p = self.palette();
        let matches = self.find_matches(cx);
        let count_label = if matches.is_empty() {
            "0 matches".to_string()
        } else {
            format!("{}/{}", (self.find_index % matches.len()) + 1, matches.len())
        };
        Some(
            h_flex()
                .w_full()
                .px_5()
                .py_1p5()
                .border_b_1()
                .border_color(p.transcript_cell_edge)
                .bg(p.transcript_cell_fill)
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(px(260.))
                        .child(self.render_slim_input(&self.find_input, window, cx)),
                )
                .child(div().text_xs().text_color(p.quiet_ink).child(count_label))
                .child(
                    Button::new("find-prev")
                        .ghost()
                        .xsmall()
                        .label("Prev")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .label("Next")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                )
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_activity_chip(&self) -> impl IntoElement {
        let p = self.palette();
        let metric = |icon_path: &'static str, count: usize| {
            let ink = if count > 0 { p.working } else { p.quiet_ink };
            h_flex()
                .h_full()
                .items_center()
                .gap(px(4.))
                .child(svg().path(icon_path).size(px(13.)).text_color(ink))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(13.))
                        .text_color(ink)
                        .child(count.to_string()),
                )
        };

        h_flex()
            .h(px(24.))
            .px(px(8.))
            .gap(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(p.composer_edge)
            .bg(p.sidebar_group_surface)
            .items_center()
            .child(metric(ICON_SUBAGENTS_PATH, self.subagent_count))
            .child(metric(ICON_HEARTBEAT_PATH, self.heartbeat_count))
            .child(metric(ICON_SHELLS_PATH, self.shell_count))
    }

    fn render_send_stop_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let mix = ease_in_out_cubic(self.stop_mix);
        let bg_color = lerp_hsla(p.composer_send_fill, p.danger, mix);
        let is_stop = self.stop_target > 0.5;

        // Exact pixel-centered 26x26 circle:
        // - Arrow is 12x12 -> (26 - 12)/2 = 7px exact integer offset
        // - Stop square is 8x8 -> (26 - 8)/2 = 9px exact integer offset
        let arrow_size = 12.0 * (1.0 - 0.35 * mix);
        let arrow_opacity = (1.0 - mix).clamp(0.0, 1.0);
        let square_size = 4.0 + 4.0 * mix;
        let square_radius = 3.0 - 1.5 * mix;
        let square_opacity = mix.clamp(0.0, 1.0);

        div()
            .id("send-stop-circle")
            .relative()
            .size(px(26.))
            .flex_shrink_0()
            .rounded_full()
            .bg(bg_color)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(arrow_opacity > 0.01, |btn| {
                btn.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(arrow_opacity)
                        .child(
                            svg()
                                .path(ICON_SEND_PATH)
                                .size(px(arrow_size))
                                .text_color(p.composer_send_mark),
                        ),
                )
            })
            .when(square_opacity > 0.01, |btn| {
                btn.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(square_opacity)
                        .child(
                            div()
                                .size(px(square_size))
                                .rounded(px(square_radius))
                                .bg(rgb(0xffffff)),
                        ),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if is_stop {
                    if let Some(sid) = this.active_session_id.clone() {
                        let _ = this.bridge.tx.send(DaemonCmd::Abort { session: sid });
                    }
                } else {
                    let steer = this.is_streaming;
                    this.send_current_prompt(steer, window, cx);
                }
            }))
    }

    fn render_composer_band(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let composer_focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let queued_total = self.queued_steer.len() + self.queued_follow_up.len();
        let model_chip_label = self
            .active_session()
            .map(|s| {
                let m = if !s.model_name.is_empty() {
                    s.model_name.as_str()
                } else if !s.model.is_empty() {
                    s.model.as_str()
                } else {
                    "Model"
                };
                if s.thinking.is_empty() {
                    m.to_string()
                } else {
                    format!("{m} · {}", s.thinking)
                }
            })
            .unwrap_or_else(|| "Model".to_string());

        let ctx_pct = self
            .stats
            .as_ref()
            .filter(|st| st.context_window > 0)
            .map(|st| format!("{:.0}%", st.percent))
            .unwrap_or_else(|| "0%".to_string());

        let handle_color = if self.dragging_composer {
            p.working
        } else if self.handle_hovered {
            p.quiet_ink
        } else {
            p.composer_handle
        };

        v_flex()
            .w_full()
            .px(px(18.))
            .pb(px(12.))
            .pt(px(2.))
            .child(
                v_flex()
                    .w_full()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(if composer_focused {
                        p.composer_edge_focus
                    } else {
                        p.composer_edge
                    })
                    .bg(p.composer_surface)
                    // Interactive resize grab handle at top center
                    .child(
                        div()
                            .id("composer-resize-handle")
                            .w_full()
                            .h(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_ns_resize()
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.handle_hovered = *hovered;
                                cx.notify();
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if ev.click_count >= 2 {
                                        this.composer_height = None;
                                        this.dragging_composer = false;
                                    } else {
                                        this.dragging_composer = true;
                                        this.drag_start_y = ev.position.y.as_f32();
                                        this.drag_start_height =
                                            this.composer_height.unwrap_or(56.0);
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .w(px(36.))
                                    .h(px(3.))
                                    .rounded(px(1.5))
                                    .bg(handle_color),
                            ),
                    )
                    // Multi-line composer textarea (supports manual drag height OR auto-grow)
                    .child(
                        div()
                            .w_full()
                            .px(px(10.))
                            .pb(px(6.))
                            .child(match self.composer_height {
                                Some(h) => Textarea::new(&self.composer)
                                    .h(px(h))
                                    .appearance(false)
                                    .bordered(false),
                                None => Textarea::new(&self.composer)
                                    .appearance(false)
                                    .bordered(false),
                            }),
                    )
                    // Pixel-aligned bottom dock: all controls share centered 24px/26px row alignment
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(34.))
                            .px(px(12.))
                            .pb(px(8.))
                            .gap(px(8.))
                            .items_center()
                            .child(self.render_activity_chip())
                            .when(queued_total > 0, |row| {
                                row.child(
                                    h_flex()
                                        .id("queue-chip-btn")
                                        .h(px(24.))
                                        .px(px(8.))
                                        .gap(px(5.))
                                        .rounded(px(6.))
                                        .border_1()
                                        .border_color(if self.open_popup == OpenPopup::QueuePanel {
                                            p.working
                                        } else {
                                            p.composer_edge
                                        })
                                        .bg(p.sidebar_group_surface)
                                        .items_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.sidebar_row_hover))
                                        .child(
                                            div()
                                                .size(px(6.))
                                                .rounded_full()
                                                .bg(p.working),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .line_height(px(13.))
                                                .text_color(p.ink)
                                                .child(format!("{queued_total} queued")),
                                        )
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.open_popup =
                                                if this.open_popup == OpenPopup::QueuePanel {
                                                    OpenPopup::None
                                                } else {
                                                    OpenPopup::QueuePanel
                                                };
                                            cx.notify();
                                        })),
                                )
                            })
                            .when_some(self.notice.clone(), |row, note| {
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.quiet_ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(note),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("ctx-ring-btn")
                                    .h(px(24.))
                                    .px(px(8.))
                                    .rounded(px(12.))
                                    .border_1()
                                    .border_color(if self.open_popup == OpenPopup::ContextCard {
                                        p.working
                                    } else {
                                        p.composer_edge
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .text_size(px(11.))
                                    .line_height(px(13.))
                                    .text_color(p.quiet_ink)
                                    .child(ctx_pct)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.open_popup =
                                            if this.open_popup == OpenPopup::ContextCard {
                                                OpenPopup::None
                                            } else {
                                                OpenPopup::ContextCard
                                            };
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("attach-btn")
                                    .size(px(24.))
                                    .rounded(px(6.))
                                    .bg(p.composer_attach_fill)
                                    .hover(|d| d.bg(p.sidebar_row_selected))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(Icon::new(IconName::Plus).size(px(12.)).text_color(p.quiet_ink))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.notice = Some("Paste or drop images in composer".into());
                                        cx.notify();
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("model-chip-btn")
                                    .h(px(24.))
                                    .px(px(8.))
                                    .gap(px(5.))
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(if self.open_popup == OpenPopup::ModelMenu {
                                        p.working
                                    } else {
                                        p.composer_edge
                                    })
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .line_height(px(13.))
                                            .text_color(p.quiet_ink)
                                            .child(model_chip_label),
                                    )
                                    .child(
                                        Icon::new(IconName::ChevronDown)
                                            .size(px(11.))
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.open_popup =
                                            if this.open_popup == OpenPopup::ModelMenu {
                                                OpenPopup::None
                                            } else {
                                                OpenPopup::ModelMenu
                                            };
                                        cx.notify();
                                    })),
                            )
                            .child(self.render_send_stop_button(cx)),
                    ),
            )
    }

    fn render_context_card_content(&self) -> impl IntoElement {
        let p = self.palette();
        let (used_str, pct_str, fill, msgs, tools, inp, outp, cached, cost) = match &self.stats {
            Some(st) => {
                let fill = if st.context_window > 0 {
                    (st.context_tokens as f32 / st.context_window as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (
                    format!("{} / {}", st.context_tokens, st.context_window),
                    format!("{:.0}%", st.percent),
                    fill,
                    st.messages.to_string(),
                    st.tool_calls.to_string(),
                    st.input_tokens.to_string(),
                    st.output_tokens.to_string(),
                    st.cache_read.to_string(),
                    format!("${:.4}", st.cost),
                )
            }
            None => (
                "—".into(),
                "0%".into(),
                0.0,
                "0".into(),
                "0".into(),
                "0".into(),
                "0".into(),
                "0".into(),
                "$0.0000".into(),
            ),
        };

        let ledger = |label: &'static str, value: String| {
            h_flex()
                .w_full()
                .justify_between()
                .child(div().text_size(px(11.)).text_color(p.quiet_ink).child(label))
                .child(div().text_size(px(11.)).text_color(p.ink).child(value))
        };

        v_flex()
            .w_full()
            .gap(px(6.))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(10.5)).text_color(p.quiet_ink).child("context"))
                    .child(div().text_size(px(12.)).font_semibold().text_color(p.ink).child(pct_str)),
            )
            .child(
                div()
                    .w_full()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(p.panel_edge)
                    .child(
                        div()
                            .h_full()
                            .w(relative(fill))
                            .rounded(px(2.))
                            .bg(p.working),
                    ),
            )
            .child(div().text_size(px(10.5)).text_color(p.quiet_ink).child(used_str))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("messages", msgs))
            .child(ledger("tool calls", tools))
            .child(ledger("input", inp))
            .child(ledger("output", outp))
            .child(ledger("cache read", cached))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("session cost", cost))
    }

    fn render_slash_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let val = self.composer.read(cx).value();
        let trimmed = val.trim_start();
        if !trimmed.starts_with('/') || trimmed.contains(' ') || trimmed.contains('\n') {
            return None;
        }
        let prefix = &trimmed[1..];
        let matches = pa_client::commands::filter(prefix);
        if matches.is_empty() {
            return None;
        }
        let p = self.palette();
        let left_offset = if self.settings.sidebar_open {
            px(262.)
        } else {
            px(62.)
        };
        let mut list = v_flex()
            .absolute()
            .bottom(px(112.))
            .left(left_offset)
            .w(px(420.))
            .p_1p5()
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_lg()
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
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(p.ink)
                            .child(format!("/{cmd_name}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(p.quiet_ink)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(cmd_desc),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.composer.update(cx, |input, cx| {
                            input.set_value(format!("/{cmd_name} "), window, cx);
                        });
                        cx.notify();
                    })),
            );
        }

        Some(list.into_any_element())
    }

    fn render_right_column(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings.right_open {
            return None;
        }
        let p = self.palette();
        let active_name = self
            .active_session()
            .map(|s| {
                if !s.name.trim().is_empty() {
                    s.name.clone()
                } else {
                    session_key(s)
                }
            })
            .unwrap_or_else(|| "No session".to_string());

        Some(
            v_flex()
                .w(px(300.))
                .h_full()
                .bg(p.window_surface)
                .px_4()
                .py_3()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(p.quiet_ink)
                        .child("SESSION"),
                )
                .child(div().text_sm().font_semibold().text_color(p.ink).child(active_name))
                .child(
                    div()
                        .p_3()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .bg(p.sidebar_group_surface)
                        .child(self.render_context_card_content()),
                )
                .when(self.active_session_id.is_some(), |col| {
                    col.child(
                        h_flex()
                            .id("right-export")
                            .h(px(30.))
                            .px_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.sidebar_new_fill)
                            .hover(|d| d.bg(p.sidebar_new_fill_hover))
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(p.ink)
                            .child("Export HTML")
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
                .child(div().flex_1())
                .child(div().text_xs().text_color(p.quiet_ink).child(self.status.clone()))
                .into_any_element(),
        )
    }

    fn render_floating_popups(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette();
        let right_offset = if self.settings.right_open {
            px(322.)
        } else {
            px(24.)
        };
        let left_offset = if self.settings.sidebar_open {
            px(262.)
        } else {
            px(62.)
        };

        match &self.open_popup {
            OpenPopup::None => self.render_slash_overlay(cx),
            OpenPopup::QueuePanel => {
                let mut col = v_flex().gap_2();
                if !self.queued_steer.is_empty() {
                    col = col.child(
                        div()
                            .text_size(px(10.5))
                            .font_semibold()
                            .text_color(p.quiet_ink)
                            .child("STEERING (NEXT STEP)"),
                    );
                    for (idx, txt) in self.queued_steer.iter().enumerate() {
                        let exp = txt.clone();
                        col = col.child(
                            h_flex()
                                .id(("q-steer", idx))
                                .px_2()
                                .py_1()
                                .rounded(px(6.))
                                .bg(p.sidebar_group_surface)
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .text_color(p.ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(txt.clone()),
                                )
                                .child(
                                    div()
                                        .id(("q-steer-del", idx))
                                        .size(px(20.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.danger_surface))
                                        .child(Icon::new(IconName::Close).size(px(11.)).text_color(p.quiet_ink))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                    session: sid,
                                                    lane: "steering".into(),
                                                    slot: idx,
                                                    expected: exp.clone(),
                                                    action: "delete".into(),
                                                });
                                            }
                                        })),
                                ),
                        );
                    }
                }
                if !self.queued_follow_up.is_empty() {
                    col = col.child(
                        div()
                            .text_size(px(10.5))
                            .font_semibold()
                            .text_color(p.quiet_ink)
                            .child("FOLLOW-UP (AFTER TURN)"),
                    );
                    for (idx, txt) in self.queued_follow_up.iter().enumerate() {
                        let exp_p = txt.clone();
                        let exp_d = txt.clone();
                        col = col.child(
                            h_flex()
                                .id(("q-fu", idx))
                                .px_2()
                                .py_1()
                                .rounded(px(6.))
                                .bg(p.sidebar_group_surface)
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_xs()
                                        .text_color(p.ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(txt.clone()),
                                )
                                .child(
                                    div()
                                        .id(("q-fu-prom", idx))
                                        .size(px(20.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.sidebar_row_hover))
                                        .child(Icon::new(IconName::ArrowUp).size(px(11.)).text_color(p.working))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                    session: sid,
                                                    lane: "followUp".into(),
                                                    slot: idx,
                                                    expected: exp_p.clone(),
                                                    action: "promote".into(),
                                                });
                                            }
                                        })),
                                )
                                .child(
                                    div()
                                        .id(("q-fu-del", idx))
                                        .size(px(20.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.danger_surface))
                                        .child(Icon::new(IconName::Close).size(px(11.)).text_color(p.quiet_ink))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                    session: sid,
                                                    lane: "followUp".into(),
                                                    slot: idx,
                                                    expected: exp_d.clone(),
                                                    action: "delete".into(),
                                                });
                                            }
                                        })),
                                ),
                        );
                    }
                }

                Some(
                    v_flex()
                        .id("floating-queue-panel")
                        .absolute()
                        .bottom(px(112.))
                        .left(left_offset)
                        .w(px(360.))
                        .p_3()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .gap_2()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("queue-scroll")
                                .max_h(px(240.))
                                .overflow_y_scrollbar()
                                .child(col),
                        )
                        .child(div().h(px(1.)).w_full().bg(p.panel_edge))
                        .child(
                            h_flex()
                                .id("queue-clear-all")
                                .h(px(26.))
                                .rounded(px(6.))
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .hover(|d| d.bg(p.danger_surface))
                                .text_xs()
                                .text_color(p.danger)
                                .child("Clear all queued")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(sid) = this.active_session_id.clone() {
                                        let _ = this
                                            .bridge
                                            .tx
                                            .send(DaemonCmd::ClearQueue { session: sid });
                                    }
                                    this.open_popup = OpenPopup::None;
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                )
            }
            OpenPopup::ModelMenu => {
                let cur_model_id = self
                    .active_session()
                    .map(|s| s.model.clone())
                    .unwrap_or_default();
                let cur_thinking = self
                    .active_session()
                    .map(|s| s.thinking.clone())
                    .filter(|t| !t.is_empty())
                    .unwrap_or_else(|| "medium".to_string());

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

                let configured_models: Vec<ModelEntry> = self
                    .models
                    .iter()
                    .filter(|m| m.configured)
                    .take(24)
                    .cloned()
                    .collect();

                let mut model_list = v_flex().gap(px(2.));
                if configured_models.is_empty() {
                    model_list = model_list.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(p.quiet_ink)
                            .child("No configured models"),
                    );
                } else {
                    for (idx, m) in configured_models.into_iter().enumerate() {
                        let selected = m.id == cur_model_id;
                        let prov = m.provider.clone();
                        let mid = m.id.clone();
                        model_list = model_list.child(
                            h_flex()
                                .id(("m-opt", idx))
                                .px_2()
                                .py_1()
                                .rounded(px(6.))
                                .justify_between()
                                .items_center()
                                .cursor_pointer()
                                .when(selected, |d| d.bg(p.sidebar_row_selected))
                                .hover(|d| d.bg(p.sidebar_row_hover))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(if selected { p.ink } else { p.quiet_ink })
                                        .child(m.name),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.sidebar_group_empty_ink)
                                        .child(m.provider),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(sid) = this.active_session_id.clone() {
                                        let _ = this.bridge.tx.send(DaemonCmd::SetModel {
                                            session: sid,
                                            provider: prov.clone(),
                                            model_id: mid.clone(),
                                        });
                                    }
                                    this.open_popup = OpenPopup::None;
                                    cx.notify();
                                })),
                        );
                    }
                }

                let mut effort_row = h_flex().gap_1().flex_wrap();
                for (idx, lvl) in thinking_levels.into_iter().enumerate() {
                    let active = lvl == cur_thinking;
                    let level_c = lvl.clone();
                    effort_row = effort_row.child(
                        div()
                            .id(("eff-opt", idx))
                            .px_2()
                            .py_0p5()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(if active { p.working } else { p.panel_edge })
                            .bg(if active {
                                p.sidebar_row_selected
                            } else {
                                gpui_kit::transparent_black()
                            })
                            .text_xs()
                            .text_color(if active { p.ink } else { p.quiet_ink })
                            .cursor_pointer()
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(lvl)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if let Some(sid) = this.active_session_id.clone() {
                                    let _ = this.bridge.tx.send(DaemonCmd::SetThinking {
                                        session: sid,
                                        level: level_c.clone(),
                                    });
                                }
                                this.open_popup = OpenPopup::None;
                                cx.notify();
                            })),
                    );
                }

                Some(
                    v_flex()
                        .id("floating-model-menu")
                        .absolute()
                        .bottom(px(112.))
                        .right(right_offset)
                        .w(px(360.))
                        .p_3()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .gap_2()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_semibold()
                                .text_color(p.quiet_ink)
                                .child("THINKING EFFORT"),
                        )
                        .child(effort_row)
                        .child(div().h(px(1.)).w_full().bg(p.panel_edge))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .font_semibold()
                                .text_color(p.quiet_ink)
                                .child("MODELS"),
                        )
                        .child(
                            div()
                                .id("model-popup-scroll")
                                .max_h(px(260.))
                                .overflow_y_scrollbar()
                                .child(model_list),
                        )
                        .into_any_element(),
                )
            }
            OpenPopup::ContextCard => Some(
                div()
                    .id("floating-ctx-card")
                    .absolute()
                    .bottom(px(112.))
                    .right(right_offset)
                    .w(px(236.))
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.panel_surface)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(self.render_context_card_content())
                    .into_any_element(),
            ),
            OpenPopup::SettingsMenu => Some(
                div()
                    .id("floating-settings-menu")
                    .absolute()
                    .top(px(36.))
                    .right(px(16.))
                    .w(px(210.))
                    .p_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.panel_surface)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .text_size(px(10.5))
                                    .font_semibold()
                                    .text_color(p.quiet_ink)
                                    .child("APPEARANCE"),
                            )
                            .child(
                                h_flex()
                                    .id("set-theme-toggle")
                                    .px_2()
                                    .py_1p5()
                                    .rounded(px(6.))
                                    .justify_between()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(div().text_xs().text_color(p.ink).child("Theme"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.quiet_ink)
                                            .child(if self.settings.dark_mode { "Dark" } else { "Light" }),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.settings.dark_mode = !this.settings.dark_mode;
                                        this.settings.save();
                                        Theme::change(
                                            if this.settings.dark_mode {
                                                ThemeMode::Dark
                                            } else {
                                                ThemeMode::Light
                                            },
                                            Some(window),
                                            cx,
                                        );
                                        cx.notify();
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("set-export-html")
                                    .px_2()
                                    .py_1p5()
                                    .rounded(px(6.))
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(div().text_xs().text_color(p.ink).child("Export session HTML"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(sid) = this.active_session_id.clone() {
                                            let _ = this
                                                .bridge
                                                .tx
                                                .send(DaemonCmd::ExportHtml { session: sid });
                                        }
                                        this.open_popup = OpenPopup::None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
            ),
            OpenPopup::SessionMenu {
                id,
                file,
                name,
                is_resident,
                ..
            } => {
                let id_c = id.clone();
                let file_ren = file.clone();
                let file_del = file.clone();
                let name_c = name.clone();
                let resident = *is_resident;
                Some(
                    div()
                        .id("floating-session-menu")
                        .absolute()
                        .top(px(96.))
                        .left(px(252.))
                        .w(px(190.))
                        .p_1p5()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            v_flex()
                                .gap_0p5()
                                .child(
                                    div()
                                        .px_2()
                                        .py_1()
                                        .text_size(px(11.))
                                        .font_semibold()
                                        .text_color(p.quiet_ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(name_c.clone()),
                                )
                                .when(!file_ren.is_empty(), |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-rename")
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.sidebar_row_hover))
                                            .text_xs()
                                            .text_color(p.ink)
                                            .child("Rename")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.start_rename(
                                                    file_ren.clone(),
                                                    name_c.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                })
                                .when(resident, |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-stop")
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.sidebar_row_hover))
                                            .text_xs()
                                            .text_color(p.ink)
                                            .child("Stop / Archive")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let _ = this.bridge.tx.send(DaemonCmd::MarkDone {
                                                    session: id_c.clone(),
                                                });
                                                this.open_popup = OpenPopup::None;
                                                cx.notify();
                                            })),
                                    )
                                })
                                .when(!resident && !file_del.is_empty(), |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-delete")
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.danger_surface))
                                            .text_xs()
                                            .text_color(p.danger)
                                            .child("Delete session")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let _ =
                                                    this.bridge.tx.send(DaemonCmd::DeleteSaved {
                                                        session_path: file_del.clone(),
                                                    });
                                                this.open_popup = OpenPopup::None;
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .into_any_element(),
                )
            }
        }
    }

    fn render_card_icon(&self, item: &TranscriptItem, color: Hsla) -> AnyElement {
        match (item.kind, item.variant) {
            (ItemKind::Thinking, _) => svg()
                .path(ICON_THINKING_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, CardVariant::Python) => svg()
                .path(ICON_PYTHON_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, CardVariant::Bash) | (_, CardVariant::BashDone) => svg()
                .path(ICON_BASH_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, _) => svg()
                .path(ICON_PYTHON_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (_, CardVariant::AgentMessage) => {
                Icon::new(IconName::Bot).size(px(14.)).text_color(color).into_any_element()
            }
            (_, CardVariant::ProviderRetry) => {
                Icon::new(IconName::Loader).size(px(14.)).text_color(color).into_any_element()
            }
            _ => Icon::new(IconName::Inbox).size(px(14.)).text_color(color).into_any_element(),
        }
    }

    fn render_item(&self, item: &TranscriptItem, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        match item.kind {
            ItemKind::User => v_flex()
                .w_full()
                .rounded(px(10.))
                .bg(p.transcript_user_fill)
                .px_4()
                .py_3()
                .gap_2()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| TextView::new(md).selectable(true)),
                )
                .when(!item.images.is_empty(), |col| {
                    col.child(
                        h_flex().gap_2().flex_wrap().children(
                            item.images.iter().cloned().map(|image| {
                                img(image)
                                    .max_w(px(360.))
                                    .max_h(px(220.))
                                    .rounded(px(6.))
                            }),
                        ),
                    )
                })
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
                .border_color(p.danger)
                .bg(p.danger_surface)
                .px_3()
                .py_2()
                .text_sm()
                .text_color(p.ink)
                .child(item.raw_body.clone())
                .into_any_element(),
            ItemKind::Thinking | ItemKind::ToolCall | ItemKind::Summary | ItemKind::Custom => {
                let id = item.id;
                let copy_payload = item.raw_body.clone();
                let icon_color = if item.active {
                    p.working
                } else if item.failed {
                    p.danger
                } else {
                    p.quiet_ink
                };

                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if item.active {
                        p.working
                    } else {
                        p.transcript_cell_edge
                    })
                    .bg(p.transcript_cell_fill)
                    .child(
                        h_flex()
                            .id(("card-hdr", id))
                            .h(px(30.))
                            .px_3()
                            .gap_2()
                            .items_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_item(id, cx);
                            }))
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(self.render_card_icon(item, icon_color)),
                            )
                            .when(!item.verb.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .font_medium()
                                        .text_color(if item.active { p.working } else { p.quiet_ink })
                                        .child(item.verb.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(if item.failed { p.danger } else { p.ink })
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(item.title.clone()),
                            )
                            .when(!item.meta.is_empty(), |row| {
                                row.child(
                                    div()
                                        .h(px(18.))
                                        .px(px(7.))
                                        .rounded(px(9.))
                                        .border_1()
                                        .border_color(p.transcript_cell_edge)
                                        .flex()
                                        .items_center()
                                        .text_size(px(11.))
                                        .line_height(px(12.))
                                        .text_color(if item.failed { p.danger } else { p.quiet_ink })
                                        .child(item.meta.clone()),
                                )
                            })
                            .when(
                                item.kind == ItemKind::ToolCall && !item.raw_body.is_empty(),
                                |row| {
                                    row.child(
                                        div()
                                            .id(("copy-btn", id))
                                            .h(px(20.))
                                            .px(px(7.))
                                            .rounded(px(5.))
                                            .flex()
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.sidebar_row_hover))
                                            .text_size(px(11.))
                                            .text_color(p.quiet_ink)
                                            .child("Copy")
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
                                .size(px(12.))
                                .text_color(p.quiet_ink),
                            ),
                    )
                    .when(item.expanded, |card| {
                        card.child(
                            div()
                                .border_t_1()
                                .border_color(p.transcript_cell_edge)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.advance_stop_anim(window);
        let p = self.palette();
        let scrolled_up = self.is_scrolled_up();

        div()
            .id("pa-root")
            .relative()
            .size_full()
            .bg(p.window_surface)
            .text_color(p.ink)
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging_composer {
                    if ev.pressed_button != Some(MouseButton::Left) {
                        this.dragging_composer = false;
                        cx.notify();
                        return;
                    }
                    let dy = this.drag_start_y - ev.position.y.as_f32();
                    let next = (this.drag_start_height + dy).clamp(44.0, 320.0);
                    this.composer_height = Some(next);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.dragging_composer {
                        this.dragging_composer = false;
                        cx.notify();
                    }
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.open_popup != OpenPopup::None {
                    this.open_popup = OpenPopup::None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
                if ctrl && key == "b" {
                    this.settings.sidebar_open = !this.settings.sidebar_open;
                    this.rail_mark_hovered = false;
                    this.settings.save();
                    cx.notify();
                } else if ctrl && key == "n" {
                    this.request_new_session(window, cx);
                } else if ctrl && key == "f" {
                    this.find_open = !this.find_open;
                    if this.find_open {
                        let focus = this.find_input.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }
                    cx.notify();
                } else if key == "escape" {
                    if this.open_popup != OpenPopup::None {
                        this.open_popup = OpenPopup::None;
                        cx.notify();
                    } else if this.find_open {
                        this.find_open = false;
                        cx.notify();
                    } else if this.renaming_path.is_some() {
                        this.renaming_path = None;
                        cx.notify();
                    } else if this.is_streaming {
                        if let Some(sid) = this.active_session_id.clone() {
                            let _ = this.bridge.tx.send(DaemonCmd::Abort { session: sid });
                        }
                    }
                }
            }))
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_titlebar(cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .child(self.render_sidebar(window, cx))
                            .child(
                                v_flex()
                                    .relative()
                                    .flex_1()
                                    .h_full()
                                    .min_w_0()
                                    .bg(p.transcript_surface)
                                    .rounded_tl(px(10.))
                                    .when(self.settings.right_open, |s| s.rounded_tr(px(10.)))
                                    .overflow_hidden()
                                    .children(self.render_find_bar(window, cx))
                                    .child(
                                        div()
                                            .relative()
                                            .flex_1()
                                            .w_full()
                                            .min_h_0()
                                            .child(
                                                list(
                                                    self.list_state.clone(),
                                                    cx.processor(|this, ix, _window, cx| {
                                                        let len = this.items.len();
                                                        let Some(item) = this.items.get(ix) else {
                                                            return div().into_any_element();
                                                        };
                                                        div()
                                                            .w_full()
                                                            .px_6()
                                                            .when(ix == 0, |d| d.pt_4())
                                                            .pb(if ix + 1 == len {
                                                                px(16.)
                                                            } else {
                                                                px(12.)
                                                            })
                                                            .child(
                                                                div()
                                                                    .w_full()
                                                                    .max_w(px(860.))
                                                                    .mx_auto()
                                                                    .child(this.render_item(item, cx)),
                                                            )
                                                            .into_any_element()
                                                    }),
                                                )
                                                .size_full(),
                                            )
                                            .vertical_scrollbar(&self.list_state)
                                            .when(scrolled_up, |wrap| {
                                                wrap.child(
                                                    h_flex()
                                                        .absolute()
                                                        .left_0()
                                                        .right_0()
                                                        .bottom(px(12.))
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .id("jump-to-bottom-btn")
                                                                .size(px(30.))
                                                                .rounded_full()
                                                                .border_1()
                                                                .border_color(p.panel_edge)
                                                                .bg(p.panel_surface)
                                                                .shadow_md()
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .cursor_pointer()
                                                                .hover(|d| d.bg(p.sidebar_row_hover))
                                                                .child(
                                                                    Icon::new(IconName::ChevronDown)
                                                                        .size(px(14.))
                                                                        .text_color(p.ink),
                                                                )
                                                                .on_click(cx.listener(|this, _, _, cx| {
                                                                    cx.stop_propagation();
                                                                    this.list_state.set_follow_mode(
                                                                        FollowMode::Tail,
                                                                    );
                                                                    cx.notify();
                                                                })),
                                                        ),
                                                )
                                            }),
                                    )
                                    .child(self.render_composer_band(window, cx)),
                            )
                            .children(self.render_right_column(cx)),
                    ),
            )
            .children(self.render_floating_popups(cx))
    }
}

fn session_key(row: &SessionRow) -> String {
    if !row.id.is_empty() {
        row.id.clone()
    } else {
        row.session_file.clone()
    }
}
