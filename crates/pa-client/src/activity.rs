//! Session activity & roster lineage module: subagent descendants, scoped heartbeats,
//! running kernel shells, and live stream activity labels.

use crate::Client;
use pa_types::daemon::KERNEL_NOT_RUNNING_MESSAGE;
use serde_json::{json, Value};
use std::time::Duration;

/// `roster_subscribe` → raw roster entries.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn list_roster(client: &Client) -> std::io::Result<Vec<Value>> {
    let response = client.request("roster_subscribe", json!({}), Duration::from_secs(3))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "roster_subscribe failed: {}",
            response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )));
    }
    let roster = response
        .pointer("/data/roster")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| std::io::Error::other("roster_subscribe returned no roster"))?;
    Ok(roster)
}

fn roster_summary(entry: &Value) -> Option<&Value> {
    entry.get("summary").filter(|summary| summary.is_object())
}

fn roster_key(prefix: &str, value: Option<&str>) -> Option<String> {
    value
        .filter(|value| !value.is_empty())
        .map(|value| format!("{prefix}:{value}"))
}

fn roster_keys(summary: &Value, parent: bool) -> Vec<String> {
    let fields = if parent {
        [
            ("active", "parentActiveSessionId"),
            ("session", "parentSessionId"),
        ]
    } else {
        [("active", "activeSessionId"), ("session", "sessionId")]
    };
    fields
        .into_iter()
        .filter_map(|(prefix, field)| {
            roster_key(prefix, summary.get(field).and_then(Value::as_str))
        })
        .collect()
}

fn roster_identity_keys(summary: &Value) -> Vec<String> {
    roster_keys(summary, false)
}

fn roster_parent_keys(summary: &Value) -> Vec<String> {
    roster_keys(summary, true)
}

fn normalize_session_path(path: &str) -> String {
    path.replace('/', "\\").to_lowercase()
}

/// The number of running subagent descendants of one Session in the raw roster.
#[must_use]
pub fn running_subagents(
    roster: &[Value],
    active_session_id: &str,
    session_id: &str,
) -> usize {
    subagent_counts(roster, active_session_id, session_id).running
}

/// How many heartbeats belong to one Session.
#[must_use]
pub fn heartbeat_count(data: &Value, active_session_id: &str, session_id: &str) -> usize {
    if session_id.is_empty() {
        return 0;
    }
    data.get("heartbeats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("job"))
        .filter(|job| {
            let job_session = job
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let job_active = job
                .get("activeSessionId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            job_session == session_id
                || (!active_session_id.is_empty() && job_active == active_session_id)
        })
        .count()
}

/// Count only the shell commands still running.
#[must_use]
pub fn running_shell_count(data: &Value) -> usize {
    data.get("activities")
        .or(Some(data))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|row| {
            row.get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| !id.is_empty())
                && row.get("status").and_then(Value::as_str) == Some("running")
        })
        .count()
}

/// `heartbeats_list` → the number of scoped heartbeats for one Session.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn scoped_heartbeat_count(
    client: &Client,
    active_session_id: &str,
    session_id: &str,
) -> std::io::Result<usize> {
    let response = client.request("heartbeats_list", json!({}), Duration::from_secs(3))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "heartbeats_list failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let data = response.get("data").unwrap_or(&Value::Null);
    if data.get("heartbeats").and_then(Value::as_array).is_none() {
        return Err(std::io::Error::other(
            "heartbeats_list returned no heartbeats array",
        ));
    }
    Ok(heartbeat_count(data, active_session_id, session_id))
}

/// `list_kernel_bash` → the number of running shell commands.
///
/// # Errors
/// Fails on transport errors or a non-success response other than the
/// kernel-not-running refusal.
pub fn running_shells(client: &Client, active_session_id: &str) -> std::io::Result<usize> {
    match client.request(
        "list_kernel_bash",
        json!({"activeSessionId": active_session_id}),
        Duration::from_secs(3),
    ) {
        Ok(response) => {
            let message = response
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if response.get("success").and_then(Value::as_bool) != Some(true) {
                if message == KERNEL_NOT_RUNNING_MESSAGE {
                    return Ok(0);
                }
                return Err(std::io::Error::other(format!(
                    "list_kernel_bash failed: {message}"
                )));
            }
            let data = response.get("data").unwrap_or(&Value::Null);
            if data.get("activities").and_then(Value::as_array).is_none() {
                return Err(std::io::Error::other(
                    "list_kernel_bash returned no activities array",
                ));
            }
            Ok(running_shell_count(data))
        }
        Err(error) => Err(std::io::Error::other(error.to_string())),
    }
}

