//! The chord that opens the search popup, kept here so it can be tested without a
//! window. It was wrong at some point in a way that made the popup look broken
//! rather than misbehave: Ctrl+K did nothing.
//!
//! The keys pressed once the popup is open are `key-role` in `ui/search.slint`,
//! because the field that receives them lives there.

/// Whether this key press opens the popup.
///
/// Ctrl+K is the chord. The toolkit reports the character a chord produces, which
/// for Ctrl+K is the vertical tab, so both spellings count. A press while the
/// popup is already open does not open it again: a second open would reset the
/// field the reader is typing in.
///
/// `search_open` is whether the popup is up. It gates the chord, never a
/// character: anything typed while the popup is up goes to its field, so nothing
/// may be turned away before the popup has been considered.
pub(crate) fn opens_search(text: &str, control: bool, search_open: bool) -> bool {
    if search_open {
        return false;
    }
    control && matches!(text, "k" | "K" | "\u{b}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctrl_k_opens_the_popup_from_a_closed_state() {
        assert!(opens_search("k", true, false));
        assert!(opens_search("K", true, false));
    }

    /// The toolkit reports a chord as the character it makes, which for Ctrl+K is
    /// the vertical tab rather than a k.
    #[test]
    fn the_vertical_tab_counts_as_ctrl_k() {
        assert!(opens_search("\u{b}", true, false));
    }

    /// A plain k is a letter the reader is typing, not the chord.
    #[test]
    fn a_bare_k_does_not_open_the_popup() {
        assert!(!opens_search("k", false, false));
        assert!(!opens_search("K", false, false));
    }

    #[test]
    fn another_chord_does_not_open_the_popup() {
        assert!(!opens_search("j", true, false));
        assert!(!opens_search("\u{a}", true, false));
    }

    /// While the popup is open its field holds the keyboard, so the chord must not
    /// open it a second time and reset what the reader has typed.
    #[test]
    fn the_chord_does_not_reopen_a_popup_that_is_already_up() {
        assert!(!opens_search("k", true, true));
        assert!(!opens_search("\u{b}", true, true));
    }
}
