//! The sheet: the rows the reader sees, and the virtualized window that draws them.

use crate::*;
use std::cell::Ref;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Reset { rows: usize },
    Set { at: usize },
    Insert { at: usize },
    Remove { at: usize },
}

pub(crate) trait Redraw {
    fn reset(&mut self, rows: Vec<TRow>);
    fn set(&mut self, at: usize, row: TRow);
    fn insert(&mut self, at: usize, row: TRow);
    fn remove(&mut self, at: usize);
}

const MAX_MOUNTED_ROWS: usize = 72;
const OVERSCAN_ROWS: usize = 20;
const SHIFT_STEP: usize = 6;

/// Accurate height estimator matched to `TranscriptRow` in `app.slint`.
fn estimate_row_height(row: &Row) -> f32 {
    match row.kind {
        // User bubble: padding 6px top + 6px bottom + inner 8px top + 8px bottom + 13px font (~18px line)
        0 => {
            let lines = row
                .body
                .lines()
                .map(|l| (l.chars().count() / 62) + 1)
                .sum::<usize>()
                .max(1);
            (lines as f32 * 18.5) + 28.0
        }
        // Assistant prose: 12px padding + 13px StyledText (~19.5px per line + paragraph spacing)
        1 => {
            let lines = row
                .body
                .lines()
                .map(|l| {
                    if l.trim().is_empty() {
                        1
                    } else {
                        (l.chars().count() / 82) + 1
                    }
                })
                .sum::<usize>()
                .max(1);
            (lines as f32 * 19.5) + 12.0
        }
        // Thinking block: 24px bar + 8px padding
        2 => {
            if row.expanded {
                let lines = row.body.lines().count().clamp(1, 6);
                32.0 + 6.0 + (lines as f32 * 16.0) + 24.0
            } else {
                32.0
            }
        }
        // Tool calls / notices
        _ => {
            if row.expanded {
                let code_lines = row.code.lines().count().min(12);
                let out_lines = row.output.lines().count().min(12);
                32.0 + ((code_lines + out_lines) as f32 * 16.0) + 24.0
            } else {
                28.0
            }
        }
    }
}

enum ModelPatch {
    None,
    Full(Vec<TRow>),
    SlideUp {
        remove_end: usize,
        prepend_rows: Vec<TRow>,
    },
    SlideDown {
        remove_start: usize,
        append_rows: Vec<TRow>,
    },
}

struct VirtualSlintRows {
    model: Rc<VecModel<TRow>>,
    ui: Option<slint::Weak<AppWindow>>,
    all_rows: Vec<TRow>,
    heights: Vec<f32>,
    measured: Vec<bool>,
    win_start: usize,
    win_end: usize,
    last_scroll_y: f32,
    last_viewport_h: f32,
    in_slint_call: bool,
    content_dirty: bool,
}

impl VirtualSlintRows {
    fn new(model: Rc<VecModel<TRow>>) -> Self {
        Self {
            model,
            ui: None,
            all_rows: Vec::new(),
            heights: Vec::new(),
            measured: Vec::new(),
            win_start: 0,
            win_end: 0,
            last_scroll_y: 0.0,
            last_viewport_h: 800.0,
            in_slint_call: false,
            content_dirty: false,
        }
    }

    fn compute_spacers(&self) -> (f32, f32, i32) {
        let top: f32 = self.heights[..self.win_start.min(self.heights.len())].iter().sum();
        let bottom: f32 = self.heights[self.win_end.min(self.heights.len())..].iter().sum();
        (top, bottom, self.win_start as i32)
    }
}

