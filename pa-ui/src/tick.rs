//! The poll tick: everything the window learns by asking rather than being told.

use crate::*;

pub(crate) struct Tick {
    pub weak: slint::Weak<AppWindow>,
    pub jobs: Sender<Job>,
    pub msg_rx: Receiver<(u64, Msg)>,
    /// which attach the window is waiting on: a reply that carries an older
    /// number is an answer to a question it has given up on
    pub attach_generation: std::cell::Cell<u64>,
    /// when the poll last ran: a long gap is a stall the reader felt
    pub last_tick: std::cell::Cell<SystemTime>,
    pub sheet: Rc<sheet::Sheet>,
    pub sidebar: Rc<sessions::Sidebar>,
    pub composer: Rc<composer::Composer>,
    pub models: Rc<models::ModelMenu>,
    pub busy_since: Rc<std::cell::Cell<SystemTime>>,
    pub window_rows: usize,
    pub drawn_user: std::cell::Cell<Option<String>>,
    pub loaded: std::cell::Cell<Option<String>>,
    pub live_state: std::cell::Cell<String>,
    pub live_since: std::cell::Cell<SystemTime>,
    pub busy_label: std::cell::Cell<String>,
    pub last_seq: std::cell::Cell<u64>,
    pub open_live: std::cell::Cell<Option<live::Live>>,
    pub stream_started: std::cell::Cell<SystemTime>,
    pub _stream_at: std::cell::Cell<SystemTime>,
    pub tail_at: std::cell::Cell<SystemTime>,
    pub queue_at: std::cell::Cell<SystemTime>,
    pub stats_at: std::cell::Cell<SystemTime>,
    pub scroll_ticks: std::cell::Cell<u64>,
}

pub(crate) struct Local {
    drawn_user: Option<String>,
    loaded: Option<String>,
    live_state: String,
    live_since: SystemTime,
    busy_label: String,
    last_seq: u64,
    open_live: Option<live::Live>,
    stream_started: SystemTime,
    _stream_at: SystemTime,
    tail_at: SystemTime,
    queue_at: SystemTime,
    stats_at: SystemTime,
    scroll_ticks: u64,
}

impl Local {
    fn load(tick: &Tick) -> Self {
        Self {
            drawn_user: tick.drawn_user.take(),
            loaded: tick.loaded.take(),
            live_state: tick.live_state.take(),
            live_since: tick.live_since.get(),
            busy_label: tick.busy_label.take(),
            last_seq: tick.last_seq.get(),
            open_live: tick.open_live.take(),
            stream_started: tick.stream_started.get(),
            _stream_at: tick._stream_at.get(),
            tail_at: tick.tail_at.get(),
            queue_at: tick.queue_at.get(),
            stats_at: tick.stats_at.get(),
            scroll_ticks: tick.scroll_ticks.get(),
        }
    }

    fn store(self, tick: &Tick) {
        tick.drawn_user.set(self.drawn_user);
        tick.loaded.set(self.loaded);
        tick.live_state.set(self.live_state);
        tick.live_since.set(self.live_since);
        tick.busy_label.set(self.busy_label);
        tick.last_seq.set(self.last_seq);
        tick.open_live.set(self.open_live);
        tick.stream_started.set(self.stream_started);
        tick._stream_at.set(self._stream_at);
        tick.tail_at.set(self.tail_at);
        tick.queue_at.set(self.queue_at);
        tick.stats_at.set(self.stats_at);
        tick.scroll_ticks.set(self.scroll_ticks);
    }
}

impl Tick {
    fn request_older(&self, ui: &AppWindow) {
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        let Some(before) = self.sheet.check_load_older(
            ui.get_follow_bottom(),
            ui.get_pager_ready(),
            ui.get_dbg_cy(),
            ui.get_dbg_ch(),
            ui.get_dbg_h(),
        ) else {
            return;
        };
        ui.set_loading_older(true);
        if crate::env_cfg::cfg().debug {
            eprintln!("[older] asking for the page before {before}");
        }
        let _ = self.jobs.send(Job::LoadOlder {
            session,
            before,
            limit: OLDER_PAGE,
        });
    }

