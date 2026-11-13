use super::*;

use crate::theme::{Resolved, Role};


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
    // The block guard uses a zero-width prefix, so folding narrow spaces here
    // cannot turn an indented line back into a CommonMark code block.
    let folded;
    let markup = if markup
        .chars()
        .any(|ch| matches!(ch, '\u{a0}' | '\u{202f}' | '\u{2009}' | '\u{2007}'))
    {
        folded = markup
            .chars()
            .map(|ch| match ch {
                '\u{202f}' | '\u{2009}' | '\u{2007}' => ' ',
                other => other,
            })
            .collect::<String>();
        folded.as_str()
    } else {
        markup
    };
    // Encoder output must parse; a failure is our bug, not a reason to show raw tags.
    StyledText::from_markdown(markup).unwrap_or_else(|error| {
        panic!("transcript markup must parse: {error}\n--- input ---\n{markup}")
    })
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


fn escape_markdown_char(ch: char, out: &mut String) -> bool {
    match ch {
        '\\' | '`' | '*' | '_' | '[' | ']' | '~' => {
            out.push('\\');
            out.push(ch);
            true
        }
        _ => false,
    }
}

pub(crate) fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => {
                if !escape_markdown_char(other, out) {
                    out.push(other);
                }
            }
        }
    }
}

/// Inline code is literal. `escape` cannot be reused: it re-encodes the HTML
/// entities the prose pass already escaped.
fn escape_code_span(text: &str, out: &mut String) {
    for ch in text.chars() {
        if !escape_markdown_char(ch, out) {
            out.push(ch);
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


/// `<`, `>` and `&` in the source become entities, so a message that *talks*
/// about a tag (``</font>``, `<div>`) draws the characters instead of opening a
/// tag the renderer then fails to close — which rejects the whole row.
/// Markdown syntax is left alone here: backticks and asterisks are still ours.
pub(crate) fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            other => out.push(other),
        }
    }
    out
}


/// How many `**` markers a line carries, ignoring escapes and code spans.
///
/// Each marker toggles bold, so an odd count means the line leaves bold open.
pub(crate) fn strong_markers(line: &str) -> usize {
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
pub(crate) fn balance_strong(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut open = false;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let (open_at, close_at) = line_font_bounds(line);
        if open_at == 0 && close_at == line.len() && line.contains("<font ") {
            open = balance_prose_with_fonts(line, open, &mut out);
            continue;
        }
        let (head, rest) = line.split_at(open_at);
        let (mid, tail) = rest.split_at(close_at - open_at);
        out.push_str(head);
        if open {
            out.push_str("**");
        }
        out.push_str(mid);
        // the markers we just added do not count: parity of the line decides
        if strong_markers(line) % 2 == 1 {
            open = !open;
        }
        if open {
            out.push_str("**");
        }
        out.push_str(tail);
    }
    out
}

fn balance_prose_with_fonts(line: &str, mut open: bool, out: &mut String) -> bool {
    let line_start = out.len();
    let mut pos = 0;
    while pos < line.len() {
        let Some(font_rel) = line[pos..].find("<font ") else {
            push_prose_chunk(&line[pos..], line_start, &mut open, out);
            break;
        };
        let chunk = &line[pos..pos + font_rel];
        push_prose_chunk(chunk, line_start, &mut open, out);

        let tag_start = pos + font_rel;
        let Some(gt_rel) = line[tag_start..].find('>') else {
            out.push_str(&line[tag_start..]);
            break;
        };
        let body_start = tag_start + gt_rel + 1;
        let Some(end_rel) = line[body_start..].find("</font>") else {
            out.push_str(&line[tag_start..]);
            break;
        };
        let body_end = body_start + end_rel;
        let after_font = body_end + "</font>".len();
        let font_open_tag = &line[tag_start..body_start];
        let font_body = &line[body_start..body_end];

        // Ensure no outer `**` stays open across `<font>...</font>`.
        if strong_markers(&out[line_start..]) % 2 == 1 {
            if out[line_start..].ends_with("**\u{200b}")
                && !out[line_start..].ends_with("\\**\u{200b}")
            {
                out.truncate(out.len() - "**\u{200b}".len());
            } else if out[line_start..].ends_with("**") && !out[line_start..].ends_with("\\**") {
                out.truncate(out.len() - 2);
            } else {
                if out[line_start..].ends_with('\u{200b}') && !chunk.ends_with('\u{200b}') {
                    out.pop();
                }
                out.push_str("**");
            }
        } else if open && out[line_start..].ends_with('\u{200b}') {
            // Drop any `\u{200b}` inserted before `<font>` solely as a bold separator.
            out.pop();
        }

        out.push_str(font_open_tag);
        let trimmed_body = font_body.trim_start_matches('\u{200b}');
        let already_bold = trimmed_body.starts_with("**")
            && trimmed_body.ends_with("**")
            && trimmed_body.len() >= 4;
        if open && !already_bold && !font_body.trim().is_empty() {
            let ws = font_body.len() - font_body.trim_start().len();
            out.push_str(&font_body[..ws]);
            out.push_str("**");
            out.push_str(&font_body[ws..]);
            out.push_str("**");
        } else {
            out.push_str(font_body);
        }
        out.push_str("</font>");

        if open && line[after_font..].starts_with("**") {
            pos = after_font + "**".len();
            open = false;
        } else {
            pos = after_font;
        }
    }
    if strong_markers(&out[line_start..]) % 2 == 1 {
        if out[line_start..].ends_with("**") && !out[line_start..].ends_with("\\**") {
            out.truncate(out.len() - 2);
        } else {
            out.push_str("**");
        }
    }
    open
}

