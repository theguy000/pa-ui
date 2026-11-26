//! Project-first Workspace registry, canonical Git repository identity,
//! managed Worktrees, Branch operations, and fast `.git/HEAD` inspection.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use pa_types::daemon::{
    BranchInfo, WorkspaceBinding, WorkspaceKind, WorkspaceRecord, WorktreeInfo,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::platform::lock_dir::LockDir;
use crate::refinement::HARNESS_STATE_DIR_NAME;
use crate::settings::storage::{atomic_write_with, AtomicWriteOptions};

/// Directory under `<agent_dir>` holding Workspace registry and per-Workspace harness dirs.
pub const WORKSPACES_DIR_NAME: &str = "workspaces";
/// Registry filename inside `<agent_dir>/workspaces/`.
pub const WORKSPACE_REGISTRY_FILE_NAME: &str = "registry.json";
/// Default managed Worktree root directory name under `<agent_dir>`.
pub const DEFAULT_WORKTREES_DIR_NAME: &str = "worktrees";
/// Settings key in `<agent_dir>/settings.json` for the managed Worktree root.
pub const WORKTREE_ROOT_SETTING_KEY: &str = "worktreeRoot";
/// Custom session entry `customType` storing a Session's static Workspace binding.
pub const WORKSPACE_BINDING_CUSTOM_TYPE: &str = "workspace_binding";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryFile {
    schema: u64,
    #[serde(default)]
    workspaces: BTreeMap<String, StoredWorkspace>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredWorkspace {
    id: String,
    name: String,
    kind: WorkspaceKind,
    root_path: String,
    last_used_at: String,
}

fn empty_registry() -> RegistryFile {
    RegistryFile {
        schema: 1,
        workspaces: BTreeMap::new(),
    }
}

/// `<agent_dir>/workspaces`.
#[must_use]
pub fn workspaces_dir(agent_dir: &Path) -> PathBuf {
    agent_dir.join(WORKSPACES_DIR_NAME)
}

/// `<agent_dir>/workspaces/registry.json`.
#[must_use]
pub fn workspace_registry_path(agent_dir: &Path) -> PathBuf {
    workspaces_dir(agent_dir).join(WORKSPACE_REGISTRY_FILE_NAME)
}

/// `<agent_dir>/workspaces/<workspace_id>`.
#[must_use]
pub fn workspace_dir(agent_dir: &Path, workspace_id: &str) -> PathBuf {
    workspaces_dir(agent_dir).join(workspace_id)
}

/// `<agent_dir>/workspaces/<workspace_id>/harness`.
#[must_use]
pub fn workspace_harness_dir(agent_dir: &Path, workspace_id: &str) -> PathBuf {
    workspace_dir(agent_dir, workspace_id).join(HARNESS_STATE_DIR_NAME)
}

/// Default managed Worktree root: `<agent_dir>/worktrees`.
#[must_use]
pub fn default_worktree_root(agent_dir: &Path) -> PathBuf {
    agent_dir.join(DEFAULT_WORKTREES_DIR_NAME)
}

fn expand_tilde_path(raw: &str) -> PathBuf {
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        if let Some(home) = pa_types::platform::home_dir() {
            return home.join(rest);
        }
    } else if raw == "~" {
        if let Some(home) = pa_types::platform::home_dir() {
            return home;
        }
    }
    PathBuf::from(raw)
}

/// Resolve the configured managed Worktree root from `<agent_dir>/settings.json`,
/// falling back to `<agent_dir>/worktrees`.
#[must_use]
pub fn configured_worktree_root(agent_dir: &Path) -> PathBuf {
    let settings_path = agent_dir.join("settings.json");
    if let Ok(text) = std::fs::read_to_string(&settings_path) {
        if let Ok(serde_json::Value::Object(map)) = serde_json::from_str(&text) {
            if let Some(raw) = map
                .get(WORKTREE_ROOT_SETTING_KEY)
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                return expand_tilde_path(raw);
            }
        }
    }
    default_worktree_root(agent_dir)
}

