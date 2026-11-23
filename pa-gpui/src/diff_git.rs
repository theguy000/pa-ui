//! Synchronous Git diff loader meant to run on background threads.

use crate::settings::DiffScope;
use gpui_kit::component::diff::{DiffFile, DiffParseOptions};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub(crate) const UNTRACKED_MAX_BYTES: u64 = 256 * 1024;
pub(crate) const LARGE_DIFF_LINE_THRESHOLD: usize = 1_500;
const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";
const EAGER_DIFF_PATH_CHUNK: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeEntry {
    pub(crate) path: PathBuf,
    pub(crate) label: String,
    pub(crate) branch: Option<String>,
    pub(crate) is_main: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RepoContext {
    pub(crate) repo_root: PathBuf,
    pub(crate) worktrees: Vec<WorktreeEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffFileSummary {
    pub(crate) path: String,
    pub(crate) old_path: Option<String>,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    pub(crate) binary: bool,
    pub(crate) untracked: bool,
    pub(crate) truncated: bool,
    pub(crate) deferred: bool,
}

pub(crate) struct LoadedDiff {
    pub(crate) repo_root: PathBuf,
    pub(crate) base_ref: String,
    pub(crate) base_branch: Option<String>,
    pub(crate) on_base_branch: bool,
    pub(crate) branches: Vec<String>,
    pub(crate) worktrees: Vec<WorktreeEntry>,
    pub(crate) files: Vec<DiffFile>,
    pub(crate) summaries: Vec<DiffFileSummary>,
    pub(crate) total_additions: usize,
    pub(crate) total_deletions: usize,
}

pub(crate) enum DiffLoadOutcome {
    NotAGitRepo,
    Loaded(LoadedDiff),
    Error(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NumstatEntry {
    path: String,
    old_path: Option<String>,
    additions: usize,
    deletions: usize,
    binary: bool,
}

pub(crate) fn is_lock_or_generated_path(path: &str) -> bool {
    let file_name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if matches!(
        file_name,
        "Cargo.lock" | "package-lock.json" | "pnpm-lock.yaml" | "yarn.lock"
    ) {
        return true;
    }
    let lower = file_name.to_ascii_lowercase();
    lower.contains(".min.")
}

pub(crate) fn should_defer_file(path: &str, additions: usize, deletions: usize) -> bool {
    is_lock_or_generated_path(path) || additions.saturating_add(deletions) > LARGE_DIFF_LINE_THRESHOLD
}

fn run_git(dir: &Path, args: &[&str]) -> std::io::Result<Output> {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.output()
}

pub(crate) fn resolve_worktree_root(cwd: &Path) -> Option<PathBuf> {
    if cwd.as_os_str().is_empty() || !cwd.exists() {
        return None;
    }
    let out = run_git(cwd, &["rev-parse", "--show-toplevel"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if root.is_empty() {
        None
    } else {
        Some(PathBuf::from(root))
    }
}

fn has_head_commit(repo_root: &Path) -> bool {
    run_git(repo_root, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .is_ok_and(|out| out.status.success())
}

fn parse_numstat_z(stdout: &[u8]) -> Vec<NumstatEntry> {
    let mut entries = Vec::new();
    let mut parts = stdout.split(|&b| b == 0).peekable();
    while let Some(raw) = parts.next() {
        if raw.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(raw);
        let mut cols = line.splitn(3, '\t');
        let (Some(add_s), Some(del_s), Some(path_col)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let binary = add_s == "-" || del_s == "-";
        let additions = if binary { 0 } else { add_s.parse().unwrap_or(0) };
        let deletions = if binary { 0 } else { del_s.parse().unwrap_or(0) };
        let (old_path, path) = if path_col.is_empty() {
            // `git diff --numstat -z` encodes renames/copies as `add\tdel\t\0old\0new\0`.
            let Some(old_raw) = parts.next() else {
                break;
            };
            let Some(new_raw) = parts.next() else {
                break;
            };
            let old_s = String::from_utf8_lossy(old_raw).into_owned();
            let new_s = String::from_utf8_lossy(new_raw).into_owned();
            (Some(old_s), new_s)
        } else {
            (None, path_col.to_string())
        };
        entries.push(NumstatEntry {
            path,
            old_path,
            additions,
            deletions,
            binary,
        });
    }
    entries
}

fn parse_patch_resilient(patch: &str) -> Vec<DiffFile> {
    if patch.trim().is_empty() {
        return Vec::new();
    }
    let opts = DiffParseOptions::new().truncated_hunks(true);
    if let Ok(files) = DiffFile::parse_with(patch, opts) {
        return files;
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    let bytes = patch.as_bytes();
    while start < bytes.len() {
        let next = patch[start..]
            .find("\ndiff --git ")
            .map(|rel| start + rel + 1)
            .unwrap_or(bytes.len());
        let chunk = &patch[start..next];
        if let Ok(mut parsed) = DiffFile::parse_with(chunk, opts) {
            out.append(&mut parsed);
        }
        start = next;
    }
    out
}

fn is_probably_binary(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(8192)];
    sample.contains(&0)
}

fn synthesize_deferred_placeholder(
    path: &str,
    additions: usize,
    deletions: usize,
) -> Option<DiffFile> {
    let git_path = path.replace('\\', "/");
    let patch = format!(
        "diff --git a/{p} b/{p}\nLarge or generated file (+{additions} -{deletions}) — loading diff on demand\n",
        p = git_path
    );
    let opts = DiffParseOptions::new().truncated_hunks(true);
    DiffFile::parse_with(&patch, opts).ok()?.into_iter().next()
}

fn read_untracked_bytes(repo_root: &Path, rel_path: &str) -> Option<(Vec<u8>, bool)> {
    let full = repo_root.join(rel_path);
    let mut file = File::open(&full).ok()?;
    let limit = UNTRACKED_MAX_BYTES as usize;
    let mut buf = Vec::new();
    file.by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut buf)
        .ok()?;
    let truncated = buf.len() > limit;
    if truncated {
        buf.truncate(limit);
    }
    Some((buf, truncated))
}

fn count_text_lines(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    let count = text.bytes().filter(|&b| b == b'\n').count();
    if text.ends_with('\n') {
        count
    } else {
        count + 1
    }
}

fn decode_text_capped(buf: &[u8], truncated: bool) -> String {
    match std::str::from_utf8(buf) {
        Ok(s) => s.to_string(),
        Err(err) if truncated => String::from_utf8_lossy(&buf[..err.valid_up_to()]).into_owned(),
        Err(_) => String::from_utf8_lossy(buf).into_owned(),
    }
}

fn build_untracked_diff_file(repo_root: &Path, rel_path: &str) -> Option<(DiffFile, bool)> {
    let (buf, truncated) = read_untracked_bytes(repo_root, rel_path)?;
    let git_path = rel_path.replace('\\', "/");
    let patch = if is_probably_binary(&buf) {
        format!(
            "diff --git a/{p} b/{p}\nnew file mode 100644\nBinary files /dev/null and b/{p} differ\n",
            p = git_path
        )
    } else {
        let text = decode_text_capped(&buf, truncated);
        synthesize_added_file_patch(&git_path, &text)
    };

    let opts = DiffParseOptions::new().truncated_hunks(true);
    let parsed = DiffFile::parse_with(&patch, opts).ok()?.into_iter().next()?;
    Some((parsed, truncated))
}

fn load_untracked_entry(
    repo_root: &Path,
    rel_path: &str,
) -> Option<(DiffFile, DiffFileSummary)> {
    let (buf, truncated) = read_untracked_bytes(repo_root, rel_path)?;
    let git_path = rel_path.replace('\\', "/");
    let binary = is_probably_binary(&buf);
    if !binary {
        let text = decode_text_capped(&buf, truncated);
        let line_count = count_text_lines(&text);
        if should_defer_file(&git_path, line_count, 0) {
            let placeholder = synthesize_deferred_placeholder(&git_path, line_count, 0)?;
            let summary = DiffFileSummary {
                path: git_path,
                old_path: None,
                additions: line_count,
                deletions: 0,
                binary: false,
                untracked: true,
                truncated,
                deferred: true,
            };
            return Some((placeholder, summary));
        }
        let patch = synthesize_added_file_patch(&git_path, &text);
        let opts = DiffParseOptions::new().truncated_hunks(true);
        let df = DiffFile::parse_with(&patch, opts).ok()?.into_iter().next()?;
        let summary = summary_from_diff_file(&df, true, truncated, false);
        return Some((df, summary));
    }

    let patch = format!(
        "diff --git a/{p} b/{p}\nnew file mode 100644\nBinary files /dev/null and b/{p} differ\n",
        p = git_path
    );
    let opts = DiffParseOptions::new().truncated_hunks(true);
    let df = DiffFile::parse_with(&patch, opts).ok()?.into_iter().next()?;
    let summary = summary_from_diff_file(&df, true, truncated, false);
    Some((df, summary))
}

fn synthesize_added_file_patch(git_path: &str, text: &str) -> String {
    if text.is_empty() {
        return format!(
            "diff --git a/{p} b/{p}\nnew file mode 100644\n",
            p = git_path
        );
    }
    let has_trailing_nl = text.ends_with('\n');
    let lines: Vec<&str> = text.split('\n').collect();
    let line_count = if has_trailing_nl {
        lines.len().saturating_sub(1)
    } else {
        lines.len()
    };
    let mut patch = String::with_capacity(text.len() + 128);
    patch.push_str(&format!(
        "diff --git a/{p} b/{p}\nnew file mode 100644\n--- /dev/null\n+++ b/{p}\n@@ -0,0 +1,{n} @@\n",
        p = git_path,
        n = line_count,
    ));
    for line in lines.iter().take(line_count) {
        let clean = line.strip_suffix('\r').unwrap_or(line);
        patch.push('+');
        patch.push_str(clean);
        patch.push('\n');
    }
    if !has_trailing_nl {
        patch.push_str("\\ No newline at end of file\n");
    }
    patch
}

fn fetch_tracked_patches(
    repo_root: &Path,
    base_ref: &str,
    entries: &[&NumstatEntry],
) -> Result<Vec<DiffFile>, String> {
    let mut out_files = Vec::new();
    for chunk in entries.chunks(EAGER_DIFF_PATH_CHUNK) {
        let mut args = vec![
            "-c",
            "core.quotepath=false",
            "diff",
            base_ref,
            "--no-ext-diff",
            "--no-color",
            "--unified=3",
            "--",
        ];
        for entry in chunk {
            args.push(entry.path.as_str());
            if let Some(old) = entry.old_path.as_deref() {
                args.push(old);
            }
        }
        let output = run_git(repo_root, &args)
            .map_err(|err| format!("Failed to run git diff: {err}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            return Err(if stderr.is_empty() {
                "git diff failed".into()
            } else {
                stderr
            });
        }
        let patch = String::from_utf8_lossy(&output.stdout);
        out_files.extend(parse_patch_resilient(&patch));
    }
    Ok(out_files)
}

pub(crate) fn load_single_file_diff(
    repo_root: &Path,
    base_ref: &str,
    path: &str,
    old_path: Option<&str>,
    untracked: bool,
) -> Option<DiffFile> {
    if untracked {
        return build_untracked_diff_file(repo_root, path).map(|(df, _)| df);
    }
    let entry = NumstatEntry {
        path: path.to_string(),
        old_path: old_path.map(str::to_string),
        additions: 0,
        deletions: 0,
        binary: false,
    };
    let files = fetch_tracked_patches(repo_root, base_ref, &[&entry]).ok()?;
    files.into_iter().next()
}

pub(crate) fn same_worktree_path(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        return ca == cb;
    }
    let norm = |p: &Path| {
        let s = p.to_string_lossy().replace('\\', "/");
        #[cfg(target_os = "windows")]
        {
            s.to_ascii_lowercase()
        }
        #[cfg(not(target_os = "windows"))]
        {
            s
        }
    };
    norm(a) == norm(b)
}

fn git_commit_ref_exists(repo_root: &Path, rev: &str) -> bool {
    let target = format!("{rev}^{{commit}}");
    run_git(repo_root, &["rev-parse", "--verify", "--quiet", &target])
        .is_ok_and(|out| out.status.success())
}

pub(crate) fn current_branch_name(repo_root: &Path) -> Option<String> {
    let out = run_git(repo_root, &["symbolic-ref", "--quiet", "--short", "HEAD"]).ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

pub(crate) fn is_on_base_branch(current_branch: Option<&str>, base_branch: &str) -> bool {
    let base_short = base_branch.strip_prefix("origin/").unwrap_or(base_branch);
    current_branch.is_some_and(|cur| cur == base_branch || cur == base_short)
}

pub(crate) fn list_branches(repo_root: &Path) -> Vec<String> {
    let Ok(out) = run_git(
        repo_root,
        &[
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads",
            "refs/remotes/origin",
        ],
    ) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut seen = HashSet::new();
    let mut branches = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let name = line.trim();
        if name.is_empty() || name == "origin" || name == "origin/HEAD" {
            continue;
        }
        if seen.insert(name.to_string()) {
            branches.push(name.to_string());
        }
    }
    branches
}

pub(crate) fn detect_base_branch(repo_root: &Path) -> Option<String> {
    if let Ok(out) = run_git(
        repo_root,
        &["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"],
    ) && out.status.success()
    {
        let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if let Some(short) = full.strip_prefix("refs/remotes/origin/") {
            if git_commit_ref_exists(repo_root, short) {
                return Some(short.to_string());
            }
            let remote_ref = format!("origin/{short}");
            if git_commit_ref_exists(repo_root, &remote_ref) {
                return Some(remote_ref);
            }
        }
    }

    for candidate in ["main", "origin/main", "master", "origin/master"] {
        if git_commit_ref_exists(repo_root, candidate) {
            return Some(candidate.to_string());
        }
    }

    if let Some(cur) = current_branch_name(repo_root)
        && git_commit_ref_exists(repo_root, &cur)
    {
        return Some(cur);
    }

    list_branches(repo_root).into_iter().next()
}

pub(crate) fn parse_worktree_list_porcelain(stdout: &str) -> Vec<WorktreeEntry> {
    let mut out = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut is_bare = false;
    let mut is_prunable = false;

    let flush = |out: &mut Vec<WorktreeEntry>,
                 path: &mut Option<PathBuf>,
                 branch: &mut Option<String>,
                 is_bare: &mut bool,
                 is_prunable: &mut bool| {
        if let Some(p) = path.take()
            && !*is_bare
            && !*is_prunable
        {
            let label = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| p.to_string_lossy().into_owned());
            let is_main = out.is_empty();
            out.push(WorktreeEntry {
                path: p,
                label,
                branch: branch.take(),
                is_main,
            });
        }
        *branch = None;
        *is_bare = false;
        *is_prunable = false;
    };

    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            flush(&mut out, &mut path, &mut branch, &mut is_bare, &mut is_prunable);
            continue;
        }
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut out, &mut path, &mut branch, &mut is_bare, &mut is_prunable);
            path = Some(PathBuf::from(rest.trim()));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            let b = rest.trim();
            branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_string());
        } else if line == "bare" {
            is_bare = true;
        } else if line == "prunable" || line.starts_with("prunable ") {
            is_prunable = true;
        }
    }
    flush(&mut out, &mut path, &mut branch, &mut is_bare, &mut is_prunable);
    out
}

