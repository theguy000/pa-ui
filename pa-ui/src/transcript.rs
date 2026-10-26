//! Session messages -> transcript rows.
//!
//! Shapes straight off the wire (verified against a live attach):
//!   {role:"user",  content:[{type:"text", text}]}
//!   {role:"assistant", content:[{type:"thinking"|"text"|"toolCall", ...}]}
//!   {role:"toolResult", toolCallId, toolName, content:[{text}]}
//!   {customType, content, display}

use serde_json::Value;
use slint::StyledText;

pub const KIND_USER: i32 = 0;
pub const KIND_AGENT: i32 = 1;
pub const KIND_THINKING: i32 = 2;
pub const KIND_CELL: i32 = 3;
pub const KIND_NOTICE: i32 = 5;

#[derive(Debug, Clone)]
pub struct Row {
    pub kind: i32,
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
}

impl Row {
    fn new(kind: i32) -> Self {
        Self {
            kind,
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
        }
    }
}

/// Build the transcript rows for the newest `limit` messages.
#[must_use]
pub fn build(messages: &[Value], limit: usize) -> (Vec<Row>, usize) {
    let start = messages.len().saturating_sub(limit);
    let window = &messages[start..];

    // tool outputs keyed by call id, so a cell carries its own result
    let mut outputs: std::collections::HashMap<String, (String, bool)> = std::collections::HashMap::new();
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
    }

    let mut rows = Vec::new();
    for message in window {
        let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
        if role == "toolResult" {
            continue; // folded into its cell
        }
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
            continue;
        }

        let Some(blocks) = message.get("content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            match block.get("type").and_then(Value::as_str).unwrap_or_default() {
                "text" => {
                    let text = block.get("text").and_then(Value::as_str).unwrap_or_default();
                    if text.trim().is_empty() {
                        continue;
                    }
                    let mut row = Row::new(if role == "user" { KIND_USER } else { KIND_AGENT });
                    row.body = markdown_markup(text);
                    rows.push(row);
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
                    rows.push(row);
                }
                _ => {}
            }
        }
    }
    (rows, start)
}

/// The last meaningful line of a failed output — usually the exception itself.
fn failure_reason(output: &str, failed: bool) -> String {
    if !failed {
        return String::new();
    }
    let line = output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("failed");
    line.chars().take(160).collect()
}

fn short_tool(name: &str) -> &str {
    match name {
        "python_repl" => "python",
        other => other,
    }
}

fn summary_line(code: &str) -> String {
    code.lines()
        .find(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| line.trim().trim_end_matches(':').to_string())
        .unwrap_or_else(|| "empty cell".to_string())
}

// ── markup ──────────────────────────────────────────────────────────

pub fn styled(markup: &str) -> StyledText {
    match StyledText::from_markdown(markup) {
        Ok(text) => text,
        Err(error) => {
            if std::env::var("PA_DEBUG").is_ok() {
                eprintln!("markup error: {error}\n--- input ---\n{markup}\n-------------");
            }
            StyledText::from_plain_text(markup)
        }
    }
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '`' => out.push_str("\\`"),
            '*' => out.push_str("\\*"),
            '_' => out.push_str("\\_"),
            '[' => out.push_str("\\["),
            ']' => out.push_str("\\]"),
            '\\' => out.push_str("\\\\"),
            other => out.push(other),
        }
    }
}

/// Plain text rendered verbatim (thinking, output, notices).
pub fn plain_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    escape(text, &mut out);
    guard_block_markers(&out)
}

/// Assistant markdown stays markdown, minus the blocks Slint cannot parse:
/// headings become bold lines, rules become blank lines, quotes lose their `>`.
pub fn markdown_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let body = trimmed.trim_start_matches('#').trim_start();
            out.push_str("**");
            out.push_str(body);
            out.push_str("**");
        } else if trimmed.len() >= 3 && trimmed.chars().all(|ch| ch == '-' || ch == '*' || ch == '_' || ch == ' ') {
            // horizontal rule: nothing to draw
        } else if let Some(rest) = trimmed.strip_prefix("> ") {
            out.push('›');
            out.push(' ');
            out.push_str(rest);
        } else {
            out.push_str(line);
        }
    }
    out
}

const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "False", "try", "while",
    "with", "yield",
];

fn wrap(color: &str, text: &str, out: &mut String) {
    out.push_str("<font color=\"");
    out.push_str(color);
    out.push_str("\">");
    escape(text, out);
    out.push_str("</font>");
}

