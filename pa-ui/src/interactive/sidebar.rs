//! The sidebar: filtering sessions, starting a new chat, renaming one, and the
//! per-session menu.

use std::cell::RefCell;
use std::rc::Rc;

use crate::*;

pub(crate) fn rename(ui: &AppWindow, job_tx: &Sender<Job>) {
    let jobs = job_tx.clone();
    let window = ui.as_weak();
    ui.on_rename_requested(move |session, name| {
        let session = session.to_string();
        if session.is_empty() || name.trim().is_empty() {
            return;
        }
        // A live row renames through the session; a done row has no worker to
        // rename, only a file, and `rename` on a saved row would start one.
        let saved = window
            .upgrade()
            .and_then(|ui| current_item(&ui, &session))
            .filter(|item| item.done)
            .map(|item| item.file.to_string());
        if let Some(session_path) = saved {
            let _ = jobs.send(Job::RenameSaved {
                session_path,
                name: name.trim().to_string(),
            });
            return;
        }
        let _ = jobs.send(Job::Rename {
            session,
            name: name.trim().to_string(),
        });
    });
}

/// The row behind an id, copied out of the window's list.
fn current_item(ui: &AppWindow, id: &str) -> Option<SessionItem> {
    ui.get_sessions().iter().find(|item| item.id == id)
}

/// The items a session row's menu offers, in the order they are drawn.
///
/// A row's actions follow from what the row is, because the commands differ: a
/// live row is stopped (`kill`), a done row is deleted by its file, and the
/// rename that leaves a done row alone is a different command from the one that
/// renames a live session. Two items only a live row has. Delete needs a
/// worker-free row, and the daemon refuses it on a live one. Share needs a
/// worker to run `export_html`, and a done row has none, so the item would only
/// fail. A menu item that can only fail is worse than no item.
///
/// `confirming` is the row mid-confirm: its Delete turns into the question, and
/// the note under it into the warning.
pub(crate) fn menu_items(done: bool, confirming: bool) -> Vec<SessionMenuItem> {
    if done {
        let delete = if confirming {
            menu_item(
                "delete",
                "Delete for good",
                "this removes the session file",
                true,
            )
        } else {
            menu_item("delete", "Delete", "", true)
        };
        vec![
            menu_item("rename", "Rename", "", false),
            menu_item("copy-id", "Copy id", "", false),
            delete,
        ]
    } else {
        vec![
            menu_item("done", "Mark done", "", false),
            menu_item("rename", "Rename", "", false),
            menu_item("share", "Share as gist", "", false),
            menu_item("copy-id", "Copy id", "", false),
        ]
    }
}

/// One row of the menu: its key, the words it shows, the warning it carries once
/// armed, and whether it sits below the separator away from the actions that
/// act on the session itself rather than on its file.
fn menu_item(key: &str, label: &str, note: &str, separated: bool) -> SessionMenuItem {
    SessionMenuItem {
        key: key.into(),
        label: label.into(),
        danger: separated,
        note: note.into(),
        separated,
    }
}

/// What a row's menu action does, worked out from the row alone so the decision
/// can be tested without a window or a daemon.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    /// stop the live session: it becomes a done row
    MarkDone,
    /// start editing the row's name in place
    Rename,
    /// write the session's viewer HTML and upload it
    Share,
    CopyId,
    /// delete the done row's file
    Delete,
    /// a key nothing offers, or an action the row does not have: a live row
    /// has no Delete, and a done row has neither Mark done nor Share
    Nothing,
}

pub(crate) fn menu_action(key: &str, done: bool) -> MenuAction {
    match key {
        "done" if !done => MenuAction::MarkDone,
        "rename" => MenuAction::Rename,
        "share" if !done => MenuAction::Share,
        "copy-id" => MenuAction::CopyId,
        "delete" if done => MenuAction::Delete,
        _ => MenuAction::Nothing,
    }
}

