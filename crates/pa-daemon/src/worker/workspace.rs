//! Worker-side Workspace binding, Worktree switching, event-driven `.git/HEAD`
//! branch refresh, and live harness-digest invalidation.

use std::path::Path;

use pa_types::daemon::{WorkspaceBinding, WorkspaceKind};
use serde_json::{json, Value};

use super::{response_failure, response_success, Worker};
use crate::protocol::DaemonResponse;

/// Shared `.git/HEAD` refresh helper callable from both `Worker` and `TurnRunner`.
pub(crate) fn refresh_workspace_branch_with(
    core: &std::sync::Arc<std::sync::Mutex<super::SessionCore>>,
    events: &std::sync::Arc<super::EventPump>,
    engine: &dyn super::SessionEngine,
) -> Option<WorkspaceBinding> {
    let current = core.lock().unwrap().workspace_binding.clone()?;
    if current.workspace_kind != WorkspaceKind::Git {
        return Some(current);
    }
    let wt_path = Path::new(&current.worktree_path);
    if !wt_path.is_dir() {
        return Some(current);
    }
    let (new_branch, _) = pa_core::workspace::read_worktree_head(wt_path);
    if new_branch == current.branch {
        return Some(current);
    }
    let mut updated = current;
    updated.branch = new_branch;
    {
        let mut guard = core.lock().unwrap();
        if let Some(store) = guard.store.as_mut() {
            let _ = store.persist_entry(
                "custom",
                json!({
                    "customType": pa_core::workspace::WORKSPACE_BINDING_CUSTOM_TYPE,
                    "data": updated,
                }),
            );
        }
        guard.workspace_binding = Some(updated.clone());
    }
    engine.set_workspace_binding(Some(updated.clone()));
    crate::user_bash::emit_session_event_frame(
        core,
        events,
        json!({ "type": "workspace_changed", "binding": updated }),
    );
    Some(updated)
}

impl Worker {
    /// Persist `binding` to the session store, sync `core` and engine state,
    /// move any live kernel to the bound worktree if requested, and emit
    /// `workspace_changed` to attached clients.
    pub(crate) fn apply_workspace_binding(
        &self,
        binding: WorkspaceBinding,
        chdir_kernel: bool,
    ) -> anyhow::Result<WorkspaceBinding> {
        let wt_path = binding.worktree_path.clone();
        {
            let mut core = self.core.lock().unwrap();
            if let Some(store) = core.store.as_mut() {
                store.persist_entry(
                    "custom",
                    json!({
                        "customType": pa_core::workspace::WORKSPACE_BINDING_CUSTOM_TYPE,
                        "data": binding,
                    }),
                )?;
            }
            core.cwd.clone_from(&wt_path);
            core.workspace_binding = Some(binding.clone());
        }
        self.engine.set_cwd(std::path::PathBuf::from(&wt_path));
        self.engine.set_workspace_binding(Some(binding.clone()));
        if chdir_kernel {
            self.engine.chdir_live_kernel(Path::new(&wt_path));
        }
        crate::user_bash::emit_session_event_frame(
            &self.core,
            &self.events,
            json!({
                "type": "workspace_changed",
                "binding": binding,
            }),
        );
        self.push_roster_delta();
        Ok(binding)
    }

