//! Session messages -> transcript rows.
//!
//! Shapes straight off the wire (verified against a live attach):
//!   {role:"user",  content:[{type:"text", text}]}
//!   {role:"assistant", content:[{type:"thinking"|"text"|"toolCall", ...}]}
//!   {role:"toolResult", toolCallId, toolName, content:[{text}]}
//!   {customType, content, display}
//!   {role:"compactionSummary", summary, tokensBefore, retainedMessageCount, ...}
//!   {role:"branchSummary", summary, fromId}


use serde_json::Value;
use slint::StyledText;

use crate::theme::{self, Resolved};

mod autolink;
mod cache;
mod cells;
mod demo;
mod markup;
#[cfg(test)]
mod tests;

// The subjects below are one thing each, re-exported here so a call site keeps
// saying `transcript::markdown_markup` and never has to know the file it lives in.
pub(crate) use autolink::*;
pub(crate) use cache::*;
pub(crate) use cells::*;
pub(crate) use demo::*;
pub(crate) use markup::*;

pub const KIND_USER: i32 = 0;

pub const KIND_AGENT: i32 = 1;

pub const KIND_THINKING: i32 = 2;

pub const KIND_CELL: i32 = 3;

/// A custom daemon message: a readable label, an optional body, and the same
/// open/closed contract as a tool cell. The daemon's `customType` is the
/// dispatch key, never the visible title.
pub const KIND_CUSTOM_MESSAGE: i32 = 5;

/// A compaction or branch summary: the context that explains why the
/// transcript jumped. It shares the body-first row treatment with a custom
/// message but has its own kind so the two can be styled apart.
pub const KIND_SUMMARY: i32 = 10;

/// One generic error line: `⚠ Error: <message>`. It is a transcript row, not a
/// Status strip state and not a card. The message is collapsed to its first
/// non-empty line, as the TUI's error row does.
pub const KIND_ERROR: i32 = 9;

/// A fenced code block, rendered on its own surface rather than as prose.
pub const KIND_CODE: i32 = 6;

/// A GFM table, drawn with box characters in a monospace face.
pub const KIND_TABLE: i32 = 7;

/// An image the user sent: drawn as a thumbnail, not as words.
pub const KIND_IMAGE: i32 = 8;

/// A Markdown blockquote: a bar and muted italic body, separate from prose rows.
pub const KIND_QUOTE: i32 = 11;

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: i32,
    /// Tool-call id for cell rows: the live event path and the store path both
    /// build a row for the same call, and this is how they are told apart.
    pub id: String,
    pub title: String,
    pub meta: String,
    pub body: String,
    pub code: String,
    pub output: String,
    pub has_code: bool,
    pub has_output: bool,
    /// A custom message has content to show under its label. Tool cells use
    /// `has_output` for their result; this is the custom-row equivalent.
    pub has_body: bool,
    pub expanded: bool,
    pub failed: bool,
    pub reason: String,
    /// base64 payload of an image row, exactly as it sits on the wire
    pub image: String,
    /// which message this row belongs to: the daemon's id when it has one,
    /// a custom-message key from its details or content, and a role+timestamp
    /// key only as the last resort. Rows are merged per message, so a rebuild
    /// can replace one message without touching the rest.
    pub message: String,
    /// how long the call took, drawn as its own column: a separator dot between
    /// two dim runs reads as a full stop
    pub elapsed: String,
    pub timestamp: String,
}

/// The one line a generic error row carries: the first non-empty line, with
/// the TUI's trailing-space convention omitted. Empty input still says Error.
fn error_line(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error")
        .to_string()
}

impl Row {
    /// The generic one-line error row. `message` is plain text; the markup
    /// builder escapes it so an error that quotes HTML cannot break the row.
    pub(crate) fn error(message: &str) -> Self {
        let line = error_line(message);
        let mut row = Self::new(KIND_ERROR);
        row.title = "Error:".to_string();
        row.body = plain_markup(&line);
        row
    }

    fn new(kind: i32) -> Self {
        Self {
            kind,
            id: String::new(),
            title: String::new(),
            meta: String::new(),
            body: String::new(),
            code: String::new(),
            output: String::new(),
            has_code: false,
            has_output: false,
            has_body: false,
            expanded: false,
            failed: false,
            reason: String::new(),
            image: String::new(),
            elapsed: String::new(),
            timestamp: String::new(),
            message: String::new(),
        }
    }
}

