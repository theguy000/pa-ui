//! The sidebar: filtering sessions, starting a new chat, renaming one.

use crate::*;

pub(crate) fn rename(ui: &AppWindow, job_tx: &Sender<Job>) {
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

pub(crate) fn new_chat(ui: &AppWindow, job_tx: &Sender<Job>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    ui.on_new_chat(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_status_line("starting a new chat".into());
        }
        let _ = jobs.send(Job::NewChat);
    });
}

