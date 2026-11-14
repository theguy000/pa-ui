//! The message that is still being written.
//!
//! One open message owns one contiguous range of rows, and every frame rebuilds
//! that whole range from the parts accumulated so far. Nothing is patched in
//! place and nothing is reconciled by id: there is exactly one representation of
//! the open message, so a second copy cannot exist to fall out of step with the
//! first. This is the shape the TUI uses - a single `streaming_index`, replaced
//! wholesale on each frame - and it is the only arrangement that has survived
//! contact with this wire. The rebuild stays cheap because a text part keeps the
//! blocks whose boundaries are final: a delta re-parses its own paragraph, not
//! the answer.

use std::cell::RefCell;

use serde_json::Value;

use crate::transcript::{self, Row};

enum Part {
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
    parts: Vec<Part>,
    /// One entry per part. A text part keeps the blocks that can no longer
    /// change, so a delta parses its own paragraph instead of the whole answer.
    cache: RefCell<Vec<PartCache>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PartKind {
    Text,
    Thinking,
}

impl PartKind {
    fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Thinking => "thinking",
        }
    }
}

/// The parsed side of one text part. `stable_rows` covers `text[..stable_len]`,
/// whose block boundaries are final; `tail_rows` covers the rest and is replaced
/// on every read.
#[derive(Default)]
struct PartCache {
    kind: Option<PartKind>,
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
                Part::Thinking(text) => out.extend(self.text_rows(index, PartKind::Thinking, text)),
                Part::Text(text) => out.extend(self.text_rows(index, PartKind::Text, text)),
                Part::Tool { .. } => out.extend(tool_row(part)),
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
    /// are exactly a full parse of `text` - a split only lands where a fence,
    /// quote or table begins or ends.
    fn text_rows(&self, index: usize, kind: PartKind, text: &str) -> Vec<Row> {
        let mut cache = self.cache.borrow_mut();
        if cache.len() <= index {
            cache.resize_with(index + 1, PartCache::default);
        }
        let part = &mut cache[index];
        if part.kind != Some(kind) || text.len() < part.parsed_len {
            *part = PartCache { kind: Some(kind), ..PartCache::default() };
        }
        let unchanged = part.parsed_len == text.len();
        if unchanged {
            let mut out = Vec::with_capacity(part.stable_rows.len() + part.tail_rows.len());
            out.extend(part.stable_rows.iter().cloned());
            out.extend(part.tail_rows.iter().cloned());
            return out;
        }
        if let Some(boundary) = stable_boundary(text, part.stable_len) {
            if boundary > part.stable_len {
                let chunk = &text[part.stable_len..boundary];
                let chunk = chunk.strip_suffix('\n').unwrap_or(chunk);
                part.stable_rows.extend(block_rows(kind.name(), chunk));
                part.stable_len = boundary;
            }
        }
        part.tail_rows = block_rows(kind.name(), &text[part.stable_len..]);
        part.parsed_len = text.len();
        let mut out = Vec::with_capacity(part.stable_rows.len() + part.tail_rows.len());
        out.extend(part.stable_rows.iter().cloned());
        out.extend(part.tail_rows.iter().cloned());
        out
    }
}

fn tool_row(part: &Part) -> Option<Row> {
    let Part::Tool {
        id,
        args,
        output,
        failed,
        bash,
        duration,
    } = part
    else {
        return None;
    };
    let mut row = transcript::tool_cell(id, args);
    if let Some(output) = output {
        transcript::finish_cell(&mut row, output, *failed);
    }
    if let Some(info) = bash {
        let code = args.get("code").and_then(Value::as_str).unwrap_or_default();
        transcript::apply_header(&mut row, code, Some(info), *duration);
    }
    Some(row)
}

/// The last position at which a new block starts in the transcript builder's
/// terms: a fence opener or closer, a quote's first line, a table's header, or
/// the blank line that ends a quote or a table. Prose is one block from one of
/// those to the next, so a blank line between paragraphs is not a boundary.
fn stable_boundary(text: &str, from: usize) -> Option<usize> {
    let mut in_fence = false;
    let mut in_table = false;
    let mut previous_quote = false;
    let mut cursor = from.min(text.len());
    let mut boundary = None;
    while cursor < text.len() {
        let end = text[cursor..]
            .find('\n')
            .map_or(text.len(), |offset| cursor + offset + 1);
        let line = &text[cursor..end];
        let trimmed = line.trim();
        if in_fence {
            if transcript::is_fence(line) {
                in_fence = false;
                boundary = Some(end);
            }
        } else if transcript::is_fence(line) {
            boundary = Some(cursor);
            in_fence = true;
        } else if transcript::is_quote(line) {
            if !previous_quote {
                boundary = Some(cursor);
            }
        } else if trimmed.is_empty() {
            if previous_quote || in_table {
                boundary = Some(cursor);
            }
            in_table = false;
        } else if transcript::starts_table(line, next_line(text, end).unwrap_or_default()) {
            if !in_table {
                boundary = Some(cursor);
            }
            in_table = true;
        } else if in_table {
            in_table = false;
        }
        previous_quote = transcript::is_quote(line);
        cursor = end;
    }
    boundary
}

fn next_line(text: &str, from: usize) -> Option<&str> {
    if from >= text.len() {
        return None;
    }
    let end = text[from..]
        .find('\n')
        .map_or(text.len(), |offset| from + offset);
    Some(&text[from..end])
}

#[cfg(test)]
thread_local! {
    static PARSED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_parsed_bytes() {
    PARSED_BYTES.with(|bytes| bytes.set(0));
}

#[cfg(test)]
fn parsed_bytes() -> usize {
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
            "> quoted\nlazy continuation\n\nnext paragraph".into(),
            "> quoted\n\n| a | b |\n| --- | --- |\n| 1 | 2 |".into(),
            "> quoted\n\nafter the quote".into(),
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
                Part::Tool { .. } => out.extend(tool_row(part)),
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
            if index > 0 {
                body.push_str("\n\n");
            }
            body.push_str(&format!(
                "Paragraph {index}: the store is the authority and the events are advisory, so a poll reconciles rather than trusts."
            ));
            if index % 5 == 4 {
                body.push_str("\n\n```rust\nfn step() {}\n```");
            }
        }
        body.push_str("\n\nFinal paragraph that keeps streaming");
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
        live.start_tool("call-1", serde_json::json!({"code": "cargo test"}));
        live.finish_tool("call-1", "ok".into(), false, None, Some(12));
        live.push_text("after the tool");
        live.push_thinking(" and more");
        let rows = live.rows();
        assert_eq!(rows, rows_full(&live));
        assert!(rows.len() > first.len());
    }
}