#[must_use]
pub fn activity_label(event: &Value) -> Option<&'static str> {
    match event.get("type").and_then(Value::as_str)? {
        "thinking_start" | "thinking_delta" => Some("Thinking"),
        "text_start" | "text_delta" => Some("Writing"),
        "toolcall_start" | "toolcall_delta" => Some("Writing code"),
        "tool_result" | "toolcall_end" => Some("Waiting"),
        "message_start" => Some("Thinking"),
        "message_end" | "stop" => Some("Waiting"),
        _ => None,
    }
}

/// Activity chip metric: gold running count when `running > 0`, otherwise
/// muted total count for the Session (`0` only when none exist).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ChipCount {
    pub running: usize,
    pub total: usize,
}

impl ChipCount {
    #[must_use]
    pub const fn new(running: usize, total: usize) -> Self {
        Self { running, total }
    }

    /// Returns `(displayed_number, is_live_gold)`.
    #[must_use]
    pub const fn display_value(self) -> (usize, bool) {
        if self.running > 0 {
            (self.running, true)
        } else {
            (self.total, false)
        }
    }
}

/// One Session-scoped heartbeat row parsed from `heartbeats_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeartbeatItem {
    pub id: String,
    pub active_session_id: String,
    pub session_id: String,
    pub status: String,
    pub source: String,
    pub delivery_mode: String,
    pub schedule: String,
    pub label: Option<String>,
    pub prompt: String,
    pub next_run_at: Option<String>,
    pub run_count: u64,
    pub last_error: Option<String>,
}

impl HeartbeatItem {
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.status == "active"
    }

    #[must_use]
    pub fn is_user_heartbeat(&self) -> bool {
        self.source != "rlm_heartbeat"
    }

    #[must_use]
    pub fn delivery_label(&self) -> &'static str {
        if self.delivery_mode == "follow_up" {
            "follow-up"
        } else {
            "steer"
        }
    }
}

/// One Session-scoped kernel bash activity row parsed from `list_kernel_bash`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellActivityItem {
    pub id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub status: String,
    pub exit_code: Option<i64>,
    pub duration_ms: Option<u64>,
}

impl ShellActivityItem {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }
}