fn apply_virtual_update(vstate: &Rc<RefCell<VirtualSlintRows>>, force: bool) {
    let (ui_weak, model, patch, top, bottom, win_start) = {
        let Ok(mut vs) = vstate.try_borrow_mut() else {
            return;
        };
        if vs.in_slint_call {
            return;
        }
        let total = vs.all_rows.len();
        if total == 0 {
            vs.win_start = 0;
            vs.win_end = 0;
            vs.in_slint_call = true;
            let m = vs.model.clone();
            let u = vs.ui.clone();
            drop(vs);
            if m.row_count() > 0 {
                m.set_vec(Vec::new());
            }
            if let Some(ui) = u.and_then(|w| w.upgrade()) {
                ui.set_top_pad(0.0);
                ui.set_bottom_pad(0.0);
                ui.set_window_start(0);
            }
            if let Ok(mut vs) = vstate.try_borrow_mut() {
                vs.in_slint_call = false;
            }
            return;
        }

        if vs.ui.is_none() {
            vs.win_start = 0;
            vs.win_end = total;
            let all = vs.all_rows.clone();
            let m = vs.model.clone();
            drop(vs);
            m.set_vec(all);
            return;
        }

        let follow_bottom = vs
            .ui
            .as_ref()
            .and_then(|w| w.upgrade())
            .map(|u| u.get_follow_bottom() && !u.get_scrollbar_dragging())
            .unwrap_or(true);

        let (target_start, target_end) = if total <= MAX_MOUNTED_ROWS {
            (0, total)
        } else if follow_bottom {
            (total.saturating_sub(MAX_MOUNTED_ROWS), total)
        } else {
            let scroll_top = (-vs.last_scroll_y).max(0.0);
            let scroll_bottom = scroll_top + vs.last_viewport_h.max(600.0);

            let mut acc = 0.0f32;
            let mut first_vis = 0usize;
            let mut last_vis = total.saturating_sub(1);
            let mut found_first = false;

            for (i, &h) in vs.heights.iter().enumerate() {
                let row_bottom = acc + h;
                if !found_first && row_bottom >= scroll_top {
                    first_vis = i;
                    found_first = true;
                }
                if acc <= scroll_bottom {
                    last_vis = i;
                } else if found_first {
                    break;
                }
                acc = row_bottom;
            }
            if !found_first {
                first_vis = total.saturating_sub(1);
            }

            let start = first_vis.saturating_sub(OVERSCAN_ROWS);
            let end = (last_vis + 1 + OVERSCAN_ROWS).min(total);
            let span = end.saturating_sub(start);
            if span < MAX_MOUNTED_ROWS {
                let extra = MAX_MOUNTED_ROWS - span;
                let s = start.saturating_sub(extra / 2);
                let e = (s + MAX_MOUNTED_ROWS).min(total);
                let s = e.saturating_sub(MAX_MOUNTED_ROWS);
                (s, e)
            } else {
                (start, (start + MAX_MOUNTED_ROWS).min(total))
            }
        };

        let old_start = vs.win_start;
        let old_end = vs.win_end;
        let dirty = vs.content_dirty;
        vs.content_dirty = false;
        let need_shift = force
            || dirty
            || old_end <= old_start
            || old_end > total
            || target_start.abs_diff(old_start) >= SHIFT_STEP
            || target_end.abs_diff(old_end) >= SHIFT_STEP
            || (target_end == total && old_end != total)
            || (target_start == 0 && old_start != 0);

        let patch = if !need_shift {
            ModelPatch::None
        } else if !force
            && !dirty
            && old_end > old_start
            && old_end <= total
            && vs.model.row_count() == (old_end - old_start)
            && target_start < old_start
            && target_end > old_start
            && target_end <= old_end
            && (old_start - target_start) <= 16
        {
            // Incremental slide UP: keep the overlapping rows untouched in Slint!
            let remove_end = old_end - target_end;
            let prepend_rows = vs.all_rows[target_start..old_start].to_vec();
            vs.win_start = target_start;
            vs.win_end = target_end;
            ModelPatch::SlideUp {
                remove_end,
                prepend_rows,
            }
        } else if !force
            && !dirty
            && old_end > old_start
            && old_end <= total
            && vs.model.row_count() == (old_end - old_start)
            && target_start >= old_start
            && target_start < old_end
            && target_end > old_end
            && (target_end - old_end) <= 16
        {
            // Incremental slide DOWN: keep the overlapping rows untouched in Slint!
            let remove_start = target_start - old_start;
            let append_rows = vs.all_rows[old_end..target_end].to_vec();
            vs.win_start = target_start;
            vs.win_end = target_end;
            ModelPatch::SlideDown {
                remove_start,
                append_rows,
            }
        } else {
            vs.win_start = target_start;
            vs.win_end = target_end;
            ModelPatch::Full(vs.all_rows[vs.win_start..vs.win_end].to_vec())
        };

        let (top, bottom, wstart) = vs.compute_spacers();
        vs.in_slint_call = true;
        (vs.ui.clone(), vs.model.clone(), patch, top, bottom, wstart)
    };

    if let Some(ui) = ui_weak.as_ref().and_then(|w| w.upgrade()) {
        if ui.get_window_start() != win_start {
            ui.set_window_start(win_start);
        }
        if (ui.get_top_pad() - top).abs() > 0.5 {
            ui.set_top_pad(top);
        }
        if (ui.get_bottom_pad() - bottom).abs() > 0.5 {
            ui.set_bottom_pad(bottom);
        }
    }

    match patch {
        ModelPatch::None => {}
        ModelPatch::Full(slice) => {
            if model.row_count() == slice.len() {
                for (i, item) in slice.into_iter().enumerate() {
                    model.set_row_data(i, item);
                }
            } else {
                model.set_vec(slice);
            }
        }
        ModelPatch::SlideUp {
            remove_end,
            prepend_rows,
        } => {
            for _ in 0..remove_end {
                let last = model.row_count().saturating_sub(1);
                model.remove(last);
            }
            for (i, row) in prepend_rows.into_iter().enumerate() {
                model.insert(i, row);
            }
        }
        ModelPatch::SlideDown {
            remove_start,
            append_rows,
        } => {
            for _ in 0..remove_start {
                if model.row_count() > 0 {
                    model.remove(0);
                }
            }
            for row in append_rows {
                model.push(row);
            }
        }
    }

    let (ui_weak2, top2, bottom2, wstart2) = {
        let Ok(mut vs) = vstate.try_borrow_mut() else { return };
        vs.in_slint_call = false;
        let (t, b, w) = vs.compute_spacers();
        (vs.ui.clone(), t, b, w)
    };
    if let Some(ui) = ui_weak2.and_then(|w| w.upgrade()) {
        if ui.get_window_start() != wstart2 {
            ui.set_window_start(wstart2);
        }
        if (ui.get_top_pad() - top2).abs() > 0.5 {
            ui.set_top_pad(top2);
        }
        if (ui.get_bottom_pad() - bottom2).abs() > 0.5 {
            ui.set_bottom_pad(bottom2);
        }
    }
}