    /// One pass: drain what the worker sent, then let the window catch up.
    pub fn run(&self) {
        let now = SystemTime::now();
        if let Ok(gap) = now.duration_since(self.last_tick.get()) {
            if gap.as_millis() > 250 && crate::env_cfg::cfg().debug {
                eprintln!("[lag] the window was busy for {} ms", gap.as_millis());
            }
        }
        self.last_tick.set(now);
        let Some(ui) = self.weak.upgrade() else { return };
        let mut l = Local::load(self);
        let jobs = &self.jobs;
        let msg_rx = &self.msg_rx;
        let sheet = &self.sheet;
        let sidebar = &self.sidebar;
        let composer = &self.composer;
        let models = &self.models;
        let busy_since = &self.busy_since;
        let window_rows = self.window_rows;

        if crate::env_cfg::cfg().debug {
            eprintln!(
                "[scroll] cy={} ch={} h={} at-bottom={}",
                ui.get_dbg_cy(),
                ui.get_dbg_ch(),
                ui.get_dbg_h(),
                ui.get_at_bottom()
            );
            l.scroll_ticks += 1;
            if l.scroll_ticks.is_multiple_of(15) {
                eprintln!(
                    "[scroll] cy={} ch={} h={} at-bottom={} older={} ready={} loading={}",
                    ui.get_dbg_cy(),
                    ui.get_dbg_ch(),
                    ui.get_dbg_h(),
                    ui.get_at_bottom(),
                    ui.get_has_older(),
                    ui.get_pager_ready(),
                    ui.get_loading_older()
                );
            }
        }

        // right-panel telemetry
        if !l.loaded.as_deref().unwrap_or("").is_empty()
            && l.stats_at.elapsed().unwrap_or_default()
                >= Duration::from_millis(if ui.get_busy() { 3000 } else { 15000 })
        {
            l.stats_at = SystemTime::now();
            if let Some(session) = l.loaded.clone() {
                let _ = jobs.send(Job::Stats { session });
            }
        }

        // what is stacked behind the running turn
        if ui.get_busy()
            && !l.loaded.as_deref().unwrap_or("").is_empty()
            && !crate::env_cfg::cfg().queue_preset
            && l.queue_at.elapsed().unwrap_or_default() >= Duration::from_millis(1500)
        {
            l.queue_at = SystemTime::now();
            if let Some(session) = l.loaded.clone() {
                let _ = jobs.send(Job::Queue { session });
            }
        }

        let poll_every = if ui.get_busy() {
            Duration::from_millis(900)
        } else {
            Duration::from_millis(3000)
        };
        if !l.loaded.as_deref().unwrap_or("").is_empty()
            && l.tail_at.elapsed().unwrap_or_default() >= poll_every
        {
            l.tail_at = SystemTime::now();
            if let Some(session) = l.loaded.clone() {
                let _ = jobs.send(Job::Tail {
                    session,
                    fresh: false,
                });
            }
        }

        // follow the selection
        let selected = ui.get_selected_id().to_string();
        if !selected.is_empty() && l.loaded.as_deref() != Some(selected.as_str()) {
            l.loaded = Some(selected.clone());
            sidebar.track_selection(&selected);
            sheet.reset_paging(&ui);
            let generation = self.attach_generation.get() + 1;
            self.attach_generation.set(generation);
            let _ = jobs.send(Job::Attach {
                session: selected,
                generation,
            });
        }

        // reaching the top pulls in the page before this one
        self.request_older(&ui);

        // 60-second idle floor trim
        let at_floor =
            ui.get_follow_bottom() && !ui.get_scrollbar_dragging() && ui.get_at_bottom();
        if let Some((gone, more)) = sheet.check_idle_floor_trim(at_floor, now) {
            ui.set_has_older(more);
            scroll_to_end_when_ready(&ui.as_weak());
            if crate::env_cfg::cfg().debug {
                eprintln!("[trim] idle floor trim unloaded {gone} rows above tail");
            }
        }

        // caret and selection moves arrive here (arrow keys, clicks); keystrokes paint themselves
        composer.repaint(&ui);

        while let Ok((generation, message)) = msg_rx.try_recv() {
            if message.is_session_scoped() && generation != self.attach_generation.get() {
                continue;
            }
            match message {
                Msg::Sessions(Ok(rows)) => {
                    if let Some(rebound) = sidebar.apply_roster(&ui, rows.clone()) {
                        l.loaded = Some(rebound);
                    }
                    let selected = ui.get_selected_id().to_string();
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        models.sync_session_row(&ui, row);
                    }
                    if !rows.iter().any(|row| row.id == selected) && ui.get_busy() {
                        ui.set_busy(false);
                        l.busy_label.clear();
                    }
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        let state = row.live_state();
                        if state != l.live_state {
                            l.live_state = state.to_string();
                            l.live_since = SystemTime::now();
                        }
                        let busy_now = state != "idle" && state != "archived";
                        if !busy_now && ui.get_busy() {
                            ui.set_busy(false);
                            l.busy_label.clear();
                            if let Some(session) = l.loaded.clone() {
                                let _ = jobs.send(Job::Tail {
                                    session,
                                    fresh: false,
                                });
                            }
                        }
                        if busy_now && !ui.get_busy() {
                            if l.busy_label.is_empty() {
                                l.busy_label = state.to_string();
                                busy_since.set(SystemTime::now());
                                ui.set_busy_label(l.busy_label.clone().into());
                            }
                            ui.set_busy(true);
                        }
                        if ui.get_busy() {
                            let held = busy_since.get().elapsed().unwrap_or_default().as_secs();
                            ui.set_busy_elapsed(format!("{held}s").into());
                        }
                    }
                }
                Msg::Sessions(Err(error)) => status::trouble(&ui, format!("daemon: {error}")),
                Msg::Activity { label } => {
                    if label != l.busy_label {
                        l.busy_label = label;
                        busy_since.set(SystemTime::now());
                        ui.set_busy_label(l.busy_label.clone().into());
                    }
                    ui.set_busy(true);
                }
                Msg::Transcript {
                    session,
                    rows,
                    skipped,
                    messages,
                } => {
                    let _ = jobs.send(Job::Commands {
                        session: session.clone(),
                    });
                    sheet.set_tail_anchor(&messages);
                    let _ = window_rows;
                    let mut built = rows;
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[attach] {} msgs -> {} rows (skipped {skipped})",
                            messages.len(),
                            built.len()
                        );
                    }
                    if std::env::var("PA_DEMO").as_deref() == Ok("fail") {
                        built.extend(transcript::demo_rows());
                    }
                    if crate::env_cfg::cfg().demo_markdown {
                        built.extend(transcript::markdown_demo_rows());
                    }
                    sheet.replace(built);
                    sheet::dump(&sheet.rows());
                    let _ = jobs.send(Job::SessionState {
                        session: session.clone(),
                    });
                    if crate::env_cfg::cfg().menu {
                        let _ = jobs.send(Job::ModelCatalog {
                            session: session.clone(),
                        });
                    }
                    let _ = jobs.send(Job::Tail {
                        session: session.clone(),
                        fresh: true,
                    });
                    scroll_to_end_when_ready(&ui.as_weak());
                }
                Msg::Tail {
                    session,
                    rows,
                    skipped,
                    messages,
                    last_entry,
                    first_entry,
                    more,
                    fresh,
                } => {
                    let follow = ui.get_follow_bottom() && !ui.get_scrollbar_dragging();
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[tail] {session} {} msgs (have {}) newest={:?}",
                            messages.len(),
                            sheet.tail_len(),
                            messages
                                .last()
                                .map(|m| m
                                    .get("timestamp")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0))
                                .unwrap_or(0)
                        );
                    }
                    let Some(should_merge) = sheet.accept_tail_page(
                        &messages,
                        &first_entry,
                        &last_entry,
                        more,
                        fresh,
                        l.open_live.is_some(),
                    ) else {
                        continue;
                    };
                    ui.set_has_older(sheet.has_older());
                    if !should_merge {
                        continue;
                    }
                    let _ = window_rows;
                    let mut built = rows;
                    if built.is_empty() && !messages.is_empty() {
                        continue;
                    }
                    if crate::env_cfg::cfg().demo_markdown {
                        built.extend(transcript::markdown_demo_rows());
                    }
                    let live_in_built = l
                        .open_live
                        .as_ref()
                        .is_some_and(|turn| built.iter().any(|row| row.message == turn.message));
                    if live_in_built || (!ui.get_busy() && !composer.in_flight.get()) {
                        l.open_live = None;
                        sheet.answer_ended();
                    }
                    sheet.merge(built);
                    let count = sheet.rows().len();
                    if crate::env_cfg::cfg().dump.is_some() {
                        sheet::dump(&sheet.rows());
                    }
                    sheet::trace("tail merge", &sheet.rows());
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[tail] rebuilt {} msgs -> {count} rows (skipped {skipped})",
                            messages.len()
                        );
                    }
                    if follow {
                        scroll_to_end_when_ready(&ui.as_weak());
                    }
                    {
                        let weak = ui.as_weak();
                        slint::Timer::single_shot(Duration::from_millis(700), move || {
                            if let Some(ui) = weak.upgrade() {
                                ui.set_pager_ready(true);
                            }
                        });
                    }
                }
                Msg::Older {
                    rows,
                    messages,
                    first_entry,
                    more,
                    ..
                } => {
                    ui.set_loading_older(false);
                    let before_y = ui.get_dbg_cy();
                    let older_len = rows.len();
                    let insert_at = SystemTime::now();
                    let (added_h, more_after) = sheet.complete_older_page(
                        rows,
                        messages.is_empty(),
                        first_entry,
                        more,
                    );
                    ui.set_has_older(more_after);
                    if added_h > 0.0 {
                        ui.invoke_set_scroll_y(before_y - added_h);
                    }
                    if crate::env_cfg::cfg().debug && !messages.is_empty() {
                        let insert_ms = insert_at
                            .elapsed()
                            .map(|d| d.as_secs_f64() * 1000.0)
                            .unwrap_or(0.0);
                        eprintln!(
                            "[older] {} msgs -> {older_len} rows ({insert_ms:.1} ms), more={more_after}",
                            messages.len()
                        );
                    }
                }
                Msg::Event { session, event } => {
                    if let Some(label) = pa_client::activity_label(&event) {
                        ui.set_busy_label(label.into());
                    }
                    if let Some(sequence) = event
                        .pointer("/meta/sequence")
                        .and_then(serde_json::Value::as_u64)
                    {
                        if l.last_seq != 0 && sequence > l.last_seq + 1 {
                            let _ = jobs.send(Job::Tail {
                                session: session.clone(),
                                fresh: false,
                            });
                        }
                        l.last_seq = sequence.max(l.last_seq);
                    }

                    let kind = event.get("type").and_then(serde_json::Value::as_str);
                    if kind == Some("session_action_update") {
                        let lane = |name: &str| -> Vec<String> {
                            event
                                .get("actions")
                                .and_then(|actions| actions.get(name))
                                .and_then(serde_json::Value::as_array)
                                .map(|rows| {
                                    rows.iter()
                                        .filter_map(serde_json::Value::as_str)
                                        .map(str::to_string)
                                        .collect()
                                })
                                .unwrap_or_default()
                        };
                        composer.project_queue(&ui, &lane("steering"), &lane("followUps"));
                        continue;
                    }
                    if matches!(kind, Some("turn_end" | "agent_end")) {
                        composer.in_flight.set(false);
                        sheet.answer_ended();
                        l.open_live = None;
                        l._stream_at = SystemTime::now() - Duration::from_secs(60);
                        let _ = jobs.send(Job::Tail {
                            session: session.clone(),
                            fresh: true,
                        });
                        continue;
                    }
                    if matches!(kind, Some("tool_execution_start" | "tool_execution_end")) {
                        let call_id = event
                            .get("toolCallId")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default()
                            .to_string();
                        if call_id.is_empty() {
                            continue;
                        }
                        let Some(turn) = l.open_live.as_mut() else {
                            continue;
                        };
                        if kind == Some("tool_execution_start") {
                            let args = event
                                .get("args")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);
                            turn.start_tool(&call_id, args);
                        } else {
                            let output = tool_output_text(event.get("result"));
                            let failed = event
                                .get("isError")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            let details =
                                event.get("result").and_then(|result| result.get("details"));
                            let bash = transcript::bash_commands(details);
                            let taken = transcript::duration_ms(details);
                            turn.finish_tool(&call_id, output, failed, bash, taken);
                        }
                        let held = hold_offset(&ui);
                        if let Some(turn) = l.open_live.as_ref() {
                            sheet.merge(turn.rows());
                        }
                        pin_offset_across_layout(&ui, held);
                        scroll_if_parked(&ui);
                        l._stream_at = SystemTime::now();
                        continue;
                    }
                    let Some(message) = event.get("message") else {
                        continue;
                    };
                    if !matches!(kind, Some("message_start" | "message_update" | "message_end")) {
                        continue;
                    }
                    let role = message.get("role").and_then(serde_json::Value::as_str);
                    if role == Some("user") {
                        let text = message_text(message);
                        let stamp = message
                            .get("timestamp")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0);
                        let key = format!(
                            "{stamp}:{}:{}",
                            text.chars().count(),
                            text.chars().take(40).collect::<String>()
                        );
                        if l.drawn_user.as_deref() != Some(key.as_str()) {
                            l.drawn_user = Some(key);
                            let _ = jobs.send(Job::Tail {
                                session: session.clone(),
                                fresh: false,
                            });
                        }
                        if kind == Some("message_end") {
                            *composer.pending_user.borrow_mut() = None;
                        }
                        continue;
                    }
                    if role != Some("assistant") {
                        continue;
                    }
                    let delta_event: &serde_json::Value = event
                        .get("assistantMessageEvent")
                        .filter(|value| !value.is_null())
                        .unwrap_or(&event);
                    let delta_kind = delta_event
                        .get("type")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    if kind == Some("message_start") || delta_kind == "start" {
                        let key = transcript::message_key(message);
                        if l.open_live.as_ref().is_some_and(|turn| turn.message == key) {
                            continue;
                        }
                        l.open_live = Some(live::Live::new(key.clone()));
                        sheet.answer_began(&key);
                        l.stream_started = SystemTime::now();
                        sheet::trace("live rows opened", &sheet.rows());
                        continue;
                    }
                    if l.open_live.is_none() && kind == Some("message_update") {
                        let key = transcript::message_key(message);
                        l.open_live = Some(live::Live::new(key.clone()));
                        sheet.answer_began(&key);
                        l.stream_started = SystemTime::now();
                    }
                    let Some(turn) = l.open_live.as_mut() else {
                        continue;
                    };
                    if !delta_kind.is_empty()
                        && !delta_kind.starts_with("text_delta")
                        && !delta_kind.starts_with("thinking_delta")
                    {
                        continue;
                    }
                    if kind == Some("message_end") {
                        continue;
                    }
                    if let Some(delta) =
                        delta_event.get("delta").and_then(serde_json::Value::as_str)
                    {
                        if delta_kind.starts_with("text") {
                            turn.push_text(delta);
                        } else if delta_kind.starts_with("thinking") {
                            turn.push_thinking(delta);
                        }
                    }
                    let held = hold_offset(&ui);
                    if let Some(turn) = l.open_live.as_ref() {
                        sheet.merge(turn.rows());
                    }
                    pin_offset_across_layout(&ui, held);
                    scroll_if_parked(&ui);
                    l._stream_at = SystemTime::now();
                    if crate::env_cfg::cfg().debug && kind == Some("message_update") {
                        eprintln!(
                            "[stream] +{:>5}ms rows {} ({} chars, {})",
                            l.stream_started.elapsed().map(|d| d.as_millis()).unwrap_or(0),
                            l.open_live.as_ref().map_or(0, |turn| turn.rows().len()),
                            0usize,
                            delta_kind
                        );
                    }
                }
                Msg::Queue {
                    steering,
                    follow_up,
                } => {
                    composer.project_queue(&ui, &steering, &follow_up);
                }
                Msg::Prompt {
                    session,
                    message,
                    images,
                } => {
                    composer.in_flight.set(true);
                    let _ = jobs.send(Job::Prompt {
                        session,
                        message,
                        images,
                    });
                }
                Msg::NewChat(id) => {
                    composer.reset_for_new_chat(&ui, &id);
                }
                Msg::QueueEdit { text } => {
                    ui.set_composer_text(text.into());
                    ui.set_composer_has_text(!ui.get_composer_text().is_empty());
                    status::clear(&ui);
                }
                Msg::ExtraCommands(rows) => {
                    composer.set_extra_commands(rows);
                }
                Msg::ModelCatalog(entries) => {
                    models.set_catalog(&ui, entries);
                }
                Msg::Stats { session, stats } => {
                    let fill = if stats.context_window > 0 {
                        (stats.context_tokens as f64 / stats.context_window as f64) as f32
                    } else {
                        0.0
                    };
                    ui.set_context_fill(fill.clamp(0.0, 1.0));
                    ui.set_context_text(
                        format!(
                            "{} / {}",
                            tokens(stats.context_tokens),
                            tokens(stats.context_window)
                        )
                        .into(),
                    );
                    ui.set_ctx_percent(format!("{:.0}%", stats.percent).into());
                    ui.set_stat_messages(stats.messages.to_string().into());
                    ui.set_stat_tools(stats.tool_calls.to_string().into());
                    ui.set_stat_in(tokens(stats.input_tokens).into());
                    ui.set_stat_out(tokens(stats.output_tokens).into());
                    ui.set_stat_cache(tokens(stats.cache_read).into());
                    let spent = stats.input_tokens + stats.output_tokens + stats.cache_read;
                    ui.set_cost_session(
                        if stats.cost > 0.0 {
                            format!("${:.2}", stats.cost)
                        } else if spent > 0 {
                            "unpriced".to_string()
                        } else {
                            "$0.00".to_string()
                        }
                        .into(),
                    );
                    let (family, has_children) = sidebar.family_cost(&session, stats.cost);
                    ui.set_cost_family(
                        if family > 0.0 {
                            format!("${:.2}", family)
                        } else {
                            "unpriced".to_string()
                        }
                        .into(),
                    );
                    ui.set_cost_family_on(has_children);
                }
                Msg::State { state, .. } => {
                    models.sync_session_state(&ui, &state);
                }
                Msg::Reconnected => {
                    l.loaded = None;
                    l.open_live = None;
                    sheet.answer_ended();
                    sheet.clear_tail_sig();
                    l.last_seq = 0;
                    status::say(&ui, "reconnecting");
                }
                Msg::PromptRejected { text } => {
                    composer.on_prompt_rejected(&ui, sheet, &text);
                }
                Msg::PromptResult { error } => {
                    composer.on_prompt_result(&ui, sheet, error);
                }
                Msg::Error(error) => status::trouble(&ui, format!("error: {error}")),
                Msg::Shared(Ok(shared)) => {
                    match crate::sessions::copy_to_clipboard(&shared.viewer_url) {
                        Ok(()) => status::say(&ui, format!("shared: {}", shared.viewer_url)),
                        Err(error) => status::trouble(
                            &ui,
                            format!("shared: {} (could not copy: {error})", shared.viewer_url),
                        ),
                    }
                }
                Msg::Shared(Err(error)) => {
                    status::trouble(&ui, format!("could not share: {error}"))
                }
            }
        }

        l.store(self);
    }
}