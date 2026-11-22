//! Synchronous Git diff loader meant to run on background threads.

use gpui_kit::component::diff::{DiffFile, DiffParseOptions};
use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

pub(crate) const UNTRACKED_MAX_BYTES: u64 = 256 * 1024;
const EMPTY_TREE_OID: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffFileSummary {
    pub(crate) path: String,
    pub(crate) old_path: Option<String>,
    pub(crate) additions: usize,
    pub(crate) deletions: usize,
    pub(crate) binary: bool,
    pub(crate) untracked: bool,
    pub(crate) truncated: bool,
}

pub(crate) struct LoadedDiff {
    pub(crate) repo_root: PathBuf,
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

fn parse_patch_resilient(patch: &str) -> Vec<DiffFile> {
    if patch.trim().is_empty() {
        return Vec::new();
    }
    let opts = DiffParseOptions::new().truncated_hunks(true);
    if let Ok(files) = DiffFile::parse_with(patch, opts) {
        return files;
    }
    // Fall back to per-file parsing if any single file block has an unsupported header/hunk.
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

fn build_untracked_diff_file(repo_root: &Path, rel_path: &str) -> Option<(DiffFile, bool)> {
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

    let git_path = rel_path.replace('\\', "/");
    let patch = if is_probably_binary(&buf) {
        format!(
            "diff --git a/{p} b/{p}\nnew file mode 100644\nBinary files /dev/null and b/{p} differ\n",
            p = git_path
        )
    } else {
        // Trim any partial UTF-8 sequence caused by the byte cap.
        let text = match std::str::from_utf8(&buf) {
            Ok(s) => s.to_string(),
            Err(err) if truncated => {
                String::from_utf8_lossy(&buf[..err.valid_up_to()]).into_owned()
            }
            Err(_) => String::from_utf8_lossy(&buf).into_owned(),
        };
        synthesize_added_file_patch(&git_path, &text)
    };

    let opts = DiffParseOptions::new().truncated_hunks(true);
    let parsed = DiffFile::parse_with(&patch, opts).ok()?.into_iter().next()?;
    Some((parsed, truncated))
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

pub(crate) fn load_uncommitted_diff(cwd: &Path) -> DiffLoadOutcome {
    let Some(repo_root) = resolve_worktree_root(cwd) else {
        return DiffLoadOutcome::NotAGitRepo;
    };

    let base_ref = if has_head_commit(&repo_root) {
        "HEAD"
    } else {
        EMPTY_TREE_OID
    };

    let diff_out = match run_git(
        &repo_root,
        &[
            "-c",
            "core.quotepath=false",
            "diff",
            base_ref,
            "--no-ext-diff",
            "--no-color",
            "--unified=3",
        ],
    ) {
        Ok(out) => out,
        Err(err) => return DiffLoadOutcome::Error(format!("Failed to run git diff: {err}")),
    };
    if !diff_out.status.success() {
        let stderr = String::from_utf8_lossy(&diff_out.stderr).trim().to_string();
        return DiffLoadOutcome::Error(if stderr.is_empty() {
            "git diff failed".into()
        } else {
            stderr
        });
    }

    let tracked_patch = String::from_utf8_lossy(&diff_out.stdout);
    let mut files = parse_patch_resilient(&tracked_patch);
    let mut summaries = Vec::with_capacity(files.len());
    for file in &files {
        summaries.push(summary_from_diff_file(file, false, false));
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
    ) {
        if untracked_out.status.success() {
            for raw in untracked_out.stdout.split(|&b| b == 0) {
                if raw.is_empty() {
                    continue;
                }
                let rel = String::from_utf8_lossy(raw);
                if let Some((df, truncated)) = build_untracked_diff_file(&repo_root, &rel) {
                    summaries.push(summary_from_diff_file(&df, true, truncated));
                    files.push(df);
                }
            }
        }
    }

    let total_additions = summaries.iter().map(|s| s.additions).sum();
    let total_deletions = summaries.iter().map(|s| s.deletions).sum();

    DiffLoadOutcome::Loaded(LoadedDiff {
        repo_root,
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
    fn resolves_nested_cwd_and_loads_uncommitted_untracked_cap_and_binary() {
        let root = make_temp_dir("uncommitted");
        init_repo(&root);

        let nested = root.join("src/nested");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn a() -> i32 { 1 }\n").unwrap();
        fs::write(root.join("icon.bin"), [0u8, 1, 2, 3, 4]).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-q", "-m", "initial"]);

        // Modify tracked text + tracked binary + add untracked text, large (>256 KB) untracked, untracked binary.
        fs::write(
            root.join("src/lib.rs"),
            "pub fn a() -> i32 { 2 }\npub fn b() -> i32 { 3 }\n",
        )
        .unwrap();
        fs::write(root.join("icon.bin"), [0u8, 9, 8, 7, 6, 5]).unwrap();
        fs::write(root.join("notes.txt"), "hello\nworld\n").unwrap();
        let big = "0123456789abcdef\n".repeat(20_000); // ~340 KB > 256 KB
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
        assert!(!lib.binary && !lib.untracked);

        let icon = by_path("icon.bin");
        assert!(icon.binary);
        assert_eq!((icon.additions, icon.deletions), (0, 0));

        let notes = by_path("notes.txt");
        assert!(notes.untracked && !notes.binary && !notes.truncated);
        assert_eq!((notes.additions, notes.deletions), (2, 0));

        let huge = by_path("huge.txt");
        assert!(huge.untracked && huge.truncated && !huge.binary);
        assert!(huge.additions > 1000 && huge.additions < 20_000);

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
}
