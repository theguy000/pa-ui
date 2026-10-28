mod transcript;

slint::include_modules!();

use pa_client::{
    attach_as, clear_queue, create_session, get_queue, get_tail, list_sessions, model_catalog,
    mutate_queue, send_prompt, session_stats, set_model, steer_prompt, stop, Client, SessionRow,
};
use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use transcript::Row;

const WINDOW_ROWS: usize = 300;

/// The TUI's loader frames (pa-tui `LOADER_FRAMES`), advanced every poll tick.
const SPINNER_FRAMES: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];

enum Job {
    Rename { session: String, name: String },
    Attach { session: String },
    Tail { session: String, fresh: bool },
    Prompt { session: String, message: String },
    Steer { session: String, message: String },
    Abort { session: String },
    Queue { session: String },
    ModelCatalog { session: String },
    QueueAct {
        session: String,
        lane: String,
        slot: usize,
        text: String,
        action: String,
    },
    Stats { session: String },
    SetModel { session: String, provider: String, model: String },
    /// start a fresh chat beside the running one (the TUI's `/new`)
    NewChat,
}

enum Msg {
    Sessions(Result<Vec<SessionRow>, String>),
    Transcript { session: String, messages: Vec<serde_json::Value> },
    Tail {
        session: String,
        messages: Vec<serde_json::Value>,
        last_entry: String,
    },
    /// one `session_event` from the attached session, forwarded verbatim
    Event { session: String, event: serde_json::Value },
    PromptResult { error: Option<String> },
    /// the daemon refused a prompt: hand the text back to the composer
    PromptRejected { text: String },
    /// the pipe came back: the old attachment is gone, re-attach
    Reconnected,
    Queue { steering: Vec<String>, follow_up: Vec<String> },
    /// the daemon's model catalog, as the menu builds it
    ModelCatalog(Vec<pa_client::ModelEntry>),
    QueueEdit { text: String },
    /// a `create` came back: the id of the brand new session
    NewChat(String),
    Stats { session: String, stats: pa_client::SessionStats },
    Activity { label: String },
    Error(String),
}

