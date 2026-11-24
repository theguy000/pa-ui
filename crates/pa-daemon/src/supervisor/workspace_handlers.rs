//! Control-plane handlers for Workspaces, Worktrees, Branches, and
//! three-scope continual harness CRUD / History / multi-scope Rollback.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use pa_core::refinement::crud::{
    delete_entry, list_entries, list_history, rollback_refinement, upsert_entry, HarnessScopeDirs,
    UpsertHarnessInput,
};
use pa_types::daemon::{DaemonCommand, DaemonResponse, HarnessScopeWire};
use serde_json::json;

use super::{
    response_failure, response_success, ClientRouting, RouteAdmission, Supervisor,
    ROUTE_TIMEOUT_MS,
};

impl Supervisor {
    pub(crate) fn broadcast_workspaces_changed(&self) {
        let _ = self.events.send((
            ClientRouting::Broadcast,
            Arc::new(json!({ "type": "workspaces_changed" })),
        ));
    }

    pub(crate) fn broadcast_harness_changed(
        &self,
        scope: HarnessScopeWire,
        workspace_id: Option<&str>,
        active_session_id: Option<&str>,
    ) {
        let mut payload = json!({
            "type": "harness_changed",
            "scope": scope,
        });
        if let Some(ws) = workspace_id {
            payload["workspaceId"] = json!(ws);
        }
        if let Some(sid) = active_session_id {
            payload["activeSessionId"] = json!(sid);
        }
        let _ = self.events.send((ClientRouting::Broadcast, Arc::new(payload)));
    }

    /// Notify live workers affected by a harness mutation so their next turn
    /// re-checks the `[harness-digest]` fingerprint automatically.
    async fn notify_workers_harness_dirty(
        &self,
        scope: HarnessScopeWire,
        active_session_id: Option<&str>,
    ) {
        let residents = match (scope, active_session_id) {
            (HarnessScopeWire::Session, Some(sid)) => match self.registry.resolve(sid).await {
                Ok(r) => vec![r],
                Err(_) => Vec::new(),
            },
            _ => self.registry.list().await,
        };
        for resident in residents {
            let _ = self
                .route_command_typed(
                    &resident,
                    "harness_digest_dirty",
                    json!({}),
                    ROUTE_TIMEOUT_MS,
                    RouteAdmission::SupervisorInternal,
                )
                .await;
        }
    }