fn path_mtime_secs(path: &Path) -> u64 {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn resolve_worktree_git_dir(worktree_path: &Path) -> Option<PathBuf> {
    let dot_git = worktree_path.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let raw = fs::read_to_string(&dot_git).ok()?;
    let rest = raw.trim().strip_prefix("gitdir:")?.trim();
    if rest.is_empty() {
        return None;
    }
    let parsed = PathBuf::from(rest);
    if parsed.is_absolute() {
        Some(parsed)
    } else {
        Some(worktree_path.join(parsed))
    }
}

fn worktree_activity_secs(worktree_path: &Path) -> u64 {
    let mut best = path_mtime_secs(worktree_path);
    if let Some(git_dir) = resolve_worktree_git_dir(worktree_path) {
        best = best
            .max(path_mtime_secs(&git_dir.join("index")))
            .max(path_mtime_secs(&git_dir.join("HEAD")))
            .max(path_mtime_secs(&git_dir.join("logs").join("HEAD")));
    }
    best
}

pub(crate) fn list_linked_worktrees(repo_root: &Path) -> Vec<WorktreeEntry> {
    let Ok(out) = run_git(repo_root, &["worktree", "list", "--porcelain"]) else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut live: Vec<(u64, WorktreeEntry)> =
        parse_worktree_list_porcelain(&String::from_utf8_lossy(&out.stdout))
            .into_iter()
            .filter(|wt| wt.path.is_dir())
            .map(|wt| (worktree_activity_secs(&wt.path), wt))
            .collect();
    live.sort_by(|(a_ts, a_wt), (b_ts, b_wt)| {
        b_ts.cmp(a_ts)
            .then_with(|| b_wt.is_main.cmp(&a_wt.is_main))
            .then_with(|| a_wt.label.cmp(&b_wt.label))
    });
    live.into_iter().map(|(_, wt)| wt).collect()
}

pub(crate) fn main_project_root_for(path: &Path) -> PathBuf {
    probe_repo_context(path)
        .and_then(|ctx| {
            ctx.worktrees
                .into_iter()
                .find(|wt| wt.is_main)
                .map(|wt| wt.path)
        })
        .unwrap_or_else(|| path.to_path_buf())
}

pub(crate) fn probe_repo_context(cwd: &Path) -> Option<RepoContext> {
    let repo_root = resolve_worktree_root(cwd)?;
    let worktrees = list_linked_worktrees(&repo_root);
    Some(RepoContext {
        repo_root,
        worktrees,
    })
}

fn resolve_merge_base(repo_root: &Path, base_branch: &str) -> Result<String, String> {
    if !has_head_commit(repo_root) {
        return Ok(EMPTY_TREE_OID.to_string());
    }
    let out = run_git(repo_root, &["merge-base", "HEAD", base_branch])
        .map_err(|err| format!("Failed to run git merge-base: {err}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!("Failed to find merge-base with {base_branch}")
        } else {
            stderr
        });
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        Err(format!("Empty merge-base for {base_branch}"))
    } else {
        Ok(sha)
    }
}

