//! Backward paging: when to ask for the page above, and where the reader's place goes after one arrives.

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
pub(crate) const TOP_SLACK: f32 = 2200.0;


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
    use super::{should_load_older, PagerState};

    /// A reader parked at the newest row has nothing above them to fetch.
    fn settled() -> PagerState {
        PagerState {
            follows_bottom: false,
            ready: true,
            loading: false,
            has_older: true,
            content_y: -3000.0,
            content_height: 2000.0,
            height: 600.0,
        }
    }

    #[test]
    fn a_page_is_fetched_only_at_the_top() {
        let mut state = settled();
        assert!(!should_load_older(state), "mid-transcript must not fetch");
        state.content_y = -2500.0;
        assert!(!should_load_older(state), "still below the slack band");
        state.content_y = -2200.0;
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
}