fn main() -> Result<(), slint::PlatformError> {
    let ui = AppWindow::new()?;
    ui.window().set_size(slint::PhysicalSize::new(1200, 830));

    // design check: seed the queue chip without waiting for a slow turn
    if let Ok(count) = std::env::var("PA_QUEUE") {
        if let Ok(count) = count.parse::<i32>() {
            let build = |from: i32, to: i32| -> Vec<QueueItem> {
                (from..=to)
                    .map(|n| {
                        let text = format!("queued message {n}");
                        QueueItem {
                            text: text.clone().into(),
                            raw: text.into(),
                            slot: n - from,
                        }
                    })
                    .collect()
            };
            let mid = count.div_euclid(2).max(1);
            ui.set_queued_count(count);
            ui.set_queued_preview("queued message 1".into());
            ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::from(build(1, mid)))));
            ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::from(if count > mid {
                build(mid + 1, count)
            } else {
                Vec::new()
            }))));
        }
    }


    // one identity for this window: the supervisor addresses session events to
    // the connections registered under it
    let client_id = format!(
        "pa-ui-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let (job_tx, job_rx) = channel::<Job>();
    let (msg_tx, msg_rx) = channel::<Msg>();
    let worker_client_id = client_id.clone();
    std::thread::spawn(move || worker(job_rx, msg_tx, worker_client_id));

    // transcript model, kept alive so a toggle can patch one row
    // the rail's filter and the rows it filters: a keystroke re-filters from
    // the cache instead of waiting for the next poll
    let session_filter: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let sessions_cache: Rc<RefCell<Vec<SessionRow>>> = Rc::new(RefCell::new(Vec::new()));
    let rows_store: Rc<RefCell<Vec<Row>>> = Rc::new(RefCell::new(Vec::new()));
    let row_model: Rc<VecModel<TRow>> = Rc::new(VecModel::default());
    ui.set_transcript(ModelRc::from(row_model.clone()));
    if std::env::var("PA_HOVER").is_ok() {
        ui.set_ctx_hover(true);
    }

    // the text of a prompt we drew optimistically, so the daemon's echo of the
    // same user message does not become a second row
    let pending_user: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // the model catalog and which provider groups the user has folded away.
    // Collapse lives here, not in Slint: it has to survive closing the menu.
    let catalog: Rc<RefCell<Vec<pa_client::ModelEntry>>> =
        Rc::new(RefCell::new(Vec::new()));
    let collapsed: Rc<RefCell<std::collections::HashSet<String>>> =
        Rc::new(RefCell::new(std::collections::HashSet::new()));
    let catalog_loaded = Rc::new(std::cell::Cell::new(false));


    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let rows = rows_store.clone();
        let model = row_model.clone();
        let pending_user = pending_user.clone();
        ui.on_send_requested(move |text| {
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[send] callback fired with {text:?}");
            }
            let message = text.trim().to_string();
            if message.is_empty() {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                ui.set_status_line("no session selected".into());
                return;
            }
            ui.set_composer_text("".into());
            // optimistic row: the prompt shows before the daemon echoes it
            let echo = serde_json::json!({
                "role": "user",
                "content": [{"type": "text", "text": message}],
            });
            if let Some(row) = transcript::build(std::slice::from_ref(&echo), 1).0.first() {
                rows.borrow_mut().push(row.clone());
                model.push(to_slint(row));
                ui.invoke_scroll_to_end();
            }
            *pending_user.borrow_mut() = Some(message.clone());
            if let Some(ui) = weak.upgrade() {
                ui.set_status_line("sent".into());
            }
            let _ = jobs.send(Job::Prompt { session, message });
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let catalog = catalog.clone();
        let collapsed2 = collapsed.clone();
        let collapsed = collapsed2.clone();
        ui.on_model_picked(move |model, provider| {
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            if provider.trim().is_empty() {
                // no provider known for this entry: say so instead of guessing
                ui.set_status_line(format!("{model}: provider unknown, not switching").into());
                return;
            }
            // the chip follows the click; the daemon's own state lands on the
            // next list refresh
            if let Some(entry) = catalog.borrow().iter().find(|entry| {
                entry.id == model.as_str() && entry.provider == provider.as_str()
            }) {
                ui.set_model_label(model_label(&entry.name, &entry.id).into());
            }
            ui.set_model_id(model.clone());
            ui.set_model_provider(provider.clone());
            refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
            ui.set_status_line(format!("switching to {model}").into());
            let _ = jobs.send(Job::SetModel {
                session,
                provider: provider.to_string(),
                model: model.to_string(),
            });
        });
    }

    {
        // opening the menu pulls the catalog once; after that it is cached, and
        // only the group fold state changes
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let catalog_loaded = catalog_loaded.clone();
        ui.on_model_menu_opened(move || {
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            if !catalog_loaded.get() {
                let _ = jobs.send(Job::ModelCatalog { session });
            }
        });
    }

    {
        // folding a group is remembered for the rest of the session, so a menu
        // reopened later comes back exactly as it was left
        let weak = ui.as_weak();
        let catalog = catalog.clone();
        let collapsed = collapsed.clone();
        ui.on_provider_toggled(move |provider| {
            let Some(ui) = weak.upgrade() else { return };
            let provider = provider.to_string();
            {
                let mut folded = collapsed.borrow_mut();
                if !folded.remove(&provider) {
                    folded.insert(provider);
                }
            }
            refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        ui.on_queue_act(move |lane, slot, action| {
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[queue] act {action} {lane}[{slot}]");
            }
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            // The row's own text is the daemon's expectedText: it proves the
            // client is editing the row it actually showed.
            let text = if lane == "steering" {
                ui.get_queued_steering()
                    .row_data(slot.max(0) as usize)
                    .map(|row| row.raw.to_string())
            } else {
                ui.get_queued_followup()
                    .row_data(slot.max(0) as usize)
                    .map(|row| row.raw.to_string())
            };
            let Some(text) = text else { return };
            let _ = jobs.send(Job::QueueAct {
                session,
                lane: lane.to_string(),
                slot: slot.max(0) as usize,
                text,
                action: action.to_string(),
            });
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        ui.on_abort_requested(move || {
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            ui.set_status_line("aborting".into());
            let _ = jobs.send(Job::Abort { session });
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let pending_user = pending_user.clone();
        ui.on_steer_requested(move |text| {
            let message = text.trim().to_string();
            if message.is_empty() {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            ui.set_composer_text("".into());
            *pending_user.borrow_mut() = None;
            ui.set_status_line("steered".into());
            let _ = jobs.send(Job::Steer { session, message });
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        ui.on_queue_clear(move || {
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            ui.set_status_line("clearing the queue".into());
            let _ = jobs.send(Job::QueueAct {
                session,
                lane: String::new(),
                slot: 0,
                text: String::new(),
                action: "clear".into(),
            });
        });
    }

    {
        let weak = ui.as_weak();
        let filter = session_filter.clone();
        let cache = sessions_cache.clone();
        ui.on_search_edited(move |query| {
            let query = query.to_string();
            *filter.borrow_mut() = query.clone();
            let rows = cache.borrow().clone();
            let items = session_items(&rows, &query);
            if let Some(ui) = weak.upgrade() {
                ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(items))));
            }
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        ui.on_new_chat(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_status_line("starting a new chat".into());
            }
            let _ = jobs.send(Job::NewChat);
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        ui.on_rename_requested(move |session, name| {
            if let Some(ui) = weak.upgrade() {
                ui.set_title_text(name.clone());
            }
            let session = session.to_string();
            if session.is_empty() || name.trim().is_empty() {
                return;
            }
            let _ = jobs.send(Job::Rename { session, name: name.trim().to_string() });
        });
    }

    {
        let rows = rows_store.clone();
        let model = row_model.clone();
        ui.on_toggle_row(move |index| {
            let index = index as usize;
            let mut rows = rows.borrow_mut();
            let Some(row) = rows.get_mut(index) else { return };
            row.expanded = !row.expanded;
            let converted = to_slint(row);
            drop(rows);
            if index < model.row_count() {
                model.set_row_data(index, converted);
            }
        });
    }

    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let rows_store_ui = rows_store.clone();
    let row_model_ui = row_model.clone();
    let mut loaded: Option<String> = None;
    let mut live_state = String::new();
    let mut live_since = SystemTime::now();
    let mut busy_label = String::new();
    let busy_since = Rc::new(std::cell::Cell::new(SystemTime::now()));
    // how many messages of the attached session we have already turned into rows
    let mut tail_len = 0usize;
    // a fixed-size window keeps its length while its contents move: never
    // treat an unchanged count as "nothing new"
    let mut tail_sig: Option<(u64, u64)> = None;
    let mut last_seq = 0u64;
    // the row currently receiving `message_update` frames: (key, model index)
    let mut live_row: Option<(String, usize)> = None;
    // when the stream last moved: a live row that has gone quiet is not proof
    // the turn is still writing, only that we stopped hearing about it
    let mut stream_at = SystemTime::now();
    // the selected session's uuid: survives the id re-keying a daemon restart causes
    let mut selected_uuid: Option<String> = None;
    let mut tail_at = SystemTime::now();
    let mut queue_at = SystemTime::now();
    let mut stats_at = SystemTime::now();
    let window_rows: usize = std::env::var("PA_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(WINDOW_ROWS);
    // the spinner runs on its own clock — 10 frames at 80 ms is one 0.8 s turn,
    // independent of how often the daemon happens to send status
    let spin_weak = ui.as_weak();
    let spin_index = std::cell::Cell::new(0usize);
    let spin_since = busy_since.clone();
    let spin_secs = std::cell::Cell::new(u64::MAX);
    let spin = Timer::default();
    spin.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let Some(ui) = spin_weak.upgrade() else { return };
        if !ui.get_busy() {
            return;
        }
        spin_index.set(spin_index.get().wrapping_add(1));
        ui.set_spinner_frame(SPINNER_FRAMES[spin_index.get() % SPINNER_FRAMES.len()].into());
        let secs = spin_since.get().elapsed().unwrap_or_default().as_secs();
        if secs != spin_secs.get() {
            spin_secs.set(secs);
            ui.set_busy_elapsed(format!("{secs}s").into());
        }
    });

    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        let Some(ui) = weak.upgrade() else { return };

        if std::env::var("PA_DEBUG").is_ok() {
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
        }

        // right-panel telemetry
        if !loaded.as_deref().unwrap_or("").is_empty()
            && stats_at.elapsed().unwrap_or_default()
                >= Duration::from_millis(if ui.get_busy() { 3000 } else { 15000 })
        {
            stats_at = SystemTime::now();
            if let Some(session) = loaded.clone() {
                let _ = jobs.send(Job::Stats { session });
            }
        }

        // what is stacked behind the running turn
        if ui.get_busy() && !loaded.as_deref().unwrap_or("").is_empty()
            // PA_QUEUE is a design check: keep the seeded lanes on screen
            && std::env::var("PA_QUEUE").is_err()
            && queue_at.elapsed().unwrap_or_default() >= Duration::from_millis(1500)
        {
            queue_at = SystemTime::now();
            if let Some(session) = loaded.clone() {
                let _ = jobs.send(Job::Queue { session });
            }
        }

        // while the agent works, keep pulling the tail of the transcript so
        // completed tool calls show up as rows — the snapshot alone never updates
        // Poll while a turn runs, and slowly the rest of the time. Gating this
        // on `busy` alone meant one missed `message_end` froze the transcript
        // for good: nothing would ever ask the store again.
        let poll_every = if ui.get_busy() {
            Duration::from_millis(900)
        } else {
            Duration::from_millis(3000)
        };
        if !loaded.as_deref().unwrap_or("").is_empty()
            && tail_at.elapsed().unwrap_or_default() >= poll_every
        {
            tail_at = SystemTime::now();
            if let Some(session) = loaded.clone() {
                let _ = jobs.send(Job::Tail { session, fresh: false });
            }
        }

        // follow the selection
        let selected = ui.get_selected_id().to_string();
        if !selected.is_empty() && loaded.as_deref() != Some(selected.as_str()) {
            loaded = Some(selected.clone());
            ui.set_status_line("loading transcript".into());
            let _ = jobs.send(Job::Attach { session: selected });
        }

        while let Ok(message) = msg_rx.try_recv() {
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
                        if let Some(uuid) = selected_uuid.as_deref() {
                            if let Some(rebound) =
                                rows.iter().find(|row| row.session_id == uuid).map(|row| row.id.clone())
                            {
                                if std::env::var("PA_DEBUG").is_ok() {
                                    eprintln!("[rebind] {selected} -> {rebound}");
                                }
                                ui.set_selected_id(rebound.clone().into());
                                selected_uuid = Some(uuid.to_string());
                                loaded = None;
                                tail_len = 0;
                                tail_sig = None;
                                last_seq = 0;
                                live_row = None;
                            }
                        }
                    }
                    let selected = ui.get_selected_id().to_string();
                    if selected_uuid.is_none() {
                        selected_uuid = uuid_of(&rows, &selected).map(str::to_string);
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
                        busy_label.clear();
                    }
                    if let Some(row) = rows.iter().find(|row| row.id == selected) {
                        let state = row.live_state();
                        if state != live_state {
                            live_state = state.to_string();
                            live_since = SystemTime::now();
                        }
                        let busy_now = state != "idle" && state != "archived";
                        if !busy_now && ui.get_busy() {
                            ui.set_busy(false);
                            busy_label.clear();
                            // the run just ended — grab whatever landed after the
                            // last refresh so the final tool calls are not lost
                            if let Some(session) = loaded.clone() {
                                let _ = jobs.send(Job::Tail { session, fresh: false });
                            }
                        }
                        if busy_now && !ui.get_busy() {
                            if busy_label.is_empty() {
                                busy_label = state.to_string();
                                busy_since.set(SystemTime::now());
                                ui.set_busy_label(busy_label.clone().into());
                            }
                            ui.set_busy(true);
                        }
                        if ui.get_busy() {
                            let held = busy_since.get().elapsed().unwrap_or_default().as_secs();
                            ui.set_busy_elapsed(format!("{held}s").into());
                        }
                        let held = live_since.elapsed().unwrap_or_default().as_secs();
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
                    if label != busy_label {
                        busy_label = label;
                        busy_since.set(SystemTime::now());
                        ui.set_busy_label(busy_label.clone().into());
                    }
                    ui.set_busy(true);
                }
                Msg::Transcript { session, messages } => {
                    if loaded.as_deref() != Some(session.as_str()) {
                        continue; // a newer selection won
                    }
                    tail_len = messages.len();
                    let (mut built, skipped) = transcript::build(&messages, window_rows);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!(
                            "[attach] {} msgs -> {} rows (skipped {skipped})",
                            messages.len(),
                            built.len()
                        );
                    }
                    if std::env::var("PA_DEMO").as_deref() == Ok("fail") {
                        built.extend(transcript::demo_rows());
                    }
                    // a prompt sent while this attach was in flight: the snapshot
                    // predates it, so the optimistic row has to be re-appended
                    if let Some(text) = pending_user.borrow().as_deref() {
                        let echo = serde_json::json!({
                            "role": "user",
                            "content": [{"type": "text", "text": text}],
                        });
                        let already = built.iter().rev().take(6).any(|row| {
                            row.kind == transcript::KIND_USER && row.title.contains(text)
                        });
                        if !already {
                            if let Some(row) =
                                transcript::build(std::slice::from_ref(&echo), 1).0.first()
                            {
                                built.push(row.clone());
                            }
                        }
                    }
                    let slint_rows: Vec<TRow> = built.iter().map(to_slint).collect();
                    let count = slint_rows.len();
                    *rows_store_ui.borrow_mut() = built;
                    row_model_ui.set_vec(slint_rows);
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
                } => {
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!(
                            "[tail] {session} {} msgs (have {}) newest={:?}",
                            messages.len(),
                            tail_len,
                            messages
                                .last()
                                .map(|m| m.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(0))
                                .unwrap_or(0)
                        );
                    }
                    if loaded.as_deref() != Some(session.as_str()) {
                        continue;
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
                    if tail_sig == Some((cursor, newest)) {
                        continue;
                    }
                    // A row is being streamed: the event path owns the view,
                    // and rebuilding now would drop the in-progress row. The
                    // next poll after the turn ends reconciles from the store.
                    if live_row.is_some()
                        && stream_at.elapsed().unwrap_or_default() < Duration::from_millis(2000)
                    {
                        continue;
                    }
                    // a quiet live row is replaced by the store's copy
                    live_row = None;
                    tail_len = messages.len();
                    tail_sig = Some((cursor, newest));
                    let (built, skipped) = transcript::build(&messages, window_rows);
                    if built.is_empty() && !messages.is_empty() {
                        continue;
                    }
                    let count = built.len();
                    let slint_rows: Vec<TRow> = built.iter().map(to_slint).collect();
                    *rows_store_ui.borrow_mut() = built;
                    row_model_ui.set_vec(slint_rows);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!(
                            "[tail] rebuilt {} msgs -> {count} rows (skipped {skipped})",
                            messages.len()
                        );
                    }
                    scroll_to_end_when_ready(&ui.as_weak());
                }
                Msg::Event { session, event } => {
                    if loaded.as_deref() != Some(session.as_str()) {
                        continue;
                    }
                    // exact TUI vocabulary straight from the stream
                    if let Some(label) = pa_client::activity_label(&event) {
                        ui.set_busy_label(label.into());
                    }
                    // gap detection: a skipped sequence means we missed frames
                    // (daemon restart, a stalled pipe) — backfill from the store
                    if let Some(sequence) = event
                        .pointer("/meta/sequence")
                        .and_then(serde_json::Value::as_u64)
                    {
                        if last_seq != 0 && sequence > last_seq + 1 {
                            let _ = jobs.send(Job::Tail {
                                session: session.clone(),
                                fresh: false,
                            });
                        }
                        last_seq = sequence.max(last_seq);
                    }

                    let kind = event.get("type").and_then(serde_json::Value::as_str);
                    // A closed turn is reconciled from the store: the live row
                    // holds whatever text last arrived, and one dropped
                    // message_end would otherwise strand the view on a
                    // half-written row for good.
                    if matches!(kind, Some("turn_end" | "agent_end")) {
                        live_row = None;
                        stream_at = SystemTime::now() - Duration::from_secs(60);
                        let _ = jobs.send(Job::Tail {
                            session: session.clone(),
                            fresh: true,
                        });
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
                        // our own prompt echo: the optimistic row already exists
                        let text = message_text(message);
                        let mut slot = pending_user.borrow_mut();
                        if slot.as_deref() == Some(text.trim()) {
                            if kind == Some("message_end") {
                                *slot = None;
                            }
                            continue;
                        }
                    }
                    let stamp = message
                        .get("timestamp")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    let key = format!("{}:{stamp}", role.unwrap_or_default());
                    let built = transcript::build(std::slice::from_ref(message), 1).0;
                    let Some(row) = built.first().cloned() else {
                        continue;
                    };
                    let open = live_row.clone().filter(|(open_key, _)| open_key == &key);
                    match open {
                        // the same message, growing: repaint that row in place
                        Some((_, index)) => {
                            if let Some(slot) = rows_store_ui.borrow_mut().get_mut(index) {
                                *slot = row.clone();
                            }
                            row_model_ui.set_row_data(index, to_slint(&row));
                        }
                        None => {
                            let mut store = rows_store_ui.borrow_mut();
                            store.push(row.clone());
                            let index = store.len() - 1;
                            drop(store);
                            row_model_ui.push(to_slint(&row));
                            live_row = Some((key.clone(), index));
                            scroll_to_end_when_ready(&ui.as_weak());
                        }
                    }
                    stream_at = SystemTime::now();
                    if std::env::var("PA_DEBUG").is_ok() && kind == Some("message_update") {
                        eprintln!(
                            "[stream] {key} patch row {} ({} chars)",
                            live_row.as_ref().map_or(usize::MAX, |(_, index)| *index),
                            message_text(message).chars().count()
                        );
                    }
                    if kind == Some("message_end") {
                        live_row = None;
                    }
                }
                Msg::Queue { steering, follow_up } => {
                    let total = steering.len() + follow_up.len();
                    let first = steering
                        .first()
                        .or_else(|| follow_up.first())
                        .cloned()
                        .unwrap_or_default();
                    ui.set_queued_count(total as i32);
                    ui.set_queued_preview(first.into());
                    // Each row keeps its lane and its slot in that lane, which is
                    // what the daemon wants back when one is edited or promoted.
                    let lane_items = |texts: &[String]| -> Vec<QueueItem> {
                        texts
                            .iter()
                            .enumerate()
                            .map(|(slot, text)| {
                                let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                                let cut: String = flat.chars().take(120).collect();
                                QueueItem {
                                    text: cut.into(),
                                    raw: text.clone().into(),
                                    slot: slot as i32,
                                }
                            })
                            .collect()
                    };
                    ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::from(lane_items(
                        &steering,
                    )))));
                    ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::from(lane_items(
                        &follow_up,
                    )))));
                }
                Msg::NewChat(id) => {
                    // selection drives everything else: the follow-selection
                    // block below attaches, and the attach reply replaces the
                    // rows. Nothing from the old chat may survive the switch -
                    // a draft is the old chat's, and a queue row belongs to a
                    // session that is still running its own turn.
                    ui.set_composer_text("".into());
                    ui.set_queued_count(0);
                    ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
                    ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
                    ui.set_busy(false);
                    ui.set_title_text("".into());
                    ui.set_status_line("new chat".into());
                    ui.set_selected_id(id.clone().into());
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[new] selected {id}");
                    }
                }
                Msg::QueueEdit { text } => {
                    // the row is out of the queue; the text comes back to where
                    // it came from, so the next Enter decides the lane again
                    ui.set_composer_text(text.into());
                    ui.set_status_line("back in the composer".into());
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
                }
                Msg::Stats { session, stats } => {
                    if loaded.as_deref() != Some(session.as_str()) {
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
                            "{} of {} · {:.0}%",
                            tokens(stats.context_tokens),
                            tokens(stats.context_window),
                            stats.percent
                        )
                        .into(),
                    );
                    ui.set_stats_text(
                        format!(
                            "{} messages · {} tools\nin {} · out {} · cache {}",
                            stats.messages,
                            stats.tool_calls,
                            tokens(stats.input_tokens),
                            tokens(stats.output_tokens),
                            tokens(stats.cache_read)
                        )
                        .into(),
                    );
                }
                Msg::Reconnected => {
                    loaded = None;
                    live_row = None;
                    tail_sig = None;
                    last_seq = 0;
                    ui.set_status_line("reconnecting".into());
                }
                Msg::PromptRejected { text } => {
                    // the daemon refused the prompt (usually a stale session id):
                    // put the text back rather than losing what was typed
                    if ui.get_composer_text().is_empty() {
                        ui.set_composer_text(text.as_str().into());
                    }
                    pending_user.borrow_mut().take();
                    ui.set_status_line("prompt not accepted — text restored".into());
                }
                Msg::PromptResult { error } => match error {
                    Some(error) => {
                        pending_user.borrow_mut().take();
                        ui.set_status_line(format!("prompt failed: {error}").into());
                    }
                    None => ui.set_status_line("working".into()),
                },
                Msg::Error(error) => ui.set_status_line(format!("error: {error}").into()),
            }
        }
    });
    std::mem::forget(poll);

    ui.run()
}