/// Update `worktreeRoot` in `<agent_dir>/settings.json` atomically.
///
/// # Errors
///
/// Returns an error if settings cannot be parsed or saved.
pub fn set_configured_worktree_root(agent_dir: &Path, raw_root: &str) -> Result<PathBuf> {
    let trimmed = raw_root.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("worktree root cannot be empty"));
    }
    std::fs::create_dir_all(agent_dir)?;
    let settings_path = agent_dir.join("settings.json");
    let mut root_obj = match std::fs::read_to_string(&settings_path) {
        Ok(text) => match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(serde_json::Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        },
        Err(_) => serde_json::Map::new(),
    };
    root_obj.insert(
        WORKTREE_ROOT_SETTING_KEY.to_owned(),
        serde_json::Value::String(trimmed.to_owned()),
    );
    let content = format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::Value::Object(root_obj))?
    );
    atomic_write_with(
        &settings_path,
        &content,
        AtomicWriteOptions { fsync: true },
    )?;
    Ok(expand_tilde_path(trimmed))
}

fn now_iso() -> String {
    crate::session::manager::format_iso_now()
}

/// Strip Windows verbatim prefix `\\?\` when canonicalizing paths for clean storage/display.
#[must_use]
pub fn normalize_path(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(windows)]
    {
        let s = canonical.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }
    canonical
}

/// Normalize a path string for cross-platform equality comparisons.
#[must_use]
pub fn path_comparison_key(path: &Path) -> String {
    let norm = normalize_path(path);
    let s = norm.to_string_lossy().replace('\\', "/");
    let trimmed = s.trim_end_matches('/');
    #[cfg(windows)]
    {
        trimmed.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        trimmed.to_owned()
    }
}

/// True when `path` resolves to the user's home folder (`~`).
/// Home folder is never auto-registered as a Workspace; Sessions there are Scratch sessions.
#[must_use]
pub fn is_home_dir(path: &Path) -> bool {
    let Some(home) = pa_types::platform::home_dir() else {
        return false;
    };
    path_comparison_key(path) == path_comparison_key(&home)
}

fn git_command(cwd: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn run_git_stdout(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = git_command(cwd)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git {}", args.join(" ")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(anyhow!(
            "git {} failed: {}",
            args.join(" "),
            if stderr.is_empty() {
                format!("exit {}", output.status)
            } else {
                stderr
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// If `path` is inside a Git working tree, returns `(main_checkout_root, current_worktree_root)`.
/// Both the main checkout and every linked worktree return the exact same `main_checkout_root`.
#[must_use]
pub fn resolve_git_identity(path: &Path) -> Option<(PathBuf, PathBuf)> {
    if !path.is_dir() {
        return None;
    }
    let toplevel_raw = run_git_stdout(path, &["rev-parse", "--show-toplevel"]).ok()?;
    let common_raw = run_git_stdout(path, &["rev-parse", "--git-common-dir"]).ok()?;
    let toplevel = normalize_path(Path::new(&toplevel_raw));
    let common_path = {
        let p = Path::new(&common_raw);
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            path.join(p)
        };
        normalize_path(&abs)
    };
    // For non-bare repos, `--git-common-dir` is `<main_root>/.git`.
    let main_root = if common_path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(".git"))
    {
        common_path
            .parent()
            .map(normalize_path)
            .unwrap_or_else(|| toplevel.clone())
    } else {
        toplevel.clone()
    };
    Some((main_root, toplevel))
}

/// Fast `.git/HEAD` reader for a Git worktree (handles both regular `.git/` dir
/// and linked worktree `.git` gitdir pointer files without spawning `git`).
/// Returns `(branch_or_short_oid, is_detached)`.
#[must_use]
pub fn read_worktree_head(worktree_path: &Path) -> (Option<String>, bool) {
    let dot_git = worktree_path.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else if dot_git.is_file() {
        let Ok(raw) = std::fs::read_to_string(&dot_git) else {
            return (None, false);
        };
        let Some(rest) = raw
            .lines()
            .next()
            .and_then(|l| l.trim().strip_prefix("gitdir:"))
        else {
            return (None, false);
        };
        let ptr = Path::new(rest.trim());
        if ptr.is_absolute() {
            ptr.to_path_buf()
        } else {
            worktree_path.join(ptr)
        }
    } else {
        return (None, false);
    };

    let Ok(head) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return (None, false);
    };
    let line = head.trim();
    if let Some(ref_name) = line.strip_prefix("ref:") {
        let r = ref_name.trim();
        let branch = r.strip_prefix("refs/heads/").unwrap_or(r).to_owned();
        (Some(branch), false)
    } else if line.len() >= 7 {
        (Some(line[..7].to_owned()), true)
    } else if !line.is_empty() {
        (Some(line.to_owned()), true)
    } else {
        (None, false)
    }
}

