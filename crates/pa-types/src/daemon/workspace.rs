//! Wire types for Project-first Workspaces, Worktrees, Branches, and
//! three-scope continual harness CRUD & refinement history.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Whether a registered Workspace is a Git repository or a plain folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceKind {
    Git,
    Plain,
}

/// One registered Workspace in `~/.prime/agent/workspaces/registry.json` and on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRecord {
    pub id: String,
    pub name: String,
    pub kind: WorkspaceKind,
    pub root_path: String,
    pub last_used_at: String,
    #[serde(default)]
    pub missing: bool,
    #[serde(default)]
    pub harness_entry_count: usize,
}

/// One Git working tree (main checkout or linked worktree) belonging to a Git Workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorktreeInfo {
    pub path: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_oid: Option<String>,
    #[serde(default)]
    pub is_main: bool,
    #[serde(default)]
    pub detached: bool,
}

/// One local Git branch in a Workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInfo {
    pub name: String,
    #[serde(default)]
    pub is_current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// Static binding persisted on a Session (`customType: "workspace_binding"`).
/// Absent on a Scratch session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceBinding {
    pub workspace_id: String,
    pub workspace_name: String,
    pub workspace_kind: WorkspaceKind,
    pub workspace_root: String,
    pub worktree_path: String,
    pub worktree_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// Continual harness scope on the wire. Accepts legacy `"local"` as `"session"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessScopeWire {
    Global,
    Workspace,
    #[serde(alias = "local")]
    Session,
}

impl HarnessScopeWire {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Workspace => "workspace",
            Self::Session => "session",
        }
    }
}

/// Continual harness entry kind on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKindWire {
    Prompt,
    Memory,
    Skill,
    Subagent,
    Factory,
}

impl HarnessKindWire {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Memory => "memory",
            Self::Skill => "skill",
            Self::Subagent => "subagent",
            Self::Factory => "factory",
        }
    }
}

/// One continual harness entry exchanged over the daemon wire protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEntryWire {
    pub id: String,
    pub kind: HarnessKindWire,
    pub title: String,
    pub content: String,
    pub path: String,
    pub scope: HarnessScopeWire,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    #[serde(default)]
    pub metadata: Value,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
    pub version: u64,
}

/// One recorded refinement run shown in Settings -> Harness -> History.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefinementHistoryItemWire {
    pub id: String,
    pub trigger: String,
    pub created_at: String,
    #[serde(default)]
    pub scopes: Vec<HarnessScopeWire>,
    #[serde(default)]
    pub changes: Vec<String>,
    #[serde(default)]
    pub evidence: String,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub rolled_back: bool,
    #[serde(default)]
    pub can_rollback: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::rt;

    #[test]
    fn workspace_binding_roundtrip() {
        rt::<WorkspaceBinding>(
            r#"{"workspaceId":"ws-1","workspaceName":"pa-ui","workspaceKind":"git","workspaceRoot":"/repo","worktreePath":"/wt/feat","worktreeName":"feat","branch":"feat/ws"}"#,
        );
    }

    #[test]
    fn harness_scope_reads_legacy_local_as_session() {
        let parsed: HarnessScopeWire = serde_json::from_str(r#""local""#).unwrap();
        assert_eq!(parsed, HarnessScopeWire::Session);
        assert_eq!(serde_json::to_string(&parsed).unwrap(), r#""session""#);
    }
}
