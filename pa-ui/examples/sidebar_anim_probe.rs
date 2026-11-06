//! Sidebar fold performance probe.
//!
//! Folds the Done group repeatedly and lets Slint's own metrics report the
//! frame rate and layer count. Run it with:
//!
//! ```text
//! PROBE_ROWS=150 PROBE_SECONDS=8 PROBE_TOGGLE_MS=300 \
//! SLINT_DEBUG_PERFORMANCE=refresh_lazy,console \
//! cargo run --profile fast -p pa-ui --example sidebar_anim_probe
//! ```
//!
//! The 150-row case used to report 1-3 fps and 150-304 layers created, one
//! opacity layer per row. The fold now reports ~130 fps and zero layers.
slint::include_modules!();

use slint::{ModelRc, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

const TOP: f32 = 88.0;
const ROW: f32 = 32.0;

fn entries(count: usize, done_folded: bool) -> Vec<SessionEntry> {
    let mut out = Vec::with_capacity(count + 3);
    let mut at = TOP;
    for (index, label, rows, folded) in [
        (0, "Active", 0usize, false),
        (1, "Idle", 0usize, false),
        (2, "Done", count, done_folded),
    ] {
        let header_at = at;
        out.push(SessionEntry {
            is_header: true,
            label: label.into(),
            count: rows as i32,
            folded,
            item: SessionItem::default(),
            at: header_at,
            target: header_at,
            seam: index,
        });
        at += ROW;
        for number in 0..rows {
            let target = header_at + ROW + number as f32 * ROW;
            out.push(SessionEntry {
                is_header: false,
                label: "".into(),
                count: 0,
                folded,
                item: SessionItem {
                    id: format!("id-{number}").into(),
                    uuid: format!("uuid-{number}").into(),
                    name: format!("Done session {number}").into(),
                    full: format!("Done session {number}").into(),
                    tip: format!("Done session {number}").into(),
                    status: "done".into(),
                    done: true,
                    file: "".into(),
                },
                at: if folded { header_at } else { target },
                target,
                seam: index,
            });
        }
        if !folded {
            at += rows as f32 * ROW;
        }
    }
    out
}

fn height(entries: &[SessionEntry]) -> f32 {
    entries
        .iter()
        .filter(|entry| entry.is_header || !entry.folded)
        .map(|entry| entry.at + ROW)
        .fold(TOP, f32::max)
}

fn main() -> Result<(), slint::PlatformError> {
    let count: usize = std::env::var("PROBE_ROWS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(150);
    let seconds: u64 = std::env::var("PROBE_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let toggle_ms: u64 = std::env::var("PROBE_TOGGLE_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(300);

    let ui = AppWindow::new()?;
    ui.window().set_size(slint::PhysicalSize::new(1200, 830));
    ui.set_sidebar_open(true);

    let folded = Rc::new(std::cell::Cell::new(false));
    let previous = Rc::new(RefCell::new(entries(count, false)));
    let done_items = previous
        .borrow()
        .iter()
        .filter(|entry| !entry.is_header)
        .map(|entry| entry.item.clone())
        .collect::<Vec<_>>();
    ui.set_done_items(ModelRc::from(Rc::new(VecModel::from(done_items))));
    ui.set_session_entries(ModelRc::from(Rc::new(VecModel::from(
        previous.borrow().clone(),
    ))));
    ui.set_sessions_height(height(&previous.borrow()));
    ui.set_prev_sessions_height(height(&previous.borrow()));
    ui.set_fold_index(2);
    ui.set_fold_open(false);
    ui.set_splice_at(ModelRc::from(Rc::new(VecModel::from(Vec::<f32>::new()))));

    let weak = ui.as_weak();
    let timer = Rc::new(Timer::default());
    let timer_stop = timer.clone();
    let folded_timer = folded.clone();
    let previous_timer = previous.clone();
    timer.start(
        TimerMode::Repeated,
        Duration::from_millis(toggle_ms),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let was = previous_timer.borrow().clone();
            let now_folded = !folded_timer.get();
            let next = entries(count, now_folded);
            let from = was.iter().map(|entry| entry.at).collect::<Vec<f32>>();
            ui.set_prev_sessions_height(height(&was));
            ui.set_sessions_height(height(&next));
            ui.set_fold_index(2);
            ui.set_fold_open(!now_folded);
            ui.set_session_entries(ModelRc::from(Rc::new(VecModel::from(next.clone()))));
            ui.set_splice_at(ModelRc::from(Rc::new(VecModel::from(from))));
            ui.invoke_trigger_list_anim();
            *previous_timer.borrow_mut() = next;
            folded_timer.set(now_folded);
        },
    );
    std::mem::forget(timer);

    let stop = timer_stop.clone();
    let stop_timer = Rc::new(Timer::default());
    stop_timer.start(
        TimerMode::SingleShot,
        Duration::from_secs(seconds),
        move || {
            stop.stop();
            let _ = slint::quit_event_loop();
        },
    );
    std::mem::forget(stop_timer);

    eprintln!("[probe] rows={count} toggle={toggle_ms}ms seconds={seconds}");
    ui.run()
}
