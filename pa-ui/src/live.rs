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
    pub timestamp: String,
    pub parts: Vec<Part>,
}

impl Live {
    #[must_use]
    pub fn new(message: String) -> Self {
        Self {
            message,
            timestamp: String::new(),
            parts: Vec::new(),
        }
    }

    pub fn set_timestamp(&mut self, timestamp: String) {
        self.timestamp = timestamp;
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

    pub fn start_tool(&mut self, id: &str, args: Value) {
        if self
            .parts
            .iter()
            .any(|part| matches!(part, Part::Tool { id: open, .. } if open == id))
        {
            return;
        }
        self.parts.push(Part::Tool {
            id: id.to_string(),
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
        for (idx, part) in self.parts.iter().enumerate() {
            match part {
                Part::Thinking(text) => {
                    let mut rows = block_rows("thinking", text);
                    let trailing = !self.parts[idx + 1..].iter().any(|later| match later {
                        Part::Tool { .. } => true,
                        Part::Text(t) | Part::Thinking(t) => !t.trim().is_empty(),
                    });
                    if trailing {
                        for row in &mut rows {
                            row.running = true;
                        }
                    }
                    out.extend(rows);
                }
                Part::Text(text) => out.extend(block_rows("text", text)),
                Part::Tool {
                    id,
                    args,
                    output,
                    failed,
                    bash,
                    duration,
                } => {
                    let mut row = transcript::tool_cell(id, args);
                    if let Some(output) = output {
                        transcript::finish_cell(&mut row, output, *failed);
                    } else {
                        row.running = true;
                    }
                    if bash.is_some() || duration.is_some() {
                        transcript::apply_header(
                            &mut row,
                            &transcript::tool_source(args),
                            bash.as_ref(),
                            *duration,
                        );
                    }
                    out.push(row);
                }
            }
        }
        for row in &mut out {
            row.message.clone_from(&self.message);
        }
        transcript::stamp_rows(&mut out, &self.timestamp);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn open_tool_call_is_running_and_finished_call_without_duration_is_settled() {
        let mut live = Live::new("m1".to_string());
        live.start_tool("c1", json!({ "code": "print(1)" }));
        let open = live.rows();
        assert_eq!(open.len(), 1);
        assert!(open[0].running);

        live.finish_tool("c1", "1".to_string(), false, None, None);
        let settled = live.rows();
        assert_eq!(settled.len(), 1);
        assert!(!settled[0].running);
        assert!(settled[0].elapsed.is_empty());
    }

    #[test]
    fn trailing_thinking_is_running_and_settles_when_followed() {
        let mut live = Live::new("m1".to_string());
        live.push_thinking("considering");
        let active = live.rows();
        assert_eq!(active.len(), 1);
        assert!(active[0].running);
        assert_eq!(active[0].title, "Thinking");

        live.push_text("answer");
        let with_text = live.rows();
        assert_eq!(with_text[0].kind, transcript::KIND_THINKING);
        assert!(!with_text[0].running);
        assert_eq!(with_text[0].title, "Thinking");
        assert!(!with_text[1].running);

        let mut live_with_tool = Live::new("m2".to_string());
        live_with_tool.push_thinking("planning");
        live_with_tool.start_tool("c2", json!({ "code": "ls" }));
        let rows = live_with_tool.rows();
        assert!(!rows[0].running);
        assert_eq!(rows[0].title, "Thinking");
        assert!(rows[1].running);
    }
}
