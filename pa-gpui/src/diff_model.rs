pub(crate) const DIFF_DEFAULT_WIDTH: f32 = 480.0;
pub(crate) const DIFF_MIN_WIDTH: f32 = 280.0;
pub(crate) const DIFF_MAX_WIDTH: f32 = 960.0;
pub(crate) const CENTER_MIN_WIDTH: f32 = 340.0;

pub(crate) fn clamp_diff_width(desired: f32, viewport_w: f32, sidebar_w: f32) -> f32 {
    if !desired.is_finite() {
        return DIFF_DEFAULT_WIDTH;
    }
    if viewport_w <= 0.0 {
        return desired.clamp(DIFF_MIN_WIDTH, DIFF_MAX_WIDTH);
    }
    let max_available = (viewport_w - sidebar_w - CENTER_MIN_WIDTH).min(DIFF_MAX_WIDTH);
    if max_available < DIFF_MIN_WIDTH {
        return max_available.max(180.0);
    }
    desired.clamp(DIFF_MIN_WIDTH, max_available)
}

pub(crate) struct DiffModel {
    width: f32,
    dragging: bool,
    drag_start_x: f32,
    drag_start_width: f32,
    handle_hovered: bool,
}

impl DiffModel {
    pub(crate) fn new(initial_width: f32) -> Self {
        let width = clamp_diff_width(initial_width, 0.0, 0.0);
        Self {
            width,
            dragging: false,
            drag_start_x: 0.0,
            drag_start_width: width,
            handle_hovered: false,
        }
    }

    pub(crate) fn width(&self) -> f32 {
        self.width
    }

    pub(crate) fn effective_width(&self, viewport_w: f32, sidebar_w: f32) -> f32 {
        clamp_diff_width(self.width, viewport_w, sidebar_w)
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

    pub(crate) fn begin_drag(&mut self, mouse_x: f32, viewport_w: f32, sidebar_w: f32) {
        self.dragging = true;
        self.drag_start_x = mouse_x;
        self.drag_start_width = self.effective_width(viewport_w, sidebar_w);
    }

    pub(crate) fn update_drag(&mut self, mouse_x: f32, viewport_w: f32, sidebar_w: f32) -> bool {
        if !self.dragging {
            return false;
        }
        let dx = self.drag_start_x - mouse_x;
        let next = clamp_diff_width(self.drag_start_width + dx, viewport_w, sidebar_w);
        if (next - self.width).abs() > f32::EPSILON {
            self.width = next;
            true
        } else {
            false
        }
    }

    pub(crate) fn end_drag(&mut self) -> Option<f32> {
        if self.dragging {
            self.dragging = false;
            Some(self.width)
        } else {
            None
        }
    }

    pub(crate) fn reset_width(&mut self) -> f32 {
        self.width = DIFF_DEFAULT_WIDTH;
        self.dragging = false;
        self.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn width_clamps_to_bounds_and_preserves_center_sheet_room() {
        assert_eq!(clamp_diff_width(480.0, 1400.0, 244.0), 480.0);
        assert_eq!(clamp_diff_width(120.0, 1400.0, 244.0), DIFF_MIN_WIDTH);
        assert_eq!(
            clamp_diff_width(1200.0, 1400.0, 244.0),
            1400.0 - 244.0 - CENTER_MIN_WIDTH
        );
        assert_eq!(clamp_diff_width(1200.0, 2000.0, 44.0), DIFF_MAX_WIDTH);
        assert_eq!(clamp_diff_width(480.0, 0.0, 244.0), 480.0);
        assert_eq!(clamp_diff_width(f32::NAN, 1400.0, 244.0), DIFF_DEFAULT_WIDTH);
        assert_eq!(clamp_diff_width(480.0, 500.0, 244.0), 180.0);
    }

    #[::core::prelude::v1::test]
    fn dragging_left_edge_widens_left_and_narrows_right_within_clamp() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);
        assert!(!model.is_dragging());
        model.begin_drag(900.0, 1400.0, 244.0);
        assert!(model.is_dragging());

        assert!(model.update_drag(800.0, 1400.0, 244.0));
        assert_eq!(model.width(), 580.0);

        assert!(model.update_drag(1300.0, 1400.0, 244.0));
        assert_eq!(model.width(), DIFF_MIN_WIDTH);

        assert_eq!(model.end_drag(), Some(DIFF_MIN_WIDTH));
        assert!(!model.is_dragging());
        assert_eq!(model.end_drag(), None);
    }

    #[::core::prelude::v1::test]
    fn reset_width_restores_default_and_cancels_drag() {
        let mut model = DiffModel::new(620.0);
        model.begin_drag(800.0, 1400.0, 244.0);
        assert_eq!(model.reset_width(), DIFF_DEFAULT_WIDTH);
        assert!(!model.is_dragging());
        assert_eq!(model.width(), DIFF_DEFAULT_WIDTH);
    }
}
