// The window's own code. Every concern lives in a module beside this file; what
// is left here is the wiring script and the constants it shares.
mod composer_canvas;
mod env_cfg;
mod images;
mod live;
mod models;
mod pager;
mod preview;
mod probes;
mod queue;
mod rows;
mod scroll;
mod send;
mod sessions;
mod slash;
mod text;
mod transcript;
mod worker;

// Each module owns one concern and is re-exported here, so a call site does not
// have to know which module a helper ended up in.
pub(crate) use models::*;
pub(crate) use pager::*;
#[allow(unused_imports)]
pub(crate) use probes::*;
pub(crate) use queue::*;
pub(crate) use rows::*;
pub(crate) use scroll::*;
pub(crate) use send::*;
pub(crate) use sessions::*;
pub(crate) use slash::*;
pub(crate) use text::*;
pub(crate) use worker::*;

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

/// How much history one scroll-to-the-top pulls in. The TUI pages by the same
/// order of magnitude; a page costs the page, not the session.
const OLDER_PAGE: usize = 150;

/// The TUI's loader frames (pa-tui `LOADER_FRAMES`), advanced every poll tick.
const SPINNER_FRAMES: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];

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
    // the byte range of the `/command` token the caret is in, so taking a command
    // can replace exactly that token and leave the words around it alone
    let slash_token: Rc<RefCell<(usize, usize)>> = Rc::new(RefCell::new((0, 0)));
    // canvas mode: the composer paints its own words so a command can be gold
    // wherever it sits. Off unless asked for: a bad paint must never take the
    // composer away from the writer.
    let canvas_mode = crate::env_cfg::cfg().canvas;
    let canvas_cell: Rc<RefCell<Option<composer_canvas::Canvas>>> = Rc::new(RefCell::new(None));
    let canvas_key: Rc<RefCell<(String, usize, i32)>> =
        Rc::new(RefCell::new((String::new(), usize::MAX, i32::MIN)));
    // the painter is defined with the callbacks, but caret moves arrive on the poll
    // tick, which is set up earlier: one slot the tick can call through
    let repaint_slot: Rc<RefCell<Option<Rc<dyn Fn(&AppWindow)>>>> =
        Rc::new(RefCell::new(None));
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
            if crate::env_cfg::cfg().debug {
                eprintln!("[send] callback fired with {text:?}");
            }
            let message = text.trim().to_string();
            if message.is_empty() && attachments.borrow().is_empty() {
                return;
            }
            let Some(ui) = weak.upgrade() else { return };
            // Sending is the reader saying "show me the answer": follow again,
            // even if they had scrolled up to read something first
            force_scroll_to_end(&ui);
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
                    if crate::env_cfg::cfg().debug {
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
            if crate::env_cfg::cfg().debug {
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
                            if crate::env_cfg::cfg().debug {
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
                    if crate::env_cfg::cfg().debug {
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
                if crate::env_cfg::cfg().debug {
                    eprintln!("[menu] hover {}/{} not in catalog", pair.0, pair.1);
                }
                return;
            };
            if crate::env_cfg::cfg().debug {
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
            // ladder, so the rung that would apply is the clamped one � the
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
            if !ui.get_effort_open() && crate::env_cfg::cfg().debug {
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
                format!("{model} � {}", thinking_level_label(&level)).into(),
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
            if crate::env_cfg::cfg().debug {
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
            if crate::env_cfg::cfg().debug {
                eprintln!("[composer] command word removed");
            }
            ui.set_composer_command("".into());
        });
    }

    // Painting on the keystroke, not on the poll tick: a letter that waits for a
    // timer is a letter that arrives late, and holding a key shows it plainly.
    let repaint_canvas = {
        let canvas_cell = canvas_cell.clone();
        let canvas_key = canvas_key.clone();
        let extra_commands = extra_commands.clone();
        Rc::new(move |ui: &AppWindow| {
            if !ui.get_canvas_mode() {
                return;
            }
            let text_now = ui.get_composer_text().to_string();
            let caret_now = usize::try_from(ui.get_composer_caret().max(0)).unwrap_or(0);
            let anchor_now = usize::try_from(ui.get_composer_anchor().max(0)).unwrap_or(0);
            let width_now = ui.get_composer_width();
            let key = (
                text_now.clone(),
                caret_now * 100_000 + anchor_now,
                (width_now * 4.0) as i32,
            );
            if *canvas_key.borrow() == key {
                return;
            }
            *canvas_key.borrow_mut() = key;
            let known = extra_commands.borrow().clone();
            let mut slot = canvas_cell.borrow_mut();
            let canvas = slot.get_or_insert_with(|| composer_canvas::Canvas::new(13.0, 18.0));
            let (image, caret) = canvas.render(
                &text_now,
                caret_now,
                (anchor_now, caret_now),
                width_now,
                |name| is_command_word(&known, name),
            );
            if crate::env_cfg::cfg().debug {
                eprintln!(
                    "[canvas] {} chars, w={width_now:.0} h={:.0} caret=({:.1},{:.1})",
                    text_now.chars().count(),
                    caret.content_height,
                    caret.x,
                    caret.y
                );
            }
            ui.set_composer_canvas(image);
            ui.set_composer_canvas_height(caret.content_height);
            ui.set_composer_caret_x(caret.x);
            ui.set_composer_caret_y(caret.y);
            ui.set_composer_caret_height(caret.height);
        })
    };

    {
        let weak = ui.as_weak();
        let extra_commands = extra_commands.clone();
        let token = slash_token.clone();
        let paint = repaint_canvas.clone();
        ui.on_slash_typed(move |text, caret| {
            let Some(ui) = weak.upgrade() else { return };
            // the words paint in the same frame as the keystroke
            paint(&ui);
            let (start, end) = slash_token_at(&text, usize::try_from(caret.max(0)).unwrap_or(0));
            *token.borrow_mut() = (start, end);
            let body = text.get(start..end).unwrap_or_default();
            let Some(prefix) = body
                .strip_prefix('/')
                .filter(|name| !name.contains(char::is_whitespace) && !name.contains('/'))
            else {
                ui.set_slash_open(false);
                return;
            };
            let rows = slash_rows(prefix, &extra_commands.borrow());
            if rows.is_empty() {
                ui.set_slash_open(false);
                return;
            }
            ui.set_slash_items(ModelRc::from(Rc::new(VecModel::from(rows))));
            ui.set_slash_index(0);
            ui.set_slash_open(true);
        });
    }

    *repaint_slot.borrow_mut() = Some(repaint_canvas.clone());

    let repaint_canvas_for_accept = {
        let canvas_cell = canvas_cell.clone();
        let canvas_key = canvas_key.clone();
        let extra_commands = extra_commands.clone();
        Rc::new(move |ui: &AppWindow| {
            if !ui.get_canvas_mode() {
                return;
            }
            let text_now = ui.get_composer_text().to_string();
            let caret_now = usize::try_from(ui.get_composer_caret().max(0)).unwrap_or(0);
            let anchor_now = usize::try_from(ui.get_composer_anchor().max(0)).unwrap_or(0);
            let width_now = ui.get_composer_width();
            let key = (
                text_now.clone(),
                caret_now * 100_000 + anchor_now,
                (width_now * 4.0) as i32,
            );
            if *canvas_key.borrow() == key {
                return;
            }
            *canvas_key.borrow_mut() = key;
            let known = extra_commands.borrow().clone();
            let mut slot = canvas_cell.borrow_mut();
            let canvas = slot.get_or_insert_with(|| composer_canvas::Canvas::new(13.0, 18.0));
            let (image, caret) = canvas.render(
                &text_now,
                caret_now,
                (anchor_now, caret_now),
                width_now,
                |name| is_command_word(&known, name),
            );
            ui.set_composer_canvas(image);
            ui.set_composer_canvas_height(caret.content_height);
            ui.set_composer_caret_x(caret.x);
            ui.set_composer_caret_y(caret.y);
            ui.set_composer_caret_height(caret.height);
        })
    };

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
        let token = slash_token.clone();
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
            let text = ui.get_composer_text().to_string();
            let (start, end) = *token.borrow();
            let (start, end) = (start.min(text.len()), end.min(text.len()));
            // In canvas mode the command is always part of the text: the painting
            // colours it wherever it sits, so there is no outside slot to keep.
            if !ui.get_canvas_mode() && start == 0 && end == text.len() {
                // The whole box is the command: it becomes the gold word in
                // front of the caret, and the box is left empty for arguments.
                ui.set_composer_command(format!("/{name}").as_str().into());
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_caret_request(0);
                return;
            }
            // Started between words: replace that token where it stands and leave
            // the rest of the sentence exactly as it was.
            let mut next = String::with_capacity(text.len() + name.len() + 2);
            next.push_str(&text[..start]);
            next.push('/');
            next.push_str(&name);
            next.push(' ');
            next.push_str(&text[end..]);
            ui.set_composer_text(next.as_str().into());
            ui.set_composer_has_text(true);
            ui.set_composer_caret_request(i32::try_from(start + name.len() + 2).unwrap_or(0));
            repaint_canvas_for_accept(&ui);
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
            if crate::env_cfg::cfg().debug {
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
            if crate::env_cfg::cfg().debug {
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
    // backward pagination: the row id the next older page resumes from, whether
    // the store has one, and whether a page is already in flight
    let oldest_cursor: Rc<std::cell::RefCell<Option<String>>> =
        Rc::new(std::cell::RefCell::new(None));
    let has_older = Rc::new(std::cell::Cell::new(false));
    let loading_older = Rc::new(std::cell::Cell::new(false));

    // Walking backwards: the page before the one on screen, asked for when the
    // reader reaches the top. The geometry comes from the list (content-y,
    // content-height, height); the decision is a plain function, so it can be
    // tested without a window.
    let request_older = {
        let jobs = job_tx.clone();
        let oldest_cursor = oldest_cursor.clone();
        let has_older = has_older.clone();
        let loading_older = loading_older.clone();
        Rc::new(move |ui: &AppWindow| {
            let state = PagerState {
                follows_bottom: ui.get_follow_bottom(),
                ready: ui.get_pager_ready(),
                loading: loading_older.get(),
                has_older: has_older.get(),
                content_y: ui.get_dbg_cy() as f32,
                content_height: ui.get_dbg_ch() as f32,
                height: ui.get_dbg_h() as f32,
            };
            if !should_load_older(state) {
                return;
            }
            let Some(before) = oldest_cursor.borrow().clone() else { return };
            let session = ui.get_selected_id().to_string();
            if session.is_empty() {
                return;
            }
            loading_older.set(true);
            ui.set_loading_older(true);
            if crate::env_cfg::cfg().debug {
                eprintln!("[older] asking for the page before {before}");
            }
            let _ = jobs.send(Job::LoadOlder {
                session,
                before,
                limit: OLDER_PAGE,
            });
        })
    };

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
    // the spinner runs on its own clock � 10 frames at 80 ms is one 0.8 s turn,
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

    let mut scroll_ticks: u64 = 0;
    let poll = Timer::default();
    poll.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        let Some(ui) = weak.upgrade() else { return };

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
            scroll_ticks += 1;
            if scroll_ticks % 15 == 0 {
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
            && !crate::env_cfg::cfg().queue_preset
            && queue_at.elapsed().unwrap_or_default() >= Duration::from_millis(1500)
        {
            queue_at = SystemTime::now();
            if let Some(session) = loaded.clone() {
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
            // a different chat starts its own walk backwards
            *oldest_cursor.borrow_mut() = None;
            has_older.set(false);
            loading_older.set(false);
            ui.set_has_older(false);
            ui.set_loading_older(false);
            ui.set_pager_ready(false);
            ui.set_status_line("loading transcript".into());
            let _ = jobs.send(Job::Attach { session: selected });
        }

        // reaching the top pulls in the page before this one
        request_older(&ui);

        // caret and selection moves arrive here (arrow keys, clicks); keystrokes
        // paint themselves
        if let Some(paint) = repaint_slot.borrow().as_ref() {
            paint(&ui);
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
                                if crate::env_cfg::cfg().debug {
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
                            // the run just ended � grab whatever landed after the
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
                    // the walk backwards starts at the oldest row of the page
                    // being shown; a fresh page re-anchors it
                    if !first_entry.is_empty() {
                        *oldest_cursor.borrow_mut() = Some(first_entry.clone());
                    }
                    has_older.set(more);
                    ui.set_has_older(more);
                    if open_live.is_some() && !fresh {
                        tail_len = messages.len();
                        tail_sig = Some((cursor, newest));
                        continue;
                    }
                    tail_len = messages.len();
                    tail_sig = Some((cursor, newest));
                    let (mut built, skipped) = transcript::build(&messages, window_rows);
                    if built.is_empty() && !messages.is_empty() {
                        continue;
                    }
                    if crate::env_cfg::cfg().demo_markdown {
                        built.extend(transcript::markdown_demo_rows());
                    }
                    let live_in_built = open_live.as_ref().is_some_and(|turn| {
                        built.iter().any(|row| row.message == turn.message)
                    });
                    if live_in_built || (!ui.get_busy() && !in_flight.get()) {
                        close_live(&mut open_live, &rows_store_ui, &row_model_ui);
                    } else if open_live.is_some() {
                        let saved = open_live.take();
                        if let Some(ref turn) = saved {
                            let mut temp = Some(live::Live { message: String::new(), start: turn.start, rows: turn.rows, parts: Vec::new() });
                            close_live(&mut temp, &rows_store_ui, &row_model_ui);
                        }
                        let mut store = rows_store_ui.borrow_mut();
                        merge_rows(&mut store, built, &row_model_ui);
                        let new_start = store.len();
                        drop(store);
                        if let Some(mut turn) = saved {
                            turn.start = new_start;
                            turn.rows = 0;
                            open_live = Some(turn);
                            sync_live(&mut open_live, &rows_store_ui, &row_model_ui);
                        }
                        if follow {
                            scroll_to_end_when_ready(&ui.as_weak());
                        }
                        continue;
                    }
                    let mut store = rows_store_ui.borrow_mut();
                    merge_rows(&mut store, built, &row_model_ui);
                    let count = store.len();
                    drop(store);
                    if crate::env_cfg::cfg().dump.is_some() {
                        dump_rows(&rows_store_ui.borrow());
                    }
                    trace_rows("tail rebuild", &rows_store_ui);
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
                    session,
                    messages,
                    first_entry,
                    more,
                } => {
                    loading_older.set(false);
                    ui.set_loading_older(false);
                    if loaded.as_deref() != Some(session.as_str()) || messages.is_empty() {
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
                    prepend_rows(&mut rows_store_ui.borrow_mut(), &older, &row_model_ui);
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
                            older.len(),
                            oldest_cursor.borrow()
                        );
                    }
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
                    // (daemon restart, a stalled pipe) � backfill from the store
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
                        let held = hold_offset(&ui);
                        sync_live(&mut open_live, &rows_store_ui, &row_model_ui);
                        pin_offset_across_layout(&ui, held);
                        scroll_if_parked(&ui);
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
                        let key = transcript::message_key(message);
                        if open_live.as_ref().is_some_and(|turn| turn.message == key) {
                            continue;
                        }
                        open_live = None;
                        let start = rows_store_ui.borrow().len();
                        open_live = Some(live::Live::new(start, key));
                        stream_started = SystemTime::now();
                        trace_rows("live row opened", &rows_store_ui);
                        continue;
                    }
                    if open_live.is_none() && kind == Some("message_update") {
                        let key = transcript::message_key(message);
                        let start = rows_store_ui.borrow().len();
                        open_live = Some(live::Live::new(start, key));
                        stream_started = SystemTime::now();
                    }
                    let Some(turn) = open_live.as_mut() else {
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
                    sync_live(&mut open_live, &rows_store_ui, &row_model_ui);
                    pin_offset_across_layout(&ui, held);
                    scroll_if_parked(&ui);
                    _stream_at = SystemTime::now();
                    if crate::env_cfg::cfg().debug && kind == Some("message_update") {
                        eprintln!(
                            "[stream] +{:>5}ms rows {} ({} chars, {})",
                            stream_started.elapsed().map(|d| d.as_millis()).unwrap_or(0),
                            open_live.as_ref().map_or(0, |turn| turn.rows),
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
                    ui.set_status_line("prompt not accepted � text restored".into());
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

    ui.set_canvas_mode(canvas_mode);
    ui.run()
}
