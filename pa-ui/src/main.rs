// The window's own code. Every concern lives in a module beside this file; what
// is left here is the wiring script and the constants it shares.
mod composer;
mod composer_canvas;
mod env_cfg;
mod images;
mod live;
mod models;
mod preview;
mod quit;
#[cfg(test)]
mod probes;
mod search;
mod sessions;
mod settings;
mod share;
mod sheet;
mod slash;
mod command_word;
mod text;
mod theme;
mod tick;
mod titlebar;
mod transcript;
mod window_icon;
mod worker;

// Each module owns one concern and is re-exported here, so a call site does not
// have to know which module a helper ended up in.
pub(crate) use composer::*;
pub(crate) use models::*;
pub(crate) use sessions::*;
pub(crate) use sheet::*;
pub(crate) use slash::*;
pub(crate) use command_word::*;
pub(crate) use text::*;
pub(crate) use tick::*;
pub(crate) use worker::*;

slint::include_modules!();

/// The session's own slash commands: (name, description, source).
pub(crate) type SlashCommands = Rc<RefCell<Vec<(String, String, String)>>>;
/// The queued row being edited: (lane, slot, text).
pub(crate) type Editing = Rc<RefCell<Option<(String, usize, String)>>>;
/// Pictures this window sent, keyed by the text that went with them.
pub(crate) type SentThumbs = Rc<RefCell<Vec<(String, slint::Image)>>>;
/// The painter the callbacks install and the runtime repaints through.
pub(crate) type Repaint = Rc<dyn Fn(&AppWindow)>;

use pa_client::{
    attach_as, clamp_level, clear_queue, create_session_with, get_commands, get_queue, list_sessions,
    model_catalog, mutate_queue, send_prompt_checked, session_state, session_stats, set_model,
    set_thinking_level, steer_prompt, stop, thinking_level_label, Client, SessionRow,
};
use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use crossbeam_channel::{unbounded as channel, Receiver, Sender};
use std::time::{Duration, SystemTime};
use transcript::Row;

const WINDOW_ROWS: usize = 300;
const OLDER_PAGE: usize = 80;

const SPINNER_FRAMES: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];

fn seed_rows(seed: &str) -> Vec<SessionRow> {
    seed.split('|')
        .map(|name| SessionRow {
            id: name.to_string(),
            session_id: name.to_string(),
            name: name.to_string(),
            last_activity: "2026-11-03T00:00:00Z".to_string(),
            ..SessionRow::default()
        })
        .collect()
}

