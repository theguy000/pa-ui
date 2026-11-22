//! Composer state model: owns drag-resize geometry, slash selection index,
//! steering/follow-up queues, in-place queue editing, attachments, busy strip animation,
//! and Send <-> Stop morph state.
//!
//! Deep in-process module: private fields, small interface, testable directly at its seam.

use crate::composer::{
    busy_strip_geometry, resolve_busy_label, Attachment, COMPOSER_MAX_HEIGHT, COMPOSER_MIN_HEIGHT,
};
use std::time::Instant;

pub(crate) struct ComposerModel {
    height: Option<f32>,
    dragging: bool,
    drag_start_y: f32,
    drag_start_height: f32,
    handle_hovered: bool,
    slash_index: usize,
    queued_steer: Vec<String>,
    queued_follow_up: Vec<String>,
    editing_queue: Option<(String, usize, String)>,
    attachments: Vec<Attachment>,
    pending_slash_cmd: Option<&'static str>,
    busy_label: String,
    busy_since: Option<Instant>,
    busy_strip_mix: f32,
    stop_mix: f32,
    stop_target: f32,
    last_anim_tick: Instant,
}

impl ComposerModel {
    pub(crate) fn new() -> Self {
        Self {
            height: None,
            dragging: false,
            drag_start_y: 0.0,
            drag_start_height: COMPOSER_MIN_HEIGHT,
            handle_hovered: false,
            slash_index: 0,
            queued_steer: Vec::new(),
            queued_follow_up: Vec::new(),
            editing_queue: None,
            attachments: Vec::new(),
            pending_slash_cmd: None,
            busy_label: String::new(),
            busy_since: None,
            busy_strip_mix: 0.0,
            stop_mix: 0.0,
            stop_target: 0.0,
            last_anim_tick: Instant::now(),
        }
    }

    // --- Unified drag-resize geometry ---

    pub(crate) fn explicit_height(&self) -> Option<f32> {
        self.height
    }

    pub(crate) fn effective_height(&self, measured_inner_height: f32) -> f32 {
        self.height
            .unwrap_or_else(|| measured_inner_height.max(COMPOSER_MIN_HEIGHT))
    }

    pub(crate) fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub(crate) fn is_handle_hovered(&self) -> bool {
        self.handle_hovered
    }

    pub(crate) fn set_handle_hovered(&mut self, hovered: bool) {
        self.handle_hovered = hovered;
    }

    pub(crate) fn begin_drag(&mut self, mouse_y: f32, measured_inner_height: f32) {
        self.dragging = true;
        self.drag_start_y = mouse_y;
        self.drag_start_height = self.effective_height(measured_inner_height);
    }

    pub(crate) fn update_drag(&mut self, mouse_y: f32) -> bool {
        if !self.dragging {
            return false;
        }
        let dy = self.drag_start_y - mouse_y;
        let next = (self.drag_start_height + dy).clamp(COMPOSER_MIN_HEIGHT, COMPOSER_MAX_HEIGHT);
        self.height = Some(next);
        true
    }

    pub(crate) fn end_drag(&mut self) -> bool {
        if self.dragging {
            self.dragging = false;
            true
        } else {
            false
        }
    }

    pub(crate) fn reset_height(&mut self) {
        self.height = None;
        self.dragging = false;
    }

    // --- Slash completion index ---

    pub(crate) fn slash_index(&self) -> usize {
        self.slash_index
    }

    pub(crate) fn reset_slash_index(&mut self) {
        self.slash_index = 0;
    }

    pub(crate) fn clamp_slash_index(&mut self, match_count: usize) {
        if match_count == 0 {
            self.slash_index = 0;
        } else if self.slash_index >= match_count {
            self.slash_index = match_count - 1;
        }
    }

    pub(crate) fn step_slash_index(&mut self, delta: isize, match_count: usize) -> bool {
        if match_count == 0 {
            return false;
        }
        let len = match_count as isize;
        self.slash_index = ((self.slash_index as isize + delta).rem_euclid(len)) as usize;
        true
    }

