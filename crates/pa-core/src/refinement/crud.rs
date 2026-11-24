//! Daemon-facing continual harness CRUD and multi-scope refinement rollback.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{anyhow, Result};
use pa_types::daemon::{
    HarnessEntryWire, HarnessKindWire, HarnessScopeWire, RefinementHistoryItemWire,
};
use serde_json::{Map, Value};

use super::planner::{apply_refinement_proposal, rollback_proposal, ApplyOptions};
use super::{
    append_scoped_refinement, get_global_harness_state_dir, load_harness_state,
    load_scoped_refinement_history, merge_refinement_histories, try_update_harness_state,
    HarnessEntry, HarnessScope, RefinementKind, RefinementResult,
};
use crate::workspace::workspace_harness_dir;

/// Target directories resolved for a CRUD / history / rollback request.
#[derive(Debug, Clone)]
pub struct HarnessScopeDirs<'a> {
    pub agent_dir: &'a Path,
    pub workspace_id: Option<&'a str>,
    pub session_id: Option<&'a str>,
    pub session_harness_dir: Option<&'a Path>,
}

#[must_use]
pub const fn wire_to_scope(scope: HarnessScopeWire) -> HarnessScope {
    match scope {
        HarnessScopeWire::Global => HarnessScope::Global,
        HarnessScopeWire::Workspace => HarnessScope::Workspace,
        HarnessScopeWire::Session => HarnessScope::Local,
    }
}

#[must_use]
pub const fn scope_to_wire(scope: HarnessScope) -> HarnessScopeWire {
    match scope {
        HarnessScope::Global => HarnessScopeWire::Global,
        HarnessScope::Workspace => HarnessScopeWire::Workspace,
        HarnessScope::Local => HarnessScopeWire::Session,
    }
}

#[must_use]
pub const fn wire_to_kind(kind: HarnessKindWire) -> RefinementKind {
    match kind {
        HarnessKindWire::Prompt => RefinementKind::Prompt,
        HarnessKindWire::Memory => RefinementKind::Memory,
        HarnessKindWire::Skill => RefinementKind::Skill,
        HarnessKindWire::Subagent => RefinementKind::Subagent,
        HarnessKindWire::Factory => RefinementKind::Factory,
    }
}

#[must_use]
pub const fn kind_to_wire(kind: RefinementKind) -> HarnessKindWire {
    match kind {
        RefinementKind::Prompt => HarnessKindWire::Prompt,
        RefinementKind::Memory => HarnessKindWire::Memory,
        RefinementKind::Skill => HarnessKindWire::Skill,
        RefinementKind::Subagent => HarnessKindWire::Subagent,
        RefinementKind::Factory => HarnessKindWire::Factory,
    }
}

fn entry_to_wire(
    entry: &HarnessEntry,
    scope: HarnessScope,
    workspace_id: Option<&str>,
    session_id: Option<&str>,
) -> HarnessEntryWire {
    HarnessEntryWire {
        id: entry.id.clone(),
        kind: kind_to_wire(entry.kind),
        title: entry.title.clone(),
        content: entry.content.clone(),
        path: entry.path.clone(),
        scope: scope_to_wire(scope),
        workspace_id: (scope == HarnessScope::Workspace)
            .then(|| workspace_id.map(str::to_owned))
            .flatten(),
        session_id: (scope == HarnessScope::Local)
            .then(|| session_id.map(str::to_owned))
            .flatten(),
        reference: (!entry.reference.is_empty()).then(|| Value::Object(entry.reference.clone())),
        arguments: (!entry.arguments.is_empty()).then(|| Value::Object(entry.arguments.clone())),
        metadata: Value::Object(entry.metadata.clone()),
        source: entry.source.clone(),
        created_at: entry.created_at.clone(),
        updated_at: entry.updated_at.clone(),
        version: entry.version,
    }
}