pub(crate) struct Sheet {
    inner: RefCell<Inner>,
    vstate: Rc<RefCell<VirtualSlintRows>>,
}

struct Inner {
    rows: Vec<Row>,
    heights: Vec<f32>,
    measured: Vec<bool>,
    last_prepend_px: f32,
    index: std::collections::HashMap<String, std::ops::Range<usize>>,
    open: Option<String>,
    drawn: Box<dyn Redraw>,
    virtual_state: Option<Rc<RefCell<VirtualSlintRows>>>,
}

impl Sheet {
    pub fn new() -> (Self, ModelRc<TRow>) {
        let model = Rc::new(VecModel::<TRow>::default());
        let vstate = Rc::new(RefCell::new(VirtualSlintRows::new(model.clone())));
        let sheet = Self {
            inner: RefCell::new(Inner {
                rows: Vec::new(),
                heights: Vec::new(),
                measured: Vec::new(),
                last_prepend_px: 0.0,
                index: std::collections::HashMap::new(),
                open: None,
                drawn: Box::new(SharedVirtualAdapter(vstate.clone())),
                virtual_state: Some(vstate.clone()),
            }),
            vstate,
        };
        (sheet, ModelRc::from(model))
    }

    pub fn bind_ui(self: &Rc<Self>, ui: &AppWindow) {
        if let Ok(mut vs) = self.vstate.try_borrow_mut() {
            vs.ui = Some(ui.as_weak());
        }
        let sheet_scroll = self.clone();
        ui.on_viewport_scrolled(move |cy, vh| {
            sheet_scroll.on_scroll(cy, vh);
        });
        let sheet_meas = self.clone();
        ui.on_row_measured(move |idx, h| {
            sheet_meas.on_row_measured(idx as usize, h);
        });
    }

    pub fn last_prepend_height(&self) -> f32 {
        self.inner.try_borrow().map(|i| i.last_prepend_px).unwrap_or(0.0)
    }