fn slugify(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 32 {
            break;
        }
    }
    let trimmed = out.trim_matches('-').to_owned();
    if trimmed.is_empty() {
        "workspace".to_owned()
    } else {
        trimmed
    }
}

fn deterministic_workspace_id(canonical_root: &Path, display_name: &str) -> String {
    let key = path_comparison_key(canonical_root);
    let digest = Sha256::digest(key.as_bytes());
    let mut short_hash = String::with_capacity(8);
    for b in &digest[..4] {
        let _ = write!(&mut short_hash, "{b:02x}");
    }
    format!("{}-{short_hash}", slugify(display_name))
}

fn display_name_for_path(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .map_or_else(|| path.to_string_lossy().into_owned(), str::to_owned)
}

fn count_workspace_harness_entries(agent_dir: &Path, workspace_id: &str) -> usize {
    let hdir = workspace_harness_dir(agent_dir, workspace_id);
    let state_path = crate::refinement::get_harness_state_path(&hdir);
    if !state_path.is_file() {
        return 0;
    }
    let state = crate::refinement::load_harness_state(&hdir, crate::refinement::HarnessScope::Workspace);
    state.entries.values().map(BTreeMap::len).sum()
}

fn load_registry_unlocked(agent_dir: &Path) -> RegistryFile {
    let path = workspace_registry_path(agent_dir);
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return empty_registry();
    };
    serde_json::from_str::<RegistryFile>(&raw).unwrap_or_else(|_| empty_registry())
}

fn update_registry<R>(
    agent_dir: &Path,
    f: impl FnOnce(&mut RegistryFile) -> Result<R>,
) -> Result<R> {
    let dir = workspaces_dir(agent_dir);
    std::fs::create_dir_all(&dir)?;
    let path = workspace_registry_path(agent_dir);
    let lock = LockDir::acquire_owned_retrying(
        &path,
        std::time::Duration::from_secs(10),
        50,
        std::time::Duration::from_millis(20),
    )?;
    let mut reg = load_registry_unlocked(agent_dir);
    let out = f(&mut reg)?;
    lock.ensure_owned()?;
    let content = format!("{}\n", serde_json::to_string_pretty(&reg)?);
    atomic_write_with(&path, &content, AtomicWriteOptions { fsync: true })?;
    Ok(out)
}

fn hydrate_record(agent_dir: &Path, stored: &StoredWorkspace) -> WorkspaceRecord {
    let missing = !Path::new(&stored.root_path).is_dir();
    let harness_entry_count = count_workspace_harness_entries(agent_dir, &stored.id);
    WorkspaceRecord {
        id: stored.id.clone(),
        name: stored.name.clone(),
        kind: stored.kind,
        root_path: stored.root_path.clone(),
        last_used_at: stored.last_used_at.clone(),
        missing,
        harness_entry_count,
    }
}