fn start_spinner(
    timer: Rc<Timer>,
    weak: slint::Weak<AppWindow>,
    index: Rc<std::cell::Cell<usize>>,
    since: Rc<std::cell::Cell<SystemTime>>,
    secs: Rc<std::cell::Cell<u64>>,
    running: Rc<std::cell::Cell<bool>>,
) {
    let tick_timer = Rc::downgrade(&timer);
    timer.start(TimerMode::Repeated, Duration::from_millis(80), move || {
        let Some(timer) = tick_timer.upgrade() else { return };
        let Some(ui) = weak.upgrade() else {
            timer.stop();
            running.set(false);
            return;
        };
        if !ui.get_busy() {
            timer.stop();
            running.set(false);
            return;
        }
        index.set(index.get().wrapping_add(1));
        ui.set_spinner_frame(SPINNER_FRAMES[index.get() % SPINNER_FRAMES.len()].into());
        let elapsed = since.get().elapsed().unwrap_or_default().as_secs();
        if elapsed != secs.get() {
            secs.set(elapsed);
            ui.set_busy_elapsed(format!("{elapsed}s").into());
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    if let Some((code, report)) = theme::check_cli(std::env::args().skip(1)) {
        print!("{report}");
        std::process::exit(code);
    }

    let ui = AppWindow::new()?;
    ui.window().set_size(slint::PhysicalSize::new(1200, 830));

    let (resolved, refusal) = theme::select(crate::env_cfg::cfg().theme.as_deref());
    if let Some(refusal) = refusal {
        eprintln!("[theme] {refusal}");
    }
    theme::apply(&ui, &resolved);

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
            ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::from(build(1, mid)))));
            ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::from(if count > mid {
                build(mid + 1, count)
            } else {
                Vec::new()
            }))));
        }
    }

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
    let (wake_tx, wake_rx) = channel::<()>();
    let worker_client_id = client_id.clone();
    std::thread::spawn(move || worker(job_rx, msg_tx, wake_tx, worker_client_id));

    // Four deep modules own the window's state and callbacks:
    let (sheet, transcript) = sheet::Sheet::new();
    let sheet = Rc::new(sheet);
    sheet.bind_ui(&ui);
    ui.set_transcript(transcript);
    if std::env::var("PA_HOVER").is_ok() {
        ui.set_ctx_hover(true);
    }

    let models = models::ModelMenu::new(&ui, &job_tx);
    let command_word = CommandWord::new();
    let composer = composer::Composer::new(&ui, &job_tx, &models, command_word.clone());
    let sidebar = sessions::Sidebar::new(&ui, &job_tx);

    let busy_since = Rc::new(std::cell::Cell::new(SystemTime::now()));
    let window_rows: usize = std::env::var("PA_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(WINDOW_ROWS);

    let spin = Rc::new(Timer::default());
    let spin_weak = ui.as_weak();
    let spin_index = Rc::new(std::cell::Cell::new(0usize));
    let spin_since = busy_since.clone();
    let spin_secs = Rc::new(std::cell::Cell::new(u64::MAX));
    let spin_running = Rc::new(std::cell::Cell::new(false));
    {
        let timer = spin.clone();
        let weak = spin_weak.clone();
        let index = spin_index.clone();
        let since = spin_since.clone();
        let secs = spin_secs.clone();
        let running = spin_running.clone();
        ui.on_busy_changed(move |busy| {
            if busy {
                if !running.get() {
                    running.set(true);
                    secs.set(u64::MAX);
                    start_spinner(
                        timer.clone(),
                        weak.clone(),
                        index.clone(),
                        since.clone(),
                        secs.clone(),
                        running.clone(),
                    );
                }
            } else {
                timer.stop();
                running.set(false);
            }
        });
    }

    let tick = Rc::new(Tick {
        weak: ui.as_weak(),
        jobs: job_tx.clone(),
        msg_rx,
        attach_generation: std::cell::Cell::new(0),
        sheet: sheet.clone(),
        sidebar: sidebar.clone(),
        composer: composer.clone(),
        command_word: command_word.clone(),
        models: models.clone(),
        busy_since: busy_since.clone(),
        window_rows,
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
    });

    // A sidebar pick changes the selection synchronously, but the transcript
    // belongs to the previous Session until the next tick. Run the tick from
    // the pick so the old sheet does not wait for an unrelated event.
    let pick_ui = ui.as_weak();
    let pick_tick = tick.clone();
    ui.on_selection_picked(move |_| {
        if let Some(ui) = pick_ui.upgrade() {
            ui.set_draft_chat(false);
        }
        pick_tick.run();
    });

    let new_tick = tick.clone();
    ui.on_new_chat(move || new_tick.open_new_chat());

    let wake_tick = tick.clone();
    ui.on_wake(move || wake_tick.run());
    let paging_tick = tick.clone();
    ui.on_scroll_paging(move || {
        if let Some(ui) = paging_tick.weak.upgrade() {
            paging_tick.request_older(&ui);
        }
    });
    // Worker messages carry a wake token; one hop moves the batch onto the
    // Slint thread. Coalesce tokens queued while the UI was busy.
    let wake_ui = ui.as_weak();
    std::thread::spawn(move || {
        while wake_rx.recv().is_ok() {
            while wake_rx.try_recv().is_ok() {}
            let weak = wake_ui.clone();
            if slint::invoke_from_event_loop(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.invoke_wake();
                }
            })
            .is_err()
            {
                break;
            }
        }
    });
    if std::env::var("PA_SLASH").is_ok() {
        tick.open_new_chat();
    }

    ui.set_canvas_mode(crate::env_cfg::cfg().canvas);
    let icon_timer = std::rc::Rc::new(slint::Timer::default());
    let icon_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
    {
        let timer = icon_timer.clone();
        let ticks = icon_ticks.clone();
        let icon_ui = ui.as_weak();
        icon_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                ticks.set(ticks.get() + 1);
                if (window_icon::apply() & titlebar::init(icon_ui.clone())) || ticks.get() > 40 {
                    timer.stop();
                }
            },
        );
    }

    ui.on_titlebar_minimize({
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.window().set_minimized(true);
            }
        }
    });
    quit::bind(&ui, &tick);

    if let Ok(path) = std::env::var("PA_SNAPSHOT") {
        let shot_ui = ui.as_weak();
        if let Ok(seed) = std::env::var("PA_SEARCH_SEED") {
            sidebar.seed(&ui, seed_rows(&seed));
        }
        if let Ok(query) = std::env::var("PA_SEARCH") {
            if !query.is_empty() {
                ui.set_search_query(query.as_str().into());
            }
            sidebar.open_search(&ui);
        }
        if let Ok(url) = std::env::var("PA_SHARE") {
            ui.set_share_url(url.as_str().into());
            ui.set_share_open(true);
        }
        if let Ok(error) = std::env::var("PA_SHARE_ERROR") {
            ui.set_share_error(error.as_str().into());
            ui.set_share_open(true);
        }
        if std::env::var("PA_SHARE_PENDING").is_ok() {
            ui.set_share_pending(true);
            ui.set_share_open(true);
        }
        if let Ok(error) = std::env::var("PA_SHARE_ACTION_ERROR") {
            ui.set_share_action_error(error.as_str().into());
            ui.set_share_open(true);
        }
        let shot_timer = std::rc::Rc::new(slint::Timer::default());
        let stop = shot_timer.clone();
        shot_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(600),
            move || {
                stop.stop();
                if let Some(ui) = shot_ui.upgrade() {
                    if let Ok(buffer) = ui.window().take_snapshot() {
                        let mut bytes = Vec::new();
                        bytes.extend_from_slice(&buffer.width().to_le_bytes());
                        bytes.extend_from_slice(&buffer.height().to_le_bytes());
                        for pixel in buffer.as_slice() {
                            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b, pixel.a]);
                        }
                        let _ = std::fs::write(&path, bytes);
                    }
                }
            },
        );
    }

    tick.run();
    ui.run()
}
