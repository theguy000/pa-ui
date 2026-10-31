//! Scrolling: who is allowed to move the view, and how the reader's place is held across a rebuild.

use crate::*;

/// Follow the newest row only when the reader is already there.
///
/// Every streamed delta and every poll used to drag the view to the bottom, so
/// scrolling up to read while the agent worked was impossible: the next frame
/// pulled the page out from under the pointer.
pub(crate) fn scroll_if_parked(ui: &AppWindow) {
    // Following is a latch the reader owns, not a measurement: a hand on the
    // scrollbar turns it off before any token can move the view, and only
    // arriving at the floor turns it back on.
    if ui.get_follow_bottom() && !ui.get_scrollbar_dragging() {
        scroll_to_end_when_ready(&ui.as_weak());
    }
}


/// Whether the reader's view must be held still while rows change.
///
/// Two facts, and the conclusion is the opposite of "should we scroll": while
/// following, the end of the transcript is the destination; while a hand is on
/// the bar, that hand owns the offset.
#[must_use]
pub fn should_hold_offset(following: bool, dragging: bool) -> bool {
    !following && !dragging
}


/// The reader's offset, if it is theirs to keep right now.
pub(crate) fn hold_offset(ui: &AppWindow) -> Option<f32> {
    if !should_hold_offset(ui.get_follow_bottom(), ui.get_scrollbar_dragging()) {
        return None;
    }
    Some(ui.get_dbg_cy())
}


/// Hold the reader's place across a model mutation.
///
/// The list recalculates its own offset from the average height of the rows it
/// has instantiated, so any mutation can move the view even though nobody
/// scrolled. The offset is therefore re-asserted here and twice more while the
/// new rows are measured and laid out.
pub(crate) fn pin_offset_across_layout(_ui: &AppWindow, _held: Option<f32>) {
    // ScrollView over VerticalLayout measures real row heights and never
    // rewrites content-y when rows grow at the bottom. Re-setting content-y
    // here on every streamed token only fights active wheel scrolling.
}


/// Whether the window is allowed to move the view at all right now.
///
/// Two facts and nothing else: does the reader want to follow, and is a hand on
/// the bar. A timeout was tried here and it was wrong: it let the stream take
/// the view back half a second after the reader stopped, which is exactly when
/// they started reading.
#[must_use]
pub fn may_autoscroll(following: bool, dragging: bool) -> bool {
    following && !dragging
}


/// Move to the newest row because the reader asked for it: a send, or a new
/// chat. The latch is for corrections nobody asked for; it must never block the
/// reader's own request.
pub(crate) fn force_scroll_to_end(ui: &AppWindow) {
    ui.set_follow_bottom(true);
    ui.invoke_force_scroll_to_end();
}


pub(crate) fn scroll_to_end_when_ready(ui: &slint::Weak<AppWindow>) {
    let Some(current) = ui.upgrade() else { return };
    if !may_autoscroll(current.get_follow_bottom(), current.get_scrollbar_dragging()) {
        return;
    }
    current.invoke_scroll_to_end();
}


#[cfg(test)]
mod autoscroll_guard_tests {
    use super::may_autoscroll;

    #[test]
    fn a_hand_on_the_bar_stops_the_window_moving_the_view() {
        assert!(!may_autoscroll(true, true));
    }

    #[test]
    fn a_latch_that_is_off_stays_off_however_long_the_reader_pauses() {
        // There is deliberately no time input here: a paused reader is a reader,
        // and the version that took a duration was the bug: it re-armed itself
        // half a second after the scrolling stopped.
        assert!(!may_autoscroll(false, false));
        assert!(!may_autoscroll(false, true));
    }

    #[test]
    fn following_and_not_dragging_is_the_only_way_the_view_moves_itself() {
        assert!(may_autoscroll(true, false));
    }
}


#[cfg(test)]
mod scroll_guard_tests {
    use super::{may_autoscroll, should_hold_offset};

    #[test]
    fn the_offset_is_held_only_for_a_reader_who_is_not_following() {
        // scrolled up and not dragging: this is the reader's view, hold it
        assert!(should_hold_offset(false, false));
        // following: the end is the destination, a held offset would fight it
        assert!(!should_hold_offset(true, false));
        // a hand on the bar owns the offset outright
        assert!(!should_hold_offset(false, true));
        assert!(!should_hold_offset(true, true));
    }

    #[test]
    fn holding_and_following_are_exact_opposites_for_a_still_hand() {
        for following in [true, false] {
            assert_ne!(
                should_hold_offset(following, false),
                may_autoscroll(following, false),
                "a mutation must not both hold the offset and scroll to the end"
            );
        }
    }

    #[test]
    fn a_hand_on_the_bar_neither_holds_nor_follows() {
        for following in [true, false] {
            assert!(!should_hold_offset(following, true));
            assert!(!may_autoscroll(following, true));
        }
    }
}