/// `river_ai` reads better as `river ai`; the catalog's own names are long.
fn provider_label(provider: &str) -> String {
    provider.replace('_', " ")
}

/// The catalog's `name` repeats the provider ("... (river.ai)"); the group
/// header already says it.
fn model_label(name: &str, id: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return id.to_string();
    }
    match trimmed.rfind(" (") {
        Some(at) if trimmed.ends_with(')') => trimmed[..at].to_string(),
        _ => trimmed.to_string(),
    }
}

/// A rough size for the right-hand hint: `200k`, `1M`.
fn context_hint(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        String::new()
    }
}

/// Rebuild the menu: configured providers first, then the rest, alphabetical.
/// A collapsed group carries no models at all, so the panel never instantiates
/// the whole catalog.
fn refresh_model_groups(
    ui: &AppWindow,
    catalog: &[pa_client::ModelEntry],
    collapsed: &std::collections::HashSet<String>,
) {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(bool, String), Vec<&pa_client::ModelEntry>> = BTreeMap::new();
    for entry in catalog {
        groups
            .entry((!entry.configured, entry.provider.clone()))
            .or_default()
            .push(entry);
    }
    let current_id = ui.get_model_id().to_string();
    let current_provider = ui.get_model_provider().to_string();
    let mut out: Vec<ModelGroup> = Vec::with_capacity(groups.len());
    let mut height = 12f32;
    for ((_, provider), mut entries) in groups {
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let open = !collapsed.contains(&provider);
        let configured = entries.first().is_some_and(|entry| entry.configured);
        let count = entries.len() as i32;
        height += 26.0;
        let models: Vec<ModelItem> = if open {
            height += 24.0 * entries.len() as f32;
            entries
                .iter()
                .map(|entry| ModelItem {
                    id: entry.id.clone().into(),
                    label: model_label(&entry.name, &entry.id).into(),
                    detail: context_hint(entry.context_window).into(),
                    current: entry.id == current_id && provider == current_provider,
                })
                .collect()
        } else {
            Vec::new()
        };
        out.push(ModelGroup {
            provider: provider.clone().into(),
            label: provider_label(&provider).into(),
            count,
            open,
            configured,
            models: ModelRc::from(Rc::new(VecModel::from(models))),
        });
    }
    ui.set_model_menu_height(height.min(340.0));
    ui.set_model_groups(ModelRc::from(Rc::new(VecModel::from(out))));
}