/// List all registered Workspaces, sorted most-recently-used first.
#[must_use]
pub fn list_workspaces(agent_dir: &Path) -> Vec<WorkspaceRecord> {
    let reg = load_registry_unlocked(agent_dir);
    let mut records: Vec<WorkspaceRecord> = reg
        .workspaces
        .values()
        .map(|s| hydrate_record(agent_dir, s))
        .collect();
    records.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at).then_with(|| a.name.cmp(&b.name)));
    records
}

/// Find a single registered Workspace by durable ID.
#[must_use]
pub fn find_workspace(agent_dir: &Path, workspace_id: &str) -> Option<WorkspaceRecord> {
    let reg = load_registry_unlocked(agent_dir);
    reg.workspaces
        .get(workspace_id)
        .map(|s| hydrate_record(agent_dir, s))
}

/// Register (or touch an existing) Workspace from any path inside a Git repo or plain folder.
/// Refuses the user's home folder (`~`).
///
/// # Errors
///
/// Returns an error if `path` does not exist, is the home directory, or the registry cannot be saved.
pub fn register_workspace(
    agent_dir: &Path,
    path: &Path,
    name_override: Option<&str>,
) -> Result<(WorkspaceRecord, WorkspaceBinding)> {
    if !path.is_dir() {
        return Err(anyhow!(
            "workspace folder does not exist: {}",
            path.display()
        ));
    }
    if is_home_dir(path) {
        return Err(anyhow!(
            "home folder cannot be registered as a Workspace; use a Scratch session instead"
        ));
    }
    let (kind, canonical_root, worktree_path) = match resolve_git_identity(path) {
        Some((main_root, wt_root)) => (WorkspaceKind::Git, main_root, wt_root),
        None => {
            let norm = normalize_path(path);
            (WorkspaceKind::Plain, norm.clone(), norm)
        }
    };
    let root_key = path_comparison_key(&canonical_root);
    let default_name = name_override
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or_else(|| display_name_for_path(&canonical_root), str::to_owned);
    let now = now_iso();

    let stored = update_registry(agent_dir, |reg| {
        if let Some(existing) = reg
            .workspaces
            .values_mut()
            .find(|w| path_comparison_key(Path::new(&w.root_path)) == root_key)
        {
            existing.last_used_at.clone_from(&now);
            existing.kind = kind;
            existing.root_path = canonical_root.to_string_lossy().into_owned();
            if let Some(name) = name_override.map(str::trim).filter(|s| !s.is_empty()) {
                existing.name = name.to_owned();
            }
            return Ok(existing.clone());
        }
        let id = deterministic_workspace_id(&canonical_root, &default_name);
        let entry = StoredWorkspace {
            id: id.clone(),
            name: default_name.clone(),
            kind,
            root_path: canonical_root.to_string_lossy().into_owned(),
            last_used_at: now.clone(),
        };
        reg.workspaces.insert(id, entry.clone());
        Ok(entry)
    })?;

    let record = hydrate_record(agent_dir, &stored);
    let binding = build_binding_from_parts(&record, &worktree_path);
    Ok((record, binding))
}

/// Update `last_used_at` on an existing Workspace.
///
/// # Errors
///
/// Returns an error if the workspace does not exist or saving fails.
pub fn touch_workspace(agent_dir: &Path, workspace_id: &str) -> Result<WorkspaceRecord> {
    let now = now_iso();
    let stored = update_registry(agent_dir, |reg| {
        let entry = reg
            .workspaces
            .get_mut(workspace_id)
            .ok_or_else(|| anyhow!("unknown workspace: {workspace_id}"))?;
        entry.last_used_at = now;
        Ok(entry.clone())
    })?;
    Ok(hydrate_record(agent_dir, &stored))
}