    // --- Attachments ---

    pub(crate) fn attachments(&self) -> &[Attachment] {
        &self.attachments
    }

    pub(crate) fn push_attachments(&mut self, found: Vec<Attachment>) {
        self.attachments.extend(found);
    }

    pub(crate) fn remove_attachment(&mut self, idx: usize) -> bool {
        if idx < self.attachments.len() {
            self.attachments.remove(idx);
            true
        } else {
            false
        }
    }

    pub(crate) fn take_attachments(&mut self) -> Vec<Attachment> {
        std::mem::take(&mut self.attachments)
    }

    // --- Queues & in-place queue edit ---

    pub(crate) fn queued_steer(&self) -> &[String] {
        &self.queued_steer
    }

    pub(crate) fn queued_follow_up(&self) -> &[String] {
        &self.queued_follow_up
    }

    pub(crate) fn queued_total(&self) -> usize {
        self.queued_steer.len() + self.queued_follow_up.len()
    }

    pub(crate) fn set_queues(&mut self, steer: Vec<String>, follow_up: Vec<String>) {
        self.queued_steer = steer;
        self.queued_follow_up = follow_up;
    }

    pub(crate) fn clear_queues(&mut self) {
        self.queued_steer.clear();
        self.queued_follow_up.clear();
        self.editing_queue = None;
    }

    pub(crate) fn begin_edit_queue(&mut self, lane: String, slot: usize, expected: String) {
        self.editing_queue = Some((lane, slot, expected));
    }

    pub(crate) fn take_edit_queue(&mut self) -> Option<(String, usize, String)> {
        self.editing_queue.take()
    }

    pub(crate) fn cancel_edit_queue(&mut self) {
        self.editing_queue = None;
    }

    pub(crate) fn is_editing_queue(&self) -> bool {
        self.editing_queue.is_some()
    }

    // --- Busy strip & Send/Stop morph ---

    pub(crate) fn set_pending_slash(&mut self, cmd: Option<&'static str>) {
        self.pending_slash_cmd = cmd;
    }