    pub fn on_scroll(&self, content_y: f32, viewport_h: f32) {
        {
            let Ok(mut vs) = self.vstate.try_borrow_mut() else {
                return;
            };
            if vs.in_slint_call {
                return;
            }
            vs.last_scroll_y = content_y;
            if viewport_h > 50.0 {
                vs.last_viewport_h = viewport_h;
            }
        }
        apply_virtual_update(&self.vstate, false);
    }

    pub fn on_row_measured(&self, at: usize, height: f32) {
        if height <= 1.0 {
            return;
        }
        if let Ok(mut inner) = self.inner.try_borrow_mut() {
            if at < inner.heights.len() {
                inner.heights[at] = height;
                inner.measured[at] = true;
            }
        }
        let sync_info = {
            let Ok(mut vs) = self.vstate.try_borrow_mut() else {
                return;
            };
            if at >= vs.heights.len() {
                return;
            }
            let old = vs.heights[at];
            let was_measured = vs.measured[at];
            let delta = height - old;
            vs.heights[at] = height;
            vs.measured[at] = true;
            if delta.abs() <= 0.5 {
                None
            } else {
                // Check if this newly-measured row sits ABOVE the visible viewport
                // (e.g. in the top overscan buffer as the user scrolls up).
                // If so, shift content-y by `-delta` so the visible rows on screen don't jump!
                let row_bottom_before: f32 = vs.heights[..at].iter().sum::<f32>() + old;
                let scroll_top = (-vs.last_scroll_y).max(0.0);
                let compensate_scroll = !was_measured && row_bottom_before <= scroll_top + 4.0;
                if compensate_scroll {
                    vs.last_scroll_y -= delta;
                }
                if vs.in_slint_call {
                    Some((vs.ui.clone(), None, compensate_scroll, delta))
                } else {
                    let (top, bottom, wstart) = vs.compute_spacers();
                    Some((vs.ui.clone(), Some((top, bottom, wstart)), compensate_scroll, delta))
                }
            }
        };
        if let Some((ui_weak, spacers, compensate_scroll, delta)) = sync_info {
            if let Some(ui) = ui_weak.and_then(|w| w.upgrade()) {
                if let Some((top, bottom, wstart)) = spacers {
                    if ui.get_window_start() != wstart {
                        ui.set_window_start(wstart);
                    }
                    if (ui.get_top_pad() - top).abs() > 0.5 {
                        ui.set_top_pad(top);
                    }
                    if (ui.get_bottom_pad() - bottom).abs() > 0.5 {
                        ui.set_bottom_pad(bottom);
                    }
                }
                if compensate_scroll && !ui.get_follow_bottom() && !ui.get_scrollbar_dragging() {
                    if let Ok(mut vs) = self.vstate.try_borrow_mut() {
                        vs.in_slint_call = true;
                    }
                    let cur_y = ui.get_dbg_cy();
                    ui.invoke_set_scroll_y(cur_y - delta);
                    if let Ok(mut vs) = self.vstate.try_borrow_mut() {
                        vs.in_slint_call = false;
                    }
                }
            }
        }
    }

