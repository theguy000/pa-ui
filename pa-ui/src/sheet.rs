//! The sheet: the rows the reader sees, backward paging, idle floor eviction,
//! reader scroll guards, and the virtualized window that draws them.

use crate::*;
use std::cell::Ref;

#[cfg(test)]
thread_local! {
    static CONVERSION_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_conversion_count() {
    CONVERSION_COUNT.with(|c| c.set(0));
}

#[cfg(test)]
pub(crate) fn conversion_count() -> usize {
    CONVERSION_COUNT.with(|c| c.get())
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Reset { rows: usize },
    Set { at: usize },
    Insert { at: usize },
    Remove { at: usize },
}

pub(crate) trait Redraw {
    fn reset(&mut self, rows: &[Row]);
    fn set(&mut self, at: usize, row: &Row);
    fn insert(&mut self, at: usize, row: &Row);
    fn remove(&mut self, at: usize);
}

pub(crate) const MAX_MOUNTED_ROWS: usize = 72;
const OVERSCAN_ROWS: usize = 20;
const SHIFT_STEP: usize = 6;

/// How close to the top counts as "the reader reached the top".
pub(crate) const TOP_SLACK: f32 = 2200.0;

/// Why the window would (or would not) ask for older history.
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

#[must_use]
pub fn should_load_older(state: PagerState) -> bool {
    if state.follows_bottom {
        return false;
    }
    if !state.ready || state.loading || !state.has_older {
        return false;
    }
    if state.content_height <= state.height {
        return false;
    }
    state.content_y >= -TOP_SLACK
}

/// Whether the reader's view must be held still while rows change.
#[must_use]
pub fn should_hold_offset(following: bool, dragging: bool) -> bool {
    !following && !dragging
}

/// The reader's offset, if it is theirs to keep right now.
pub(crate) fn hold_offset(ui: &AppWindow) -> Option<f32> {
    if !should_hold_offset(ui.get_follow_bottom(), ui.get_scrollbar_dragging()) {
        return None;
    }
    Some(ui.get_dbg_cy())
}

/// Hold the reader's place across a model mutation.
pub(crate) fn pin_offset_across_layout(_ui: &AppWindow, _held: Option<f32>) {}

/// Whether the window is allowed to move the view at all right now.
#[must_use]
pub fn may_autoscroll(following: bool, dragging: bool) -> bool {
    following && !dragging
}

/// Follow the newest row only when the reader is already there.
pub(crate) fn scroll_if_parked(ui: &AppWindow) {
    if ui.get_follow_bottom() && !ui.get_scrollbar_dragging() {
        scroll_to_end_when_ready(&ui.as_weak());
    }
}

/// Move to the newest row because the reader asked for it: a send, or a new chat.
pub(crate) fn force_scroll_to_end(ui: &AppWindow) {
    ui.set_follow_bottom(true);
    ui.invoke_force_scroll_to_end();
}

pub(crate) fn scroll_to_end_when_ready(ui: &slint::Weak<AppWindow>) {
    let Some(current) = ui.upgrade() else { return };
    if !may_autoscroll(current.get_follow_bottom(), current.get_scrollbar_dragging()) {
        return;
    }
    current.invoke_scroll_to_end();
}

/// Accurate height estimator matched to `TranscriptRow` in `app.slint`.
pub(crate) fn estimate_row_height(row: &Row) -> f32 {
    match row.kind {
        0 => {
            let lines = row
                .body
                .lines()
                .map(|l| (l.chars().count() / 54) + 1)
                .sum::<usize>()
                .max(1);
            (lines as f32 * 21.5) + 28.0
        }
        1 => {
            let lines = row
                .body
                .lines()
                .map(|l| {
                    if l.trim().is_empty() {
                        1
                    } else {
                        (l.chars().count() / 71) + 1
                    }
                })
                .sum::<usize>()
                .max(1);
            (lines as f32 * 22.5) + 12.0
        }
        transcript::KIND_QUOTE => {
            let lines = row
                .body
                .lines()
                .map(|l| {
                    if l.trim().is_empty() {
                        1
                    } else {
                        (l.chars().count() / 68) + 1
                    }
                })
                .sum::<usize>()
                .max(1);
            (lines as f32 * 22.5) + 12.0
        }
        2 => {
            if row.expanded {
                let lines = row.body.lines().count().clamp(1, 6);
                32.0 + 6.0 + (lines as f32 * 16.0) + 24.0
            } else {
                32.0
            }
        }
        transcript::KIND_CUSTOM_MESSAGE | transcript::KIND_SUMMARY => {
            if !row.expanded || !row.has_body {
                34.0
            } else {
                let body = transcript::plain(&row.body);
                let lines = body
                    .lines()
                    .map(|line| {
                        if line.trim().is_empty() {
                            1
                        } else {
                            (line.chars().count() / 80) + 1
                        }
                    })
                    .sum::<usize>()
                    .max(1);
                60.0 + (lines as f32 * 19.0)
            }
        }
        9 => {
            // The error text box is 118px narrower than prose, and the row
            // carries markup; estimate from the plain words at that width.
            let body = transcript::plain(&row.body);
            let lines = body
                .lines()
                .map(|line| (line.chars().count() / 64) + 1)
                .sum::<usize>()
                .max(1);
            (lines as f32 * 21.5) + 8.0
        }
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
    all_rows: Vec<Row>,
    cached: Vec<Option<TRow>>,
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
            cached: Vec::new(),
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

    fn materialize_range(&mut self, range: std::ops::Range<usize>) -> Vec<TRow> {
        let mut out = Vec::with_capacity(range.len());
        for i in range {
            let trow = if let Some(cached) = &self.cached[i] {
                cached.clone()
            } else {
                let converted = to_slint(&self.all_rows[i]);
                self.cached[i] = Some(converted.clone());
                converted
            };
            out.push(trow);
        }
        out
    }

    fn evict_outside_window(&mut self) {
        for (i, slot) in self.cached.iter_mut().enumerate() {
            if i < self.win_start || i >= self.win_end {
                *slot = None;
            }
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
            let all = vs.materialize_range(0..total);
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
            let min_start = total.saturating_sub(MAX_MOUNTED_ROWS);
            let start = if !force
                && !vs.content_dirty
                && vs.win_end <= total
                && vs.win_start <= min_start
                && min_start - vs.win_start < SHIFT_STEP
            {
                vs.win_start
            } else {
                min_start
            };
            (start, total)
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
        } else {
            let p = if !force
                && !dirty
                && old_end > old_start
                && old_end <= total
                && vs.model.row_count() == (old_end - old_start)
                && target_start < old_start
                && target_end > old_start
                && target_end <= old_end
                && (old_start - target_start) <= 16
            {
                let remove_end = old_end - target_end;
                let prepend_rows = vs.materialize_range(target_start..old_start);
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
                let remove_start = target_start - old_start;
                let append_rows = vs.materialize_range(old_end..target_end);
                ModelPatch::SlideDown {
                    remove_start,
                    append_rows,
                }
            } else {
                ModelPatch::Full(vs.materialize_range(target_start..target_end))
            };
            vs.win_start = target_start;
            vs.win_end = target_end;
            vs.evict_outside_window();
            p
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

/// Paging & floor-eviction state owned inside the Sheet module.
pub(crate) struct Paging {
    pub tail_from: Option<String>,
    pub tail_cursor: Option<String>,
    pub tail_more: bool,
    pub oldest_cursor: Option<String>,
    pub has_older: bool,
    pub loading_older: bool,
    pub floor_since: Option<SystemTime>,
    pub history_rows: usize,
    pub tail_len: usize,
    pub tail_sig: Option<(u64, u64)>,
}

impl Paging {
    pub fn new(history_rows: usize) -> Self {
        Self {
            tail_from: None,
            tail_cursor: None,
            tail_more: false,
            oldest_cursor: None,
            has_older: false,
            loading_older: false,
            floor_since: None,
            history_rows,
            tail_len: 0,
            tail_sig: None,
        }
    }

    pub fn reset(&mut self) {
        self.tail_from = None;
        self.tail_cursor = None;
        self.tail_more = false;
        self.oldest_cursor = None;
        self.has_older = false;
        self.loading_older = false;
        self.floor_since = None;
        self.tail_len = 0;
        self.tail_sig = None;
    }
}

pub(crate) struct Sheet {
    inner: RefCell<Inner>,
    vstate: Rc<RefCell<VirtualSlintRows>>,
    paging: RefCell<Paging>,
}

struct Inner {
    rows: Vec<Row>,
    last_prepend_px: f32,
    index: std::collections::HashMap<String, std::ops::Range<usize>>,
    open: Option<String>,
    /// Where the next row that must follow the open answer lands. The open
    /// range itself never grows when a row is placed after it, so without this
    /// cursor every later row would land at the same boundary and reverse the
    /// arrival order.
    after_open_end: Option<usize>,
    drawn: Box<dyn Redraw>,
    virtual_state: Option<Rc<RefCell<VirtualSlintRows>>>,
}

impl Sheet {
    pub fn new() -> (Self, ModelRc<TRow>) {
        let history_rows: usize = std::env::var("PA_HISTORY")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1500);
        Self::with_history(history_rows)
    }

    pub fn with_history(history_rows: usize) -> (Self, ModelRc<TRow>) {
        let model = Rc::new(VecModel::<TRow>::default());
        let vstate = Rc::new(RefCell::new(VirtualSlintRows::new(model.clone())));
        let sheet = Self {
            inner: RefCell::new(Inner {
                rows: Vec::new(),
                last_prepend_px: 0.0,
                index: std::collections::HashMap::new(),
                open: None,
                after_open_end: None,
                drawn: Box::new(SharedVirtualAdapter(vstate.clone())),
                virtual_state: Some(vstate.clone()),
            }),
            vstate,
            paging: RefCell::new(Paging::new(history_rows)),
        };
        (sheet, ModelRc::from(model))
    }

    pub fn bind_ui(self: &Rc<Self>, ui: &AppWindow) {
        if let Ok(mut vs) = self.vstate.try_borrow_mut() {
            vs.ui = Some(ui.as_weak());
        }
        let sheet_scroll = self.clone();
        let scroll_ui = ui.as_weak();
        ui.on_viewport_scrolled(move |cy, vh| {
            sheet_scroll.on_scroll(cy, vh);
            if let Some(ui) = scroll_ui.upgrade() {
                ui.invoke_scroll_paging();
            }
        });
        let sheet_meas = self.clone();
        ui.on_row_measured(move |idx, h| {
            sheet_meas.on_row_measured(idx as usize, h);
        });
        let sheet_toggle = self.clone();
        ui.on_toggle_row(move |index| {
            sheet_toggle.toggle(index as usize);
        });
    }

    pub fn reset_paging(&self, ui: &AppWindow) {
        self.paging.borrow_mut().reset();
        ui.set_has_older(false);
        ui.set_loading_older(false);
        ui.set_pager_ready(false);
    }

    pub fn set_tail_anchor(&self, messages: &[serde_json::Value]) {
        let mut p = self.paging.borrow_mut();
        p.tail_len = messages.len();
        p.tail_from = messages.first().map(transcript::message_key);
    }

    pub fn tail_len(&self) -> usize {
        self.paging.borrow().tail_len
    }

    /// Check if an incoming Tail page should be applied, updating cursors and `has_older`.
    /// Returns `true` when the caller should merge the page's rows.
    pub fn accept_tail_page(
        &self,
        messages: &[serde_json::Value],
        first_entry: &str,
        last_entry: &str,
        more: bool,
        fresh: bool,
        has_open_live: bool,
    ) -> Option<bool> {
        if messages.is_empty() {
            return None;
        }
        let newest = messages
            .last()
            .and_then(|message| message.get("timestamp"))
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let cursor = page_cursor(last_entry);
        let mut p = self.paging.borrow_mut();
        if p.tail_sig == Some((cursor, newest)) && !fresh {
            return None;
        }
        let next_tail_cursor = (!first_entry.is_empty()).then(|| first_entry.to_string());
        p.tail_cursor = next_tail_cursor.clone();
        p.tail_more = more;
        let has_prepended = p
            .tail_from
            .as_deref()
            .is_some_and(|key| self.has_rows_above(key));
        let mut updated_has_older = None;
        if !has_prepended {
            if let Some(c) = next_tail_cursor {
                p.oldest_cursor = Some(c);
            }
            p.has_older = more;
            updated_has_older = Some(more);
        }
        p.tail_len = messages.len();
        p.tail_sig = Some((cursor, newest));
        if has_open_live && !fresh {
            return Some(false);
        }
        p.tail_from = messages.first().map(transcript::message_key);
        let _ = updated_has_older;
        Some(true)
    }

    pub fn has_older(&self) -> bool {
        self.paging.borrow().has_older
    }

    pub fn loading_older(&self) -> bool {
        self.paging.borrow().loading_older
    }

    pub fn clear_tail_sig(&self) {
        self.paging.borrow_mut().tail_sig = None;
    }

    /// Check if upward paging should fire; if so, marks loading and returns `before` cursor.
    pub fn check_load_older(
        &self,
        follows_bottom: bool,
        ready: bool,
        content_y: f32,
        content_height: f32,
        height: f32,
    ) -> Option<String> {
        let mut p = self.paging.borrow_mut();
        let state = PagerState {
            follows_bottom,
            ready,
            loading: p.loading_older,
            has_older: p.has_older,
            content_y,
            content_height,
            height,
        };
        if !should_load_older(state) {
            return None;
        }
        let before = p.oldest_cursor.clone()?;
        p.loading_older = true;
        Some(before)
    }

    /// Complete an older page load, prepending rows and updating `oldest_cursor` and `has_older`.
    /// Returns `(added_height_px, has_older)`.
    pub fn complete_older_page(
        &self,
        rows: Vec<Row>,
        messages_empty: bool,
        first_entry: String,
        more: bool,
    ) -> (f32, bool) {
        {
            let mut p = self.paging.borrow_mut();
            p.loading_older = false;
            if messages_empty || rows.is_empty() {
                p.has_older = false;
                return (0.0, false);
            }
        }
        self.prepend(rows);
        let mut p = self.paging.borrow_mut();
        p.oldest_cursor = if first_entry.is_empty() {
            None
        } else {
            Some(first_entry)
        };
        p.has_older = more && p.oldest_cursor.is_some();
        (self.last_prepend_height(), p.has_older)
    }

    /// Check if the sheet has been idle at the floor for >= 60s with > `history_rows` rows.
    /// When it trims, restores `oldest_cursor` and `has_older` to the tail page's cursor.
    pub fn check_idle_floor_trim(&self, at_floor_ignoring_load: bool, now: SystemTime) -> Option<(usize, bool)> {
        let (should_trim, newest) = {
            let mut p = self.paging.borrow_mut();
            let at_floor = at_floor_ignoring_load && !p.loading_older;
            if !at_floor {
                p.floor_since = None;
                return None;
            }
            let since = match p.floor_since {
                Some(t) => t,
                None => {
                    p.floor_since = Some(now);
                    now
                }
            };
            if self.rows().len() > p.history_rows
                && now.duration_since(since).unwrap_or_default() >= Duration::from_secs(60)
            {
                p.floor_since = Some(now);
                (true, p.tail_from.clone())
            } else {
                (false, None)
            }
        };
        if !should_trim {
            return None;
        }
        let newest = newest?;
        let gone = self.trim_to_tail(&newest);
        if gone > 0 {
            let mut p = self.paging.borrow_mut();
            p.oldest_cursor = p.tail_cursor.clone();
            let more = p.tail_more && p.oldest_cursor.is_some();
            p.has_older = more;
            Some((gone, more))
        } else {
            None
        }
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

    /// Put one generic error row into the Sheet. It lands after the open live
    /// Message group, so a streaming answer stays contiguous; with no live
    /// answer it lands at the end. It never changes busy state, the Command
    /// word, the Composer, or the live Turn.
    pub fn show_error(&self, message: &str) {
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            inner.place_after_open(&[Row::error(message)]);
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, false);
        }
    }

    pub fn replace(&self, rows: Vec<Row>) {
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            inner.open = None;
            inner.after_open_end = None;
            inner.last_prepend_px = 0.0;
            inner.index = reindex(&rows);
            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights = rows.iter().map(estimate_row_height).collect();
                    vs.measured = vec![false; rows.len()];
                }
            }
            inner.drawn.reset(&rows);
            inner.rows = rows;
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

    /// Merge newly arrived rows after the open live answer, giving a repeated
    /// fallback key its own row instead of patching the earlier occurrence.
    pub fn merge_new_after_open(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            for (key, range) in group_by_message(&page) {
                let fresh = &page[range];
                let key = inner.next_free_key(&key);
                inner.place_keyed_after_open(&key, fresh);
            }
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, false);
        }
    }

    /// Merge newly arrived rows at the landing point, keeping repeated
    /// fallback keys as separate rows.
    pub fn merge_new(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            for (key, range) in group_by_message(&page) {
                let fresh = &page[range];
                let key = inner.next_free_key(&key);
                inner.place_keyed(&key, fresh);
            }
            inner.virtual_state.clone()
        };
        if let Some(vs) = vstate {
            apply_virtual_update(&vs, false);
        }
    }

    /// Merge a user prompt after the open live answer, for a steering message that
    /// starts while the assistant is still streaming.
    pub fn merge_after_open(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            for (key, range) in group_by_message(&page) {
                let fresh = &page[range];
                if key.is_empty() {
                    inner.place_after_open(fresh);
                    continue;
                }
                match inner.index.get(&key).cloned() {
                    Some(current) => inner.patch(&key, current, fresh),
                    None => inner.place_keyed_after_open(&key, fresh),
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
            if let Some(cursor) = inner.after_open_end {
                inner.after_open_end = Some(cursor + total);
            }

            let mut added_px = 0.0f32;
            let mut new_rows = Vec::with_capacity(total);
            let mut new_heights = Vec::with_capacity(total);
            let mut at = 0usize;
            for (key, range) in fresh {
                let len = range.len();
                for row in &page[range] {
                    let est = estimate_row_height(row);
                    added_px += est;
                    new_rows.push(row.clone());
                    new_heights.push(est);
                    at += 1;
                }
                inner.index.insert(key, at - len..at);
            }
            inner.last_prepend_px = added_px;
            inner.rows.splice(0..0, new_rows.iter().cloned());

            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights.splice(0..0, new_heights);
                    vs.measured.splice(0..0, std::iter::repeat_n(false, total));
                    vs.all_rows.splice(0..0, new_rows);
                    vs.cached.splice(0..0, std::iter::repeat_n(None, total));
                    vs.win_start += total;
                    vs.win_end += total;
                    vs.last_scroll_y -= added_px;
                }
                Some(vs_rc.clone())
            } else {
                for (i, row) in new_rows.iter().enumerate() {
                    inner.drawn.insert(i, row);
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
            let mut inner = self.inner.borrow_mut();
            inner.open = Some(key.to_string());
            inner.after_open_end = None;
        }
    }

    pub fn answer_ended(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.open = None;
        inner.after_open_end = None;
    }

    pub fn toggle(&self, at: usize) {
        let vstate = {
            let mut inner = self.inner.borrow_mut();
            let Some(row) = inner.rows.get_mut(at) else { return };
            row.expanded = !row.expanded;
            let est = estimate_row_height(row);
            let updated = row.clone();
            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    if at < vs.heights.len() {
                        vs.heights[at] = est;
                        vs.measured[at] = false;
                    }
                }
            }
            inner.drawn.set(at, &updated);
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
            inner.index = reindex(&inner.rows);
            if let Some(cursor) = inner.after_open_end {
                inner.after_open_end = Some(cursor.saturating_sub(from));
            }

            if let Some(vs_rc) = &inner.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    let r_end = from.min(vs.all_rows.len());
                    vs.all_rows.drain(0..r_end);
                    let c_end = from.min(vs.cached.len());
                    vs.cached.drain(0..c_end);
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
    fn reset(&mut self, rows: &[Row]) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            vs.all_rows = rows.to_vec();
            vs.cached = vec![None; rows.len()];
            vs.content_dirty = true;
            vs.win_start = 0;
            vs.win_end = 0;
        }
    }
    fn set(&mut self, at: usize, row: &Row) {
        let mut model_update = None;
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            if at < vs.all_rows.len() {
                vs.all_rows[at] = row.clone();
                if at >= vs.win_start && at < vs.win_end {
                    let trow = to_slint(row);
                    vs.cached[at] = Some(trow.clone());
                    model_update = Some((vs.model.clone(), at - vs.win_start, trow));
                } else {
                    vs.cached[at] = None;
                }
            }
        }
        if let Some((model, idx, r)) = model_update {
            model.set_row_data(idx, r);
        }
    }
    fn insert(&mut self, at: usize, row: &Row) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            let pos = at.min(vs.all_rows.len());
            vs.all_rows.insert(pos, row.clone());
            vs.cached.insert(pos, None);
            if vs.win_end > vs.win_start && pos < vs.win_start {
                vs.win_start += 1;
                vs.win_end += 1;
            } else if vs.win_end <= vs.win_start || pos < vs.win_end {
                vs.content_dirty = true;
            }
        }
    }
    fn remove(&mut self, at: usize) {
        if let Ok(mut vs) = self.0.try_borrow_mut() {
            if at < vs.all_rows.len() {
                vs.all_rows.remove(at);
                vs.cached.remove(at);
                if vs.win_end > vs.win_start && at < vs.win_start {
                    vs.win_start -= 1;
                    vs.win_end -= 1;
                } else if at < vs.win_end {
                    vs.content_dirty = true;
                }
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
            if let Some(vs_rc) = &self.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    vs.heights.insert(at + offset, est);
                    vs.measured.insert(at + offset, false);
                }
            }
            self.drawn.insert(at + offset, row);
        }
    }

    fn place_at(&mut self, at: usize, rows: &[Row]) {
        self.insert_rows(at, rows);
        self.shift_from(at, rows.len() as isize);
        if let Some(cursor) = self.after_open_end {
            if at <= cursor {
                self.after_open_end = Some(cursor + rows.len());
            }
        }
    }

    fn place(&mut self, rows: &[Row]) {
        self.place_at(self.landing(), rows);
    }

    fn place_keyed(&mut self, key: &str, rows: &[Row]) {
        let at = self.landing();
        self.place_at(at, rows);
        self.index.insert(key.to_string(), at..at + rows.len());
    }

    fn next_free_key(&self, key: &str) -> String {
        if key.is_empty() || !self.index.contains_key(key) {
            return key.to_string();
        }
        let mut suffix = 2;
        loop {
            let candidate = format!("{key}#{suffix}");
            if !self.index.contains_key(&candidate) {
                return candidate;
            }
            suffix += 1;
        }
    }

    fn place_keyed_after_open(&mut self, key: &str, rows: &[Row]) {
        let at = self.place_after_open(rows);
        self.index.insert(key.to_string(), at..at + rows.len());
    }

    /// A row that must not split the open live Message group: it lands after
    /// the group, at the point the reader is already looking.
    fn place_after_open(&mut self, rows: &[Row]) -> usize {
        let open_end = self
            .open
            .as_ref()
            .and_then(|key| self.index.get(key))
            .map_or(self.rows.len(), |range| range.end);
        let at = self
            .after_open_end
            .map_or(open_end, |cursor| cursor.max(open_end))
            .min(self.rows.len());
        self.place_at(at, rows);
        self.after_open_end = Some(at + rows.len());
        at
    }

    fn shift_from(&mut self, at: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for range in self.index.values_mut() {
            if range.start >= at {
                range.start = range.start.saturating_add_signed(delta);
                range.end = range.end.saturating_add_signed(delta);
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
            if let Some(vs_rc) = &self.virtual_state {
                if let Ok(mut vs) = vs_rc.try_borrow_mut() {
                    if at < vs.heights.len() && !vs.measured[at] {
                        vs.heights[at] = est;
                    }
                }
            }
            self.drawn.set(at, row);
        }
        if adjusted.len() > had {
            let at = start + had;
            let delta = (adjusted.len() - had) as isize;
            self.insert_rows(at, &adjusted[had..]);
            self.shift_from(at, delta);
            if let Some(cursor) = self.after_open_end {
                if at <= cursor {
                    self.after_open_end = Some(cursor.saturating_add_signed(delta));
                }
            }
            self.index.insert(key.to_string(), start..start + adjusted.len());
        } else if adjusted.len() < had {
            let removed_start = start + adjusted.len();
            let removed_end = start + had;
            for at in (removed_start..removed_end).rev() {
                self.rows.remove(at);
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
            let removed = (had - adjusted.len()) as isize;
            self.shift_from(removed_end, -removed);
            if let Some(cursor) = self.after_open_end {
                if cursor >= removed_end {
                    self.after_open_end = Some(cursor.saturating_sub(removed as usize));
                } else if cursor >= removed_start {
                    self.after_open_end = Some(removed_start);
                }
            }
            self.index.insert(key.to_string(), start..start + adjusted.len());
        }
    }
}

fn group_by_message(rows: &[Row]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0
            && rows[index - 1].message == row.message
            && row.kind != crate::transcript::KIND_CUSTOM_MESSAGE
        {
            if let Some((_, range)) = out.last_mut() {
                range.end = index + 1;
                continue;
            }
        }
        let key = if row.message.is_empty() {
            String::new()
        } else {
            let count = seen.entry(row.message.clone()).or_insert(0);
            *count += 1;
            if *count == 1 {
                row.message.clone()
            } else {
                format!("{}#{}", row.message, count)
            }
        };
        out.push((key, index..index + 1));
    }
    out
}

fn reindex(rows: &[Row]) -> std::collections::HashMap<String, std::ops::Range<usize>> {
    group_by_message(rows)
        .into_iter()
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

fn row_identity(row: &Row) -> String {
    if !row.id.is_empty() {
        return format!("id:{}", row.id);
    }
    format!("{}#{}#{}", row.message, row.kind, row.title)
}

pub(crate) fn to_slint(row: &Row) -> TRow {
    #[cfg(test)]
    CONVERSION_COUNT.with(|c| c.set(c.get() + 1));
    TRow {
        kind: row.kind,
        title: transcript::plain(&row.title).into(),
        meta: transcript::plain(&row.meta).into(),
        body: transcript::styled(&row.body),
        code: transcript::styled(&row.code),
        output: transcript::styled(&row.output),
        has_code: row.has_code,
        has_output: row.has_output,
        has_body: row.has_body,
        expanded: row.expanded,
        failed: row.failed,
        reason: transcript::plain(&row.reason).into(),
        image: images::from_wire(&row.image),
        elapsed: row.elapsed.clone().into(),
        timestamp: row.timestamp.clone().into(),
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