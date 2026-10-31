//! The status line: the one place the window says it could not do what was asked,
//! or that the daemon link is not there.
//!
//! Deliberately narrow. Everything else the reader can already see for themselves —
//! a sent message lands in the transcript, a model switch moves the chip, a queue
//! row that is being edited shows up in the composer — so nothing else writes here.
//! The line was read by nothing between the sidebar rebuild and now; it is an
//! exception channel, not an activity log.

use crate::*;

/// Something the window refused, or that failed. Shown in the failure tint.
pub(crate) fn trouble(ui: &AppWindow, message: impl Into<SharedString>) {
    ui.set_status_line(message.into());
    ui.set_status_line_bad(true);
}

/// A neutral note, such as the opening "connecting".
pub(crate) fn say(ui: &AppWindow, message: impl Into<SharedString>) {
    ui.set_status_line(message.into());
    ui.set_status_line_bad(false);
}

/// The last thing worked, so there is nothing outstanding to report.
pub(crate) fn clear(ui: &AppWindow) {
    ui.set_status_line("".into());
    ui.set_status_line_bad(false);
}