    pub fn rows(&self) -> Ref<'_, Vec<Row>> {
        Ref::map(self.inner.borrow(), |inner| &inner.rows)
    }

    pub fn replace(&self, rows: Vec<Row>) {
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            inner.index.clear();
            inner.open = None;
            inner.last_prepend_px = 0.0;
            for (offset, row) in rows.iter().enumerate() {
                let key = row.message.clone();
                if key.is_empty() {
                    continue;
                }
                match inner.index.get_mut(&key) {
                    Some(range) => range.end = offset + 1,
                    None => {
                        inner.index.insert(key, offset..offset + 1);
                    }
                }
            }
            inner.heights = rows.iter().map(estimate_row_height).collect();
            inner.measured = vec![false; rows.len()];
            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights = inner.heights.clone();
                    vs.measured = inner.measured.clone();
                }
            }
            let trows: Vec<TRow> = rows.iter().map(to_slint).collect();
            inner.rows = rows;
            inner.drawn.reset(trows);
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, true);
        }
    }

    pub fn merge(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            for (key, range) in group_by_message(&page) {
                let fresh = &page[range];
                if key.is_empty() {
                    inner.place(fresh);
                    continue;
                }
                match inner.index.get(&key).cloned() {
                    Some(current) => inner.patch(&key, current, fresh),
                    None => inner.place_keyed(&key, fresh),
                }
            }
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, false);
        }
    }

    pub fn prepend(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            let fresh: Vec<(String, std::ops::Range<usize>)> = group_by_message(&page)
                .into_iter()
                .filter(|(key, _)| !inner.index.contains_key(key))
                .collect();
            let total: usize = fresh.iter().map(|(_, range)| range.len()).sum();
            if total == 0 {
                inner.last_prepend_px = 0.0;
                return;
            }
            for range in inner.index.values_mut() {
                range.start += total;
                range.end += total;
            }

            let mut added_px = 0.0f32;
            let mut new_rows = Vec::with_capacity(total);
            let mut new_heights = Vec::with_capacity(total);
            let mut new_trows = Vec::with_capacity(total);
            let mut at = 0usize;
            for (key, range) in fresh {
                let len = range.len();
                for row in &page[range] {
                    let est = estimate_row_height(row);
                    added_px += est;
                    new_rows.push(row.clone());
                    new_heights.push(est);
                    new_trows.push(to_slint(row));
                    at += 1;
                }
                inner.index.insert(key, at - len..at);
            }
            inner.last_prepend_px = added_px;

            inner.rows.splice(0..0, new_rows);
            inner.heights.splice(0..0, new_heights.clone());
            inner.measured.splice(0..0, std::iter::repeat_n(false, total));

            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights.splice(0..0, new_heights);
                    vs.measured.splice(0..0, std::iter::repeat_n(false, total));
                    vs.all_rows.splice(0..0, new_trows);
                    vs.win_start += total;
                    vs.win_end += total;
                    vs.last_scroll_y -= added_px;
                }
                Some(vs_rc.clone())
            } else {
                for (i, trow) in new_trows.into_iter().enumerate() {
                    inner.drawn.insert(i, trow);
                }
                None
            }
        };
        if let Some(vs_rc) = vstate {
            let (ui_weak, top, bottom, wstart) = {
                let vs = vs_rc.borrow();
                let (t, b, w) = vs.compute_spacers();
                (vs.ui.clone(), t, b, w)
            };
            if let Some(ui) = ui_weak.and_then(|w| w.upgrade()) {
                ui.set_window_start(wstart);
                ui.set_top_pad(top);
                ui.set_bottom_pad(bottom);
            }
        }
    }

    pub fn answer_began(&self, key: &str) {
        if !key.is_empty() {
            self.inner.borrow_mut().open = Some(key.to_string());
        }
    }

    pub fn answer_ended(&self) {
        self.inner.borrow_mut().open = None;
    }

    pub fn toggle(&self, at: usize) {
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            let Some(row) = inner.rows.get_mut(at) else { return };
            row.expanded = !row.expanded;
            let est = estimate_row_height(row);
            let drawn = to_slint(row);
            if at < inner.heights.len() {
                inner.heights[at] = est;
                inner.measured[at] = false;
            }
            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    if at < vs.heights.len() {
                        vs.heights[at] = est;
                        vs.measured[at] = false;
                    }
                }
            }
            inner.drawn.set(at, drawn);
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, false);
        }
    }

    pub fn has_rows_above(&self, newest_page: &str) -> bool {
        self.inner
            .borrow()
            .index
            .get(newest_page)
            .is_some_and(|range| range.start > 0)
    }

    pub fn trim_to_tail(&self, newest_page: &str) -> usize {
        let (dropped, vstate) = {
            let mut inner = self.inner.borrow_mut();
            let Some(from) = inner.index.get(newest_page).map(|range| range.start) else {
                return 0;
            };
            if from == 0 {
                return 0;
            }
            inner.rows.drain(0..from);
            let h_end = from.min(inner.heights.len());
            inner.heights.drain(0..h_end);
            let m_end = from.min(inner.measured.len());
            inner.measured.drain(0..m_end);
            inner.index = reindex(&inner.rows);

            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    let r_end = from.min(vs.all_rows.len());
                    vs.all_rows.drain(0..r_end);
                    let vh_end = from.min(vs.heights.len());
                    vs.heights.drain(0..vh_end);
                    let vm_end = from.min(vs.measured.len());
                    vs.measured.drain(0..vm_end);
                    vs.win_start = vs.win_start.saturating_sub(from);
                    vs.win_end = vs.win_end.saturating_sub(from);
                }
            } else {
                for at in (0..from).rev() {
                    inner.drawn.remove(at);
                }
            }
            (from, inner.virtual_state.clone())
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, true);
        }
        dropped
    }

    pub fn take_back_prompt(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let echo = serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": text }],
        });
        let Some(candidate) = transcript::build(std::slice::from_ref(&echo), 1).0.into_iter().next()
        else {
            return;
        };
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            let Some(last) = inner.rows.last() else { return };
            if last.kind != candidate.kind || last.body != candidate.body {
                return;
            }
            let at = inner.rows.len() - 1;
            let key = inner.rows[at].message.clone();
            inner.rows.remove(at);
            if at < inner.heights.len() {
                inner.heights.remove(at);
                inner.measured.remove(at);
            }
            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    if at < vs.heights.len() {
                        vs.heights.remove(at);
                        vs.measured.remove(at);
                    }
                }
            }
            inner.drawn.remove(at);
            let spread = inner.index.get(&key).map(|range| (range.start, range.end));
            match spread {
                Some((start, end)) if end > start + 1 => {
                    if let Some(range) = inner.index.get_mut(&key) {
                        range.end -= 1;
                    }
                }
                Some(_) => {
                    inner.index.remove(&key);
                }
                None => {}
            }
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, true);
        }
    }
}

