//! Session roster module: owns the session list, active selection, model catalog,
//! live session state/stats, activity counters, and grouped sidebar projection.
//!
//! Deep in-process module: private state, small interface, testable directly at its seam.

use crate::popups::model_label;
use crate::settings::AppSettings;
use crate::sidebar::session_key;
use pa_client::{clamp_level, ModelEntry, SessionRow, SessionState, SessionStats};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ResolvedModel {
    pub(crate) provider: String,
    pub(crate) model_id: String,
    pub(crate) label: String,
    pub(crate) thinking: String,
}

pub(crate) fn row_matches_selector(row: &SessionRow, selector: &str) -> bool {
    !selector.is_empty()
        && (session_key(row) == selector
            || (!row.session_id.is_empty() && row.session_id == selector)
            || (!row.session_file.is_empty() && row.session_file == selector))
}

pub(crate) struct GroupedSessions<'a> {
    pub(crate) active: Vec<&'a SessionRow>,
    pub(crate) idle: Vec<&'a SessionRow>,
    pub(crate) done: Vec<&'a SessionRow>,
}

pub(crate) enum ModelPickEffect {
    DraftOnly,
    SetModel {
        session: String,
        provider: String,
        model_id: String,
    },
    SetModelAndThinking {
        session: String,
        provider: String,
        model_id: String,
        level: String,
    },
    None,
}

pub(crate) struct SessionRoster {
    sessions: Vec<SessionRow>,
    active_session_id: Option<String>,
    models: Vec<ModelEntry>,
    session_state: Option<SessionState>,
    stats: Option<SessionStats>,
    draft_chat: bool,
    draft_model_picked: bool,
    draft_cwd: Option<String>,
    pending_start_cwd: Option<String>,
    subagent_count: usize,
    heartbeat_count: usize,
    shell_count: usize,
}