pub(crate) fn load_diff_for_scope(
    cwd: &Path,
    scope: DiffScope,
    requested_base: Option<&str>,
) -> DiffLoadOutcome {
    let Some(repo_root) = resolve_worktree_root(cwd) else {
        return DiffLoadOutcome::NotAGitRepo;
    };
    let worktrees = list_linked_worktrees(&repo_root);
    let current_branch = current_branch_name(&repo_root);

    match scope {
        DiffScope::Uncommitted => {
            let base_ref = if has_head_commit(&repo_root) {
                "HEAD".to_string()
            } else {
                EMPTY_TREE_OID.to_string()
            };
            let mut outcome = load_diff_against_base(repo_root, base_ref);
            if let DiffLoadOutcome::Loaded(ref mut loaded) = outcome {
                loaded.worktrees = worktrees;
            }
            outcome
        }
        DiffScope::Branch => {
            let branches = list_branches(&repo_root);
            let selected_base = requested_base
                .map(str::trim)
                .filter(|b| !b.is_empty() && git_commit_ref_exists(&repo_root, b))
                .map(str::to_string)
                .or_else(|| detect_base_branch(&repo_root));

            let (base_ref, base_branch, on_base_branch) = match selected_base {
                Some(base_name) => {
                    let on_base = is_on_base_branch(current_branch.as_deref(), &base_name);
                    let merge_base = match resolve_merge_base(&repo_root, &base_name) {
                        Ok(sha) => sha,
                        Err(err) => return DiffLoadOutcome::Error(err),
                    };
                    (merge_base, Some(base_name), on_base)
                }
                None => (EMPTY_TREE_OID.to_string(), None, false),
            };

            let mut outcome = load_diff_against_base(repo_root, base_ref);
            if let DiffLoadOutcome::Loaded(ref mut loaded) = outcome {
                loaded.base_branch = base_branch;
                loaded.on_base_branch = on_base_branch;
                loaded.branches = branches;
                loaded.worktrees = worktrees;
            }
            outcome
        }
    }
}