/// Relink an existing Workspace to a new folder on disk while keeping its durable `id`
/// and Workspace continual harness intact.
///
/// # Errors
///
/// Returns an error if `new_path` does not exist, is the home folder, or `workspace_id` is unknown.
pub fn relink_workspace(
    agent_dir: &Path,
    workspace_id: &str,
    new_path: &Path,
) -> Result<WorkspaceRecord> {
    if !new_path.is_dir() {
        return Err(anyhow!(
            "relink folder does not exist: {}",
            new_path.display()
        ));
    }
    if is_home_dir(new_path) {
        return Err(anyhow!("cannot relink a Workspace to the home folder"));
    }
    let (kind, canonical_root) = match resolve_git_identity(new_path) {
        Some((main_root, _)) => (WorkspaceKind::Git, main_root),
        None => (WorkspaceKind::Plain, normalize_path(new_path)),
    };
    let now = now_iso();
    let stored = update_registry(agent_dir, |reg| {
        let entry = reg
            .workspaces
            .get_mut(workspace_id)
            .ok_or_else(|| anyhow!("unknown workspace: {workspace_id}"))?;
        entry.kind = kind;
        entry.root_path = canonical_root.to_string_lossy().into_owned();
        entry.last_used_at = now;
        Ok(entry.clone())
    })?;
    Ok(hydrate_record(agent_dir, &stored))
}

