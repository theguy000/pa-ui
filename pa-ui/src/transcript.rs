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
/// A fenced code block, rendered on its own surface rather than as prose.
pub const KIND_CODE: i32 = 6;
/// A GFM table, drawn with box characters in a monospace face.
pub const KIND_TABLE: i32 = 7;
/// An image the user sent: drawn as a thumbnail, not as words.
pub const KIND_IMAGE: i32 = 8;

#[derive(Debug, Clone)]
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
                    rows.push(row);
                }
                _ => {}
            }
        }
    }
    (rows, start)
}

/// A cell row for a tool call that has just started, shaped exactly like the one
/// the store will produce for it so the reconcile at turn end changes nothing
/// on screen.
#[must_use]
pub fn tool_cell(tool_name: &str, call_id: &str, args: &Value) -> Row {
    let code = args.get("code").and_then(Value::as_str).unwrap_or_default();
    let mut row = Row::new(KIND_CELL);
    row.id = call_id.to_string();
    let fallback = args.to_string();
    let summary = summary_line(if code.is_empty() { &fallback } else { code });
    row.title = format!("{} · {}", short_tool(tool_name), summary);
    row.code = highlight_python(code);
    row.has_code = !code.is_empty();
    let lines = code.lines().count();
    row.meta = if code.is_empty() {
        "running".to_string()
    } else if lines == 1 {
        "1 line".to_string()
    } else {
        format!("{lines} lines")
    };
    row
}

/// Fill in a running cell's result, including the failure line an errored call
/// carries so the cell can show why it failed instead of just sitting there.
pub fn finish_cell(row: &mut Row, output: &str, failed: bool) {
    row.output = plain_markup(output);
    row.has_output = !output.trim().is_empty();
    row.failed = failed;
    row.reason = failure_reason(output, failed);
    if row.code.is_empty() {
        row.meta = if failed { "failed".to_string() } else { "done".to_string() };
    }
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

/// Text as the layout engine wants it: no non-breaking or thin spaces.
///
/// Slint lays text out through parley, and parley panics outright when an elide
/// range lands inside a multi-byte character — `end byte index 7 is not a char
/// boundary; it is inside '\u{a0}'`. A U+00A0 that reaches a narrow elided
/// label is therefore enough to kill the window. A transcript has no use for a
/// space that refuses to break, so they are folded to plain spaces.
#[must_use]
pub fn plain(text: &str) -> String {
    if !text
        .chars()
        .any(|ch| matches!(ch, '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{2007}'))
    {
        return text.to_string();
    }
    text.chars()
        .map(|ch| match ch {
            '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{2007}' => ' ',
            other => other,
        })
        .collect()
}

pub fn styled(markup: &str) -> StyledText {
    let markup = &plain(markup);
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

/// Undo the HTML entities that arrive in pasted source (`&#x20;`, `&amp;`).
///
/// Terminals and chat surfaces agree on these; leaving them encoded shows
/// `&#x20;` where a space belongs.
#[must_use]
pub fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'&' {
            let next = text[index..].chars().next().unwrap_or(' ');
            out.push(next);
            index += next.len_utf8();
            continue;
        }
        let Some(semi) = text[index..].find(';').map(|offset| index + offset) else {
            out.push('&');
            index += 1;
            continue;
        };
        let body = &text[index + 1..semi];
        let decoded = if let Some(hex) = body.strip_prefix("#x").or_else(|| body.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = body.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            match body {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                // folded, not preserved: U+00A0 crashes the layout engine
                "nbsp" => Some(' '),
                _ => None,
            }
        };
        match decoded {
            Some(ch) => {
                out.push(ch);
                index = semi + 1;
            }
            None => {
                out.push('&');
                index += 1;
            }
        }
    }
    out
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
    let decoded = decode_entities(text);
    let mut out = String::with_capacity(decoded.len() + 16);
    escape(&decoded, &mut out);
    guard_block_markers(&out)
}

/// Assistant markdown stays markdown, minus the blocks Slint cannot parse:
/// headings become bold lines, rules become blank lines, quotes lose their `>`.
/// How many `**` markers a line carries, ignoring escapes and code spans.
///
/// Each marker toggles bold, so an odd count means the line leaves bold open.
fn strong_markers(line: &str) -> usize {
    let bytes: Vec<char> = line.chars().collect();
    let mut count = 0;
    let mut in_code = false;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index];
        if escaped {
            escaped = false;
            index += 1;
            continue;
        }
        match ch {
            '\\' => {
                escaped = true;
                index += 1;
            }
            '`' => {
                in_code = !in_code;
                index += 1;
            }
            '*' if !in_code && index + 1 < bytes.len() && bytes[index + 1] == '*' => {
                count += 1;
                index += 2;
            }
            _ => index += 1,
        }
    }
    count
}