/// The row's menu, wired to the window: each row offers its own item set, and a
/// picked item does that row's own thing.
pub(crate) fn menu(ui: &AppWindow, job_tx: &Sender<Job>) {
    let jobs = job_tx.clone();
    let window = ui.as_weak();
    // the row waiting on its Delete confirmation, if any
    let confirming: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // the dots: the row's items, read from what the row is
    let open_window = ui.as_weak();
    let open_confirming = confirming.clone();
    ui.on_session_menu_opened(move |id| {
        let id = id.to_string();
        let Some(ui) = open_window.upgrade() else {
            return;
        };
        let Some(item) = current_item(&ui, &id) else {
            return;
        };
        // another row's half-made confirmation is not this row's
        let mut confirming = open_confirming.borrow_mut();
        if confirming.as_deref() != Some(id.as_str()) {
            *confirming = None;
        }
        let items = menu_items(item.done, confirming.is_some());
        ui.set_menu_items(ModelRc::from(Rc::new(VecModel::from(items))));
    });

    // a menu that closes by clicking away forgets the confirmation with it
    let close_confirming = confirming.clone();
    ui.on_menu_closed(move || {
        *close_confirming.borrow_mut() = None;
    });

    let picked_confirming = confirming.clone();
    ui.on_session_menu_picked(move |id, key| {
        let id = id.to_string();
        let key = key.to_string();
        let Some(ui) = window.upgrade() else { return };
        let mut confirming = picked_confirming.borrow_mut();
        // The poll can drop a row between the two presses, and the second press
        // has to go through: the row the first press confirmed is the one the
        // reader meant, whether this poll still lists it or not.
        let done = match current_item(&ui, &id) {
            Some(item) => item.done,
            None if confirming.as_deref() == Some(id.as_str()) => true,
            None => return,
        };
        match menu_action(&key, done) {
            // the first Delete asks, the second one acts: the TUI's two-press
            // confirm, with the item itself as the second press
            MenuAction::Delete if confirming.as_deref() != Some(id.as_str()) => {
                *confirming = Some(id.clone());
                let items = menu_items(true, true);
                ui.set_menu_items(ModelRc::from(Rc::new(VecModel::from(items))));
                return;
            }
            MenuAction::Delete => {
                *confirming = None;
                let path = current_item(&ui, &id)
                    .map(|item| item.file.to_string())
                    .unwrap_or_default();
                if path.is_empty() {
                    crate::status::trouble(
                        &ui,
                        "could not delete: the row carries no session file",
                    );
                } else {
                    let _ = jobs.send(Job::DeleteSaved { session_path: path });
                }
                // The menu's panel is the row's own area, so the redraw that
                // drops it cannot come from the list: hide it here instead.
                ui.set_menu_id("".into());
                return;
            }
            MenuAction::MarkDone => {
                *confirming = None;
                let _ = jobs.send(Job::MarkDone {
                    session: id.clone(),
                });
            }
            MenuAction::Rename => {
                *confirming = None;
                let Some(item) = current_item(&ui, &id) else {
                    return;
                };
                ui.set_menu_id("".into());
                ui.set_editing_id(id.clone().into());
                ui.set_draft(item.full.clone());
                return;
            }
            MenuAction::Share => {
                *confirming = None;
                let _ = jobs.send(Job::Share {
                    session: id.clone(),
                });
            }
            MenuAction::CopyId => {
                *confirming = None;
                let Some(item) = current_item(&ui, &id) else {
                    return;
                };
                match copy_to_clipboard(&item.uuid) {
                    Ok(()) => crate::status::say(&ui, "session id copied"),
                    Err(error) => crate::status::trouble(&ui, format!("could not copy: {error}")),
                }
            }
            MenuAction::Nothing => {
                *confirming = None;
            }
        }
        ui.set_menu_id("".into());
    });
}

/// Put the last copied id on the system clipboard. The window's own backend
/// keeps its clipboard private, so the one call that needs it goes to arboard,
/// the crate the backend itself uses.
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("no clipboard: {error}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| error.to_string())
}

pub(crate) fn new_chat(ui: &AppWindow, job_tx: &Sender<Job>) {
    let jobs = job_tx.clone();
    ui.on_new_chat(move || {
        let _ = jobs.send(Job::NewChat);
    });
}

/// Rebuild the column from what the window holds right now, and hand the window
/// the y every entry started from before the fold so `row-y` slides each entry
/// from its old position to its new `at`.
fn compose_entries(ui: &AppWindow, was: [bool; 3]) {
    let (active, idle, done) = (
        ui.get_active_items().iter().collect::<Vec<_>>(),
        ui.get_idle_items().iter().collect::<Vec<_>>(),
        ui.get_done_items().iter().collect::<Vec<_>>(),
    );
    let old_entries =
        crate::sessions::session_entries(&active, &idle, &done, was[0], was[1], was[2]);
    let entries = crate::sessions::session_entries(
        &active,
        &idle,
        &done,
        ui.get_active_folded(),
        ui.get_idle_folded(),
        ui.get_done_folded(),
    );
    let saved_cy = ui.invoke_get_sidebar_cy();
    // the lists have the same rows in the same order, only the ys moved, so
    // each entry starts at its own old y
    let from_y: Vec<f32> = old_entries.iter().map(|entry| entry.at).collect();
    ui.set_prev_sessions_height(crate::sessions::entries_height(&old_entries));
    ui.set_sessions_height(crate::sessions::entries_height(&entries));
    ui.set_session_entries(ModelRc::from(Rc::new(VecModel::from(entries))));
    ui.set_splice_at(ModelRc::from(Rc::new(VecModel::from(from_y))));
    ui.invoke_trigger_list_anim();
    ui.invoke_restore_sidebar_cy(saved_cy);
}

/// A group's header was clicked. The fold state is the window's, and the y each
/// row started from comes from here, so the window only has to run the animation.
pub(crate) fn folds(ui: &AppWindow) {
    let window = ui.as_weak();
    let before: Rc<RefCell<[bool; 3]>> = Rc::new(RefCell::new([
        ui.get_active_folded(),
        ui.get_idle_folded(),
        ui.get_done_folded(),
    ]));
    ui.on_fold_changed(move || {
        let Some(ui) = window.upgrade() else { return };
        let now = [
            ui.get_active_folded(),
            ui.get_idle_folded(),
            ui.get_done_folded(),
        ];
        let was = before.replace(now);
        let Some(index) = (0..3).find(|index| now[*index] != was[*index]) else {
            return;
        };
        ui.set_fold_index(index as i32);
        ui.set_fold_open(!now[index]);
        compose_entries(&ui, was);
    });
}
