//! The message that is still being written.
//!
//! One open message owns one contiguous range of rows, and every frame rebuilds
//! that whole range from the parts accumulated so far. Nothing is patched in
//! place and nothing is reconciled by id: there is exactly one representation of
//! the open message, so a second copy cannot exist to fall out of step with the
//! first. The rebuild stays cheap because a text part keeps the blocks whose
//! boundaries are final: a delta re-parses its own paragraph, not the answer. This is the shape the TUI uses - a single `streaming_index`, replaced
//! wholesale on each frame - and it is the only arrangement that has survived
//! contact with this wire.

use std::cell::RefCell;

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
    /// One entry per part. A text part keeps the blocks that can no longer
    /// change, so a delta parses its own paragraph instead of the whole answer.
    cache: RefCell<Vec<PartCache>>,
}

/// The parsed side of one text part. `stable_rows` covers `text[..stable_len]`,
/// whose block boundaries are final; `tail_rows` covers the rest and is replaced
/// on every read.
#[derive(Default)]
struct PartCache {
    kind: u8,
    parsed_len: usize,
    stable_len: usize,
    stable_rows: Vec<Row>,
    tail_rows: Vec<Row>,
}

impl Live {
    #[must_use]
    pub fn new(message: String) -> Self {
        Self {
            message,
            timestamp: String::new(),
            parts: Vec::new(),
            cache: RefCell::new(Vec::new()),
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
        for (index, part) in self.parts.iter().enumerate() {
            match part {
                Part::Thinking(text) => out.extend(self.text_rows(index, 2, "thinking", text)),
                Part::Text(text) => out.extend(self.text_rows(index, 1, "text", text)),
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
        transcript::stamp_rows(&mut out, &self.timestamp);
        out
    }

    /// A text part's rows: the blocks whose boundaries are final stay parsed,
    /// and only the trailing unfinished block is built again. The returned rows
    /// are exactly a full parse of `text` - the boundary rule keeps a list or a
    /// fence from being cut in half.
    fn text_rows(&self, index: usize, kind: u8, kind_name: &str, text: &str) -> Vec<Row> {
        let mut cache = self.cache.borrow_mut();
        if cache.len() <= index {
            cache.resize_with(index + 1, PartCache::default);
        }
        let part = &mut cache[index];
        if part.kind != kind || text.len() < part.parsed_len {
            *part = PartCache { kind, ..PartCache::default() };
        }
        if let Some(boundary) = stable_boundary(text, part.stable_len) {
            if boundary > part.stable_len {
                part.stable_rows
                    .extend(block_rows(kind_name, &text[part.stable_len..boundary]));
                part.stable_len = boundary;
            }
        }
        part.tail_rows = block_rows(kind_name, &text[part.stable_len..]);
        part.parsed_len = text.len();
        let mut out = Vec::with_capacity(part.stable_rows.len() + part.tail_rows.len());
        out.extend(part.stable_rows.iter().cloned());
        out.extend(part.tail_rows.iter().cloned());
        out
    }
}

/// The last line break after which a block is certainly finished: a blank line
/// outside a fence, not followed by a list item, an indented line or a rule,
/// and not preceded by a list item (a loose list can span blank lines).
fn stable_boundary(text: &str, from: usize) -> Option<usize> {
    let mut in_fence = false;
    let mut cursor = from.min(text.len());
    let mut boundary = None;
    let mut previous: Option<&str> = None;
    while cursor < text.len() {
        let end = text[cursor..]
            .find('\n')
            .map_or(text.len(), |offset| cursor + offset + 1);
        let line = &text[cursor..end];
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_fence = !in_fence;
            previous = Some(trimmed);
        } else if trimmed.is_empty() {
            if !in_fence {
                let next = next_nonempty_line(text, end);
                if next.is_some_and(|next| !joins_block(previous, next)) {
                    boundary = Some(end);
                }
            }
        } else {
            previous = Some(trimmed);
        }
        cursor = end;
    }
    boundary
}

fn next_nonempty_line(text: &str, from: usize) -> Option<&str> {
    let mut cursor = from.min(text.len());
    while cursor < text.len() {
        let end = text[cursor..]
            .find('\n')
            .map_or(text.len(), |offset| cursor + offset + 1);
        let line = text[cursor..end].trim();
        if !line.is_empty() {
            return Some(line);
        }
        cursor = end;
    }
    None
}

/// Whether a blank line here is inside a construct: the next line continues the
/// previous block, or the previous line could still absorb it.
fn joins_block(previous: Option<&str>, next: &str) -> bool {
    let starts_list = |line: &str| {
        line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("+ ")
            || line.starts_with('>')
            || {
                let digits = line.chars().take_while(char::is_ascii_digit).count();
                digits > 0 && line[digits..].starts_with(". ")
            }
    };
    let next_indented = next.starts_with(' ') || next.starts_with('\t');
    let next_is_rule = next.len() >= 3
        && next
            .chars()
            .all(|ch| matches!(ch, '-' | '*' | '_' | '=' | ' '));
    next_indented
        || next_is_rule
        || next.starts_with('#')
        || next.starts_with("```")
        || starts_list(next)
        || previous.is_some_and(starts_list)
}

#[cfg(test)]
thread_local! {
    static PARSED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn reset_parsed_bytes() {
    PARSED_BYTES.with(|bytes| bytes.set(0));
}

#[cfg(test)]
pub(crate) fn parsed_bytes() -> usize {
    PARSED_BYTES.with(std::cell::Cell::get)
}

/// One prose block, drawn by the same builder the finished transcript uses, so a
/// streaming code fence already looks like the code block it will become.
fn block_rows(kind: &str, text: &str) -> Vec<Row> {
    #[cfg(test)]
    PARSED_BYTES.with(|bytes| bytes.set(bytes.get() + text.len()));
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

    fn corpus() -> Vec<String> {
        vec![
            "One paragraph.\n\nTwo paragraphs, with **bold** and `code`.\n\nThree.".into(),
            "before\n\n```rust\nfn a() {}\n\nfn b() {}\n```\n\nafter".into(),
            "- one\n\n- two\n\n- three\n\nafter the list".into(),
            "- one\n- two\n\nafter the list".into(),
            "> quoted\n> more\n\nafter\n\n> second quote".into(),
            "| a | b |\n| --- | --- |\n| 1 | 2 |\n\nafter the table".into(),
            "# Head\n\nbody\n\n## Sub\n\nmore body".into(),
            "before\n\n```rust\nfn a() {}\n\nstill inside the fence".into(),
            "para\n\n---\n\nafter the rule".into(),
            "1. first\n\n2. second\n\n3. third\n\nclosing prose".into(),
            "text with a hard  \nbreak and a trailing space \n\nnext".into(),
        ]
    }

    /// The rows a full parse of the accumulated parts produces, without the
    /// incremental cache.
    fn rows_full(live: &Live) -> Vec<Row> {
        let mut out: Vec<Row> = Vec::new();
        for part in &live.parts {
            match part {
                Part::Thinking(text) => out.extend(block_rows("thinking", text)),
                Part::Text(text) => out.extend(block_rows("text", text)),
                Part::Tool { .. } => {}
            }
        }
        for row in &mut out {
            row.message.clone_from(&live.message);
        }
        transcript::stamp_rows(&mut out, &live.timestamp);
        out
    }

    #[test]
    fn incremental_rows_equal_a_full_parse_at_every_prefix() {
        for text in corpus() {
            let mut live = Live::new("m".into());
            live.set_timestamp("12:00".into());
            let mut cursor = 0;
            let mut step = 1;
            while cursor < text.len() {
                let mut end = (cursor + step).min(text.len());
                while end < text.len() && !text.is_char_boundary(end) {
                    end += 1;
                }
                live.push_text(&text[cursor..end]);
                cursor = end;
                step = (step % 7) + 1;
                assert_eq!(
                    live.rows(),
                    rows_full(&live),
                    "prefix {} of {text:?}",
                    cursor
                );
            }
        }
    }

    #[test]
    fn a_delta_parses_only_the_trailing_block() {
        let mut body = String::new();
        for index in 0..40 {
            body.push_str(&format!(
                "Paragraph {index}: the store is the authority and the events are advisory, so a poll reconciles rather than trusts.\n\n"
            ));
        }
        let mut live = Live::new("m".into());
        live.push_text(&body);
        let before = live.rows();
        assert!(!before.is_empty());

        reset_parsed_bytes();
        live.push_text("one more delta");
        let after = live.rows();
        let parsed = parsed_bytes();
        assert!(
            parsed < 400,
            "a delta on a {} byte answer parsed {parsed} bytes",
            body.len()
        );
        assert_eq!(after, rows_full(&live));
        assert_eq!(after.len(), before.len());
    }

    #[test]
    fn thinking_parts_track_their_own_cache() {
        let mut live = Live::new("m".into());
        live.push_thinking("first thought\n\nsecond thought");
        let first = live.rows();
        live.push_text("answer");
        live.push_thinking(" and more");
        let rows = live.rows();
        assert_eq!(rows, rows_full(&live));
        assert!(rows.len() > first.len());
    }
}