fn dir_for_scope(dirs: &HarnessScopeDirs<'_>, scope: HarnessScope) -> Result<std::path::PathBuf> {
    match scope {
        HarnessScope::Global => Ok(get_global_harness_state_dir(dirs.agent_dir)),
        HarnessScope::Workspace => {
            let ws_id = dirs
                .workspace_id
                .ok_or_else(|| anyhow!("workspace scope requires a workspaceId"))?;
            Ok(workspace_harness_dir(dirs.agent_dir, ws_id))
        }
        HarnessScope::Local => dirs
            .session_harness_dir
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("session scope requires an active session")),
    }
}

fn slug_entry_id(title: &str) -> String {
    let mut out = String::new();
    let mut last_us = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_us = false;
        } else if !last_us && !out.is_empty() {
            out.push('_');
            last_us = true;
        }
        if out.len() >= 48 {
            break;
        }
    }
    let trimmed = out.trim_matches('_').to_owned();
    if trimmed.is_empty() {
        format!("entry_{}", super::executor::generate_refinement_id())
    } else {
        trimmed
    }
}

/// List entries across one scope or all available scopes, filtered by optional kind and query.
#[must_use]
pub fn list_entries(
    dirs: &HarnessScopeDirs<'_>,
    scope_filter: Option<HarnessScopeWire>,
    kind_filter: Option<HarnessKindWire>,
    query: Option<&str>,
) -> Vec<HarnessEntryWire> {
    let scopes: Vec<HarnessScope> = match scope_filter {
        Some(sc) => vec![wire_to_scope(sc)],
        None => {
            let mut all = vec![HarnessScope::Global];
            if dirs.workspace_id.is_some() {
                all.push(HarnessScope::Workspace);
            }
            if dirs.session_harness_dir.is_some() {
                all.push(HarnessScope::Local);
            }
            all
        }
    };
    let q = query
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let kind_match = kind_filter.map(wire_to_kind);

    let mut out = Vec::new();
    for scope in scopes {
        let Ok(dir) = dir_for_scope(dirs, scope) else {
            continue;
        };
        let state = load_harness_state(&dir, scope);
        for (kind, map) in &state.entries {
            if let Some(km) = kind_match {
                if *kind != km {
                    continue;
                }
            }
            for entry in map.values() {
                if let Some(ref needle) = q {
                    let hay = format!(
                        "{} {} {} {}",
                        entry.id.to_lowercase(),
                        entry.title.to_lowercase(),
                        entry.path.to_lowercase(),
                        entry.content.to_lowercase()
                    );
                    if !hay.contains(needle) {
                        continue;
                    }
                }
                out.push(entry_to_wire(entry, scope, dirs.workspace_id, dirs.session_id));
            }
        }
    }
    out.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.title.cmp(&b.title)));
    out
}

/// Input payload for creating or updating one harness entry.
#[derive(Debug, Clone)]
pub struct UpsertHarnessInput {
    pub scope: HarnessScopeWire,
    pub entry_id: Option<String>,
    pub kind: HarnessKindWire,
    pub title: String,
    pub content: String,
    pub path: Option<String>,
    pub reference: Option<Value>,
    pub arguments: Option<Value>,
    pub metadata: Option<Value>,
}

