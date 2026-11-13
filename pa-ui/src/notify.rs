//! When a Session's turn ending becomes a notice, and where it is delivered.

use crate::sessions::{display_name, is_top_level, tag_of, RowTag};
use crate::{AppWindow, SessionRow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionNotification {
    Off,
    Unfocused,
    Always,
}

impl SessionNotification {
    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "Always" => Self::Always,
            "Only when unfocused" => Self::Unfocused,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TurnOutcome {
    Finished,
    Failed,
}

/// Where one turn ending is shown. Both channels are independent: the in-app
/// setting governs the banner, the Session notification setting governs the
/// system toast.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Delivery {
    pub in_app: bool,
    pub system: bool,
}

/// What the window is doing when the turn ends.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Turn {
    pub focused: bool,
    /// The turn belongs to the Session the reader has open.
    pub selected: bool,
    pub session_notifications: SessionNotification,
    pub in_app: bool,
}

/// One decision for both channels. A turn in the Session the reader is looking
/// at is never announced: they can already see it. Every other turn follows the
/// two settings.
pub(crate) fn deliver(turn: Turn) -> Delivery {
    if turn.focused && turn.selected {
        return Delivery::default();
    }
    Delivery {
        in_app: turn.in_app && turn.focused && !turn.selected,
        system: match turn.session_notifications {
            SessionNotification::Off => false,
            SessionNotification::Unfocused => !turn.focused,
            SessionNotification::Always => true,
        },
    }
}

pub(crate) trait ToastNotifier {
    fn send(&self, session_id: &str, title: &str, body: &str, failed: bool) -> Result<(), String>;
}

pub(crate) struct WindowsToastNotifier;

impl ToastNotifier for WindowsToastNotifier {
    fn send(&self, session_id: &str, title: &str, body: &str, failed: bool) -> Result<(), String> {
        crate::titlebar::show_toast(session_id, title, body, failed)
    }
}

/// Put one turn ending on screen. A newer notice replaces the older one.
pub(crate) fn show(ui: &AppWindow, session: &str, name: &str, failed: bool) {
    ui.set_notice_session(session.into());
    ui.set_notice_name(name.into());
    ui.set_notice_failed(failed);
    ui.set_notice_open(true);
    ui.set_notice_tick(ui.get_notice_tick() + 1);
}

pub(crate) fn clear(ui: &AppWindow) {
    ui.set_notice_open(false);
    ui.set_notice_session("".into());
    ui.set_notice_name("".into());
    ui.set_notice_failed(false);
}

pub(crate) fn session_label(rows: &[SessionRow], session: &str) -> String {
    rows.iter()
        .find(|row| row.id == session)
        .map(|row| display_name(row).to_string())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "Session".to_string())
}

fn same_row(left: &SessionRow, right: &SessionRow) -> bool {
    left.id == right.id || (!left.session_id.is_empty() && left.session_id == right.session_id)
}

/// Top-level Sessions that moved out of Running between two roster snapshots.
pub(crate) fn finished_top_level_rows(
    before: &[SessionRow],
    after: &[SessionRow],
    skip_ids: &[String],
) -> Vec<(String, String, TurnOutcome)> {
    let mut finished = Vec::new();
    for prev in before {
        if !is_top_level(prev) || tag_of(prev) != RowTag::Active {
            continue;
        }
        let Some(now) = after.iter().find(|row| same_row(prev, row)) else {
            continue;
        };
        if tag_of(now) == RowTag::Active {
            continue;
        }
        if skip_ids.iter().any(|skip| skip == &prev.id || skip == &now.id) {
            continue;
        }
        let label = display_name(now);
        let name = if label.trim().is_empty() {
            display_name(prev).to_string()
        } else {
            label.to_string()
        };
        let outcome = if now.worker_state == "failed" {
            TurnOutcome::Failed
        } else {
            TurnOutcome::Finished
        };
        finished.push((now.id.clone(), name, outcome));
    }
    finished
}

