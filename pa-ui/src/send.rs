//! Sending: whether a send is allowed right now, and the reason when it is not.

#[allow(unused_imports)]
use crate::*;

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