#[cfg(test)]
pub(crate) fn load_uncommitted_diff(cwd: &Path) -> DiffLoadOutcome {
    load_diff_for_scope(cwd, DiffScope::Uncommitted, None)
}

pub(crate) fn load_diff_against_base(repo_root: PathBuf, base_ref: String) -> DiffLoadOutcome {
    let numstat_out = match run_git(
        &repo_root,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            &base_ref,
            "--numstat",
            "--no-ext-diff",
            "-z",
        ],
    ) {
        Ok(out) => out,
        Err(err) => return DiffLoadOutcome::Error(format!("Failed to run git diff: {err}")),
    };
    if !numstat_out.status.success() {
        let stderr = String::from_utf8_lossy(&numstat_out.stderr)
            .trim()
            .to_string();
        return DiffLoadOutcome::Error(if stderr.is_empty() {
            "git diff failed".into()
        } else {
            stderr
        });
    }

    let entries = parse_numstat_z(&numstat_out.stdout);
    let eager_entries: Vec<&NumstatEntry> = entries
        .iter()
        .filter(|entry| !should_defer_file(&entry.path, entry.additions, entry.deletions))
        .collect();

    let eager_files = if eager_entries.is_empty() {
        Vec::new()
    } else {
        match fetch_tracked_patches(&repo_root, &base_ref, &eager_entries) {
            Ok(files) => files,
            Err(err) => return DiffLoadOutcome::Error(err),
        }
    };

    let mut eager_map: HashMap<String, DiffFile> = HashMap::with_capacity(eager_files.len());
    for df in eager_files {
        eager_map.insert(df.path().to_string(), df);
    }

    let mut files = Vec::with_capacity(entries.len());
    let mut summaries = Vec::with_capacity(entries.len());
    for entry in entries {
        if should_defer_file(&entry.path, entry.additions, entry.deletions) {
            if let Some(placeholder) =
                synthesize_deferred_placeholder(&entry.path, entry.additions, entry.deletions)
            {
                summaries.push(DiffFileSummary {
                    path: entry.path,
                    old_path: entry.old_path,
                    additions: entry.additions,
                    deletions: entry.deletions,
                    binary: entry.binary,
                    untracked: false,
                    truncated: false,
                    deferred: true,
                });
                files.push(placeholder);
            }
        } else if let Some(df) = eager_map.remove(&entry.path) {
            let summary = summary_from_diff_file(&df, false, false, false);
            summaries.push(summary);
            files.push(df);
        }
    }

    if let Ok(untracked_out) = run_git(
        &repo_root,
        &[
            "-c",
            "core.quotepath=false",
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
        ],
    ) && untracked_out.status.success()
    {
        for raw in untracked_out.stdout.split(|&b| b == 0) {
            if raw.is_empty() {
                continue;
            }
            let rel = String::from_utf8_lossy(raw);
            if let Some((df, summary)) = load_untracked_entry(&repo_root, &rel) {
                summaries.push(summary);
                files.push(df);
            }
        }
    }

    let total_additions = summaries.iter().map(|s| s.additions).sum();
    let total_deletions = summaries.iter().map(|s| s.deletions).sum();

    DiffLoadOutcome::Loaded(LoadedDiff {
        repo_root,
        base_ref,
        base_branch: None,
        on_base_branch: false,
        branches: Vec::new(),
        worktrees: Vec::new(),
        files,
        summaries,
        total_additions,
        total_deletions,
    })
}