fn push_prose_chunk(chunk: &str, line_start: usize, open: &mut bool, out: &mut String) {
    if chunk.is_empty() {
        return;
    }
    let need_opener = *open
        && strong_markers(&out[line_start..]) % 2 == 0
        && !chunk.trim_matches('\u{200b}').trim().is_empty()
        && !chunk.starts_with("**");
    if need_opener {
        let ws = chunk.len() - chunk.trim_start().len();
        out.push_str(&chunk[..ws]);
        out.push_str("**");
        out.push_str(&chunk[ws..]);
    } else {
        out.push_str(chunk);
    }
    if strong_markers(chunk) % 2 == 1 {
        *open = !*open;
    }
}

fn line_font_bounds(line: &str) -> (usize, usize) {
    let Some(suffix_start) = line.len().checked_sub("</font>".len()) else {
        return (0, line.len());
    };
    if !line.ends_with("</font>") {
        return (0, line.len());
    }
    let mut pos = 0;
    while let Some(after_open) = line[pos..].strip_prefix("<font ") {
        let Some(gt) = after_open.find('>') else {
            break;
        };
        let body_start = pos + "<font ".len() + gt + 1;
        if let Some(end_rel) = line[body_start..].find("</font>") {
            let after_close = body_start + end_rel + "</font>".len();
            if after_close < line.len() && line[after_close..].starts_with(" <font ") {
                pos = after_close + 1;
                continue;
            }
        }
        // Only treat `<font ...>` as wrapping the line when its nesting stays
        // open until the trailing `</font>`.
        let mut depth = 1usize;
        let mut scan = body_start;
        while scan < suffix_start {
            if line[scan..].starts_with("<font ") {
                depth += 1;
                scan += "<font ".len();
            } else if line[scan..].starts_with("</font>") {
                depth -= 1;
                if depth == 0 {
                    return (0, line.len());
                }
                scan += "</font>".len();
            } else {
                let ch = line[scan..].chars().next().unwrap_or(' ');
                scan += ch.len_utf8();
            }
        }
        if depth == 1 {
            return (body_start, suffix_start);
        }
        break;
    }
    (0, line.len())
}


pub(crate) fn quote_body(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}


/// A message's markdown, in the renderer's own markup: a heading becomes a bold
/// line in the heading's ink, a rule becomes a blank line, a quote becomes a
/// bar and quote ink, and a backticked word becomes the inline code's ink instead
/// of the renderer's cramped box. Everything the renderer cannot parse is taken
/// out on the way.
pub fn markdown_markup(text: &str, theme: &Resolved) -> String {
    let decoded = decode_entities(text);
    let mut out = String::with_capacity(decoded.len() + 16);
    for (index, line) in decoded.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            // headings carry their own ink, the way the TUI colours mdHeading,
            // and bold, because there is no size step inside one text run
            let body = escape_html(&autolink_markup(
                trimmed.trim_start_matches('#').trim_start(),
            ));
            open_font(theme, Role::TranscriptHeadingInk, &mut out);
            out.push_str("**");
            out.push_str(&body);
            out.push_str("**</font>");
        } else if trimmed.len() >= 3 && trimmed.chars().all(|ch| ch == '-' || ch == '*' || ch == '_' || ch == ' ') {
            // horizontal rule: nothing to draw
        } else if let Some(rest) = quote_body(line) {
            open_font(theme, Role::TranscriptQuoteEdge, &mut out);
            out.push('│');
            out.push_str("</font> ");
            open_font(theme, Role::TranscriptQuoteInk, &mut out);
            out.push_str(&escape_html(&autolink_markup(rest)));
            out.push_str("</font>");
        } else {
            out.push_str(&escape_html(&autolink_markup(line)));
        }
    }
    // the indentation guard has to be the last thing: an indented line is a code
    // block to CommonMark, and the renderer has no code blocks
    guard_block_markers(&balance_strong(&highlight_inline_code(&out, theme)))
}