    pub(crate) fn take_pending_slash(&mut self) -> Option<&'static str> {
        self.pending_slash_cmd.take()
    }

    pub(crate) fn busy_strip_wanted(&self, is_streaming: bool) -> bool {
        is_streaming || !self.queued_steer.is_empty() || !self.queued_follow_up.is_empty()
    }

    pub(crate) fn set_busy_label(&mut self, label: &str, force: bool) {
        let next = resolve_busy_label(self.pending_slash_cmd, label, force);
        if self.busy_label != next {
            self.busy_label = next.to_string();
            self.busy_since = Some(Instant::now());
        } else if self.busy_since.is_none() {
            self.busy_since = Some(Instant::now());
        }
    }

    pub(crate) fn clear_busy(&mut self) {
        self.pending_slash_cmd = None;
        self.busy_label.clear();
        self.busy_since = None;
    }

    pub(crate) fn busy_strip_metrics(&self, is_streaming: bool) -> (f32, f32, &str, u64) {
        let opening = self.busy_strip_wanted(is_streaming);
        let (h, op) = busy_strip_geometry(self.busy_strip_mix, opening);
        let label = if self.busy_label.is_empty() {
            "Working"
        } else {
            self.busy_label.as_str()
        };
        let elapsed_secs = self
            .busy_since
            .map(|since| since.elapsed().as_secs())
            .unwrap_or(0);
        (h, op, label, elapsed_secs)
    }

    pub(crate) fn has_busy_label(&self) -> bool {
        !self.busy_label.is_empty()
    }

    pub(crate) fn stop_mix(&self) -> f32 {
        self.stop_mix
    }

    pub(crate) fn is_stop_target(&self) -> bool {
        self.stop_target > 0.5
    }

    pub(crate) fn sync_stop_target(&mut self, is_streaming: bool, composer_empty: bool) {
        if is_streaming {
            if self.busy_label.is_empty() {
                self.set_busy_label("Working", false);
            } else if self.busy_since.is_none() {
                self.busy_since = Some(Instant::now());
            }
        } else {
            self.busy_label.clear();
            self.busy_since = None;
        }
        let target = if is_streaming && composer_empty && self.attachments.is_empty() {
            1.0
        } else {
            0.0
        };
        if (target - self.stop_target).abs() > 0.001 {
            self.stop_target = target;
            self.last_anim_tick = Instant::now();
        }
    }

    /// Advance Send/Stop and busy-strip animations by elapsed wall time.
    /// Returns `true` if another animation frame should be requested.
    pub(crate) fn advance_animations(&mut self, is_streaming: bool) -> bool {
        let now = Instant::now();
        let dt = now
            .duration_since(self.last_anim_tick)
            .as_secs_f32()
            .min(0.05);
        self.last_anim_tick = now;

        let mut animating = false;
        let stop_diff = self.stop_target - self.stop_mix;
        if stop_diff.abs() > 0.005 {
            let step = (dt / 0.22).min(1.0);
            self.stop_mix = if stop_diff > 0.0 {
                (self.stop_mix + step).min(self.stop_target)
            } else {
                (self.stop_mix - step).max(self.stop_target)
            };
            animating = true;
        } else {
            self.stop_mix = self.stop_target;
        }

        let strip_target = if self.busy_strip_wanted(is_streaming) {
            1.0
        } else {
            0.0
        };
        let strip_diff = strip_target - self.busy_strip_mix;
        if strip_diff.abs() > 0.005 {
            let duration = if strip_diff > 0.0 { 0.19 } else { 0.11 };
            let step = (dt / duration).min(1.0);
            self.busy_strip_mix = if strip_diff > 0.0 {
                (self.busy_strip_mix + step).min(strip_target)
            } else {
                (self.busy_strip_mix - step).max(strip_target)
            };
            animating = true;
        } else {
            self.busy_strip_mix = strip_target;
        }

        animating
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_drag_starts_from_unified_min_height_without_jumping() {
        let mut model = ComposerModel::new();
        assert_eq!(model.effective_height(22.0), COMPOSER_MIN_HEIGHT);

        // Dragging upward by 10px from rest expands smoothly from COMPOSER_MIN_HEIGHT.
        model.begin_drag(500.0, 22.0);
        assert!(model.update_drag(490.0));
        assert_eq!(model.explicit_height(), Some(COMPOSER_MIN_HEIGHT + 10.0));

        // Dragging downward below minimum clamps cleanly at COMPOSER_MIN_HEIGHT.
        assert!(model.update_drag(900.0));
        assert_eq!(model.explicit_height(), Some(COMPOSER_MIN_HEIGHT));

        // Dragging far upward clamps at COMPOSER_MAX_HEIGHT.
        assert!(model.update_drag(0.0));
        assert_eq!(model.explicit_height(), Some(COMPOSER_MAX_HEIGHT));

        assert!(model.end_drag());
        assert!(!model.is_dragging());
    }

    #[test]
    fn composer_busy_label_respects_pending_slash_and_force_override() {
        let mut model = ComposerModel::new();
        model.set_pending_slash(Some("compact"));
        model.set_busy_label("Waiting", false);
        let (_, _, label, _) = model.busy_strip_metrics(true);
        assert_eq!(label, "Compacting");

        model.set_busy_label("Refining", true);
        let (_, _, label2, _) = model.busy_strip_metrics(true);
        assert_eq!(label2, "Refining");
    }

    #[test]
    fn composer_slash_index_wraps_and_clamps() {
        let mut model = ComposerModel::new();
        assert!(model.step_slash_index(1, 3));
        assert_eq!(model.slash_index(), 1);
        assert!(model.step_slash_index(-2, 3));
        assert_eq!(model.slash_index(), 2);
        model.clamp_slash_index(1);
        assert_eq!(model.slash_index(), 0);
    }
}