struct SharedVirtualAdapter(Rc<RefCell<VirtualSlintRows>>);

impl Redraw for SharedVirtualAdapter {
    fn reset(&mut self, rows: Vec<TRow>) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            vs.all_rows = rows;
            vs.content_dirty = true;
            vs.win_start = 0;
            vs.win_end = 0;
        }
    }
    fn set(&mut self, at: usize, row: TRow) {
        let mut model_update = None;
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            if at < vs.all_rows.len() {
                vs.all_rows[at] = row.clone();
                if at >= vs.win_start && at < vs.win_end {
                    model_update = Some((vs.model.clone(), at - vs.win_start, row));
                }
            }
        }
        if let Some((model, idx, r)) = model_update {
            model.set_row_data(idx, r);
        }
    }
    fn insert(&mut self, at: usize, row: TRow) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            let pos = at.min(vs.all_rows.len());
            vs.all_rows.insert(pos, row);
            vs.content_dirty = true;
        }
    }
    fn remove(&mut self, at: usize) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            if at < vs.all_rows.len() {
                vs.all_rows.remove(at);
                vs.content_dirty = true;
            }
        }
    }
}

impl Inner {
    fn landing(&self) -> usize {
        self.open
            .as_ref()
            .and_then(|key| self.index.get(key))
            .map_or(self.rows.len(), |range| range.start)
    }

