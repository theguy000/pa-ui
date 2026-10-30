//! Session messages -> transcript rows.
//!
//! Shapes straight off the wire (verified against a live attach):
//!   {role:"user",  content:[{type:"text", text}]}
//!   {role:"assistant", content:[{type:"thinking"|"text"|"toolCall", ...}]}
//!   {role:"toolResult", toolCallId, toolName, content:[{text}]}
//!   {customType, content, display}


use serde_json::Value;
use slint::{Model, SharedString, StyledText};

mod cache;
mod cells;
mod demo;
mod markup;
#[cfg(test)]
mod tests;

// The subjects below are one thing each, re-exported here so a call site keeps
// saying `transcript::markdown_markup` and never has to know the file it lives in.
#[allow(unused_imports)]
pub(crate) use cache::*;
pub(crate) use cells::*;
pub(crate) use demo::*;
pub(crate) use markup::*;

pub const KIND_USER: i32 = 0;

pub const KIND_AGENT: i32 = 1;

pub const KIND_THINKING: i32 = 2;

pub const KIND_CELL: i32 = 3;

pub const KIND_NOTICE: i32 = 5;

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
    pub expanded: bool,
    pub failed: bool,
    pub reason: String,
    /// base64 payload of an image row, exactly as it sits on the wire
    pub image: String,
    pub mime: String,
    /// which message this row belongs to: the daemon's id when it has one, a
    /// role+timestamp key when it does not. Rows are merged per message, so a
    /// rebuild can replace one message without touching the rest.
    pub message: String,
    /// how long the call took, drawn as its own column: a separator dot between
    /// two dim runs reads as a full stop
    pub elapsed: String,
}

impl Row {
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
            expanded: false,
            failed: false,
            reason: String::new(),
            image: String::new(),
            mime: String::new(),
            elapsed: String::new(),
            message: String::new(),
        }
    }
}

/// One message -> its rows. `owner` is stamped on every row so a rebuild can
/// replace a message where it stands.
fn build_message(
    message: &Value,
    owner: &str,
    outputs: &std::collections::HashMap<String, (String, bool)>,
    bashes: &std::collections::HashMap<String, BashInfo>,
    durations: &std::collections::HashMap<String, u64>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
    if message.get("customType").is_some() {
        let text = message.get("content").and_then(Value::as_str).unwrap_or_default();
        let mut row = Row::new(KIND_NOTICE);
        row.body = plain_markup(text);
        row.title = message
            .get("customType")
            .and_then(Value::as_str)
            .unwrap_or("notice")
            .to_string();
        rows.push(row);
        for row in &mut rows {
            row.message = owner.to_string();
        }
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
                row.mime = block
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("image/png")
                    .to_string();
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
                            row.body = markdown_markup(&prose);
                            rows.push(row);
                        }
                        Block::Code { lang, body } => {
                            let mut row = Row::new(KIND_CODE);
                            row.code = highlight_code(&lang, &body);
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
                let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                let code = block
                    .pointer("/arguments/code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let mut row = Row::new(KIND_CELL);
                row.id = id.to_string();
                let fallback = block.get("arguments").map(ToString::to_string).unwrap_or_default();
                let summary = summary_line(if code.is_empty() { &fallback } else { code });
                row.title = format!("{} · {}", short_tool(name), summary);
                row.code = highlight_python(code);
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
                let built = build_message(message, &owner, &outputs, &bashes, &durations);
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
