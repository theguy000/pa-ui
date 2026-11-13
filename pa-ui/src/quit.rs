use crate::{AppWindow, Tick};
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

fn evaluate_close(ui: &AppWindow, tick: &Tick, confirmed: &Cell<bool>) -> CloseDecision {
    if confirmed.get() {
        return CloseDecision::Quit;
    }
    let settled = tick.settled_session_id();
    let running = ui.get_busy() || tick.sidebar.has_running_top_level(settled.as_deref());
    close_decision(running, ui.get_setting_confirm_quit())
}

fn open_confirmation(ui: &AppWindow) {
    if !ui.get_quit_confirm_open() {
        ui.set_quit_confirm_open(true);
        ui.set_quit_confirm_focus_tick(ui.get_quit_confirm_focus_tick() + 1);
    }
}

fn trigger_close(ui: &AppWindow) {
    if crate::titlebar::has_window() {
        if let Err(error) = crate::titlebar::close() {
            eprintln!("[quit] {error}");
        }
        return;
    }
    if let Err(error) = ui
        .window()
        .dispatch_event_with_result(slint::platform::WindowEvent::CloseRequested)
    {
        eprintln!("[quit] close dispatch failed: {error}");
    }
}

pub(crate) fn bind(ui: &AppWindow, tick: &Rc<Tick>) {
    let confirmed = Rc::new(Cell::new(false));

    {
        let weak = ui.as_weak();
        let tick = tick.clone();
        let confirmed = confirmed.clone();
        ui.window().on_close_requested(move || {
            let Some(ui) = weak.upgrade() else {
                return slint::CloseRequestResponse::HideWindow;
            };
            match evaluate_close(&ui, &tick, &confirmed) {
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
        ui.on_quit_requested(move || {
            let Some(ui) = weak.upgrade() else { return };
            trigger_close(&ui);
        });
    }

    {
        let weak = ui.as_weak();
        ui.on_quit_cancelled(move || {
            let Some(ui) = weak.upgrade() else { return };
            ui.set_quit_confirm_open(false);
            if ui.get_search_open() {
                ui.set_search_focus_tick(ui.get_search_focus_tick() + 1);
            } else if !ui.get_settings_menu() && !ui.get_share_open() {
                ui.invoke_focus_composer();
            }
        });
    }

    {
        let weak = ui.as_weak();
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

    #[test]
    fn close_decision_requires_both_running_turn_and_setting() {
        assert_eq!(close_decision(false, false), CloseDecision::Quit);
        assert_eq!(close_decision(false, true), CloseDecision::Quit);
        assert_eq!(close_decision(true, false), CloseDecision::Quit);
        assert_eq!(close_decision(true, true), CloseDecision::Confirm);
    }
}