/// Slint's `StyledText` has no italic property, so the TUI's quote emphasis is
/// carried by per-line markdown markers after the line has been styled.
pub fn quote_markup(text: &str, theme: &Resolved) -> String {
    markdown_markup(text, theme)
        .split('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("*{line}*")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}


/// Inline code, as a colour rather than a box.
///
/// The renderer's own treatment is a rectangle that hugs the glyphs with no
/// padding and no styling hook; `highlight_inline_code` replaces the span with
/// bold text in the inline code's own ink instead. No rectangle, no cramped
/// edges, and it still reads as "this is a symbol, not prose".
pub(crate) fn highlight_inline_code(text: &str, theme: &Resolved) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            out.push(ch);
            if let Some(escaped) = chars.next() {
                out.push(escaped);
            }
            continue;
        }
        if ch != '`' {
            out.push(ch);
            continue;
        }
        let mut ticks = String::from("`");
        while chars.peek() == Some(&'`') {
            ticks.push(chars.next().unwrap_or('`'));
        }
        let mut probe = chars.clone();
        let mut body = String::new();
        let mut closed = false;
        while let Some(next) = probe.next() {
            if next == '<' {
                break;
            }
            if next == '`' {
                let mut closing = String::from("`");
                while probe.peek() == Some(&'`') {
                    closing.push(probe.next().unwrap_or('`'));
                }
                if closing.len() == ticks.len() {
                    closed = true;
                    break;
                }
                body.push_str(&closing);
                continue;
            }
            body.push(next);
        }
        if !closed {
            for _ in 0..ticks.len() {
                out.push_str(r"\`");
            }
            continue;
        }
        chars = probe;
        if !body.trim().is_empty() {
            // A fresh span carries its weight inside the colour tag: at the
            // start of a line Slint reads `**<font>` as literal markers, while
            // `<font>**...**` parses. A span already inside bold only needs a
            // zero-width separator so the opening markers parse there.
            let bold_already = strong_markers(&out) % 2 == 1;
            if bold_already && out.ends_with("**") {
                out.push('\u{200b}');
            }
            open_font(theme, Role::TranscriptInlineCodeInk, &mut out);
            if !bold_already {
                out.push_str("**");
            }
            for (index, line) in body.trim().split('\n').enumerate() {
                if index > 0 {
                    if !bold_already {
                        out.push_str("**");
                    }
                    out.push_str("</font>");
                    out.push('\n');
                    if bold_already {
                        out.push('\u{200b}');
                    }
                    open_font(theme, Role::TranscriptInlineCodeInk, &mut out);
                    if !bold_already {
                        out.push_str("**");
                    }
                }
                if index > 0
                    && !bold_already
                    && (line.starts_with(' ') || line.starts_with('\t'))
                {
                    out.push('\u{200b}');
                }
                escape_code_span(line, &mut out);
            }
            if !bold_already {
                out.push_str("**");
            }
            out.push_str("</font>");
        } else {
            out.push_str(&ticks);
            out.push_str(&body);
            out.push_str(&ticks);
        }
    }
    out
}


/// One piece of a message: prose, a quote, a fenced block, or a table.
pub(crate) enum Block {
    Prose(String),
    Quote(String),
    Code { lang: String, body: String },
    Table(String),
}


