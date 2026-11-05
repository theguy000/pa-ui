//! Sharing a session as a gist: the daemon writes the viewer HTML, and the
//! GitHub CLI uploads it. The TUI's `/share`, with the window's own process.

use crate::Client;

/// One session shared as a gist: what the reader is given.
pub(crate) struct Shared {
    pub viewer_url: String,
}

/// The share viewer URL for a gist id: the `PI_SHARE_VIEWER_URL` override, else
/// the product default, with the id as the fragment. The TUI's `/share` builds
/// the same URL from the same names.
fn share_viewer_url(gist_id: &str) -> String {
    let base = std::env::var("PI_SHARE_VIEWER_URL")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://pi.dev/session/".to_string());
    format!("{base}#{gist_id}")
}

/// The gist id from the URL `gh gist create` prints: the last path segment.
fn gist_id_from_url(url: &str) -> Option<&str> {
    let segment = url.trim().rsplit('/').next()?;
    (!segment.is_empty()).then_some(segment)
}

/// Share a session as a gist, the way the TUI's `/share` does it: the daemon
/// writes the session's viewer HTML, and the GitHub CLI uploads it. The window
/// holds no token of its own; if `gh` is missing or logged out, its own message
/// comes back as the failure.
///
/// `export_html` reaches the daemon through the worker holding the session, so
/// a session nothing is running cannot be exported. The menu says that rather
/// than sending a command that would start a worker to answer it.
pub(crate) fn share_as_gist(client: &Client, session: &str) -> Result<Shared, String> {
    let pid = std::process::id();
    let file = std::env::temp_dir().join(format!("pa-ui-share-{pid}-{session}.html"));
    let written = pa_client::export_html(client, session, &file.to_string_lossy())
        .map_err(|error| format!("could not export the session: {error}"))?;
    // the path the daemon reports, and the temp path this window removes
    let path = std::path::PathBuf::from(written);

    // The TUI gates on `gh auth status` first, so a logged-out CLI says so
    // rather than failing inside the upload.
    let mut probe = gh_command();
    let logged_in = probe
        .args(["auth", "status"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !logged_in {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&file);
        return Err("the GitHub CLI is not logged in: run `gh auth login`".to_string());
    }

    // `gh gist create` names the gist file after the path it is handed. The
    // viewer at PI_SHARE_VIEWER_URL looks for `session.html` when the fragment
    // names only the gist, so the upload is read from stdin under that name; the
    // temp path stays unique so two shares cannot collide.
    let input = std::fs::File::open(&path).map_err(|error| {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&file);
        format!("could not read the exported session: {error}")
    })?;
    let output = gh_command()
        .args([
            "gist",
            "create",
            "--public=false",
            "--filename",
            "session.html",
            "-",
        ])
        .stdin(std::process::Stdio::from(input))
        .output()
        .map_err(|error| {
            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(&file);
            format!("could not run the GitHub CLI (gh): {error}")
        });
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&file);
    let output = output?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        return Err(if message.is_empty() {
            "the GitHub CLI refused the gist".to_string()
        } else {
            message.to_string()
        });
    }
    let gist_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let gist_id = gist_id_from_url(&gist_url)
        .ok_or_else(|| "could not read the gist's id from gh's output".to_string())?;
    Ok(Shared {
        viewer_url: share_viewer_url(gist_id),
    })
}

/// The unauthenticated GitHub API quota the share viewer spends. The viewer
/// fetches the gist from `api.github.com` in the reader's browser, so the limit
/// that matters is the public one (60 requests an hour per IP), not the token
/// quota `gh` uses. `/rate_limit` itself is free to ask.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ViewerQuota {
    pub remaining: i64,
    pub reset: i64,
}

impl ViewerQuota {
    /// How long until the core bucket refills, rounded up to whole minutes.
    pub(crate) fn wait_minutes(self) -> i64 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs() as i64);
        ((self.reset - now).max(0) + 59) / 60
    }
}

/// Read the core quota out of GitHub's `/rate_limit` answer.
fn parse_viewer_quota(body: &str) -> Option<ViewerQuota> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let core = value.pointer("/resources/core")?;
    Some(ViewerQuota {
        remaining: core.get("remaining")?.as_i64()?,
        reset: core.get("reset")?.as_i64()?,
    })
}

/// Ask GitHub for the unauthenticated quota the viewer will spend. The request
/// is deliberately made with `curl` and no token: `gh` would report the token's
/// 5,000-an-hour budget, which is not the budget the browser gets.
pub(crate) fn viewer_quota() -> Result<ViewerQuota, String> {
    let output = quota_command()
        .args(["-sS", "--max-time", "5", "https://api.github.com/rate_limit"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|error| format!("could not check GitHub's API quota: {error}"))?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        let message = message.trim();
        return Err(if message.is_empty() {
            "could not check GitHub's API quota".to_string()
        } else {
            format!("could not check GitHub's API quota: {message}")
        });
    }
    let body = String::from_utf8_lossy(&output.stdout);
    parse_viewer_quota(&body).ok_or_else(|| "could not read GitHub's API quota".to_string())
}

#[cfg(windows)]
fn quota_command() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("curl");
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(not(windows))]
fn quota_command() -> std::process::Command {
    std::process::Command::new("curl")
}

/// Open a share link in the reader's browser. There is no toolkit hook for this,
/// so the platform's own opener is used: `rundll32` on Windows (a GUI process,
/// so no console flashes over the window), `open` on macOS, and `xdg-open` on
/// the other Unixes. The URL comes from `share_viewer_url`, not from anything a
/// session wrote, so it is safe to hand to the opener as one argument.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    let mut command = open_command(url);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not open the link: {error}"))
}

#[cfg(windows)]
fn open_command(url: &str) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("rundll32");
    command.args(["url.dll,FileProtocolHandler", url]);
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(target_os = "macos")]
fn open_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("open");
    command.arg(url);
    command
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command
}

/// The `gh` command, hidden on Windows so neither the auth probe nor the upload
/// flashes a console over the window. The same `CREATE_NO_WINDOW` flag the
/// window uses for the icon build's own child process.
#[cfg(windows)]
fn gh_command() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("gh");
    command.creation_flags(0x0800_0000);
    command
}

#[cfg(not(windows))]
fn gh_command() -> std::process::Command {
    std::process::Command::new("gh")
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_quota_is_read_from_the_core_bucket() {
        let quota = parse_viewer_quota(
            r#"{"resources":{"core":{"limit":60,"remaining":0,"reset":1793863127}}}"#,
        )
        .expect("a core bucket");
        assert_eq!(quota.remaining, 0);
        assert_eq!(quota.reset, 1_793_863_127);
    }

    #[test]
    fn a_body_without_a_core_bucket_is_not_a_quota() {
        assert!(parse_viewer_quota(r#"{"resources":{"search":{"limit":10}}}"#).is_none());
        assert!(parse_viewer_quota("not json").is_none());
    }
}
