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
        self.sessions.iter().find(|s| session_key(s) == sid)
    }

    /// Update the roster from a daemon snapshot. Returns a session key to auto-select
    /// when no session is currently active and the user is not composing a draft chat.
    pub(crate) fn apply_sessions(&mut self, rows: Vec<SessionRow>) -> Option<String> {
        self.sessions = rows;
        if self.active_session_id.is_none() && !self.draft_chat {
            self.sessions.first().map(session_key)
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

    pub(crate) fn apply_session_state(&mut self, session: &str, state: SessionState) {
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
    }

    pub(crate) fn select_session_optimistic(&mut self, key: &str) {
        self.draft_chat = false;
        self.draft_model_picked = false;
        if let Some(row) = self.sessions.iter_mut().find(|s| session_key(s) == key) {
            if !row.is_resident() || row.lifecycle == "archived" {
                row.worker_state = "ready".into();
                row.lifecycle = "live".into();
                row.activity = "idle".into();
                row.is_session_active = false;
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
}
