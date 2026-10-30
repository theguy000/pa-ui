//! The queued lanes' buttons: promote, edit, remove, clear.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn clear(ui: &AppWindow, job_tx: &Sender<Job>) {
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

pub(crate) fn act(ui: &AppWindow, editing: &Rc<RefCell<Option<(String, usize, String)>>>, job_tx: &Sender<Job>) {
    let weak = ui.as_weak();
    let editing = editing.clone();
    let jobs = job_tx.clone();
    ui.on_queue_act(move |lane, slot, action| {
        let Some(ui) = weak.upgrade() else { return };
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