    /// `session_bind_workspace { workspaceId?, path?, worktreePath? }`:
    /// bind a Scratch session to a Workspace (refuses rebinding an already-bound
    /// session to a different Workspace).
    pub(crate) async fn handle_session_bind_workspace(&self, payload: &Value) -> DaemonResponse {
        if let Err(response) = self.require_created("session_bind_workspace") {
            return response;
        }
        let existing_ws_id = {
            let core = self.core.lock().unwrap();
            core.workspace_binding
                .as_ref()
                .map(|b| b.workspace_id.clone())
        };
        let agent_dir = self.config.agent_dir.clone();
        let ws_id = payload
            .get("workspaceId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let path_str = payload
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let wt_str = payload
            .get("worktreePath")
            .and_then(Value::as_str)
            .map(str::to_owned);

        let resolved = tokio::task::spawn_blocking(move || -> anyhow::Result<WorkspaceBinding> {
            let (ws, default_binding) = if let Some(id) = ws_id {
                let ws = pa_core::workspace::find_workspace(&agent_dir, &id)
                    .ok_or_else(|| anyhow::anyhow!("unknown workspace: {id}"))?;
                let _ = pa_core::workspace::touch_workspace(&agent_dir, &id);
                let wt = wt_str
                    .as_deref()
                    .map(Path::new)
                    .unwrap_or_else(|| Path::new(&ws.root_path));
                let b = pa_core::workspace::binding_for_worktree(&ws, wt)?;
                (ws, b)
            } else if let Some(raw_path) = path_str {
                let expanded = crate::paths::expand_tilde(&raw_path)?;
                let (ws, b) = pa_core::workspace::register_workspace(&agent_dir, &expanded, None)?;
                let b = if let Some(wt) = wt_str.as_deref() {
                    pa_core::workspace::binding_for_worktree(&ws, Path::new(wt))?
                } else {
                    b
                };
                (ws, b)
            } else {
                anyhow::bail!("session_bind_workspace requires workspaceId or path");
            };
            if let Some(cur_id) = existing_ws_id {
                if cur_id != ws.id {
                    anyhow::bail!(
                        "Session is already bound to Workspace {cur_id}; start a new session for a different Workspace"
                    );
                }
            }
            Ok(default_binding)
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("bind task failed: {e}")));

        match resolved.and_then(|b| self.apply_workspace_binding(b, true)) {
            Ok(binding) => response_success(
                None,
                "session_bind_workspace",
                Some(json!({ "binding": binding })),
            ),
            Err(error) => response_failure(None, "session_bind_workspace", &error.to_string(), None),
        }
    }

    /// `session_switch_worktree { worktreePath }`:
    /// switch a bound session to another Worktree of its Workspace.
    pub(crate) async fn handle_session_switch_worktree(&self, payload: &Value) -> DaemonResponse {
        if let Err(response) = self.require_created("session_switch_worktree") {
            return response;
        }
        let Some(wt_raw) = payload.get("worktreePath").and_then(Value::as_str) else {
            return response_failure(
                None,
                "session_switch_worktree",
                "session_switch_worktree requires worktreePath",
                None,
            );
        };
        let Some(current) = self.core.lock().unwrap().workspace_binding.clone() else {
            return response_failure(
                None,
                "session_switch_worktree",
                "Scratch session is not bound to a Workspace",
                None,
            );
        };
        let agent_dir = self.config.agent_dir.clone();
        let wt_raw = wt_raw.to_owned();
        let resolved = tokio::task::spawn_blocking(move || -> anyhow::Result<WorkspaceBinding> {
            let ws = pa_core::workspace::find_workspace(&agent_dir, &current.workspace_id)
                .ok_or_else(|| anyhow::anyhow!("unknown workspace: {}", current.workspace_id))?;
            let target = crate::paths::expand_tilde(&wt_raw)?;
            let binding = pa_core::workspace::binding_for_worktree(&ws, &target)?;
            let _ = pa_core::workspace::touch_workspace(&agent_dir, &ws.id);
            Ok(binding)
        })
        .await
        .unwrap_or_else(|e| Err(anyhow::anyhow!("switch worktree task failed: {e}")));

        match resolved.and_then(|b| self.apply_workspace_binding(b, true)) {
            Ok(binding) => response_success(
                None,
                "session_switch_worktree",
                Some(json!({ "binding": binding })),
            ),
            Err(error) => response_failure(None, "session_switch_worktree", &error.to_string(), None),
        }
    }

    /// Lightweight `.git/HEAD` check on the bound worktree: updates cached branch
    /// and emits `workspace_changed` whenever the branch changed.
    pub(crate) fn maybe_refresh_workspace_branch(&self) -> Option<WorkspaceBinding> {
        let current = self.core.lock().unwrap().workspace_binding.clone()?;
        if current.workspace_kind != WorkspaceKind::Git {
            return Some(current);
        }
        let wt_path = Path::new(&current.worktree_path);
        if !wt_path.is_dir() {
            return Some(current);
        }
        let (new_branch, _) = pa_core::workspace::read_worktree_head(wt_path);
        if new_branch == current.branch {
            return Some(current);
        }
        let mut updated = current;
        updated.branch = new_branch;
        {
            let mut core = self.core.lock().unwrap();
            if let Some(store) = core.store.as_mut() {
                let _ = store.persist_entry(
                    "custom",
                    json!({
                        "customType": pa_core::workspace::WORKSPACE_BINDING_CUSTOM_TYPE,
                        "data": updated,
                    }),
                );
            }
            core.workspace_binding = Some(updated.clone());
        }
        self.engine.set_workspace_binding(Some(updated.clone()));
        crate::user_bash::emit_session_event_frame(
            &self.core,
            &self.events,
            json!({
                "type": "workspace_changed",
                "binding": updated,
            }),
        );
        self.push_roster_delta();
        Some(updated)
    }

    /// `session_refresh_workspace`: re-reads `.git/HEAD` on the bound worktree (triggered on
    /// window focus or after branch switch).
    pub(crate) fn handle_session_refresh_workspace(&self) -> DaemonResponse {
        if let Err(response) = self.require_created("session_refresh_workspace") {
            return response;
        }
        let binding = self.maybe_refresh_workspace_branch();
        response_success(
            None,
            "session_refresh_workspace",
            Some(json!({ "binding": binding })),
        )
    }

    /// Internal supervisor notification: mark this session's `[harness-digest]` dirty so the
    /// next turn picks up changes made from Settings or another session.
    pub(crate) fn handle_harness_digest_dirty(&self) -> DaemonResponse {
        self.engine.mark_harness_digest_dirty();
        response_success(None, "harness_digest_dirty", None)
    }

}