/// A stable number for a page cursor, so the poll can tell "same page" quickly.
fn page_cursor(last_entry: &str) -> u64 {
    last_entry
        .bytes()
        .fold(0u64, |hash, byte| hash.wrapping_mul(31).wrapping_add(u64::from(byte)))
}

fn worker(jobs: Receiver<Job>, out: Sender<Msg>, client_id: String) {
    let mut client: Option<Client> = None;

    // Reads go over a connection that never attaches: once a connection is
    // attached to a session the daemon serves its store view, which lags the
    // live agent by whole turns. A bare connection always answers with the
    // current messages (verified: attached 106 frozen while a fresh one read
    // 108 and kept moving).

    let mut last_list = SystemTime::now() - Duration::from_secs(60);
    let mut attached: Option<String> = None;
    // prompts awaiting a daemon verdict: a fire-and-forget send would otherwise
    // swallow a rejection and the message would vanish with no trace
    let mut pending_prompts: Vec<(String, String)> = Vec::new();
    let mut resume: Option<(String, u64)> = None;

    loop {
        if client.is_none() {
            match Client::connect() {
                Ok(connected) => {
                    client = Some(connected);
                    // a new connection has no attachments: the UI re-attaches
                    // with the resume cursor so the daemon replays the gap
                    attached = None;
                    resume = None;
                    let _ = out.send(Msg::Reconnected);
                }
                Err(error) => {
                    let _ = out.send(Msg::Error(error.to_string()));
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }
        }
        let Some(active) = client.as_ref() else { continue };
        // the daemon may have restarted under us; without this the app keeps
        // reading a dead pipe and the transcript freezes until it is restarted
        if !active.alive() {
            client = None;
            attached = None;
            let _ = out.send(Msg::Error("daemon connection lost — reconnecting".into()));
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }

        while let Ok(job) = jobs.try_recv() {
            match job {
                Job::Rename { session, name } => {
                    let _ = active.send(
                        "rename",
                        serde_json::json!({"activeSessionId": session, "name": name}),
                    );
                    last_list = SystemTime::now() - Duration::from_secs(60);
                }
                Job::Attach { session } => match attach_as(active, &session, Some(&client_id), resume.clone()) {
                    Ok(messages) => {
                        attached = Some(session.clone());
                        let _ = out.send(Msg::Transcript { session, messages });
                    }
                    Err(error) => {
                        let _ = out.send(Msg::Error(error.to_string()));
                    }
                },
                Job::SetModel {
                    session,
                    provider,
                    model,
                } => {
                    let outcome = set_model(active, &session, &provider, &model);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[model] {provider}/{model} -> {outcome:?}");
                    }
                    // a provider that is not signed in fails here: say so rather
                    // than leaving the chip naming a model the session is not on
                    if let Err(error) = outcome {
                        let _ = out.send(Msg::Activity {
                            label: format!("{provider}/{model}: {error}"),
                        });
                    }
                }
                Job::NewChat => match create_session(active) {
                    Ok(id) => {
                        if std::env::var("PA_DEBUG").is_ok() {
                            eprintln!("[new] created {id}");
                        }
                        let _ = out.send(Msg::NewChat(id));
                    }
                    Err(error) => {
                        let _ = out.send(Msg::Activity {
                            label: format!("new chat: {error}"),
                        });
                    }
                },
                Job::Stats { session } => {
                    if let Ok(stats) = session_stats(active, &session) {
                        let _ = out.send(Msg::Stats { session, stats });
                    }
                }
                Job::ModelCatalog { session } => {
                    match model_catalog(active, &session) {
                        Ok(catalog) => {
                            if std::env::var("PA_DEBUG").is_ok() {
                                eprintln!("[models] {} entries", catalog.len());
                            }
                            let _ = out.send(Msg::ModelCatalog(catalog));
                        }
                        Err(error) => {
                            let _ = out.send(Msg::Activity {
                                label: format!("model catalog: {error}"),
                            });
                        }
                    }
                }
                Job::Queue { session } => {
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        let _ = out.send(Msg::Queue { steering, follow_up });
                    }
                }
                Job::QueueAct {
                    session,
                    lane,
                    slot,
                    text,
                    action,
                } => {
                    // `clear` is not a mutation of one row: it empties both lanes
                    if action == "clear" {
                        let outcome = clear_queue(active, &session);
                        if std::env::var("PA_DEBUG").is_ok() {
                            eprintln!("[queue] clear -> {outcome:?}");
                        }
                        if let Ok((steering, follow_up)) = get_queue(active, &session) {
                            let _ = out.send(Msg::Queue { steering, follow_up });
                        }
                        continue;
                    }
                    let mutation = match action.as_str() {
                        // a follow-up promoted to the steering lane is the same
                        // text re-laned: the daemon delivers it inside the turn
                        // that is already running
                        "promote" => serde_json::json!({
                            "type": "replace",
                            "lane": "steering",
                            "text": text,
                        }),
                        // editing pulls the message back into the composer, so
                        // the queue entry goes away first
                        _ => serde_json::json!({"type": "delete"}),
                    };
                    let outcome = mutate_queue(active, &session, &lane, slot, &text, mutation);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[queue] {action} {lane}[{slot}] -> {outcome:?}");
                    }
                    match outcome {
                        Ok(status) if status == "applied" => {
                            if action == "edit" {
                                let _ = out.send(Msg::QueueEdit { text: text.clone() });
                            }
                            // the lanes as the daemon now has them
                            if let Ok((steering, follow_up)) = get_queue(active, &session) {
                                let _ = out.send(Msg::Queue { steering, follow_up });
                            }
                        }
                        Ok(_) => {
                            let _ = out.send(Msg::Activity {
                                label: "that queue row changed already".into(),
                            });
                        }
                        Err(error) => {
                            let _ = out.send(Msg::Activity { label: error.to_string() });
                        }
                    }
                }
                Job::Abort { session } => {
                    // `stop` = abort_and_send_queued when the daemon has it, so the
                    // queue keeps flowing behind the stopped run
                    if let Err(error) = stop(active, &session) {
                        let _ = out.send(Msg::PromptResult { error: Some(error.to_string()) });
                    }
                }
                Job::Steer { session, message } => {
                    let outcome = steer_prompt(active, &session, &message);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[steer] session={session} message={message:?} outcome={outcome:?}");
                    }
                    if let Err(error) = outcome {
                        let _ = out.send(Msg::PromptResult { error: Some(error.to_string()) });
                    }
                }
                Job::Prompt { session, message } => {
                    let outcome = send_prompt(active, &session, &message);
                    if let Ok(id) = &outcome {
                        pending_prompts.push((id.clone(), message.clone()));
                    }
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[prompt] session={session} message={message:?} outcome={outcome:?}");
                    }
                    let error = outcome.err().map(|e| e.to_string());
                    let _ = out.send(Msg::PromptResult { error });
                }
                Job::Tail { session, fresh } => {
                    // Event frames are droppable: the daemon publishes them with
                    // try_send into a bounded per-connection queue, so a client
                    // that falls behind simply loses message_update/message_end -
                    // silently, with no notice. That is why a turn can stop
                    // half-written. The store is the authority.
                    //
                    // Which store view you get depends on the connection: a
                    // long-lived one keeps answering from the snapshot it fell
                    // out of step with (measured: store at 18 while it returned
                    // 17 for 30s), while a freshly opened connection always
                    // answers with current messages. So a forced sync opens a new
                    // connection and drops it; ordinary polls reuse one.
                    // Every read opens its own connection: a connection that has
                    // been used once keeps answering with the snapshot it saw at
                    // connect time (measured 102 -> 102 across nine seconds
                    // while fresh connections advanced 96 -> 108). The cost is a
                    // local pipe open per poll; the alternative is a window that
                    // stops one turn short.
                    let _ = fresh;
                    // one tail page, not the whole branch: the typed cursor the
                    // terminal UI uses, so a long session costs the same as a
                    // short one
                    let read = get_tail(active, &session, WINDOW_ROWS);
                    match read {
                    Ok((messages, last_entry, _more)) => {
                        let _ = out.send(Msg::Tail {
                            session,
                            messages,
                            last_entry,
                        });
                    }
                    Err(error) => {
                        if std::env::var("PA_DEBUG").is_ok() {
                            eprintln!("[tail] {session} FAILED: {error}");
                        }
                    }
                    }
                }
            }
        }

        // drain live events for the attached session: this is what turns a
        // coarse "running tool" into the TUI's Thinking / Writing / Writing code
        if attached.is_some() {
            while let Some(frame) = active.recv(Duration::from_millis(0)) {
                let frame_type = frame
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("?");
                if std::env::var("PA_DEBUG").is_ok() {
                    let event_type = frame
                        .get("event")
                        .and_then(|event| event.get("type"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("");
                    eprintln!("[frame] {frame_type} {event_type}");
                    if std::env::var("PA_DUMP").is_ok() {
                        let text = frame.to_string();
                        eprintln!("[dump] {}", &text[..text.len().min(700)]);
                    }
                }
                if frame_type == "response" {
                    let id = frame.get("id").and_then(serde_json::Value::as_str).unwrap_or("");
                    if let Some(position) = pending_prompts.iter().position(|(open, _)| open == id) {
                        let (_, text) = pending_prompts.remove(position);
                        if frame.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
                            let reason = frame
                                .get("error")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("rejected by the daemon");
                            let _ = out.send(Msg::PromptRejected { text });
                            let _ = out.send(Msg::Error(reason.to_string()));
                        }
                    }
                    continue;
                }
                if frame_type != "session_event" {
                    continue;
                }
                let session = frame
                    .get("activeSessionId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if Some(session.to_string()) != attached {
                    continue;
                }
                if let Some(meta) = frame.get("meta") {
                    let generation = meta
                        .get("generation")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default();
                    let sequence = meta
                        .get("sequence")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0);
                    if !generation.is_empty() && sequence > 0 {
                        resume = Some((generation.to_string(), sequence));
                    }
                }
                if let Some(event) = frame.get("event") {
                    let _ = out.send(Msg::Event {
                        session: session.to_string(),
                        event: event.clone(),
                    });
                    if let Some(label) = pa_client::activity_label(event) {
                        let _ = out.send(Msg::Activity { label: label.to_string() });
                    }
                }
            }
        }

        if last_list.elapsed().unwrap_or_default() > Duration::from_millis(2000) {
            let result = list_sessions(active);
            if result.is_err() {
                client = None;
            }
            let _ = out.send(Msg::Sessions(result.map_err(|error| error.to_string())));
            last_list = SystemTime::now();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Scroll the transcript to the newest row, retrying while the layout settles.
///
/// A single call can land before `content-height` is known, which leaves the
/// view at the top — the exact bug that made a chat open on its oldest row.
fn scroll_to_end_when_ready(ui: &slint::Weak<AppWindow>) {
    for delay in [30u64, 120, 320, 700, 1500, 3000] {
        let ui = ui.clone();
        slint::Timer::single_shot(Duration::from_millis(delay), move || {
            if let Some(ui) = ui.upgrade() {
                ui.invoke_scroll_to_end();
            }
        });
    }
}

/// 1234 → "1.2k", 47_011_000 → "47.0M".
fn tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.0}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}

