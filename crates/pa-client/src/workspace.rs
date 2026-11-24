//! Typed daemon client wrappers for Workspace, Worktree, Branch, and
//! three-scope continual harness commands (Spec #140).

use std::io;
use std::time::Duration;

use pa_types::daemon::{
    BranchInfo, HarnessEntryWire, HarnessKindWire, HarnessScopeWire, RefinementHistoryItemWire,
    WorkspaceBinding, WorkspaceRecord, WorktreeInfo,
};
use serde_json::{json, Value};

use crate::Client;

fn decode_field<T: serde::de::DeserializeOwned>(response: &Value, key: &str) -> io::Result<T> {
    let data = response
        .get("data")
        .ok_or_else(|| io::Error::other("daemon response missing data"))?;
    let val = data
        .get(key)
        .ok_or_else(|| io::Error::other(format!("daemon response missing data.{key}")))?;
    serde_json::from_value(val.clone()).map_err(|e| io::Error::other(e.to_string()))
}

fn check_ok(response: &Value, label: &str) -> io::Result<()> {
    if response.get("success").and_then(Value::as_bool) == Some(false) {
        let msg = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return Err(io::Error::other(format!("{label}: {msg}")));
    }
    Ok(())
}

/// `workspace_list` -> registered Workspaces + configured Worktree Root path.
pub fn list_workspaces(client: &Client) -> io::Result<(Vec<WorkspaceRecord>, String)> {
    let resp = client.request("workspace_list", json!({}), Duration::from_secs(10))?;
    check_ok(&resp, "workspace_list")?;
    let workspaces: Vec<WorkspaceRecord> = decode_field(&resp, "workspaces")?;
    let root = resp
        .get("data")
        .and_then(|d| d.get("worktreeRoot"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok((workspaces, root))
}

/// `workspace_register` -> register (or deduplicate) a folder / Git repo.
pub fn register_workspace(
    client: &Client,
    path: &str,
    name: Option<&str>,
) -> io::Result<(WorkspaceRecord, WorkspaceBinding)> {
    let mut payload = json!({ "path": path });
    if let Some(n) = name {
        payload["name"] = json!(n);
    }
    let resp = client.request("workspace_register", payload, Duration::from_secs(15))?;
    check_ok(&resp, "workspace_register")?;
    let workspace: WorkspaceRecord = decode_field(&resp, "workspace")?;
    let binding: WorkspaceBinding = decode_field(&resp, "binding")?;
    Ok((workspace, binding))
}

/// `workspace_relink` -> update a Workspace's root path after the folder moved.
pub fn relink_workspace(
    client: &Client,
    workspace_id: &str,
    new_path: &str,
) -> io::Result<WorkspaceRecord> {
    let resp = client.request(
        "workspace_relink",
        json!({ "workspaceId": workspace_id, "newPath": new_path }),
        Duration::from_secs(15),
    )?;
    check_ok(&resp, "workspace_relink")?;
    decode_field(&resp, "workspace")
}

/// `workspace_delete` -> remove a Workspace from the registry (and optionally its harness).
pub fn delete_workspace(
    client: &Client,
    workspace_id: &str,
    delete_harness: bool,
) -> io::Result<bool> {
    let resp = client.request(
        "workspace_delete",
        json!({ "workspaceId": workspace_id, "deleteHarness": delete_harness }),
        Duration::from_secs(10),
    )?;
    check_ok(&resp, "workspace_delete")?;
    Ok(resp
        .get("data")
        .and_then(|d| d.get("deleted"))
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// `worktree_list` -> live Git worktrees for a Workspace.
pub fn list_worktrees(client: &Client, workspace_id: &str) -> io::Result<Vec<WorktreeInfo>> {
    let resp = client.request(
        "worktree_list",
        json!({ "workspaceId": workspace_id }),
        Duration::from_secs(10),
    )?;
    check_ok(&resp, "worktree_list")?;
    decode_field(&resp, "worktrees")
}

/// `worktree_create` -> create a linked worktree under the configured Worktree Root.
pub fn create_worktree(
    client: &Client,
    workspace_id: &str,
    name: &str,
    base_branch: Option<&str>,
) -> io::Result<WorktreeInfo> {
    let mut payload = json!({ "workspaceId": workspace_id, "name": name });
    if let Some(b) = base_branch {
        payload["baseBranch"] = json!(b);
    }
    let resp = client.request("worktree_create", payload, Duration::from_secs(20))?;
    check_ok(&resp, "worktree_create")?;
    decode_field(&resp, "worktree")
}

/// `branch_list` -> local branches for a Git worktree path.
pub fn list_branches(client: &Client, worktree_path: &str) -> io::Result<Vec<BranchInfo>> {
    let resp = client.request(
        "branch_list",
        json!({ "worktreePath": worktree_path }),
        Duration::from_secs(10),
    )?;
    check_ok(&resp, "branch_list")?;
    decode_field(&resp, "branches")
}

/// `branch_switch` -> switch branches in place inside a worktree.
pub fn switch_branch(
    client: &Client,
    worktree_path: &str,
    branch: &str,
    active_session_id: Option<&str>,
) -> io::Result<String> {
    let mut payload = json!({ "worktreePath": worktree_path, "branch": branch });
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    let resp = client.request("branch_switch", payload, Duration::from_secs(15))?;
    check_ok(&resp, "branch_switch")?;
    decode_field(&resp, "branch")
}

/// `branch_create` -> create & checkout a new local branch inside a worktree.
pub fn create_branch(
    client: &Client,
    worktree_path: &str,
    branch: &str,
    start_point: Option<&str>,
    active_session_id: Option<&str>,
) -> io::Result<String> {
    let mut payload = json!({ "worktreePath": worktree_path, "branch": branch });
    if let Some(sp) = start_point {
        payload["startPoint"] = json!(sp);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    let resp = client.request("branch_create", payload, Duration::from_secs(15))?;
    check_ok(&resp, "branch_create")?;
    decode_field(&resp, "branch")
}

/// `session_bind_workspace` -> bind a live session to a Workspace/Worktree (or `None` for Scratch).
pub fn bind_session_workspace(
    client: &Client,
    session: &str,
    workspace_id: Option<&str>,
    worktree_path: Option<&str>,
) -> io::Result<Option<WorkspaceBinding>> {
    let mut payload = json!({ "activeSessionId": session });
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(wt) = worktree_path {
        payload["worktreePath"] = json!(wt);
    }
    let resp = client.request("session_bind_workspace", payload, Duration::from_secs(15))?;
    check_ok(&resp, "session_bind_workspace")?;
    let val = resp
        .get("data")
        .and_then(|d| d.get("binding"))
        .cloned()
        .unwrap_or(Value::Null);
    if val.is_null() {
        Ok(None)
    } else {
        serde_json::from_value(val).map(Some).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// `session_switch_worktree` -> switch a live session's bound worktree within its Workspace.
pub fn switch_session_worktree(
    client: &Client,
    session: &str,
    worktree_path: &str,
) -> io::Result<Option<WorkspaceBinding>> {
    let resp = client.request(
        "session_switch_worktree",
        json!({ "activeSessionId": session, "worktreePath": worktree_path }),
        Duration::from_secs(15),
    )?;
    check_ok(&resp, "session_switch_worktree")?;
    let val = resp
        .get("data")
        .and_then(|d| d.get("binding"))
        .cloned()
        .unwrap_or(Value::Null);
    if val.is_null() {
        Ok(None)
    } else {
        serde_json::from_value(val).map(Some).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// `session_refresh_workspace` -> re-read `.git/HEAD` for a live session.
pub fn refresh_session_workspace(
    client: &Client,
    session: &str,
) -> io::Result<Option<WorkspaceBinding>> {
    let resp = client.request(
        "session_refresh_workspace",
        json!({ "activeSessionId": session }),
        Duration::from_secs(10),
    )?;
    check_ok(&resp, "session_refresh_workspace")?;
    let val = resp
        .get("data")
        .and_then(|d| d.get("binding"))
        .cloned()
        .unwrap_or(Value::Null);
    if val.is_null() {
        Ok(None)
    } else {
        serde_json::from_value(val).map(Some).map_err(|e| io::Error::other(e.to_string()))
    }
}

/// `harness_list` -> list continual harness entries for a scope.
pub fn harness_list(
    client: &Client,
    scope: Option<HarnessScopeWire>,
    workspace_id: Option<&str>,
    active_session_id: Option<&str>,
    kind: Option<HarnessKindWire>,
    query: Option<&str>,
) -> io::Result<Vec<HarnessEntryWire>> {
    let mut payload = json!({});
    if let Some(sc) = scope {
        payload["scope"] = json!(sc);
    }
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    if let Some(k) = kind {
        payload["kind"] = json!(k);
    }
    if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
        payload["query"] = json!(q);
    }
    let resp = client.request("harness_list", payload, Duration::from_secs(10))?;
    check_ok(&resp, "harness_list")?;
    decode_field(&resp, "entries")
}

/// `harness_upsert` -> create or update a continual harness entry.
#[allow(clippy::too_many_arguments)]
pub fn harness_upsert(
    client: &Client,
    scope: HarnessScopeWire,
    workspace_id: Option<&str>,
    active_session_id: Option<&str>,
    entry_id: Option<&str>,
    kind: HarnessKindWire,
    title: &str,
    content: &str,
    path: Option<&str>,
) -> io::Result<HarnessEntryWire> {
    let mut payload = json!({
        "scope": scope,
        "kind": kind,
        "title": title,
        "content": content,
    });
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    if let Some(id) = entry_id {
        payload["entryId"] = json!(id);
    }
    if let Some(p) = path.filter(|s| !s.trim().is_empty()) {
        payload["path"] = json!(p);
    }
    let resp = client.request("harness_upsert", payload, Duration::from_secs(10))?;
    check_ok(&resp, "harness_upsert")?;
    decode_field(&resp, "entry")
}

/// `harness_delete` -> remove a continual harness entry.
pub fn harness_delete(
    client: &Client,
    scope: HarnessScopeWire,
    workspace_id: Option<&str>,
    active_session_id: Option<&str>,
    kind: HarnessKindWire,
    entry_id: &str,
) -> io::Result<bool> {
    let mut payload = json!({
        "scope": scope,
        "kind": kind,
        "entryId": entry_id,
    });
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    let resp = client.request("harness_delete", payload, Duration::from_secs(10))?;
    check_ok(&resp, "harness_delete")?;
    Ok(resp
        .get("data")
        .and_then(|d| d.get("deleted"))
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// `harness_history` -> refinement history across scopes.
pub fn harness_history(
    client: &Client,
    scope: Option<HarnessScopeWire>,
    workspace_id: Option<&str>,
    active_session_id: Option<&str>,
) -> io::Result<Vec<RefinementHistoryItemWire>> {
    let mut payload = json!({});
    if let Some(sc) = scope {
        payload["scope"] = json!(sc);
    }
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    let resp = client.request("harness_history", payload, Duration::from_secs(10))?;
    check_ok(&resp, "harness_history")?;
    decode_field(&resp, "history")
}

/// `harness_rollback` -> roll back a single- or multi-scope refinement event by id.
pub fn harness_rollback(
    client: &Client,
    refinement_id: &str,
    workspace_id: Option<&str>,
    active_session_id: Option<&str>,
) -> io::Result<Value> {
    let mut payload = json!({ "refinementId": refinement_id });
    if let Some(ws) = workspace_id {
        payload["workspaceId"] = json!(ws);
    }
    if let Some(sid) = active_session_id {
        payload["activeSessionId"] = json!(sid);
    }
    let resp = client.request("harness_rollback", payload, Duration::from_secs(15))?;
    check_ok(&resp, "harness_rollback")?;
    Ok(resp.get("data").cloned().unwrap_or(Value::Null))
}