/// The text a custom message carries. Custom messages are usually a string,
/// but injected prompts may carry content blocks; only text blocks belong on
/// this row, and non-text blocks are not silently shown as prose.
fn custom_message_text(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

struct CustomMessageMeta {
    label: String,
    meta: String,
    failed: bool,
    has_body: bool,
}

fn plain_meta(label: &str, has_body: bool) -> CustomMessageMeta {
    CustomMessageMeta {
        label: label.to_string(),
        meta: String::new(),
        failed: false,
        has_body,
    }
}

fn failed_meta(label: &str, has_body: bool) -> CustomMessageMeta {
    CustomMessageMeta {
        label: label.to_string(),
        meta: String::new(),
        failed: true,
        has_body,
    }
}

fn custom_slash_name(details: &Value) -> Option<&str> {
    details.pointer("/command/name").and_then(Value::as_str)
}

/// The refinement outcome's state from the daemon's edit list. The daemon
/// writes `edits` per operation with applied flags; the summary text alone is
/// optimistic and must not decide whether the row failed.
fn refinement_state(details: &Value) -> (String, String, bool) {
    let Some(edits) = details.get("edits").and_then(Value::as_array) else {
        let failed = details.get("success").and_then(Value::as_bool) == Some(false)
            || details.get("error").is_some();
        return ("Refinement".to_string(), String::new(), failed);
    };
    let total = edits.len();
    let applied = edits
        .iter()
        .filter(|edit| edit.get("applied").and_then(Value::as_bool) == Some(true))
        .count();
    let (label, failed) = if total == 0 {
        ("Refinement".to_string(), false)
    } else if applied == 0 {
        ("Refinement failed".to_string(), true)
    } else if applied < total {
        ("Refinement partial".to_string(), false)
    } else {
        ("Refinement".to_string(), false)
    };
    let meta = if total == 0 {
        String::new()
    } else {
        format!("{applied}/{total} edits")
    };
    (label, meta, failed)
}

/// One match owns custom-type treatment, so adding a type has one place to
/// change.
fn custom_message_meta(custom_type: &str, body: &str, details: &Value) -> CustomMessageMeta {
    let has_body = !body.trim().is_empty();
    match custom_type {
        "session_slash_command" => {
            let text = body.trim();
            let label = text.split_whitespace().next().unwrap_or("Command").to_string();
            // A bare command is the whole row. Arguments stay available in
            // the body instead of being cut from the one-line label.
            let has_arguments =
                text.lines().count() > 1 || text.split_whitespace().count() > 1;
            CustomMessageMeta {
                label,
                meta: String::new(),
                failed: false,
                has_body: has_arguments && has_body,
            }
        }
        "session_slash_command_result" => CustomMessageMeta {
            label: match custom_slash_name(details) {
                Some("compact") => "Compaction",
                Some("refine") => "Refinement",
                Some("goal") => "Goal",
                Some("autonomous") => "Autonomous mode",
                _ => "Command result",
            }
            .to_string(),
            meta: String::new(),
            failed: details.get("success").and_then(Value::as_bool) == Some(false),
            has_body,
        },
        "agent_message" => plain_meta("Agent message", has_body),
        "async_bash_completion" => {
            let exit_code = details
                .get("exitCode")
                .or_else(|| details.get("exit_code"))
                .and_then(Value::as_i64);
            CustomMessageMeta {
                label: "Background shell command finished".to_string(),
                meta: exit_code.map_or_else(String::new, |code| format!("exit {code}")),
                failed: exit_code.is_some_and(|code| code != 0),
                has_body,
            }
        }
        "rlm_child_terminal_notice" => plain_meta("Subagent finished", has_body),
        "rlm_child_failure" => failed_meta("Subagent failed", has_body),
        "compaction_outcome" => {
            let outcome = details.get("outcome").and_then(Value::as_str).unwrap_or("");
            CustomMessageMeta {
                label: "Compaction".to_string(),
                meta: outcome.to_string(),
                failed: matches!(outcome, "failed" | "cancelled"),
                has_body,
            }
        }
        "refinement_outcome" => {
            let (label, meta, failed) = refinement_state(details);
            CustomMessageMeta {
                label,
                meta,
                failed,
                has_body,
            }
        }
        "provider_retry_outcome" => CustomMessageMeta {
            label: "Provider retry".to_string(),
            meta: String::new(),
            failed: details.get("success").and_then(Value::as_bool) == Some(false),
            has_body,
        },
        "model_prompt_error" => failed_meta("Model prompt error", has_body),
        "heartbeat_prompt" => plain_meta("Heartbeat", has_body),
        "goal_context" => plain_meta("Goal context", has_body),
        "ipython_state_restored" => plain_meta("Python state restored", has_body),
        "python_skills_unavailable" => failed_meta("Python skills unavailable", has_body),
        "autonomous_status" => plain_meta("Autonomous mode", has_body),
        "session_renamed" => plain_meta("Session renamed", has_body),
        "factory_progress_notice" => plain_meta("Factory progress", has_body),
        "mcp_connection_outcome" => plain_meta("MCP connection", has_body),
        _ => plain_meta("Custom message", has_body),
    }
}

/// The body is visible by default; the header closes it.
fn custom_message_row(
    custom_type: &str,
    body: &str,
    details: &Value,
    theme: &Resolved,
) -> Row {
    let meta = custom_message_meta(custom_type, body, details);
    let mut row = Row::new(KIND_CUSTOM_MESSAGE);
    row.title = meta.label;
    row.meta = meta.meta;
    row.failed = meta.failed;
    if meta.has_body {
        set_body(&mut row, body, theme);
    }
    row
}

/// Body rows start open; the header closes them.
fn set_body(row: &mut Row, body: &str, theme: &Resolved) {
    row.has_body = !body.trim().is_empty();
    if row.has_body {
        row.body = markdown_markup(body, theme);
        row.expanded = true;
    }
}

/// A summary role is a summary row, not a custom message. Compaction summaries
/// are emitted before the retained messages; branch summaries stay where the
/// conversation returned from the branch.
fn summary_row(message: &Value, role: &str, theme: &Resolved) -> Option<Row> {
    let summary = message.get("summary").and_then(Value::as_str).unwrap_or_default();
    let mut row = Row::new(KIND_SUMMARY);
    match role {
        "compactionSummary" => {
            row.title = "Context compacted".to_string();
            let tokens = message.get("tokensBefore").and_then(Value::as_u64);
            let retained = message
                .get("retainedMessageCount")
                .and_then(Value::as_u64);
            row.meta = [
                tokens.map_or_else(String::new, |value| format!("{value} tokens before")),
                retained.map_or_else(String::new, |value| format!("{value} retained")),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" \u{00b7} ");
            let focus = message
                .get("customInstructions")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim();
            let body = if focus.is_empty() {
                summary.to_string()
            } else {
                format!("**Focus:** {focus}\n\n{summary}")
            };
            set_body(&mut row, &body, theme);
        }
        "branchSummary" => {
            row.title = "Branch summary".to_string();
            let from = message.get("fromId").and_then(Value::as_str).unwrap_or_default();
            row.meta = if from.is_empty() {
                String::new()
            } else {
                format!("from {from}")
            };
            set_body(&mut row, summary, theme);
        }
        _ => return None,
    }
    Some(row)
}

/// One message -> its rows. `owner` is stamped on every row so a rebuild can
/// replace a message where it stands.
fn build_message(
    message: &Value,
    owner: &str,
    outputs: &std::collections::HashMap<String, (String, bool)>,
    bashes: &std::collections::HashMap<String, BashInfo>,
    durations: &std::collections::HashMap<String, u64>,
    theme: &Resolved,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
    if let Some(mut row) = summary_row(message, role, theme) {
        row.message = owner.to_string();
        return vec![row];
    }
    if let Some(custom_type) = message.get("customType").and_then(Value::as_str) {
        // The wire carries internal bookkeeping rows (kernel state, harness
        // digests, model-facing refinement notices). They are deliberately
        // not transcript material.
        if message.get("display").and_then(Value::as_bool) == Some(false) {
            return rows;
        }
        let body = custom_message_text(message);
        let details = message.get("details").unwrap_or(&Value::Null);
        let mut row = custom_message_row(custom_type, &body, details, theme);
        row.message = owner.to_string();
        rows.push(row);
        return rows;
    }

    let Some(blocks) = message.get("content").and_then(Value::as_array) else {
        return rows;
    };
    for block in blocks {
        match block.get("type").and_then(Value::as_str).unwrap_or_default() {
            // An image is a row of its own. Without this a message that is
            // nothing but a picture produced no row at all, and one with
            // words lost the picture: the blocks were read for text only.
            "image" => {
                let Some(data) = block.get("data").and_then(Value::as_str) else {
                    continue;
                };
                if data.is_empty() {
                    continue;
                }
                let mut row = Row::new(KIND_IMAGE);
                row.image = data.to_string();
                rows.push(row);
            }
            "text" => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                // A message is not one row: a fenced block needs its own
                // surface and a table needs a grid, so each is its own row
                // and the prose between them keeps the markdown renderer.
                for piece in split_blocks(text) {
                    match piece {
                        Block::Prose(prose) => {
                            let mut row =
                                Row::new(if role == "user" { KIND_USER } else { KIND_AGENT });
                            row.body = markdown_markup(&prose, theme);
                            rows.push(row);
                        }
                        Block::Quote(quote) => {
                            let mut row = Row::new(KIND_QUOTE);
                            row.body = quote_markup(&quote, theme);
                            rows.push(row);
                        }
                        Block::Code { lang, body } => {
                            let mut row = Row::new(KIND_CODE);
                            row.code = highlight_code(&lang, &body, theme);
                            row.meta = lang;
                            row.has_code = true;
                            rows.push(row);
                        }
                        Block::Table(rendered) => {
                            let mut row = Row::new(KIND_TABLE);
                            row.body = plain_markup(&rendered);
                            rows.push(row);
                        }
                    }
                }
            }
            "thinking" => {
                let text = block.get("thinking").and_then(Value::as_str).unwrap_or_default();
                if text.trim().is_empty() {
                    continue;
                }
                let mut row = Row::new(KIND_THINKING);
                row.body = plain_markup(text);
                row.meta = format!("{} lines", text.lines().count());
                rows.push(row);
            }
            "toolCall" => {
                let id = block.get("id").and_then(Value::as_str).unwrap_or_default();
                let code = block
                    .pointer("/arguments/code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut row = Row::new(KIND_CELL);
                row.id = id.to_string();
                row.code = highlight_python(code, theme);
                row.has_code = !code.is_empty();
                if let Some((output, failed)) = outputs.get(id) {
                    row.output = plain_markup(output);
                    row.has_output = !output.trim().is_empty();
                    row.failed = *failed;
                    row.reason = failure_reason(output, *failed);
                }
                let lines = code.lines().count();
                row.meta = if lines == 1 { "1 line".to_string() } else { format!("{lines} lines") };
                let info = bashes.get(&row.id).cloned();
                let taken = durations.get(&row.id).copied();
                apply_header(&mut row, code, info.as_ref(), taken);
                rows.push(row);
            }
            _ => {}
        }
    }
    let stamp = crate::appearance::format_timestamp(message.get("timestamp"));
    let mut stamped = false;
    for row in &mut rows {
        row.message = owner.to_string();
        if !stamped && !stamp.is_empty() && matches!(row.kind, KIND_USER | KIND_AGENT) {
            row.timestamp.clone_from(&stamp);
            stamped = true;
        }
    }
    rows
}

/// Build the transcript rows for the newest `limit` messages.
pub fn build(messages: &[Value], limit: usize) -> (Vec<Row>, usize) {
    // The theme the window is drawn in, read once here rather than carried
    // through the worker, the live turn and every cell. It is one value for the
    // life of the process, which is also what makes the per-message cache below
    // safe: a row is a row under the theme it was built in.
    let theme = theme::current();
    let start = messages.len().saturating_sub(limit);
    let window = &messages[start..];

    // tool outputs keyed by call id, so a cell carries its own result
    let mut outputs: std::collections::HashMap<String, (String, bool)> = std::collections::HashMap::new();
    // and the bash preview the tool reported for itself: the TUI titles a cell
    // by the command that ran, not by the first line of the cell's source
    let mut bashes: std::collections::HashMap<String, BashInfo> = std::collections::HashMap::new();
    let mut durations: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for message in window {
        if message.get("role").and_then(Value::as_str) != Some("toolResult") {
            continue;
        }
        let id = message.get("toolCallId").and_then(Value::as_str).unwrap_or_default();
        let text = message
            .get("content")
            .and_then(Value::as_array)
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|block| block.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let failed = text.trim_start().starts_with("Traceback");
        outputs.insert(id.to_string(), (text, failed));
        if let Some(info) = bash_commands(message.get("details")) {
            bashes.insert(id.to_string(), info);
        }
        if let Some(ms) = duration_ms(message.get("details")) {
            durations.insert(id.to_string(), ms);
        }
    }

    let mut rows = Vec::new();
    for message in window {
        // folded into its cell
        if message.get("role").and_then(Value::as_str) == Some("toolResult") {
            continue;
        }
        let owner = message_key(message);
        let stamp = fingerprint(message, &outputs, &bashes, &durations);
        match cache_get(&owner, stamp) {
            Some(cached) => rows.extend(cached),
            None => {
                let built = build_message(message, &owner, &outputs, &bashes, &durations, &theme);
                cache_put(&owner, stamp, &built);
                rows.extend(built);
            }
        }
    }
    (rows, start)
}

/// The daemon's id for a message, or a stable stand-in. Custom rows prefer
/// their details id, then their content: the two wire paths use different
/// timestamp shapes, so a timestamp cannot key them together.
#[must_use]
pub fn message_key(message: &Value) -> String {
    if let Some(custom_type) = message.get("customType").and_then(Value::as_str) {
        if let Some(id) = message
            .pointer("/details/id")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        {
            return format!("custom:{custom_type}:{id}");
        }
        let body = custom_message_text(message);
        return format!(
            "custom:{custom_type}:{}:{}",
            body.chars().count(),
            body.chars().take(40).collect::<String>()
        );
    }
    if let Some(id) = message.get("id").and_then(Value::as_str) {
        if !id.is_empty() {
            return id.to_string();
        }
    }
    let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
    let stamp = message
        .get("timestamp")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    format!("{role}:{stamp}")
}