/// A `**span**` that runs over a line break is not something the Slint text
/// parser accepts, so it printed the asterisks instead of the weight. Each line
/// is rebalanced here: a span still open at the end of a line is closed there
/// and reopened at the start of the next one.
fn balance_strong(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut open = false;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if open {
            out.push_str("**");
        }
        out.push_str(line);
        // the markers we just added do not count: parity of the line decides
        if strong_markers(line) % 2 == 1 {
            open = !open;
        }
        if open {
            out.push_str("**");
        }
    }
    out
}

pub fn markdown_markup(text: &str) -> String {
    let decoded = decode_entities(text);
    let text = decoded.as_str();
    let mut out = String::with_capacity(text.len() + 16);
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            // headings carry the accent, the way the TUI colours mdHeading, and
            // bold, because there is no size step inside one text run
            let body = trimmed.trim_start_matches('#').trim_start();
            out.push_str("<font color=\"#c8b158\">**");
            out.push_str(body);
            out.push_str("**</font>");
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
    balance_strong(&out)
}

/// One piece of a message: the prose between the blocks, a fenced block, or a
/// table. Slint's markdown renderer handles emphasis, but it has no notion of a
/// code surface or a table grid, so those are lifted out and drawn here.
enum Block {
    Prose(String),
    Code { lang: String, body: String },
    Table(String),
}

/// Split one table row into cells on unescaped pipes, dropping the empty edge
/// cells a leading or trailing pipe leaves behind.
fn split_table_row(line: &str) -> Vec<String> {
    let mut cells: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for ch in line.chars() {
        match ch {
            '\\' => {
                escaped = !escaped;
                cell.push(ch);
            }
            '|' if !escaped => {
                cells.push(std::mem::take(&mut cell));
            }
            _ => {
                escaped = false;
                cell.push(ch);
            }
        }
    }
    cells.push(cell);
    if cells.len() > 1 && cells.first().is_some_and(|edge| edge.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.len() > 1 && cells.last().is_some_and(|edge| edge.trim().is_empty()) {
        cells.pop();
    }
    cells
        .into_iter()
        .map(|cell| cell.trim().replace("\\|", "|"))
        .collect()
}

/// The `|---|:--:|` row that turns the line above it into a table header.
fn is_delimiter_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('-') {
        return false;
    }
    let cells = split_table_row(trimmed);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let cell = cell.trim();
            let core = cell.trim_start_matches(':').trim_end_matches(':');
            !core.is_empty()
                && core.chars().all(|ch| ch == '-')
                && cell.chars().all(|ch| ch == '-' || ch == ':')
        })
}

/// `:--` left, `--:` right, `:-:` centred, as GFM reads them.
fn table_alignment(cell: &str) -> u8 {
    let cell = cell.trim();
    let left = cell.starts_with(':');
    let right = cell.ends_with(':') && cell.len() > 1;
    match (left, right) {
        (true, true) => 2,
        (true, false) => 0,
        (false, true) => 1,
        (false, false) => 0,
    }
}

fn truncate_to(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    let mut out: String = text.chars().take(width.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Draw a pipe table the way the TUI does: a header band, a rule, the rows, and
/// a box around the whole thing. It is text, not a widget, so it stays copyable
/// and needs no per-cell model.
fn render_table(raw: &[&str]) -> Option<String> {
    if raw.len() < 2 {
        return None;
    }
    let header = split_table_row(raw[0]);
    let aligns: Vec<u8> = split_table_row(raw[1]).iter().map(|cell| table_alignment(cell)).collect();
    if header.is_empty() || aligns.len() != header.len() {
        return None;
    }
    let columns = header.len();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in &raw[2..] {
        let mut cells = split_table_row(line);
        cells.resize(columns, String::new());
        cells.truncate(columns);
        rows.push(cells);
    }
    let mut widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .chain(std::iter::once(header[column].chars().count()))
                .max()
                .unwrap_or(3)
                .max(3)
        })
        .collect();
    // a table wider than the pane would wrap and lose its shape: give up the
    // widest column first, and elide whatever no longer fits
    let total = |widths: &[usize]| widths.iter().sum::<usize>() + 3 * columns + 1;
    while total(&widths) > 92 {
        let Some((widest, _)) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > 3)
            .max_by_key(|(_, width)| **width)
        else {
            break;
        };
        widths[widest] -= 1;
    }
    let pad = |text: &str, width: usize, align: u8| {
        let text = truncate_to(text, width);
        let fill = width.saturating_sub(text.chars().count());
        match align {
            1 => format!("{}{}", " ".repeat(fill), text),
            2 => {
                let left = fill / 2;
                format!("{}{}{}", " ".repeat(left), text, " ".repeat(fill - left))
            }
            _ => format!("{}{}", text, " ".repeat(fill)),
        }
    };
    let rule = |left: &str, mid: &str, right: &str| {
        let parts: Vec<String> = widths.iter().map(|width| "─".repeat(width + 2)).collect();
        format!("{left}{}{right}", parts.join(mid))
    };
    let mut out: Vec<String> = Vec::new();
    out.push(rule("┌", "┬", "┐"));
    let head: Vec<String> = (0..columns)
        .map(|column| pad(&header[column], widths[column], aligns[column]))
        .collect();
    out.push(format!("│ {} │", head.join(" │ ")));
    out.push(rule("├", "┼", "┤"));
    for row in &rows {
        let cells: Vec<String> = (0..columns)
            .map(|column| pad(&row[column], widths[column], aligns[column]))
            .collect();
        out.push(format!("│ {} │", cells.join(" │ ")));
    }
    out.push(rule("└", "┴", "┘"));
    Some(out.join("\n"))
}

