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
        let _ = std::fs::remove_file(&file);
        return Err("the GitHub CLI is not logged in: run `gh auth login`".to_string());
    }

    let output = gh_command()
        .args(["gist", "create", "--public=false"])
        .arg(&path)
        .stdin(std::process::Stdio::null())
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
