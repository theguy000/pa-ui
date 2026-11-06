// The window's own code. Every concern lives in a module beside this file; what
// is left here is the wiring script and the constants it shares.
mod composer;
mod composer_canvas;
mod env_cfg;
mod images;
mod live;
mod models;
mod preview;
#[cfg(test)]
mod probes;
mod search;
mod sessions;
mod share;
mod sheet;
mod slash;
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
/// The painter the callbacks install and the poll tick calls through.
pub(crate) type Repaint = Rc<dyn Fn(&AppWindow)>;

use pa_client::{
    attach_as, clamp_level, clear_queue, create_session, get_commands, get_queue, list_sessions,
    model_catalog, mutate_queue, send_prompt_checked, session_state, session_stats, set_model,
    set_thinking_level, steer_prompt, stop, thinking_level_label, Client, SessionRow,
};
use slint::{Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
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
    let worker_client_id = client_id.clone();
    std::thread::spawn(move || worker(job_rx, msg_tx, worker_client_id));

    // Four deep modules own the window's state and callbacks:
    let (sheet, transcript) = sheet::Sheet::new();
    let sheet = Rc::new(sheet);
    sheet.bind_ui(&ui);
    ui.set_transcript(transcript);
    if std::env::var("PA_HOVER").is_ok() {
        ui.set_ctx_hover(true);
    }

    let models = models::ModelMenu::new(&ui, &job_tx);
    let composer = composer::Composer::new(&ui, &job_tx, &models);
    let sidebar = sessions::Sidebar::new(&ui, &job_tx);

    let busy_since = Rc::new(std::cell::Cell::new(SystemTime::now()));
    let window_rows: usize = std::env::var("PA_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(WINDOW_ROWS);

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

    let tick = Rc::new(Tick {
        weak: ui.as_weak(),
        jobs: job_tx.clone(),
        msg_rx,
        attach_generation: std::cell::Cell::new(0),
        last_tick: std::cell::Cell::new(SystemTime::now()),
        sheet: sheet.clone(),
        sidebar: sidebar.clone(),
        composer: composer.clone(),
        models: models.clone(),
        busy_since: busy_since.clone(),
        window_rows,
        drawn_user: std::cell::Cell::new(None),
        loaded: std::cell::Cell::new(None),
        live_state: std::cell::Cell::new(String::new()),
        live_since: std::cell::Cell::new(SystemTime::now()),
        busy_label: std::cell::Cell::new(String::new()),
        command_label: std::cell::Cell::new(None),
        last_seq: std::cell::Cell::new(0),
        open_live: std::cell::Cell::new(None),
        stream_started: std::cell::Cell::new(SystemTime::now()),
        _stream_at: std::cell::Cell::new(SystemTime::now()),
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
    if std::env::var("PA_SLASH").is_ok() {
        let _ = job_tx.send(Job::NewChat);
    }

    std::mem::forget(poll);

    ui.set_canvas_mode(crate::env_cfg::cfg().canvas);
    let icon_timer = std::rc::Rc::new(slint::Timer::default());
    let icon_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
    {
        let timer = icon_timer.clone();
        let ticks = icon_ticks.clone();
        icon_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(50),
            move || {
                ticks.set(ticks.get() + 1);
                if (window_icon::apply() & titlebar::init()) || ticks.get() > 40 {
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
    ui.on_titlebar_close(titlebar::close);

    let bar_timer = std::rc::Rc::new(slint::Timer::default());
    {
        let ui_weak = ui.as_weak();
        let timer = bar_timer.clone();
        bar_timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(60),
            move || {
                if let Some(ui) = ui_weak.upgrade() {
                    ui.set_titlebar_maximized(ui.window().is_maximized());
                    ui.set_titlebar_max_hover(titlebar::cursor_over_maximize());
                } else {
                    timer.stop();
                }
            },
        );
    }

    if let Ok(path) = std::env::var("PA_SNAPSHOT") {
        let shot_ui = ui.as_weak();
        if let Ok(seed) = std::env::var("PA_SEARCH_SEED") {
            sidebar.seed(&ui, seed_rows(&seed));
        }
        if let Ok(query) = std::env::var("PA_SEARCH") {
            if !query.is_empty() {
                ui.set_search_query(query.as_str().into());
            }
            sessions::open_search(&ui, &sidebar.cache.borrow());
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

    ui.run()
}