impl SessionRoster {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Vec::new(),
            active_session_id: None,
            models: Vec::new(),
            session_state: None,
            stats: None,
            draft_chat: false,
            draft_model_picked: false,
            draft_cwd: None,
            pending_start_cwd: None,
            subagent_count: 0,
            heartbeat_count: 0,
            shell_count: 0,
        }
    }

    pub(crate) fn active_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    pub(crate) fn set_active_id(&mut self, id: Option<String>) {
        self.active_session_id = id;
    }

    pub(crate) fn is_draft(&self) -> bool {
        self.draft_chat
    }

    pub(crate) fn is_draft_or_empty(&self) -> bool {
        self.draft_chat || self.active_session_id.is_none()
    }

    pub(crate) fn draft_cwd(&self) -> Option<&str> {
        self.draft_cwd.as_deref()
    }

    pub(crate) fn pending_start_cwd(&self) -> Option<&str> {
        self.pending_start_cwd.as_deref()
    }

    pub(crate) fn set_draft_cwd(&mut self, cwd: Option<String>) {
        self.draft_cwd = cwd.and_then(|raw| {
            let trimmed = raw.trim().to_string();
            (!trimmed.is_empty()).then_some(trimmed)
        });
    }

    pub(crate) fn take_draft_cwd(&mut self) -> Option<String> {
        let taken = self.draft_cwd.take();
        self.pending_start_cwd = taken.clone();
        taken
    }

    pub(crate) fn models(&self) -> &[ModelEntry] {
        &self.models
    }

    pub(crate) fn stats(&self) -> Option<&SessionStats> {
        self.stats.as_ref()
    }

    pub(crate) fn activity_counts(&self) -> (usize, usize, usize) {
        (self.subagent_count, self.heartbeat_count, self.shell_count)
    }

    pub(crate) fn active_session(&self) -> Option<&SessionRow> {
        let sid = self.active_session_id.as_deref()?;
        self.sessions
            .iter()
            .find(|s| row_matches_selector(s, sid))
    }

    /// Update the roster from a daemon snapshot. Returns a session key to auto-select
    /// when no session is currently active and the user is not composing a draft chat.
    pub(crate) fn apply_sessions(&mut self, mut rows: Vec<SessionRow>) -> Option<String> {
        let active_was_live = self
            .active_session()
            .is_some_and(|r| r.is_resident() && r.lifecycle != "archived");
        if active_was_live {
            if let Some(sid) = self.active_session_id.as_deref() {
                if let Some(row) = rows.iter_mut().find(|r| row_matches_selector(r, sid)) {
                    if !row.is_resident() || row.lifecycle == "archived" {
                        row.worker_state = "ready".into();
                        row.lifecycle = "live".into();
                        if row.activity.is_empty() {
                            row.activity = "idle".into();
                        }
                    }
                }
            }
        }
        self.sessions = rows;
        if self.active_session().is_some() {
            self.pending_start_cwd = None;
        }
        if self.active_session_id.is_none() && !self.draft_chat {
            let auto = self.sessions.first().map(session_key);
            if auto.is_some() {
                self.draft_cwd = None;
                self.pending_start_cwd = None;
            }
            auto
        } else {
            None
        }
    }

    /// Whether the currently active session row reports live streaming in the roster.
    pub(crate) fn active_row_is_streaming(&self) -> Option<bool> {
        self.active_session().map(|row| row.is_streaming)
    }

    pub(crate) fn apply_models(&mut self, models: Vec<ModelEntry>, settings: &AppSettings) {
        self.models = models;
        if self.draft_chat && !self.draft_model_picked {
            if let Some(first) = self.models.iter().find(|m| m.configured) {
                let saved = settings
                    .model_thinking_for(&first.provider, &first.id)
                    .map(|lvl| {
                        if first.thinking_levels.is_empty() {
                            lvl.to_string()
                        } else {
                            clamp_level(&first.thinking_levels, lvl)
                        }
                    })
                    .unwrap_or_else(|| {
                        first
                            .thinking_levels
                            .first()
                            .cloned()
                            .unwrap_or_default()
                    });
                self.session_state = Some(SessionState {
                    provider: first.provider.clone(),
                    model_id: first.id.clone(),
                    thinking_level: saved,
                    ..SessionState::default()
                });
            }
        }
    }

    pub(crate) fn set_workspace_binding(
        &mut self,
        session: &str,
        binding: Option<pa_client::WorkspaceBinding>,
    ) {
        if let Some(row) = self
            .sessions
            .iter_mut()
            .find(|r| row_matches_selector(r, session))
        {
            if let Some(ref b) = binding {
                row.cwd = b.worktree_path.clone();
            }
            row.workspace_binding = binding;
        }
    }

    pub(crate) fn apply_session_state(&mut self, session: &str, state: SessionState) {
        if let Some(binding) = state.workspace_binding.clone() {
            self.set_workspace_binding(session, Some(binding));
        }
        if self.active_session_id.as_deref() == Some(session) {
            self.session_state = Some(state);
        }
    }

    pub(crate) fn apply_stats(&mut self, session: &str, stats: SessionStats) {
        if self.active_session_id.as_deref() == Some(session) {
            self.stats = Some(stats);
        }
    }

    pub(crate) fn apply_activity(&mut self, subagents: usize, heartbeats: usize, shells: usize) {
        self.subagent_count = subagents;
        self.heartbeat_count = heartbeats;
        self.shell_count = shells;
    }

    pub(crate) fn begin_draft(&mut self) {
        let cur = self.current_model();
        self.draft_chat = true;
        self.draft_model_picked = false;
        self.draft_cwd = None;
        self.pending_start_cwd = None;
        self.active_session_id = None;
        self.stats = None;
        self.subagent_count = 0;
        self.heartbeat_count = 0;
        self.shell_count = 0;
        if !cur.provider.is_empty() && !cur.model_id.is_empty() {
            self.session_state = Some(SessionState {
                provider: cur.provider,
                model_id: cur.model_id,
                thinking_level: cur.thinking,
                ..SessionState::default()
            });
        } else {
            self.session_state = None;
        }
    }

    pub(crate) fn clear_draft(&mut self) {
        self.draft_chat = false;
        self.draft_model_picked = false;
        self.draft_cwd = None;
    }

    pub(crate) fn select_session_optimistic(&mut self, key: &str) -> bool {
        self.draft_chat = false;
        self.draft_model_picked = false;
        self.draft_cwd = None;
        self.pending_start_cwd = None;
        let same_active = self
            .active_session()
            .is_some_and(|r| row_matches_selector(r, key))
            || self.active_session_id.as_deref() == Some(key);
        if !same_active {
            self.session_state = None;
            self.stats = None;
            self.subagent_count = 0;
            self.heartbeat_count = 0;
            self.shell_count = 0;
        }
        self.active_session_id = Some(key.to_string());
        let mut promoted_from_done = false;
        if let Some(row) = self
            .sessions
            .iter_mut()
            .find(|s| row_matches_selector(s, key))
        {
            if !row.is_resident() || row.lifecycle == "archived" {
                row.worker_state = "ready".into();
                row.lifecycle = "live".into();
                row.activity = "idle".into();
                row.is_session_active = false;
                promoted_from_done = true;
            }
        }
        promoted_from_done
    }

    pub(crate) fn rebind_session_id(&mut self, requested: &str, resolved: &str) {
        self.draft_chat = false;
        self.draft_model_picked = false;
        self.draft_cwd = None;
        self.pending_start_cwd = None;
        let switched = self.active_session_id.as_deref() != Some(requested)
            && self.active_session_id.as_deref() != Some(resolved)
            && !self
                .active_session()
                .is_some_and(|r| row_matches_selector(r, requested) || row_matches_selector(r, resolved));
        if switched {
            self.session_state = None;
            self.stats = None;
            self.subagent_count = 0;
            self.heartbeat_count = 0;
            self.shell_count = 0;
        }
        self.active_session_id = Some(resolved.to_string());
        if requested != resolved {
            if let Some(row) = self
                .sessions
                .iter_mut()
                .find(|s| row_matches_selector(s, requested) || row_matches_selector(s, resolved))
            {
                if row.session_id.is_empty() && !row.id.is_empty() && row.id != resolved {
                    row.session_id = row.id.clone();
                }
                row.id = resolved.to_string();
                if !row.is_resident() || row.lifecycle == "archived" {
                    row.worker_state = "ready".into();
                    row.lifecycle = "live".into();
                    row.activity = "idle".into();
                    row.is_session_active = false;
                }
            }
        }
    }

    pub(crate) fn mark_active_streaming(&mut self) {
        let Some(sid) = self.active_session_id.clone() else {
            return;
        };
        if let Some(row) = self.sessions.iter_mut().find(|s| session_key(s) == sid) {
            if row.worker_state.is_empty() {
                row.worker_state = "ready".into();
            }
            row.lifecycle = "live".into();
            row.activity = "working".into();
            row.is_session_active = true;
            row.is_streaming = true;
        }
    }

    pub(crate) fn mark_stopped(&mut self, id: &str) {
        if let Some(row) = self
            .sessions
            .iter_mut()
            .find(|s| s.id == id || session_key(s) == id)
        {
            row.worker_state.clear();
            row.lifecycle = "archived".into();
            row.activity = "idle".into();
            row.is_streaming = false;
            row.is_session_active = false;
        }
    }

    pub(crate) fn rename(&mut self, key: &str, name: String) {
        if let Some(row) = self.sessions.iter_mut().find(|s| session_key(s) == key) {
            row.name = name;
        }
    }

    pub(crate) fn consume_draft_model_for_start(
        &mut self,
    ) -> (Option<(String, String)>, Option<String>) {
        let cur = self.current_model();
        let model = (!cur.provider.is_empty() && !cur.model_id.is_empty())
            .then_some((cur.provider, cur.model_id));
        let level = model
            .as_ref()
            .and_then(|_| (!cur.thinking.is_empty()).then_some(cur.thinking));
        self.draft_chat = false;
        self.draft_model_picked = false;
        (model, level)
    }

    pub(crate) fn pick_model(
        &mut self,
        provider: &str,
        model_id: &str,
        levels: &[String],
        settings: &AppSettings,
    ) -> ModelPickEffect {
        let saved_level = settings
            .model_thinking_for(provider, model_id)
            .map(|lvl| {
                if levels.is_empty() {
                    lvl.to_string()
                } else {
                    clamp_level(levels, lvl)
                }
            });
        let st = self.session_state.get_or_insert_with(SessionState::default);
        st.provider = provider.to_string();
        st.model_id = model_id.to_string();
        if let Some(level) = &saved_level {
            st.thinking_level = level.clone();
        } else if !levels.is_empty() {
            st.thinking_level = clamp_level(levels, &st.thinking_level);
        }

        if self.draft_chat || self.active_session_id.is_none() {
            self.draft_model_picked = true;
            ModelPickEffect::DraftOnly
        } else if let Some(sid) = self.active_session_id.clone() {
            if let Some(level) = saved_level {
                ModelPickEffect::SetModelAndThinking {
                    session: sid,
                    provider: provider.to_string(),
                    model_id: model_id.to_string(),
                    level,
                }
            } else {
                ModelPickEffect::SetModel {
                    session: sid,
                    provider: provider.to_string(),
                    model_id: model_id.to_string(),
                }
            }
        } else {
            ModelPickEffect::None
        }
    }

    pub(crate) fn pick_effort(
        &mut self,
        provider: &str,
        model_id: &str,
        level: &str,
        settings: &mut AppSettings,
    ) -> Option<String> {
        settings.set_model_thinking(provider, model_id, level);
        let st = self.session_state.get_or_insert_with(SessionState::default);
        st.provider = provider.to_string();
        st.model_id = model_id.to_string();
        st.thinking_level = level.to_string();
        if self.draft_chat || self.active_session_id.is_none() {
            self.draft_model_picked = true;
            None
        } else {
            self.active_session_id.clone()
        }
    }

    pub(crate) fn current_model(&self) -> ResolvedModel {
        let row = self.active_session();
        let state = self.session_state.as_ref();

        let provider = state
            .map(|s| s.provider.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.model_provider.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let model_id = state
            .map(|s| s.model_id.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.model.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let thinking = state
            .map(|s| s.thinking_level.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.thinking.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let label = self
            .models
            .iter()
            .find(|m| m.id == model_id && (provider.is_empty() || m.provider == provider))
            .map(|m| model_label(&m.name, &m.id))
            .or_else(|| {
                row.filter(|s| !s.model_name.is_empty())
                    .map(|s| model_label(&s.model_name, &s.model))
            })
            .or_else(|| (!model_id.is_empty()).then(|| model_label("", &model_id)))
            .unwrap_or_else(|| "Model".to_string());

        ResolvedModel {
            provider,
            model_id,
            label,
            thinking,
        }
    }

    pub(crate) fn grouped_sessions<'a>(&'a self, raw_query: &str) -> GroupedSessions<'a> {
        let query = raw_query.trim().to_ascii_lowercase();
        let matches_query = |s: &&SessionRow| -> bool {
            if query.is_empty() {
                return true;
            }
            s.name.to_ascii_lowercase().contains(&query)
                || s.id.to_ascii_lowercase().contains(&query)
                || s.cwd.to_ascii_lowercase().contains(&query)
        };
        let is_top_level = |s: &&SessionRow| -> bool {
            s.depth == 0 && s.parent.is_empty() && s.parent_active.is_empty()
        };

        let active = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| s.roster_status() == "Running")
            .filter(matches_query)
            .collect();
        let idle = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| s.roster_status() == "Idle")
            .filter(matches_query)
            .collect();
        let done = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| !matches!(s.roster_status(), "Running" | "Idle"))
            .filter(matches_query)
            .take(60)
            .collect();

        GroupedSessions { active, idle, done }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, lifecycle: &str, activity: &str, worker_state: &str) -> SessionRow {
        SessionRow {
            id: id.into(),
            name: name.into(),
            lifecycle: lifecycle.into(),
            activity: activity.into(),
            worker_state: worker_state.into(),
            ..SessionRow::default()
        }
    }

    #[test]
    fn roster_groups_sessions_auto_selects_and_tracks_optimistic_transitions() {
        let mut roster = SessionRoster::new();
        let auto = roster.apply_sessions(vec![
            row("s1", "Alpha", "live", "working", "ready"),
            row("s2", "Beta", "live", "idle", "ready"),
            row("s3", "Gamma", "archived", "idle", ""),
        ]);
        assert_eq!(auto.as_deref(), Some("s1"));
        roster.set_active_id(auto);

        let grouped = roster.grouped_sessions("");
        assert_eq!(grouped.active.len(), 1);
        assert_eq!(grouped.idle.len(), 1);
        assert_eq!(grouped.done.len(), 1);

        let filtered = roster.grouped_sessions("bet");
        assert!(filtered.active.is_empty());
        assert_eq!(filtered.idle.len(), 1);
        assert!(filtered.done.is_empty());

        roster.rename("s1", "Alpha Renamed".into());
        assert_eq!(roster.active_session().unwrap().name, "Alpha Renamed");

        roster.mark_stopped("s1");
        assert_eq!(roster.grouped_sessions("").active.len(), 0);
        assert_eq!(roster.grouped_sessions("").done.len(), 2);
    }

    #[test]
    fn draft_chat_preserves_model_and_emits_draft_only_pick_effect() {
        let mut roster = SessionRoster::new();
        let settings = AppSettings::default();
        roster.begin_draft();
        assert!(roster.is_draft());
        assert!(roster.active_id().is_none());

        let effect = roster.pick_model("openai", "gpt-5", &["low".into(), "high".into()], &settings);
        assert!(matches!(effect, ModelPickEffect::DraftOnly));
        let cur = roster.current_model();
        assert_eq!(cur.provider, "openai");
        assert_eq!(cur.model_id, "gpt-5");

        let (model, _level) = roster.consume_draft_model_for_start();
        assert_eq!(model, Some(("openai".into(), "gpt-5".into())));
        assert!(!roster.is_draft());
    }

    #[::core::prelude::v1::test]
    fn draft_cwd_is_set_taken_on_start_and_cleared_on_session_switch() {
        let mut roster = SessionRoster::new();
        roster.begin_draft();
        assert_eq!(roster.draft_cwd(), None);

        roster.set_draft_cwd(Some("  /repo/demo  ".into()));
        assert_eq!(roster.draft_cwd(), Some("/repo/demo"));
        assert_eq!(roster.take_draft_cwd().as_deref(), Some("/repo/demo"));
        assert_eq!(roster.draft_cwd(), None);

        roster.begin_draft();
        roster.set_draft_cwd(Some("/repo/other".into()));
        roster.select_session_optimistic("sess-1");
        assert_eq!(roster.draft_cwd(), None);
    }

    #[::core::prelude::v1::test]
    fn switching_sessions_clears_stale_state_uses_row_model_and_rebinds_archived_id() {
        let mut roster = SessionRoster::new();
        let mut s1 = row("s1", "One", "live", "idle", "ready");
        s1.model_provider = "openai".into();
        s1.model = "gpt-5".into();
        let mut s2 = row("", "Archived", "archived", "idle", "");
        s2.session_file = "/tmp/archived.jsonl".into();
        s2.model_provider = "anthropic".into();
        s2.model = "claude-sonnet-4".into();
        s2.thinking = "high".into();

        let auto = roster.apply_sessions(vec![s1, s2]);
        roster.set_active_id(auto);
        roster.apply_session_state(
            "s1",
            SessionState {
                provider: "openai".into(),
                model_id: "gpt-5".into(),
                thinking_level: "low".into(),
                is_streaming: false,
                workspace_binding: None,
            },
        );
        roster.apply_stats(
            "s1",
            SessionStats {
                context_tokens: 42_000,
                context_window: 200_000,
                percent: 21.0,
                ..SessionStats::default()
            },
        );
        roster.apply_activity(2, 1, 3);

        roster.select_session_optimistic("/tmp/archived.jsonl");
        assert_eq!(roster.active_id(), Some("/tmp/archived.jsonl"));
        assert!(roster.stats().is_none());
        assert_eq!(roster.activity_counts(), (0, 0, 0));
        let cur = roster.current_model();
        assert_eq!(cur.provider, "anthropic");
        assert_eq!(cur.model_id, "claude-sonnet-4");
        assert_eq!(cur.thinking, "high");

        roster.rebind_session_id("/tmp/archived.jsonl", "s2-live");
        assert_eq!(roster.active_id(), Some("s2-live"));
        assert_eq!(roster.active_session().map(|r| r.id.as_str()), Some("s2-live"));
    }

    #[::core::prelude::v1::test]
    fn selecting_done_session_optimistically_promotes_to_live_and_matches_session_id() {
        let mut roster = SessionRoster::new();
        let mut s_done = row("uuid-done", "Finished Work", "live", "idle", "");
        s_done.session_id = "uuid-done".into();
        s_done.session_file = "/tmp/uuid-done.jsonl".into();
        roster.apply_sessions(vec![s_done.clone()]);
        assert_eq!(roster.grouped_sessions("").done.len(), 1);
        assert_eq!(roster.grouped_sessions("").idle.len(), 0);

        let promoted = roster.select_session_optimistic("uuid-done");
        assert!(promoted);
        assert_eq!(roster.grouped_sessions("").done.len(), 0);
        assert_eq!(roster.grouped_sessions("").idle.len(), 1);

        // An in-flight stale list before worker registration must not flicker active row back to Done.
        roster.apply_sessions(vec![s_done]);
        assert_eq!(roster.grouped_sessions("").done.len(), 0);
        assert_eq!(roster.grouped_sessions("").idle.len(), 1);
        assert!(row_matches_selector(roster.active_session().unwrap(), "uuid-done"));
    }
}