/// Small python tokenizer -> coloured markup. Not a parser: keywords, strings,
/// comments and numbers, which is what makes a cell scannable.
pub fn highlight_python(code: &str) -> String {
    const KEY: &str = "#c9a86a";
    const STR: &str = "#8fb573";
    const COM: &str = "#5f6159";
    const NUM: &str = "#d0a75c";

    let mut out = String::with_capacity(code.len() * 2);
    let mut chars = code.chars().peekable();
    let mut word = String::new();
    let mut flush_word = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        if KEYWORDS.contains(&word.as_str()) {
            wrap(KEY, word, out);
        } else {
            escape(word, out);
        }
        word.clear();
    };

    while let Some(ch) = chars.next() {
        match ch {
            quote @ ('\'' | '"') => {
                flush_word(&mut word, &mut out);
                // triple-quoted block strings swallow newlines and would otherwise
                // reach the markdown parser as code fences / headings
                let mut literal = String::new();
                literal.push(quote);
                let triple = {
                    let mut probe = chars.clone();
                    probe.next() == Some(quote) && probe.next() == Some(quote)
                };
                if triple {
                    chars.next();
                    chars.next();
                    literal.push(quote);
                    literal.push(quote);
                    let mut run = 0;
                    while let Some(next) = chars.next() {
                        literal.push(next);
                        if next == quote {
                            run += 1;
                            if run == 3 {
                                break;
                            }
                        } else {
                            run = 0;
                        }
                    }
                } else {
                    while let Some(next) = chars.next() {
                        literal.push(next);
                        if next == '\\' {
                            if let Some(escaped) = chars.next() {
                                literal.push(escaped);
                            }
                            continue;
                        }
                        if next == quote {
                            break;
                        }
                    }
                }
                wrap(STR, &literal, &mut out);
            }
            '#' => {
                flush_word(&mut word, &mut out);
                let mut comment = String::from("#");
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                    comment.push(next);
                }
                wrap(COM, &comment, &mut out);
                out.push('\n');
            }
            ch if ch.is_alphanumeric() || ch == '_' => word.push(ch),
            ch => {
                flush_word(&mut word, &mut out);
                if ch.is_ascii_digit() {
                    let mut number = ch.to_string();
                    while let Some(next) = chars.peek() {
                        if next.is_ascii_digit() || *next == '.' {
                            number.push(chars.next().unwrap_or_default());
                        } else {
                            break;
                        }
                    }
                    wrap(NUM, &number, &mut out);
                } else {
                    escape(&ch.to_string(), &mut out);
                }
            }
        }
    }
    flush_word(&mut word, &mut out);
    guard_block_markers(&out)
}

/// CommonMark reads a 4-space indent as a code block and a leading `#`/`-`/`>`
/// as a heading/list/quote — both unsupported in Slint. Leading spaces become
/// NBSP (same advance width, no block syntax) and marker lines get a zero-width
/// space in front.
fn guard_block_markers(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len() + 32);
    for (index, line) in markup.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        for _ in 0..indent {
            out.push('\u{00a0}');
        }
        let rest = &line[indent..];
        let first = rest.chars().next();
        if matches!(first, Some('#' | '>' | '-' | '+' | '*' | '=' | '~' | '|' | '0'..='9')) {
            out.push('\u{200b}');
        }
        out.push_str(rest);
    }
    out
}

/// Two synthetic rows (clean + failed) so the failure affordance can be seen
/// without waiting for a real traceback.
#[must_use]
pub fn demo_rows() -> Vec<Row> {
    let mut ok = Row::new(KIND_CELL);
    ok.title = "python · print(1 + 1)".to_string();
    ok.meta = "1 line".to_string();
    ok.code = highlight_python("print(1 + 1)");
    ok.output = plain_markup("2");
    ok.has_code = true;
    ok.has_output = true;

    let mut failed = Row::new(KIND_CELL);
    failed.title = "python · rows = [r for r in data]".to_string();
    failed.meta = "2 lines".to_string();
    failed.code = highlight_python("rows = [r for r in data]\nprint(rows[3][\"name\"])\n");
    failed.output = plain_markup(
        "Traceback (most recent call last):\n  File \"<cell-9>\", line 2, in <module>\n    print(rows[3][\"name\"])\n          ~~~~^^^\nKeyError: 'name'",
    );
    failed.has_code = true;
    failed.has_output = true;
    failed.failed = true;
    failed.reason = "KeyError: 'name'".to_string();
    vec![ok, failed]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cell_and_result(output: &str) -> Vec<Row> {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": [{
                    "type": "toolCall",
                    "id": "call_1",
                    "name": "python_repl",
                    "arguments": {"code": "print(rows[3][\"name\"])"}
                }]
            }),
            json!({
                "role": "toolResult",
                "toolCallId": "call_1",
                "toolName": "python_repl",
                "content": [{"text": output}]
            }),
        ];
        build(&messages, 300).0
    }

    #[test]
    fn traceback_marks_the_cell_failed_with_its_reason() {
        let rows = cell_and_result(
            "Traceback (most recent call last):\n  File \"<cell-3>\", line 1\n    raise KeyError\nKeyError: 'name'",
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].failed);
        assert_eq!(rows[0].reason, "KeyError: 'name'");
    }

    #[test]
    fn clean_output_is_not_a_failure() {
        let rows = cell_and_result("2");
        assert!(!rows[0].failed);
        assert!(rows[0].reason.is_empty());
    }

    #[test]
    fn code_markup_survives_the_parser() {
        let markup = highlight_python("def f():\n    # four space indent\n    return 'x'");
        assert!(StyledText::from_markdown(&markup).is_ok(), "parser rejected: {markup}");
    }
}
