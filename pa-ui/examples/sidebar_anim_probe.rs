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
const HEADER: f32 = 26.0;
const ROW: f32 = 32.0;
const GROUP_PAD: f32 = 4.0;
const GROUP_GAP: f32 = 10.0;
const GROUP_BODY_GAP: f32 = 2.0;

fn group_boxes(count: usize, done_folded: bool) -> Vec<SidebarGroupBox> {
    let mut out = Vec::with_capacity(3);
    let mut at = TOP;
    for (index, rows, folded) in [
        (0usize, 0usize, false),
        (1, 0usize, false),
        (2, count, done_folded),
    ] {
        let box_h = if !folded && rows > 0 {
            GROUP_PAD + HEADER + GROUP_BODY_GAP + rows as f32 * ROW + GROUP_PAD
        } else {
            GROUP_PAD + HEADER + GROUP_PAD
        };
        out.push(SidebarGroupBox {
            at,
            height: box_h,
            empty: rows == 0,
        });
        if index < 2 {
            at += box_h + GROUP_GAP;
        }
    }
    out
}

fn entries(count: usize, done_folded: bool) -> Vec<SessionEntry> {
    let mut out = Vec::with_capacity(count + 3);
    let mut at = TOP;
    for (index, label, rows, folded) in [
        (0, "Active", 0usize, false),
        (1, "Idle", 0usize, false),
        (2, "Done", count, done_folded),
    ] {
        let box_at = at;
        let header_at = box_at + GROUP_PAD;
        out.push(SessionEntry {
            is_header: true,
            label: label.into(),
            folded,
            item: SessionItem::default(),
            at: header_at,
            target: header_at,
            seam: index,
        });
        for number in 0..rows {
            let target = header_at + HEADER + GROUP_BODY_GAP + number as f32 * ROW;
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
        let box_h = if !folded && rows > 0 {
            GROUP_PAD + HEADER + GROUP_BODY_GAP + rows as f32 * ROW + GROUP_PAD
        } else {
            GROUP_PAD + HEADER + GROUP_PAD
        };
        if index < 2 {
            at = box_at + box_h + GROUP_GAP;
        }
    }
    out
}

fn height(entries: &[SessionEntry]) -> f32 {
    entries
        .iter()
        .filter(|entry| entry.is_header || !entry.folded)
        .map(|entry| {
            entry.at
                + if entry.is_header { HEADER } else { ROW }
                + GROUP_PAD
        })
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
    let initial_boxes = group_boxes(count, false);
    let prev_boxes = Rc::new(RefCell::new(initial_boxes.clone()));
    let done_items = previous
        .borrow()
        .iter()
        .filter(|entry| !entry.is_header)
        .map(|entry| entry.item.clone())
        .collect::<Vec<_>>();
    ui.set_done_items(ModelRc::from(Rc::new(VecModel::from(done_items))));
    ui.set_prev_group_boxes(ModelRc::from(Rc::new(VecModel::from(
        initial_boxes.clone(),
    ))));
    ui.set_group_boxes(ModelRc::from(Rc::new(VecModel::from(initial_boxes))));
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
    let prev_boxes_timer = prev_boxes.clone();
    timer.start(
        TimerMode::Repeated,
        Duration::from_millis(toggle_ms),
        move || {
            let Some(ui) = weak.upgrade() else { return };
            let was = previous_timer.borrow().clone();
            let was_boxes = prev_boxes_timer.borrow().clone();
            let now_folded = !folded_timer.get();
            let next = entries(count, now_folded);
            let next_boxes = group_boxes(count, now_folded);
            let from = was.iter().map(|entry| entry.at).collect::<Vec<f32>>();
            ui.set_prev_sessions_height(height(&was));
            ui.set_sessions_height(height(&next));
            ui.set_fold_index(2);
            ui.set_fold_open(!now_folded);
            ui.set_prev_group_boxes(ModelRc::from(Rc::new(VecModel::from(was_boxes))));
            ui.set_group_boxes(ModelRc::from(Rc::new(VecModel::from(next_boxes.clone()))));
            ui.set_session_entries(ModelRc::from(Rc::new(VecModel::from(next.clone()))));
            ui.set_splice_at(ModelRc::from(Rc::new(VecModel::from(from))));
            ui.invoke_trigger_list_anim();
            *previous_timer.borrow_mut() = next;
            *prev_boxes_timer.borrow_mut() = next_boxes;
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