/// Concatenated text of a wire message's content parts.
fn message_text(message: &serde_json::Value) -> String {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text"))
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

fn to_slint(row: &Row) -> TRow {
    TRow {
        kind: row.kind,
        title: row.title.clone().into(),
        meta: row.meta.clone().into(),
        body: transcript::styled(&row.body),
        code: transcript::styled(&row.code),
        output: transcript::styled(&row.output),
        has_code: row.has_code,
        has_output: row.has_output,
        expanded: row.expanded,
        failed: row.failed,
        reason: row.reason.clone().into(),
    }
}

fn to_item(row: &SessionRow) -> SessionItem {
    let status = if row.lifecycle != "live" {
        "parked"
    } else if row.activity == "working" || row.is_streaming {
        "working"
    } else {
        "idle"
    };
    let full = display_name(row);
    // every row gets a card: consistency was asked for explicitly, twice
    let tip = full.clone();
    SessionItem {
        id: row.id.clone().into(),
        uuid: row.session_id.clone().into(),
        name: trim_to(&full, 90).into(),
        age: age_of(if row.last_activity.is_empty() {
            &row.modified
        } else {
            &row.last_activity
        })
        .into(),
        full,
        tip,
        status: status.into(),
        scroll: marquee_shift(&trim_to(&display_name(row), 90)),
    }
}

/// The rail's rows for a query: a session matches if its name or either id
/// carries the text, case aside. An empty query keeps everything.
fn session_items(rows: &[SessionRow], query: &str) -> Vec<SessionItem> {
    let needle = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            needle.is_empty()
                || display_name(row).to_lowercase().contains(&needle)
                || row.id.to_lowercase().contains(&needle)
                || row.session_id.to_lowercase().contains(&needle)
        })
        .map(to_item)
        .collect()
}