/// Split one table row into cells on unescaped pipes, dropping the empty edge
/// cells a leading or trailing pipe leaves behind.
pub(crate) fn split_table_row(line: &str) -> Vec<String> {
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
pub(crate) fn is_delimiter_row(line: &str) -> bool {
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
pub(crate) fn table_alignment(cell: &str) -> u8 {
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


pub(crate) fn truncate_to(text: &str, width: usize) -> String {
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
pub(crate) fn render_table(raw: &[&str]) -> Option<String> {
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


pub(crate) fn flush_prose(prose: &mut Vec<String>, out: &mut Vec<Block>) {
    if prose.iter().any(|line| !line.trim().is_empty()) {
        out.push(Block::Prose(prose.join("\n")));
    }
    prose.clear();
}


fn starts_other_block(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#')
        || trimmed.starts_with("```")
        || trimmed.starts_with("- ")
        || trimmed.starts_with("* ")
        || trimmed.starts_with("+ ")
    {
        return true;
    }
    if trimmed.len() >= 3 && trimmed.chars().all(|ch| matches!(ch, '-' | '*' | '_' | ' ')) {
        return true;
    }
    let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && trimmed[digits..].starts_with(". ")
}


/// Carve a message into prose, fenced blocks and tables.
pub(crate) fn split_blocks(text: &str) -> Vec<Block> {
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
        if let Some(first) = quote_body(line) {
            flush_prose(&mut prose, &mut out);
            let mut body = vec![first.to_string()];
            let mut lazy = !first.trim().is_empty();
            index += 1;
            while index < lines.len() {
                let next = lines[index];
                if next.trim().is_empty() {
                    break;
                }
                if let Some(rest) = quote_body(next) {
                    body.push(rest.to_string());
                    lazy = true;
                    index += 1;
                    continue;
                }
                if !lazy
                    || starts_other_block(next)
                    || (next.contains('|') && lines.get(index + 1).is_some_and(|row| is_delimiter_row(row)))
                {
                    break;
                }
                body.push(next.to_string());
                index += 1;
            }
            while body.first().is_some_and(|line| line.trim().is_empty()) {
                body.remove(0);
            }
            if !body.is_empty() {
                out.push(Block::Quote(body.join("\n")));
            }
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
pub fn highlight_code(lang: &str, code: &str, theme: &Resolved) -> String {
    let language = lang
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(language.as_str(), "" | "py" | "python" | "python3") {
        highlight_python(code, theme)
    } else {
        plain_markup(code)
    }
}


pub(crate) const KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is", "lambda",
    "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "False", "try", "while",
    "with", "yield",
];


/// Open a `<font>` span in a role's colour; the caller closes it.
///
/// The markup is the one place a colour leaves the theme as text rather than as
/// a `slint::Color`, so this is where a role becomes the hex a `<font>` tag
/// takes. Nothing here decides a value: the role is the whole of the choice.
fn open_font(theme: &Resolved, role: Role, out: &mut String) {
    out.push_str("<font color=\"");
    out.push_str(&theme.get(role).text());
    out.push_str("\">");
}


pub(crate) fn wrap(theme: &Resolved, role: Role, text: &str, out: &mut String) {
    open_font(theme, role, out);
    escape(text, out);
    out.push_str("</font>");
}


/// Small python tokenizer -> coloured markup. Not a parser: keywords, strings and
/// comments, which is what makes a cell scannable.
///
/// A number is not one of them, and never was: a digit opens a word run, so the
/// arm below takes `1.5` before a number run could, and the code is drawn in the
/// ink of the block it sits in. That is also why there is no role for one: it
/// would paint nothing until that arm changes.
pub fn highlight_python(code: &str, theme: &Resolved) -> String {
    let decoded = decode_entities(code);
    let code = decoded.as_str();
    const KEY: Role = Role::TranscriptCodeKeywordInk;
    const STR: Role = Role::TranscriptCodeStringInk;
    const COM: Role = Role::TranscriptCodeCommentInk;

    let mut out = String::with_capacity(code.len() * 2);
    let mut chars = code.chars().peekable();
    let mut word = String::new();
    let flush_word = |word: &mut String, out: &mut String| {
        if word.is_empty() {
            return;
        }
        if KEYWORDS.contains(&word.as_str()) {
            wrap(theme, KEY, word, out);
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
                    for next in chars.by_ref() {
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
                wrap(theme, STR, &literal, &mut out);
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
                wrap(theme, COM, &comment, &mut out);
                out.push('\n');
            }
            ch if ch.is_alphanumeric() || ch == '_' => word.push(ch),
            ch => {
                flush_word(&mut word, &mut out);
                escape(&ch.to_string(), &mut out);
            }
        }
    }
    flush_word(&mut word, &mut out);
    guard_block_markers(&out)
}


/// CommonMark reads a 4-space indent as a code block and a leading `#`/`-`/`>`
/// as a heading/list/quote — both unsupported in Slint. A zero-width prefix
/// disables that block syntax without putting NBSP characters into text layout.
pub(crate) fn guard_block_markers(markup: &str) -> String {
    let mut out = String::with_capacity(markup.len() + 32);
    for (index, line) in markup.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 0 {
            out.push('\u{200b}');
            out.push_str(&line[..indent]);
        }
        let rest = &line[indent..];
        let first = rest.chars().next();
        if indent == 0 && matches!(first, Some('#' | '>' | '-' | '+' | '*' | '=' | '~' | '|' | '0'..='9')) {
            out.push('\u{200b}');
        }
        out.push_str(rest);
    }
    out
}