/// Running and total subagent descendants of one Session in the raw roster.
#[must_use]
pub fn subagent_counts(
    roster: &[Value],
    active_session_id: &str,
    session_id: &str,
) -> ChipCount {
    let mut parents: Vec<(String, u64, Option<String>)> = [
        roster_key("active", Some(active_session_id)),
        roster_key("session", Some(session_id)),
    ]
    .into_iter()
    .flatten()
    .map(|key| (key, 0, None))
    .collect();
    if parents.is_empty() {
        return ChipCount::default();
    }
    let root_ids: std::collections::HashSet<String> =
        parents.iter().map(|(key, _, _)| key.clone()).collect();
    if let Some(root) = roster.iter().find(|entry| {
        roster_summary(entry).is_some_and(|summary| {
            roster_identity_keys(summary)
                .iter()
                .any(|key| root_ids.contains(key))
        })
    }) {
        if let Some(summary) = roster_summary(root) {
            let depth = summary.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
            let file = summary
                .get("sessionFile")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(normalize_session_path);
            for parent in &mut parents {
                parent.1 = depth;
                parent.2.clone_from(&file);
            }
        }
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut cursor = 0;
    let mut running = 0;
    let mut total = 0;
    while cursor < parents.len() {
        let (parent_key, parent_depth, parent_file) = parents[cursor].clone();
        cursor += 1;
        for entry in roster {
            let Some(summary) = roster_summary(entry) else {
                continue;
            };
            let depth = summary.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
            let subagent = summary
                .get("runtimeKind")
                .and_then(Value::as_str)
                .map_or(depth > 0, |kind| kind == "subagent");
            if !subagent {
                continue;
            }
            let identity = roster_identity_keys(summary);
            if identity.iter().any(|key| root_ids.contains(key)) {
                continue;
            }
            let matches_id = roster_parent_keys(summary)
                .iter()
                .any(|key| key == &parent_key);
            let matches_path = parent_file.as_ref().is_some_and(|file| {
                depth == parent_depth.saturating_add(1)
                    && summary
                        .get("parentSessionPath")
                        .and_then(Value::as_str)
                        .is_some_and(|path| normalize_session_path(path) == *file)
            });
            if !matches_id && !matches_path {
                continue;
            }
            let key = entry
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .or_else(|| identity.first().cloned())
                .unwrap_or_else(|| format!("{parent_key}:{}", seen.len()));
            if !seen.insert(key) {
                continue;
            }
            total += 1;
            if entry.get("status").and_then(Value::as_str) == Some("running") {
                running += 1;
            }
            let file = summary
                .get("sessionFile")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(normalize_session_path);
            for child_key in identity {
                parents.push((child_key, depth, file.clone()));
            }
        }
    }
    ChipCount::new(running, total)
}

/// Parse Session-scoped heartbeats from a `heartbeats_list` payload.
/// Active heartbeats sort before paused heartbeats.
#[must_use]
pub fn parse_scoped_heartbeats(
    data: &Value,
    active_session_id: &str,
    session_id: &str,
) -> Vec<HeartbeatItem> {
    if session_id.is_empty() && active_session_id.is_empty() {
        return Vec::new();
    }
    let mut items: Vec<HeartbeatItem> = data
        .get("heartbeats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let job = row.get("job")?;
            let id = job.get("id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let job_session = job
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let job_active = job
                .get("activeSessionId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let matches = (!session_id.is_empty() && job_session == session_id)
                || (!active_session_id.is_empty() && job_active == active_session_id);
            if !matches {
                return None;
            }
            let status = job
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("active")
                .to_string();
            let source = job
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("heartbeat")
                .to_string();
            let delivery_mode = job
                .get("deliveryMode")
                .and_then(Value::as_str)
                .unwrap_or("steer")
                .to_string();
            let schedule = job
                .pointer("/schedule/expression")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("every 5m")
                .to_string();
            let label = job
                .get("label")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let prompt = job
                .get("prompt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let next_run_at = job
                .get("nextRunAt")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let run_count = job.get("runCount").and_then(Value::as_u64).unwrap_or(0);
            let last_error = job
                .get("lastError")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Some(HeartbeatItem {
                id: id.to_string(),
                active_session_id: job_active.to_string(),
                session_id: job_session.to_string(),
                status,
                source,
                delivery_mode,
                schedule,
                label,
                prompt,
                next_run_at,
                run_count,
                last_error,
            })
        })
        .collect();
    items.sort_by_key(|item| (!item.is_active(), !item.is_user_heartbeat()));
    items
}

#[must_use]
pub fn heartbeat_counts(items: &[HeartbeatItem]) -> ChipCount {
    let running = items.iter().filter(|h| h.is_active()).count();
    ChipCount::new(running, items.len())
}

/// Parse kernel bash activities from `list_kernel_bash`, sorting running shells
/// first while preserving order within each group.
#[must_use]
pub fn parse_shell_activities(data: &Value) -> Vec<ShellActivityItem> {
    let rows = data.get("activities").unwrap_or(data);
    let mut items: Vec<ShellActivityItem> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let id = row.get("id")?.as_str()?.trim();
            if id.is_empty() {
                return None;
            }
            let command = row
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .replace(['\r', '\n'], " ")
                .trim()
                .to_string();
            let pid = row
                .get("pid")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok());
            let started_at = row
                .get("startedAt")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let status = row
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("finished")
                .to_string();
            let exit_code = row.get("exitCode").and_then(Value::as_i64);
            let duration_ms = row.get("durationMs").and_then(Value::as_u64);
            Some(ShellActivityItem {
                id: id.to_string(),
                command,
                pid,
                started_at,
                status,
                exit_code,
                duration_ms,
            })
        })
        .collect();
    items.sort_by_key(|item| !item.is_running());
    items
}

#[must_use]
pub fn shell_counts(items: &[ShellActivityItem]) -> ChipCount {
    let running = items.iter().filter(|s| s.is_running()).count();
    ChipCount::new(running, items.len())
}

/// Fetch all heartbeats scoped strictly to the selected Session.
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn list_scoped_heartbeats(
    client: &Client,
    active_session_id: &str,
    session_id: &str,
) -> std::io::Result<Vec<HeartbeatItem>> {
    let response = client.request("heartbeats_list", json!({}), Duration::from_secs(3))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "heartbeats_list failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let data = response.get("data").unwrap_or(&Value::Null);
    Ok(parse_scoped_heartbeats(data, active_session_id, session_id))
}

