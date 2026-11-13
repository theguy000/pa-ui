//! One event pass: drain worker messages, then let the window catch up.

use crate::*;

fn binding_selection(selected: &str, previous: &str, active: &str) -> Option<String> {
    (selected == previous && !active.is_empty() && previous != active).then(|| active.to_string())
}

pub(crate) struct Tick {
    pub weak: slint::Weak<AppWindow>,
    pub jobs: Sender<Job>,
    pub msg_rx: Receiver<(u64, Msg)>,
    /// which attach the window is waiting on: a reply that carries an older
    /// number is an answer to a question it has given up on
    pub attach_generation: std::cell::Cell<u64>,
    pub sheet: Rc<sheet::Sheet>,
    pub sidebar: Rc<sessions::Sidebar>,
    pub composer: Rc<composer::Composer>,
    pub command_word: CommandWord,
    pub models: Rc<models::ModelMenu>,
    pub busy_since: Rc<std::cell::Cell<SystemTime>>,
    pub window_rows: usize,
    pub drawn_user: std::cell::Cell<Option<String>>,
    pub loaded: std::cell::Cell<Option<String>>,
    pub live_state: std::cell::Cell<String>,
    pub live_since: std::cell::Cell<SystemTime>,
    pub busy_label: std::cell::Cell<String>,
    /// The Session whose last Turn ended and whose strip must not be reopened
    /// by a late roster snapshot. Cleared by the next Turn.
    pub settled_session: std::cell::Cell<Option<String>>,
    pub last_seq: std::cell::Cell<u64>,
    pub open_live: std::cell::Cell<Option<live::Live>>,
    pub stream_started: std::cell::Cell<SystemTime>,
    pub scroll_ticks: std::cell::Cell<u64>,
    /// Error lines that belong to a Session other than the one on screen.
    /// They are replayed when that Session's Sheet is attached again.
    pub retained_errors: std::cell::RefCell<std::collections::HashMap<String, Vec<String>>>,
    /// The draft a refused prompt should restore when its Session comes back.
    pub retained_rejections: std::cell::RefCell<std::collections::HashMap<String, String>>,
    pub roster_seeded: std::cell::Cell<bool>,
    pub turn_failed: std::cell::Cell<bool>,
    pub turn_aborted: std::cell::Cell<bool>,
    pub announced_sessions: std::cell::RefCell<std::collections::HashSet<String>>,
    pub toast: std::cell::RefCell<Rc<dyn crate::notify::ToastNotifier>>,
}

pub(crate) struct Local {
    drawn_user: Option<String>,
    loaded: Option<String>,
    live_state: String,
    live_since: SystemTime,
    busy_label: String,
    settled_session: Option<String>,
    last_seq: u64,
    open_live: Option<live::Live>,
    stream_started: SystemTime,
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
            settled_session: tick.settled_session.take(),
            last_seq: tick.last_seq.get(),
            open_live: tick.open_live.take(),
            stream_started: tick.stream_started.get(),
            scroll_ticks: tick.scroll_ticks.get(),
        }
    }

    fn store(self, tick: &Tick) {
        tick.drawn_user.set(self.drawn_user);
        tick.loaded.set(self.loaded);
        tick.live_state.set(self.live_state);
        tick.live_since.set(self.live_since);
        tick.busy_label.set(self.busy_label);
        tick.settled_session.set(self.settled_session);
        tick.last_seq.set(self.last_seq);
        tick.open_live.set(self.open_live);
        tick.stream_started.set(self.stream_started);
        tick.scroll_ticks.set(self.scroll_ticks);
    }
}

fn reset_activity(ui: &AppWindow) {
    ui.set_activity_subagents(0);
    ui.set_activity_heartbeats(0);
    ui.set_activity_shells(0);
}

/// Start the strip for a Turn the engine has just announced. The TUI starts a
/// loader on `agent_start`/`turn_start`; the window must not stay settled just
/// because the Turn was started by the TUI or by automatic compaction rather
/// than by this window's composer.
fn open_status(
    ui: &AppWindow,
    l: &mut Local,
    busy_since: &std::cell::Cell<SystemTime>,
    label: &str,
) {
    l.settled_session = None;
    if !ui.get_busy() {
        l.busy_label = label.to_string();
        busy_since.set(SystemTime::now());
        ui.set_busy_label(l.busy_label.clone().into());
        ui.set_busy_elapsed("0s".into());
        ui.set_busy(true);
    }
}

/// Replace the strip's word for a phase that outranks the running loader, as
/// compaction does in the TUI.
fn force_status(
    ui: &AppWindow,
    l: &mut Local,
    busy_since: &std::cell::Cell<SystemTime>,
    label: &str,
) {
    l.settled_session = None;
    l.busy_label = label.to_string();
    busy_since.set(SystemTime::now());
    ui.set_busy_label(l.busy_label.clone().into());
    ui.set_busy_elapsed("0s".into());
    ui.set_busy(true);
}

fn count_i32(count: usize) -> i32 {
    i32::try_from(count).unwrap_or(i32::MAX)
}

pub(crate) fn draft_generation(ui: &AppWindow) -> u64 {
    u64::try_from(ui.get_draft_generation()).unwrap_or(0)
}

/// A reply belongs on screen only while the window is still showing its
/// session. An empty selection is the startup state, before the roster picks
/// one; tests and the first attach still need that reply.
fn session_is_current(ui: &AppWindow, session: &str) -> bool {
    if ui.get_draft_chat() {
        return false;
    }
    let selected = ui.get_selected_id();
    selected.is_empty() || selected.as_str() == session
}

impl Tick {
    fn follow_selection(
        &self,
        ui: &AppWindow,
        l: &mut Local,
        sidebar: &sessions::Sidebar,
        sheet: &sheet::Sheet,
        composer: &composer::Composer,
        jobs: &Sender<Job>,
        busy_since: &std::cell::Cell<SystemTime>,
    ) {
        let selected = ui.get_selected_id().to_string();
        let selected_session_id = if selected.is_empty() {
            String::new()
        } else {
            sidebar.session_id_for(&selected)
        };
        if selected.is_empty() {
            reset_activity(&ui);
        }
        if !selected.is_empty() && l.loaded.as_deref() != Some(selected.as_str()) {
            l.loaded = Some(selected.clone());
            // The reader is looking at a different Session: no settlement from
            // the one they left can decide what this one's strip may show.
            l.settled_session = None;
            sidebar.track_selection(&selected);
            sheet.reset_paging(&ui);
            sheet.replace(Vec::new());
            sheet::force_scroll_to_end(&ui);
            reset_activity(&ui);
            l.open_live = None;
            l.last_seq = 0;
            self.turn_failed.set(false);
            self.turn_aborted.set(false);
            composer.in_flight.set(false);
            self.sync_status_to_selection(&ui, l, &selected, busy_since);
            let generation = self.attach_generation.get() + 1;
            self.attach_generation.set(generation);
            let _ = jobs.send(Job::Attach {
                session: selected.clone(),
                generation,
            });
            let _ = jobs.send(Job::ActivityRead {
                session: selected.clone(),
                session_id: selected_session_id.clone(),
            });
        }
    }

    pub(crate) fn open_new_chat(&self) {
        let Some(ui) = self.weak.upgrade() else { return };
        let mut l = Local::load(self);
        reset_activity(&ui);
        let selected = ui.get_selected_id().to_string();
        if !selected.is_empty() {
            let cwd = self
                .sidebar
                .cache
                .borrow()
                .iter()
                .find(|row| row.id == selected)
                .map(|row| row.cwd.clone());
            if let Some(cwd) = cwd {
                ui.set_draft_cwd(cwd.into());
            }
        }
        self.sidebar.forget_selection();
        self.sheet.replace(Vec::new());
        self.sheet.reset_paging(&ui);
        self.sheet.clear_tail_sig();
        self.sheet.set_tail_anchor(&[]);
        let generation = self.attach_generation.get().wrapping_add(1);
        self.attach_generation.set(generation);
        ui.set_draft_generation(i32::try_from(generation).unwrap_or(i32::MAX));
        l.drawn_user = None;
        l.loaded = None;
        l.settled_session = None;
        l.last_seq = 0;
        l.open_live = None;
        l.live_state.clear();
        l.live_since = SystemTime::now();
        self.composer.open_unsaved_chat(&ui);
        self.clear_status(&ui, &mut l);
        sheet::force_scroll_to_end(&ui);
        l.store(self);
    }