    async fn resolve_session_harness_context(
        &self,
        active_session_id: Option<&str>,
    ) -> (Option<String>, Option<PathBuf>, Vec<pa_core::refinement::RefinementResult>) {
        let Some(sid) = active_session_id else {
            return (None, None, Vec::new());
        };
        let session_file: Option<PathBuf> = if let Ok(resident) = self.registry.resolve(sid).await {
            resident
                .descriptor
                .lock()
                .await
                .session_file
                .as_ref()
                .map(PathBuf::from)
        } else {
            let direct = PathBuf::from(sid);
            if direct.is_file() {
                Some(direct)
            } else {
                let local_candidate = self
                    .options
                    .agent_dir
                    .join("sessions")
                    .join(format!("{sid}.jsonl"));
                if local_candidate.is_file() {
                    Some(local_candidate)
                } else {
                    crate::paths::sessions_dir(&self.options.agent_dir)
                        .ok()
                        .map(|dir| dir.join(format!("{sid}.jsonl")))
                        .filter(|p| p.is_file())
                }
            }
        };
        let Some(file_path) = session_file else {
            return (Some(sid.to_owned()), None, Vec::new());
        };
        let harness_dir = pa_core::session_engine::harness_digest::local_harness_dir_for_log(&file_path);
        let history = tokio::task::spawn_blocking(move || {
            crate::session_store::SessionFile::open(&file_path)
                .map(|sf| sf.refinement_history())
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        (Some(sid.to_owned()), harness_dir, history)
    }

    pub(crate) async fn handle_workspace_and_harness_command(
        &self,
        command: &DaemonCommand,
        command_id: Option<&str>,
        type_name: &str,
    ) -> Option<DaemonResponse> {
        let agent_dir = self.options.agent_dir.clone();
        match command {
            DaemonCommand::WorkspaceList { .. } => {
                let res = tokio::task::spawn_blocking(move || {
                    let workspaces = pa_core::workspace::list_workspaces(&agent_dir);
                    let worktree_root = pa_core::workspace::configured_worktree_root(&agent_dir)
                        .to_string_lossy()
                        .into_owned();
                    json!({
                        "workspaces": workspaces,
                        "worktreeRoot": worktree_root,
                    })
                })
                .await;
                Some(match res {
                    Ok(data) => response_success(command_id, type_name, Some(data)),
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::WorkspaceRegister { path, name, .. } => {
                let path = path.clone();
                let name = name.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let expanded = crate::paths::expand_tilde(&path)?;
                    pa_core::workspace::register_workspace(&agent_dir, &expanded, name.as_deref())
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("register task failed: {e}")));
                Some(match res {
                    Ok((workspace, binding)) => {
                        self.broadcast_workspaces_changed();
                        response_success(
                            command_id,
                            type_name,
                            Some(json!({ "workspace": workspace, "binding": binding })),
                        )
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::WorkspaceRelink {
                workspace_id,
                new_path,
                ..
            } => {
                let workspace_id = workspace_id.clone();
                let new_path = new_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let expanded = crate::paths::expand_tilde(&new_path)?;
                    pa_core::workspace::relink_workspace(&agent_dir, &workspace_id, &expanded)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("relink task failed: {e}")));
                Some(match res {
                    Ok(workspace) => {
                        self.broadcast_workspaces_changed();
                        response_success(command_id, type_name, Some(json!({ "workspace": workspace })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::WorkspaceDelete {
                workspace_id,
                delete_harness,
                ..
            } => {
                let workspace_id = workspace_id.clone();
                let delete_harness = delete_harness.unwrap_or(true);
                let res = tokio::task::spawn_blocking(move || {
                    pa_core::workspace::delete_workspace(&agent_dir, &workspace_id, delete_harness)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("delete task failed: {e}")));
                Some(match res {
                    Ok(deleted) => {
                        if deleted {
                            self.broadcast_workspaces_changed();
                        }
                        response_success(command_id, type_name, Some(json!({ "deleted": deleted })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::WorktreeList { workspace_id, .. } => {
                let workspace_id = workspace_id.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let ws = pa_core::workspace::find_workspace(&agent_dir, &workspace_id)
                        .ok_or_else(|| anyhow::anyhow!("unknown workspace: {workspace_id}"))?;
                    pa_core::workspace::list_worktrees(&ws)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("worktree_list task failed: {e}")));
                Some(match res {
                    Ok(worktrees) => response_success(
                        command_id,
                        type_name,
                        Some(json!({ "worktrees": worktrees })),
                    ),
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::WorktreeCreate {
                workspace_id,
                name,
                base_branch,
                ..
            } => {
                let workspace_id = workspace_id.clone();
                let name = name.clone();
                let base_branch = base_branch.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let ws = pa_core::workspace::find_workspace(&agent_dir, &workspace_id)
                        .ok_or_else(|| anyhow::anyhow!("unknown workspace: {workspace_id}"))?;
                    pa_core::workspace::create_worktree(
                        &agent_dir,
                        &ws,
                        &name,
                        base_branch.as_deref(),
                    )
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("worktree_create task failed: {e}")));
                Some(match res {
                    Ok(worktree) => {
                        self.broadcast_workspaces_changed();
                        response_success(
                            command_id,
                            type_name,
                            Some(json!({ "worktree": worktree })),
                        )
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::BranchList { worktree_path, .. } => {
                let worktree_path = worktree_path.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let expanded = crate::paths::expand_tilde(&worktree_path)?;
                    pa_core::workspace::list_branches(&expanded)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("branch_list task failed: {e}")));
                Some(match res {
                    Ok(branches) => {
                        response_success(command_id, type_name, Some(json!({ "branches": branches })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::BranchSwitch {
                worktree_path,
                branch,
                active_session_id,
                ..
            } => {
                let wt = worktree_path.clone();
                let br = branch.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let expanded = crate::paths::expand_tilde(&wt)?;
                    pa_core::workspace::switch_branch(&expanded, &br)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("branch_switch task failed: {e}")));
                if let Ok(ref current) = res {
                    if let Some(sid) = active_session_id.as_deref() {
                        if let Ok(resident) = self.registry.resolve(sid).await {
                            let _ = self
                                .route_command_typed(
                                    &resident,
                                    "session_refresh_workspace",
                                    json!({}),
                                    ROUTE_TIMEOUT_MS,
                                    RouteAdmission::SupervisorInternal,
                                )
                                .await;
                        }
                    }
                    return Some(response_success(
                        command_id,
                        type_name,
                        Some(json!({ "branch": current })),
                    ));
                }
                Some(response_failure(
                    command_id,
                    type_name,
                    &res.unwrap_err().to_string(),
                    None,
                ))
            }
            DaemonCommand::BranchCreate {
                worktree_path,
                branch,
                start_point,
                active_session_id,
                ..
            } => {
                let wt = worktree_path.clone();
                let br = branch.clone();
                let sp = start_point.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let expanded = crate::paths::expand_tilde(&wt)?;
                    pa_core::workspace::create_branch(&expanded, &br, sp.as_deref())
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("branch_create task failed: {e}")));
                if let Ok(ref current) = res {
                    if let Some(sid) = active_session_id.as_deref() {
                        if let Ok(resident) = self.registry.resolve(sid).await {
                            let _ = self
                                .route_command_typed(
                                    &resident,
                                    "session_refresh_workspace",
                                    json!({}),
                                    ROUTE_TIMEOUT_MS,
                                    RouteAdmission::SupervisorInternal,
                                )
                                .await;
                        }
                    }
                    return Some(response_success(
                        command_id,
                        type_name,
                        Some(json!({ "branch": current })),
                    ));
                }
                Some(response_failure(
                    command_id,
                    type_name,
                    &res.unwrap_err().to_string(),
                    None,
                ))
            }
            DaemonCommand::HarnessList {
                scope,
                workspace_id,
                active_session_id,
                kind,
                query,
                ..
            } => {
                let (sid_opt, sess_hdir, _) = self
                    .resolve_session_harness_context(active_session_id.as_deref())
                    .await;
                let ws_id = workspace_id.clone();
                let scope = *scope;
                let kind = *kind;
                let query = query.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let dirs = HarnessScopeDirs {
                        agent_dir: &agent_dir,
                        workspace_id: ws_id.as_deref(),
                        session_id: sid_opt.as_deref(),
                        session_harness_dir: sess_hdir.as_deref(),
                    };
                    list_entries(&dirs, scope, kind, query.as_deref())
                })
                .await;
                Some(match res {
                    Ok(entries) => {
                        response_success(command_id, type_name, Some(json!({ "entries": entries })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::HarnessUpsert {
                scope,
                workspace_id,
                active_session_id,
                entry_id,
                kind,
                title,
                content,
                path,
                reference,
                arguments,
                metadata,
                ..
            } => {
                let (sid_opt, sess_hdir, _) = self
                    .resolve_session_harness_context(active_session_id.as_deref())
                    .await;
                let ws_id = workspace_id.clone();
                let scope = *scope;
                let input = UpsertHarnessInput {
                    scope,
                    entry_id: entry_id.clone(),
                    kind: *kind,
                    title: title.clone(),
                    content: content.clone(),
                    path: path.clone(),
                    reference: reference.clone(),
                    arguments: arguments.clone(),
                    metadata: metadata.clone(),
                };
                let res = tokio::task::spawn_blocking(move || {
                    let dirs = HarnessScopeDirs {
                        agent_dir: &agent_dir,
                        workspace_id: ws_id.as_deref(),
                        session_id: sid_opt.as_deref(),
                        session_harness_dir: sess_hdir.as_deref(),
                    };
                    upsert_entry(&dirs, input)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("harness_upsert task failed: {e}")));
                Some(match res {
                    Ok(entry) => {
                        self.broadcast_harness_changed(
                            scope,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                        );
                        self.notify_workers_harness_dirty(scope, active_session_id.as_deref())
                            .await;
                        response_success(command_id, type_name, Some(json!({ "entry": entry })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::HarnessDelete {
                scope,
                workspace_id,
                active_session_id,
                kind,
                entry_id,
                ..
            } => {
                let (sid_opt, sess_hdir, _) = self
                    .resolve_session_harness_context(active_session_id.as_deref())
                    .await;
                let ws_id = workspace_id.clone();
                let scope = *scope;
                let kind = *kind;
                let entry_id = entry_id.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let dirs = HarnessScopeDirs {
                        agent_dir: &agent_dir,
                        workspace_id: ws_id.as_deref(),
                        session_id: sid_opt.as_deref(),
                        session_harness_dir: sess_hdir.as_deref(),
                    };
                    delete_entry(&dirs, scope, kind, &entry_id)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("harness_delete task failed: {e}")));
                Some(match res {
                    Ok(deleted) => {
                        if deleted {
                            self.broadcast_harness_changed(
                                scope,
                                workspace_id.as_deref(),
                                active_session_id.as_deref(),
                            );
                            self.notify_workers_harness_dirty(scope, active_session_id.as_deref())
                                .await;
                        }
                        response_success(command_id, type_name, Some(json!({ "deleted": deleted })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::HarnessHistory {
                scope,
                workspace_id,
                active_session_id,
                ..
            } => {
                let (sid_opt, sess_hdir, session_hist) = self
                    .resolve_session_harness_context(active_session_id.as_deref())
                    .await;
                let ws_id = workspace_id.clone();
                let scope = *scope;
                let res = tokio::task::spawn_blocking(move || {
                    let dirs = HarnessScopeDirs {
                        agent_dir: &agent_dir,
                        workspace_id: ws_id.as_deref(),
                        session_id: sid_opt.as_deref(),
                        session_harness_dir: sess_hdir.as_deref(),
                    };
                    list_history(&dirs, scope, &session_hist)
                })
                .await;
                Some(match res {
                    Ok(items) => {
                        response_success(command_id, type_name, Some(json!({ "history": items })))
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            DaemonCommand::HarnessRollback {
                refinement_id,
                workspace_id,
                active_session_id,
                ..
            } => {
                let (sid_opt, sess_hdir, session_hist) = self
                    .resolve_session_harness_context(active_session_id.as_deref())
                    .await;
                let ws_id = workspace_id.clone();
                let ref_id = refinement_id.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let dirs = HarnessScopeDirs {
                        agent_dir: &agent_dir,
                        workspace_id: ws_id.as_deref(),
                        session_id: sid_opt.as_deref(),
                        session_harness_dir: sess_hdir.as_deref(),
                    };
                    rollback_refinement(&dirs, &ref_id, &session_hist)
                })
                .await
                .unwrap_or_else(|e| Err(anyhow::anyhow!("harness_rollback task failed: {e}")));
                Some(match res {
                    Ok((touched_scopes, result)) => {
                        for sc in &touched_scopes {
                            self.broadcast_harness_changed(
                                *sc,
                                workspace_id.as_deref(),
                                active_session_id.as_deref(),
                            );
                            self.notify_workers_harness_dirty(*sc, active_session_id.as_deref())
                                .await;
                        }
                        response_success(
                            command_id,
                            type_name,
                            Some(json!({
                                "rolledBack": true,
                                "scopes": touched_scopes,
                                "result": result,
                            })),
                        )
                    }
                    Err(e) => response_failure(command_id, type_name, &e.to_string(), None),
                })
            }
            _ => None,
        }
    }
}

/// Resolve a safe fallback working directory when reopening a saved session whose
/// original `cwd` or linked worktree was deleted on disk.
pub(crate) fn fallback_cwd_for_deleted_worktree(agent_dir: &Path, session_path: &Path) -> Option<String> {
    if let Ok(store) = crate::session_store::SessionFile::open(session_path) {
        if let Some(binding) = store.workspace_binding() {
            if Path::new(&binding.worktree_path).is_dir() {
                return Some(binding.worktree_path);
            }
            if let Some(ws) = pa_core::workspace::find_workspace(agent_dir, &binding.workspace_id) {
                if Path::new(&ws.root_path).is_dir() {
                    return Some(ws.root_path);
                }
            }
            if Path::new(&binding.workspace_root).is_dir() {
                return Some(binding.workspace_root);
            }
        }
    }
    pa_types::platform::home_dir()
        .filter(|h| h.is_dir())
        .map(|h| h.to_string_lossy().into_owned())
}



#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::SupervisorOptions;
    use pa_types::daemon::HarnessKindWire;
    use std::process::Command;

    fn make_supervisor(dir: &Path) -> Supervisor {
        Supervisor::new(SupervisorOptions {
            socket_path: dir.join("daemon.sock"),
            agent_dir: dir.join("agent"),
        })
        .expect("supervisor")
    }

    fn init_git_repo(dir: &Path) {
        std::fs::create_dir_all(dir).expect("create repo dir");
        let run = |args: &[&str]| {
            let out = Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .expect("git command");
            assert!(
                out.status.success(),
                "git {:?} failed: {}",
                args,
                String::from_utf8_lossy(&out.stderr)
            );
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.name", "Test User"]);
        run(&["config", "user.email", "test@example.com"]);
        std::fs::write(dir.join("README.md"), "hello\n").expect("write readme");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "init"]);
    }

    fn write_session_file(path: &Path, cwd: &str, binding: Option<&pa_types::daemon::WorkspaceBinding>) {
        let mut store = crate::session_store::SessionFile::create(cwd, None, 0);
        store.path = path.to_path_buf();
        store.persist_entry("custom", json!({ "customType": "test_init", "data": {} })).expect("init write");
        if let Some(b) = binding {
            store
                .persist_entry(
                    "custom",
                    json!({
                        "customType": pa_core::workspace::WORKSPACE_BINDING_CUSTOM_TYPE,
                        "data": b,
                    }),
                )
                .expect("persist workspace_binding");
        }
    }

    #[tokio::test]
    async fn supervisor_workspace_worktree_branch_and_relink_flow() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let repo = dir.path().join("demo-repo");
        init_git_repo(&repo);
        let sup = make_supervisor(dir.path());
        let mut rx = sup.events.subscribe();

        // 1. Register workspace
        let reg_cmd = DaemonCommand::WorkspaceRegister {
            id: Some("c1".into()),
            path: repo.to_string_lossy().into_owned(),
            name: None,
            rest: Default::default(),
        };
        let reg_resp = sup
            .handle_workspace_and_harness_command(&reg_cmd, Some("c1"), "workspace_register")
            .await
            .expect("handled");
        assert!(reg_resp.success, "register failed: {:?}", reg_resp.error);
        let ws_id = reg_resp.data.as_ref().unwrap()["workspace"]["id"]
            .as_str()
            .unwrap()
            .to_string();
        let (_, ev) = rx.try_recv().expect("workspaces_changed event");
        assert_eq!(ev["type"], "workspaces_changed");

        // 2. Create linked worktree
        let wt_cmd = DaemonCommand::WorktreeCreate {
            id: Some("c2".into()),
            workspace_id: ws_id.clone(),
            name: "feat-alpha".into(),
            base_branch: None,
            rest: Default::default(),
        };
        let wt_resp = sup
            .handle_workspace_and_harness_command(&wt_cmd, Some("c2"), "worktree_create")
            .await
            .expect("handled");
        assert!(wt_resp.success, "worktree_create failed: {:?}", wt_resp.error);
        let wt_path = wt_resp.data.as_ref().unwrap()["worktree"]["path"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(Path::new(&wt_path).is_dir());

        // 3. Registering linked worktree path resolves to same workspace_id
        let reg_wt = DaemonCommand::WorkspaceRegister {
            id: Some("c3".into()),
            path: wt_path.clone(),
            name: None,
            rest: Default::default(),
        };
        let reg_wt_resp = sup
            .handle_workspace_and_harness_command(&reg_wt, Some("c3"), "workspace_register")
            .await
            .expect("handled");
        assert!(reg_wt_resp.success);
        assert_eq!(
            reg_wt_resp.data.as_ref().unwrap()["workspace"]["id"].as_str(),
            Some(ws_id.as_str())
        );

        // 4. Worktree list returns both main + linked worktree
        let wt_list = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::WorktreeList {
                    id: Some("c4".into()),
                    workspace_id: ws_id.clone(),
                    rest: Default::default(),
                },
                Some("c4"),
                "worktree_list",
            )
            .await
            .expect("handled");
        assert!(wt_list.success);
        assert_eq!(
            wt_list.data.as_ref().unwrap()["worktrees"]
                .as_array()
                .unwrap()
                .len(),
            2
        );

        // 5. Branch create & switch inside linked worktree
        let br_create = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::BranchCreate {
                    id: Some("c5".into()),
                    worktree_path: wt_path.clone(),
                    branch: "feat-beta".into(),
                    start_point: None,
                    active_session_id: None,
                    rest: Default::default(),
                },
                Some("c5"),
                "branch_create",
            )
            .await
            .expect("handled");
        assert!(br_create.success, "branch_create failed: {:?}", br_create.error);
        assert_eq!(
            br_create.data.as_ref().unwrap()["branch"].as_str(),
            Some("feat-beta")
        );

        let br_switch = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::BranchSwitch {
                    id: Some("c6".into()),
                    worktree_path: wt_path.clone(),
                    branch: "feat-alpha".into(),
                    active_session_id: None,
                    rest: Default::default(),
                },
                Some("c6"),
                "branch_switch",
            )
            .await
            .expect("handled");
        assert!(br_switch.success, "branch_switch failed: {:?}", br_switch.error);
        assert_eq!(
            br_switch.data.as_ref().unwrap()["branch"].as_str(),
            Some("feat-alpha")
        );

        // 6. Session binding scan + deleted-worktree fallback
        let sessions_dir = dir.path().join("agent").join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let sess_file = sessions_dir.join("sess-demo.jsonl");
        let binding = pa_types::daemon::WorkspaceBinding {
            workspace_id: ws_id.clone(),
            workspace_name: "demo-repo".into(),
            workspace_kind: pa_types::daemon::WorkspaceKind::Git,
            workspace_root: repo.to_string_lossy().into_owned(),
            worktree_path: wt_path.clone(),
            worktree_name: "feat-alpha".into(),
            branch: Some("feat-alpha".into()),
        };
        write_session_file(&sess_file, &wt_path, Some(&binding));

        let reopened = crate::session_store::SessionFile::open(&sess_file).unwrap();
        let read_binding = reopened.workspace_binding().expect("workspace_binding present");
        assert_eq!(read_binding.workspace_id, ws_id);
        assert_eq!(read_binding.branch.as_deref(), Some("feat-alpha"));

        // Remove linked worktree on disk -> fallback must land on main checkout!
        let rm_out = Command::new("git")
            .args(["worktree", "remove", "--force", &wt_path])
            .current_dir(&repo)
            .output()
            .expect("git worktree remove");
        assert!(rm_out.status.success());
        let fallback = fallback_cwd_for_deleted_worktree(&dir.path().join("agent"), &sess_file)
            .expect("fallback cwd");
        assert_eq!(
            std::fs::canonicalize(&fallback).unwrap(),
            std::fs::canonicalize(&repo).unwrap()
        );

        // 7. WorkspaceDelete removes workspace from registry
        let del_resp = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::WorkspaceDelete {
                    id: Some("c7".into()),
                    workspace_id: ws_id.clone(),
                    delete_harness: Some(true),
                    rest: Default::default(),
                },
                Some("c7"),
                "workspace_delete",
            )
            .await
            .expect("handled");
        assert!(del_resp.success);
        assert_eq!(del_resp.data.as_ref().unwrap()["deleted"], true);
    }

    #[tokio::test]
    async fn supervisor_three_scope_harness_crud_history_and_rollback() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let sup = make_supervisor(dir.path());
        let mut rx = sup.events.subscribe();

        // Create a saved session file so session scope resolves cleanly
        let sessions_dir = dir.path().join("agent").join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();
        let sess_file = sessions_dir.join("sess-h1.jsonl");
        write_session_file(&sess_file, &dir.path().to_string_lossy(), None);

        // Upsert Global, Workspace, and Session entries
        for (scope, ws, sid, title) in [
            (HarnessScopeWire::Global, None, None, "Global Rule"),
            (HarnessScopeWire::Workspace, Some("ws-demo".into()), None, "Workspace Fact"),
            (HarnessScopeWire::Session, None, Some("sess-h1".into()), "Session Note"),
        ] {
            let resp = sup
                .handle_workspace_and_harness_command(
                    &DaemonCommand::HarnessUpsert {
                        id: None,
                        scope,
                        workspace_id: ws,
                        active_session_id: sid,
                        entry_id: None,
                        kind: HarnessKindWire::Memory,
                        title: title.into(),
                        content: format!("body for {title}"),
                        path: Some("general".into()),
                        reference: None,
                        arguments: None,
                        metadata: None,
                        rest: Default::default(),
                    },
                    None,
                    "harness_upsert",
                )
                .await
                .unwrap();
            assert!(resp.success, "upsert {:?} failed: {:?}", scope, resp.error);
            let (_, ev) = rx.try_recv().expect("harness_changed event");
            assert_eq!(ev["type"], "harness_changed");
        }

        // List Workspace entries
        let ws_list = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::HarnessList {
                    id: None,
                    scope: Some(HarnessScopeWire::Workspace),
                    workspace_id: Some("ws-demo".into()),
                    active_session_id: Some("sess-h1".into()),
                    kind: None,
                    query: None,
                    rest: Default::default(),
                },
                None,
                "harness_list",
            )
            .await
            .unwrap();
        assert!(ws_list.success);
        let ws_entries = ws_list.data.as_ref().unwrap()["entries"].as_array().unwrap();
        assert_eq!(ws_entries.len(), 1);
        let ws_entry_id = ws_entries[0]["id"].as_str().unwrap().to_string();

        // Delete Workspace entry
        let del = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::HarnessDelete {
                    id: None,
                    scope: HarnessScopeWire::Workspace,
                    workspace_id: Some("ws-demo".into()),
                    active_session_id: None,
                    kind: HarnessKindWire::Memory,
                    entry_id: ws_entry_id,
                    rest: Default::default(),
                },
                None,
                "harness_delete",
            )
            .await
            .unwrap();
        assert!(del.success);
        assert_eq!(del.data.as_ref().unwrap()["deleted"], true);

        // Verify Workspace list is now empty
        let ws_list2 = sup
            .handle_workspace_and_harness_command(
                &DaemonCommand::HarnessList {
                    id: None,
                    scope: Some(HarnessScopeWire::Workspace),
                    workspace_id: Some("ws-demo".into()),
                    active_session_id: None,
                    kind: None,
                    query: None,
                    rest: Default::default(),
                },
                None,
                "harness_list",
            )
            .await
            .unwrap();
        assert_eq!(
            ws_list2.data.as_ref().unwrap()["entries"].as_array().unwrap().len(),
            0
        );
    }
}
