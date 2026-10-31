//! The queue: the lanes the agent reports, projected onto the window's busy strip.

use crate::*;

pub(crate) fn apply_queue(
    ui: &AppWindow,
    steering: &[String],
    follow_up: &[String],
    sent_thumbs: &SentThumbs,
) {
    if crate::env_cfg::cfg().debug {
        eprintln!(
            "[queue] projection steering={} follow_up={steering:?}->{follow_up:?}",
            steering.len()
        );
    }
    let total = steering.len() + follow_up.len();
    ui.set_queued_count(total as i32);

    // Nothing here opens the panel: a strip that keeps a popup open after the
    // send is noise, so the count is what says something is waiting.
    //
    // Each row keeps its lane and its slot in that lane, which is what the
    // daemon wants back when one is edited or promoted.
    let lane_items = |texts: &[String]| -> Vec<QueueItem> {
        texts
            .iter()
            .enumerate()
            .map(|(slot, text)| {
                let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let cut: String = flat.chars().take(120).collect();
                // the picture this text went with, if it was sent from this window
                let thumb = sent_thumbs
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(sent, _)| sent.trim() == text.trim())
                    .map(|(_, thumb)| thumb.clone());
                QueueItem {
                    text: cut.into(),
                    raw: text.clone().into(),
                    slot: slot as i32,
                    thumb: thumb.unwrap_or_default(),
                }
            })
            .collect()
    };
    ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::from(lane_items(steering)))));
    ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::from(lane_items(follow_up)))));
}
