//! Session messages -> transcript rows.
//!
//! Shapes straight off the wire (verified against a live attach):
//!   {role:"user",  content:[{type:"text", text}]}
//!   {role:"assistant", content:[{type:"thinking"|"text"|"toolCall", ...}]}
//!   {role:"toolResult", toolCallId, toolName, content:[{text}]}
//!   {customType, content, display}


use serde_json::Value;
use slint::StyledText;

use crate::theme::{self, Resolved};

mod cache;
mod cells;
mod demo;
mod markup;
#[cfg(test)]
mod tests;

// The subjects below are one thing each, re-exported here so a call site keeps
// saying `transcript::markdown_markup` and never has to know the file it lives in.
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
pub const KIND_NOTICE: i32 = 5;

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
    /// which message this row belongs to: the daemon's id when it has one, a
    /// role+timestamp key when it does not. Rows are merged per message, so a
    /// rebuild can replace one message without touching the rest.
    pub message: String,
    /// how long the call took, drawn as its own column: a separator dot between
    /// two dim runs reads as a full stop
    pub elapsed: String,
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
            message: String::new(),
        }
    }
}

/// The text a custom message carries. Custom messages are usually a string,
/// but injected prompts may carry content blocks; the text blocks are the
/// readable ones, and images are handled by their own row in ordinary prose.
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

/// The command name attached to a session slash-command row.
fn custom_slash_name<'a>(details: &'a Value) -> Option<&'a str> {
    details.pointer("/command/name").and_then(Value::as_str)
}

/// A custom message's readable label. Raw daemon type names are a developer
/// vocabulary, not transcript copy.
fn custom_message_label(custom_type: &str, body: &str, details: &Value) -> String {
    match custom_type {
        "session_slash_command" => {
            if body.trim().is_empty() {
                "Command".to_string()
            } else {
                body.trim().lines().next().unwrap_or("Command").to_string()
            }
        }
        "session_slash_command_result" => match custom_slash_name(details) {
            Some("compact") => "Compaction".to_string(),
            Some("refine") => "Refinement".to_string(),
            Some("goal") => "Goal".to_string(),
            Some("autonomous") => "Autonomous mode".to_string(),
            _ => "Command result".to_string(),
        },
        "agent_message" => "Agent message".to_string(),
        "async_bash_completion" => "Background shell command finished".to_string(),
        "rlm_child_terminal_notice" => "Subagent finished".to_string(),
        "rlm_child_failure" => "Subagent failed".to_string(),
        "compaction_outcome" => "Compaction".to_string(),
        "refinement_outcome" => "Refinement".to_string(),
        "provider_retry_outcome" => "Provider retry".to_string(),
        "model_prompt_error" => "Model prompt error".to_string(),
        "heartbeat_prompt" => "Heartbeat".to_string(),
        "goal_context" => "Goal context".to_string(),
        "ipython_state_restored" => "Python state restored".to_string(),
        "python_skills_unavailable" => "Python skills unavailable".to_string(),
        "autonomous_status" => "Autonomous mode".to_string(),
        "session_renamed" => "Session renamed".to_string(),
        "factory_progress_notice" => "Factory progress".to_string(),
        "mcp_connection_outcome" => "MCP connection".to_string(),
        other => humanize_type(other),
    }
}

/// A fallback for a custom type the viewer does not know yet. It keeps the
/// message readable instead of making the raw wire name the headline.
fn humanize_type(custom_type: &str) -> String {
    let text = custom_type.replace('_', " ").replace('-', " ");
    let mut words = text.split_whitespace();
    let Some(first) = words.next() else {
        return "Custom message".to_string();
    };
    let mut chars = first.chars();
    let initial = chars.next().unwrap_or('C');
    let mut label: String = initial.to_uppercase().collect();
    label.push_str(chars.as_str());
    for word in words {
        label.push(' ');
        label.push_str(word);
    }
    label
}

/// Whether a custom row is an error or failed operation.
fn custom_message_failed(custom_type: &str, details: &Value) -> bool {
    match custom_type {
        "session_slash_command_result" | "provider_retry_outcome" => {
            details.get("success").and_then(Value::as_bool) == Some(false)
        }
        "model_prompt_error" | "rlm_child_failure" | "python_skills_unavailable" => true,
        "compaction_outcome" => matches!(
            details.get("outcome").and_then(Value::as_str),
            Some("failed" | "cancelled")
        ),
        _ => false,
    }
}

/// Short results open themselves; a long agent message starts closed so it
/// does not take the whole sheet before the reader asks for it.
fn custom_message_is_short(body: &str) -> bool {
    let body = body.trim();
    body.lines().count() <= 4 && body.chars().count() <= 320
}

/// One daemon custom message -> one notice row. The row carries the body the
/// old title-only notice dropped, and it can open and close like a cell.
fn custom_message_row(
    custom_type: &str,
    body: &str,
    details: &Value,
    theme: &Resolved,
) -> Row {
    let mut row = Row::new(KIND_NOTICE);
    row.title = custom_message_label(custom_type, body, details);
    row.failed = custom_message_failed(custom_type, details);
    row.has_body = custom_type != "session_slash_command" && !body.trim().is_empty();
    if row.has_body {
        row.body = markdown_markup(body, theme);
        row.expanded = custom_message_is_short(body);
    }
    row
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
    for row in &mut rows {
        row.message = owner.to_string();
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

/// The daemon's id for a message, or a stable stand-in built from what every
/// frame carries: role and timestamp.
#[must_use]
pub fn message_key(message: &Value) -> String {
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