/// The session the selection follows: ids are re-keyed by a daemon restart,
/// the session's uuid is not.
fn uuid_of<'a>(rows: &'a [SessionRow], id: &str) -> Option<&'a str> {
    rows.iter()
        .find(|row| row.id == id)
        .map(|row| row.session_id.as_str())
}

fn marquee_shift(name: &str) -> f32 {
    let width = name.chars().count() as f32 * 6.6;
    (width - 178.0).max(0.0)
}

fn menu_width(models: &[ModelItem]) -> f32 {
    let widest = models
        .iter()
        .map(|model| model.label.chars().count() as f32 * 6.9)
        .fold(0.0f32, f32::max);
    (widest + 44.0).clamp(120.0, 220.0)
}

fn display_name(row: &SessionRow) -> SharedString {
    let name = row.name.trim();
    let usable = !name.is_empty() && name != "(no messages)";
    if usable {
        return trim_to(name, 80).into();
    }
    let tail = |text: &str, width: usize| -> String {
        let count = text.chars().count();
        text.chars().skip(count.saturating_sub(width)).collect()
    };
    if let Some(last) = row.cwd.split(['\\', '/']).filter(|part| !part.is_empty()).next_back() {
        return format!("{last} · {}", tail(&row.id, 6)).into();
    }
    tail(&row.id, 10).into()
}

fn trim_to(text: &str, width: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= width {
        return cleaned;
    }
    cleaned.chars().take(width.saturating_sub(1)).chain(['…']).collect()
}

fn age_of(stamp: &str) -> String {
    let Some(seconds) = parse_iso_seconds(stamp) else {
        return String::new();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    let delta = (now - seconds).max(0);
    match delta {
        0..=59 => "now".to_string(),
        60..=3599 => format!("{}m", delta / 60),
        3600..=86_399 => format!("{}h", delta / 3600),
        _ => format!("{}d", delta / 86_400),
    }
}

fn parse_iso_seconds(stamp: &str) -> Option<i64> {
    if stamp.len() < 19 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| stamp.get(range).and_then(|part| part.parse::<i64>().ok());
    let year = num(0..4)?;
    let month = num(5..7)?;
    let day = num(8..10)?;
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let second = num(17..19)?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}