fn summary_from_diff_file(
    file: &DiffFile,
    untracked: bool,
    truncated: bool,
    deferred: bool,
) -> DiffFileSummary {
    let path = file.path().to_string();
    let old_path = file
        .original_path()
        .map(|p| p.to_string())
        .filter(|old| old != &path);
    DiffFileSummary {
        path,
        old_path,
        additions: file.additions(),
        deletions: file.deletions(),
        binary: file.is_binary(),
        untracked,
        truncated,
        deferred,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn make_temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "pa-gpui-diff-test-{label}-{}-{nanos}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = run_git(dir, args).expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test User"]);
        git(dir, &["config", "core.autocrlf", "false"]);
    }

    #[::core::prelude::v1::test]
    fn non_git_directory_returns_not_a_git_repo() {
        let dir = make_temp_dir("nongit");
        let outcome = load_uncommitted_diff(&dir);
        assert!(matches!(outcome, DiffLoadOutcome::NotAGitRepo));
        let _ = fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn deferral_detects_lockfiles_minified_and_large_line_threshold() {
        assert!(is_lock_or_generated_path("Cargo.lock"));
        assert!(is_lock_or_generated_path("web/package-lock.json"));
        assert!(is_lock_or_generated_path("pnpm-lock.yaml"));
        assert!(is_lock_or_generated_path("yarn.lock"));
        assert!(is_lock_or_generated_path("dist/app.min.js"));
        assert!(is_lock_or_generated_path("static/theme.MIN.css"));
        assert!(!is_lock_or_generated_path("src/main.rs"));

        assert!(!should_defer_file("src/main.rs", 1000, 500));
        assert!(should_defer_file("src/main.rs", 1000, 501));
        assert!(should_defer_file("Cargo.lock", 1, 0));
    }

    #[::core::prelude::v1::test]
    fn resolves_nested_cwd_and_loads_uncommitted_untracked_cap_and_binary() {
        let root = make_temp_dir("uncommitted");
        init_repo(&root);

        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn a() -> i32 { 1 }\n").unwrap();
        fs::write(root.join("icon.bin"), [0u8, 1, 2, 3, 4]).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "initial"]);

        fs::write(
            root.join("src/lib.rs"),
            "pub fn a() -> i32 { 2 }\npub fn b() -> i32 { 3 }\n",
        )
        .unwrap();
        fs::write(root.join("icon.bin"), [0u8, 9, 8, 7, 6, 5]).unwrap();
        fs::write(root.join("notes.txt"), "hello\nworld\n").unwrap();
        // ~340 KB with ~1,000 long lines (>256 KB cap, under 1,500-line deferral threshold)
        let long_line = format!("{}\n", "x".repeat(350));
        let big = long_line.repeat(1_000);
        fs::write(root.join("huge.txt"), big).unwrap();
        fs::write(root.join("blob.dat"), [0x89u8, b'P', b'N', b'G', 0x00, 0x01]).unwrap();

        let DiffLoadOutcome::Loaded(loaded) = load_uncommitted_diff(&nested) else {
            panic!("expected Loaded diff");
        };
        assert_eq!(
            fs::canonicalize(&loaded.repo_root).unwrap(),
            fs::canonicalize(&root).unwrap()
        );
        assert_eq!(loaded.files.len(), loaded.summaries.len());

        let by_path = |p: &str| {
            loaded
                .summaries
                .iter()
                .find(|s| s.path == p)
                .unwrap_or_else(|| panic!("missing summary for {p}"))
        };

        let lib = by_path("src/lib.rs");
        assert_eq!((lib.additions, lib.deletions), (2, 1));
        assert!(!lib.binary && !lib.untracked && !lib.deferred);

        let icon = by_path("icon.bin");
        assert!(icon.binary);
        assert_eq!((icon.additions, icon.deletions), (0, 0));

        let notes = by_path("notes.txt");
        assert!(notes.untracked && !notes.binary && !notes.truncated && !notes.deferred);
        assert_eq!((notes.additions, notes.deletions), (2, 0));

        let huge = by_path("huge.txt");
        assert!(huge.untracked && huge.truncated && !huge.binary);
        assert!(huge.additions > 500 && huge.additions < 1_000);

        let blob = by_path("blob.dat");
        assert!(blob.untracked && blob.binary);
        assert_eq!((blob.additions, blob.deletions), (0, 0));

        assert_eq!(
            loaded.total_additions,
            loaded.summaries.iter().map(|s| s.additions).sum::<usize>()
        );
        assert_eq!(
            loaded.total_deletions,
            loaded.summaries.iter().map(|s| s.deletions).sum::<usize>()
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn deferred_lockfile_and_large_file_skip_initial_patch_and_load_on_demand() {
        let root = make_temp_dir("deferred");
        init_repo(&root);

        fs::write(root.join("src_small.rs"), "fn a() {}\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 3\n").unwrap();
        fs::write(root.join("big.txt"), "seed\n").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "initial"]);

        fs::write(root.join("src_small.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        fs::write(root.join("Cargo.lock"), "version = 3\nname = \"demo\"\n").unwrap();
        let big_body: String = (0..1_600).map(|i| format!("line-{i}\n")).collect();
        fs::write(root.join("big.txt"), big_body).unwrap();
        fs::write(root.join("bundle.min.js"), "var a=1;\nvar b=2;\n").unwrap();

        let DiffLoadOutcome::Loaded(loaded) = load_uncommitted_diff(&root) else {
            panic!("expected Loaded diff");
        };

        let find = |p: &str| {
            let ix = loaded
                .summaries
                .iter()
                .position(|s| s.path == p)
                .unwrap_or_else(|| panic!("missing {p}"));
            (&loaded.summaries[ix], &loaded.files[ix])
        };

        let (small_s, small_f) = find("src_small.rs");
        assert!(!small_s.deferred);
        assert_eq!(small_f.additions(), 1);

        let (lock_s, lock_f) = find("Cargo.lock");
        assert!(lock_s.deferred);
        assert_eq!(lock_s.additions, 1);
        assert_eq!(lock_f.additions(), 0); // placeholder has no patch lines yet

        let (big_s, big_f) = find("big.txt");
        assert!(big_s.deferred);
        assert_eq!(big_s.additions, 1_600);
        assert_eq!(big_s.deletions, 1);
        assert_eq!(big_f.additions(), 0);

        let (min_s, min_f) = find("bundle.min.js");
        assert!(min_s.deferred && min_s.untracked);
        assert_eq!(min_s.additions, 2);
        assert_eq!(min_f.additions(), 0);

        let lock_loaded =
            load_single_file_diff(&loaded.repo_root, &loaded.base_ref, "Cargo.lock", None, false)
                .expect("on-demand tracked lockfile");
        assert_eq!(lock_loaded.additions(), 1);

        let big_loaded =
            load_single_file_diff(&loaded.repo_root, &loaded.base_ref, "big.txt", None, false)
                .expect("on-demand large file");
        assert_eq!(big_loaded.additions(), 1_600);

        let min_loaded = load_single_file_diff(
            &loaded.repo_root,
            &loaded.base_ref,
            "bundle.min.js",
            None,
            true,
        )
        .expect("on-demand untracked minified file");
        assert_eq!(min_loaded.additions(), 2);

        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn deferred_renamed_file_preserves_old_path_and_loads_on_demand() {
        let root = make_temp_dir("deferred-rename");
        init_repo(&root);
        let body: String = (0..50).map(|i| format!("row-{i}
")).collect();
        fs::write(root.join("old.min.js"), &body).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "init"]);

        git(&root, &["mv", "old.min.js", "new.min.js"]);
        fs::write(root.join("new.min.js"), format!("{body}row-50
")).unwrap();

        let DiffLoadOutcome::Loaded(loaded) =
            load_diff_for_scope(&root, DiffScope::Uncommitted, None)
        else {
            panic!("expected Loaded diff");
        };
        let summary = loaded
            .summaries
            .iter()
            .find(|s| s.path == "new.min.js")
            .expect("renamed summary");
        assert!(summary.deferred);
        assert_eq!(summary.old_path.as_deref(), Some("old.min.js"));

        let df = load_single_file_diff(
            &loaded.repo_root,
            &loaded.base_ref,
            &summary.path,
            summary.old_path.as_deref(),
            summary.untracked,
        )
        .expect("load renamed deferred diff");
        assert_eq!(df.original_path().map(|p| p.as_ref()), Some("old.min.js"));
        assert_eq!(df.additions(), 1);

        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn unborn_repo_loads_staged_and_untracked_without_head() {
        let root = make_temp_dir("unborn");
        init_repo(&root);
        fs::write(root.join("staged.txt"), "one\n").unwrap();
        git(&root, &["add", "staged.txt"]);
        fs::write(root.join("untracked.txt"), "two\n").unwrap();

        let DiffLoadOutcome::Loaded(loaded) = load_uncommitted_diff(&root) else {
            panic!("expected Loaded diff on unborn repo");
        };
        let paths: Vec<&str> = loaded.summaries.iter().map(|s| s.path.as_str()).collect();
        assert!(paths.contains(&"staged.txt"));
        assert!(paths.contains(&"untracked.txt"));
        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn base_branch_fallback_prefers_origin_head_then_main_then_master() {
        let root = make_temp_dir("base-fallback");
        init_repo(&root);
        git(&root, &["checkout", "-q", "-b", "master"]);
        fs::write(root.join("a.txt"), "base
").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "master init"]);
        assert_eq!(detect_base_branch(&root).as_deref(), Some("master"));

        git(&root, &["branch", "main"]);
        assert_eq!(detect_base_branch(&root).as_deref(), Some("main"));

        git(&root, &["branch", "develop"]);
        let dev_sha = String::from_utf8_lossy(
            &run_git(&root, &["rev-parse", "develop"]).unwrap().stdout,
        )
        .trim()
        .to_string();
        git(
            &root,
            &["update-ref", "refs/remotes/origin/develop", &dev_sha],
        );
        git(
            &root,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ],
        );
        assert_eq!(detect_base_branch(&root).as_deref(), Some("develop"));

        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn branch_diff_uses_merge_base_working_tree_and_detects_on_base() {
        let root = make_temp_dir("branch-diff");
        init_repo(&root);
        git(&root, &["checkout", "-q", "-b", "main"]);
        fs::write(root.join("shared.txt"), "v1
").unwrap();
        fs::write(root.join("main_only.txt"), "a
").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "main base"]);

        let DiffLoadOutcome::Loaded(on_main) =
            load_diff_for_scope(&root, DiffScope::Branch, None)
        else {
            panic!("expected Loaded on main");
        };
        assert_eq!(on_main.base_branch.as_deref(), Some("main"));
        assert!(on_main.on_base_branch);
        assert!(on_main.summaries.is_empty());

        git(&root, &["checkout", "-q", "-b", "feat/work"]);
        fs::write(root.join("committed.txt"), "c1
c2
").unwrap();
        git(&root, &["add", "committed.txt"]);
        git(&root, &["commit", "-q", "-m", "branch commit"]);

        // Advance main after branch point so merge-base differs from main tip.
        git(&root, &["checkout", "-q", "main"]);
        fs::write(root.join("main_only.txt"), "a
b
").unwrap();
        git(&root, &["add", "main_only.txt"]);
        git(&root, &["commit", "-q", "-m", "advance main"]);

        git(&root, &["checkout", "-q", "feat/work"]);
        fs::write(root.join("shared.txt"), "v1
v2
").unwrap();
        fs::write(root.join("scratch.txt"), "untracked
").unwrap();

        let DiffLoadOutcome::Loaded(feat_diff) =
            load_diff_for_scope(&root, DiffScope::Branch, None)
        else {
            panic!("expected Loaded on feature branch");
        };
        assert_eq!(feat_diff.base_branch.as_deref(), Some("main"));
        assert!(!feat_diff.on_base_branch);
        assert!(feat_diff.branches.contains(&"main".to_string()));
        assert!(feat_diff.branches.contains(&"feat/work".to_string()));

        let paths: Vec<&str> = feat_diff
            .summaries
            .iter()
            .map(|s| s.path.as_str())
            .collect();
        assert!(paths.contains(&"committed.txt"));
        assert!(paths.contains(&"shared.txt"));
        assert!(paths.contains(&"scratch.txt"));
        assert!(!paths.contains(&"main_only.txt"));

        let _ = fs::remove_dir_all(&root);
    }

    #[::core::prelude::v1::test]
    fn linked_worktree_enumeration_lists_multiple_worktrees() {
        let root = make_temp_dir("wt-main");
        let wt_sibling = make_temp_dir("wt-sibling");
        let _ = fs::remove_dir_all(&wt_sibling);
        init_repo(&root);
        git(&root, &["checkout", "-q", "-b", "main"]);
        fs::write(root.join("readme.txt"), "hello
").unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "init"]);

        let wt_str = wt_sibling.to_string_lossy().into_owned();
        git(&root, &["worktree", "add", "-q", "-b", "feat/side", &wt_str]);

        let worktrees = list_linked_worktrees(&root);
        assert_eq!(worktrees.len(), 2);
        let main_wt = worktrees.iter().find(|wt| wt.is_main).expect("main worktree");
        assert_eq!(main_wt.branch.as_deref(), Some("main"));
        let side_wt = worktrees.iter().find(|wt| !wt.is_main).expect("side worktree");
        assert_eq!(side_wt.branch.as_deref(), Some("feat/side"));
        assert!(same_worktree_path(&side_wt.path, &wt_sibling));

        let _ = run_git(&root, &["worktree", "remove", "--force", &wt_str]);
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&wt_sibling);
    }

    #[::core::prelude::v1::test]
    fn porcelain_parser_skips_bare_and_prunable_worktrees() {
        let raw = "worktree /repos/app
HEAD 1111111
branch refs/heads/main

worktree /repos/app-stale
HEAD 2222222
branch refs/heads/old
prunable gitdir file points to non-existent location

worktree /repos/app-live
HEAD 3333333
branch refs/heads/feat/live
";
        let entries = parse_worktree_list_porcelain(raw);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].branch.as_deref(), Some("main"));
        assert!(entries[0].is_main);
        assert_eq!(entries[1].branch.as_deref(), Some("feat/live"));
        assert!(!entries[1].is_main);
    }
}