    fn insert_rows(&mut self, at: usize, rows: &[Row]) {
        for (offset, row) in rows.iter().enumerate() {
            let est = estimate_row_height(row);
            self.rows.insert(at + offset, row.clone());
            self.heights.insert(at + offset, est);
            self.measured.insert(at + offset, false);
            if let Some(vs_rc) = &self.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights.insert(at + offset, est);
                    vs.measured.insert(at + offset, false);
                }
            }
            self.drawn.insert(at + offset, to_slint(row));
        }
    }

    fn place(&mut self, rows: &[Row]) {
        let at = self.landing();
        self.insert_rows(at, rows);
        self.shift_from(at, rows.len() as isize);
    }

    fn place_keyed(&mut self, key: &str, rows: &[Row]) {
        let at = self.landing();
        self.insert_rows(at, rows);
        self.shift_from(at, rows.len() as isize);
        self.index.insert(key.to_string(), at..at + rows.len());
    }

    fn shift_from(&mut self, at: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for range in self.index.values_mut() {
            if range.start >= at {
                range.start = (range.start as isize + delta) as usize;
                range.end = (range.end as isize + delta) as usize;
            }
        }
    }

    fn patch(&mut self, key: &str, current: std::ops::Range<usize>, fresh: &[Row]) {
        let opened: std::collections::HashSet<String> = self.rows[current.clone()]
            .iter()
            .filter(|row| row.expanded)
            .map(row_identity)
            .collect();
        let adjusted: Vec<Row> = fresh
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.expanded = opened.contains(&row_identity(&row));
                row
            })
            .collect();

        let start = current.start;
        let had = current.len();
        let common = had.min(adjusted.len());
        for (offset, row) in adjusted.iter().enumerate().take(common) {
            let at = start + offset;
            if self.rows[at] == *row {
                continue;
            }
            let est = estimate_row_height(row);
            self.rows[at] = row.clone();
            if at < self.heights.len() && !self.measured[at] {
                self.heights[at] = est;
                if let Some(vs_rc) = &self.virtual_state {
                    if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                        if at < vs.heights.len() {
                            vs.heights[at] = est;
                        }
                    }
                }
            }
            self.drawn.set(at, to_slint(row));
        }
        if adjusted.len() > had {
            self.insert_rows(start + had, &adjusted[had..]);
            self.shift_from(start + 1, (adjusted.len() - had) as isize);
            self.index.insert(key.to_string(), start..start + adjusted.len());
        } else if adjusted.len() < had {
            for at in (start + adjusted.len()..start + had).rev() {
                self.rows.remove(at);
                if at < self.heights.len() {
                    self.heights.remove(at);
                    self.measured.remove(at);
                }
                if let Some(vs_rc) = &self.virtual_state {
                    if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                        if at < vs.heights.len() {
                            vs.heights.remove(at);
                            vs.measured.remove(at);
                        }
                    }
                }
                self.drawn.remove(at);
            }
            self.shift_from(start + 1, -((had - adjusted.len()) as isize));
            self.index.insert(key.to_string(), start..start + adjusted.len());
        }
    }
}

fn group_by_message(rows: &[Row]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match out.last_mut() {
            Some((key, range)) if *key == row.message => range.end = index + 1,
            _ => out.push((row.message.clone(), index..index + 1)),
        }
    }
    out
}

fn reindex(rows: &[Row]) -> std::collections::HashMap<String, std::ops::Range<usize>> {
    let mut out: std::collections::HashMap<String, std::ops::Range<usize>> =
        std::collections::HashMap::new();
    for (offset, row) in rows.iter().enumerate() {
        if row.message.is_empty() {
            continue;
        }
        match out.get_mut(&row.message) {
            Some(range) => range.end = offset + 1,
            None => {
                out.insert(row.message.clone(), offset..offset + 1);
            }
        }
    }
    out
}

fn row_identity(row: &Row) -> String {
    if !row.id.is_empty() {
        return format!("id:{}", row.id);
    }
    format!("{}#{}#{}", row.message, row.kind, row.title)
}

pub(crate) fn to_slint(row: &Row) -> TRow {
    TRow {
        kind: row.kind,
        title: transcript::plain(&row.title).into(),
        meta: transcript::plain(&row.meta).into(),
        body: transcript::styled(&row.body),
        code: transcript::styled(&row.code),
        output: transcript::styled(&row.output),
        has_code: row.has_code,
        has_output: row.has_output,
        expanded: row.expanded,
        failed: row.failed,
        reason: transcript::plain(&row.reason).into(),
        image: images::from_wire(&row.image),
        elapsed: row.elapsed.clone().into(),
    }
}

pub(crate) fn dump(rows: &[Row]) {
    let Some(path) = crate::env_cfg::cfg().dump.clone() else {
        return;
    };
    let dump: String = rows
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                row.kind,
                row.id,
                row.title.replace('\n', " "),
                row.meta.replace('\n', " "),
                row.elapsed.replace('\n', " "),
                row.body.replace('\n', " ").chars().take(90).collect::<String>()
            )
        })
        .collect();
    let _ = std::fs::write(path, dump);
}

pub(crate) fn trace(what: &str, rows: &[Row]) {
    if !crate::env_cfg::cfg().debug {
        return;
    }
    let tail: Vec<String> = rows
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|row| {
            let body: String = row.body.chars().take(18).collect();
            format!("{}:{:?}", row.kind, body)
        })
        .collect();
    eprintln!("[rows] {what:24} n={:<4} tail={:?}", rows.len(), tail);
}

#[cfg(test)]
mod tests;