    pub(crate) fn request_older(&self, ui: &AppWindow) {
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

    /// Take the Status strip down and forget the command word that held it.
    fn clear_status(&self, ui: &AppWindow, l: &mut Local) {
        self.command_word.clear();
        l.busy_label.clear();
        ui.set_busy_label("".into());
        ui.set_busy(false);
    }

    /// The strip belongs to the selected Session. Put it into that Session's
    /// state before the next event arrives, so a Turn from the previous
    /// selection cannot keep its label or timer up here.
    fn sync_status_to_selection(
        &self,
        ui: &AppWindow,
        l: &mut Local,
        session: &str,
        busy_since: &std::cell::Cell<SystemTime>,
    ) {
        let state = self
            .sidebar
            .cache
            .borrow()
            .iter()
            .find(|row| row.id == session)
            .map_or_else(|| "idle".to_string(), |row| row.live_state().to_string());
        l.live_state = state.clone();
        l.live_since = SystemTime::now();
        l.settled_session = None;
        self.command_word.clear();
        if matches!(state.as_str(), "idle" | "archived") {
            self.clear_status(ui, l);
            return;
        }
        l.busy_label = state.clone();
        busy_since.set(SystemTime::now());
        ui.set_busy_label(state.into());
        ui.set_busy_elapsed("0s".into());
        ui.set_busy(true);
    }

    /// Record one error against its Session. A current Session gets the row
    /// immediately; every Session keeps it so a later attach can put it back.
    fn record_error(&self, ui: &AppWindow, session: Option<&str>, message: &str) {
        let current = ui.get_selected_id();
        let on_screen = !(ui.get_draft_chat() && session.is_some())
            && (current.is_empty() || session.is_none_or(|target| target == current.as_str()));
        if on_screen {
            self.sheet.show_error(message);
        }
        if let Some(session) = session.filter(|session| !session.is_empty()) {
            const KEPT_PER_SESSION: usize = 20;
            let mut kept = self.retained_errors.borrow_mut();
            let errors = kept.entry(session.to_string()).or_default();
            if errors.len() == KEPT_PER_SESSION {
                errors.remove(0);
            }
            errors.push(message.to_string());
        }
    }

    fn restore_errors(&self, session: &str) {
        let errors = self
            .retained_errors
            .borrow()
            .get(session)
            .cloned()
            .unwrap_or_default();
        for message in errors {
            self.sheet.show_error(&message);
        }
    }

    fn restore_rejection(&self, ui: &AppWindow, sheet: &sheet::Sheet, session: &str) {
        let text = self.retained_rejections.borrow_mut().remove(session);
        if let Some(text) = text {
            self.composer.on_prompt_rejected(ui, sheet, &text);
        }
    }

    /// One pass: drain what the worker sent, then let the window catch up.
    pub fn run(&self) {
        let now = SystemTime::now();
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
        if ui.get_busy() && l.busy_label.is_empty() {
            let label = ui.get_busy_label().to_string();
            if !label.is_empty() {
                l.busy_label = label;
                busy_since.set(SystemTime::now());
            }
        }

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

        self.follow_selection(&ui, &mut l, sidebar, sheet, composer, jobs, busy_since);

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
                    let before = sidebar.cache.borrow().clone();
                    let was_seeded = self.roster_seeded.replace(true);
                    let mut skip_finished = Vec::new();
                    self.announced_sessions.borrow_mut().retain(|id| {
                        let still_running = rows
                            .iter()
                            .any(|row| row.id == *id && row.roster_status() == "Running");
                        if !still_running {
                            skip_finished.push(id.clone());
                        }
                        still_running
                    });
                    if let Some(rebound) = sidebar.apply_roster(&ui, rows.clone()) {
                        l.loaded = Some(rebound);
                    }
                    if was_seeded {
                        let toast = self.toast.borrow().clone();
                        for (id, name, outcome) in crate::notify::finished_top_level_rows(
                            &before,
                            &rows,
                            &skip_finished,
                        ) {
                            crate::notify::dispatch(&ui, toast.as_ref(), &id, &name, outcome);
                        }
                    }
                    let selected = ui.get_selected_id().to_string();
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        models.sync_session_row(&ui, row);
                    }
                    if !ui.get_draft_chat() && !rows.iter().any(|row| row.id == selected) {
                        l.settled_session = None;
                        if ui.get_busy() {
                            self.clear_status(&ui, &mut l);
                        }
                    }
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        let state = row.live_state();
                        let state_changed = state != l.live_state;
                        if state_changed {
                            l.live_state = state.to_string();
                            l.live_since = SystemTime::now();
                        }
                        let busy_now = state != "idle" && state != "archived";
                        // The engine's terminal frame has already ended this
                        // Session's Turn. A roster snapshot that still says it
                        // is running is older than that frame (a dropped idle
                        // push, or a list that crossed the end); it must not put
                        // the strip back up. Only a new Turn may do that.
                        let settled = l.settled_session.as_deref() == Some(selected.as_str());
                        // A roster answer can cross a prompt that has just been
                        // accepted but not yet picked up. While that send is in
                        // flight, an idle row is stale; keep the strip and its
                        // command word up until the turn starts or the send fails.
                        let sending = composer.in_flight.get()
                            && busy_since.get().elapsed().unwrap_or_default()
                                < Duration::from_secs(5);
                        if !busy_now && !sending {
                            let was_busy = ui.get_busy();
                            self.clear_status(&ui, &mut l);
                            // The roster has reported the Session idle: the
                            // settlement has served its purpose, and the next
                            // running row is a new Turn.
                            l.settled_session = None;
                            if was_busy {
                                if let Some(session) = l.loaded.clone() {
                                    let _ = jobs.send(Job::Tail {
                                        session,
                                        fresh: false,
                                    });
                                }
                            }
                        }
                        if busy_now && !settled {
                            // A slash command's word outranks the stream's
                            // generic Thinking/Writing labels; compaction
                            // outranks both while the daemon reports it.
                            let desired = if let Some(label) = self.command_word.word_for_state(state) {
                                Some(label.to_string())
                            } else if state_changed || l.busy_label.is_empty() {
                                Some(state.to_string())
                            } else {
                                None
                            };
                            if let Some(label) = desired {
                                if label != l.busy_label {
                                    l.busy_label = label;
                                    busy_since.set(SystemTime::now());
                                    ui.set_busy_label(l.busy_label.clone().into());
                                }
                            }
                            if !ui.get_busy() {
                                if l.busy_label.is_empty() {
                                    l.busy_label = state.to_string();
                                    busy_since.set(SystemTime::now());
                                    ui.set_busy_label(l.busy_label.clone().into());
                                }
                                ui.set_busy(true);
                            }
                        }
                        if ui.get_busy() {
                            let held = busy_since.get().elapsed().unwrap_or_default().as_secs();
                            ui.set_busy_elapsed(format!("{held}s").into());
                        }
                    }
                }
                Msg::Sessions(Err(_)) => {}
                Msg::DirectLinkLost { session } => {
                    if ui.get_selected_id().to_string() == session {
                        l.loaded = None;
                    }
                }
                Msg::SessionBinding { previous, active } => {
                    if let Some(active) = binding_selection(&ui.get_selected_id(), &previous, &active)
                    {
                        ui.set_selected_id(active.into());
                        l.loaded = None;
                    }
                }
                Msg::Activity { label } => {
                    // The TUI only changes the loader's word while a loader is
                    // already running. An activity frame is not a Turn start:
                    // a late frame from the Turn that has just ended must not
                    // wake an idle strip.
                    if !ui.get_busy() || ui.get_draft_chat() {
                        continue;
                    }
                    let desired = self.command_word.displayed(&l.live_state, &label);
                    if desired != l.busy_label {
                        l.busy_label = desired;
                        busy_since.set(SystemTime::now());
                        ui.set_busy_label(l.busy_label.clone().into());
                    }
                }
                Msg::Transcript {
                    session,
                    rows,
                    skipped,
                    messages,
                } => {
                    if !session_is_current(&ui, &session) {
                        continue;
                    }
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
                    sheet::force_scroll_to_end(&ui);
                    self.restore_errors(&session);
                    self.restore_rejection(&ui, sheet, &session);
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
                    let _ = jobs.send(Job::Stats {
                        session: session.clone(),
                    });
                    let _ = jobs.send(Job::Queue {
                        session: session.clone(),
                    });
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
                    if !session_is_current(&ui, &session) {
                        continue;
                    }
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
                    session,
                    rows,
                    messages,
                    first_entry,
                    more,
                } => {
                    if !session_is_current(&ui, &session) {
                        continue;
                    }
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
                    if !session_is_current(&ui, &session) {
                        continue;
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
                    if kind == Some("message_start")
                        && event
                            .pointer("/message/role")
                            .and_then(serde_json::Value::as_str)
                            == Some("user")
                    {
                        // A user message is a new Turn: the previous Turn's
                        // settlement no longer holds the strip down.
                        l.settled_session = None;
                        sheet::force_scroll_to_end(&ui);
                    }
                    if matches!(kind, Some("turn_start" | "agent_start")) {
                        self.turn_failed.set(false);
                        self.turn_aborted.set(false);
                        self.announced_sessions.borrow_mut().remove(&session);
                        open_status(&ui, &mut l, busy_since, "Waiting");
                    }
                    if kind == Some("compaction_start") {
                        force_status(&ui, &mut l, busy_since, "Compacting");
                    }
                    if kind == Some("auto_retry_start") {
                        open_status(&ui, &mut l, busy_since, "Waiting");
                    }
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
                    if kind == Some("turn_end") {
                        if turn_aborted(&event) {
                            self.turn_aborted.set(true);
                        }
                        if let Some(error) = turn_failure(&event) {
                            self.turn_failed.set(true);
                            let current = ui.get_selected_id();
                            let failed = if current.is_empty() || session == current.as_str() {
                                self.command_word.failure_label().unwrap_or("Turn failed")
                            } else {
                                "Turn failed"
                            };
                            self.record_error(
                                &ui,
                                Some(&session),
                                &format!("{failed}: {error}"),
                            );
                        }
                    }
                    if kind == Some("compaction_end") {
                        if let Some(error) = compaction_failure(&event) {
                            self.turn_failed.set(true);
                            self.record_error(&ui, Some(&session), &error);
                        }
                    }
                    if kind == Some("agent_end") {
                        let aborted = self.turn_aborted.replace(false);
                        let failed = self.turn_failed.replace(false);
                        if !aborted {
                            let rows = sidebar.cache.borrow();
                            let top_level = rows
                                .iter()
                                .find(|row| row.id == session)
                                .is_none_or(crate::sessions::is_top_level);
                            if top_level
                                && self.announced_sessions.borrow_mut().insert(session.clone())
                            {
                                let name = crate::notify::session_label(&rows, &session);
                                let outcome = if failed {
                                    crate::notify::TurnOutcome::Failed
                                } else {
                                    crate::notify::TurnOutcome::Finished
                                };
                                let toast = self.toast.borrow().clone();
                                crate::notify::dispatch(&ui, toast.as_ref(), &session, &name, outcome);
                            }
                        }
                    }
                    if matches!(kind, Some("turn_end" | "agent_end" | "compaction_end")) {
                        composer.in_flight.set(false);
                        sheet.answer_ended();
                        l.open_live = None;
                        l.settled_session = Some(session.clone());
                        self.clear_status(&ui, &mut l);
                        let _ = jobs.send(Job::Tail {
                            session: session.clone(),
                            fresh: true,
                        });
                        let _ = jobs.send(Job::Stats {
                            session: session.clone(),
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
                        // The stream says "Thinking" for every message, including
                        // the command that started the turn. Keep the command's
                        // own word on the status strip for as long as it runs.
                        // Only a turn already in flight counts: a replayed user
                        // message must not wake an idle strip.
                        if ui.get_busy() {
                            if let Some(label) = self.command_word.hold(&text) {
                                if l.busy_label != label {
                                    l.busy_label = label.to_string();
                                    busy_since.set(SystemTime::now());
                                    ui.set_busy_label(label.into());
                                }
                            }
                        }
                        let stamp = message
                            .get("timestamp")
                            .and_then(serde_json::Value::as_i64)
                            .unwrap_or(0);
                        let key = format!(
                            "{stamp}:{}:{}",
                            text.chars().count(),
                            text.chars().take(40).collect::<String>()
                        );
                        if kind == Some("message_start") {
                            let (rows, _) = transcript::build(std::slice::from_ref(message), 1);
                            if l.open_live.is_some() {
                                sheet.merge_after_open(rows);
                            } else {
                                sheet.merge(rows);
                            }
                        }
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
                    if message.get("customType").is_some() {
                        if kind == Some("message_start") {
                            let (rows, _) = transcript::build(std::slice::from_ref(message), 1);
                            let replayed = message
                                .pointer("/details/id")
                                .and_then(serde_json::Value::as_str)
                                .is_some_and(|id| !id.is_empty());
                            match (l.open_live.is_some(), replayed) {
                                (true, true) => sheet.merge_after_open(rows),
                                (true, false) => sheet.merge_new_after_open(rows),
                                (false, true) => sheet.merge(rows),
                                (false, false) => sheet.merge_new(rows),
                            }
                            scroll_if_parked(&ui);
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
                    session,
                    steering,
                    follow_up,
                } => {
                    if !session_is_current(&ui, &session) {
                        continue;
                    }
                    composer.project_queue(&ui, &steering, &follow_up);
                }
                Msg::Prompt {
                    session,
                    message,
                    images,
                } => {
                    l.settled_session = None;
                    composer.in_flight.set(true);
                    let _ = jobs.send(Job::Prompt {
                        session,
                        message,
                        images,
                    });
                }
                Msg::ChatStarted { id, generation } => {
                    if !ui.get_draft_chat() || generation != draft_generation(&ui) {
                        continue;
                    }
                    self.attach_generation.set(generation);
                    l.drawn_user = None;
                    l.loaded = Some(id.clone());
                    l.settled_session = None;
                    l.last_seq = 0;
                    l.open_live = None;
                    l.live_state.clear();
                    l.live_since = SystemTime::now();
                    composer.bind_new_chat(&ui, &id);
                    self.sheet.replace(Vec::new());
                    self.sheet.reset_paging(&ui);
                    self.sheet.clear_tail_sig();
                    self.sidebar.track_selection(&id);
                }
                Msg::StartChatFailed {
                    generation,
                    session,
                    text,
                    images,
                    error,
                } => {
                    if generation != draft_generation(&ui) {
                        continue;
                    }
                    let on_screen = match session.as_deref() {
                        Some(session) => {
                            !ui.get_draft_chat() && ui.get_selected_id().as_str() == session
                        }
                        None => ui.get_draft_chat(),
                    };
                    if !on_screen {
                        continue;
                    }
                    composer.on_start_chat_rejected(&ui, &text, &images);
                    l.settled_session = None;
                    self.clear_status(&ui, &mut l);
                    self.record_error(&ui, session.as_deref(), &error);
                }
                Msg::QueueEdit { text } => {
                    if ui.get_draft_chat() {
                        continue;
                    }
                    ui.set_composer_text(text.into());
                    ui.set_composer_has_text(!ui.get_composer_text().is_empty());
                }
                Msg::ExtraCommands(rows) => {
                    composer.set_extra_commands(rows);
                }
                Msg::ModelCatalogChanged => {
                    models.invalidate();
                    let selected = ui.get_selected_id().to_string();
                    if !selected.is_empty() {
                        let _ = jobs.send(Job::ModelCatalog { session: selected });
                    }
                }
                Msg::ModelCatalog(entries) => {
                    models.set_catalog(&ui, entries);
                }
                Msg::Stats { session, stats } => {
                    if !session_is_current(&ui, &session) {
                        continue;
                    }
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
                Msg::ActivityCounts {
                    session,
                    subagents,
                    heartbeats,
                    shells,
                } => {
                    if session == ui.get_selected_id().as_str() {
                        if let Some(count) = subagents {
                            ui.set_activity_subagents(count_i32(count));
                        }
                        if let Some(count) = heartbeats {
                            ui.set_activity_heartbeats(count_i32(count));
                        }
                        if let Some(count) = shells {
                            ui.set_activity_shells(count_i32(count));
                        }
                    }
                }
                Msg::State { state, .. } => {
                    if ui.get_draft_chat() {
                        continue;
                    }
                    models.sync_session_state(&ui, &state);
                }
                Msg::Reconnected => {
                    self.roster_seeded.set(false);
                    self.turn_failed.set(false);
                    self.turn_aborted.set(false);
                    self.announced_sessions.borrow_mut().clear();
                    l.loaded = None;
                    l.open_live = None;
                    l.settled_session = None;
                    self.clear_status(&ui, &mut l);
                    composer.in_flight.set(false);
                    sheet.answer_ended();
                    sheet.clear_tail_sig();
                    l.last_seq = 0;
                }
                Msg::PromptRejected { session, text } => {
                    if ui.get_draft_chat() {
                        continue;
                    }
                    let current = ui.get_selected_id();
                    if current.is_empty()
                        || session.as_deref().is_none_or(|target| target == current.as_str())
                    {
                        l.settled_session = None;
                        self.clear_status(&ui, &mut l);
                        composer.on_prompt_rejected(&ui, sheet, &text);
                    } else if let Some(session) = session.filter(|session| !session.is_empty()) {
                        self.retained_rejections
                            .borrow_mut()
                            .insert(session, text);
                    }
                }
                Msg::PromptResult { session, error } => {
                    if ui.get_draft_chat() {
                        continue;
                    }
                    let current = ui.get_selected_id();
                    let on_screen = current.is_empty()
                        || session.as_deref().is_none_or(|target| target == current.as_str());
                    let failed = error.is_some();
                    if failed && on_screen {
                        self.command_word.clear();
                    }
                    if on_screen {
                        // The take-back removes the user's echo while it is still
                        // the last Row; append the error only after that.
                        composer.on_prompt_result(sheet, failed);
                    }
                    if let Some(error) = error {
                        self.record_error(&ui, session.as_deref(), &error);
                    }
                }
                Msg::Error { session, message } => {
                    self.record_error(&ui, session.as_deref(), &message);
                }
                Msg::SessionClosed { session, reason } => {
                    let message = session_closed_message(&reason);
                    self.record_error(&ui, Some(&session), &message);
                    if !ui.get_draft_chat() {
                        let current = ui.get_selected_id();
                        if current.is_empty() || session == current.as_str() {
                            composer.in_flight.set(false);
                            sheet.answer_ended();
                            l.open_live = None;
                            l.settled_session = None;
                            self.clear_status(&ui, &mut l);
                        }
                    }
                    let rows = sidebar.cache.borrow();
                    let top_level = rows
                        .iter()
                        .find(|row| row.id == session)
                        .is_none_or(crate::sessions::is_top_level);
                    let failure_close = !matches!(reason.as_str(), "shutdown" | "replaced");
                    if top_level
                        && self.announced_sessions.borrow_mut().insert(session.clone())
                        && failure_close
                    {
                        let name = crate::notify::session_label(&rows, &session);
                        let toast = self.toast.borrow().clone();
                        crate::notify::dispatch(
                            &ui,
                            toast.as_ref(),
                            &session,
                            &name,
                            crate::notify::TurnOutcome::Failed,
                        );
                    }
                }
                Msg::Shared(Ok(shared)) => {
                    // The card opened when the menu item was taken; fill it with
                    // the link the reader came for, and say whether the window
                    // already put it on the clipboard.
                    let copied = crate::sessions::copy_to_clipboard(&shared.viewer_url);
                    let copy_error = copied
                        .as_ref()
                        .err()
                        .map(|error| format!("could not copy: {error}"))
                        .unwrap_or_default();
                    ui.set_share_pending(false);
                    ui.set_share_url(shared.viewer_url.into());
                    ui.set_share_error("".into());
                    ui.set_share_copied(copied.is_ok());
                    ui.set_share_action_error(copy_error.into());
                }
                Msg::Shared(Err(error)) => {
                    ui.set_share_pending(false);
                    ui.set_share_url("".into());
                    ui.set_share_error(error.into());
                    ui.set_share_action_error("".into());
                    ui.set_share_copied(false);
                }
                Msg::ViewerQuota(Ok(quota)) if quota.remaining <= 0 => {
                    // The card's Open control is about to hand the reader to a
                    // page that spends this quota. Say what is wrong instead of
                    // opening a viewer that can only show GitHub's raw 403.
                    let minutes = quota.wait_minutes();
                    let message = if minutes > 1 {
                        format!(
                            "GitHub's public API quota is exhausted on this network. \
                             The viewer will fail until it resets in about {minutes} minutes."
                        )
                    } else if minutes == 1 {
                        "GitHub's public API quota is exhausted on this network. \
                         The viewer will fail until it resets in about 1 minute."
                            .to_string()
                    } else {
                        "GitHub's public API quota is exhausted on this network. \
                         The viewer will fail until it resets."
                            .to_string()
                    };
                    ui.set_share_action_error(message.into());
                }
                Msg::ViewerQuota(_) => {
                    if !ui.get_share_open() {
                        continue;
                    }
                    let url = ui.get_share_url().to_string();
                    if url.is_empty() {
                        continue;
                    }
                    ui.set_share_action_error("".into());
                    if let Err(error) = crate::share::open_url(&url) {
                        ui.set_share_action_error(error.into());
                    }
                }
            }
        }

        // A roster/session-binding message can move the selection. Attach the
        // new Session in this pass instead of waiting for another wake.
        self.follow_selection(&ui, &mut l, sidebar, sheet, composer, jobs, busy_since);
        l.store(self);
    }
}

/// The TUI's close-reason table. A known close is a specific Error row; an
/// unknown reason still says the Session closed rather than dropping the frame.
fn session_closed_message(reason: &str) -> String {
    match reason {
        "shutdown" => "The Prime Agent daemon shut down while this window was attached. The session transcript remains saved; restart Prime Agent and reopen it from Agents View.".to_string(),
        "replaced" => "The daemon replaced this agent session with another session. Reopen the current session from Agents View.".to_string(),
        "killed" => "The daemon stopped this agent session. Its transcript remains saved and can be reopened from Agents View.".to_string(),
        "" => "The daemon closed this session. Its transcript remains saved.".to_string(),
        other => format!("The daemon closed this session ({other}). Its transcript remains saved."),
    }
}

/// The failure a `turn_end` carries. A provider refusal is on the empty
/// assistant message (`message.errorMessage`, `stopReason: "error"`), while an
/// engine failure can arrive as the top-level `error`. A deliberate Stop is
/// not an error row: its aborted message already says the Turn was stopped.
fn turn_aborted(event: &serde_json::Value) -> bool {
    let message = event.get("message");
    event.get("aborted").and_then(serde_json::Value::as_bool) == Some(true)
        || message
            .and_then(|message| message.get("stopReason"))
            .and_then(serde_json::Value::as_str)
            == Some("aborted")
}

fn turn_failure(event: &serde_json::Value) -> Option<String> {
    let message = event.get("message");
    if turn_aborted(event) {
        return None;
    }
    event
        .get("error")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            message
                .and_then(|message| message.get("errorMessage"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|error| !error.trim().is_empty())
        .map(str::to_string)
}

/// A compaction failure arrives as `compaction_end`, not as a Turn error.
/// The daemon's message may already start with "Compaction failed:"; keep one
/// operation prefix rather than stacking "Compacting failed: Compaction failed:".
fn compaction_failure(event: &serde_json::Value) -> Option<String> {
    if event
        .get("errorSeverity")
        .and_then(serde_json::Value::as_str)
        != Some("error")
    {
        return None;
    }
    let error = event
        .get("errorMessage")
        .and_then(serde_json::Value::as_str)
        .filter(|error| !error.trim().is_empty())?;
    if let Some(rest) = error.strip_prefix("Compaction failed:") {
        return Some(format!("Compacting failed:{rest}"));
    }
    Some(format!("Compacting failed: {error}"))
}

#[cfg(test)]
mod status_strip_tests {
    use super::*;

    struct Harness {
        ui: AppWindow,
        tick: Rc<Tick>,
        tx: Sender<(u64, Msg)>,
        jobs: Receiver<Job>,
    }

    impl Harness {
        fn new() -> Self {
            let ui = AppWindow::new().expect("window");
            let (job_tx, jobs) = channel::<Job>();
            let (tx, rx) = channel::<(u64, Msg)>();
            let (sheet, transcript) = sheet::Sheet::new();
            let sheet = Rc::new(sheet);
            sheet.bind_ui(&ui);
            ui.set_transcript(transcript);
            let models = models::ModelMenu::new(&ui, &job_tx);
            let command_word = CommandWord::new();
            let composer = composer::Composer::new(&ui, &job_tx, &models, command_word.clone());
            let sidebar = sessions::Sidebar::new(&ui, &job_tx);
            let busy_since = Rc::new(std::cell::Cell::new(SystemTime::now()));
            let tick = Rc::new(Tick {
                weak: ui.as_weak(),
                jobs: job_tx,
                msg_rx: rx,
                attach_generation: std::cell::Cell::new(0),
                sheet,
                sidebar,
                composer,
                command_word,
                models,
                busy_since,
                window_rows: 300,
                drawn_user: std::cell::Cell::new(None),
                loaded: std::cell::Cell::new(None),
                live_state: std::cell::Cell::new(String::new()),
                live_since: std::cell::Cell::new(SystemTime::now()),
                busy_label: std::cell::Cell::new(String::new()),
                settled_session: std::cell::Cell::new(None),
                last_seq: std::cell::Cell::new(0),
                open_live: std::cell::Cell::new(None),
                stream_started: std::cell::Cell::new(SystemTime::now()),
                scroll_ticks: std::cell::Cell::new(0),
                retained_errors: std::cell::RefCell::new(std::collections::HashMap::new()),
                retained_rejections: std::cell::RefCell::new(std::collections::HashMap::new()),
                roster_seeded: std::cell::Cell::new(false),
                turn_failed: std::cell::Cell::new(false),
                turn_aborted: std::cell::Cell::new(false),
                announced_sessions: std::cell::RefCell::new(std::collections::HashSet::new()),
                toast: std::cell::RefCell::new(Rc::new(crate::notify::WindowsToastNotifier)),
            });
            Self {
                ui,
                tick,
                tx,
                jobs,
            }
        }

        fn send(&self, message: Msg) {
            self.tx.send((0, message)).expect("message");
        }

        fn send_at(&self, generation: u64, message: Msg) {
            self.tx.send((generation, message)).expect("message");
        }

        fn run(&self) {
            self.tick.run();
        }
    }

    fn user_start(text: &str) -> Msg {
        Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": text}]
                }
            }),
        }
    }

    fn turn_end() -> Msg {
        Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "turn_end"}),
        }
    }

    fn turn_start(kind: &str) -> Msg {
        Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": kind}),
        }
    }

    fn take_jobs(h: &Harness) -> Vec<Job> {
        let mut jobs = Vec::new();
        while let Ok(job) = h.jobs.try_recv() {
            jobs.push(job);
        }
        jobs
    }

    fn assert_strip(h: &Harness, busy: bool, label: &str) {
        assert_eq!(h.ui.get_busy(), busy);
        assert_eq!(h.ui.get_busy_label().as_str(), label);
    }

    fn attach(h: &Harness, session: &str) {
        h.ui.set_selected_id(session.into());
        h.run();
        h.send_at(
            h.tick.attach_generation.get(),
            Msg::Transcript {
                session: session.to_string(),
                rows: Vec::new(),
                skipped: 0,
                messages: Vec::new(),
            },
        );
        h.run();
    }

    fn saved_session_row(id: &str, name: &str, worker_state: &str, cwd: &str) -> SessionRow {
        SessionRow {
            id: id.to_string(),
            session_id: id.to_string(),
            name: name.to_string(),
            cwd: cwd.to_string(),
            lifecycle: "draft".to_string(),
            worker_state: worker_state.to_string(),
            ..SessionRow::default()
        }
    }

    fn drawn_session_ids(h: &Harness) -> Vec<String> {
        h.ui.get_sessions()
            .iter()
            .map(|row| row.id.to_string())
            .collect()
    }

    fn error_rows(h: &Harness) -> Vec<String> {
        h.tick
            .sheet
            .rows()
            .iter()
            .filter(|row| row.kind == crate::transcript::KIND_ERROR)
            .map(|row| row.body.clone())
            .collect()
    }

    fn a_command_word_survives_the_streams_thinking_activity() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        assert_strip(&h, true, "Refining");

        h.send(Msg::Activity {
            label: "Thinking".to_string(),
        });
        h.run();
        assert_strip(&h, true, "Refining");
    }

    fn a_turn_end_takes_the_command_word_off_the_status_strip() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/compact"));
        h.run();
        assert_strip(&h, true, "Compacting");

        h.send(turn_end());
        h.run();
        assert_strip(&h, false, "");
        assert_eq!(h.tick.command_word.word_for_state("Thinking"), None);
    }

    fn a_rejected_prompt_takes_the_status_strip_down() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        assert_strip(&h, true, "Refining");

        h.send(Msg::PromptRejected {
            session: None,
            text: "/refine".to_string(),
        });
        h.run();
        assert_strip(&h, false, "");
    }

    fn a_reconnect_takes_the_status_strip_down() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        assert_strip(&h, true, "Refining");

        h.send(Msg::Reconnected);
        h.run();
        assert_strip(&h, false, "");
    }

    fn a_new_chat_takes_the_status_strip_down() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/compact"));
        h.run();
        assert_strip(&h, true, "Compacting");

        h.tick.open_new_chat();
        h.run();
        assert_strip(&h, false, "");
    }

    fn roster_state_for(session: &str, running: bool) -> Msg {
        Msg::Sessions(Ok(vec![SessionRow {
            id: session.to_string(),
            session_id: session.to_string(),
            name: session.to_string(),
            activity: if running { "working" } else { "idle" }.to_string(),
            lifecycle: "live".to_string(),
            is_streaming: running,
            is_session_active: running,
            is_running_tools: running,
            ..SessionRow::default()
        }]))
    }

    fn roster_state(running: bool) -> Msg {
        roster_state_for("s", running)
    }

    /// A roster snapshot that still says the Session is running can cross the
    /// turn's own `turn_end`. It must not put the strip back up: the engine has
    /// already said the Turn is over, and the stream's terminal frame is the
    /// authority the TUI follows too.
    fn a_stale_roster_snapshot_does_not_reopen_the_strip_after_turn_end() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        assert_strip(&h, true, "Executing");

        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.send(roster_state(true));
        h.run();
        assert_strip(&h, false, "");
    }

    /// The TUI only changes the loader's word while a Turn is running. An
    /// activity frame on its own is not a Turn start; a late frame from the
    /// previous Turn must not wake an idle strip.
    fn an_activity_frame_does_not_open_the_strip_by_itself() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(Msg::Activity {
            label: "Writing code".to_string(),
        });
        h.run();
        assert_strip(&h, false, "");
    }

    fn switching_sessions_moves_the_strip_to_the_new_selection() {
        let h = Harness::new();
        {
            let mut cache = h.tick.sidebar.cache.borrow_mut();
            cache.push(SessionRow {
                id: "busy".to_string(),
                session_id: "uuid-busy".to_string(),
                is_session_active: true,
                ..SessionRow::default()
            });
            cache.push(SessionRow {
                id: "calm".to_string(),
                session_id: "uuid-calm".to_string(),
                ..SessionRow::default()
            });
        }

        h.ui.set_selected_id("busy".into());
        h.run();
        assert_strip(&h, true, "Waiting");

        h.ui.set_selected_id("calm".into());
        h.run();
        assert_strip(&h, false, "");
    }

    /// The settlement is released by the next Turn, not forever: once a new
    /// prompt starts, the roster's running state is the current Turn again.
    fn switching_sessions_clears_the_settlement() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.ui.set_selected_id("b".into());
        h.run();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        assert_strip(&h, true, "Executing");
    }

    fn a_composer_send_releases_the_settlement_on_agent_start() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.ui.invoke_send_requested("next".into());
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_start("agent_start"));
        h.run();
        assert_strip(&h, true, "Waiting");
        assert!(
            h.tick.settled_session.take().is_none(),
            "agent_start released the previous Turn's settlement"
        );
    }

    fn an_idle_roster_releases_the_settlement() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.send(roster_state(false));
        h.run();
        h.send(roster_state(true));
        h.run();
        assert_strip(&h, true, "Executing");
    }

    fn an_auto_retry_start_after_settlement_opens_the_strip() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.send_at(h.tick.attach_generation.get(), turn_start("auto_retry_start"));
        h.run();
        assert_strip(&h, true, "Waiting");
    }

    fn a_new_turn_start_after_settlement_opens_the_strip() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");
        h.ui.set_busy_elapsed("99s".into());

        h.send_at(h.tick.attach_generation.get(), turn_start("turn_start"));
        h.run();
        assert_strip(&h, true, "Waiting");
        assert_eq!(h.ui.get_busy_elapsed().as_str(), "0s");
    }

    fn a_compaction_start_after_settlement_opens_the_strip() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.send_at(h.tick.attach_generation.get(), turn_start("compaction_start"));
        h.run();
        assert_strip(&h, true, "Compacting");

        // Compaction outranks a running loader, as it does in the TUI.
        h.ui.set_busy_label("Writing".into());
        h.send_at(h.tick.attach_generation.get(), turn_start("compaction_start"));
        h.run();
        assert_strip(&h, true, "Compacting");
    }

    fn a_new_prompt_lets_the_roster_open_the_strip_again() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        h.send(roster_state(true));
        h.run();
        assert_strip(&h, true, "Executing");

        h.send_at(h.tick.attach_generation.get(), turn_end());
        h.run();
        assert_strip(&h, false, "");

        h.send_at(
            h.tick.attach_generation.get(),
            Msg::Prompt {
                session: "s".to_string(),
                message: "next".to_string(),
                images: Vec::new(),
            },
        );
        h.run();
        h.send(roster_state(true));
        h.run();
        assert_strip(&h, true, "Executing");
    }

    fn an_error_for_another_session_does_not_land_in_this_sheet() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());

        h.send(Msg::Error {
            session: Some("a".to_string()),
            message: "Session a failed".to_string(),
        });
        h.run();
        assert!(h.tick.sheet.rows().is_empty());

        h.send(Msg::Error {
            session: Some("b".to_string()),
            message: "Session b failed".to_string(),
        });
        h.run();
        let rows = h.tick.sheet.rows();
        let error = rows.last().expect("the current Session's error landed");
        assert!(error.body.contains("Session b failed"));
    }

    fn an_error_for_another_session_is_replayed_when_that_session_opens() {
        let h = Harness::new();
        attach(&h, "a");
        attach(&h, "b");

        h.send(Msg::Error {
            session: Some("a".to_string()),
            message: "Session a failed while it was hidden".to_string(),
        });
        h.run();
        assert!(error_rows(&h).is_empty());

        attach(&h, "a");
        let errors = error_rows(&h);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Session a failed while it was hidden"));
    }

    fn an_error_already_shown_comes_back_when_the_reader_returns() {
        let h = Harness::new();
        attach(&h, "a");
        h.send(Msg::Error {
            session: Some("a".to_string()),
            message: "Session a failed on screen".to_string(),
        });
        h.run();
        assert_eq!(error_rows(&h).len(), 1);

        attach(&h, "b");
        assert!(error_rows(&h).is_empty());

        attach(&h, "a");
        let errors = error_rows(&h);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("Session a failed on screen"));
    }

    fn errors_from_two_sessions_do_not_mix() {
        let h = Harness::new();
        attach(&h, "a");
        h.send(Msg::Error {
            session: Some("a".to_string()),
            message: "a failed".to_string(),
        });
        h.run();
        attach(&h, "b");
        h.send(Msg::Error {
            session: Some("b".to_string()),
            message: "b failed".to_string(),
        });
        h.run();

        let shown = error_rows(&h);
        assert_eq!(shown.len(), 1);
        assert!(shown[0].contains("b failed"));

        attach(&h, "a");
        let shown = error_rows(&h);
        assert_eq!(shown.len(), 1);
        assert!(shown[0].contains("a failed"));
        assert!(!shown[0].contains("b failed"));
    }

    fn a_refusal_for_another_session_does_not_touch_the_current_sheet() {
        let h = Harness::new();
        attach(&h, "a");
        attach(&h, "b");

        h.send(Msg::PromptRejected {
            session: Some("a".to_string()),
            text: "the refused prompt".to_string(),
        });
        h.run();
        assert_eq!(h.ui.get_composer_text().as_str(), "");

        h.send(Msg::PromptResult {
            session: Some("a".to_string()),
            error: Some("session a refused the prompt".to_string()),
        });
        h.run();
        assert!(error_rows(&h).is_empty());
        assert_eq!(h.ui.get_composer_text().as_str(), "");

        attach(&h, "a");
        let errors = error_rows(&h);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("session a refused the prompt"));
        assert_eq!(h.ui.get_composer_text().as_str(), "the refused prompt");
    }

    fn a_failed_tool_stays_a_failed_cell_not_an_error_row() {
        let h = Harness::new();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "m1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "tool_execution_start",
                "toolCallId": "call_1",
                "toolName": "bash",
                "args": {"code": "exit 1"}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "tool_execution_end",
                "toolCallId": "call_1",
                "isError": true,
                "result": {"content": [{"type": "text", "text": "exit 1"}]}
            }),
        });
        h.run();

        let rows = h.tick.sheet.rows();
        assert!(rows.iter().any(|row| row.kind == crate::transcript::KIND_CELL && row.failed));
        assert!(!rows.iter().any(|row| row.kind == crate::transcript::KIND_ERROR));
    }

    fn a_session_close_is_an_error_row_and_stops_the_turn() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.ui.set_busy(true);
        h.ui.set_busy_label("Refining".into());

        h.send(Msg::SessionClosed {
            session: "s".to_string(),
            reason: "killed".to_string(),
        });
        h.run();

        assert_strip(&h, false, "");
        let errors = error_rows(&h);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("daemon stopped this agent session"));
    }

    fn a_session_close_for_another_session_is_retained() {
        let h = Harness::new();
        attach(&h, "a");
        attach(&h, "b");

        h.send(Msg::SessionClosed {
            session: "a".to_string(),
            reason: "shutdown".to_string(),
        });
        h.run();
        assert!(error_rows(&h).is_empty());

        attach(&h, "a");
        let errors = error_rows(&h);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("daemon shut down"));
    }

    fn a_failed_refine_names_the_command_in_the_sheet() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        assert_strip(&h, true, "Refining");

        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "turn_end",
                "message": {
                    "stopReason": "error",
                    "errorMessage": "provider returned 429"
                }
            }),
        });
        h.run();

        assert_strip(&h, false, "");
        let rows = h.tick.sheet.rows();
        let error = rows.last().expect("the failure row landed");
        assert_eq!(error.kind, crate::transcript::KIND_ERROR);
        assert!(error.body.contains("Refining failed: provider returned 429"));
    }

    fn a_deliberate_stop_does_not_write_an_error_row() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "turn_end",
                "message": { "stopReason": "aborted", "errorMessage": "No response produced." }
            }),
        });
        h.run();
        assert!(!h
            .tick
            .sheet
            .rows()
            .iter()
            .any(|row| row.kind == crate::transcript::KIND_ERROR));
    }

    fn a_failed_compaction_names_the_operation() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/compact"));
        h.run();
        assert_strip(&h, true, "Compacting");
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "compaction_end",
                "errorSeverity": "error",
                "errorMessage": "Compaction failed: context too large"
            }),
        });
        h.run();
        let rows = h.tick.sheet.rows();
        let error = rows.last().expect("the compaction failure landed");
        assert_eq!(error.kind, crate::transcript::KIND_ERROR);
        assert!(error.body.contains("Compacting failed: context too large"));
        assert!(!error.body.contains("Compaction failed"));
    }

    fn an_error_lands_in_the_sheet_without_touching_the_strip() {
        let h = Harness::new();
        h.ui.set_busy(true);
        h.send(user_start("/refine"));
        h.run();
        assert_strip(&h, true, "Refining");

        h.send(Msg::Error {
            session: None,
            message: "Background shell command failed: exit 1".to_string(),
        });
        h.run();

        assert_strip(&h, true, "Refining");
        let rows = h.tick.sheet.rows();
        let error = rows.last().expect("the error row landed");
        assert_eq!(error.kind, crate::transcript::KIND_ERROR);
        assert!(error.body.contains("Background shell command failed: exit 1"));
    }

    fn a_parked_command_does_not_claim_the_running_turn() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.ui.set_busy(true);
        h.ui.set_busy_label("Thinking".into());

        h.ui.invoke_send_requested("/refine".into());

        assert_strip(&h, true, "Thinking");
        assert_eq!(h.tick.command_word.word_for_state("Thinking"), None);
        match h.jobs.try_recv() {
            Ok(Job::Prompt { message, .. }) => assert_eq!(message, "/refine"),
            _ => panic!("the composer did not send the command"),
        }
    }

    // One test, not six: Slint binds its platform to the test thread, and the
    // harness runs each test on a new thread. The scenarios share this thread.
    #[test]
    fn the_status_strip_holds_and_clears_command_words() {
        let settings_path = std::env::temp_dir().join(format!(
            "pa-ui-window-settings-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&settings_path);
        std::env::set_var("PA_SETTINGS_FILE", &settings_path);
        a_command_word_survives_the_streams_thinking_activity();
        a_turn_end_takes_the_command_word_off_the_status_strip();
        a_rejected_prompt_takes_the_status_strip_down();
        a_reconnect_takes_the_status_strip_down();
        a_new_chat_takes_the_status_strip_down();
        a_stale_roster_snapshot_does_not_reopen_the_strip_after_turn_end();
        an_activity_frame_does_not_open_the_strip_by_itself();
        a_new_prompt_lets_the_roster_open_the_strip_again();
        a_new_turn_start_after_settlement_opens_the_strip();
        a_compaction_start_after_settlement_opens_the_strip();
        an_idle_roster_releases_the_settlement();
        an_auto_retry_start_after_settlement_opens_the_strip();
        switching_sessions_clears_the_settlement();
        switching_sessions_moves_the_strip_to_the_new_selection();
        a_composer_send_releases_the_settlement_on_agent_start();
        a_parked_command_does_not_claim_the_running_turn();
        an_error_lands_in_the_sheet_without_touching_the_strip();
        a_failed_refine_names_the_command_in_the_sheet();
        a_deliberate_stop_does_not_write_an_error_row();
        a_failed_compaction_names_the_operation();
        an_error_for_another_session_does_not_land_in_this_sheet();
        an_error_for_another_session_is_replayed_when_that_session_opens();
        an_error_already_shown_comes_back_when_the_reader_returns();
        errors_from_two_sessions_do_not_mix();
        a_refusal_for_another_session_does_not_touch_the_current_sheet();
        a_failed_tool_stays_a_failed_cell_not_an_error_row();
        a_session_close_is_an_error_row_and_stops_the_turn();
        a_session_close_for_another_session_is_retained();
        the_activity_readout_is_selected_session_owned();
        a_transcript_from_another_session_cannot_replace_the_sheet();
        a_tail_from_another_session_cannot_replace_the_sheet();
        an_older_page_from_another_session_cannot_enter_the_sheet();
        switching_sessions_clears_the_previous_sheet();
        attaching_a_session_resumes_follow_bottom();
        attaching_asks_for_stats_and_queue();
        a_queue_for_another_session_does_not_touch_the_lanes();
        a_new_user_prompt_brings_the_transcript_to_the_floor();
        a_user_prompt_is_drawn_before_its_reply();
        a_steering_prompt_follows_the_open_reply();
        a_custom_message_renders_live_and_reconciles_on_tail();
        a_second_custom_message_keeps_arrival_order();
        a_repeated_custom_body_gets_its_own_row();
        a_new_chat_is_local_until_the_first_send();
        a_model_picked_in_the_draft_does_not_reach_the_daemon();
        the_first_send_carries_the_draft_model_and_level();
        a_create_failure_keeps_the_unsaved_chat_and_its_prompt();
        a_prompt_failure_after_create_keeps_the_session_and_retry_state();
        the_sidebar_hides_only_inactive_anonymous_empty_saved_rows();
        a_stale_roster_cannot_steal_a_newly_bound_chat();
        a_stale_start_failure_does_not_touch_a_newer_unsaved_chat();
        a_retry_after_a_prompt_failure_sends_only_the_prompt();
        a_create_failure_restores_the_prompt_attachments();
        a_partial_model_choice_is_not_a_draft_model();
        a_rejection_keeps_new_composer_input_and_does_not_restore_old_images();
        opening_a_new_chat_inherits_the_selected_cwd();
        a_stale_start_result_does_not_replace_the_current_unsaved_chat();
        reopen_last_session_uses_the_client_side_selection();
        settings_popup_mutual_exclusion_and_persistence();
        settings_choices_expand_collapse_and_select();
        notifications_in_app_and_system_toast_adapter_follow_policy();
        let _ = std::fs::remove_file(&settings_path);
        std::env::remove_var("PA_SETTINGS_FILE");
    }

    fn the_activity_readout_is_selected_session_owned() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.send_at(1, Msg::ActivityCounts {
            session: "s".to_string(),
            subagents: Some(1),
            heartbeats: Some(2),
            shells: Some(1),
        });
        h.run();
        assert_eq!(h.ui.get_activity_subagents(), 1);
        assert_eq!(h.ui.get_activity_heartbeats(), 2);
        assert_eq!(h.ui.get_activity_shells(), 1);

        h.send_at(1, Msg::ActivityCounts {
            session: "other".to_string(),
            subagents: Some(9),
            heartbeats: Some(9),
            shells: Some(9),
        });
        h.run();
        assert_eq!(h.ui.get_activity_subagents(), 1);
        assert_eq!(h.ui.get_activity_heartbeats(), 2);
        assert_eq!(h.ui.get_activity_shells(), 1);

        // One failed half keeps the other half's last good value.
        h.send_at(1, Msg::ActivityCounts {
            session: "s".to_string(),
            subagents: None,
            heartbeats: None,
            shells: Some(0),
        });
        h.run();
        assert_eq!(h.ui.get_activity_subagents(), 1);
        assert_eq!(h.ui.get_activity_heartbeats(), 2);
        assert_eq!(h.ui.get_activity_shells(), 0);

        // A new selection resets the readout; the roster then supplies the
        // running-subagent number through the activity reply.
        h.ui.set_selected_id("parent".into());
        h.run();
        assert_eq!(h.ui.get_activity_subagents(), 0);
        h.send_at(2, Msg::ActivityCounts {
            session: "parent".to_string(),
            subagents: Some(2),
            heartbeats: Some(0),
            shells: Some(0),
        });
        h.run();
        assert_eq!(h.ui.get_activity_subagents(), 2);
    }

    fn rows_for(text: &str) -> Vec<transcript::Row> {
        transcript::build(
            &[serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": text}],
                "timestamp": 1 + text.len() as u64
            })],
            1,
        )
        .0
    }

    fn sheet_text(h: &Harness) -> String {
        h.tick
            .sheet
            .rows()
            .iter()
            .map(|row| transcript::plain(&row.body))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn messages_for(text: &str) -> Vec<serde_json::Value> {
        vec![serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": text}],
            "timestamp": 1 + text.len() as u64
        })]
    }

    fn switching_sessions_clears_the_previous_sheet() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "b".to_string(),
                rows: rows_for("the old session"),
                skipped: 0,
                messages: messages_for("the old session"),
            },
        );
        h.run();
        assert!(sheet_text(&h).contains("the old session"));

        h.ui.set_follow_bottom(false);
        h.ui.set_selected_id("a".into());
        h.run();
        assert!(
            h.tick.sheet.rows().is_empty(),
            "a new selection must not keep painting the old session"
        );
        assert!(
            h.ui.get_follow_bottom(),
            "a new selection must resume following the newest row"
        );
    }

    fn attaching_a_session_resumes_follow_bottom() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());
        h.run();
        h.ui.set_follow_bottom(false);
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "b".to_string(),
                rows: rows_for("the selected session"),
                skipped: 0,
                messages: messages_for("the selected session"),
            },
        );
        h.run();
        assert!(
            h.ui.get_follow_bottom(),
            "attaching a session must show its newest row"
        );
    }

    fn a_queue_for_another_session_does_not_touch_the_lanes() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Queue {
                session: "other".to_string(),
                steering: vec!["not mine".to_string()],
                follow_up: Vec::new(),
            },
        );
        h.run();
        assert_eq!(h.ui.get_queued_count(), 0);

        h.send_at(
            generation,
            Msg::Queue {
                session: "s".to_string(),
                steering: vec!["mine".to_string()],
                follow_up: Vec::new(),
            },
        );
        h.run();
        assert_eq!(h.ui.get_queued_count(), 1);
        assert_eq!(h.ui.get_queued_steering().row_count(), 1);
    }

    fn attaching_asks_for_stats_and_queue() {
        let h = Harness::new();
        h.ui.set_selected_id("s".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        let _ = take_jobs(&h);
        h.send_at(
            generation,
            Msg::Transcript {
                session: "s".to_string(),
                rows: rows_for("the selected session"),
                skipped: 0,
                messages: messages_for("the selected session"),
            },
        );
        h.run();
        let jobs = take_jobs(&h);
        assert!(
            jobs.iter().any(|job| matches!(job, Job::Stats { session } if session == "s")),
            "attach must refresh stats"
        );
        assert!(
            jobs.iter().any(|job| matches!(job, Job::Queue { session } if session == "s")),
            "attach must read the parked queue"
        );
    }

    fn a_user_prompt_is_drawn_before_its_reply() {
        let h = Harness::new();
        let user = serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": "the prompt"}],
            "timestamp": 1
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "message_start", "message": user.clone()}),
        });
        h.run();
        assert_eq!(
            h.tick
                .sheet
                .rows()
                .iter()
                .filter(|row| row.kind == transcript::KIND_USER)
                .count(),
            1,
            "the prompt must be in the sheet before the reply"
        );
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "message_start", "message": user}),
        });
        h.run();
        assert_eq!(
            h.tick
                .sheet
                .rows()
                .iter()
                .filter(|row| row.kind == transcript::KIND_USER)
                .count(),
            1,
            "a replayed user message must patch the row, not duplicate it"
        );

        let assistant = serde_json::json!({
            "id": "a1",
            "role": "assistant",
            "timestamp": 2,
            "content": [{"type": "text", "text": "the reply"}]
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": assistant,
                "assistantMessageEvent": {"type": "text_delta", "delta": "the reply"}
            }),
        });
        h.run();

        let rows = h.tick.sheet.rows();
        let user_index = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_USER)
            .expect("the user row");
        let reply_index = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_AGENT)
            .expect("the reply row");
        assert!(
            user_index < reply_index,
            "the prompt must precede its reply"
        );
        drop(rows);

        let messages = vec![
            serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": "the prompt"}],
                "timestamp": 1
            }),
            assistant,
        ];
        let (page, _) = transcript::build(&messages, 10);
        h.send(Msg::Tail {
            session: "s".to_string(),
            rows: page,
            skipped: 0,
            messages: messages.clone(),
            last_entry: "a1".to_string(),
            first_entry: "u1".to_string(),
            more: false,
            fresh: false,
        });
        h.run();
        let rows = h.tick.sheet.rows();
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == transcript::KIND_USER)
                .count(),
            1,
            "a skipped live tail must leave the prompt in place"
        );
        drop(rows);

        let (page, _) = transcript::build(&messages, 10);
        h.send(Msg::Tail {
            session: "s".to_string(),
            rows: page,
            skipped: 0,
            messages: messages.clone(),
            last_entry: "a1".to_string(),
            first_entry: "u1".to_string(),
            more: false,
            fresh: true,
        });
        h.run();
        let rows = h.tick.sheet.rows();
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == transcript::KIND_USER)
                .count(),
            1,
            "the tail must patch the prompt, not duplicate it"
        );
        let user_index = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_USER)
            .unwrap();
        let reply_index = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_AGENT)
            .unwrap();
        assert!(user_index < reply_index);
    }

    fn a_steering_prompt_follows_the_open_reply() {
        let h = Harness::new();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "the first prompt"}],
                    "timestamp": 1
                }
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": {
                    "id": "a1",
                    "role": "assistant",
                    "timestamp": 2,
                    "content": [{"type": "text", "text": "the reply"}]
                },
                "assistantMessageEvent": {"type": "text_delta", "delta": "the reply"}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {
                    "role": "user",
                    "content": [{"type": "text", "text": "steer now"}],
                    "timestamp": 3
                }
            }),
        });
        h.run();
        let rows = h.tick.sheet.rows();
        let steering = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_USER && row.body.contains("steer now"))
            .expect("the steering row");
        let reply = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_AGENT)
            .expect("the reply row");
        assert!(
            reply < steering,
            "a steering prompt follows the reply already on screen"
        );
    }

    fn a_custom_message_renders_live_and_reconciles_on_tail() {
        let h = Harness::new();
        let pre_custom = serde_json::json!({
            "role": "custom",
            "customType": "autonomous_status",
            "content": "Autonomous mode active",
            "display": true,
            "timestamp": 100
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "message_start", "message": pre_custom}),
        });
        h.run();
        assert_eq!(
            h.tick
                .sheet
                .rows()
                .iter()
                .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                .count(),
            1,
            "a custom message before the reply renders immediately"
        );

        let reply_1 = serde_json::json!({
            "id": "a1",
            "role": "assistant",
            "timestamp": 101,
            "content": [{"type": "text", "text": "first reply"}]
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": reply_1.clone(),
                "assistantMessageEvent": {"type": "text_delta", "delta": "first reply"}
            }),
        });
        h.run();

        let agent_msg_live = serde_json::json!({
            "role": "custom",
            "customType": "agent_message",
            "content": "subagent check-in",
            "display": true,
            "details": {"id": "agentmsg_1"},
            "timestamp": 102
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "message_start", "message": agent_msg_live}),
        });
        h.run();

        {
            let live_rows = h.tick.sheet.rows();
            assert_eq!(
                live_rows
                    .iter()
                    .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                    .count(),
                2,
                "the mid-turn agent message renders before the next reply"
            );
            let first_live = live_rows
                .iter()
                .position(|row| row.kind == transcript::KIND_AGENT && row.body.contains("first reply"))
                .expect("first reply");
            let mid_live = live_rows
                .iter()
                .position(|row| {
                    row.kind == transcript::KIND_CUSTOM_MESSAGE
                        && row.body.contains("subagent check-in")
                })
                .expect("live agent message");
            assert!(
                first_live < mid_live,
                "the live agent message lands after the reply already streaming"
            );
        }

        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({"type": "message_start", "message": agent_msg_live.clone()}),
        });
        h.run();
        {
            let rows = h.tick.sheet.rows();
            assert_eq!(
                rows.iter()
                    .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                    .count(),
                2,
                "a replayed custom message_start patches its row"
            );
        }

        let reply_2 = serde_json::json!({
            "id": "a2",
            "role": "assistant",
            "timestamp": 103,
            "content": [{"type": "text", "text": "second reply"}]
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a2", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": reply_2.clone(),
                "assistantMessageEvent": {"type": "text_delta", "delta": "second reply"}
            }),
        });
        h.run();

        let messages = vec![
            serde_json::json!({
                "id": "c1",
                "role": "custom",
                "customType": "autonomous_status",
                "content": "Autonomous mode active",
                "display": true,
                "timestamp": "2026-11-08T19:00:00Z"
            }),
            reply_1,
            serde_json::json!({
                "id": "c2",
                "role": "custom",
                "customType": "agent_message",
                "content": "subagent check-in",
                "display": true,
                "details": {"id": "agentmsg_1"},
                "timestamp": "2026-11-08T19:00:02Z"
            }),
            reply_2,
        ];
        let (page, _) = transcript::build(&messages, 10);
        h.send(Msg::Tail {
            session: "s".to_string(),
            rows: page,
            skipped: 0,
            messages,
            last_entry: "a2".to_string(),
            first_entry: "c0".to_string(),
            more: false,
            fresh: true,
        });
        h.run();

        let rows = h.tick.sheet.rows();
        assert_eq!(
            rows.iter()
                .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                .count(),
            2,
            "tail reconciliation with ISO timestamps must not duplicate custom messages"
        );
        let r1 = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_AGENT && row.body.contains("first reply"))
            .expect("first reply");
        let mid = rows
            .iter()
            .position(|row| {
                row.kind == transcript::KIND_CUSTOM_MESSAGE && row.body.contains("subagent check-in")
            })
            .expect("mid-turn agent message");
        let r2 = rows
            .iter()
            .position(|row| row.kind == transcript::KIND_AGENT && row.body.contains("second reply"))
            .expect("second reply");
        assert!(
            r1 < mid && mid < r2,
            "expected first_reply ({r1}) < agent_message ({mid}) < second_reply ({r2})"
        );
    }

    fn a_second_custom_message_keeps_arrival_order() {
        let h = Harness::new();
        let reply = serde_json::json!({
            "id": "a1",
            "role": "assistant",
            "timestamp": 100,
            "content": [{"type": "text", "text": "reply"}]
        });
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": reply,
                "assistantMessageEvent": {"type": "text_delta", "delta": "reply"}
            }),
        });
        h.run();
        for (index, (body, stamp)) in
            [("first check-in", 101), ("second check-in", 102)].into_iter().enumerate()
        {
            h.send(Msg::Event {
                session: "s".to_string(),
                event: serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "role": "custom",
                        "customType": "agent_message",
                        "content": body,
                        "display": true,
                        "details": {"id": format!("agentmsg_{stamp}")},
                        "timestamp": stamp
                    }
                }),
            });
            h.run();
            if index == 0 {
                h.send(Msg::Event {
                    session: "s".to_string(),
                    event: serde_json::json!({
                        "type": "tool_execution_start",
                        "toolCallId": "call-live",
                        "toolName": "python_repl",
                        "args": {"code": "x = 1"}
                    }),
                });
                h.run();
            }
        }
        let rows = h.tick.sheet.rows();
        let first = rows
            .iter()
            .position(|row| row.body.contains("first check-in"))
            .expect("first custom row");
        let second = rows
            .iter()
            .position(|row| row.body.contains("second check-in"))
            .expect("second custom row");
        assert!(
            first < second,
            "after-open customs keep arrival order: first={first}, second={second}"
        );
    }

    fn a_repeated_custom_body_gets_its_own_row() {
        let h = Harness::new();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_start",
                "message": {"id": "a1", "role": "assistant", "content": []}
            }),
        });
        h.run();
        h.send(Msg::Event {
            session: "s".to_string(),
            event: serde_json::json!({
                "type": "message_update",
                "message": {
                    "id": "a1",
                    "role": "assistant",
                    "timestamp": 100,
                    "content": [{"type": "text", "text": "reply"}]
                },
                "assistantMessageEvent": {"type": "text_delta", "delta": "reply"}
            }),
        });
        h.run();
        for stamp in [101, 102] {
            h.send(Msg::Event {
                session: "s".to_string(),
                event: serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "role": "custom",
                        "customType": "autonomous_status",
                        "content": "same body",
                        "display": true,
                        "timestamp": stamp
                    }
                }),
            });
            h.run();
        }
        {
            let rows = h.tick.sheet.rows();
            assert_eq!(
                rows.iter()
                    .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                    .count(),
                2,
                "repeated custom bodies must not collapse into one row"
            );
        }

        let messages = vec![
            serde_json::json!({
                "id": "a1",
                "role": "assistant",
                "timestamp": 100,
                "content": [{"type": "text", "text": "reply"}]
            }),
            serde_json::json!({
                "id": "c1",
                "role": "custom",
                "customType": "autonomous_status",
                "content": "same body",
                "display": true,
                "timestamp": "2026-11-08T19:00:01Z"
            }),
            serde_json::json!({
                "id": "c2",
                "role": "custom",
                "customType": "autonomous_status",
                "content": "same body",
                "display": true,
                "timestamp": "2026-11-08T19:00:02Z"
            }),
        ];
        let (page, _) = transcript::build(&messages, 10);
        h.send(Msg::Tail {
            session: "s".to_string(),
            rows: page,
            skipped: 0,
            messages,
            last_entry: "c2".to_string(),
            first_entry: "a1".to_string(),
            more: false,
            fresh: true,
        });
        h.run();
        let after = h.tick.sheet.rows();
        assert_eq!(
            after
                .iter()
                .filter(|row| row.kind == transcript::KIND_CUSTOM_MESSAGE)
                .count(),
            2,
            "tail reconciliation must not duplicate repeated custom bodies"
        );
    }

    fn a_new_user_prompt_brings_the_transcript_to_the_floor() {
        let h = Harness::new();
        h.ui.set_follow_bottom(false);
        h.send(
            Msg::Event {
                session: "s".to_string(),
                event: serde_json::json!({
                    "type": "message_start",
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": "a new prompt"}]
                    }
                }),
            },
        );
        h.run();
        assert!(
            h.ui.get_follow_bottom(),
            "a new user prompt must bring the newest row into view"
        );
    }

    fn a_transcript_from_another_session_cannot_replace_the_sheet() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "b".to_string(),
                rows: rows_for("the selected session"),
                skipped: 0,
                messages: messages_for("the selected session"),
            },
        );
        h.run();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "a".to_string(),
                rows: rows_for("a different session"),
                skipped: 0,
                messages: messages_for("a different session"),
            },
        );
        h.run();
        let text = sheet_text(&h);
        assert!(text.contains("the selected session"), "{text}");
        assert!(!text.contains("a different session"), "{text}");
    }

    fn a_tail_from_another_session_cannot_replace_the_sheet() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "b".to_string(),
                rows: rows_for("the selected session"),
                skipped: 0,
                messages: messages_for("the selected session"),
            },
        );
        h.run();
        h.send_at(
            generation,
            Msg::Tail {
                session: "a".to_string(),
                rows: rows_for("a different session"),
                skipped: 0,
                messages: messages_for("a different session"),
                last_entry: "a1".to_string(),
                first_entry: "a1".to_string(),
                more: false,
                fresh: true,
            },
        );
        h.run();
        let text = sheet_text(&h);
        assert!(text.contains("the selected session"), "{text}");
        assert!(!text.contains("a different session"), "{text}");
    }

    fn an_older_page_from_another_session_cannot_enter_the_sheet() {
        let h = Harness::new();
        h.ui.set_selected_id("b".into());
        h.run();
        let generation = h.tick.attach_generation.get();
        h.send_at(
            generation,
            Msg::Transcript {
                session: "b".to_string(),
                rows: rows_for("the selected session"),
                skipped: 0,
                messages: messages_for("the selected session"),
            },
        );
        h.run();
        h.send_at(
            generation,
            Msg::Older {
                session: "a".to_string(),
                rows: rows_for("a different session"),
                messages: messages_for("a different session"),
                first_entry: "a1".to_string(),
                more: false,
            },
        );
        h.run();
        let text = sheet_text(&h);
        assert!(text.contains("the selected session"), "{text}");
        assert!(!text.contains("a different session"), "{text}");
    }



    #[test]
    fn session_binding_selects_the_new_id_only_for_the_old_selection() {
        assert_eq!(
            binding_selection("old", "old", "new").as_deref(),
            Some("new")
        );
        assert!(binding_selection("mine", "old", "new").is_none());
        assert!(binding_selection("old", "old", "").is_none());
        assert!(binding_selection("old", "old", "old").is_none());
    }

    fn a_new_chat_is_local_until_the_first_send() {
        let h = Harness::new();
        *h.tick.sidebar.selected_uuid.borrow_mut() = Some("old".to_string());
        h.tick.open_new_chat();
        h.run();

        assert!(h.tick.sidebar.selected_uuid.borrow().is_none());

        assert!(h.ui.get_draft_chat());
        assert_eq!(h.ui.get_selected_id().as_str(), "");
        assert!(h.tick.sheet.rows().is_empty());
        assert!(take_jobs(&h).is_empty(), "New Chat must not reach the daemon");

        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());
        let jobs = take_jobs(&h);
        assert!(matches!(
            jobs.as_slice(),
            [Job::StartChat { message, .. }] if message == "hello"
        ));

        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);
        h.send(Msg::ChatStarted { id: "new".to_string(), generation });
        h.run();

        assert!(!h.ui.get_draft_chat());
        assert_eq!(h.ui.get_selected_id().as_str(), "new");
    }

    fn a_model_picked_in_the_draft_does_not_reach_the_daemon() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.invoke_model_picked("model".into(), "provider".into());

        assert!(take_jobs(&h).is_empty());
        assert!(h.ui.get_draft_model_picked());
        assert_eq!(h.ui.get_model_id().as_str(), "model");
        assert_eq!(h.ui.get_model_provider().as_str(), "provider");
    }

    fn the_first_send_carries_the_draft_model_and_level() {
        let h = Harness::new();
        h.ui.set_model_id("model".into());
        h.ui.set_model_provider("provider".into());
        h.ui.set_current_level("high".into());
        h.tick.open_new_chat();
        h.ui.set_draft_cwd("C:/work".into());
        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());

        let jobs = take_jobs(&h);

        assert!(matches!(
            jobs.as_slice(),
            [Job::StartChat {
                model: Some((provider, model)),
                level: Some(level),
                cwd: Some(cwd),
                ..
            }] if provider.as_str() == "provider"
                && model.as_str() == "model"
                && level.as_str() == "high"
                && cwd.as_str() == "C:/work"
        ));
    }

    fn a_create_failure_keeps_the_unsaved_chat_and_its_prompt() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());
        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);

        h.send(Msg::StartChatFailed {
            generation,
            session: None,
            text: "hello".to_string(),
            images: Vec::new(),
            error: "Could not start a new Session: no daemon".to_string(),
        });
        h.run();

        assert!(h.ui.get_draft_chat());
        assert_eq!(h.ui.get_selected_id().as_str(), "");
        assert_eq!(h.ui.get_composer_text().as_str(), "hello");
        assert!(h.ui.get_composer_has_text());
        assert!(error_rows(&h).iter().any(|row| row.contains("no daemon")));
    }

    fn a_prompt_failure_after_create_keeps_the_session_and_retry_state() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());
        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);
        h.send(Msg::ChatStarted { id: "new".to_string(), generation });
        h.run();

        h.send(Msg::StartChatFailed {
            generation,
            session: Some("new".to_string()),
            text: "hello".to_string(),
            images: Vec::new(),
            error: "the prompt was refused".to_string(),
        });
        h.run();

        assert!(!h.ui.get_draft_chat());
        assert_eq!(h.ui.get_selected_id().as_str(), "new");
        assert_eq!(h.ui.get_composer_text().as_str(), "hello");
        assert!(error_rows(&h)
            .iter()
            .any(|row| row.contains("the prompt was refused")));
    }

    fn the_sidebar_hides_only_inactive_anonymous_empty_saved_rows() {
        let h = Harness::new();
        let rows = vec![
            saved_session_row("empty", "", "", ""),
            saved_session_row("plan", "fresh plan", "", ""),
            saved_session_row("live", "live", "ready", ""),
        ];
        h.send(Msg::Sessions(Ok(rows.clone())));
        h.run();

        let ids = drawn_session_ids(&h);
        assert!(!ids.contains(&"empty".to_string()));
        assert!(ids.contains(&"plan".to_string()));
        assert!(ids.contains(&"live".to_string()));

        h.ui.set_selected_id("empty".into());
        h.send(Msg::Sessions(Ok(rows)));
        h.run();

        assert!(drawn_session_ids(&h).contains(&"empty".to_string()));
    }

    fn a_stale_roster_cannot_steal_a_newly_bound_chat() {
        let h = Harness::new();
        h.tick.open_new_chat();
        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);
        h.send(Msg::ChatStarted { id: "new".to_string(), generation });
        h.run();

        h.send(roster_state_for("old", false));
        h.run();

        assert_eq!(h.ui.get_selected_id().as_str(), "new");
        assert!(!h.ui.get_draft_chat());
    }

    fn a_stale_start_failure_does_not_touch_a_newer_unsaved_chat() {
        let h = Harness::new();
        h.tick.open_new_chat();
        let stale = draft_generation(&h.ui);
        h.tick.open_new_chat();
        h.ui.set_composer_text("new text".into());

        h.send(Msg::StartChatFailed {
            generation: stale,
            session: None,
            text: "old text".to_string(),
            images: Vec::new(),
            error: "old failure".to_string(),
        });
        h.run();

        assert!(h.ui.get_draft_chat());
        assert_eq!(h.ui.get_composer_text().as_str(), "new text");
        assert!(error_rows(&h).is_empty());
    }

    fn a_retry_after_a_prompt_failure_sends_only_the_prompt() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());
        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);
        h.send(Msg::ChatStarted { id: "new".to_string(), generation });
        h.run();
        h.send(Msg::StartChatFailed {
            generation,
            session: Some("new".to_string()),
            text: "hello".to_string(),
            images: Vec::new(),
            error: "the prompt was refused".to_string(),
        });
        h.run();
        take_jobs(&h);

        h.ui.set_composer_text("hello".into());
        h.ui.invoke_send_requested("hello".into());
        let jobs = take_jobs(&h);

        assert!(!jobs.iter().any(|job| matches!(job, Job::StartChat { .. })));
        assert!(jobs.iter().any(|job| matches!(
            job,
            Job::Prompt { session, message, .. } if session == "new" && message == "hello"
        )));
    }

    fn a_create_failure_restores_the_prompt_attachments() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.set_composer_text("look".into());
        h.ui.invoke_send_requested("look".into());
        let generation = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);

        h.send(Msg::StartChatFailed {
            generation,
            session: None,
            text: "look".to_string(),
            images: vec![serde_json::json!({
                "type": "image", "data": "", "mimeType": "image/png"
            })],
            error: "Could not start a new Session: no daemon".to_string(),
        });
        h.run();

        assert_eq!(h.ui.get_composer_text().as_str(), "look");
        assert_eq!(h.ui.get_attachment_images().row_count(), 1);
    }

    fn a_partial_model_choice_is_not_a_draft_model() {
        let h = Harness::new();
        h.ui.set_model_id("model".into());
        h.ui.set_model_provider("".into());
        h.tick.open_new_chat();
        assert!(!h.ui.get_draft_model_picked());

        h.ui.set_model_id("".into());
        h.ui.set_model_provider("provider".into());
        h.tick.open_new_chat();
        assert!(!h.ui.get_draft_model_picked());
    }

    fn a_rejection_keeps_new_composer_input_and_does_not_restore_old_images() {
        let h = Harness::new();
        h.tick.open_new_chat();
        h.ui.set_composer_text("old".into());
        h.ui.invoke_send_requested("old".into());
        let generation = draft_generation(&h.ui);
        h.ui.set_composer_text("new text".into());

        h.send(Msg::StartChatFailed {
            generation,
            session: None,
            text: "old".to_string(),
            images: vec![serde_json::json!({
                "type": "image", "data": "", "mimeType": "image/png"
            })],
            error: "failed".to_string(),
        });
        h.run();

        assert_eq!(h.ui.get_composer_text().as_str(), "new text");
        assert_eq!(h.ui.get_attachment_images().row_count(), 0);
    }

    fn opening_a_new_chat_inherits_the_selected_cwd() {
        let h = Harness::new();
        h.send(Msg::Sessions(Ok(vec![
            saved_session_row("one", "one", "", "C:/one"),
            saved_session_row("two", "two", "", "C:/two"),
        ])));
        h.run();
        h.ui.set_selected_id("two".into());
        h.run();

        h.tick.open_new_chat();

        assert_eq!(h.ui.get_draft_cwd().as_str(), "C:/two");
    }

    fn a_stale_start_result_does_not_replace_the_current_unsaved_chat() {
        let h = Harness::new();
        h.tick.open_new_chat();
        let stale = u64::try_from(h.ui.get_draft_generation()).unwrap_or(0);
        h.tick.open_new_chat();

        h.send(Msg::ChatStarted { id: "old".to_string(), generation: stale });
        h.run();

        assert!(h.ui.get_draft_chat());
        assert_eq!(h.ui.get_selected_id().as_str(), "");
    }

    fn reopen_last_session_uses_the_client_side_selection() {
        let path = std::path::PathBuf::from(
            std::env::var("PA_SETTINGS_FILE").expect("test settings file"),
        );
        let rows = || {
            let mut a = saved_session_row("a", "a", "", "");
            a.session_id = "uuid-a".to_string();
            let mut b = saved_session_row("b", "b", "", "");
            b.session_id = "uuid-b".to_string();
            vec![a, b]
        };

        let mut stored = crate::settings::Settings::default();
        stored.reopen_last_session = true;
        stored.last_session = "uuid-b".to_string();
        stored.save_to(&path).expect("stored selection");
        let h = Harness::new();
        h.send(Msg::Sessions(Ok(rows())));
        h.run();
        assert_eq!(h.ui.get_selected_id().as_str(), "b");

        h.tick.sidebar.track_selection("a");
        let disk = crate::settings::Settings::load_from(&path);
        assert_eq!(disk.last_session, "uuid-a");

        stored.reopen_last_session = false;
        stored.last_session = "uuid-b".to_string();
        stored.save_to(&path).expect("stored selection");
        let h = Harness::new();
        h.send(Msg::Sessions(Ok(rows())));
        h.run();
        assert_ne!(h.ui.get_selected_id().as_str(), "b");

        stored.reopen_last_session = true;
        stored.last_session = "uuid-missing".to_string();
        stored.save_to(&path).expect("stored selection");
        let h = Harness::new();
        h.send(Msg::Sessions(Ok(rows())));
        h.run();
        assert_ne!(h.ui.get_selected_id().as_str(), "b");
    }

    fn settings_popup_mutual_exclusion_and_persistence() {
        let path = std::path::PathBuf::from(
            std::env::var("PA_SETTINGS_FILE").expect("test settings file"),
        );
        let h = Harness::new();

        h.ui.set_search_open(true);
        h.ui.set_share_open(true);
        h.ui.set_model_menu(true);
        h.ui.set_queue_open(true);
        h.ui.invoke_settings_toggle_requested();
        assert!(h.ui.get_settings_menu());
        assert!(!h.ui.get_search_open());
        assert!(!h.ui.get_share_open());
        assert!(!h.ui.get_model_menu());
        assert!(!h.ui.get_queue_open());

        h.ui.invoke_search_open_requested();
        assert!(h.ui.get_search_open());
        assert!(!h.ui.get_settings_menu());

        h.ui.invoke_settings_toggle_requested();
        assert!(h.ui.get_settings_menu());
        h.ui.set_settings_page("appearance".into());
        assert!(h.ui.get_settings_menu());
        assert_eq!(h.ui.get_settings_page().as_str(), "appearance");

        h.ui.invoke_theme_picked("mono".into());
        let themes = h.ui.get_theme_items();
        let mono_current = (0..themes.row_count())
            .filter_map(|i| themes.row_data(i))
            .find(|item| item.id.as_str() == "mono")
            .map(|item| item.current)
            .unwrap_or(false);
        assert!(mono_current);

        assert!(h.ui.get_setting_in_app_notifications());
        h.ui.invoke_setting_toggled("in-app-notifications".into());
        assert!(!h.ui.get_setting_in_app_notifications());

        h.ui.invoke_setting_chosen("queue-key".into(), "Alt+Enter".into());
        assert_eq!(h.ui.get_setting_queue_key().as_str(), "Alt+Enter");

        let disk = crate::settings::Settings::load_from(&path);
        assert_eq!(disk.theme, "mono");
        assert!(!disk.in_app_notifications);
        assert_eq!(disk.queue_key, "Alt+Enter");

        h.ui.set_settings_page("general".into());
        h.ui.invoke_settings_search_edited("theme".into());
        assert_eq!(h.ui.get_settings_page().as_str(), "appearance");

        h.ui.set_settings_page("appearance".into());
        h.ui.invoke_settings_search_edited("queue".into());
        assert_eq!(h.ui.get_settings_page().as_str(), "general");

        h.ui.set_settings_page("appearance".into());
        h.ui.invoke_settings_search_edited("order".into());
        assert_eq!(h.ui.get_settings_page().as_str(), "general");

        h.ui.set_settings_page("general".into());
        h.ui.invoke_settings_search_edited("font".into());
        assert!(h.ui.get_settings_show_typography());
        assert!(!h.ui.get_settings_show_notifications());
        assert_eq!(h.ui.get_settings_page().as_str(), "appearance");

        h.ui.invoke_settings_closed();
        assert!(!h.ui.get_settings_menu());
    }

    fn settings_choices_expand_collapse_and_select() {
        let h = Harness::new();
        h.ui.invoke_settings_toggle_requested();
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "");

        for key in [
            "session-notifications",
            "session-order",
            "queue-key",
            "code-font",
        ] {
            h.ui.invoke_toggle_settings_choice(key.into());
            assert_eq!(h.ui.get_settings_open_choice().as_str(), key);
            h.ui.invoke_toggle_settings_choice(key.into());
            assert_eq!(h.ui.get_settings_open_choice().as_str(), "");
        }

        h.ui.invoke_toggle_settings_choice("session-notifications".into());
        h.ui.invoke_toggle_settings_choice("session-order".into());
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "session-order");

        h.ui.invoke_setting_chosen("session-order".into(), "Name".into());
        assert_eq!(h.ui.get_setting_session_order().as_str(), "Name");
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "");

        h.ui.invoke_toggle_settings_choice("queue-key".into());
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "queue-key");
        h.ui.invoke_dismiss_settings_choice();
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "");
        assert!(h.ui.get_settings_menu());

        h.ui.invoke_toggle_settings_choice("code-font".into());
        h.ui.invoke_settings_closed();
        assert!(!h.ui.get_settings_menu());
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "");

        h.ui.invoke_settings_toggle_requested();
        assert!(h.ui.get_settings_menu());
        assert_eq!(h.ui.get_settings_open_choice().as_str(), "");
    }

    fn notifications_in_app_and_system_toast_adapter_follow_policy() {
        type RecordedToast = (String, String, String, bool);
        #[derive(Clone)]
        struct RecordingToast {
            sent: Rc<std::cell::RefCell<Vec<RecordedToast>>>,
            fail: Rc<std::cell::Cell<bool>>,
        }
        impl crate::notify::ToastNotifier for RecordingToast {
            fn send(
                &self,
                session_id: &str,
                title: &str,
                body: &str,
                failed: bool,
            ) -> Result<(), String> {
                self.sent.borrow_mut().push((
                    session_id.to_string(),
                    title.to_string(),
                    body.to_string(),
                    failed,
                ));
                if self.fail.get() {
                    Err("toast unavailable".to_string())
                } else {
                    Ok(())
                }
            }
        }

        let sent = Rc::new(std::cell::RefCell::new(Vec::new()));
        let fail = Rc::new(std::cell::Cell::new(false));
        let recorder = Rc::new(RecordingToast {
            sent: sent.clone(),
            fail: fail.clone(),
        });

        if let Ok(path) = std::env::var("PA_SETTINGS_FILE") {
            let _ = std::fs::remove_file(path);
        }
        let h = Harness::new();
        *h.tick.toast.borrow_mut() = recorder;
        h.ui.set_window_active(true);
        h.ui.invoke_setting_chosen("session-notifications".into(), "Only when unfocused".into());
        assert!(h.ui.get_setting_in_app_notifications());

        let mk_row = |id: &str, uuid: &str, name: &str, working: bool, depth: u64| {
            SessionRow::from_value(&serde_json::json!({
                "activeSessionId": id,
                "sessionId": uuid,
                "firstMessage": name,
                "workerState": "ready",
                "activity": if working { "working" } else { "idle" },
                "rlmDepth": depth
            }))
        };

        // Initial roster + Transcript replay do not notify.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", true, 0),
            mk_row("s3", "u3", "Background two", true, 0),
            mk_row("sub1", "usub", "Subagent", true, 1),
        ])));
        h.run();
        h.ui.set_selected_id("s1".into());
        h.send(Msg::Transcript {
            session: "s1".to_string(),
            rows: Vec::new(),
            skipped: 0,
            messages: Vec::new(),
        });
        h.run();
        assert!(!h.ui.get_notice_open());
        assert!(sent.borrow().is_empty());

        // Background top-level Session finishes while focused -> in-app notice only (subagent ignored).
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", false, 0),
            mk_row("s3", "u3", "Background two", true, 0),
            mk_row("sub1", "usub", "Subagent", false, 1),
        ])));
        h.run();
        assert!(h.ui.get_notice_open());
        assert_eq!(h.ui.get_notice_session().as_str(), "s2");
        assert_eq!(h.ui.get_notice_name().as_str(), "Background one");
        assert!(!h.ui.get_notice_failed());
        assert!(sent.borrow().is_empty());
        let tick_before = h.ui.get_notice_tick();

        // Newer notice replaces current notice instead of stacking.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", false, 0),
            mk_row("s3", "u3", "Background two", false, 0),
        ])));
        h.run();
        assert!(h.ui.get_notice_open());
        assert_eq!(h.ui.get_notice_session().as_str(), "s3");
        assert_eq!(h.ui.get_notice_name().as_str(), "Background two");
        assert!(h.ui.get_notice_tick() > tick_before);

        // Dismiss clears notice without changing selection.
        h.ui.invoke_notice_dismissed();
        assert!(!h.ui.get_notice_open());
        assert_eq!(h.ui.get_selected_id().as_str(), "s1");

        // Clicking notice opens that Session only when selection actually changes.
        let picked = Rc::new(std::cell::Cell::new(0usize));
        {
            let picked = picked.clone();
            h.ui.on_selection_picked(move |_| picked.set(picked.get() + 1));
        }
        crate::notify::show(&h.ui, "s2", "Background one", false);
        h.ui.invoke_notice_picked("s2".into());
        assert!(!h.ui.get_notice_open());
        assert_eq!(h.ui.get_selected_id().as_str(), "s2");
        assert_eq!(picked.get(), 1);
        crate::notify::show(&h.ui, "s2", "Background one", false);
        h.ui.invoke_notice_picked("s2".into());
        assert!(!h.ui.get_notice_open());
        assert_eq!(picked.get(), 1);

        // Empty selected_id + empty session_id still dispatches rather than treating "" as selected.
        h.ui.set_selected_id("".into());
        crate::notify::dispatch(
            &h.ui,
            h.tick.toast.borrow().as_ref(),
            "",
            "Fallback",
            crate::notify::TurnOutcome::Finished,
        );
        assert!(h.ui.get_notice_open());
        h.ui.invoke_notice_dismissed();
        h.ui.set_selected_id("s2".into());

        // Reconnect suppresses stale Running -> Idle transitions.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.send(Msg::Reconnected);
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        assert!(!h.ui.get_notice_open());
        assert!(sent.borrow().is_empty());

        // Always delivers toast while focused (even if toast fails, banner still opens).
        h.ui.invoke_setting_chosen("session-notifications".into(), "Always".into());
        fail.set(true);
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        assert!(h.ui.get_notice_open());
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(sent.borrow()[0].0, "s1");
        assert!(!sent.borrow()[0].3);
        h.ui.invoke_notice_dismissed();
        fail.set(false);
        sent.borrow_mut().clear();

        // Unfocused selected Session emits failed toast once on turn_end error + agent_end,
        // keeps announced_session across stale Running snapshots and other-session turn_start,
        // and clears it once the roster shows s2 idle or s2 starts a new turn.
        h.ui.set_window_active(false);
        h.ui.invoke_setting_chosen("session-notifications".into(), "Only when unfocused".into());
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", true, 0),
        ])));
        h.run();
        let gen = h.tick.attach_generation.get();
        h.send_at(gen, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({
                "type": "turn_end",
                "message": {"role": "assistant", "stopReason": "error", "errorMessage": "quota"}
            }),
        });
        h.send_at(gen, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "agent_end"}),
        });
        h.run();
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(sent.borrow()[0], ("s2".into(), "Session failed".into(), "Background one".into(), true));
        sent.borrow_mut().clear();

        // A stale snapshot where s2 is still Running keeps announced_session intact, and switching
        // to s1 with a turn_start for s1 keeps s2's announced flag, so SessionClosed on s2 does not double-notify.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", true, 0),
        ])));
        h.run();
        h.ui.set_selected_id("s1".into());
        h.run();
        let gen_s1 = h.tick.attach_generation.get();
        h.send_at(gen_s1, Msg::Event {
            session: "s1".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen_s1, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(sent.borrow().is_empty());

        // Once the roster catches up (s2 idle) and s2 closes, SessionClosed notifies.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.send_at(gen_s1, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(sent.borrow()[0], ("s2".into(), "Session failed".into(), "Background one".into(), true));
        sent.borrow_mut().clear();

        // A new turn_start on s2 clears announced_session so a second close notifies again; Off suppresses it.
        h.ui.set_selected_id("s2".into());
        h.run();
        let gen_s2 = h.tick.attach_generation.get();
        h.send_at(gen_s2, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen_s2, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert_eq!(sent.borrow().len(), 1);
        sent.borrow_mut().clear();

        // Subagent agent_end & SessionClosed do not notify even when Always is active;
        // closing an unrelated session leaves selected turn busy, while closing the selected
        // session clears busy and still notifies while draft_chat is open.
        h.ui.invoke_setting_chosen("session-notifications".into(), "Always".into());
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
            mk_row("sub1", "usub", "Subagent", true, 1),
        ])));
        h.run();
        h.ui.set_selected_id("sub1".into());
        h.run();
        let gen_sub = h.tick.attach_generation.get();
        h.ui.set_window_active(false);
        h.send_at(gen_sub, Msg::Event {
            session: "sub1".to_string(),
            event: serde_json::json!({"type": "agent_end"}),
        });
        h.send_at(gen_sub, Msg::SessionClosed {
            session: "sub1".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(sent.borrow().is_empty());

        h.ui.set_selected_id("s2".into());
        h.run();
        let gen_busy = h.tick.attach_generation.get();
        h.send_at(gen_busy, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.run();
        assert!(h.ui.get_busy());
        h.send_at(gen_busy, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(!h.ui.get_busy());
        assert_eq!(sent.borrow().len(), 1);
        sent.borrow_mut().clear();

        // Closing another session does not clear selected s2's busy strip.
        h.send_at(gen_busy, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.run();
        assert!(h.ui.get_busy());
        h.send_at(gen_busy, Msg::SessionClosed {
            session: "s1".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(h.ui.get_busy());
        sent.borrow_mut().clear();

        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.ui.set_draft_chat(true);
        h.ui.set_window_active(true);
        h.send_at(gen_busy, Msg::SessionClosed {
            session: "s1".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(h.ui.get_notice_open());
        assert_eq!(h.ui.get_notice_session().as_str(), "s1");
        assert!(h.ui.get_notice_failed());
        assert_eq!(sent.borrow().len(), 1);
        h.ui.invoke_notice_dismissed();
        h.ui.set_draft_chat(false);
        sent.borrow_mut().clear();

        // Switching away after turn_end(error) on s1 resets turn_failed before s2's agent_end.
        h.ui.set_selected_id("s1".into());
        h.run();
        let gen_err = h.tick.attach_generation.get();
        h.send_at(gen_err, Msg::Event {
            session: "s1".to_string(),
            event: serde_json::json!({
                "type": "turn_end",
                "message": {"role": "assistant", "stopReason": "error", "errorMessage": "quota"}
            }),
        });
        h.run();
        h.ui.set_selected_id("s2".into());
        h.run();
        let gen_clean = h.tick.attach_generation.get();
        h.ui.set_window_active(false);
        h.send_at(gen_clean, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "agent_end"}),
        });
        h.run();
        assert_eq!(sent.borrow().len(), 1);
        assert_eq!(sent.borrow()[0], ("s2".into(), "Session finished".into(), "Background one".into(), false));
        sent.borrow_mut().clear();

        // Concurrent agent_end(s2) + SessionClosed(s1) + catching-up roster does not duplicate s2,
        // and daemon shutdown / replaced SessionClosed does not mislabel a Session as failed.
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", true, 0),
        ])));
        h.run();
        h.send_at(gen_clean, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen_clean, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "agent_end"}),
        });
        h.send_at(gen_clean, Msg::SessionClosed {
            session: "s1".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert_eq!(sent.borrow().len(), 2);
        sent.borrow_mut().clear();

        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        assert!(sent.borrow().is_empty());

        h.send_at(gen_clean, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen_clean, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "shutdown".to_string(),
        });
        h.run();
        assert!(sent.borrow().is_empty());

        h.ui.invoke_setting_chosen("session-notifications".into(), "Off".into());
        h.send_at(gen_clean, Msg::Event {
            session: "s2".to_string(),
            event: serde_json::json!({"type": "turn_start"}),
        });
        h.send_at(gen_clean, Msg::SessionClosed {
            session: "s2".to_string(),
            reason: "killed".to_string(),
        });
        h.run();
        assert!(sent.borrow().is_empty());

        // Disabling in-app notices while focused suppresses the banner without disabling Always toasts.
        h.ui.set_window_active(true);
        h.ui.invoke_setting_chosen("session-notifications".into(), "Always".into());
        h.ui.invoke_setting_toggled("in-app-notifications".into());
        assert!(!h.ui.get_setting_in_app_notifications());
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", true, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        h.send(Msg::Sessions(Ok(vec![
            mk_row("s1", "u1", "Focused chat", false, 0),
            mk_row("s2", "u2", "Background one", false, 0),
        ])));
        h.run();
        assert!(!h.ui.get_notice_open());
        assert_eq!(sent.borrow().len(), 1);
    }
}