/// Create or update one entry atomically under `{file}.lock`.
///
/// # Errors
///
/// Returns an error if title/content are empty, scope target is missing, or the locked write fails.
pub fn upsert_entry(
    dirs: &HarnessScopeDirs<'_>,
    input: UpsertHarnessInput,
) -> Result<HarnessEntryWire> {
    let title = input.title.trim().to_owned();
    let content = input.content.trim().to_owned();
    if title.is_empty() || content.is_empty() {
        return Err(anyhow!("title and content cannot be empty"));
    }
    let scope = wire_to_scope(input.scope);
    let kind = wire_to_kind(input.kind);
    let dir = dir_for_scope(dirs, scope)?;
    let now = crate::session::manager::format_iso_now();
    let path_str = input
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(match kind {
            RefinementKind::Prompt => "policy",
            _ => "general",
        })
        .to_owned();

    let (saved, _) = try_update_harness_state(&dir, scope, |state| {
        let bucket = state.entries.entry(kind).or_default();
        let id = input
            .entry_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map_or_else(|| slug_entry_id(&title), str::to_owned);
        let prev = bucket.get(&id).cloned();
        let reference = input
            .reference
            .and_then(|v| v.as_object().cloned())
            .or_else(|| prev.as_ref().map(|p| p.reference.clone()))
            .unwrap_or_else(Map::new);
        let arguments = input
            .arguments
            .and_then(|v| v.as_object().cloned())
            .or_else(|| prev.as_ref().map(|p| p.arguments.clone()))
            .unwrap_or_else(Map::new);
        let metadata = input
            .metadata
            .and_then(|v| v.as_object().cloned())
            .or_else(|| prev.as_ref().map(|p| p.metadata.clone()))
            .unwrap_or_else(Map::new);

        let entry = HarnessEntry {
            id: id.clone(),
            kind,
            title,
            content,
            path: path_str,
            scope: Some(scope),
            reference,
            arguments,
            metadata,
            source: "settings".to_owned(),
            created_at: prev
                .as_ref()
                .map_or_else(|| now.clone(), |p| p.created_at.clone()),
            updated_at: now.clone(),
            version: prev.as_ref().map_or(1, |p| p.version.saturating_add(1)),
        };
        bucket.insert(id, entry.clone());
        Ok(entry)
    })?;

    Ok(entry_to_wire(&saved, scope, dirs.workspace_id, dirs.session_id))
}

/// Delete one entry atomically under `{file}.lock`.
///
/// # Errors
///
/// Returns an error if the target scope directory cannot be resolved or locked.
pub fn delete_entry(
    dirs: &HarnessScopeDirs<'_>,
    scope_wire: HarnessScopeWire,
    kind_wire: HarnessKindWire,
    entry_id: &str,
) -> Result<bool> {
    let scope = wire_to_scope(scope_wire);
    let kind = wire_to_kind(kind_wire);
    let dir = dir_for_scope(dirs, scope)?;
    let (removed, _) = try_update_harness_state(&dir, scope, |state| {
        Ok(state
            .entries
            .get_mut(&kind)
            .and_then(|b| b.remove(entry_id))
            .is_some())
    })?;
    Ok(removed)
}

fn collect_refinement_results(
    dirs: &HarnessScopeDirs<'_>,
    session_history: &[RefinementResult],
) -> Vec<RefinementResult> {
    let global_dir = get_global_harness_state_dir(dirs.agent_dir);
    let global = load_scoped_refinement_history(&global_dir, HarnessScope::Global);
    let workspace = dirs.workspace_id.map(|id| {
        let ws_dir = workspace_harness_dir(dirs.agent_dir, id);
        load_scoped_refinement_history(&ws_dir, HarnessScope::Workspace)
    });
    let mut slices: Vec<&[RefinementResult]> = vec![&global];
    if let Some(ref ws) = workspace {
        slices.push(ws);
    }
    slices.push(session_history);
    merge_refinement_histories(&slices)
}

