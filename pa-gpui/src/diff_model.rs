use crate::diff_git::{DiffFileSummary, DiffLoadOutcome};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

pub(crate) const DIFF_DEFAULT_WIDTH: f32 = 480.0;
pub(crate) const DIFF_MIN_WIDTH: f32 = 280.0;
pub(crate) const DIFF_NARROW_FLOOR: f32 = 180.0;
pub(crate) const DIFF_MAX_WIDTH: f32 = 960.0;
pub(crate) const CENTER_MIN_WIDTH: f32 = 340.0;

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
    if max_available < DIFF_MIN_WIDTH {
        return max_available.max(DIFF_NARROW_FLOOR.min(room));
    }
    desired.clamp(DIFF_MIN_WIDTH, max_available)
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
    session_key: Option<String>,
    cwd: Option<String>,
    repo_root: Option<PathBuf>,
    summaries: Vec<DiffFileSummary>,
    total_additions: usize,
    total_deletions: usize,
    // Files start collapsed by default; store only paths the user explicitly expanded per session.
    expanded_by_session: HashMap<String, HashSet<String>>,
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
            session_key: None,
            cwd: None,
            repo_root: None,
            summaries: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
            expanded_by_session: HashMap::new(),
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

    pub(crate) fn cancel_drag(&mut self) {
        self.dragging = false;
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

    pub(crate) fn total_additions(&self) -> usize {
        self.total_additions
    }

    pub(crate) fn total_deletions(&self) -> usize {
        self.total_deletions
    }

    pub(crate) fn matches_target(&self, session_key: Option<&str>, cwd: Option<&str>) -> bool {
        self.session_key.as_deref() == session_key
            && self.cwd.as_deref() == cwd
            && self.status != DiffViewStatus::Idle
    }

    pub(crate) fn mark_not_a_git_repo(&mut self, session_key: Option<String>, cwd: Option<String>) {
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.session_key = session_key;
        self.cwd = cwd;
        self.repo_root = None;
        self.summaries.clear();
        self.total_additions = 0;
        self.total_deletions = 0;
        self.status = DiffViewStatus::NotAGitRepo;
    }

    /// Starts a load generation for `(session_key, cwd)`. Returns `(generation, clear_existing)`
    /// where `clear_existing` is true when switching sessions/directories so no stale diff remains.
    pub(crate) fn begin_load(
        &mut self,
        session_key: Option<String>,
        cwd: Option<String>,
    ) -> (u64, bool) {
        let switched = self.session_key != session_key || self.cwd != cwd;
        self.generation = self.generation.wrapping_add(1);
        self.loading = true;
        self.session_key = session_key;
        self.cwd = cwd;
        if switched || matches!(self.status, DiffViewStatus::Idle | DiffViewStatus::NotAGitRepo) {
            self.repo_root = None;
            self.summaries.clear();
            self.total_additions = 0;
            self.total_deletions = 0;
            self.status = DiffViewStatus::Loading;
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
        match outcome {
            DiffLoadOutcome::NotAGitRepo => {
                self.repo_root = None;
                self.summaries.clear();
                self.total_additions = 0;
                self.total_deletions = 0;
                self.status = DiffViewStatus::NotAGitRepo;
                Some(Vec::new())
            }
            DiffLoadOutcome::Error(msg) => {
                self.repo_root = None;
                self.summaries.clear();
                self.total_additions = 0;
                self.total_deletions = 0;
                self.status = DiffViewStatus::Error(msg.clone());
                Some(Vec::new())
            }
            DiffLoadOutcome::Loaded(loaded) => {
                self.repo_root = Some(loaded.repo_root.clone());
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
        } else {
            self.expanded_by_session
                .entry(key)
                .or_default()
                .insert(path.to_string());
        }
    }

    /// Expands the clicked jump-list file in per-session memory.
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

    fn sample_loaded(paths: &[&str]) -> DiffLoadOutcome {
        let summaries = paths
            .iter()
            .map(|p| DiffFileSummary {
                path: (*p).to_string(),
                old_path: None,
                additions: 3,
                deletions: 1,
                binary: false,
                untracked: false,
                truncated: false,
            })
            .collect::<Vec<_>>();
        DiffLoadOutcome::Loaded(LoadedDiff {
            repo_root: PathBuf::from("/repo"),
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
        assert_eq!(clamp_diff_width(f32::NAN, 700.0, 244.0), DIFF_NARROW_FLOOR);
        assert_eq!(clamp_diff_width(480.0, 500.0, 244.0), DIFF_NARROW_FLOOR);
        assert_eq!(clamp_diff_width(480.0, 320.0, 244.0), 76.0);
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

        // Even a single changed file starts collapsed by default.
        let (g1, clear1) = model.begin_load(Some("sess-1".into()), Some("/repo-a".into()));
        assert!(clear1);
        let expanded1 = model
            .apply_load_outcome(g1, &sample_loaded(&["src/main.rs"]))
            .expect("current gen");
        assert!(expanded1.is_empty());
        assert!(!model.is_file_expanded("src/main.rs"));
        assert!(!model.all_expanded());

        // Clicking a file in the jump list expands it.
        model.expand_for_jump("src/main.rs");
        assert!(model.is_file_expanded("src/main.rs"));
        assert!(model.all_expanded());

        // Switching to session 2 clears stale data and starts collapsed.
        let (g2, clear2) = model.begin_load(Some("sess-2".into()), Some("/repo-b".into()));
        assert!(clear2);
        assert!(model.summaries().is_empty());
        // Stale generation from session 1 is ignored.
        assert!(model
            .apply_load_outcome(g1, &sample_loaded(&["stale.rs"]))
            .is_none());
        let expanded2 = model
            .apply_load_outcome(g2, &sample_loaded(&["src/main.rs", "README.md"]))
            .expect("gen 2");
        assert!(expanded2.is_empty());
        assert!(!model.is_file_expanded("src/main.rs"));

        // Switching back to session 1 restores src/main.rs expanded while new file stays collapsed.
        let (g3, clear3) = model.begin_load(Some("sess-1".into()), Some("/repo-a".into()));
        assert!(clear3);
        let expanded3 = model
            .apply_load_outcome(g3, &sample_loaded(&["src/main.rs", "src/new.rs"]))
            .expect("gen 3");
        assert_eq!(expanded3, vec!["src/main.rs"]);
        assert!(model.is_file_expanded("src/main.rs"));
        assert!(!model.is_file_expanded("src/new.rs"));

        // Expand all / collapse all toggle cleanly.
        model.set_all_collapsed(false);
        assert!(model.all_expanded());
        model.set_all_collapsed(true);
        assert!(!model.all_expanded());
        assert!(!model.is_file_expanded("src/main.rs"));
    }
}