fn flush_prose(prose: &mut Vec<String>, out: &mut Vec<Block>) {
    if prose.iter().any(|line| !line.trim().is_empty()) {
        out.push(Block::Prose(prose.join("\n")));
    }
    prose.clear();
}

/// Carve a message into prose, fenced blocks and tables.
fn split_blocks(text: &str) -> Vec<Block> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<Block> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            flush_prose(&mut prose, &mut out);
            let lang = rest.trim().to_string();
            let mut body: Vec<&str> = Vec::new();
            index += 1;
            while index < lines.len() && !lines[index].trim_start().starts_with("```") {
                body.push(lines[index]);
                index += 1;
            }
            index += 1; // the closing fence, when there is one
            out.push(Block::Code {
                lang,
                body: body.join("\n"),
            });
            continue;
        }
        if trimmed.contains('|') && index + 1 < lines.len() && is_delimiter_row(lines[index + 1]) {
            flush_prose(&mut prose, &mut out);
            let mut raw: Vec<&str> = vec![line, lines[index + 1]];
            index += 2;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && lines[index].contains('|')
            {
                raw.push(lines[index]);
                index += 1;
            }
            match render_table(&raw) {
                Some(rendered) => out.push(Block::Table(rendered)),
                // not a table after all: show the source, never swallow it
                None => prose.extend(raw.iter().map(|line| (*line).to_string())),
            }
            continue;
        }
        prose.push(line.to_string());
        index += 1;
    }
    flush_prose(&mut prose, &mut out);
    out
}

