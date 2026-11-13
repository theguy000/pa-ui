use crate::{sessions, AppWindow, SessionRow};
use slint::ComponentHandle;
use std::cell::Cell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CloseDecision {
    Quit,
    Confirm,
}

pub(crate) fn close_decision(any_running_turn: bool, confirm_quit: bool) -> CloseDecision {
    if any_running_turn && confirm_quit {
        CloseDecision::Confirm
    } else {
        CloseDecision::Quit
    }
}

pub(crate) fn has_running_turn(rows: &[SessionRow], selected_busy: bool) -> bool {
    selected_busy
        || rows
            .iter()
            .any(|row| sessions::is_top_level(row) && sessions::tag_of(row) == sessions::RowTag::Active)
}

fn evaluate_close(ui: &AppWindow, sidebar: &sessions::Sidebar, confirmed: &Cell<bool>) -> CloseDecision {
    if confirmed.get() {
        return CloseDecision::Quit;
    }
    let running = has_running_turn(&sidebar.cache.borrow(), ui.get_busy());
    close_decision(running, ui.get_setting_confirm_quit())
}

fn open_confirmation(ui: &AppWindow) {
    if !ui.get_quit_confirm_open() {
        ui.set_quit_confirm_open(true);
        ui.set_quit_confirm_focus_tick(ui.get_quit_confirm_focus_tick() + 1);
    }
}

fn trigger_close(ui: &AppWindow) {
    if !crate::titlebar::close() {
        let _ = ui
            .window()
            .dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested);
    }
}

pub(crate) fn bind(ui: &AppWindow, sidebar: &Rc<sessions::Sidebar>) {
    let confirmed = Rc::new(Cell::new(false));

    {
        let weak = ui.as_weak();
        let sidebar = sidebar.clone();
        let confirmed = confirmed.clone();
        ui.window().on_close_requested(move || {
            let Some(ui) = weak.upgrade() else {
                return slint::CloseRequestResponse::HideWindow;
            };
            match evaluate_close(&ui, &sidebar, &confirmed) {
                CloseDecision::Quit => slint::CloseRequestResponse::HideWindow,
                CloseDecision::Confirm => {
                    open_confirmation(&ui);
                    slint::CloseRequestResponse::KeepWindowShown
                }
            }
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_titlebar_close(move || {
            let Some(ui) = weak.upgrade() else { return };
            trigger_close(&ui);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_quit_cancelled(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_quit_confirm_open(false);
            ui.invoke_focus_composer();
        });
    }

    {
        let weak = ui.as_weak();
        let confirmed = confirmed;
        ui.on_quit_confirmed(move || {
            let Some(ui) = weak.upgrade() else { return };
            confirmed.set(true);
            ui.set_quit_confirm_open(false);
            trigger_close(&ui);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn close_decision_requires_both_running_turn_and_setting() {
        assert_eq!(close_decision(false, false), CloseDecision::Quit);
        assert_eq!(close_decision(false, true), CloseDecision::Quit);
        assert_eq!(close_decision(true, false), CloseDecision::Quit);
        assert_eq!(close_decision(true, true), CloseDecision::Confirm);
    }

    #[test]
    fn running_turn_checks_selected_busy_and_top_level_active_rows_only() {
        let idle = SessionRow::from_value(&json!({
            "activeSessionId": "a1",
            "sessionId": "s1",
            "workerState": "idle",
            "activity": "idle",
            "rlmDepth": 0
        }));
        let active_subagent = SessionRow::from_value(&json!({
            "activeSessionId": "sub1",
            "sessionId": "sub-s1",
            "parentSessionId": "s1",
            "workerState": "busy",
            "activity": "working",
            "isSessionActive": true,
            "rlmDepth": 1
        }));
        let active_top_level = SessionRow::from_value(&json!({
            "activeSessionId": "a2",
            "sessionId": "s2",
            "workerState": "busy",
            "activity": "working",
            "isSessionActive": true,
            "rlmDepth": 0
        }));
        let archived_top_level = SessionRow::from_value(&json!({
            "id": "s3",
            "lifecycle": "archived",
            "workerState": "busy",
            "activity": "working",
            "isSessionActive": true,
            "rlmDepth": 0
        }));

        assert!(!has_running_turn(&[], false));
        assert!(has_running_turn(&[], true));
        assert!(!has_running_turn(&[idle.clone(), active_subagent.clone()], false));
        assert!(!has_running_turn(&[idle.clone(), archived_top_level], false));
        assert!(has_running_turn(&[idle, active_subagent, active_top_level], false));
    }
}
