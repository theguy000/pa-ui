//! The message that is still being written.
//!
//! One open message owns one contiguous range of rows, and every frame rebuilds
//! that whole range from the parts accumulated so far. Nothing is patched in
//! place and nothing is reconciled by id: there is exactly one representation of
//! the open message, so a second copy cannot exist to fall out of step with the
//! first. This is the shape the TUI uses - a single `streaming_index`, replaced
//! wholesale on each frame - and it is the only arrangement that has survived
//! contact with this wire.

use serde_json::Value;

use crate::transcript::{self, Row};

pub enum Part {
    Thinking(String),
    Text(String),
    Tool {
        id: String,
        name: String,
        args: Value,
        output: Option<String>,
        failed: bool,
        /// what the tool said it ran, when it says
        bash: Option<transcript::BashInfo>,
        /// and how long it took
        duration: Option<u64>,
    },
}

pub struct Live {
    /// which message these rows are: the store's copy of it replaces them
    pub message: String,
    /// where the message's rows begin in the store
    pub start: usize,
    /// how many rows it owns right now
    pub rows: usize,
    pub parts: Vec<Part>,
}

impl Live {
    #[must_use]
    pub fn new(start: usize, message: String) -> Self {
        Self {
            message,
            start,
            rows: 0,
            parts: Vec::new(),
        }
    }

    pub fn push_text(&mut self, delta: &str) {
        match self.parts.last_mut() {
            Some(Part::Text(text)) => text.push_str(delta),
            _ => self.parts.push(Part::Text(delta.to_string())),
        }
    }

    pub fn push_thinking(&mut self, delta: &str) {
        match self.parts.last_mut() {
            Some(Part::Thinking(text)) => text.push_str(delta),
            _ => self.parts.push(Part::Thinking(delta.to_string())),
        }
    }

    /// The frame's own copy of the prose wins when it carries one: a delta can
    /// be missed, the message cannot.
    pub fn set_text(&mut self, text: &str) {
        match self
            .parts
            .iter_mut()
            .rev()
            .find_map(|part| match part {
                Part::Text(existing) => Some(existing),
                _ => None,
            }) {
            Some(existing) => *existing = text.to_string(),
            None => self.parts.push(Part::Text(text.to_string())),
        }
    }

    pub fn start_tool(&mut self, id: &str, name: &str, args: Value) {
        if self
            .parts
            .iter()
            .any(|part| matches!(part, Part::Tool { id: open, .. } if open == id))
        {
            return;
        }
        self.parts.push(Part::Tool {
            id: id.to_string(),
            name: name.to_string(),
            args,
            output: None,
            failed: false,
            bash: None,
            duration: None,
        });
    }

    pub fn finish_tool(
        &mut self,
        id: &str,
        output: String,
        failed: bool,
        bash: Option<transcript::BashInfo>,
        duration: Option<u64>,
    ) {
        for part in self.parts.iter_mut().rev() {
            if let Part::Tool {
                id: open,
                output: slot,
                failed: flag,
                bash: bash_slot,
                duration: elapsed,
                ..
            } = part
            {
                if open == id {
                    *slot = Some(output);
                    *flag = failed;
                    *bash_slot = bash;
                    *elapsed = duration;
                    return;
                }
            }
        }
    }

    /// The rows for the message as it stands: prose, cells and all, in the order
    /// the parts arrived.
    #[must_use]
    pub fn rows(&self) -> Vec<Row> {
        let mut out: Vec<Row> = Vec::new();
        for part in &self.parts {
            match part {
                Part::Thinking(text) => out.extend(block_rows("thinking", text)),
                Part::Text(text) => out.extend(block_rows("text", text)),
                Part::Tool {
                    id,
                    name,
                    args,
                    output,
                    failed,
                    bash,
                    duration,
                } => {
                    let mut row = transcript::tool_cell(name, id, args);
                    if let Some(output) = output {
                        transcript::finish_cell(&mut row, output, *failed);
                    }
                    if let Some(info) = bash {
                        let code = args.get("code").and_then(Value::as_str).unwrap_or_default();
                        transcript::apply_header(&mut row, code, Some(info), *duration);
                    }
                    out.push(row);
                }
            }
        }
        for row in &mut out {
            row.message.clone_from(&self.message);
        }
        out
    }
}

/// One prose block, drawn by the same builder the finished transcript uses, so a
/// streaming code fence already looks like the code block it will become.
fn block_rows(kind: &str, text: &str) -> Vec<Row> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let message = serde_json::json!({
        "id": format!("live:{kind}"),
        "role": "assistant",
        "content": [{ "type": kind, kind: text }],
    });
    transcript::build(std::slice::from_ref(&message), 1).0
}
