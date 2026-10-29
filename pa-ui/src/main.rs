mod composer_canvas;
mod images;
mod live;
mod preview;
mod transcript;

slint::include_modules!();

use pa_client::{
    attach_as, clear_queue, create_session, get_queue, get_tail, list_sessions, model_catalog,
    clamp_level, get_commands, mutate_queue, send_prompt_checked, session_state, session_stats, set_model,
    set_thinking_level, steer_prompt, stop, thinking_level_label, Client, SessionRow,
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
    Prompt {
        session: String,
        message: String,
        images: Vec<serde_json::Value>,
    },
    Steer {
        session: String,
        message: String,
        images: Vec<serde_json::Value>,
    },
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
    /// An edit of a parked message: replaces it where it stands, so the queue
    /// keeps its order instead of sending the message to the back.
    QueueEdit {
        session: String,
        lane: String,
        slot: usize,
        expected: String,
        text: String,
        images: Vec<serde_json::Value>,
    },
    Stats { session: String },
    /// the session's own slash commands (templates and skills)
    Commands { session: String },
    SetModel { session: String, provider: String, model: String },
    /// the running model and the level it thinks at, straight from `get_state`
    SessionState { session: String },
    /// a level picked off a model's ladder: switching models first when the row
    /// was not the running one
    SetLevel { session: String, provider: String, model: String, level: String, switch: bool },
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
        /// the turn just closed: reconcile for real instead of appending
        fresh: bool,
    },
    /// one `session_event` from the attached session, forwarded verbatim
    Event { session: String, event: serde_json::Value },
    PromptResult { error: Option<String> },
    /// the daemon refused a prompt: hand the text back to the composer
    PromptRejected { text: String },
    /// the pipe came back: the old attachment is gone, re-attach
    Reconnected,
    Queue { steering: Vec<String>, follow_up: Vec<String> },
    /// the parked row went away before its edit landed: the text goes out as a
    /// new prompt rather than being lost
    Prompt { session: String, message: String, images: Vec<serde_json::Value> },
    /// the session's own slash commands, on top of the builtin table
    ExtraCommands(Vec<(String, String, String)>),
    /// the daemon's model catalog, as the menu builds it
    ModelCatalog(Vec<pa_client::ModelEntry>),
    QueueEdit { text: String },
    /// a `create` came back: the id of the brand new session
    NewChat(String),
    Stats { session: String, stats: pa_client::SessionStats },
    State { session: String, state: pa_client::SessionState },
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
                            thumb: slint::Image::default(),
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

    // A turn is running: set when a prompt is accepted, cleared when the turn
    // closes. The `busy` property lags behind that, and image sends are refused
    // on this flag rather than on what the last poll reported.
    let in_flight: Rc<std::cell::Cell<bool>> = Rc::new(std::cell::Cell::new(false));
    // the text of a prompt we drew optimistically, so the daemon's echo of the
    // same user message does not become a second row
    let pending_user: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
    // the last user message the daemon told us about, so its several frames
    // draw one row
    let mut drawn_user: Option<String> = None;
    // The parked row currently held in the composer, if any: (lane, slot,
    // expected text). Sending while this is set replaces that row instead of
    // appending a new message to the back of the queue.
    let editing: Rc<RefCell<Option<(String, usize, String)>>> = Rc::new(RefCell::new(None));
    // the session's own commands (templates and skills): the builtin table is
    // only half of what the agent accepts
    let extra_commands: Rc<RefCell<Vec<(String, String, String)>>> =
        Rc::new(RefCell::new(Vec::new()));
    // the composer's painted canvas: shaped once per change, not per frame
    let canvas_cell = Rc::new(RefCell::new(composer_canvas::Canvas::new(13.0, 18.0)));
    let canvas_key: Rc<RefCell<(String, usize, i32)>> =
        Rc::new(RefCell::new((String::new(), usize::MAX, i32::MAX)));
    // Pictures this window sent, by the text they went with: the queue
    // projection names text only, so this is the only source for a thumbnail on
    // a waiting row.
    let sent_thumbs: Rc<RefCell<Vec<(String, slint::Image)>>> = Rc::new(RefCell::new(Vec::new()));
    let attachments: Rc<RefCell<Vec<images::Attachment>>> = Rc::new(RefCell::new(Vec::new()));
    let attachment_model = Rc::new(VecModel::<slint::Image>::from(Vec::new()));
    ui.set_attachment_images(ModelRc::from(attachment_model.clone()));

    // the model catalog and which provider groups the user has folded away.
    // Collapse lives here, not in Slint: it has to survive closing the menu.
    let catalog: Rc<RefCell<Vec<pa_client::ModelEntry>>> =
        Rc::new(RefCell::new(Vec::new()));
    let collapsed: Rc<RefCell<std::collections::HashSet<String>>> =
        Rc::new(RefCell::new(std::collections::HashSet::new()));
    let catalog_loaded = Rc::new(std::cell::Cell::new(false));
    // the row whose effort ladder is open, so the pick knows its target
    let hovered: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));


    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let rows = rows_store.clone();
        let model = row_model.clone();
        let pending_user = pending_user.clone();
        let attachments = attachments.clone();
        let attachment_model = attachment_model.clone();
        let catalog_for_send = catalog.clone();
        let in_flight_for_send = in_flight.clone();
        let sent_thumbs = sent_thumbs.clone();
        let editing_to_send = editing.clone();
        ui.on_send_requested(move |text| {
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[send] callback fired with {text:?}");
            }
            let message = text.trim().to_string();
            if message.is_empty() && attachments.borrow().is_empty() {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            // Two ways an image send goes wrong, both refused here: a model that
            // cannot see images drops the whole turn, and an image sent into a
            // running turn arrives with the picture missing.
            if !attachments.borrow().is_empty() {
                let model = ui.get_model_id().to_string();
                let provider = ui.get_model_provider().to_string();
                let capable = catalog_for_send
                    .borrow()
                    .iter()
                    .find(|entry| entry.id == model && entry.provider == provider)
                    .is_none_or(|entry| entry.supports_images);
                let in_flight_now = ui.get_busy() || in_flight_for_send.get();
                if let Some(reason) = image_send_blocker(true, capable, in_flight_now) {
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[send] held: {reason}");
                    }
                    ui.set_status_line(reason.into());
                    return;
                }
            }
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                ui.set_status_line("no session selected".into());
                return;
            }

            // The word in front of the caret plus whatever was typed after it.
            let command_word = ui.get_composer_command().to_string();
            let message = if command_word.is_empty() {
                message
            } else if message.trim().is_empty() {
                command_word
            } else {
                format!("{command_word} {message}")
            };
            // A command is only sent when the agent runs it. Everything else in
            // the table is a window action: it either happens here or says
            // plainly that it is not wired, but it never leaves as chat words.
            match slash_dispatch(&message) {
                SlashAction::Send => {}
                SlashAction::Model => {
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_composer_command("".into());
                    ui.set_model_menu(true);
                    ui.set_status_line("pick a model".into());
                    return;
                }
                SlashAction::EffortHint => {
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_composer_command("".into());
                    ui.set_status_line("the thinking level opens from the model menu".into());
                    return;
                }
                SlashAction::NewChat => {
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_composer_command("".into());
                    let _ = jobs.send(Job::NewChat);
                    return;
                }
                SlashAction::Quit => {
                    let _ = slint::quit_event_loop();
                    return;
                }
                SlashAction::Rename(name) => {
                    if name.is_empty() {
                        ui.set_status_line("give the chat a name: /name <name>".into());
                        return;
                    }
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_composer_command("".into());
                    let _ = jobs.send(Job::Rename {
                        session,
                        name,
                    });
                    return;
                }
                SlashAction::NotWired(name) => {
                    // the text stays where it is: nothing is lost, and nothing
                    // pretends to have happened
                    ui.set_status_line(
                        format!("/{name} belongs to the terminal app and isn't wired here yet")
                            .into(),
                    );
                    return;
                }
            }
            ui.set_composer_text("".into());
            ui.set_composer_has_text(false);
            ui.set_composer_command("".into());
            {
                let held = attachments.borrow();
                if let Some(first) = held.first() {
                    if let Ok(thumb) = slint::Image::load_from_path(&first.path) {
                        let mut thumbs = sent_thumbs.borrow_mut();
                        thumbs.push((message.clone(), thumb));
                        while thumbs.len() > 8 {
                            thumbs.remove(0);
                        }
                    }
                }
            }
            let blocks: Vec<serde_json::Value> = {
                let mut held = attachments.borrow_mut();
                let blocks = held.iter().map(images::Attachment::block).collect();
                held.clear();
                blocks
            };
            while attachment_model.row_count() > 0 {
                attachment_model.remove(attachment_model.row_count() - 1);
            }
            // An edit of a parked message replaces it where it stands: taking it
            // out and sending it again put it at the back of the queue, which is
            // not what editing means.
            if let Some((lane, slot, expected)) = editing_to_send.borrow_mut().take() {
                ui.set_status_line("parked message updated".into());
                let _ = jobs.send(Job::QueueEdit {
                    session,
                    lane,
                    slot,
                    expected,
                    text: message,
                    images: blocks,
                });
                return;
            }
            // Where the message belongs depends on whether a turn owns the
            // transcript. While one runs, this is a follow-up: it waits in the
            // queue lanes and only becomes a row once it actually runs. Drawing
            // it in the transcript first was the flash - the bubble appeared,
            // the queue arrived, the row was swept away and everything shifted.
            // Nothing is drawn here. Tthe row appears when the daemon echoes the
            // message - the same way the TUI does it - so the transcript never
            // shows a message the daemon has not accepted, and never has to take
            // one back.
            *pending_user.borrow_mut() = Some(message.clone());
            // The turn is already starting; the poll that reports it is up to a
            // second away, and until then the button still offers to send, so a
            // second press queues instead of stopping. Busy starts here.
            if !ui.get_busy() {
                ui.set_busy_label("Waiting".into());
                ui.set_busy(true);
            }
            if let Some(ui) = weak.upgrade() {
                ui.set_status_line("sent".into());
            }
            in_flight_for_send.set(true);
            let _ = jobs.send(Job::Prompt { session, message, images: blocks });
        });
    }

    // Attachments: the bytes live here, the model holds the thumbnails. The
    // readers shell out, so they run off the event loop and come back through
    // `invoke_from_event_loop`.
    // The readers block (a dialog, a PowerShell round trip), so they run on their
    // own thread and drop their result into a channel the event loop drains: an
    // Rc<VecModel> cannot cross a thread boundary, a Vec can.
    let (attach_tx, attach_rx) = std::sync::mpsc::channel::<Vec<images::Attachment>>();
    {
        let tx_pick = attach_tx.clone();
        ui.on_attach_requested(move || {
            let tx = tx_pick.clone();
            std::thread::spawn(move || {
                let _ = tx.send(images::pick_files());
            });
        });
        let tx_paste = attach_tx.clone();
        ui.on_attach_pasted(move || {
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[attach] paste chord");
            }
            // the answer decides the key: true consumes it, false lets the text
            // input paste whatever text is on the clipboard
            if !images::clipboard_has_image() {
                return false;
            }
            let tx = tx_paste.clone();
            std::thread::spawn(move || {
                let _ = tx.send(images::read_clipboard().into_iter().collect());
            });
            true
        });
        let attachments = attachments.clone();
        let model = attachment_model.clone();
        ui.on_attachment_removed(move |index| {
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            let mut held = attachments.borrow_mut();
            if index < held.len() {
                held.remove(index);
                model.remove(index);
            }
        });
    }
    {
        let attachments = attachments.clone();
        let model = attachment_model.clone();
        let drain = Timer::default();
        drain.start(TimerMode::Repeated, std::time::Duration::from_millis(120), move || {
            while let Ok(found) = attach_rx.try_recv() {
                let mut held = attachments.borrow_mut();
                for attachment in found {
                    match slint::Image::load_from_path(&attachment.path) {
                        Ok(thumb) => {
                            if std::env::var("PA_DEBUG").is_ok() {
                                eprintln!(
                                    "[attach] {} ({} base64 chars) {}x{}",
                                    attachment.mime,
                                    attachment.base64.len(),
                                    thumb.size().width,
                                    thumb.size().height
                                );
                            }
                            held.push(attachment);
                            model.push(thumb);
                        }
                        // sendable without a preview; a blank tile is worse than none
                        Err(error) => eprintln!("[attach] no thumbnail: {error}"),
                    }
                }
            }
        });
        std::mem::forget(drain);
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
                session: session.clone(),
                provider: provider.to_string(),
                model: model.to_string(),
            });
            // the switch's own level lands on the next poll, not from here
            let _ = jobs.send(Job::SessionState { session });
        });
    }

    // Leaving a row closes its ladder. A short grace, not an instant: crossing
    // the 6px between the row and the ladder would otherwise close the thing
    // the pointer is on its way into.
    let hover_close = Rc::new(Timer::default());
    // which row's ladder was last built. Cleared when the ladder closes: the
    // hover repeats on every pointer move, and a remembered pair that outlives
    // the popup makes the same row read as "already open" - hover away to a
    // header and back and nothing came up until a different row was touched.
    let hover_current: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    {
        let weak = ui.as_weak();
        let timer = hover_close.clone();
        let current = hover_current.clone();
        ui.on_pointer_moved(move || {
            let Some(ui) = weak.upgrade() else { return };
            if !ui.get_effort_open() {
                return;
            }
            let weak = ui.as_weak();
            let current = current.clone();
            timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
                if let Some(ui) = weak.upgrade() {
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[menu] ladder closed");
                    }
                    *current.borrow_mut() = None;
                    ui.set_effort_open(false);
                }
            });
        });
    }
    {
        let timer = hover_close.clone();
        ui.on_effort_hovered(move || {
            timer.stop();
        });
    }

    {
        // A row's ladder hangs off the row. The hover repeats on every pointer
        // move, so the pair is remembered and only a change rebuilds the list.
        let current = hover_current.clone();
        let weak = ui.as_weak();
        let catalog = catalog.clone();
        let hovered = hovered.clone();
        let timer = hover_close.clone();
        ui.on_model_hovered(move |provider, id, row_y| {
            timer.stop();
            let Some(ui) = weak.upgrade() else { return };
            let pair = (provider.to_string(), id.to_string());
            if current.borrow().as_ref() == Some(&pair) {
                return;
            }
            let catalog = catalog.borrow();
            let Some(entry) = catalog
                .iter()
                .find(|entry| entry.provider == pair.0 && entry.id == pair.1)
            else {
                if std::env::var("PA_DEBUG").is_ok() {
                    eprintln!("[menu] hover {}/{} not in catalog", pair.0, pair.1);
                }
                return;
            };
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!(
                    "[menu] hover {}/{} levels={:?}",
                    pair.0, pair.1, entry.thinking_levels
                );
            }
            // nothing to choose on a model with no thinking surface
            if entry.thinking_levels.len() < 2 {
                ui.set_effort_open(false);
                *current.borrow_mut() = Some(pair);
                return;
            }
            // Switching keeps the level and clamps it to the new model's own
            // ladder, so the rung that would apply is the clamped one — the
            // submenu shows where this model would land, not a guess.
            let landed = clamp_level(&entry.thinking_levels, &ui.get_current_level());
            let items: Vec<EffortItem> = entry
                .thinking_levels
                .iter()
                .map(|candidate| EffortItem {
                    level: candidate.clone().into(),
                    label: transcript::plain(thinking_level_label(candidate)).into(),
                    current: *candidate == landed,
                })
                .collect();
            ui.set_effort_items(ModelRc::from(Rc::new(VecModel::from(items))));
            ui.set_effort_y(row_y);
            if !ui.get_effort_open() && std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[menu] ladder open at y={row_y}");
            }
            ui.set_effort_open(true);
            *hovered.borrow_mut() = Some(pair.clone());
            *current.borrow_mut() = Some(pair);
        });
    }

    {
        let weak = ui.as_weak();
        let jobs = job_tx.clone();
        let catalog = catalog.clone();
        let hovered = hovered.clone();
        ui.on_effort_picked(move |index| {
            let Some(ui) = weak.upgrade() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            let Some((provider, model)) = hovered.borrow().clone() else { return };
            let level = catalog
                .borrow()
                .iter()
                .find(|entry| entry.provider == provider && entry.id == model)
                .and_then(|entry| {
                    usize::try_from(index)
                        .ok()
                        .and_then(|index| entry.thinking_levels.get(index).cloned())
                });
            let Some(level) = level else { return };
            let switch = !(model == ui.get_model_id().to_string()
                && provider == ui.get_model_provider().to_string());
            ui.set_effort_open(false);
            *hover_current.borrow_mut() = None;
            ui.set_status_line(
                format!("{model} · {}", thinking_level_label(&level)).into(),
            );
            let _ = jobs.send(Job::SetLevel {
                session,
                provider,
                model,
                level,
                switch,
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
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!(
                    "[menu] opened box=({},{},{}) height={}",
                    ui.get_box_x(),
                    ui.get_box_y(),
                    ui.get_box_w(),
                    ui.get_model_menu_height()
                );
            }
            if !catalog_loaded.get() {
                let _ = jobs.send(Job::ModelCatalog {
                    session: session.clone(),
                });
            }
            // the level can move under us (the CLI, another window), so the
            // menu opens on a fresh reading rather than the last poll's
            let _ = jobs.send(Job::SessionState { session });
        });
    }

    {
        // The command menu. Everything it shows comes from pa-client's copy of the
    // daemon's table, so the window cannot offer a command the agent does not
    // have, and the descriptions are the daemon's own words.
    {
        let weak = ui.as_weak();
        ui.on_composer_command_cleared(move || {
            let Some(ui) = weak.upgrade() else { return };
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[composer] command word removed");
            }
            ui.set_composer_command("".into());
        });
    }

    {
        let weak = ui.as_weak();
        let extra_commands = extra_commands.clone();
        ui.on_slash_typed(move |text| {
            let Some(ui) = weak.upgrade() else { return };
            let Some(prefix) = slash_name_token(&text) else {
                ui.set_slash_open(false);
                return;
            };
            let rows = slash_rows(&prefix, &extra_commands.borrow());
            ui.set_slash_items(ModelRc::from(Rc::new(VecModel::from(rows))));
            ui.set_slash_index(0);
            ui.set_slash_open(true);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_slash_move(move |delta| {
            let Some(ui) = weak.upgrade() else { return };
            let count = ui.get_slash_items().row_count() as i32;
            if !ui.get_slash_open() || count == 0 {
                return;
            }
            let next = (ui.get_slash_index() + delta).rem_euclid(count);
            ui.set_slash_index(next);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_slash_dismiss(move || {
            if let Some(ui) = weak.upgrade() {
                ui.set_slash_open(false);
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_slash_accept(move || {
            let Some(ui) = weak.upgrade() else { return };
            let index = ui.get_slash_index().max(0) as usize;
            let rows = ui.get_slash_items();
            let Some(row) = rows.row_data(index) else {
                ui.set_slash_open(false);
                return;
            };
            ui.set_slash_open(false);
            let name = row.name.to_string();
            // The commands this window can carry out itself open their own
            // pickers instead of leaving a word in the box to press Enter on.
            if name == "model" {
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_model_menu(true);
                ui.set_status_line("pick a model".into());
                return;
            }
            // The command goes into the box like text, in gold, with the caret
            // placed after it: setting text alone leaves the caret inside the
            // half-typed word, which is what made it feel like broken
            // autocomplete.
            let command = format!("/{name} ");
            // the word goes in front of the caret; the box keeps the argument
            ui.set_composer_command(format!("/{name}").as_str().into());
            ui.set_composer_text("".into());
            ui.set_composer_has_text(false);
            ui.set_composer_caret_request(0);
        });
    }

    // folding a group is remembered for the rest of the session, so a menu
        // reopened later comes back exactly as it was left
        let weak = ui.as_weak();
        let catalog = catalog.clone();
        let collapsed = collapsed.clone();
        ui.on_provider_toggled(move |provider| {
            let Some(ui) = weak.upgrade() else { return };
            let provider = provider.to_string();
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("[menu] fold {provider}");
            }
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
            if action == "edit" {
                // The row stays where it is: it is being changed, not withdrawn.
                *editing.borrow_mut() = Some((lane.to_string(), slot.max(0) as usize, text.clone()));
                let _ = jobs.send(Job::Queue { session });
                // the row's text comes back to the composer, and the row stays
                // visible: it is being edited, not withdrawn
                ui.set_composer_text(text.as_str().into());
                ui.set_composer_has_text(!text.is_empty());
                ui.set_status_line("editing the parked message".into());
                return;
            }
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
        let attachments = attachments.clone();
        let attachment_model = attachment_model.clone();
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
            ui.set_composer_has_text(false);
            let blocks: Vec<serde_json::Value> = {
                let mut held = attachments.borrow_mut();
                let blocks = held.iter().map(images::Attachment::block).collect();
                held.clear();
                blocks
            };
            while attachment_model.row_count() > 0 {
                attachment_model.remove(attachment_model.row_count() - 1);
            }
            *pending_user.borrow_mut() = None;
            ui.set_status_line("steered".into());
            let _ = jobs.send(Job::Steer { session, message, images: blocks });
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
    // the message still being written: one range of rows, rebuilt whole
    let mut open_live: Option<live::Live> = None;
    // when the row's first delta landed: the update timestamps are what tell a
    // real stream from one paint at the end
    let mut stream_started = SystemTime::now();
    // when the stream last moved: a live row that has gone quiet is not proof
    // the turn is still writing, only that we stopped hearing about it
    let mut _stream_at = SystemTime::now();
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

        // The canvas is the only thing that draws the message, so it is repainted
        // whenever the words, the caret or the width change - and not otherwise.
        {
            let text_now = ui.get_composer_text().to_string();
            let caret_now = usize::try_from(ui.get_composer_caret().max(0)).unwrap_or(0);
            let width_now = ui.get_composer_width();
            let key = (text_now.clone(), caret_now, (width_now * 10.0) as i32);
            if *canvas_key.borrow() != key {
                *canvas_key.borrow_mut() = key;
                let known = extra_commands.borrow().clone();
                let (image, caret) = canvas_cell.borrow_mut().render(
                    &text_now,
                    caret_now,
                    width_now,
                    |name| is_command_word(&known, name),
                );
                if std::env::var("PA_DEBUG").is_ok() {
                    let gold = composer_canvas::spans(&text_now, |name| {
                        is_command_word(&extra_commands.borrow(), name)
                    })
                    .iter()
                    .filter(|span| span.command)
                    .count();
                    eprintln!(
                        "[canvas] {} chars, {gold} command run(s), w={width_now:.0} h={:.0} caret=({:.1},{:.1})",
                        text_now.chars().count(),
                        caret.content_height,
                        caret.x,
                        caret.y
                    );
                }
                ui.set_composer_canvas(image);
                ui.set_composer_caret_x(caret.x);
                ui.set_composer_caret_y(caret.y);
                ui.set_composer_caret_height(caret.height);
                ui.set_composer_canvas_height(caret.content_height);
            }
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
                                close_live(&mut open_live, &rows_store_ui, &row_model_ui);
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
                    // the session's own commands arrive with the session
                    let _ = jobs.send(Job::Commands {
                        session: session.clone(),
                    });
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
                    if std::env::var("PA_DEMO").as_deref() == Ok("markdown") {
                        built.extend(transcript::markdown_demo_rows());
                    }
                    // a prompt sent while this snapshot was in flight: the
                    // store predates it, so the row we drew has to survive the
                    // rebuild
                    dump_rows(&built);
                    let slint_rows: Vec<TRow> = built.iter().map(to_slint).collect();
                    let count = slint_rows.len();
                    *rows_store_ui.borrow_mut() = built;
                    row_model_ui.set_vec(slint_rows);
                    // which model is running, and at what effort: the chip and
                    // the menu's ladder both read from this
                    let _ = jobs.send(Job::SessionState {
                        session: session.clone(),
                    });
                    if std::env::var("PA_MENU").is_ok() {
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
                    fresh,
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
                    // A closed turn reconciles even when the page is the same
                    // one the busy path already folded in: that page was
                    // appended beside the streamed row, and the signature alone
                    // would keep both copies on screen for good.
                    if tail_sig == Some((cursor, newest)) && !fresh {
                        continue;
                    }
                    // a quiet live row is replaced by the store's copy
                    close_live(&mut open_live, &rows_store_ui, &row_model_ui);
                    tail_len = messages.len();
                    tail_sig = Some((cursor, newest));
                    let (mut built, skipped) = transcript::build(&messages, window_rows);
                    if built.is_empty() && !messages.is_empty() {
                        continue;
                    }
                    // While a turn runs, the store's copy of it is still growing:
                    // its rows change shape from poll to poll (a thinking row
                    // becomes a tool cell, a cell becomes a code block), and
                    // replacing the model on each one made the transcript jump up
                    // and down as it settled. So the live view is only ever
                    // APPENDED to while busy - nothing already on screen is
                    // rewritten - and the single reconcile happens at turn end.
                    // A turn is live if the daemon says so OR if a message is
                    // still being written into the view: the busy flag flickers
                    // between tool calls, and a rebuild taken on the flicker
                    // replaces rows the store has not persisted yet, which tears
                    // a dozen rows off the transcript and puts them back.
                    // A message that is still open owns its rows for as long as
                    // it is open: a rebuild that lands during a long tool call
                    // (no deltas, so the quiet timer expires) would otherwise
                    // replace rows the store has not written yet and tear the
                    // transcript down by however many rows that turn had.
                    // While this client is drawing the open turn — a live row,
                    // or cells keyed by call id — the store's copy of that same
                    // turn is not a second opinion worth having. Appending the
                    // rows the store had written so far is what put the previous
                    // answer back on screen for a few seconds after a send and
                    // then drew the same code block twice: `built` is a rebuilt,
                    // windowed list, so its index range stops lining up with the
                    // rows already shown the moment the store reshapes one row or
                    // the window slides. Nothing is appended now; the store only
                    // speaks when nothing is open, or when a turn closes.
                    let open = open_live.is_some();
                    if open && !fresh {
                        tail_len = messages.len();
                        tail_sig = Some((cursor, newest));
                        continue;
                    }
                    if open {
                        // a turn just closed: give whatever we drew a moment to
                        // find its way into the store, so the transcript never
                        // shrinks under the reader's eyes
                    }
                    // the same hold on every poll, or the prompt we drew
                    // vanishes on the first rebuild and the transcript jumps
                    if std::env::var("PA_DEMO").as_deref() == Ok("markdown") {
                        // the demo rides every rebuild, or the next poll wipes it
                        built.extend(transcript::markdown_demo_rows());
                    }
                    // Merged per message, not replaced wholesale: a message the
                    // store sends replaces its own rows, a message it does not
                    // mention is left exactly as drawn, and a message it has
                    // never sent is appended. Nothing here compares sizes, so a
                    // store that is one message behind cannot shrink or blank
                    // anything.
                    let mut store = rows_store_ui.borrow_mut();
                    merge_rows(&mut store, built, &row_model_ui);
                    let count = store.len();
                    drop(store);
                    dump_rows(&rows_store_ui.borrow());
                    trace_rows("tail rebuild", &rows_store_ui);
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
                        // The answer stays exactly where it is. Dropping it and
                        // rebuilding from a store that is still one message
                        // behind is what made it collapse to a line and then
                        // come back: the screen may not shrink on the strength
                        // of a store that has not caught up.
                        // The live rows stay, carrying the message they belong
                        // to: the store's copy of that message replaces them
                        // when it arrives, in place.
                        open_live = None;
                        _stream_at = SystemTime::now() - Duration::from_secs(60);
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
                        let Some(turn) = open_live.as_mut() else {
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
                        sync_live(&mut open_live, &rows_store_ui, &row_model_ui);
                        scroll_to_end_when_ready(&ui.as_weak());
                        _stream_at = SystemTime::now();
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
                        if drawn_user.as_deref() != Some(key.as_str()) {
                            drawn_user = Some(key);
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
                        // A message that opens takes a range of its own, after
                        // whatever the last one owned. The previous range has to
                        // be dropped first: two ranges for one answer is exactly
                        // the double that kept coming back.
                        close_live(&mut open_live, &rows_store_ui, &row_model_ui);
                        let start = rows_store_ui.borrow().len();
                        open_live = Some(live::Live::new(start, transcript::message_key(message)));
                        stream_started = SystemTime::now();
                        trace_rows("live row opened", &rows_store_ui);
                        continue;
                    }
                    let Some(turn) = open_live.as_mut() else {
                        // a delta with no open message is the tail of something
                        // already reconciled; the store has it
                        continue;
                    };
                    if let Some(delta) = delta_event.get("delta").and_then(serde_json::Value::as_str) {
                        // Only the prose belongs in the body. A tool call also
                        // streams deltas, and they are the call's JSON: feeding
                        // them in printed the code as text, then the real cell
                        // arrived and it looked like the code had moved.
                        if delta_kind.starts_with("text") {
                            turn.push_text(delta);
                        } else if delta_kind.starts_with("thinking") {
                            turn.push_thinking(delta);
                        }
                    }
                    // the frame's own copy of the message wins over the deltas
                    let whole = message_text(message);
                    if !whole.is_empty() {
                        turn.set_text(&whole);
                    }
                    sync_live(&mut open_live, &rows_store_ui, &row_model_ui);
                    scroll_to_end_when_ready(&ui.as_weak());
                    _stream_at = SystemTime::now();
                    if std::env::var("PA_DEBUG").is_ok() && kind == Some("message_update") {
                        eprintln!(
                            "[stream] +{:>5}ms rows {} ({} chars, {})",
                            stream_started.elapsed().map(|d| d.as_millis()).unwrap_or(0),
                            open_live.as_ref().map_or(0, |turn| turn.rows),
                            whole.chars().count(),
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
                        if let Some(prefix) = slash_name_token(&opening) {
                            let rows: Vec<SlashItem> = pa_client::commands::filter(&prefix)
                                .iter()
                                .map(|command| SlashItem {
                                    name: command.name.into(),
                                    hint: command.hint.into(),
                                    description: command.description.into(),
                                })
                                .collect();
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
                    if std::env::var("PA_DEBUG").is_ok() {
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
                    if std::env::var("PA_MENU").is_ok() && !ui.get_model_menu() {
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
                Msg::State { session, state } => {
                    if loaded.as_deref() != Some(session.as_str()) {
                        continue;
                    }
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
                    loaded = None;
                    close_live(&mut open_live, &rows_store_ui, &row_model_ui);
                    tail_sig = None;
                    last_seq = 0;
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
                    drop_pending_prompt(&rows_store_ui, &row_model_ui, &text);
                    ui.set_status_line("prompt not accepted — text restored".into());
                }
                Msg::PromptResult { error } => match error {
                    Some(error) => {
                        let text = pending_user.borrow_mut().take().unwrap_or_default();
                        drop_pending_prompt(&rows_store_ui, &row_model_ui, &text);
                        ui.set_status_line(format!("prompt failed: {error}").into());
                    }
                    None => ui.set_status_line("working".into()),
                },
                Msg::Error(error) => ui.set_status_line(format!("error: {error}").into()),
            }
        }
    });
    // A look at the command menu in a session of its own: the flag opens a fresh
    // chat first, so the draft in the user's own session is never touched.
    if let Ok(opening) = std::env::var("PA_SLASH") {
        let _ = job_tx.send(Job::NewChat);
    }

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

/// Write the built transcript to `PA_DUMP`, so a row that appears twice can be
/// counted instead of guessed at.
/// Merge a rebuild into the rows already on screen, message by message.
///
/// This is the TUI's model, transplanted: its chat holds one entry per message
/// and a rebuild replaces an entry it was given, appends one it has not seen, and
/// leaves the rest alone. A flat row list can do the same as long as every row
/// knows its message, which is what `Row.message` is for.
fn merge_rows(
    store: &mut Vec<transcript::Row>,
    incoming: Vec<transcript::Row>,
    model: &Rc<VecModel<TRow>>,
) {
    if store.is_empty() {
        let slint_rows: Vec<TRow> = incoming.iter().map(to_slint).collect();
        *store = incoming;
        model.set_vec(slint_rows);
        return;
    }
    // group both sides by message, keeping the order they arrived in
    let group = |rows: &[transcript::Row]| -> Vec<(String, usize, usize)> {
        let mut out: Vec<(String, usize, usize)> = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            match out.last_mut() {
                Some((key, _, end)) if *key == row.message => *end = index + 1,
                _ => out.push((row.message.clone(), index, index + 1)),
            }
        }
        out
    };
    for (key, start, end) in group(&incoming) {
        let fresh: Vec<transcript::Row> = incoming[start..end].to_vec();
        let existing = group(store);
        match existing.iter().find(|(open, _, _)| *open == key) {
            Some((_, at, until)) => {
                // replace that message's rows where they stand
                let at = (*at).min(store.len());
                let until = (*until).min(store.len());
                for index in (at..until).rev() {
                    store.remove(index);
                    model.remove(index);
                }
                for (offset, row) in fresh.iter().enumerate() {
                    store.insert(at + offset, row.clone());
                    model.insert(at + offset, to_slint(row));
                }
            }
            None => {
                let at = store.len();
                for (offset, row) in fresh.iter().enumerate() {
                    store.insert(at + offset, row.clone());
                    model.insert(at + offset, to_slint(row));
                }
            }
        }
    }
}

/// Why an image send cannot go out right now, if it cannot.
///
/// Two rules, both learned from the daemon's behaviour:
/// - a model that cannot take images means the turn is dropped outright;
/// - an image sent while a turn is in flight is folded into that running turn,
///   and the picture is lost on the way (text survives, images do not).
fn image_send_blocker(has_images: bool, model_capable: bool, turn_in_flight: bool) -> Option<&'static str> {
    if !has_images {
        return None;
    }
    if !model_capable {
        return Some("this model cannot take images - switch to a vision model with /model, then send");
    }
    // Mid-turn is allowed: the daemon parks the prompt in the follow-up lane and
    // delivers it, images and all, as the probe `queued_image_prompt_lands`
    // shows. Refusing here just made the send look broken.
    let _ = turn_in_flight;
    None
}

#[cfg(test)]
mod image_send_tests {
    use super::image_send_blocker;

    #[test]
    fn text_only_is_never_blocked() {
        assert_eq!(image_send_blocker(false, false, true), None);
    }

    #[test]
    fn images_with_a_blind_model_are_refused() {
        assert!(image_send_blocker(true, false, false).is_some());
        assert!(image_send_blocker(true, false, true).is_some());
    }

    #[test]
    fn images_mid_turn_queue_rather_than_refuse() {
        assert_eq!(image_send_blocker(true, true, true), None);
    }

    #[test]
    fn images_to_a_vision_model_between_turns_go_out() {
        assert_eq!(image_send_blocker(true, true, false), None);
    }
}

fn apply_queue(
    ui: &AppWindow,
    steering: &[String],
    follow_up: &[String],
    sent_thumbs: &Rc<RefCell<Vec<(String, slint::Image)>>>,
) {
    if std::env::var("PA_DEBUG").is_ok() {
        eprintln!(
            "[queue] projection steering={} follow_up={steering:?}->{follow_up:?}",
            steering.len()
        );
    }
                let total = steering.len() + follow_up.len();
                let first = steering
                    .first()
                    .or_else(|| follow_up.first())
                    .cloned()
                    .unwrap_or_default();
                ui.set_queued_count(total as i32);
                ui.set_queued_preview(first.into());
                // The panel is opened by the user, never by the queue: a
                // strip that keeps a popup open after the send is noise.
                // The count on the strip is what says something is waiting.
                // Each row keeps its lane and its slot in that lane, which is
                // what the daemon wants back when one is edited or promoted.
                let lane_items = |texts: &[String]| -> Vec<QueueItem> {
                    texts
                        .iter()
                        .enumerate()
                        .map(|(slot, text)| {
                            let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                            let cut: String = flat.chars().take(120).collect();
                            // the picture this text went with, if it was
                            // sent from this window
                            let thumb = sent_thumbs
                                .borrow()
                                .iter()
                                .rev()
                                .find(|(sent, _)| sent.trim() == text.trim())
                                .map(|(_, thumb)| thumb.clone());
                            QueueItem {
                                text: cut.into(),
                                raw: text.clone().into(),
                                slot: slot as i32,
                                thumb: thumb.unwrap_or_default(),
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

/// Whether a bare word, typed after a slash, names a command this session has.
///
/// Builtins come from the copied table; the session's own entries may carry a
/// `skill:` prefix while the typed word does not.
fn is_command_word(extra: &[(String, String, String)], name: &str) -> bool {
    if pa_client::commands::find(name).is_some() {
        return true;
    }
    extra.iter().any(|(known, _, _)| {
        let bare = known.rsplit(':').next().unwrap_or(known.as_str());
        known == name || bare == name
    })
}

/// The menu's rows: the builtin table first, then whatever this session adds on
/// top of it, both narrowed by the same prefix.
fn slash_rows(prefix: &str, extra: &[(String, String, String)]) -> Vec<SlashItem> {
    let needle = prefix.trim_start_matches('/').to_lowercase();
    let mut rows: Vec<SlashItem> = pa_client::commands::filter(prefix)
        .iter()
        .map(|command| SlashItem {
            name: command.name.into(),
            hint: command.hint.into(),
            description: command.description.into(),
        })
        .collect();
    for (name, description, hint) in extra {
        let bare = name.strip_prefix("skill:").unwrap_or(name);
        if !name.to_lowercase().starts_with(&needle) && !bare.to_lowercase().starts_with(&needle) {
            continue;
        }
        rows.push(SlashItem {
            name: name.as_str().into(),
            hint: hint.as_str().into(),
            description: description.as_str().into(),
        });
    }
    rows
}

/// The command name being typed, or `None` when the message is not a command
/// yet.
///
/// The menu belongs to the word right after the slash: once a space is typed the
/// user is into the arguments and the list goes away, the same rule the TUI uses
/// (only the first line, only before the first space).
fn slash_name_token(text: &str) -> Option<String> {
    let first = text.lines().next().unwrap_or("");
    let body = first.strip_prefix('/')?;
    if body.contains(char::is_whitespace) || body.contains('/') {
        return None;
    }
    Some(body.to_string())
}

/// What the window should do with a message that starts with a slash.
///
/// The daemon's table splits commands in two: the agent runs `compact`,
/// `refine`, `goal` and `autonomous` itself, so those are sent as written. Every
/// other builtin is a *window* action — sending `/system-prompt` down the pipe
/// gave the agent a chat message that read like a command and did nothing.
fn slash_dispatch(text: &str) -> SlashAction {
    let trimmed = text.trim();
    let Some(body) = trimmed.strip_prefix('/') else {
        return SlashAction::Send;
    };
    let name = body.split_whitespace().next().unwrap_or("");
    let rest = body[name.len()..].trim().to_string();
    let Some(command) = pa_client::commands::find(name) else {
        // not a builtin: skills and prompt templates are the agent's to run
        return SlashAction::Send;
    };
    if command.session_side {
        return SlashAction::Send;
    }
    match name {
        "model" => SlashAction::Model,
        "effort" | "thinking" => SlashAction::EffortHint,
        "new" | "clear" => SlashAction::NewChat,
        "quit" => SlashAction::Quit,
        "name" => SlashAction::Rename(rest),
        _ => SlashAction::NotWired(command.name),
    }
}

/// The decision above, as a value so the send path stays readable.
#[derive(Debug, PartialEq, Eq)]
enum SlashAction {
    Send,
    Model,
    EffortHint,
    NewChat,
    Quit,
    Rename(String),
    NotWired(&'static str),
}


fn dump_rows(rows: &[Row]) {
    let Ok(path) = std::env::var("PA_DUMP") else {
        return;
    };
    let dump: String = rows
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                row.kind,
                row.id,
                row.title.replace('\n', " "),
                row.meta.replace('\n', " "),
                row.elapsed.replace('\n', " "),
                row.body.replace('\n', " ").chars().take(90).collect::<String>()
            )
        })
        .collect();
    let _ = std::fs::write(path, dump);
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
    let effort = ui.get_current_effort().to_string();
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
                .map(|entry| {
                    let current = entry.id == current_id && provider == current_provider;
                    ModelItem {
                        id: entry.id.clone().into(),
                        label: transcript::plain(&model_label(&entry.name, &entry.id)).into(),
                        detail: context_hint(entry.context_window).into(),
                        // only the running model has a level to name
                        effort: if current { effort.clone() } else { String::new() }.into(),
                        current,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        out.push(ModelGroup {
            provider: provider.clone().into(),
            label: transcript::plain(&provider_label(&provider)).into(),
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

/// A worker that panics used to vanish without a word: no events, no replies,
/// no poll - a window that keeps painting a transcript nobody is updating. The
/// work happens behind a catch so a panic in one frame is loud.
fn worker(jobs: Receiver<Job>, out: Sender<Msg>, client_id: String) {
    let reporter = out.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        worker_inner(jobs, out, client_id);
    }));
    if outcome.is_err() {
        eprintln!("[worker] panicked - the window is now static");
        let _ = reporter.send(Msg::Error("engine stopped — reopen the window".into()));
    }
}

fn worker_inner(jobs: Receiver<Job>, out: Sender<Msg>, client_id: String) {
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
                Job::QueueEdit {
                    session,
                    lane,
                    slot,
                    expected,
                    text,
                    images,
                } => {
                    let mut mutation =
                        serde_json::json!({"type": "replace", "text": text.clone()});
                    if !images.is_empty() {
                        // the daemon keeps the parked attachments unless a new
                        // list is present
                        mutation["images"] = serde_json::Value::Array(images.clone());
                    }
                    let outcome = mutate_queue(active, &session, &lane, slot, &expected, mutation);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[queue] replace {lane}[{slot}] -> {outcome:?}");
                    }
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        let _ = out.send(Msg::Queue { steering, follow_up });
                    }
                    match outcome {
                        Ok(status) if status == "applied" => {}
                        // the row moved or went away under us: send it as a new
                        // message rather than lose what was typed
                        other => {
                            if std::env::var("PA_DEBUG").is_ok() {
                                eprintln!("[queue] replace rejected ({other:?}); sending as new");
                            }
                            let _ = out.send(Msg::Prompt { session, message: text, images });
                        }
                    }
                }
                Job::Commands { session } => {
                    let rows = get_commands(active, &session).unwrap_or_default();
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!("[commands] {} extra command(s)", rows.len());
                    }
                    let _ = out.send(Msg::ExtraCommands(rows));
                }
                Job::Stats { session } => {
                    if let Ok(stats) = session_stats(active, &session) {
                        let _ = out.send(Msg::Stats { session, stats });
                    }
                }
                Job::SessionState { session } => {
                    if let Ok(state) = session_state(active, &session) {
                        if std::env::var("PA_DEBUG").is_ok() {
                            eprintln!(
                                "[state] {}/{} at {}",
                                state.provider, state.model_id, state.thinking_level
                            );
                        }
                        let _ = out.send(Msg::State { session, state });
                    }
                }
                Job::SetLevel {
                    session,
                    provider,
                    model,
                    level,
                    switch,
                } => {
                    // a level belongs to a model, so the switch goes first: the
                    // daemon clamps the level to whatever model is running
                    let switched = if switch {
                        set_model(active, &session, &provider, &model)
                    } else {
                        Ok(String::new())
                    };
                    if let Err(error) = switched {
                        let _ = out.send(Msg::Activity {
                            label: format!("{provider}/{model}: {error}"),
                        });
                        continue;
                    }
                    match set_thinking_level(active, &session, &level) {
                        Ok(_) => {
                            if std::env::var("PA_DEBUG").is_ok() {
                                eprintln!("[model] {provider}/{model} effort -> {level}");
                            }
                            if let Ok(state) = session_state(active, &session) {
                                let _ = out.send(Msg::State { session, state });
                            }
                        }
                        Err(error) => {
                            let _ = out.send(Msg::Activity {
                                label: format!("effort {level}: {error}"),
                            });
                        }
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
                Job::Steer { session, message, images } => {
                    let outcome = steer_prompt(active, &session, &message, &images);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!(
                            "[steer] session={session} message={message:?} images={} outcome={outcome:?}",
                            images.len()
                        );
                    }
                    if let Err(error) = outcome {
                        let _ = out.send(Msg::PromptResult { error: Some(error.to_string()) });
                    }
                }
                Job::Prompt { session, message, images } => {
                    // A prompt names the chat it belongs to. If this connection
                    // is bound to another chat - or to none - bind first, then
                    // send: sending into a session the connection is not
                    // attached to is how a message can be accepted and then go
                    // nowhere. The TUI does the same rebind-and-send.
                    if attached.as_deref() != Some(session.as_str()) {
                        match attach_as(active, &session, Some(&client_id), resume.clone()) {
                            Ok(messages) => {
                                attached = Some(session.clone());
                                let _ = out.send(Msg::Transcript {
                                    session: session.clone(),
                                    messages,
                                });
                            }
                            Err(error) => {
                                if std::env::var("PA_DEBUG").is_ok() {
                                    eprintln!("[prompt] bind before send failed: {error}");
                                }
                                let _ = out.send(Msg::PromptRejected { text: message });
                                let _ = out.send(Msg::PromptResult {
                                    error: Some(format!(
                                        "the window could not reach this chat: {error}"
                                    )),
                                });
                                continue;
                            }
                        }
                    }
                    // The daemon's verdict, awaited: an accepted message and a
                    // refused one used to look identical from here, so a refusal
                    // was silence. This is what the TUI's submit does.
                    let outcome = send_prompt_checked(active, &session, &message, &images);
                    if std::env::var("PA_DEBUG").is_ok() {
                        eprintln!(
                            "[prompt] session={session} message={message:?} images={} outcome={outcome:?}",
                            images.len()
                        );
                    }
                    // Ask what the daemon actually queued. This is the only
                    // source for those lanes: the window may guess that a turn
                    // is running, but it may not guess that a message is waiting.
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        let _ = out.send(Msg::Queue { steering, follow_up });
                    }
                    match outcome {
                        Ok(()) => {
                            let _ = out.send(Msg::PromptResult { error: None });
                        }
                        Err(reason) => {
                            // The text goes back to the composer and the reason
                            // is said plainly, instead of the message vanishing.
                            let _ = out.send(Msg::PromptRejected { text: message });
                            let _ = out.send(Msg::PromptResult {
                                error: Some(reason),
                            });
                        }
                    }
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
                            fresh,
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
                        // chars, not bytes: slicing a String at byte 700 splits
                        // whatever multi-byte character sits there and panics the
                        // reader thread, which is the thread every event and every
                        // reply arrives on - the window goes deaf and silent
                        let text: String = frame.to_string().chars().take(700).collect();
                        eprintln!("[dump] {text}");
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
thread_local! {
    /// One timer for every follow request. Each call restarts it, so a burst of
    /// rebuilds produces a single scroll when they stop - not one scroll per
    /// rebuild, each landing on a half-measured row height and creeping the
    /// view down in visible steps.
    static FOLLOW_TIMER: std::cell::RefCell<Option<slint::Timer>> =
        const { std::cell::RefCell::new(None) };
}

/// The prompt we drew the moment it was sent, put back on top of a rebuild that
/// predates the daemon's echo. Without it the row disappears for a poll and
/// reappears, which reads as the transcript jumping under the pointer.
/// Pull the optimistic row back out when the daemon refuses the prompt: the
/// transcript must not keep showing a message that was never sent.
fn drop_pending_prompt(
    rows: &Rc<RefCell<Vec<transcript::Row>>>,
    model: &slint::VecModel<TRow>,
    text: &str,
) {
    if text.is_empty() {
        return;
    }
    let echo = serde_json::json!({
        "role": "user",
        "content": [{ "type": "text", "text": text }],
    });
    let Some(candidate) = transcript::build(std::slice::from_ref(&echo), 1).0.into_iter().next()
    else {
        return;
    };
    let mut store = rows.borrow_mut();
    let Some(last) = store.last() else { return };
    if last.kind != candidate.kind || last.body != candidate.body {
        return;
    }
    store.pop();
    let index = store.len();
    drop(store);
    model.remove(index);
}

/// Every change to the row model, logged in order: the shape of a send is a
/// sequence of appends, rebuilds and patches, and a jump is one of them landing
/// in the wrong order. Only compiled in at PA_DEBUG.
/// Walk the open message's range back to what it looks like now: the old rows
/// go, the new ones take their place. Called for every frame, because the range
/// is the message - there is no second copy to keep in step.
fn sync_live(
    live: &mut Option<live::Live>,
    store: &Rc<RefCell<Vec<transcript::Row>>>,
    model: &Rc<VecModel<TRow>>,
) {
    let Some(turn) = live.as_mut() else {
        return;
    };
    let rows = turn.rows();
    let mut rows_store = store.borrow_mut();
    let end = (turn.start + turn.rows).min(rows_store.len());
    let begin = turn.start.min(end);
    for index in (begin..end).rev() {
        rows_store.remove(index);
        model.remove(index);
    }
    for (offset, row) in rows.iter().enumerate() {
        let index = turn.start + offset;
        rows_store.insert(index, row.clone());
        model.insert(index, to_slint(row));
    }
    turn.rows = rows.len();
}

/// Drop the open range: the store speaks for that message from here on.
fn close_live(
    live: &mut Option<live::Live>,
    store: &Rc<RefCell<Vec<transcript::Row>>>,
    model: &Rc<VecModel<TRow>>,
) {
    let _ = &store;
    if let Some(turn) = live.take() {
        let mut rows_store = store.borrow_mut();
        let end = (turn.start + turn.rows).min(rows_store.len());
        let begin = turn.start.min(end);
        for index in (begin..end).rev() {
            rows_store.remove(index);
            model.remove(index);
        }
    }
}

fn trace_rows(what: &str, rows: &Rc<RefCell<Vec<transcript::Row>>>) {
    if std::env::var("PA_DEBUG").is_err() {
        return;
    }
    let rows = rows.borrow();
    let tail: Vec<String> = rows
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|row| {
            let body: String = row.body.chars().take(18).collect();
            format!("{}:{:?}", row.kind, body)
        })
        .collect();
    eprintln!("[rows] {what:24} n={:<4} tail={:?}", rows.len(), tail);
}

fn scroll_to_end_when_ready(ui: &slint::Weak<AppWindow>) {
    FOLLOW_TIMER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let timer = slot.get_or_insert_with(slint::Timer::default);
        let ui = ui.clone();
        // tall rows (a code block, a table) can settle a frame later than the
        // model change, so one short correction follows the first
        timer.start(slint::TimerMode::SingleShot, Duration::from_millis(55), move || {
            if let Some(ui) = ui.upgrade() {
                ui.invoke_scroll_to_end();
                let late = ui.as_weak();
                slint::Timer::single_shot(Duration::from_millis(180), move || {
                    if let Some(ui) = late.upgrade() {
                        ui.invoke_scroll_to_end();
                    }
                });
            }
        });
    });
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

/// Tool results arrive as a model-facing value: sometimes a string, sometimes
/// `{ output, ... }`, sometimes a list of content blocks. Take the text.
fn tool_output_text(result: Option<&serde_json::Value>) -> String {
    fn text_of(value: &serde_json::Value, depth: usize) -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(items) => {
                let parts: Vec<String> = items.iter().filter_map(|item| text_of(item, depth + 1)).collect();
                (!parts.is_empty()).then(|| parts.join("\n"))
            }
            serde_json::Value::Object(map) if depth < 4 => {
                for key in ["text", "output", "stdout", "content", "result", "message"] {
                    if let Some(inner) = map.get(key) {
                        if let Some(text) = text_of(inner, depth + 1) {
                            return Some(text);
                        }
                    }
                }
                Some(value.to_string())
            }
            _ => None,
        }
    }
    result
        .and_then(|value| text_of(value, 0))
        .unwrap_or_default()
}

fn to_slint(row: &Row) -> TRow {
    TRow {
        kind: row.kind,
        title: transcript::plain(&row.title).into(),
        meta: transcript::plain(&row.meta).into(),
        body: transcript::styled(&row.body),
        code: transcript::styled(&row.code),
        output: transcript::styled(&row.output),
        has_code: row.has_code,
        has_output: row.has_output,
        expanded: row.expanded,
        failed: row.failed,
        reason: transcript::plain(&row.reason).into(),
        image: images::from_wire(&row.image, &row.mime),
        elapsed: row.elapsed.clone().into(),
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

#[cfg(test)]
mod slash_menu_tests {
    use super::slash_name_token;

    #[test]
    fn the_menu_belongs_to_the_word_after_the_slash() {
        assert_eq!(slash_name_token("/"), Some(String::new()));
        assert_eq!(slash_name_token("/mo"), Some("mo".to_string()));
        // a space means the arguments have started: the list goes away
        assert_eq!(slash_name_token("/model "), None);
        assert_eq!(slash_name_token("/new --name x"), None);
        // prose is not a command
        assert_eq!(slash_name_token("hello"), None);
        assert_eq!(slash_name_token("a /model"), None);
        // a second line is the message, not the command
        assert_eq!(slash_name_token("/model\nsecond"), Some("model".to_string()));
        assert_eq!(slash_name_token("/a/b"), None);
    }

    #[test]
    fn the_agent_runs_its_own_commands() {
        use super::{slash_dispatch, SlashAction};
        // the four session commands go down the pipe as written
        assert_eq!(slash_dispatch("/compact"), SlashAction::Send);
        assert_eq!(slash_dispatch("/refine tighten the notes"), SlashAction::Send);
        // a skill is not a builtin: it is the agent's to run too
        assert_eq!(slash_dispatch("/skill:review the diff"), SlashAction::Send);
        assert_eq!(slash_dispatch("just prose"), SlashAction::Send);
    }

    #[test]
    fn window_commands_never_leave_as_words() {
        use super::{slash_dispatch, SlashAction};
        assert_eq!(slash_dispatch("/model"), SlashAction::Model);
        assert_eq!(slash_dispatch("/new"), SlashAction::NewChat);
        assert_eq!(slash_dispatch("/quit"), SlashAction::Quit);
        assert_eq!(
            slash_dispatch("/name plan the week"),
            SlashAction::Rename("plan the week".to_string())
        );
        // the one that was silently sent to the agent and did nothing
        assert_eq!(
            slash_dispatch("/system-prompt"),
            SlashAction::NotWired("system-prompt")
        );
    }
}

#[cfg(test)]
mod slash_rows_tests {
    use super::slash_rows;

    #[test]
    fn the_table_comes_first_and_the_sessions_own_follow() {
        let extra = vec![
            (
                "skill:review".to_string(),
                "review the diff".to_string(),
                "your request".to_string(),
            ),
            (
                "template:standup".to_string(),
                "daily standup".to_string(),
                "[day]".to_string(),
            ),
        ];
        let all = slash_rows("", &extra);
        assert!(all.len() == 43, "41 builtins + 2 session commands, got {}", all.len());
        assert_eq!(all.last().map(|row| row.name.to_string()), Some("template:standup".to_string()));
    }

    #[test]
    fn a_prefix_narrows_both_sources() {
        let extra = vec![(
            "skill:compact-review".to_string(),
            "x".to_string(),
            "your request".to_string(),
        )];
        let rows = slash_rows("compact", &extra);
        let names: Vec<String> = rows.iter().map(|row| row.name.to_string()).collect();
        assert!(names.contains(&"compact".to_string()));
        // the skill matches on its bare word too, so a typed `compact` finds it
        assert!(names.contains(&"skill:compact-review".to_string()));
    }

    #[test]
    fn compact_and_refine_are_in_the_menu() {
        let names: Vec<String> = slash_rows("", &[])
            .iter()
            .map(|row| row.name.to_string())
            .collect();
        assert!(names.contains(&"compact".to_string()));
        assert!(names.contains(&"refine".to_string()));
    }
}