/// List refinement history items across scopes for Settings -> Harness -> History.
#[must_use]
pub fn list_history(
    dirs: &HarnessScopeDirs<'_>,
    scope_filter: Option<HarnessScopeWire>,
    session_history: &[RefinementResult],
) -> Vec<RefinementHistoryItemWire> {
    let merged = collect_refinement_results(dirs, session_history);
    let rolled_back_ids: BTreeSet<String> = merged
        .iter()
        .filter_map(|r| r.rollback_of.clone())
        .collect();
    let filter_scope = scope_filter.map(wire_to_scope);

    let mut out = Vec::new();
    for item in merged.into_iter().rev() {
        let mut scopes: Vec<HarnessScope> = Vec::new();
        for edit in &item.applied_edits {
            if !edit.applied {
                continue;
            }
            let sc = edit
                .scope
                .or_else(|| edit.after.as_ref().or(edit.before.as_ref()).and_then(|e| e.scope))
                .or(item.scope)
                .unwrap_or(HarnessScope::Local);
            if !scopes.contains(&sc) {
                scopes.push(sc);
            }
        }
        if scopes.is_empty() {
            if let Some(sc) = item.scope {
                scopes.push(sc);
            }
        }
        if let Some(fs) = filter_scope {
            if !scopes.contains(&fs) {
                continue;
            }
        }
        let changes: Vec<String> = item
            .applied_edits
            .iter()
            .filter(|e| e.applied)
            .map(|e| {
                let sc = e
                    .scope
                    .or_else(|| e.after.as_ref().or(e.before.as_ref()).and_then(|x| x.scope))
                    .or(item.scope)
                    .unwrap_or(HarnessScope::Local);
                let label = e
                    .after
                    .as_ref()
                    .or(e.before.as_ref())
                    .map_or(e.id.as_str(), |x| x.title.as_str());
                format!("{}: {:?} {} ({})", sc.as_str(), e.action, label, e.id)
            })
            .collect();
        let rolled_back = rolled_back_ids.contains(&item.id);
        let can_rollback = item.rollback_of.is_none() && !rolled_back && !changes.is_empty();
        let created_at = item
            .applied_edits
            .iter()
            .filter_map(|e| e.after.as_ref().or(e.before.as_ref()).map(|x| x.updated_at.clone()))
            .next()
            .unwrap_or_default();
        out.push(RefinementHistoryItemWire {
            id: item.id.clone(),
            trigger: item
                .rollback_of
                .as_ref()
                .map_or_else(|| item.summary.clone(), |rb| format!("Rollback of {rb}")),
            created_at,
            scopes: scopes.into_iter().map(scope_to_wire).collect(),
            changes,
            evidence: item.rationale.clone(),
            outcome: item.expected_outcome.clone(),
            rolled_back,
            can_rollback,
        });
    }
    out
}