/// Delete a Workspace from the registry and optionally remove its harness folder.
///
/// # Errors
///
/// Returns an error if the registry update or harness directory removal fails.
pub fn delete_workspace(
    agent_dir: &Path,
    workspace_id: &str,
    delete_harness: bool,
) -> Result<bool> {
    let removed = update_registry(agent_dir, |reg| Ok(reg.workspaces.remove(workspace_id).is_some()))?;
    if removed && delete_harness {
        let dir = workspace_dir(agent_dir, workspace_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
    }
    Ok(removed)
}

fn build_binding_from_parts(workspace: &WorkspaceRecord, worktree_path: &Path) -> WorkspaceBinding {
    let norm_wt = normalize_path(worktree_path);
    let branch = if workspace.kind == WorkspaceKind::Git {
        read_worktree_head(&norm_wt).0
    } else {
        None
    };
    let worktree_name = if path_comparison_key(&norm_wt)
        == path_comparison_key(Path::new(&workspace.root_path))
    {
        workspace.name.clone()
    } else {
        display_name_for_path(&norm_wt)
    };
    WorkspaceBinding {
        workspace_id: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        workspace_kind: workspace.kind,
        workspace_root: workspace.root_path.clone(),
        worktree_path: norm_wt.to_string_lossy().into_owned(),
        worktree_name,
        branch,
    }
}

/// Build a validated `WorkspaceBinding` for a Worktree inside `workspace`.
/// Rejects paths that belong to a different Git repository or outside a plain Workspace.
///
/// # Errors
///
/// Returns an error if `worktree_path` does not exist or does not belong to `workspace`.
pub fn binding_for_worktree(
    workspace: &WorkspaceRecord,
    worktree_path: &Path,
) -> Result<WorkspaceBinding> {
    if !worktree_path.is_dir() {
        return Err(anyhow!(
            "worktree folder does not exist: {}",
            worktree_path.display()
        ));
    }
    let ws_root_key = path_comparison_key(Path::new(&workspace.root_path));
    match workspace.kind {
        WorkspaceKind::Git => {
            let Some((main_root, wt_root)) = resolve_git_identity(worktree_path) else {
                return Err(anyhow!(
                    "{} is not a Git worktree of Workspace {}",
                    worktree_path.display(),
                    workspace.name
                ));
            };
            if path_comparison_key(&main_root) != ws_root_key {
                return Err(anyhow!(
                    "{} belongs to a different Git repository than Workspace {}",
                    worktree_path.display(),
                    workspace.name
                ));
            }
            Ok(build_binding_from_parts(workspace, &wt_root))
        }
        WorkspaceKind::Plain => {
            if path_comparison_key(worktree_path) != ws_root_key {
                return Err(anyhow!(
                    "plain Workspace {} does not support linked worktrees",
                    workspace.name
                ));
            }
            Ok(build_binding_from_parts(workspace, worktree_path))
        }
    }
}

/// List all Git worktrees (main checkout + linked worktrees) for a Workspace.
///
/// # Errors
///
/// Returns an error if the workspace folder is missing or `git worktree list` fails.
pub fn list_worktrees(workspace: &WorkspaceRecord) -> Result<Vec<WorktreeInfo>> {
    let root = Path::new(&workspace.root_path);
    if !root.is_dir() {
        return Err(anyhow!("workspace folder is missing: {}", workspace.root_path));
    }
    if workspace.kind == WorkspaceKind::Plain {
        return Ok(vec![WorktreeInfo {
            path: workspace.root_path.clone(),
            name: workspace.name.clone(),
            branch: None,
            head_oid: None,
            is_main: true,
            detached: false,
        }]);
    }
    let raw = run_git_stdout(root, &["worktree", "list", "--porcelain"])?;
    let root_key = path_comparison_key(root);
    let mut items = Vec::new();
    let mut cur_path: Option<PathBuf> = None;
    let mut cur_head: Option<String> = None;
    let mut cur_branch: Option<String> = None;
    let mut cur_detached = false;
    let mut cur_bare = false;

    let flush = |items: &mut Vec<WorktreeInfo>,
                 cur_path: &mut Option<PathBuf>,
                 cur_head: &mut Option<String>,
                 cur_branch: &mut Option<String>,
                 cur_detached: &mut bool,
                 cur_bare: &mut bool| {
        if let Some(p) = cur_path.take() {
            if !*cur_bare {
                let norm = normalize_path(&p);
                let is_main = path_comparison_key(&norm) == root_key;
                let name = if is_main {
                    workspace.name.clone()
                } else {
                    display_name_for_path(&norm)
                };
                items.push(WorktreeInfo {
                    path: norm.to_string_lossy().into_owned(),
                    name,
                    branch: cur_branch.take(),
                    head_oid: cur_head.take(),
                    is_main,
                    detached: *cur_detached,
                });
            }
        }
        *cur_head = None;
        *cur_branch = None;
        *cur_detached = false;
        *cur_bare = false;
    };

    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush(
                &mut items,
                &mut cur_path,
                &mut cur_head,
                &mut cur_branch,
                &mut cur_detached,
                &mut cur_bare,
            );
            continue;
        }
        if let Some(p) = trimmed.strip_prefix("worktree ") {
            cur_path = Some(PathBuf::from(p.trim()));
        } else if let Some(h) = trimmed.strip_prefix("HEAD ") {
            cur_head = Some(h.trim().to_owned());
        } else if let Some(b) = trimmed.strip_prefix("branch ") {
            let b = b.trim();
            cur_branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_owned());
        } else if trimmed == "detached" {
            cur_detached = true;
        } else if trimmed == "bare" {
            cur_bare = true;
        }
    }
    flush(
        &mut items,
        &mut cur_path,
        &mut cur_head,
        &mut cur_branch,
        &mut cur_detached,
        &mut cur_bare,
    );
    Ok(items)
}

fn sanitize_worktree_name(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("worktree name cannot be empty"));
    }
    if trimmed.contains("..") || trimmed.starts_with('-') {
        return Err(anyhow!("invalid worktree name: {trimmed}"));
    }
    let dir_name: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let dir_name = dir_name.trim_matches(['-', '.']).to_owned();
    if dir_name.is_empty() {
        return Err(anyhow!("invalid worktree name: {trimmed}"));
    }
    Ok(dir_name)
}

