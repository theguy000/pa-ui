//! The poll tick: everything the window learns by asking rather than being told.

use crate::*;

/// What the notice row says where history was unloaded.
const HISTORY_UNLOADED: &str = "Some history here was unloaded to keep the window fast.";

pub(crate) struct Tick {
    pub weak: slint::Weak<AppWindow>,
    pub jobs: Sender<Job>,
    pub msg_rx: Receiver<(u64, Msg)>,
    /// which attach the window is waiting on: a reply that carries an older
    /// number is an answer to a question it has given up on
    pub attach_generation: std::cell::Cell<u64>,
    /// the oldest message of the newest page: everything above it is history,
    /// and history is what gets unloaded
    pub tail_from: std::cell::RefCell<Option<String>>,
    /// how many messages of history stay resident above the newest page
    pub history_messages: usize,
    pub sheet: Rc<sheet::Sheet>,
    pub session_filter: Rc<RefCell<String>>,
    pub sessions_cache: Rc<RefCell<Vec<SessionRow>>>,
    pub in_flight: Rc<std::cell::Cell<bool>>,
    pub pending_user: Rc<RefCell<Option<String>>>,
    pub extra_commands: Rc<RefCell<Vec<(String, String, String)>>>,
    pub repaint_slot: Rc<RefCell<Option<Rc<dyn Fn(&AppWindow)>>>>,
    pub sent_thumbs: Rc<RefCell<Vec<(String, slint::Image)>>>,
    pub catalog: Rc<RefCell<Vec<pa_client::ModelEntry>>>,
    pub collapsed: Rc<RefCell<std::collections::HashSet<String>>>,
    pub catalog_loaded: Rc<std::cell::Cell<bool>>,
    pub busy_since: Rc<std::cell::Cell<SystemTime>>,
    pub oldest_cursor: Rc<RefCell<Option<String>>>,
    pub has_older: Rc<std::cell::Cell<bool>>,
    pub loading_older: Rc<std::cell::Cell<bool>>,
    pub window_rows: usize,
    pub drawn_user: std::cell::Cell<Option<String>>,
    pub loaded: std::cell::Cell<Option<String>>,
    pub live_state: std::cell::Cell<String>,
    pub live_since: std::cell::Cell<SystemTime>,
    pub busy_label: std::cell::Cell<String>,
    pub tail_len: std::cell::Cell<usize>,
    pub tail_sig: std::cell::Cell<Option<(u64, u64)>>,
    pub last_seq: std::cell::Cell<u64>,
    pub open_live: std::cell::Cell<Option<live::Live>>,
    pub stream_started: std::cell::Cell<SystemTime>,
    pub _stream_at: std::cell::Cell<SystemTime>,
    pub selected_uuid: std::cell::Cell<Option<String>>,
    pub tail_at: std::cell::Cell<SystemTime>,
    pub queue_at: std::cell::Cell<SystemTime>,
    pub stats_at: std::cell::Cell<SystemTime>,
    pub scroll_ticks: std::cell::Cell<u64>,
}

/// The state a tick carries between one call and the next.
///
/// It is taken out of the window's cells at the start of a tick and put back at
/// the end, so the body below reads as it did when these were plain locals.
pub(crate) struct Local {
    drawn_user: Option<String>,
    loaded: Option<String>,
    live_state: String,
    live_since: SystemTime,
    busy_label: String,
    tail_len: usize,
    tail_sig: Option<(u64, u64)>,
    last_seq: u64,
    open_live: Option<live::Live>,
    stream_started: SystemTime,
    _stream_at: SystemTime,
    selected_uuid: Option<String>,
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
            tail_len: tick.tail_len.get(),
            tail_sig: tick.tail_sig.take(),
            last_seq: tick.last_seq.get(),
            open_live: tick.open_live.take(),
            stream_started: tick.stream_started.get(),
            _stream_at: tick._stream_at.get(),
            selected_uuid: tick.selected_uuid.take(),
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
        tick.tail_len.set(self.tail_len);
        tick.tail_sig.set(self.tail_sig);
        tick.last_seq.set(self.last_seq);
        tick.open_live.set(self.open_live);
        tick.stream_started.set(self.stream_started);
        tick._stream_at.set(self._stream_at);
        tick.selected_uuid.set(self.selected_uuid);
        tick.tail_at.set(self.tail_at);
        tick.queue_at.set(self.queue_at);
        tick.stats_at.set(self.stats_at);
        tick.scroll_ticks.set(self.scroll_ticks);
    }
}

