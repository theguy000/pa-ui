//! The command word a slash command gives to a running Turn.
//!
//! One owner for the whole lifecycle: the Composer records the word when it
//! sends an idle command, the Tick keeps it through stream activity, and both
//! clear it when the Turn ends or the send fails. Keeping the state here means
//! the window has one answer to "which command word is held?" instead of a
//! copy in the Composer and another in the Tick.

use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Word {
    Refining,
    Compacting,
}

impl Word {
    fn from_message(text: &str) -> Option<Self> {
        let body = text.trim().strip_prefix('/')?;
        let name = body.split_whitespace().next()?;
        match name {
            "compact" => Some(Self::Compacting),
            "refine" => Some(Self::Refining),
            _ => None,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Refining => "Refining",
            Self::Compacting => "Compacting",
        }
    }

    const fn failure_text(self) -> &'static str {
        match self {
            Self::Refining => "Refining failed",
            Self::Compacting => "Compacting failed",
        }
    }
}

/// The one owner of the command word held for the running Turn.
#[derive(Clone, Default)]
pub(crate) struct CommandWord {
    held: Rc<RefCell<Option<Word>>>,
}

impl CommandWord {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record the command word in `text`, if `text` starts with one.
    ///
    /// A message without a command word does not disturb the held word; the
    /// caller clears first when it wants a plain prompt to replace one.
    pub(crate) fn hold(&self, text: &str) -> Option<&'static str> {
        let word = Word::from_message(text);
        if let Some(word) = word {
            *self.held.borrow_mut() = Some(word);
        }
        word.map(Word::text)
    }

    pub(crate) fn clear(&self) {
        *self.held.borrow_mut() = None;
    }

    fn held_word(&self) -> Option<&'static str> {
        self.held.borrow().map(Word::text)
    }

    /// The word that outranks the stream for this daemon state, if any.
    ///
    /// Compaction outranks a slash command's word.
    pub(crate) fn word_for_state(&self, state: &str) -> Option<&'static str> {
        if state == "Compacting" {
            Some(Word::Compacting.text())
        } else {
            self.held_word()
        }
    }

    /// The operation name a failed command Turn carries into the Sheet. This
    /// is the single owner of the mapping: the Tick never compares the display
    /// words. An automatic compaction has no held word; its Turn failure stays
    /// the generic "Turn failed".
    pub(crate) fn failure_label(&self) -> Option<&'static str> {
        self.held.borrow().map(|word| word.failure_text())
    }

    /// The word the Status strip should show for the current state.
    ///
    /// Compaction outranks a slash command's word, which outranks the stream's
    /// generic activity word.
    pub(crate) fn displayed(&self, state: &str, activity: &str) -> String {
        self.word_for_state(state)
            .map_or_else(|| activity.to_string(), str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_leading_slash_command_names_a_word() {
        let word = CommandWord::new();
        assert_eq!(word.hold("/compact"), Some("Compacting"));
        assert_eq!(word.hold("/compact keep the plan"), Some("Compacting"));
        assert_eq!(word.hold("  /refine  "), Some("Refining"));
        assert_eq!(word.hold("/refine tighten the notes"), Some("Refining"));
        assert_eq!(word.hold("/model"), None);
        assert_eq!(word.hold("please /refine this"), None);
        // The window's dispatcher matches command names exactly; the word
        // follows the same rule instead of naming a message it will send as
        // plain prose.
        assert_eq!(word.hold("/Compact"), None);
    }

    #[test]
    fn the_word_holds_through_stream_activity_and_compaction_wins() {
        let word = CommandWord::new();
        word.hold("/refine");
        assert_eq!(word.displayed("Thinking", "Thinking"), "Refining");
        assert_eq!(word.displayed("Refining", "Writing"), "Refining");
        assert_eq!(word.displayed("Compacting", "Writing"), "Compacting");
    }

    #[test]
    fn a_failure_names_the_held_command() {
        let word = CommandWord::new();
        word.hold("/refine");
        assert_eq!(word.failure_label(), Some("Refining failed"));
        word.hold("/compact");
        assert_eq!(word.failure_label(), Some("Compacting failed"));
        word.clear();
        assert_eq!(word.failure_label(), None);
    }

    #[test]
    fn clear_returns_the_strip_to_the_stream_word() {
        let word = CommandWord::new();
        word.hold("/compact");
        word.clear();
        assert_eq!(word.displayed("Thinking", "Thinking"), "Thinking");
        assert_eq!(word.word_for_state("Thinking"), None);
    }
}
