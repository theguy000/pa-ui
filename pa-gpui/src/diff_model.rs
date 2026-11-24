use crate::diff_git::{same_worktree_path, DiffFileSummary, DiffLoadOutcome, RepoContext, WorktreeEntry};
use crate::settings::DiffScope;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

pub(crate) const DIFF_DEFAULT_WIDTH: f32 = 480.0;
pub(crate) const DIFF_MIN_WIDTH: f32 = 280.0;
pub(crate) const DIFF_NARROW_FLOOR: f32 = 180.0;
pub(crate) const DIFF_MAX_WIDTH: f32 = 960.0;
pub(crate) const CENTER_MIN_WIDTH: f32 = crate::composer::CHAT_COLUMN_MIN_WIDTH;

pub(crate) fn clamp_diff_width(desired: f32, viewport_w: f32, sidebar_w: f32) -> f32 {
    let desired = if desired.is_finite() {
        desired
    } else {
        DIFF_DEFAULT_WIDTH
    };
    if viewport_w <= 0.0 {
        return desired.clamp(DIFF_MIN_WIDTH, DIFF_MAX_WIDTH);
    }
    let room = (viewport_w - sidebar_w).max(0.0);
    let max_available = (room - CENTER_MIN_WIDTH).min(DIFF_MAX_WIDTH);
    if max_available < DIFF_NARROW_FLOOR {
        return 0.0;
    }
    if max_available < DIFF_MIN_WIDTH {
        return max_available;
    }
    desired.clamp(DIFF_MIN_WIDTH, max_available)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StickyHeaderLayout {
    pub(crate) file_ix: usize,
    pub(crate) top_offset_px: i32,
}

#[derive(Default)]
pub(crate) struct StickyDiffTracker {
    pub(crate) viewport_top: f32,
    pub(crate) header_height: f32,
    pub(crate) frame_headers: Vec<(usize, f32)>,
    pub(crate) hovered_file: Option<usize>,
}

/// Computes which expanded file header (if any) should stick to the top of the diff viewport,
/// plus an optional negative Y push-up offset (`<= 0`) as the next file header approaches.
pub(crate) fn compute_sticky_diff_header(
    viewport_top: f32,
    header_height: f32,
    frame_headers: &[(usize, f32)],
    expanded: &[bool],
    prev_sticky: Option<usize>,
    hovered_file: Option<usize>,
) -> Option<StickyHeaderLayout> {
    if expanded.is_empty() {
        return None;
    }
    let sticky_h = header_height.max(28.0);
    let mut headers: Vec<(usize, f32)> = frame_headers
        .iter()
        .copied()
        .filter(|(ix, _)| *ix < expanded.len())
        .collect();
    headers.sort_by_key(|(ix, _)| *ix);
    headers.dedup_by_key(|(ix, _)| *ix);

    let finish = |file_ix: usize, next_top: Option<f32>| -> Option<StickyHeaderLayout> {
        if !expanded.get(file_ix).copied().unwrap_or(false) {
            return None;
        }
        let top_offset_px = match next_top {
            Some(ny) if ny < viewport_top + sticky_h => {
                ((ny - viewport_top - sticky_h).round() as i32).clamp(-(sticky_h.ceil() as i32), 0)
            }
            _ => 0,
        };
        if top_offset_px <= -(sticky_h.ceil() as i32) {
            return None;
        }
        Some(StickyHeaderLayout {
            file_ix,
            top_offset_px,
        })
    };

    // 1. Any rendered header scrolled above the top edge of the viewport.
    if let Some((above_pos, &(above_ix, _))) = headers
        .iter()
        .enumerate()
        .rev()
        .find(|(_, (_, y))| *y < viewport_top - 1.5)
    {
        let next_top = headers
            .get(above_pos + 1)
            .filter(|(next_ix, _)| *next_ix == above_ix + 1)
            .map(|(_, y)| *y);
        if next_top.is_some_and(|ny| ny <= viewport_top + 1.5) {
            return None;
        }
        return finish(above_ix, next_top);
    }

    // 2. No rendered header is above the top, but at least one header is visible below.
    if let Some(&(first_ix, first_y)) = headers.first() {
        if first_y <= viewport_top + 1.5 {
            return None;
        }
        if first_ix > 0 {
            return finish(first_ix - 1, Some(first_y));
        }
        return None;
    }

    // 3. No file headers are visible in the current viewport (deep inside a long file).
    if let Some(h) = hovered_file.filter(|&ix| expanded.get(ix).copied().unwrap_or(false)) {
        return finish(h, None);
    }
    if let Some(prev) = prev_sticky.filter(|&ix| expanded.get(ix).copied().unwrap_or(false)) {
        return finish(prev, None);
    }
    let mut expanded_iter = expanded
        .iter()
        .enumerate()
        .filter_map(|(ix, &open)| open.then_some(ix));
    if let (Some(only), None) = (expanded_iter.next(), expanded_iter.next()) {
        return finish(only, None);
    }
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiffViewStatus {
    Idle,
    Loading,
    NotAGitRepo,
    Empty,
    Ready,
    Error(String),
}

pub(crate) struct DiffModel {
    width: f32,
    dragging: bool,
    drag_start_x: f32,
    drag_start_width: f32,
    handle_hovered: bool,
    status: DiffViewStatus,
    loading: bool,
    generation: u64,
    context_generation: u64,
    context_cwd: Option<String>,
    debounce_seq: u64,
    session_key: Option<String>,
    cwd: Option<String>,
    scope: Option<DiffScope>,
    requested_base: Option<String>,
    repo_root: Option<PathBuf>,
    base_ref: Option<String>,
    base_branch: Option<String>,
    on_base_branch: bool,
    branches: Vec<String>,
    worktrees: Vec<WorktreeEntry>,
    summaries: Vec<DiffFileSummary>,
    loading_paths: HashSet<String>,
    total_additions: usize,
    total_deletions: usize,
    // Files start collapsed by default; store only paths the reader explicitly expanded per session.
    expanded_by_session: HashMap<String, HashSet<String>>,
    selected_worktree_by_session: HashMap<String, PathBuf>,
    selected_base_by_session: HashMap<String, String>,
    recent_worktrees: HashMap<String, Vec<WorktreeEntry>>,
    expanded_projects: HashSet<String>,
    recent_probe_generation: u64,
    draft_folder_generation: u64,
    sticky_header: Option<StickyHeaderLayout>,
    sticky_tracker: Rc<RefCell<StickyDiffTracker>>,
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
            status: DiffViewStatus::Idle,
            loading: false,
            generation: 0,
            context_generation: 0,
            context_cwd: None,
            debounce_seq: 0,
            session_key: None,
            cwd: None,
            scope: None,
            requested_base: None,
            repo_root: None,
            base_ref: None,
            base_branch: None,
            on_base_branch: false,
            branches: Vec::new(),
            worktrees: Vec::new(),
            summaries: Vec::new(),
            loading_paths: HashSet::new(),
            total_additions: 0,
            total_deletions: 0,
            expanded_by_session: HashMap::new(),
            selected_worktree_by_session: HashMap::new(),
            selected_base_by_session: HashMap::new(),
            recent_worktrees: HashMap::new(),
            expanded_projects: HashSet::new(),
            recent_probe_generation: 0,
            draft_folder_generation: 0,
            sticky_header: None,
            sticky_tracker: Rc::new(RefCell::new(StickyDiffTracker::default())),
        }
    }

    pub(crate) fn sticky_header(&self) -> Option<StickyHeaderLayout> {
        self.sticky_header
    }

    pub(crate) fn sticky_tracker(&self) -> Rc<RefCell<StickyDiffTracker>> {
        self.sticky_tracker.clone()
    }

    pub(crate) fn set_sticky_header(&mut self, next: Option<StickyHeaderLayout>) -> bool {
        if self.sticky_header != next {
            self.sticky_header = next;
            true
        } else {
            false
        }
    }

    pub(crate) fn clear_sticky_header(&mut self) {
        self.sticky_header = None;
        let mut tr = self.sticky_tracker.borrow_mut();
        tr.frame_headers.clear();
        tr.hovered_file = None;
    }

    #[cfg(test)]
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

    pub(crate) fn cancel_pending_on_close(&mut self) {
        let was_loading = self.loading;
        self.dragging = false;
        self.loading = false;
        self.generation = self.generation.wrapping_add(1);
        self.debounce_seq = self.debounce_seq.wrapping_add(1);
        self.loading_paths.clear();
        if was_loading {
            self.context_generation = self.context_generation.wrapping_add(1);
            self.context_cwd = None;
        }
    }

    pub(crate) fn reset_width(&mut self) -> f32 {
        self.width = DIFF_DEFAULT_WIDTH;
        self.dragging = false;
        self.width
    }

    pub(crate) fn status(&self) -> &DiffViewStatus {
        &self.status
    }

    pub(crate) fn is_loading(&self) -> bool {
        self.loading
    }

    pub(crate) fn summaries(&self) -> &[DiffFileSummary] {
        &self.summaries
    }

    pub(crate) fn is_file_loading(&self, path: &str) -> bool {
        self.loading_paths.contains(path)
    }

    pub(crate) fn total_additions(&self) -> usize {
        self.total_additions
    }

    pub(crate) fn total_deletions(&self) -> usize {
        self.total_deletions
    }

    pub(crate) fn base_branch(&self) -> Option<&str> {
        self.base_branch.as_deref()
    }

    pub(crate) fn on_base_branch(&self) -> bool {
        self.on_base_branch
    }

    pub(crate) fn branches(&self) -> &[String] {
        &self.branches
    }

    pub(crate) fn worktrees(&self) -> &[WorktreeEntry] {
        &self.worktrees
    }

    #[allow(dead_code)]
    pub(crate) fn has_multiple_worktrees(&self) -> bool {
        self.worktrees.len() > 1
    }

    #[allow(dead_code)]
    pub(crate) fn active_worktree(&self) -> Option<&WorktreeEntry> {
        let root = self.repo_root.as_deref()?;
        self.worktrees
            .iter()
            .find(|wt| same_worktree_path(&wt.path, root))
    }

    #[allow(dead_code)]
    pub(crate) fn main_worktree_label(&self) -> Option<&str> {
        self.worktrees
            .iter()
            .find(|wt| wt.is_main)
            .or_else(|| self.worktrees.first())
            .map(|wt| wt.label.as_str())
    }

    #[allow(dead_code)]
    pub(crate) fn begin_recent_worktrees_probe(&mut self) -> u64 {
        self.recent_probe_generation = self.recent_probe_generation.wrapping_add(1);
        self.recent_probe_generation
    }

    pub(crate) fn bump_draft_folder_generation(&mut self) -> u64 {
        self.draft_folder_generation = self.draft_folder_generation.wrapping_add(1);
        self.draft_folder_generation
    }

    pub(crate) fn is_current_draft_folder_generation(&self, generation: u64) -> bool {
        generation == self.draft_folder_generation
    }

    #[allow(dead_code)]
    pub(crate) fn apply_recent_worktrees(
        &mut self,
        generation: u64,
        map: HashMap<String, Vec<WorktreeEntry>>,
    ) -> bool {
        if generation != self.recent_probe_generation {
            return false;
        }
        self.recent_worktrees = map;
        true
    }

    #[allow(dead_code)]
    pub(crate) fn worktrees_for_project(&self, project_path: &str) -> &[WorktreeEntry] {
        self.recent_worktrees
            .get(project_path)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    #[allow(dead_code)]
    pub(crate) fn is_project_expanded(&self, project_path: &str) -> bool {
        self.expanded_projects.contains(project_path)
    }

    #[allow(dead_code)]
    pub(crate) fn toggle_project_expanded(&mut self, project_path: &str) {
        if !self.expanded_projects.remove(project_path) {
            self.expanded_projects.insert(project_path.to_string());
        }
    }

    pub(crate) fn begin_context_probe(&mut self, cwd: Option<&str>) -> Option<(u64, PathBuf)> {
        if self.context_cwd.as_deref() == cwd {
            return None;
        }
        self.context_generation = self.context_generation.wrapping_add(1);
        self.context_cwd = cwd.map(str::to_string);
        match cwd {
            Some(path) if !path.trim().is_empty() => {
                self.repo_root = None;
                self.worktrees.clear();
                Some((self.context_generation, PathBuf::from(path)))
            }
            _ => {
                self.repo_root = None;
                self.worktrees.clear();
                None
            }
        }
    }

    pub(crate) fn apply_context_probe(&mut self, generation: u64, ctx: Option<RepoContext>) -> bool {
        if generation != self.context_generation {
            return false;
        }
        match ctx {
            Some(ctx) => {
                self.repo_root = Some(ctx.repo_root);
                self.worktrees = ctx.worktrees;
            }
            None => {
                self.repo_root = None;
                self.worktrees.clear();
            }
        }
        true
    }

    #[allow(dead_code)]
    pub(crate) fn is_active_worktree(&self, path: &Path) -> bool {
        self.repo_root
            .as_deref()
            .is_some_and(|root| same_worktree_path(root, path))
    }

    pub(crate) fn effective_cwd(
        &self,
        session_key: Option<&str>,
        session_cwd: Option<&str>,
    ) -> Option<String> {
        if let Some(key) = session_key
            && let Some(wt) = self.selected_worktree_by_session.get(key)
        {
            return Some(wt.to_string_lossy().into_owned());
        }
        session_cwd.map(str::to_string)
    }

    #[allow(dead_code)]
    pub(crate) fn set_selected_worktree(&mut self, session_key: Option<&str>, path: PathBuf) {
        if let Some(key) = session_key {
            self.selected_worktree_by_session
                .insert(key.to_string(), path);
        }
    }

    pub(crate) fn requested_base_for_session(&self, session_key: Option<&str>) -> Option<&str> {
        session_key.and_then(|key| self.selected_base_by_session.get(key).map(String::as_str))
    }

    pub(crate) fn set_selected_base(&mut self, session_key: Option<&str>, branch: String) {
        if let Some(key) = session_key {
            self.selected_base_by_session
                .insert(key.to_string(), branch);
        }
    }

    pub(crate) fn matches_target(
        &self,
        session_key: Option<&str>,
        cwd: Option<&str>,
        scope: DiffScope,
        requested_base: Option<&str>,
    ) -> bool {
        self.session_key.as_deref() == session_key
            && self.cwd.as_deref() == cwd
            && self.scope == Some(scope)
            && self.requested_base.as_deref() == requested_base
            && self.status != DiffViewStatus::Idle
    }

    /// Gates debounced turn/tool completion refreshes on the right column being open.
    /// Returns `None` when closed so zero timers or git work are spawned.
    pub(crate) fn begin_debounced_refresh(&mut self, right_open: bool) -> Option<u64> {
        if !right_open {
            return None;
        }
        self.debounce_seq = self.debounce_seq.wrapping_add(1);
        Some(self.debounce_seq)
    }

    pub(crate) fn should_fire_debounced_refresh(&self, seq: u64, right_open: bool) -> bool {
        right_open && seq == self.debounce_seq
    }

    pub(crate) fn mark_not_a_git_repo(&mut self, session_key: Option<String>, cwd: Option<String>) {
        self.generation = self.generation.wrapping_add(1);
        self.context_generation = self.context_generation.wrapping_add(1);
        self.context_cwd = cwd.clone();
        self.loading = false;
        self.session_key = session_key;
        self.cwd = cwd;
        self.scope = None;
        self.requested_base = None;
        self.repo_root = None;
        self.base_ref = None;
        self.base_branch = None;
        self.on_base_branch = false;
        self.branches.clear();
        self.worktrees.clear();
        self.summaries.clear();
        self.loading_paths.clear();
        self.total_additions = 0;
        self.total_deletions = 0;
        self.status = DiffViewStatus::NotAGitRepo;
        self.clear_sticky_header();
    }

    /// Starts a load generation for `(session_key, cwd, scope, requested_base)`. Returns
    /// `(generation, clear_existing)` where `clear_existing` is true when switching targets so no
    /// stale diff remains.
    pub(crate) fn begin_load(
        &mut self,
        session_key: Option<String>,
        cwd: Option<String>,
        scope: DiffScope,
        requested_base: Option<String>,
    ) -> (u64, bool) {
        let switched = self.session_key != session_key
            || self.cwd != cwd
            || self.scope != Some(scope)
            || self.requested_base != requested_base;
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        self.loading_paths.clear();
        self.session_key = session_key;
        self.cwd = cwd;
        self.scope = Some(scope);
        self.requested_base = requested_base;
        if switched || matches!(self.status, DiffViewStatus::Idle | DiffViewStatus::NotAGitRepo) {
            self.context_generation = self.context_generation.wrapping_add(1);
            self.context_cwd = None;
            self.repo_root = None;
            self.base_ref = None;
            self.base_branch = None;
            self.on_base_branch = false;
            self.branches.clear();
            self.worktrees.clear();
            self.summaries.clear();
            self.total_additions = 0;
            self.total_deletions = 0;
            self.status = DiffViewStatus::Loading;
            self.clear_sticky_header();
            (self.generation, true)
        } else {
            (self.generation, false)
        }
    }

    /// Applies a finished background load if its generation is still current.
    /// Returns `Some(expanded_paths)` for the caller to mirror into `DiffState`, or `None` if stale.
    pub(crate) fn apply_load_outcome(
        &mut self,
        generation: u64,
        outcome: &DiffLoadOutcome,
    ) -> Option<Vec<String>> {
        if generation != self.generation {
            return None;
        }
        self.loading = false;
        self.loading_paths.clear();
        match outcome {
            DiffLoadOutcome::NotAGitRepo => {
                self.context_generation = self.context_generation.wrapping_add(1);
                self.context_cwd = self.cwd.clone();
                self.repo_root = None;
                self.base_ref = None;
                self.base_branch = None;
                self.on_base_branch = false;
                self.branches.clear();
                self.worktrees.clear();
                self.summaries.clear();
                self.total_additions = 0;
                self.total_deletions = 0;
                self.status = DiffViewStatus::NotAGitRepo;
                Some(Vec::new())
            }
            DiffLoadOutcome::Error(msg) => {
                self.context_generation = self.context_generation.wrapping_add(1);
                self.context_cwd = None;
                self.repo_root = None;
                self.base_ref = None;
                self.base_branch = None;
                self.on_base_branch = false;
                self.branches.clear();
                self.worktrees.clear();
                self.summaries.clear();
                self.total_additions = 0;
                self.total_deletions = 0;
                self.status = DiffViewStatus::Error(msg.clone());
                Some(Vec::new())
            }
            DiffLoadOutcome::Loaded(loaded) => {
                self.context_generation = self.context_generation.wrapping_add(1);
                self.context_cwd = self.cwd.clone();
                self.repo_root = Some(loaded.repo_root.clone());
                self.base_ref = Some(loaded.base_ref.clone());
                self.base_branch = loaded.base_branch.clone();
                self.on_base_branch = loaded.on_base_branch;
                self.branches = loaded.branches.clone();
                self.worktrees = loaded.worktrees.clone();
                self.summaries = loaded.summaries.clone();
                self.total_additions = loaded.total_additions;
                self.total_deletions = loaded.total_deletions;
                self.status = if self.summaries.is_empty() {
                    DiffViewStatus::Empty
                } else {
                    DiffViewStatus::Ready
                };
                Some(self.expanded_paths_for_current_files())
            }
        }
    }

    /// If `path` is currently deferred and not already loading, marks it in-flight and returns
    /// `(generation, repo_root, base_ref, old_path, untracked)`.
    pub(crate) fn begin_deferred_file_load(
        &mut self,
        path: &str,
    ) -> Option<(u64, PathBuf, String, Option<String>, bool)> {
        let summary = self.summaries.iter().find(|s| s.path == path)?;
        if !summary.deferred || self.loading_paths.contains(path) {
            return None;
        }
        let repo_root = self.repo_root.clone()?;
        let base_ref = self.base_ref.clone()?;
        let old_path = summary.old_path.clone();
        let untracked = summary.untracked;
        self.loading_paths.insert(path.to_string());
        Some((self.generation, repo_root, base_ref, old_path, untracked))
    }

    /// Completes an on-demand single-file load if its generation still matches.
    /// Returns the file's index in `summaries` when the caller should splice the loaded `DiffFile`.
    pub(crate) fn finish_deferred_file_load(
        &mut self,
        generation: u64,
        path: &str,
        succeeded: bool,
    ) -> Option<usize> {
        if generation != self.generation {
            return None;
        }
        self.loading_paths.remove(path);
        let ix = self.summaries.iter().position(|s| s.path == path)?;
        if succeeded {
            self.summaries[ix].deferred = false;
            Some(ix)
        } else {
            None
        }
    }

    pub(crate) fn deferred_paths(&self) -> Vec<String> {
        self.summaries
            .iter()
            .filter(|s| s.deferred)
            .map(|s| s.path.clone())
            .collect()
    }

    pub(crate) fn is_file_expanded(&self, path: &str) -> bool {
        let Some(key) = self.session_key.as_deref() else {
            return false;
        };
        self.expanded_by_session
            .get(key)
            .is_some_and(|set| set.contains(path))
    }

    pub(crate) fn set_file_collapsed(&mut self, path: &str, collapsed: bool) {
        let Some(key) = self.session_key.clone() else {
            return;
        };
        if collapsed {
            if let Some(set) = self.expanded_by_session.get_mut(&key) {
                set.remove(path);
            }
            if self
                .sticky_header
                .and_then(|s| self.summaries.get(s.file_ix))
                .is_some_and(|s| s.path == path)
            {
                self.clear_sticky_header();
            }
        } else {
            self.expanded_by_session
                .entry(key)
                .or_default()
                .insert(path.to_string());
        }
    }

    pub(crate) fn expand_for_jump(&mut self, path: &str) {
        self.set_file_collapsed(path, false);
    }

    pub(crate) fn all_expanded(&self) -> bool {
        !self.summaries.is_empty()
            && self
                .summaries
                .iter()
                .all(|s| self.is_file_expanded(&s.path))
    }

    pub(crate) fn set_all_collapsed(&mut self, collapsed: bool) {
        let Some(key) = self.session_key.clone() else {
            return;
        };
        if collapsed {
            if let Some(set) = self.expanded_by_session.get_mut(&key) {
                set.clear();
            }
            self.clear_sticky_header();
        } else {
            let set = self.expanded_by_session.entry(key).or_default();
            for summary in &self.summaries {
                set.insert(summary.path.clone());
            }
        }
    }

    pub(crate) fn expanded_paths_for_current_files(&self) -> Vec<String> {
        let Some(key) = self.session_key.as_deref() else {
            return Vec::new();
        };
        let Some(set) = self.expanded_by_session.get(key) else {
            return Vec::new();
        };
        self.summaries
            .iter()
            .filter(|s| set.contains(&s.path))
            .map(|s| s.path.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff_git::LoadedDiff;

    fn sample_loaded(paths: &[(&str, bool)]) -> DiffLoadOutcome {
        let summaries = paths
            .iter()
            .map(|(p, deferred)| DiffFileSummary {
                path: (*p).to_string(),
                old_path: None,
                additions: 3,
                deletions: 1,
                binary: false,
                added: false,
                untracked: false,
                truncated: false,
                deferred: *deferred,
            })
            .collect::<Vec<_>>();
        DiffLoadOutcome::Loaded(LoadedDiff {
            repo_root: PathBuf::from("/repo"),
            base_ref: "HEAD".into(),
            base_branch: None,
            on_base_branch: false,
            branches: Vec::new(),
            worktrees: Vec::new(),
            files: Vec::new(),
            total_additions: summaries.len() * 3,
            total_deletions: summaries.len(),
            summaries,
        })
    }

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
        assert_eq!(clamp_diff_width(480.0, 980.0, 244.0), 980.0 - 244.0 - CENTER_MIN_WIDTH);
        assert_eq!(clamp_diff_width(f32::NAN, 760.0, 244.0), 0.0);
        assert_eq!(clamp_diff_width(480.0, 500.0, 244.0), 0.0);
        assert_eq!(clamp_diff_width(480.0, 760.0, 44.0), 760.0 - 44.0 - CENTER_MIN_WIDTH);
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

    #[::core::prelude::v1::test]
    fn files_start_collapsed_jump_expands_and_memory_is_per_session() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);

        let (g1, clear1) = model.begin_load(
            Some("sess-1".into()),
            Some("/repo-a".into()),
            DiffScope::Uncommitted,
            None,
        );
        assert!(clear1);
        let expanded1 = model
            .apply_load_outcome(g1, &sample_loaded(&[("src/main.rs", false)]))
            .expect("current gen");
        assert!(expanded1.is_empty());
        assert!(!model.is_file_expanded("src/main.rs"));
        assert!(!model.all_expanded());

        model.expand_for_jump("src/main.rs");
        assert!(model.is_file_expanded("src/main.rs"));
        assert!(model.all_expanded());

        let (g2, clear2) = model.begin_load(
            Some("sess-2".into()),
            Some("/repo-b".into()),
            DiffScope::Uncommitted,
            None,
        );
        assert!(clear2);
        assert!(model.summaries().is_empty());
        assert!(model
            .apply_load_outcome(g1, &sample_loaded(&[("stale.rs", false)]))
            .is_none());
        let expanded2 = model
            .apply_load_outcome(
                g2,
                &sample_loaded(&[("src/main.rs", false), ("README.md", false)]),
            )
            .expect("gen 2");
        assert!(expanded2.is_empty());
        assert!(!model.is_file_expanded("src/main.rs"));

        let (g3, clear3) = model.begin_load(
            Some("sess-1".into()),
            Some("/repo-a".into()),
            DiffScope::Uncommitted,
            None,
        );
        assert!(clear3);
        let expanded3 = model
            .apply_load_outcome(
                g3,
                &sample_loaded(&[("src/main.rs", false), ("src/new.rs", false)]),
            )
            .expect("gen 3");
        assert_eq!(expanded3, vec!["src/main.rs"]);
        assert!(model.is_file_expanded("src/main.rs"));
        assert!(!model.is_file_expanded("src/new.rs"));

        model.set_all_collapsed(false);
        assert!(model.all_expanded());
        model.set_all_collapsed(true);
        assert!(!model.all_expanded());
        assert!(!model.is_file_expanded("src/main.rs"));
    }

    #[::core::prelude::v1::test]
    fn closed_right_column_gates_debounced_refresh_and_latest_seq_wins() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);
        assert_eq!(model.begin_debounced_refresh(false), None);

        let seq1 = model.begin_debounced_refresh(true).expect("open");
        assert!(model.should_fire_debounced_refresh(seq1, true));
        assert!(!model.should_fire_debounced_refresh(seq1, false));

        let seq2 = model.begin_debounced_refresh(true).expect("open");
        assert!(!model.should_fire_debounced_refresh(seq1, true));
        assert!(model.should_fire_debounced_refresh(seq2, true));
    }

    #[::core::prelude::v1::test]
    fn deferred_file_load_tracks_inflight_and_discards_stale_generation() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);
        let (g1, _) = model.begin_load(
            Some("s1".into()),
            Some("/repo".into()),
            DiffScope::Uncommitted,
            None,
        );
        model
            .apply_load_outcome(
                g1,
                &sample_loaded(&[("src/lib.rs", false), ("Cargo.lock", true)]),
            )
            .expect("gen 1");

        assert!(model.begin_deferred_file_load("src/lib.rs").is_none());
        let (load_gen, root, base_ref, old_path, untracked) = model
            .begin_deferred_file_load("Cargo.lock")
            .expect("deferred file should begin loading");
        assert_eq!(load_gen, g1);
        assert_eq!(root, PathBuf::from("/repo"));
        assert_eq!(base_ref, "HEAD");
        assert_eq!(old_path, None);
        assert!(!untracked);
        assert!(model.is_file_loading("Cargo.lock"));
        assert!(model.begin_deferred_file_load("Cargo.lock").is_none());

        let (g2, _) = model.begin_load(
            Some("s1".into()),
            Some("/repo".into()),
            DiffScope::Uncommitted,
            None,
        );
        assert!(!model.is_file_loading("Cargo.lock"));
        assert_eq!(model.finish_deferred_file_load(load_gen, "Cargo.lock", true), None);

        model
            .apply_load_outcome(g2, &sample_loaded(&[("Cargo.lock", true)]))
            .expect("gen 2");
        let (load_gen2, _, _, _, _) = model.begin_deferred_file_load("Cargo.lock").unwrap();
        assert_eq!(
            model.finish_deferred_file_load(load_gen2, "Cargo.lock", true),
            Some(0)
        );
        assert!(!model.summaries()[0].deferred);
        assert!(!model.is_file_loading("Cargo.lock"));
    }

    #[::core::prelude::v1::test]
    fn per_session_worktree_and_base_memory_persists_across_session_switches() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);
        let temp_wt_a = std::env::temp_dir().join(format!("pa-wt-a-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&temp_wt_a);
        let temp_wt_a_str = temp_wt_a.to_string_lossy().into_owned();

        assert_eq!(
            model.effective_cwd(Some("sess-a"), Some("/cwd-a")).as_deref(),
            Some("/cwd-a")
        );
        model.set_selected_worktree(Some("sess-a"), temp_wt_a.clone());
        model.set_selected_base(Some("sess-a"), "develop".into());
        assert_eq!(
            model.effective_cwd(Some("sess-a"), Some("/cwd-a")).as_deref(),
            Some(temp_wt_a_str.as_str())
        );
        assert_eq!(
            model.requested_base_for_session(Some("sess-a")),
            Some("develop")
        );

        assert_eq!(
            model.effective_cwd(Some("sess-b"), Some("/cwd-b")).as_deref(),
            Some("/cwd-b")
        );
        assert_eq!(model.requested_base_for_session(Some("sess-b")), None);

        assert_eq!(
            model.effective_cwd(Some("sess-a"), Some("/cwd-a")).as_deref(),
            Some(temp_wt_a_str.as_str())
        );
        assert_eq!(
            model.requested_base_for_session(Some("sess-a")),
            Some("develop")
        );
        let (g, _) = model.begin_load(
            Some("sess-a".into()),
            Some(temp_wt_a_str.clone()),
            DiffScope::Uncommitted,
            Some("develop".into()),
        );
        model.apply_load_outcome(g, &sample_loaded(&[("a.rs", false)]));
        assert!(model.matches_target(
            Some("sess-a"),
            Some(temp_wt_a_str.as_str()),
            DiffScope::Uncommitted,
            Some("develop"),
        ));
        assert!(!model.matches_target(
            Some("sess-a"),
            Some(temp_wt_a_str.as_str()),
            DiffScope::Branch,
            Some("develop"),
        ));

        let _ = std::fs::remove_dir_all(&temp_wt_a);
    }

    #[::core::prelude::v1::test]
    fn context_probe_recovers_after_non_git_load_and_ignores_stale_probe() {
        let mut model = DiffModel::new(DIFF_DEFAULT_WIDTH);
        let (p1, _) = model
            .begin_context_probe(Some("/repo-a"))
            .expect("initial probe for repo-a");
        assert!(model.apply_context_probe(
            p1,
            Some(RepoContext {
                repo_root: PathBuf::from("/repo-a"),
                worktrees: vec![WorktreeEntry {
                    path: PathBuf::from("/repo-a"),
                    label: "repo-a".into(),
                    branch: Some("main".into()),
                    is_main: true,
                }],
            }),
        ));
        assert!(model.active_worktree().is_some());

        let (g1, _) = model.begin_load(
            Some("s-home".into()),
            Some("/home".into()),
            DiffScope::Uncommitted,
            None,
        );
        assert!(!model.apply_context_probe(p1, None));
        model.apply_load_outcome(g1, &DiffLoadOutcome::NotAGitRepo);
        assert!(model.active_worktree().is_none());

        let (p2, path2) = model
            .begin_context_probe(Some("/repo-a"))
            .expect("returning to repo-a while closed must re-probe");
        assert_eq!(path2, PathBuf::from("/repo-a"));
        assert!(p2 != p1);
    }

    #[::core::prelude::v1::test]
    fn sticky_header_tracks_scroll_pushup_and_collapsed_files() {
        let exp = [true, true, false, true];
        // At scroll=0, F0 header sits right at viewport_top => no sticky overlay.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(0, 100.0)], &exp, None, None),
            None
        );
        // Scrolled down 20px into F0 => F0 pins at top_offset_px = 0.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(0, 80.0)], &exp, None, None),
            Some(StickyHeaderLayout { file_ix: 0, top_offset_px: 0 })
        );
        // Deep inside F0 (no headers visible) => keeps prev_sticky = 0.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[], &exp, Some(0), None),
            Some(StickyHeaderLayout { file_ix: 0, top_offset_px: 0 })
        );
        // F1 header enters from bottom at y=220 => still F0 at top_offset_px = 0.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(1, 220.0)], &exp, Some(0), None),
            Some(StickyHeaderLayout { file_ix: 0, top_offset_px: 0 })
        );
        // F1 header approaches within 32px of viewport_top (y=118) => pushes F0 up by -14px.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(1, 118.0)], &exp, Some(0), None),
            Some(StickyHeaderLayout { file_ix: 0, top_offset_px: -14 })
        );
        // F1 header reaches viewport_top => sticky clears cleanly.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(1, 100.0)], &exp, Some(0), None),
            None
        );
        // Collapsed file (F2) scrolled above top never produces a sticky header.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[(2, 70.0), (3, 102.0)], &exp, Some(1), None),
            None
        );
        // Hovered file recovers sticky identity after a blind jump with no visible headers.
        assert_eq!(
            compute_sticky_diff_header(100.0, 32.0, &[], &exp, Some(0), Some(3)),
            Some(StickyHeaderLayout { file_ix: 3, top_offset_px: 0 })
        );
    }
}