/// Create a linked Git Worktree for `workspace` under `<worktree_root>/<workspace_slug>/<name>`.
///
/// # Errors
///
/// Returns an error if the workspace is not a Git repo, `name` is invalid, target already exists,
/// or `git worktree add` fails.
pub fn create_worktree(
    agent_dir: &Path,
    workspace: &WorkspaceRecord,
    name: &str,
    base_branch: Option<&str>,
) -> Result<WorktreeInfo> {
    if workspace.kind != WorkspaceKind::Git {
        return Err(anyhow!(
            "Workspace {} is not a Git repository",
            workspace.name
        ));
    }
    let root = Path::new(&workspace.root_path);
    if !root.is_dir() {
        return Err(anyhow!("workspace folder is missing: {}", workspace.root_path));
    }
    let branch_name = name.trim();
    let folder_name = sanitize_worktree_name(branch_name)?;
    let wt_root = configured_worktree_root(agent_dir);
    let ws_bucket = wt_root.join(slugify(&workspace.name));
    std::fs::create_dir_all(&ws_bucket)?;
    let target = ws_bucket.join(&folder_name);
    if target.exists() {
        return Err(anyhow!(
            "worktree target folder already exists: {}",
            target.display()
        ));
    }
    let target_str = target.to_string_lossy().into_owned();

    // Check if local branch already exists.
    let branch_ref = format!("refs/heads/{branch_name}");
    let branch_exists = run_git_stdout(root, &["show-ref", "--verify", "--quiet", &branch_ref]).is_ok();
    if branch_exists {
        run_git_stdout(root, &["worktree", "add", "--", &target_str, branch_name])?;
    } else if let Some(base) = base_branch.map(str::trim).filter(|s| !s.is_empty()) {
        run_git_stdout(root, &["worktree", "add", "-b", branch_name, &target_str, base])?;
    } else {
        run_git_stdout(root, &["worktree", "add", "-b", branch_name, &target_str])?;
    }

    let norm = normalize_path(&target);
    let (branch, detached) = read_worktree_head(&norm);
    Ok(WorktreeInfo {
        path: norm.to_string_lossy().into_owned(),
        name: folder_name,
        branch,
        head_oid: run_git_stdout(&norm, &["rev-parse", "HEAD"]).ok(),
        is_main: false,
        detached,
    })
}

/// List local branches for a Git worktree.
///
/// # Errors
///
/// Returns an error if `worktree_path` is missing or `git branch` fails.
pub fn list_branches(worktree_path: &Path) -> Result<Vec<BranchInfo>> {
    if !worktree_path.is_dir() {
        return Err(anyhow!(
            "worktree folder does not exist: {}",
            worktree_path.display()
        ));
    }
    let raw = run_git_stdout(
        worktree_path,
        &[
            "branch",
            "--format=%(refname:short)\t%(HEAD)\t%(worktreepath)",
        ],
    )?;
    let mut out = Vec::new();
    for line in raw.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let name = parts.next().unwrap_or("").trim().to_owned();
        let head = parts.next().unwrap_or("").trim();
        let wt = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| normalize_path(Path::new(s)).to_string_lossy().into_owned());
        if !name.is_empty() {
            out.push(BranchInfo {
                name,
                is_current: head == "*",
                worktree_path: wt,
            });
        }
    }
    Ok(out)
}

/// Switch the checked-out branch inside `worktree_path` in place.
///
/// # Errors
///
/// Returns an error if the branch name is invalid or `git switch` fails.
pub fn switch_branch(worktree_path: &Path, branch: &str) -> Result<Option<String>> {
    let trimmed = branch.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return Err(anyhow!("invalid branch name: {trimmed}"));
    }
    run_git_stdout(worktree_path, &["switch", "--", trimmed])?;
    Ok(read_worktree_head(worktree_path).0)
}

