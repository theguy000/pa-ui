// The window's own code. Every concern lives in a module beside this file; what
// is left here is the wiring script and the constants it shares.
mod composer_canvas;
mod env_cfg;
mod images;
mod interactive;
mod live;
mod models;
mod pager;
mod preview;
mod probes;
mod queue;
mod scroll;
mod sheet;
mod send;
mod sessions;
mod slash;
mod text;
mod tick;
mod transcript;
mod worker;

// Each module owns one concern and is re-exported here, so a call site does not
// have to know which module a helper ended up in.
pub(crate) use models::*;
pub(crate) use pager::*;
#[allow(unused_imports)]
pub(crate) use probes::*;
pub(crate) use queue::*;
pub(crate) use scroll::*;
pub(crate) use send::*;
pub(crate) use sessions::*;
pub(crate) use slash::*;
pub(crate) use text::*;
pub(crate) use tick::*;
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
    let (msg_tx, msg_rx) = channel::<(u64, Msg)>();
    let worker_client_id = client_id.clone();
    std::thread::spawn(move || worker(job_rx, msg_tx, worker_client_id));

    // transcript model, kept alive so a toggle can patch one row
    // the rail's filter and the rows it filters: a keystroke re-filters from
    // the cache instead of waiting for the next poll
    let session_filter: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let sessions_cache: Rc<RefCell<Vec<SessionRow>>> = Rc::new(RefCell::new(Vec::new()));
    // The sheet owns the rows and the list the window draws. Everything below
    // hands it rows; nothing else keeps the two in step.
    let (sheet, transcript) = sheet::Sheet::new();
    let sheet = Rc::new(sheet);
    ui.set_transcript(transcript);
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


    interactive::send::send(&ui, &attachment_model, &attachments, &catalog, &editing, &in_flight, &job_tx, &pending_user, &sent_thumbs);

    // Attachments: the bytes live here, the model holds the thumbnails. The
    // readers shell out, so they run off the event loop and come back through
    // `invoke_from_event_loop`.
    // The readers block (a dialog, a PowerShell round trip), so they run on their
    // own thread and drop their result into a channel the event loop drains: an
    // Rc<VecModel> cannot cross a thread boundary, a Vec can.
    let (attach_tx, attach_rx) = std::sync::mpsc::channel::<Vec<images::Attachment>>();
    let attach_rx = Rc::new(attach_rx);
    interactive::attachments::pick(&ui, &attach_tx, &attachment_model, &attachments);
    interactive::attachments::drain(&attach_rx, &attachment_model, &attachments);

    interactive::menu::picked(&ui, &catalog, &collapsed, &job_tx);

    // Leaving a row closes its ladder. A short grace, not an instant: crossing
    // the 6px between the row and the ladder would otherwise close the thing
    // the pointer is on its way into.
    let hover_close = Rc::new(Timer::default());
    // which row's ladder was last built. Cleared when the ladder closes: the
    // hover repeats on every pointer move, and a remembered pair that outlives
    // the popup makes the same row read as "already open" - hover away to a
    // header and back and nothing came up until a different row was touched.
    let hover_current: Rc<RefCell<Option<(String, String)>>> = Rc::new(RefCell::new(None));
    interactive::menu::pointer(&ui, &hover_close, &hover_current);
    interactive::menu::effort_hover(&ui, &hover_close);

    interactive::menu::hover(&ui, &catalog, &hover_close, &hover_current, &hovered);

    interactive::menu::effort_pick(&ui, &hover_current, &catalog, &hovered, &job_tx);

    interactive::menu::open(&ui, &catalog_loaded, &job_tx);

    // The command menu. Everything it shows comes from pa-client's copy of the
    // daemon's table, so the window cannot offer a command the agent does not
    // have, and the descriptions are the daemon's own words.
    interactive::slash::cleared(&ui);

    // Painting on the keystroke, not on the poll tick: a letter that waits for a
    // timer is a letter that arrives late, and holding a key shows it plainly.
    let repaint_canvas = interactive::canvas::painter(&canvas_cell, &canvas_key, &extra_commands, true);

    interactive::slash::typed(&ui, &extra_commands, &repaint_canvas, &slash_token);

    *repaint_slot.borrow_mut() = Some(repaint_canvas.clone());

    let repaint_canvas_for_accept =
        interactive::canvas::painter(&canvas_cell, &canvas_key, &extra_commands, false);

    interactive::slash::step(&ui);

    interactive::slash::dismiss(&ui);

    interactive::slash::accept(&ui, &repaint_canvas_for_accept, &slash_token);

    interactive::menu::provider_toggled(&ui, &catalog, &collapsed);

    interactive::queue::act(&ui, &editing, &job_tx);

    interactive::send::abort(&ui, &job_tx);

    interactive::send::steer(&ui, &attachment_model, &attachments, &job_tx, &pending_user);

    interactive::queue::clear(&ui, &job_tx);

    interactive::rail::search(&ui, &session_filter, &sessions_cache);

    interactive::rail::new_chat(&ui, &job_tx);

    interactive::rail::rename(&ui, &job_tx);

    interactive::rows::toggle(&ui, &sheet);

    let oldest_cursor: Rc<std::cell::RefCell<Option<String>>> =
        Rc::new(std::cell::RefCell::new(None));
    let has_older = Rc::new(std::cell::Cell::new(false));
    let loading_older = Rc::new(std::cell::Cell::new(false));

    // Walking backwards: the page before the one on screen, asked for when the
    // reader reaches the top. The geometry comes from the list (content-y,
    // content-height, height); the decision is a plain function, so it can be
    // tested without a window.
    let busy_since = Rc::new(std::cell::Cell::new(SystemTime::now()));
    // how many rows of history stay on the sheet above the newest page. Rows,
    // because a row is what the window pays to lay out - about 1.5 ms each.
    let history_rows: usize = std::env::var("PA_HISTORY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(600);
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

    let poll = Timer::default();

    let tick = Rc::new(Tick {
        weak: ui.as_weak(),
        jobs: job_tx.clone(),
        msg_rx,
        attach_generation: std::cell::Cell::new(0),
        tail_from: std::cell::RefCell::new(None),
        history_rows,
        last_tick: std::cell::Cell::new(SystemTime::now()),
        sheet: sheet.clone(),
        session_filter: session_filter.clone(),
        sessions_cache: sessions_cache.clone(),
        in_flight: in_flight.clone(),
        pending_user: pending_user.clone(),
        extra_commands: extra_commands.clone(),
        repaint_slot: repaint_slot.clone(),
        sent_thumbs: sent_thumbs.clone(),
        catalog: catalog.clone(),
        collapsed: collapsed.clone(),
        catalog_loaded: catalog_loaded.clone(),
        busy_since: busy_since.clone(),
        oldest_cursor: oldest_cursor.clone(),
        has_older: has_older.clone(),
        loading_older: loading_older.clone(),
        window_rows,
        drawn_user: std::cell::Cell::new(None),
        loaded: std::cell::Cell::new(None),
        live_state: std::cell::Cell::new(String::new()),
        live_since: std::cell::Cell::new(SystemTime::now()),
        busy_label: std::cell::Cell::new(String::new()),
        tail_len: std::cell::Cell::new(0),
        tail_sig: std::cell::Cell::new(None),
        last_seq: std::cell::Cell::new(0),
        open_live: std::cell::Cell::new(None),
        stream_started: std::cell::Cell::new(SystemTime::now()),
        _stream_at: std::cell::Cell::new(SystemTime::now()),
        selected_uuid: std::cell::Cell::new(None),
        tail_at: std::cell::Cell::new(SystemTime::now()),
        queue_at: std::cell::Cell::new(SystemTime::now()),
        stats_at: std::cell::Cell::new(SystemTime::now()),
        scroll_ticks: std::cell::Cell::new(0),
    });

    let poll = Timer::default();
    let poll_tick = tick.clone();
    poll.start(TimerMode::Repeated, Duration::from_millis(120), move || {
        poll_tick.run();
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
