//! Backward paging: when to ask for the page above, and where the reader's place goes after one arrives.

#[allow(unused_imports)]
use crate::*;

/// Why the window would (or would not) ask for older history.
///
/// Kept as a value so the rule can be tested: the pager used to be a Slint
/// expression, where the only way to check it was to scroll a real window by
/// hand.
#[derive(Debug, Clone, Copy)]
pub struct PagerState {
    /// the reader is parked at the newest row (nothing to fetch above them)
    pub follows_bottom: bool,
    /// the transcript has settled since the last rebuild
    pub ready: bool,
    /// a page is already in flight
    pub loading: bool,
    /// the store said there is a page before this one
    pub has_older: bool,
    /// scroll offset of the list (negative once scrolled down)
    pub content_y: f32,
    /// full height of the list's content
    pub content_height: f32,
    /// height of the list itself
    pub height: f32,
}


/// How close to the top counts as "the reader reached the top".
pub(crate) const TOP_SLACK: f32 = 90.0;


/// Where the list must be scrolled after a page of older rows went in above the
/// viewport: the same rows stay under the reader's eye, no jump.
///
/// `anchor-y` is the offset before the page, and the two heights are the
/// content height before and after. Prepend only ever grows the content upwards,
/// so a shrink is treated as "nothing happened" rather than a jump.
#[must_use]
pub fn anchor_after_prepend(anchor_y: f32, height_before: f32, height_after: f32) -> f32 {
    let grown = (height_after - height_before).max(0.0);
    anchor_y - grown
}


#[must_use]
pub fn should_load_older(state: PagerState) -> bool {
    // nothing is above the newest row, so being at the bottom means nothing to do
    if state.follows_bottom {
        return false;
    }
    // the first frames of a chat sit at offset zero, which looks exactly like a
    // reader at the top; `ready` is what tells them apart
    if !state.ready || state.loading || !state.has_older {
        return false;
    }
    // a transcript shorter than the view cannot be scrolled, so it cannot have
    // been scrolled to the top either
    if state.content_height <= state.height {
        return false;
    }
    state.content_y >= -TOP_SLACK
}


#[cfg(test)]
mod pager_tests {
    use super::{anchor_after_prepend, prepend_rows, should_load_older, to_slint, PagerState, TRow};
    use crate::transcript::{self, Row};
    use slint::{Model, VecModel};
    use std::rc::Rc;

    fn row(message: &str, body: &str) -> Row {
        // the struct's fields are all public, and `Row::new` is the crate's own
        // constructor - a test only needs a key and a body
        let mut row = transcript::Row {
            kind: transcript::KIND_AGENT,
            id: String::new(),
            title: String::new(),
            meta: String::new(),
            body: String::new(),
            code: String::new(),
            output: String::new(),
            has_code: false,
            has_output: false,
            expanded: false,
            failed: false,
            reason: String::new(),
            image: String::new(),
            mime: String::new(),
            elapsed: String::new(),
            message: String::new(),
        };
        row.message = message.to_string();
        row.body = body.to_string();
        row
    }

    /// A reader parked at the newest row has nothing above them to fetch.
    fn settled() -> PagerState {
        PagerState {
            follows_bottom: false,
            ready: true,
            loading: false,
            has_older: true,
            content_y: -900.0,
            content_height: 2000.0,
            height: 600.0,
        }
    }

    #[test]
    fn a_page_is_fetched_only_at_the_top() {
        let mut state = settled();
        assert!(!should_load_older(state), "mid-transcript must not fetch");
        state.content_y = -120.0;
        assert!(!should_load_older(state), "still below the slack band");
        state.content_y = -90.0;
        assert!(should_load_older(state), "at the top: fetch");
        state.content_y = 0.0;
        assert!(should_load_older(state), "clamped at the very top");
    }

    #[test]
    fn nothing_is_fetched_when_there_is_nothing_to_fetch() {
        let mut state = settled();
        state.content_y = 0.0;
        // the reader is at the bottom, so nothing is above them
        state.follows_bottom = true;
        assert!(!should_load_older(state));
        // the store says this was the first page
        state.follows_bottom = false;
        state.has_older = false;
        assert!(!should_load_older(state));
        // a page is already on its way
        state.has_older = true;
        state.loading = true;
        assert!(!should_load_older(state));
        // the window has just opened and the list has not settled yet
        state.loading = false;
        state.ready = false;
        assert!(!should_load_older(state));
        // a short transcript cannot be scrolled at all
        state.ready = true;
        state.content_height = 400.0;
        assert!(!should_load_older(state));
    }

    #[test]
    fn the_reader_stays_on_the_row_they_were_reading() {
        // at the very top, a 1500 px page goes in above: the viewport follows it
        // down by 1500 px, so the words that were on screen are still on screen
        assert_eq!(anchor_after_prepend(0.0, 2000.0, 3500.0), -1500.0);
        // half way up: same rule, the anchor is simply deeper already
        assert_eq!(anchor_after_prepend(-900.0, 2000.0, 3500.0), -2400.0);
        // nothing grew (a page that carried no visible rows)
        assert_eq!(anchor_after_prepend(-900.0, 2000.0, 2000.0), -900.0);
        // a height that has not been re-measured yet must not jump the wrong way
        assert_eq!(anchor_after_prepend(-900.0, 2000.0, 1500.0), -900.0);
    }

    #[test]
    fn older_rows_go_above_and_the_model_follows() {
        let mut store = vec![row("m2", "second"), row("m3", "third")];
        let model = Rc::new(VecModel::<TRow>::from(
            store.iter().map(to_slint).collect::<Vec<_>>(),
        ));
        let older = vec![row("m0", "first"), row("m1", "middle")];
        prepend_rows(&mut store, &older, &model);

        let order: Vec<String> = store.iter().map(|row| row.message.clone()).collect();
        assert_eq!(order, ["m0", "m1", "m2", "m3"]);
        assert_eq!(model.row_count(), 4);
        assert_eq!(model.row_data(0).unwrap().kind, transcript::KIND_AGENT);
    }

    #[test]
    fn a_page_that_arrives_after_the_merge_still_keeps_its_rows() {
        // the poll runs while a page is in flight: the prepend must survive a
        // rebuild that does not mention the older messages
        let mut store = vec![row("m2", "second")];
        let model = Rc::new(VecModel::<TRow>::from(
            store.iter().map(to_slint).collect::<Vec<_>>(),
        ));
        prepend_rows(&mut store, &[row("m1", "first")], &model);
        super::merge_rows(&mut store, vec![row("m2", "second, edited")], &model);

        let order: Vec<String> = store.iter().map(|row| row.message.clone()).collect();
        assert_eq!(order, ["m1", "m2"], "the older page was dropped by the merge");
        assert_eq!(store[1].body, "second, edited");
        assert_eq!(model.row_count(), 2);
    }

    #[test]
    fn an_empty_page_changes_nothing() {
        let mut store = vec![row("m1", "one")];
        let model = Rc::new(VecModel::<TRow>::from(
            store.iter().map(to_slint).collect::<Vec<_>>(),
        ));
        prepend_rows(&mut store, &[], &model);
        assert_eq!(store.len(), 1);
        assert_eq!(model.row_count(), 1);
    }
}