/// Create or update the Session's user heartbeat (`heartbeat_set`).
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn set_heartbeat(
    client: &Client,
    active_session_id: &str,
    schedule: &str,
    prompt: &str,
    delivery_mode: &str,
) -> std::io::Result<()> {
    let response = client.request(
        "heartbeat_set",
        json!({
            "activeSessionId": active_session_id,
            "schedule": schedule,
            "prompt": prompt,
            "deliveryMode": delivery_mode,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "heartbeat_set failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(())
}

/// Pause, resume, or stop a heartbeat by `job_id` (`heartbeat_manage`).
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn manage_heartbeat(
    client: &Client,
    active_session_id: &str,
    job_id: &str,
    action: &str,
) -> std::io::Result<()> {
    let response = client.request(
        "heartbeat_manage",
        json!({
            "activeSessionId": active_session_id,
            "jobId": job_id,
            "action": action,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "heartbeat_manage failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(())
}

/// Fetch kernel bash activities for one Session (`list_kernel_bash`).
/// A session without a running kernel returns an empty list cleanly.
///
/// # Errors
/// Fails on transport or unexpected daemon command error.
pub fn list_kernel_shells(
    client: &Client,
    active_session_id: &str,
) -> std::io::Result<Vec<ShellActivityItem>> {
    let response = client.request(
        "list_kernel_bash",
        json!({"activeSessionId": active_session_id}),
        Duration::from_secs(3),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        let msg = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if msg == KERNEL_NOT_RUNNING_MESSAGE {
            return Ok(Vec::new());
        }
        return Err(std::io::Error::other(format!(
            "list_kernel_bash failed: {msg}"
        )));
    }
    let data = response.get("data").unwrap_or(&Value::Null);
    Ok(parse_shell_activities(data))
}

/// Fetch the tail output of one kernel bash command (`tail_kernel_bash`).
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn tail_kernel_shell(
    client: &Client,
    active_session_id: &str,
    activity_id: &str,
    lines: u32,
) -> std::io::Result<String> {
    let clamped = lines.clamp(1, 200);
    let response = client.request(
        "tail_kernel_bash",
        json!({
            "activeSessionId": active_session_id,
            "activityId": activity_id,
            "lines": clamped,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "tail_kernel_bash failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let tail = response
        .pointer("/data/tail")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(tail)
}

/// Kill a running kernel bash command (`kill_kernel_bash`).
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn kill_kernel_shell(
    client: &Client,
    active_session_id: &str,
    activity_id: &str,
) -> std::io::Result<()> {
    let response = client.request(
        "kill_kernel_bash",
        json!({
            "activeSessionId": active_session_id,
            "activityId": activity_id,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "kill_kernel_bash failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(())
}


/// One Session-scoped subagent entry (direct or nested descendant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentItem {
    /// Stable identifier for UI selection (`child_id` if present, else `active_session_id`).
    pub id: String,
    /// Parent-scoped RLM child id (required for `cancel_rlm_child` / `delete_rlm_subagent`).
    pub child_id: String,
    pub active_session_id: String,
    pub parent_active_session_id: String,
    pub name: String,
    pub status: String,
    /// Nesting depth relative to the selected Session (`0` = direct child, `1+` = nested).
    pub depth: usize,
    pub model: String,
    pub activity: String,
    pub duration_ms: Option<u64>,
    pub answer_preview: Option<String>,
}

impl SubagentItem {
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.status == "running"
    }

    #[must_use]
    pub fn can_manage(&self) -> bool {
        !self.child_id.is_empty() && !self.parent_active_session_id.is_empty()
    }
}

/// One compact read-only message block rendered in a Subagent preview or Right sidebar tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubagentMessagePreview {
    pub role: String,
    pub text: String,
}

#[must_use]
pub fn subagent_items_counts(items: &[SubagentItem]) -> ChipCount {
    let running = items.iter().filter(|s| s.is_running()).count();
    ChipCount::new(running, items.len())
}

/// Build an ordered descendant tree of subagents belonging to the selected Session,
/// merging `roster_subscribe` entries (for direct + nested lineage) with `get_rlm_children`
/// metadata (for `rlmChildId`, `progressNote`, `durationMs`, and recently settled direct children).
#[must_use]
pub fn parse_session_subagents(
    roster: &[Value],
    children: &[Value],
    active_session_id: &str,
    session_id: &str,
) -> Vec<SubagentItem> {
    let mut root_keys: Vec<(String, u64, Option<String>)> = [
        roster_key("active", Some(active_session_id)),
        roster_key("session", Some(session_id)),
    ]
    .into_iter()
    .flatten()
    .map(|key| (key, 0, None))
    .collect();
    if root_keys.is_empty() && children.is_empty() {
        return Vec::new();
    }
    let root_ids: std::collections::HashSet<String> =
        root_keys.iter().map(|(key, _, _)| key.clone()).collect();
    if let Some(root) = roster.iter().find(|entry| {
        roster_summary(entry).is_some_and(|summary| {
            roster_identity_keys(summary)
                .iter()
                .any(|key| root_ids.contains(key))
        })
    }) {
        if let Some(summary) = roster_summary(root) {
            let depth = summary.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
            let file = summary
                .get("sessionFile")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(normalize_session_path);
            for parent in &mut root_keys {
                parent.1 = depth;
                parent.2.clone_from(&file);
            }
        }
    }

    // Index direct `get_rlm_children` rows by activeSessionId and sessionId for fast enrichment.
    let mut child_by_active: std::collections::HashMap<String, &Value> =
        std::collections::HashMap::new();
    let mut child_by_session: std::collections::HashMap<String, &Value> =
        std::collections::HashMap::new();
    for ch in children {
        if let Some(a) = ch
            .get("activeSessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            child_by_active.insert(a.to_string(), ch);
        }
        if let Some(s) = ch
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            child_by_session.insert(s.to_string(), ch);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_children(
        roster: &[Value],
        child_by_active: &std::collections::HashMap<String, &Value>,
        child_by_session: &std::collections::HashMap<String, &Value>,
        root_ids: &std::collections::HashSet<String>,
        parent_keys: &[(String, u64, Option<String>)],
        parent_active_id: &str,
        rel_depth: usize,
        seen: &mut std::collections::HashSet<String>,
        out: &mut Vec<SubagentItem>,
    ) {
        type MatchedRosterEntry<'a> = (&'a Value, &'a Value, Vec<String>, u64, Option<String>);
        let mut matched_entries: Vec<MatchedRosterEntry<'_>> = Vec::new();
        for entry in roster {
            let Some(summary) = roster_summary(entry) else {
                continue;
            };
            let depth = summary.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
            let is_sub = summary
                .get("runtimeKind")
                .and_then(Value::as_str)
                .map_or(depth > 0, |kind| kind == "subagent");
            if !is_sub {
                continue;
            }
            let identity = roster_identity_keys(summary);
            if identity.iter().any(|key| root_ids.contains(key)) {
                continue;
            }
            let matches_parent = parent_keys.iter().any(|(pkey, pdepth, pfile)| {
                let by_id = roster_parent_keys(summary).iter().any(|k| k == pkey);
                let by_path = pfile.as_ref().is_some_and(|file| {
                    depth == pdepth.saturating_add(1)
                        && summary
                            .get("parentSessionPath")
                            .and_then(Value::as_str)
                            .is_some_and(|path| normalize_session_path(path) == *file)
                });
                by_id || by_path
            });
            if !matches_parent {
                continue;
            }
            let dedupe_key = entry
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or_else(|| identity.first().cloned())
                .unwrap_or_else(|| format!("sub-{rel_depth}-{}", seen.len()));
            if !seen.insert(dedupe_key) {
                continue;
            }
            let file = summary
                .get("sessionFile")
                .and_then(Value::as_str)
                .filter(|path| !path.is_empty())
                .map(normalize_session_path);
            matched_entries.push((entry, summary, identity, depth, file));
        }

        // Running siblings first, preserving stable order within each group.
        matched_entries.sort_by_key(|(entry, _, _, _, _)| {
            entry.get("status").and_then(Value::as_str) != Some("running")
        });

        for (entry, summary, identity, abs_depth, file) in matched_entries {
            let active_id = summary
                .get("activeSessionId")
                .and_then(Value::as_str)
                .or_else(|| entry.get("agentId").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            let durable_id = summary
                .get("sessionId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let enriched = if rel_depth == 0 {
                child_by_active
                    .get(&active_id)
                    .or_else(|| child_by_session.get(durable_id))
                    .copied()
            } else {
                None
            };

            let child_id = enriched
                .and_then(|c| {
                    c.get("id")
                        .or_else(|| c.get("rlmChildId"))
                        .and_then(Value::as_str)
                })
                .or_else(|| summary.get("rlmChildId").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            let name = enriched
                .and_then(|c| c.get("sessionName").and_then(Value::as_str))
                .or_else(|| summary.get("sessionName").and_then(Value::as_str))
                .or_else(|| summary.get("name").and_then(Value::as_str))
                .filter(|s| !s.trim().is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    if !child_id.is_empty() {
                        child_id.clone()
                    } else {
                        active_id.chars().take(12).collect()
                    }
                });
            let status = entry
                .get("status")
                .and_then(Value::as_str)
                .or_else(|| enriched.and_then(|c| c.get("status").and_then(Value::as_str)))
                .unwrap_or("idle")
                .to_string();
            let model = summary
                .get("modelId")
                .or_else(|| summary.get("model"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let progress_note = enriched
                .and_then(|c| c.get("progressNote").and_then(Value::as_str))
                .or_else(|| summary.get("progressNote").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let answer_preview = enriched
                .and_then(|c| c.get("answerPreview").and_then(Value::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let activity = progress_note
                .map(str::to_string)
                .or_else(|| answer_preview.clone())
                .or_else(|| {
                    summary
                        .get("activity")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| status.clone());
            let duration_ms = enriched
                .and_then(|c| c.get("durationMs").and_then(Value::as_u64))
                .or_else(|| summary.get("durationMs").and_then(Value::as_u64));
            let stable_id = if !child_id.is_empty() {
                child_id.clone()
            } else {
                active_id.clone()
            };

            out.push(SubagentItem {
                id: stable_id,
                child_id,
                active_session_id: active_id.clone(),
                parent_active_session_id: parent_active_id.to_string(),
                name,
                status,
                depth: rel_depth,
                model,
                activity,
                duration_ms,
                answer_preview,
            });

            let next_parents: Vec<(String, u64, Option<String>)> = identity
                .into_iter()
                .map(|k| (k, abs_depth, file.clone()))
                .collect();
            if !next_parents.is_empty() {
                collect_children(
                    roster,
                    child_by_active,
                    child_by_session,
                    root_ids,
                    &next_parents,
                    &active_id,
                    rel_depth + 1,
                    seen,
                    out,
                );
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    if !root_keys.is_empty() {
        collect_children(
            roster,
            &child_by_active,
            &child_by_session,
            &root_ids,
            &root_keys,
            active_session_id,
            0,
            &mut seen,
            &mut out,
        );
    }

    // Append any direct `get_rlm_children` entries not present in the live roster
    // (e.g., recently completed or cancelled direct children).
    let mut extra_direct: Vec<SubagentItem> = Vec::new();
    for ch in children {
        let child_id = ch
            .get("id")
            .or_else(|| ch.get("rlmChildId"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let active_id = ch
            .get("activeSessionId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if child_id.is_empty() && active_id.is_empty() {
            continue;
        }
        let already = out.iter().any(|existing| {
            (!child_id.is_empty() && existing.child_id == child_id)
                || (!active_id.is_empty() && existing.active_session_id == active_id)
        });
        if already {
            continue;
        }
        let name = ch
            .get("sessionName")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                if !child_id.is_empty() {
                    child_id.clone()
                } else {
                    active_id.chars().take(12).collect()
                }
            });
        let status = ch
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("completed")
            .to_string();
        let answer_preview = ch
            .get("answerPreview")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let model = ch
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let error_msg = ch
            .get("error")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let activity = ch
            .get("progressNote")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| answer_preview.clone())
            .or(error_msg)
            .unwrap_or_else(|| status.clone());
        let duration_ms = ch.get("durationMs").and_then(Value::as_u64);
        let stable_id = if !child_id.is_empty() {
            child_id.clone()
        } else {
            active_id.clone()
        };
        extra_direct.push(SubagentItem {
            id: stable_id,
            child_id,
            active_session_id: active_id,
            parent_active_session_id: active_session_id.to_string(),
            name,
            status,
            depth: 0,
            model,
            activity,
            duration_ms,
            answer_preview,
        });
    }
    extra_direct.sort_by_key(|s| !s.is_running());
    out.extend(extra_direct);

    out
}

/// Extract readable read-only message blocks from raw `get_messages` values.
#[must_use]
pub fn extract_subagent_messages(messages: &[Value]) -> Vec<SubagentMessagePreview> {
    let mut out = Vec::new();
    for raw in messages {
        let msg = raw.get("message").unwrap_or(raw);
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if role.is_empty() || role == "toolResult" {
            continue;
        }
        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = msg.get("content").and_then(Value::as_str) {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        } else if let Some(arr) = msg.get("content").and_then(Value::as_array) {
            for block in arr {
                match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str) {
                            let trimmed = t.trim();
                            if !trimmed.is_empty() {
                                parts.push(trimmed.to_string());
                            }
                        }
                    }
                    Some("toolCall") => {
                        let tool_name = block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("tool");
                        parts.push(format!("[{tool_name}]"));
                    }
                    _ => {}
                }
            }
        }
        if !parts.is_empty() {
            let label = if role == "user" { "task" } else { role };
            out.push(SubagentMessagePreview {
                role: label.to_string(),
                text: parts.join("\n"),
            });
        }
    }
    out
}

/// Fetch Session-scoped subagent descendants (roster lineage + `get_rlm_children`).
///
/// # Errors
/// Fails if `roster_subscribe` fails.
pub fn list_session_subagents(
    client: &Client,
    active_session_id: &str,
    session_id: &str,
) -> std::io::Result<Vec<SubagentItem>> {
    let roster = list_roster(client)?;
    let children = client
        .request(
            "get_rlm_children",
            json!({"activeSessionId": active_session_id}),
            Duration::from_secs(3),
        )
        .ok()
        .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
        .and_then(|r| r.pointer("/data/children").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    Ok(parse_session_subagents(
        &roster,
        &children,
        active_session_id,
        session_id,
    ))
}

/// Fetch read-only transcript messages for a Subagent Session (`get_messages`).
///
/// # Errors
/// Fails on transport or daemon error.
pub fn fetch_subagent_messages(
    client: &Client,
    subagent_active_session_id: &str,
) -> std::io::Result<Vec<SubagentMessagePreview>> {
    let raw = crate::get_messages_within(client, subagent_active_session_id, Duration::from_secs(5))?;
    Ok(extract_subagent_messages(&raw))
}

/// Stop a running Subagent (`cancel_rlm_child`), which records `"Cancelled by user"` on its parent.
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn cancel_subagent(
    client: &Client,
    parent_active_session_id: &str,
    child_id: &str,
) -> std::io::Result<bool> {
    let response = client.request(
        "cancel_rlm_child",
        json!({
            "activeSessionId": parent_active_session_id,
            "childId": child_id,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "cancel_rlm_child failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(response
        .pointer("/data/cancelled")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

/// Delete an inactive Subagent (`delete_rlm_subagent`).
///
/// # Errors
/// Fails on transport or daemon command error.
pub fn delete_subagent(
    client: &Client,
    parent_active_session_id: &str,
    child_id: &str,
) -> std::io::Result<bool> {
    let response = client.request(
        "delete_rlm_subagent",
        json!({
            "activeSessionId": parent_active_session_id,
            "childId": child_id,
        }),
        Duration::from_secs(5),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "delete_rlm_subagent failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(response
        .pointer("/data/deleted")
        .and_then(Value::as_bool)
        .unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chip_count_displays_gold_running_or_muted_total() {
        assert_eq!(ChipCount::new(0, 0).display_value(), (0, false));
        assert_eq!(ChipCount::new(0, 3).display_value(), (3, false));
        assert_eq!(ChipCount::new(2, 5).display_value(), (2, true));
    }

    #[test]
    fn parse_scoped_heartbeats_filters_session_and_sorts_active_first() {
        let data = json!({
            "heartbeats": [
                {
                    "job": {
                        "id": "hb-other",
                        "status": "active",
                        "activeSessionId": "s-other",
                        "sessionId": "uuid-other",
                        "prompt": "other session",
                        "schedule": { "expression": "every 1m" }
                    }
                },
                {
                    "job": {
                        "id": "hb-paused",
                        "status": "paused",
                        "source": "rlm_heartbeat",
                        "deliveryMode": "follow_up",
                        "activeSessionId": "s1",
                        "sessionId": "uuid-1",
                        "prompt": "agent watch",
                        "schedule": { "expression": "*/10 * * * *" },
                        "runCount": 3
                    }
                },
                {
                    "job": {
                        "id": "hb-active",
                        "status": "active",
                        "source": "heartbeat",
                        "deliveryMode": "steer",
                        "activeSessionId": "s1",
                        "sessionId": "uuid-1",
                        "prompt": "check build",
                        "schedule": { "expression": "every 5m" },
                        "nextRunAt": "2026-11-24T12:05:00Z",
                        "runCount": 1
                    }
                }
            ]
        });
        let items = parse_scoped_heartbeats(&data, "s1", "uuid-1");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "hb-active");
        assert!(items[0].is_active());
        assert!(items[0].is_user_heartbeat());
        assert_eq!(items[0].delivery_label(), "steer");
        assert_eq!(items[1].id, "hb-paused");
        assert!(!items[1].is_active());
        assert!(!items[1].is_user_heartbeat());
        assert_eq!(items[1].delivery_label(), "follow-up");
        assert_eq!(heartbeat_counts(&items), ChipCount::new(1, 2));
        assert!(parse_scoped_heartbeats(&data, "", "").is_empty());
    }

    #[test]
    fn parse_shell_activities_sorts_running_first_and_counts_running_and_total() {
        let data = json!({
            "activities": [
                { "id": "", "command": "ignored", "status": "running" },
                { "id": "sh-done", "command": "git status\n", "pid": 100, "status": "finished", "exitCode": 0, "durationMs": 200 },
                { "id": "sh-run", "command": "cargo test", "pid": 200, "status": "running", "durationMs": 4500 },
                { "id": "sh-fail", "command": "npm run lint", "pid": 300, "status": "failed", "exitCode": 1, "durationMs": 900 }
            ]
        });
        let items = parse_shell_activities(&data);
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].id, "sh-run");
        assert!(items[0].is_running());
        assert_eq!(items[1].id, "sh-done");
        assert_eq!(items[1].command, "git status");
        assert_eq!(items[2].id, "sh-fail");
        assert_eq!(items[2].exit_code, Some(1));
        assert_eq!(shell_counts(&items), ChipCount::new(1, 3));
    }

    #[test]
    fn subagent_counts_tracks_running_and_total_across_nested_descendants() {
        let roster = vec![
            json!({
                "agentId": "c1",
                "status": "running",
                "summary": {
                    "activeSessionId": "c1",
                    "sessionId": "u1",
                    "runtimeKind": "subagent",
                    "parentActiveSessionId": "root",
                    "parentSessionId": "u-root"
                }
            }),
            json!({
                "agentId": "c2",
                "status": "completed",
                "summary": {
                    "activeSessionId": "c2",
                    "sessionId": "u2",
                    "runtimeKind": "subagent",
                    "rlmDepth": 2,
                    "parentActiveSessionId": "c1",
                    "parentSessionId": "u1"
                }
            }),
            json!({
                "agentId": "top-level-ignored",
                "status": "running",
                "summary": {
                    "activeSessionId": "top-level-ignored",
                    "sessionId": "u-top",
                    "runtimeKind": "top-level",
                    "rlmDepth": 1,
                    "parentActiveSessionId": "root"
                }
            }),
            json!({
                "agentId": "other",
                "status": "running",
                "summary": {
                    "activeSessionId": "other",
                    "sessionId": "u-other",
                    "runtimeKind": "subagent",
                    "rlmDepth": 1,
                    "parentActiveSessionId": "unrelated-root"
                }
            }),
        ];
        assert_eq!(
            subagent_counts(&roster, "root", "u-root"),
            ChipCount::new(1, 2)
        );
        assert_eq!(running_subagents(&roster, "root", "u-root"), 1);
    }

    #[test]
    fn parse_session_subagents_builds_nested_tree_sorts_running_first_and_excludes_unrelated() {
        let roster = vec![
            json!({
                "agentId": "c-done",
                "status": "completed",
                "summary": {
                    "activeSessionId": "c-done",
                    "sessionId": "u-done",
                    "sessionName": "audit-shells",
                    "runtimeKind": "subagent",
                    "rlmDepth": 1,
                    "parentActiveSessionId": "root",
                    "parentSessionId": "u-root",
                    "modelId": "claude-sonnet-4"
                }
            }),
            json!({
                "agentId": "c-run",
                "status": "running",
                "summary": {
                    "activeSessionId": "c-run",
                    "sessionId": "u-run",
                    "sessionName": "audit-heartbeats",
                    "runtimeKind": "subagent",
                    "parentActiveSessionId": "root",
                    "parentSessionId": "u-root",
                    "modelId": "claude-sonnet-4"
                }
            }),
            json!({
                "agentId": "c-nested",
                "status": "running",
                "summary": {
                    "activeSessionId": "c-nested",
                    "sessionId": "u-nested",
                    "sessionName": "check-cron-tests",
                    "runtimeKind": "subagent",
                    "rlmDepth": 2,
                    "parentActiveSessionId": "c-run",
                    "parentSessionId": "u-run",
                    "modelId": "claude-sonnet-4"
                }
            }),
            json!({
                "agentId": "c-other",
                "status": "running",
                "summary": {
                    "activeSessionId": "c-other",
                    "sessionId": "u-other",
                    "sessionName": "foreign-worker",
                    "runtimeKind": "subagent",
                    "rlmDepth": 1,
                    "parentActiveSessionId": "other-root"
                }
            }),
        ];
        let children = vec![
            json!({
                "id": "child-1",
                "activeSessionId": "c-run",
                "sessionId": "u-run",
                "sessionName": "audit-heartbeats",
                "status": "running",
                "progressNote": "Inspecting arms.rs",
                "durationMs": 12_000
            }),
            json!({
                "id": "child-old",
                "activeSessionId": "c-old",
                "sessionName": "finished-earlier",
                "status": "cancelled",
                "model": "claude-sonnet-4",
                "answerPreview": "Cancelled by user",
                "durationMs": 3_000
            }),
        ];
        let items = parse_session_subagents(&roster, &children, "root", "u-root");
        assert_eq!(items.len(), 4);
        assert_eq!(items[0].name, "audit-heartbeats");
        assert_eq!(items[0].depth, 0);
        assert_eq!(items[0].child_id, "child-1");
        assert_eq!(items[0].activity, "Inspecting arms.rs");
        assert!(items[0].can_manage());
        assert_eq!(items[1].name, "check-cron-tests");
        assert_eq!(items[1].depth, 1);
        assert_eq!(items[2].name, "audit-shells");
        assert_eq!(items[2].depth, 0);
        assert_eq!(items[3].name, "finished-earlier");
        assert_eq!(items[3].child_id, "child-old");
        assert_eq!(items[3].model, "claude-sonnet-4");
        assert_eq!(items[3].status, "cancelled");
        assert!(items[3].can_manage());
        assert_eq!(subagent_items_counts(&items), ChipCount::new(2, 4));
    }

    #[test]
    fn extract_subagent_messages_formats_roles_and_tool_calls() {
        let msgs = vec![
            json!({"role": "user", "content": "Audit cron tests"}),
            json!({"role": "assistant", "content": [
                {"type": "text", "text": "Checking store.rs"},
                {"type": "toolCall", "name": "python_repl"}
            ]}),
            json!({"role": "toolResult", "content": "ignored raw output"}),
        ];
        let parsed = extract_subagent_messages(&msgs);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].role, "task");
        assert_eq!(parsed[0].text, "Audit cron tests");
        assert_eq!(parsed[1].role, "assistant");
        assert_eq!(parsed[1].text, "Checking store.rs\n[python_repl]");
    }
}