/// Create and check out a new branch inside `worktree_path`.
///
/// # Errors
///
/// Returns an error if the branch name is invalid or `git switch -c` fails.
pub fn create_branch(
    worktree_path: &Path,
    branch: &str,
    start_point: Option<&str>,
) -> Result<Option<String>> {
    let trimmed = branch.trim();
    if trimmed.is_empty() || trimmed.starts_with('-') {
        return Err(anyhow!("invalid branch name: {trimmed}"));
    }
    if let Some(sp) = start_point.map(str::trim).filter(|s| !s.is_empty()) {
        run_git_stdout(worktree_path, &["switch", "-c", trimmed, sp])?;
    } else {
        run_git_stdout(worktree_path, &["switch", "-c", trimmed])?;
    }
    Ok(read_worktree_head(worktree_path).0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn init_git_repo(dir: &Path) {
        assert!(git_command(dir).args(["init", "-b", "main"]).status().unwrap().success());
        assert!(git_command(dir)
            .args(["config", "user.email", "test@example.com"])
            .status()
            .unwrap()
            .success());
        assert!(git_command(dir)
            .args(["config", "user.name", "Test"])
            .status()
            .unwrap()
            .success());
        std::fs::write(dir.join("README.md"), "hello\n").unwrap();
        assert!(git_command(dir).args(["add", "."]).status().unwrap().success());
        assert!(git_command(dir)
            .args(["commit", "-m", "init"])
            .status()
            .unwrap()
            .success());
    }

    #[test]
    fn linked_worktree_shares_workspace_identity_and_branch_ops_work() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let repo_dir = tmp.path().join("my-repo");
        std::fs::create_dir_all(&repo_dir).unwrap();
        init_git_repo(&repo_dir);

        let (ws, binding) = register_workspace(&agent_dir, &repo_dir, None).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Git);
        assert_eq!(binding.branch.as_deref(), Some("main"));

        // Create managed worktree under <agent_dir>/worktrees/my-repo/feat-a
        let wt = create_worktree(&agent_dir, &ws, "feat-a", None).unwrap();
        assert!(!wt.is_main);
        assert_eq!(wt.branch.as_deref(), Some("feat-a"));

        // Registering from the linked worktree returns the exact same Workspace ID
        let (ws2, binding2) = register_workspace(&agent_dir, Path::new(&wt.path), None).unwrap();
        assert_eq!(ws2.id, ws.id);
        assert_eq!(binding2.branch.as_deref(), Some("feat-a"));

        // Branch create & switch inside linked worktree
        let b = create_branch(Path::new(&wt.path), "feat-b", None).unwrap();
        assert_eq!(b.as_deref(), Some("feat-b"));
        assert_eq!(read_worktree_head(Path::new(&wt.path)).0.as_deref(), Some("feat-b"));

        let branches = list_branches(Path::new(&wt.path)).unwrap();
        assert!(branches.iter().any(|x| x.name == "feat-b" && x.is_current));
        assert!(branches.iter().any(|x| x.name == "main"));

        let worktrees = list_worktrees(&ws).unwrap();
        assert_eq!(worktrees.len(), 2);
    }

    #[test]
    fn relink_and_delete_workspace_preserve_and_cleanup() {
        let tmp = TempDir::new().unwrap();
        let agent_dir = tmp.path().join("agent");
        let dir_a = tmp.path().join("plain-a");
        let dir_b = tmp.path().join("plain-b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let (ws, _) = register_workspace(&agent_dir, &dir_a, None).unwrap();
        assert_eq!(ws.kind, WorkspaceKind::Plain);
        std::fs::create_dir_all(workspace_harness_dir(&agent_dir, &ws.id)).unwrap();
        std::fs::write(workspace_harness_dir(&agent_dir, &ws.id).join("marker"), "ok").unwrap();

        let relinked = relink_workspace(&agent_dir, &ws.id, &dir_b).unwrap();
        assert_eq!(relinked.id, ws.id);
        assert_eq!(path_comparison_key(Path::new(&relinked.root_path)), path_comparison_key(&dir_b));
        assert!(workspace_harness_dir(&agent_dir, &ws.id).join("marker").exists());

        assert!(delete_workspace(&agent_dir, &ws.id, true).unwrap());
        assert!(find_workspace(&agent_dir, &ws.id).is_none());
        assert!(!workspace_dir(&agent_dir, &ws.id).exists());
    }
}
