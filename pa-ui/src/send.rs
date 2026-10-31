//! Sending: whether a send is allowed right now, and the reason when it is not.

/// Why an image send cannot go out right now, if it cannot.
///
/// Two rules, both learned from the daemon's behaviour:
/// - a model that cannot take images means the turn is dropped outright;
/// - an image sent while a turn is in flight is folded into that running turn,
///   and the picture is lost on the way (text survives, images do not).
pub(crate) fn image_send_blocker(has_images: bool, model_capable: bool, turn_in_flight: bool) -> Option<&'static str> {
    if !has_images {
        return None;
    }
    if !model_capable {
        return Some("this model cannot take images - switch to a vision model with /model, then send");
    }
    // Mid-turn is allowed: the daemon parks the prompt in the follow-up lane and
    // delivers it, images and all, as the probe `queued_image_prompt_lands`
    // shows. Refusing here just made the send look broken.
    let _ = turn_in_flight;
    None
}


/// The words a send carries: the command word that sits in front of the caret
/// (`/compact`, taken from the menu) and whatever the reader typed after it.
///
/// The box and the command word are two halves of one message. Reading only the
/// box is what made Enter on a bare `/compact` - and the Send button with it -
/// look dead: the box is empty, the command word is not.
pub(crate) fn compose_message(command_word: &str, typed: &str) -> String {
    let typed = typed.trim();
    if command_word.is_empty() {
        typed.to_string()
    } else if typed.is_empty() {
        command_word.to_string()
    } else {
        format!("{command_word} {typed}")
    }
}


#[cfg(test)]
mod compose_message_tests {
    use super::compose_message;

    #[test]
    fn a_command_alone_is_the_whole_message() {
        // the chip's word with nothing typed after it still has to go out
        assert_eq!(compose_message("/compact", ""), "/compact");
        assert_eq!(compose_message("/new", "   "), "/new");
    }

    #[test]
    fn a_command_keeps_the_words_typed_after_it() {
        assert_eq!(compose_message("/refine", "tighten the notes"), "/refine tighten the notes");
        // the box is trimmed, so a stray space does not become a doubled one
        assert_eq!(compose_message("/name", "  plan the week  "), "/name plan the week");
    }

    #[test]
    fn prose_goes_out_as_typed() {
        assert_eq!(compose_message("", "  hello there  "), "hello there");
        assert_eq!(compose_message("", ""), "");
    }
}


#[cfg(test)]
mod image_send_tests {
    use super::image_send_blocker;

    #[test]
    fn text_only_is_never_blocked() {
        assert_eq!(image_send_blocker(false, false, true), None);
    }

    #[test]
    fn images_with_a_blind_model_are_refused() {
        assert!(image_send_blocker(true, false, false).is_some());
        assert!(image_send_blocker(true, false, true).is_some());
    }

    #[test]
    fn images_mid_turn_queue_rather_than_refuse() {
        assert_eq!(image_send_blocker(true, true, true), None);
    }

    #[test]
    fn images_to_a_vision_model_between_turns_go_out() {
        assert_eq!(image_send_blocker(true, true, false), None);
    }
}