impl Tick {
    fn request_older(&self, ui: &AppWindow) {
        let state = PagerState {
            follows_bottom: ui.get_follow_bottom(),
            ready: ui.get_pager_ready(),
            loading: self.loading_older.get(),
            has_older: self.has_older.get(),
            content_y: ui.get_dbg_cy() as f32,
            content_height: ui.get_dbg_ch() as f32,
            height: ui.get_dbg_h() as f32,
        };
        if !should_load_older(state) {
            return;
        }
        let Some(before) = self.oldest_cursor.borrow().clone() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        self.loading_older.set(true);
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
        let Some(ui) = self.weak.upgrade() else { return };
        let mut l = Local::load(self);
        let jobs = &self.jobs;
        let msg_rx = &self.msg_rx;
        let sheet = &self.sheet;
        let tail_from = &self.tail_from;
        let session_filter = &self.session_filter;
        let sessions_cache = &self.sessions_cache;
        let in_flight = &self.in_flight;
        let pending_user = &self.pending_user;
        let extra_commands = &self.extra_commands;
        let repaint_slot = &self.repaint_slot;
        let sent_thumbs = &self.sent_thumbs;
        let catalog = &self.catalog;
        let collapsed = &self.collapsed;
        let catalog_loaded = &self.catalog_loaded;
        let busy_since = &self.busy_since;
        let window_rows = self.window_rows;
        let oldest_cursor = &self.oldest_cursor;
        let has_older = &self.has_older;
        let loading_older = &self.loading_older;

        if crate::env_cfg::cfg().debug {
            ui.set_status_line(
                format!(
                    "cy={} ch={} h={} at-bottom={}",
                    ui.get_dbg_cy(),
                    ui.get_dbg_ch(),
                    ui.get_dbg_h(),
                    ui.get_at_bottom()
                )
                .into(),
            );
            // the pager only moves on a scroll; when it does not, this says which
            // of the four conditions was false
            l.scroll_ticks += 1;
            if l.scroll_ticks % 15 == 0 {
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
        if ui.get_busy() && !l.loaded.as_deref().unwrap_or("").is_empty()
            // PA_QUEUE is a design check: keep the seeded lanes on screen
            && !crate::env_cfg::cfg().queue_preset
            && l.queue_at.elapsed().unwrap_or_default() >= Duration::from_millis(1500)
        {
            l.queue_at = SystemTime::now();
            if let Some(session) = l.loaded.clone() {
                let _ = jobs.send(Job::Queue { session });
            }
        }

        // while the agent works, keep pulling the tail of the transcript so
        // completed tool calls show up as rows � the snapshot alone never updates
        // Poll while a turn runs, and slowly the rest of the time. Gating this
        // on `busy` alone meant one missed `message_end` froze the transcript
        // for good: nothing would ever ask the store again.
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
                let _ = jobs.send(Job::Tail { session, fresh: false });
            }
        }

        // follow the selection
        let selected = ui.get_selected_id().to_string();
        if !selected.is_empty() && l.loaded.as_deref() != Some(selected.as_str()) {
            l.loaded = Some(selected.clone());
            // a different chat starts its own walk backwards
            *oldest_cursor.borrow_mut() = None;
            has_older.set(false);
            loading_older.set(false);
            ui.set_has_older(false);
            ui.set_loading_older(false);
            ui.set_pager_ready(false);
            ui.set_status_line("loading transcript".into());
            let generation = self.attach_generation.get() + 1;
            self.attach_generation.set(generation);
            let _ = jobs.send(Job::Attach {
                session: selected,
                generation,
            });
        }

        // reaching the top pulls in the page before this one
        self.request_older(&ui);

        // caret and selection moves arrive here (arrow keys, clicks); keystrokes
        // paint themselves
        if let Some(paint) = repaint_slot.borrow().as_ref() {
            paint(&ui);
        }

        while let Ok((generation, message)) = msg_rx.try_recv() {
            // One rule for every session-scoped reply: if the window has
            // attached since this request was issued, the answer belongs to a
            // chat the reader has left. It is dropped here, before any state
            // can see it. A session id could not tell this apart from a reply
            // sent before a re-attach of the same chat.
            if message.is_session_scoped() && generation != self.attach_generation.get() {
                continue;
            }
            match message {
                Msg::Sessions(Ok(rows)) => {
                    // kept so a keystroke can re-filter without waiting for the
                    // next poll
                    *sessions_cache.borrow_mut() = rows.clone();
                    let items: Vec<SessionItem> =
                        session_items(&rows, &session_filter.borrow());
                    let live = rows.iter().filter(|row| row.lifecycle == "live").count();
                    ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(items))));
                    ui.set_status_line(format!("{live} live of {} known", rows.len()).into());
                    if ui.get_selected_id().is_empty() {
                        let preferred = std::env::var("PA_SELECT").ok();
                        let chosen = preferred
                            .as_deref()
                            .and_then(|wanted| rows.iter().find(|row| row.id == wanted))
                            .or_else(|| rows.first());
                        if let Some(first) = chosen {
                            ui.set_selected_id(first.id.clone().into());
                        }
                    }

                    // The daemon re-keys session ids across restarts. Follow the
                    // selection by its uuid so the chat, the busy chip and the
                    // prompt target keep pointing at the same session.
                    let selected = ui.get_selected_id().to_string();
                    if !selected.is_empty() && !rows.iter().any(|row| row.id == selected) {
                        if let Some(uuid) = l.selected_uuid.as_deref() {
                            if let Some(rebound) =
                                rows.iter().find(|row| row.session_id == uuid).map(|row| row.id.clone())
                            {
                                if crate::env_cfg::cfg().debug {
                                    eprintln!("[rebind] {selected} -> {rebound}");
                                }
                                ui.set_selected_id(rebound.clone().into());
                                l.selected_uuid = Some(uuid.to_string());
                                l.loaded = None;
                                l.tail_len = 0;
                                l.tail_sig = None;
                                l.last_seq = 0;
                                l.open_live = None;
                                sheet.answer_ended();
                            }
                        }
                    }
                    let selected = ui.get_selected_id().to_string();
                    if l.selected_uuid.is_none() {
                        l.selected_uuid = uuid_of(&rows, &selected).map(str::to_string);
                    }
                    // the chip names the model this session is actually on
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        if !row.model.is_empty() {
                            let label = if row.model_name.is_empty() {
                                row.model.clone()
                            } else {
                                model_label(&row.model_name, &row.model)
                            };
                            if ui.get_model_label().as_str() != label {
                                ui.set_model_label(label.into());
                            }
                            ui.set_model_id(row.model.clone().into());
                            ui.set_model_provider(row.model_provider.clone().into());
                        }
                    }

