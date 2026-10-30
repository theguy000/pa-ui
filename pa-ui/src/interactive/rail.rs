//! The rail: filtering sessions, starting a new chat, renaming one.

#[allow(unused_imports)]
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

pub(crate) fn search(ui: &AppWindow, session_filter: &Rc<RefCell<String>>, sessions_cache: &Rc<RefCell<Vec<SessionRow>>>) {
    let weak = ui.as_weak();
    let filter = session_filter.clone();
    let cache = sessions_cache.clone();
    ui.on_search_edited(move |query| {
        let Some(ui) = weak.upgrade() else { return };
        let query = query.to_string();
        *filter.borrow_mut() = query.clone();
        let rows = cache.borrow().clone();
        let items = session_items(&rows, &query);
        if let Some(ui) = weak.upgrade() {
            ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(items))));
        }
    });
}