pub(crate) fn dispatch(
    ui: &AppWindow,
    notifier: &dyn ToastNotifier,
    session_id: &str,
    name: &str,
    outcome: TurnOutcome,
) {
    let selected_id = ui.get_selected_id();
    let selected = !selected_id.is_empty() && selected_id.as_str() == session_id;
    let decision = deliver(Turn {
        focused: ui.get_window_active(),
        selected,
        session_notifications: SessionNotification::parse(
            ui.get_setting_session_notifications().as_str(),
        ),
        in_app: ui.get_setting_in_app_notifications(),
    });
    let failed = outcome == TurnOutcome::Failed;
    if decision.in_app {
        show(ui, session_id, name, failed);
    }
    if decision.system {
        let title = if failed {
            "Session failed"
        } else {
            "Session finished"
        };
        let body = if name.trim().is_empty() {
            "Session updated"
        } else {
            name
        };
        if let Err(error) = notifier.send(session_id, title, body, failed) {
            eprintln!("[notify] {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn(focused: bool, selected: bool, setting: SessionNotification, in_app: bool) -> Turn {
        Turn {
            focused,
            selected,
            session_notifications: setting,
            in_app,
        }
    }

    fn row(fields: serde_json::Value) -> SessionRow {
        SessionRow::from_value(&fields)
    }

    #[test]
    fn a_setting_is_read_from_its_stored_word() {
        assert_eq!(SessionNotification::parse("Off"), SessionNotification::Off);
        assert_eq!(
            SessionNotification::parse("Only when unfocused"),
            SessionNotification::Unfocused
        );
        assert_eq!(
            SessionNotification::parse("Always"),
            SessionNotification::Always
        );
        assert_eq!(SessionNotification::parse("Bogus"), SessionNotification::Off);
    }

    #[test]
    fn the_session_the_reader_is_looking_at_is_never_announced() {
        for setting in [
            SessionNotification::Off,
            SessionNotification::Unfocused,
            SessionNotification::Always,
        ] {
            assert_eq!(deliver(turn(true, true, setting, true)), Delivery::default());
        }
    }

    #[test]
    fn unfocused_selected_session_still_sends_a_system_toast() {
        assert_eq!(
            deliver(turn(false, true, SessionNotification::Unfocused, true)),
            Delivery {
                in_app: false,
                system: true
            }
        );
        assert_eq!(
            deliver(turn(false, true, SessionNotification::Always, false)),
            Delivery {
                in_app: false,
                system: true
            }
        );
        assert_eq!(
            deliver(turn(false, true, SessionNotification::Off, true)),
            Delivery::default()
        );
    }

    #[test]
    fn off_suppresses_the_toast_but_not_the_banner() {
        assert_eq!(
            deliver(turn(true, false, SessionNotification::Off, true)),
            Delivery {
                in_app: true,
                system: false
            }
        );
        assert_eq!(
            deliver(turn(false, false, SessionNotification::Off, true)),
            Delivery::default()
        );
    }

    #[test]
    fn only_when_unfocused_delivers_the_toast_only_behind_the_window() {
        assert_eq!(
            deliver(turn(true, false, SessionNotification::Unfocused, false)),
            Delivery::default()
        );
        assert_eq!(
            deliver(turn(false, false, SessionNotification::Unfocused, false)),
            Delivery {
                in_app: false,
                system: true
            }
        );
    }

    #[test]
    fn always_delivers_the_toast_in_both_focus_states() {
        assert_eq!(
            deliver(turn(true, false, SessionNotification::Always, false)),
            Delivery {
                in_app: false,
                system: true
            }
        );
        assert_eq!(
            deliver(turn(false, false, SessionNotification::Always, false)),
            Delivery {
                in_app: false,
                system: true
            }
        );
    }

    #[test]
    fn the_banner_needs_the_window_in_front() {
        assert_eq!(
            deliver(turn(false, false, SessionNotification::Off, true)),
            Delivery::default()
        );
        assert_eq!(
            deliver(turn(true, false, SessionNotification::Off, true)),
            Delivery {
                in_app: true,
                system: false
            }
        );
    }

    #[test]
    fn finished_rows_detect_only_top_level_running_to_non_running_transitions() {
        let active_top = row(json!({
            "activeSessionId": "a1", "sessionId": "u1", "firstMessage": "Alpha",
            "workerState": "ready", "activity": "working"
        }));
        let idle_top = row(json!({
            "activeSessionId": "a1", "sessionId": "u1", "firstMessage": "Alpha",
            "workerState": "ready", "activity": "idle"
        }));
        let active_sub = row(json!({
            "activeSessionId": "sub1", "sessionId": "usub1", "parentSessionId": "u1",
            "rlmDepth": 1, "firstMessage": "Sub", "workerState": "ready", "activity": "working"
        }));
        let idle_sub = row(json!({
            "activeSessionId": "sub1", "sessionId": "usub1", "parentSessionId": "u1",
            "rlmDepth": 1, "firstMessage": "Sub", "workerState": "ready", "activity": "idle"
        }));
        let stayed_idle = row(json!({
            "activeSessionId": "i1", "sessionId": "ui1", "firstMessage": "Idle",
            "workerState": "ready", "activity": "idle"
        }));
        let archived_from_idle = row(json!({
            "activeSessionId": "i1", "sessionId": "ui1", "firstMessage": "Idle",
            "lifecycle": "archived"
        }));

        assert_eq!(
            finished_top_level_rows(
                &[active_top.clone(), active_sub, stayed_idle],
                &[idle_top.clone(), idle_sub, archived_from_idle],
                &[],
            ),
            vec![("a1".to_string(), "Alpha".to_string(), TurnOutcome::Finished)]
        );
        let skip_a1 = vec!["a1".to_string()];
        assert!(finished_top_level_rows(std::slice::from_ref(&active_top), &[idle_top], &skip_a1).is_empty());

        let failed_top = row(json!({
            "activeSessionId": "a1", "sessionId": "u1", "firstMessage": "Alpha",
            "workerState": "failed", "activity": "idle"
        }));
        assert_eq!(
            finished_top_level_rows(std::slice::from_ref(&active_top), &[failed_top], &[]),
            vec![("a1".to_string(), "Alpha".to_string(), TurnOutcome::Failed)]
        );

        let rekeyed_idle = row(json!({
            "activeSessionId": "a2", "sessionId": "u1", "firstMessage": "Alpha",
            "workerState": "ready", "activity": "idle"
        }));
        assert_eq!(
            finished_top_level_rows(
                std::slice::from_ref(&active_top),
                std::slice::from_ref(&rekeyed_idle),
                &[],
            ),
            vec![("a2".to_string(), "Alpha".to_string(), TurnOutcome::Finished)]
        );
        assert!(finished_top_level_rows(
            std::slice::from_ref(&active_top),
            std::slice::from_ref(&rekeyed_idle),
            &skip_a1,
        )
        .is_empty());
        assert!(finished_top_level_rows(&[active_top], &[rekeyed_idle], &["a2".to_string()]).is_empty());

        let blank_prev = row(json!({
            "activeSessionId": "b1", "sessionId": "", "firstMessage": "Blank",
            "workerState": "ready", "activity": "working"
        }));
        let blank_other = row(json!({
            "activeSessionId": "b2", "sessionId": "", "firstMessage": "Blank",
            "workerState": "ready", "activity": "idle"
        }));
        assert!(finished_top_level_rows(&[blank_prev], &[blank_other], &[]).is_empty());
    }

    #[test]
    fn windows_toast_notifier_errors_without_a_window() {
        let err = WindowsToastNotifier
            .send("s1", "Session finished", "Alpha", false)
            .unwrap_err();
        assert!(!err.is_empty());
    }
}
