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
    let mut parents: Vec<(String, u64, Option<String>)> = [
        roster_key("active", Some(active_session_id)),
        roster_key("session", Some(session_id)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .into_iter()
    .map(|key| (key, 0, None))
    .collect();
    if parents.is_empty() {
        return 0;
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
    let mut running = 0;
    let mut index = 0;
    while index < parents.len() {
        let (parent_key, parent_depth, parent_file) = parents[index].clone();
        index += 1;
        for entry in roster {
            let Some(summary) = roster_summary(entry) else {
                continue;
            };
            let identity = roster_identity_keys(summary);
            if identity.iter().any(|key| root_ids.contains(key)) {
                continue;
            }
            let depth = summary.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0);
            let subagent = summary
                .get("runtimeKind")
                .and_then(Value::as_str)
                .map_or(depth > 0, |kind| kind == "subagent");
            if !subagent {
                continue;
            }
            let matches_id = roster_parent_keys(summary)
                .iter()
                .any(|key| key == &parent_key);
            let matches_path = parent_file.as_deref().is_some_and(|file| {
                depth == parent_depth + 1
                    && summary
                        .get("parentSessionPath")
                        .and_then(Value::as_str)
                        .is_some_and(|path| normalize_session_path(path) == file)
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
    running
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