                    // nothing known about this selection: nothing is running
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
                            // the run just ended � grab whatever landed after the
                            // last refresh so the final tool calls are not lost
                            if let Some(session) = l.loaded.clone() {
                                let _ = jobs.send(Job::Tail { session, fresh: false });
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
                        let held = l.live_since.elapsed().unwrap_or_default().as_secs();
                        let busy = state != "idle" && state != "archived";
                        ui.set_activity(state.into());
                        ui.set_activity_busy(busy);
                        ui.set_activity_elapsed(if busy && held > 0 {
                            format!("{held}s").into()
                        } else {
                            SharedString::default()
                        });
                    }
                }
                Msg::Sessions(Err(error)) => ui.set_status_line(format!("daemon: {error}").into()),
                Msg::Activity { label } => {
                    if label != l.busy_label {
                        l.busy_label = label;
                        busy_since.set(SystemTime::now());
                        ui.set_busy_label(l.busy_label.clone().into());
                    }
                    ui.set_busy(true);
                }
                Msg::Transcript { session, messages } => {
                    // the session's own commands arrive with the session
                    let _ = jobs.send(Job::Commands {
                        session: session.clone(),
                    });
                    l.tail_len = messages.len();
                    *tail_from.borrow_mut() = messages.first().map(transcript::message_key);
                    let (mut built, skipped) = transcript::build(&messages, window_rows);
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
                    // a prompt sent while this snapshot was in flight: the
                    // store predates it, so the row we drew has to survive the
                    // rebuild
                    sheet.replace(built);
                    let count = sheet.rows().len();
                    sheet::dump(&sheet.rows());
                    // which model is running, and at what effort: the chip and
                    // the menu's ladder both read from this
                    let _ = jobs.send(Job::SessionState {
                        session: session.clone(),
                    });
                    if crate::env_cfg::cfg().menu {
                        let _ = jobs.send(Job::ModelCatalog {
                            session: session.clone(),
                        });
                    }
                    let note = if skipped > 0 {
                        format!("{count} rows (newest {window_rows} of {})", messages.len())
                    } else {
                        format!("{count} rows")
                    };
                    ui.set_status_line(note.into());
                    // land on the newest turn, like every chat
                    scroll_to_end_when_ready(&ui.as_weak());
                }
                Msg::Tail {
                    session,
                    messages,
                    last_entry,
                    first_entry,
                    more,
                    fresh,
                } => {
                    // Where the reader is now, before the model changes: a
                    // rebuild must not drag them back to the newest row when they
                    // have scrolled up to read. This is what made scrolling up
                    // while the agent worked impossible.
                    let follow = ui.get_follow_bottom() && !ui.get_scrollbar_dragging();
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[tail] {session} {} msgs (have {}) newest={:?}",
                            messages.len(),
                            l.tail_len,
                            messages
                                .last()
                                .map(|m| m.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(0))
                                .unwrap_or(0)
                        );
                    }
                    // An empty list usually means the id went stale (the daemon
                    // restarted and re-keyed its sessions). Never let that blank
                    // a transcript that is already on screen.
                    if messages.is_empty() {
                        continue;
                    }
                    // the page cursor is the fingerprint: it moves when the tail
                    // moves, even when the row count is unchanged
                    let newest = messages
                        .last()
                        .and_then(|message| message.get("timestamp"))
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    let cursor = page_cursor(&last_entry);
                    // A closed turn reconciles even when the page is the same
                    // one the busy path already folded in: that page was
                    // appended beside the streamed row, and the signature alone
                    // would keep both copies on screen for good.
                    if l.tail_sig == Some((cursor, newest)) && !fresh {
                        continue;
                    }
                    // the walk backwards starts at the oldest row of the page
                    // being shown; a fresh page re-anchors it
                    if !first_entry.is_empty() {
                        *oldest_cursor.borrow_mut() = Some(first_entry.clone());
                    }
                    has_older.set(more);
                    ui.set_has_older(more);
                    if l.open_live.is_some() && !fresh {
                        l.tail_len = messages.len();
                        l.tail_sig = Some((cursor, newest));
                        continue;
                    }
                    l.tail_len = messages.len();
                    l.tail_sig = Some((cursor, newest));
                    *tail_from.borrow_mut() = messages.first().map(transcript::message_key);
                    let (mut built, skipped) = transcript::build(&messages, window_rows);
                    if built.is_empty() && !messages.is_empty() {
                        continue;
                    }
                    if crate::env_cfg::cfg().demo_markdown {
                        built.extend(transcript::markdown_demo_rows());
                    }
                    // The store now carries the message being written: its copy
                    // replaces the streamed rows where they stand. Taking the
                    // turn out and putting it back - the fake Live this arm used
                    // to build - existed only because the range lived out here.
                    let live_in_built = l
                        .open_live
                        .as_ref()
                        .is_some_and(|turn| built.iter().any(|row| row.message == turn.message));
                    if live_in_built || (!ui.get_busy() && !in_flight.get()) {
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
                        eprintln!("[tail] rebuilt {} msgs -> {count} rows (skipped {skipped})", messages.len());
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
                    messages,
                    first_entry,
                    more,
                    ..
                } => {
                    loading_older.set(false);
                    ui.set_loading_older(false);
                    if messages.is_empty() {
                        // nothing to prepend: stop asking rather than loop
                        has_older.set(false);
                        ui.set_has_older(false);
                        continue;
                    }
                    let (older, _) = transcript::build(&messages, messages.len());
                    if older.is_empty() {
                        has_older.set(false);
                        ui.set_has_older(false);
                        continue;
                    }
                    // Where the reader was, and how tall the content was, before
                    // the page goes in. Both are read from the list itself.
                    let before_y = ui.get_dbg_cy() as f32;
                    let before_h = ui.get_dbg_ch() as f32;
                    let older_len = older.len();
                    sheet.prepend(older);
                    // A session of any length must not cost the window more than
                    // a window: the stretch between the reader and the newest
                    // page goes, with one notice standing in for it.
                    if let Some(newest) = tail_from.borrow().clone() {
                        let gone = sheet.narrow(&newest, self.history_messages, HISTORY_UNLOADED);
                        if gone > 0 {
                            if crate::env_cfg::cfg().debug {
                                eprintln!("[narrow] {gone} rows unloaded above the newest page");
                            }
                        }
                    }
                    *oldest_cursor.borrow_mut() = if first_entry.is_empty() {
                        None
                    } else {
                        Some(first_entry)
                    };
                    has_older.set(more && oldest_cursor.borrow().is_some());
                    ui.set_has_older(has_older.get());
                    // a list view measures the new rows a frame or two later, so
                    // the correction is applied again once they have heights
                    for delay in [30u64, 200] {
                        let weak = ui.as_weak();
                        slint::Timer::single_shot(Duration::from_millis(delay), move || {
                            if let Some(ui) = weak.upgrade() {
                                let after = ui.get_dbg_ch() as f32;
                                let y = anchor_after_prepend(before_y, before_h, after);
                                ui.invoke_set_scroll_y(y as f32);
                            }
                        });
                    }
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[older] {} msgs -> {} rows, more={more} after={:?}",
                            messages.len(),
                            older_len,
                            oldest_cursor.borrow()
                        );
                    }
                }
                Msg::Event { session, event } => {
                    // exact TUI vocabulary straight from the stream
                    if let Some(label) = pa_client::activity_label(&event) {
                        ui.set_busy_label(label.into());
                    }
                    // gap detection: a skipped sequence means we missed frames
                    // (daemon restart, a stalled pipe) � backfill from the store
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
                    // The queue's own truth: the daemon pushes the projection
                    // whenever either lane changes, so a delivered message leaves
                    // the rows the moment it stops waiting. Polling alone could
                    // only notice after the fact, and nothing polled at all while
                    // the window sat idle.
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
                        apply_queue(&ui, &lane("steering"), &lane("followUps"), &sent_thumbs);
                        continue;
                    }
                    // A closed turn is reconciled from the store: the live row
                    // holds whatever text last arrived, and one dropped
                    // message_end would otherwise strand the view on a
                    // half-written row for good.
                    if matches!(kind, Some("turn_end" | "agent_end")) {
                        in_flight.set(false);
                        sheet.answer_ended();
                        // The answer stays exactly where it is. Dropping it and
                        // rebuilding from a store that is still one message
                        // behind is what made it collapse to a line and then
                        // come back: the screen may not shrink on the strength
                        // of a store that has not caught up.
                        // The live rows stay, carrying the message they belong
                        // to: the store's copy of that message replaces them
                        // when it arrives, in place.
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
                            // no message is open: the turn's own frames are the
                            // only thing that may draw a cell, and the store
                            // will carry it when the turn closes
                            continue;
                        };
                        if kind == Some("tool_execution_start") {
                            let tool_name = event
                                .get("toolName")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("tool");
                            let args = event
                                .get("args")
                                .cloned()
                                .unwrap_or(serde_json::Value::Null);
                            turn.start_tool(&call_id, tool_name, args);
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
                        // The store owns this row. Drawing it here as well is how
                        // it ended up on screen twice: the rebuild had already
                        // brought it, then the echo drew a second one. Ask for a
                        // poll and let the one owner speak.
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
                            *pending_user.borrow_mut() = None;
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
                    // Ignore toolcall JSON deltas and *_end markers that don't change visible prose/thinking
                    if !delta_kind.is_empty() && !delta_kind.starts_with("text_delta") && !delta_kind.starts_with("thinking_delta") {
                        continue;
                    }
                    if kind == Some("message_end") {
                        continue;
                    }
                    if let Some(delta) = delta_event.get("delta").and_then(serde_json::Value::as_str) {
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
                Msg::Queue { steering, follow_up } => {
                    apply_queue(&ui, &steering, &follow_up, &sent_thumbs);
                }
                Msg::Prompt { session, message, images } => {
                    in_flight.set(true);
                    let _ = jobs.send(Job::Prompt { session, message, images });
                }
                Msg::NewChat(id) => {
                    // PA_SLASH: a look at the command menu in this fresh chat,
                    // never in a session that holds the user's draft.
                    if let Ok(opening) = std::env::var("PA_SLASH") {
                        ui.set_composer_text(opening.as_str().into());
                        ui.set_composer_has_text(!opening.is_empty());
                        let opening_text = ui.get_composer_text().to_string();
                        if let Some(name) = opening_text.trim().strip_prefix('/') {
                            let rows = slash_rows(name, &extra_commands.borrow());
                            if !rows.is_empty() {
                                ui.set_slash_items(ModelRc::from(Rc::new(VecModel::from(rows))));
                                ui.set_slash_open(true);
                            }
                        }
                    }
                    // selection drives everything else: the follow-selection
                    // block below attaches, and the attach reply replaces the
                    // rows. Nothing from the old chat may survive the switch -
                    // a draft is the old chat's, and a queue row belongs to a
                    // session that is still running its own turn.
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_queued_count(0);
                    ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
                    ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
                    ui.set_busy(false);
                    ui.set_title_text("".into());
                    ui.set_status_line("new chat".into());
                    ui.set_selected_id(id.clone().into());
                    if crate::env_cfg::cfg().debug {
                        eprintln!("[new] selected {id}");
                    }
                }
                Msg::QueueEdit { text } => {
                    // the row is out of the queue; the text comes back to where
                    // it came from, so the next Enter decides the lane again
                    ui.set_composer_text(text.into());
                    ui.set_composer_has_text(!ui.get_composer_text().is_empty());
                    ui.set_status_line("back in the composer".into());
                }
                Msg::ExtraCommands(rows) => {
                    *extra_commands.borrow_mut() = rows;
                }
                Msg::ModelCatalog(entries) => {
                    // every group starts folded: 35 providers open at once is a
                    // wall, not a menu. Only the first load seeds that, so a
                    // provider the user unfolds stays unfolded.
                    if !catalog_loaded.get() {
                        collapsed.borrow_mut().extend(
                            entries.iter().map(|entry| entry.provider.clone()),
                        );
                    }
                    catalog_loaded.set(true);
                    *catalog.borrow_mut() = entries;
                    refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
                    // a headless hook: open the menu without a pointer, so the
                    // geometry and the hover path can be read off the log
                    if crate::env_cfg::cfg().menu && !ui.get_model_menu() {
                        // the headless hook needs rows, not 35 folded headers
                        collapsed.borrow_mut().clear();
                        refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
                        ui.set_model_menu(true);
                        eprintln!(
                            "[menu] auto-open box=({},{},{}) height={}",
                            ui.get_box_x(),
                            ui.get_box_y(),
                            ui.get_box_w(),
                            ui.get_model_menu_height()
                        );
                    }
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
                    // A model with no price in the catalog reports zero, and
                    // "$0.00" would claim the run was free: say the number is
                    // unknown rather than invent one.
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
                    // What this session costs on its own understates it once it
                    // has spawned: a child's spend is the same bill. The child
                    // rows carry their own cost and name their parent, so the
                    // family total needs no extra command.
                    let mut children_cost = 0.0f64;
                    let mut has_children = false;
                    if let Ok(cache) = sessions_cache.try_borrow() {
                        let own_sid = cache
                            .iter()
                            .find(|row| row.id == session)
                            .map(|row| row.session_id.clone());
                        if let Some(own_sid) = own_sid.filter(|sid| !sid.is_empty()) {
                            for row in cache.iter().filter(|row| row.parent == own_sid) {
                                has_children = true;
                                children_cost += row.cost;
                            }
                        }
                    }
                    let family = stats.cost + children_cost;
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
                    ui.set_current_effort(thinking_level_label(&state.thinking_level).into());
                    ui.set_current_level(state.thinking_level.clone().into());
                    // the daemon is the truth about which model runs; the chip
                    // follows it rather than the last thing clicked
                    if !state.model_id.is_empty()
                        && (state.model_id != ui.get_model_id().to_string()
                            || state.provider != ui.get_model_provider().to_string())
                    {
                        let label = catalog
                            .borrow()
                            .iter()
                            .find(|entry| {
                                entry.id == state.model_id && entry.provider == state.provider
                            })
                            .map(|entry| model_label(&entry.name, &entry.id))
                            .unwrap_or_else(|| state.model_id.clone());
                        ui.set_model_label(label.into());
                        ui.set_model_id(state.model_id.clone().into());
                        ui.set_model_provider(state.provider.clone().into());
                    }
                    refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
                }
                Msg::Reconnected => {
                    l.loaded = None;
                    l.open_live = None;
                    sheet.answer_ended();
                    l.tail_sig = None;
                    l.last_seq = 0;
                    ui.set_status_line("reconnecting".into());
                }
                Msg::PromptRejected { text } => {
                    // the daemon refused the prompt (usually a stale session id):
                    // put the text back rather than losing what was typed, and
                    // take the row we drew back out - a refused message that
                    // stays on screen is a message that was never sent
                    if ui.get_composer_text().is_empty() {
                        ui.set_composer_text(text.as_str().into());
                    ui.set_composer_has_text(!ui.get_composer_text().is_empty());
                    }
                    pending_user.borrow_mut().take();
                    sheet.take_back_prompt(&text);
                    ui.set_status_line("prompt not accepted � text restored".into());
                }
                Msg::PromptResult { error } => match error {
                    Some(error) => {
                        let text = pending_user.borrow_mut().take().unwrap_or_default();
                        sheet.take_back_prompt(&text);
                        ui.set_status_line(format!("prompt failed: {error}").into());
                    }
                    None => ui.set_status_line("working".into()),
                },
                Msg::Error(error) => ui.set_status_line(format!("error: {error}").into()),
            }
        }

        l.store(self);
    }
}