/// Highlight a fenced block. Only the python tokenizer exists; anything else is
/// shown verbatim rather than guessed at.
#[must_use]
pub fn highlight_code(lang: &str, code: &str) -> String {
    let language = lang
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(language.as_str(), "" | "py" | "python" | "python3") {
        highlight_python(code)
    } else {
        plain_markup(code)
    }
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
    let decoded = decode_entities(code);
    let code = decoded.as_str();
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


/// A scratch message covering every block and inline style the transcript can
/// draw - `PA_DEMO=markdown` splices it in so the rendering can be looked at
/// without writing a message into a real session.
#[must_use]
pub fn markdown_demo_rows() -> Vec<Row> {
    let text = "## receipts and throughput\n\n\
        Some **bold**, some *italic*, a word of `inline code`, and a [link](https://example.com).\n\
        A second line of the same paragraph, long enough to reach the wrap and show\n\
        how the prose column ends.\n\n\
        ```python\n\
        def total(rows: list[dict]) -> float:\n    \"\"\"Sum the priced rows.\"\"\"\n    \
        return sum(row[\"cost\"] for row in rows if row.get(\"cost\"))\n\n\
        # a comment, a string, and a number\n\
        rate = 0.0025 * 1450\n\
        print(f\"total: {total([]):.2f}\", rate)\n\
        ```\n\n\
        | model | in | out | cost |\n\
        |:--|--:|--:|--:|\n\
        | deepseek-v4.1-flash | 462k | 39k | $0.00 |\n\
        | claude-4.5-sonnet | 12.4k | 2.1k | $0.19 |\n\n\
        A closing paragraph under the table, so the spacing on both edges shows.\n";
    let message = serde_json::json!({
        "role": "assistant",
        "timestamp": 0,
        "content": [{ "type": "text", "text": text }],
    });
    build(std::slice::from_ref(&message), 1).0
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
    fn pasted_entities_are_decoded_not_shown() {
        assert_eq!(decode_entities("&#x20;a &amp; b"), " a & b");
        assert_eq!(decode_entities("caf&#233;"), "café");
        assert!(!markdown_markup("&#x20;    x").contains("&#x20;"));
        assert!(!plain_markup("a&nbsp;b").contains("&nbsp;"));
    }

    #[test]
    fn code_markup_survives_the_parser() {
        let markup = highlight_python("def f():\n    # four space indent\n    return 'x'");
        assert!(StyledText::from_markdown(&markup).is_ok(), "parser rejected: {markup}");
    }
}

#[cfg(test)]
mod strong_tests {
    use super::*;

    #[test]
    fn a_span_across_a_line_break_is_closed_and_reopened() {
        assert_eq!(
            balance_strong("one **two\nthree** four"),
            "one **two**\n**three** four"
        );
    }


    #[test]
    fn a_message_splits_into_prose_code_and_table() {
        let text = "before\n\n```python\nprint(1)\n```\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\nafter";
        let blocks = split_blocks(text);
        assert_eq!(blocks.len(), 4, "prose, code, table, prose");
        assert!(matches!(blocks[0], Block::Prose(_)));
        assert!(matches!(blocks[1], Block::Code { .. }));
        assert!(matches!(blocks[2], Block::Table(_)));
        assert!(matches!(blocks[3], Block::Prose(_)));
    }

    #[test]
    fn an_unclosed_fence_still_makes_a_code_row() {
        let blocks = split_blocks("look:\n```rust\nfn main() {}");
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            Block::Code { lang, body } => {
                assert_eq!(lang, "rust");
                assert_eq!(body, "fn main() {}");
            }
            other => panic!("expected code, got {}", matches!(other, Block::Prose(_))),
        }
    }

    #[test]
    fn a_pipe_line_without_a_delimiter_row_stays_prose() {
        let blocks = split_blocks("a | b\nnot a table");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::Prose(_)));
    }

    #[test]
    fn tables_pad_columns_and_honour_right_alignment() {
        let table = render_table(&["| model | cost |", "|:--|--:|", "| a | $1.50 |", "| longer | $2 |"])
            .expect("a valid table");
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 6, "top, header, rule, two rows, bottom");
        assert!(lines[0].starts_with('\u{250c}') && lines[0].ends_with('\u{2510}'));
        assert!(lines[1].contains("\u{2502} model"));
        // right aligned: the shorter price keeps its padding on the left
        assert!(lines[4].contains("  $2 \u{2502}"), "{:?}", lines[4]);
        assert!(lines.iter().all(|line| line.chars().count() == lines[0].chars().count()));
    }

    #[test]
    fn a_wide_table_gives_up_width_instead_of_wrapping() {
        let wide = "x".repeat(60);
        let table = render_table(&[
            &format!("| {wide} | {wide} | {wide} |"),
            "|---|---|---|",
            &format!("| {wide} | {wide} | {wide} |"),
        ])
        .expect("a valid table");
        let width = table.lines().next().map_or(0, |line| line.chars().count());
        assert!(width <= 92, "table stayed {width} wide");
        assert!(table.contains('\u{2026}'), "overflow elides, it does not wrap");
    }

    #[test]
    fn balanced_lines_are_left_alone() {
        assert_eq!(balance_strong("**a** and **b**"), "**a** and **b**");
        assert_eq!(balance_strong("plain"), "plain");
    }

    #[test]
    fn an_odd_line_closes_where_it_ends() {
        assert_eq!(balance_strong("a **b\nc"), "a **b**\n**c**");
    }

    #[test]
    fn code_spans_and_escapes_do_not_count() {
        assert_eq!(balance_strong("`a**b` c"), "`a**b` c");
        assert_eq!(balance_strong(r"a \*\* b"), r"a \*\* b");
    }

    #[test]
    fn a_span_opened_by_a_heading_stays_closed() {
        // headings become bold inside the accent colour, so the pass sees
        // balanced markers and must not add any
        assert_eq!(
            markdown_markup("# Title"),
            "<font color=\"#c8b158\">**Title**</font>"
        );
    }
}

#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn an_image_block_becomes_its_own_row() {
        let message = serde_json::json!({"role": "user", "content": [
            {"type": "text", "text": "look at this"},
            {"type": "image", "data": "QUJD", "mimeType": "image/png"},
        ]});
        let (rows, _) = build(std::slice::from_ref(&message), 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, KIND_USER);
        assert_eq!(rows[1].kind, KIND_IMAGE);
        assert_eq!(rows[1].image, "QUJD");
        assert_eq!(rows[1].mime, "image/png");
    }

    #[test]
    fn an_image_only_message_is_still_a_row() {
        let message = serde_json::json!({"role": "user", "content": [
            {"type": "image", "data": "QUJD", "mimeType": "image/png"},
        ]});
        let (rows, _) = build(std::slice::from_ref(&message), 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, KIND_IMAGE);
    }
}