/// Roll back a refinement across every scope it touched.
/// Returns the list of scopes actually modified plus the recorded rollback `RefinementResult`.
///
/// # Errors
///
/// Returns an error if `refinement_id` is not found or any touched scope store cannot be updated.
pub fn rollback_refinement(
    dirs: &HarnessScopeDirs<'_>,
    refinement_id: &str,
    session_history: &[RefinementResult],
) -> Result<(Vec<HarnessScopeWire>, RefinementResult)> {
    let merged = collect_refinement_results(dirs, session_history);
    let target = merged
        .into_iter()
        .find(|r| r.id == refinement_id)
        .ok_or_else(|| anyhow!("Refinement {refinement_id} not found"))?;

    // Partition applied edits by their scope so multi-scope refinements roll back cleanly.
    let mut by_scope: std::collections::BTreeMap<HarnessScope, Vec<super::AppliedRefinementEdit>> =
        std::collections::BTreeMap::new();
    for edit in &target.applied_edits {
        if !edit.applied {
            continue;
        }
        let sc = edit
            .scope
            .or_else(|| edit.after.as_ref().or(edit.before.as_ref()).and_then(|x| x.scope))
            .or(target.scope)
            .unwrap_or(HarnessScope::Local);
        by_scope.entry(sc).or_default().push(edit.clone());
    }
    if by_scope.is_empty() {
        return Err(anyhow!("Refinement {refinement_id} has no applied edits to roll back"));
    }

    let rollback_id = super::executor::generate_refinement_id();
    let mut combined_edits = Vec::new();
    let mut touched_scopes = Vec::new();
    let mut last_path = String::new();

    for (scope, edits) in by_scope {
        let dir = dir_for_scope(dirs, scope)?;
        let scoped_target = RefinementResult {
            id: target.id.clone(),
            summary: target.summary.clone(),
            rationale: target.rationale.clone(),
            expected_outcome: target.expected_outcome.clone(),
            applied_edits: edits,
            harness_state_path: target.harness_state_path.clone(),
            rollback_of: None,
            scope: Some(scope),
        };
        let proposal = rollback_proposal(&scoped_target);
        let rb_id_clone = rollback_id.clone();
        let target_id_clone = target.id.clone();
        let (mut res, written_path) = try_update_harness_state(&dir, scope, |state| {
            Ok(apply_refinement_proposal(
                state,
                &proposal,
                ApplyOptions {
                    id: rb_id_clone,
                    rollback_of: Some(target_id_clone),
                    scope: Some(scope),
                    baseline_state: None,
                    factory_enabled: true,
                },
            ))
        })?;
        for e in &mut res.applied_edits {
            e.scope = Some(scope);
        }
        res.harness_state_path = written_path.to_string_lossy().into_owned();
        if matches!(scope, HarnessScope::Global | HarnessScope::Workspace) {
            let _ = append_scoped_refinement(&dir, &res);
        }
        last_path = res.harness_state_path.clone();
        combined_edits.extend(res.applied_edits);
        touched_scopes.push(scope_to_wire(scope));
    }

    let summary_result = RefinementResult {
        id: rollback_id,
        summary: format!("Rolled back {refinement_id}"),
        rationale: format!("Rollback of {refinement_id} from Settings"),
        expected_outcome: "Prior harness entries restored".to_owned(),
        applied_edits: combined_edits,
        harness_state_path: last_path,
        rollback_of: Some(refinement_id.to_owned()),
        scope: (touched_scopes.len() == 1).then(|| wire_to_scope(touched_scopes[0])),
    };
    Ok((touched_scopes, summary_result))
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::refinement::{merge_harness_states_three, RefinementAction, RefinementResult};
    use tempfile::TempDir;

    #[test]
    fn crud_across_three_scopes_and_precedence_merge() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let sess_dir = tmp.path().join("sess-harness");
        let dirs = HarnessScopeDirs {
            agent_dir: &agent_dir,
            workspace_id: Some("ws-1"),
            session_id: Some("sess-1"),
            session_harness_dir: Some(&sess_dir),
        };

        let g = upsert_entry(
            &dirs,
            UpsertHarnessInput {
                scope: HarnessScopeWire::Global,
                entry_id: Some("shared".into()),
                kind: HarnessKindWire::Memory,
                title: "Global shared".into(),
                content: "from global".into(),
                path: Some("general".into()),
                reference: None,
                arguments: None,
                metadata: None,
            },
        )
        .unwrap();
        assert_eq!(g.version, 1);

        let w = upsert_entry(
            &dirs,
            UpsertHarnessInput {
                scope: HarnessScopeWire::Workspace,
                entry_id: Some("shared".into()),
                kind: HarnessKindWire::Memory,
                title: "Workspace shared".into(),
                content: "from workspace".into(),
                path: Some("build".into()),
                reference: None,
                arguments: None,
                metadata: None,
            },
        )
        .unwrap();
        assert_eq!(w.scope, HarnessScopeWire::Workspace);

        let all = list_entries(&dirs, None, None, None);
        assert_eq!(all.len(), 2);

        // Precedence: Workspace overrides Global for colliding id `shared`
        let g_state = load_harness_state(&get_global_harness_state_dir(&agent_dir), HarnessScope::Global);
        let w_state = load_harness_state(&workspace_harness_dir(&agent_dir, "ws-1"), HarnessScope::Workspace);
        let merged = merge_harness_states_three(&g_state, Some(&w_state), None);
        assert_eq!(merged.entries[&RefinementKind::Memory]["shared"].content, "from workspace");
        assert_eq!(merged.entries[&RefinementKind::Memory]["shared"].scope, Some(HarnessScope::Workspace));

        // Session overrides Workspace for colliding id `shared`
        upsert_entry(
            &dirs,
            UpsertHarnessInput {
                scope: HarnessScopeWire::Session,
                entry_id: Some("shared".into()),
                kind: HarnessKindWire::Memory,
                title: "Session shared".into(),
                content: "from session".into(),
                path: None,
                reference: None,
                arguments: None,
                metadata: None,
            },
        )
        .unwrap();
        let s_state = load_harness_state(&sess_dir, HarnessScope::Local);
        let merged3 = merge_harness_states_three(&g_state, Some(&w_state), Some(&s_state));
        assert_eq!(merged3.entries[&RefinementKind::Memory]["shared"].content, "from session");
        assert_eq!(merged3.entries[&RefinementKind::Memory]["shared"].scope, Some(HarnessScope::Local));

        assert!(delete_entry(&dirs, HarnessScopeWire::Session, HarnessKindWire::Memory, "shared").unwrap());
        assert_eq!(list_entries(&dirs, Some(HarnessScopeWire::Session), None, None).len(), 0);
    }

    #[test]
    fn multi_scope_rollback_restores_workspace_and_session() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let sess_dir = tmp.path().join("sess-harness");
        let dirs = HarnessScopeDirs {
            agent_dir: &agent_dir,
            workspace_id: Some("ws-1"),
            session_id: Some("sess-1"),
            session_harness_dir: Some(&sess_dir),
        };

        // Seed created entries in workspace & session
        upsert_entry(
            &dirs,
            UpsertHarnessInput {
                scope: HarnessScopeWire::Workspace,
                entry_id: Some("ws_m1".into()),
                kind: HarnessKindWire::Memory,
                title: "WS rule".into(),
                content: "keep clean".into(),
                path: None,
                reference: None,
                arguments: None,
                metadata: None,
            },
        )
        .unwrap();
        upsert_entry(
            &dirs,
            UpsertHarnessInput {
                scope: HarnessScopeWire::Session,
                entry_id: Some("s_m1".into()),
                kind: HarnessKindWire::Memory,
                title: "Sess note".into(),
                content: "temp".into(),
                path: None,
                reference: None,
                arguments: None,
                metadata: None,
            },
        )
        .unwrap();

        let ws_after = load_harness_state(&workspace_harness_dir(&agent_dir, "ws-1"), HarnessScope::Workspace)
            .entries[&RefinementKind::Memory]["ws_m1"]
            .clone();
        let sess_after = load_harness_state(&sess_dir, HarnessScope::Local)
            .entries[&RefinementKind::Memory]["s_m1"]
            .clone();

        let multi_ref = RefinementResult {
            id: "refine_multi_1".into(),
            summary: "Multi-scope refine".into(),
            rationale: "test".into(),
            expected_outcome: "ok".into(),
            applied_edits: vec![
                crate::refinement::AppliedRefinementEdit {
                    action: RefinementAction::Create,
                    kind: RefinementKind::Memory,
                    id: "ws_m1".into(),
                    title: Some(ws_after.title.clone()),
                    content: Some(ws_after.content.clone()),
                    path: Some(ws_after.path.clone()),
                    reference: None,
                    arguments: None,
                    metadata: None,
                    before: None,
                    after: Some(ws_after),
                    applied: true,
                    error: None,
                    reason: None,
                    scope: Some(HarnessScope::Workspace),
                },
                crate::refinement::AppliedRefinementEdit {
                    action: RefinementAction::Create,
                    kind: RefinementKind::Memory,
                    id: "s_m1".into(),
                    title: Some(sess_after.title.clone()),
                    content: Some(sess_after.content.clone()),
                    path: Some(sess_after.path.clone()),
                    reference: None,
                    arguments: None,
                    metadata: None,
                    before: None,
                    after: Some(sess_after),
                    applied: true,
                    error: None,
                    reason: None,
                    scope: Some(HarnessScope::Local),
                },
            ],
            harness_state_path: String::new(),
            rollback_of: None,
            scope: None,
        };

        let hist = list_history(&dirs, None, &[multi_ref.clone()]);
        assert_eq!(hist.len(), 1);
        assert!(hist[0].can_rollback);
        assert_eq!(hist[0].scopes.len(), 2);

        let (touched, rb) = rollback_refinement(&dirs, "refine_multi_1", &[multi_ref]).unwrap();
        assert_eq!(touched.len(), 2);
        assert_eq!(rb.rollback_of.as_deref(), Some("refine_multi_1"));
        assert!(list_entries(&dirs, Some(HarnessScopeWire::Workspace), None, None).is_empty());
        assert!(list_entries(&dirs, Some(HarnessScopeWire::Session), None, None).is_empty());
    }
}
