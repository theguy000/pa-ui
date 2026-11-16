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
    let decoded = decode_entities(text).replace("\r\n", "\n").replace('\r', "\n");
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


pub(crate) fn unclosed_strong_depth(slice: &str) -> usize {
    unclosed_strong_depth_before(slice, ' ')
}

fn unclosed_strong_depth_before(slice: &str, eol_after: char) -> usize {
    let mut depth = 0usize;
    let mut in_code = false;
    let mut prev = ' ';
    let mut rest = slice;
    while let Some(ch) = rest.chars().next() {
        if ch == '\\' {
            rest = &rest[1..];
            if let Some(escaped) = rest.chars().next() {
                prev = escaped;
                rest = &rest[escaped.len_utf8()..];
            } else {
                prev = '\\';
            }
            continue;
        }
        if ch == '`' {
            in_code = !in_code;
            prev = '`';
            rest = &rest[1..];
            continue;
        }
        if !in_code && ch == '<' {
            if let Some((_, after_gt)) = rest.split_once('>') {
                prev = '>';
                rest = after_gt;
                continue;
            }
        }
        if !in_code {
            if let Some(after_first) = rest.strip_prefix('*') {
                let after_stars = after_first.trim_start_matches('*');
                let run = rest.bytes().take_while(|&b| b == b'*').count();
                if run >= 2 {
                    let after = after_stars.chars().next().unwrap_or(eol_after);
                    let after_ws = after.is_whitespace();
                    let before_ws = prev.is_whitespace();
                    let after_p = after.is_ascii_punctuation();
                    let before_p = prev.is_ascii_punctuation();
                    let left = !after_ws && (!after_p || before_ws || before_p);
                    let right = !before_ws && (!before_p || after_ws || after_p);
                    for _ in 0..(run / 2) {
                        if right && (!left || depth > 0) {
                            depth = depth.saturating_sub(1);
                        } else if left {
                            depth += 1;
                        }
                    }
                }
                prev = '*';
                rest = after_stars;
                continue;
            }
        }
        prev = ch;
        rest = &rest[ch.len_utf8()..];
    }
    depth
}

/// A `**span**` that runs over a line break is not something the Slint text
/// parser accepts, so it printed the asterisks instead of the weight. Each line
/// is rebalanced here: a span still open at the end of a line is closed there
/// and reopened at the start of the next one.
pub(crate) fn balance_strong(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut open_depth = 0usize;
    for (index, line) in text.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let (open_at, close_at) = line_font_bounds(line);
        if open_at == 0 && close_at == line.len() && line.contains("<font ") {
            open_depth = balance_prose_with_fonts(line, open_depth, &mut out);
            continue;
        }
        let (head, rest) = line.split_at(open_at);
        let (mid, tail) = rest.split_at(close_at - open_at);
        let is_heading_line = open_at > 0 && close_at < line.len() && !head.contains("</font>");
        if is_heading_line {
            open_depth = 0;
        }
        out.push_str(head);
        if mid.trim().is_empty() {
            out.push_str(mid);
            out.push_str(tail);
            continue;
        }
        if mid.contains("<font ") {
            open_depth = balance_prose_with_fonts(mid, open_depth, &mut out);
            if is_heading_line {
                open_depth = 0;
            }
            out.push_str(tail);
            continue;
        }
        let mid_start = out.len();
        if open_depth > 0 {
            let ws = mid.len() - mid.trim_start().len();
            out.push_str(&mid[..ws]);
            for idx in 0..open_depth {
                if idx > 0 {
                    out.push('\u{200b}');
                }
                out.push_str("**");
            }
            if mid[ws..].starts_with('*') {
                out.push('\u{200b}');
            }
            out.push_str(&mid[ws..]);
        } else {
            out.push_str(mid);
        }
        open_depth = unclosed_strong_depth(&out[mid_start..]);
        if open_depth > 0 {
            close_strong_before_trailing_ws(mid_start, open_depth, &mut out);
        }
        if is_heading_line {
            open_depth = 0;
        }
        out.push_str(tail);
    }
    out
}

fn balance_prose_with_fonts(line: &str, mut open_depth: usize, out: &mut String) -> usize {
    let line_start = out.len();
    let mut pos = 0;
    while pos < line.len() {
        let Some(font_rel) = line[pos..].find("<font ") else {
            push_prose_chunk(&line[pos..], line_start, &mut open_depth, out);
            break;
        };
        let chunk = &line[pos..pos + font_rel];
        push_prose_chunk(chunk, line_start, &mut open_depth, out);

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

        // Ensure no outer `**` stays open across `<font>...</font>`, placing any
        // closing `**` before trailing whitespace so CommonMark recognizes it.
        let unclosed_here = unclosed_strong_depth_before(&out[line_start..], '<');
        if unclosed_here > 0 {
            close_strong_before_trailing_ws_before(line_start, unclosed_here, '<', out);
            open_depth = open_depth.max(unclosed_here);
        } else if open_depth > 0 && out[line_start..].ends_with('\u{200b}') {
            out.pop();
        }

        out.push_str(font_open_tag);
        let trimmed_body = font_body.trim_start_matches('\u{200b}');
        let already_bold = trimmed_body.starts_with("**")
            && trimmed_body.ends_with("**")
            && trimmed_body.len() >= 4;
        if open_depth > 0 && !already_bold && !font_body.trim().is_empty() {
            let ws = font_body.len() - font_body.trim_start().len();
            out.push_str(&font_body[..ws]);
            out.push_str("**");
            out.push_str(&font_body[ws..]);
            out.push_str("**");
        } else {
            out.push_str(font_body);
        }
        out.push_str("</font>");

        if open_depth > 0 && line[after_font..].starts_with("**") {
            pos = after_font + "**".len();
            open_depth -= 1;
        } else {
            pos = after_font;
        }
    }
    let unclosed_tail = unclosed_strong_depth(&out[line_start..]);
    if unclosed_tail > 0 {
        close_strong_before_trailing_ws(line_start, unclosed_tail, out);
        open_depth = open_depth.max(unclosed_tail);
    }
    open_depth
}

fn close_strong_before_trailing_ws(line_start: usize, count: usize, out: &mut String) {
    close_strong_before_trailing_ws_before(line_start, count, ' ', out);
}

fn close_strong_before_trailing_ws_before(
    line_start: usize,
    mut count: usize,
    eol_after: char,
    out: &mut String,
) {
    if count == 0 {
        return;
    }
    let trimmed_len = line_start + out[line_start..].trim_end().len();
    let ws = out[trimmed_len..].to_string();
    let next_after = if ws.is_empty() { eol_after } else { ' ' };
    out.truncate(trimmed_len);
    while count > 0 {
        let cur_depth = unclosed_strong_depth_before(&out[line_start..], next_after);
        let popped = if out[line_start..].ends_with("**\u{200b}")
            && !out[line_start..].ends_with("\\**\u{200b}")
        {
            let prev = out.len();
            out.truncate(prev - "**\u{200b}".len());
            if unclosed_strong_depth_before(&out[line_start..], next_after) < cur_depth {
                true
            } else {
                out.push_str("**\u{200b}");
                false
            }
        } else if out[line_start..].ends_with("**") && !out[line_start..].ends_with("\\**") {
            let prev = out.len();
            out.truncate(prev - 2);
            if unclosed_strong_depth_before(&out[line_start..], next_after) < cur_depth {
                true
            } else {
                out.push_str("**");
                false
            }
        } else {
            false
        };
        if !popped {
            if out[line_start..].ends_with('\u{200b}') {
                out.pop();
            }
            if out[line_start..].ends_with('*') && !out[line_start..].ends_with("\\*") {
                out.push('\u{200b}');
            }
            out.push_str("**");
        }
        count -= 1;
    }
    out.push_str(&ws);
}

fn push_prose_chunk(chunk: &str, line_start: usize, open_depth: &mut usize, out: &mut String) {
    if chunk.is_empty() {
        return;
    }
    if chunk.trim_matches('\u{200b}').trim().is_empty() {
        out.push_str(chunk);
        return;
    }
    let need_opener = *open_depth > 0
        && unclosed_strong_depth(&out[line_start..]) == 0
        && !chunk.trim_matches('\u{200b}').trim().is_empty()
        && !chunk.starts_with("**");
    if need_opener {
        let ws = chunk.len() - chunk.trim_start().len();
        out.push_str(&chunk[..ws]);
        for idx in 0..*open_depth {
            if idx > 0 {
                out.push('\u{200b}');
            }
            out.push_str("**");
        }
        if chunk[ws..].starts_with('*') {
            out.push('\u{200b}');
        }
        out.push_str(&chunk[ws..]);
    } else {
        out.push_str(chunk);
    }
    *open_depth = unclosed_strong_depth(&out[line_start..]);
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
        if pos == 0 {
            if let Some(end_rel) = line[body_start..].find("</font>") {
                let after_close = body_start + end_rel + "</font>".len();
                if after_close < line.len() && line[after_close..].starts_with(" <font ") {
                    pos = after_close + 1;
                    continue;
                }
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
/// line in the heading's ink, a rule becomes a muted divider, a quote becomes a
/// bar and quote ink, and a backticked word becomes the inline code's ink instead
/// of the renderer's cramped box. Everything the renderer cannot parse is taken
/// out on the way.
pub fn markdown_markup(text: &str, theme: &Resolved) -> String {
    let decoded = decode_entities(text).replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::with_capacity(decoded.len() + 16);
    for (index, line) in decoded.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            let body = trimmed.trim_start_matches('#').trim_start().trim_end();
            if !body.is_empty() {
                push_heading_line(body, theme, &mut out);
            }
        } else if is_horizontal_rule(trimmed) {
            open_font(theme, Role::TranscriptQuoteEdge, &mut out);
            out.push_str("\u{2500}\u{2500}\u{2500}</font>");
        } else if let Some(rest) = quote_body(line) {
            open_font(theme, Role::TranscriptQuoteEdge, &mut out);
            out.push('\u{2502}');
            out.push_str("</font>");
            if !rest.trim().is_empty() {
                out.push(' ');
                open_font(theme, Role::TranscriptQuoteInk, &mut out);
                out.push_str(&escape_html(&autolink_markup(rest)));
                out.push_str("</font>");
            }
        } else if let Some((bullet, body)) = unordered_list_item(line) {
            out.push_str(&bullet);
            out.push_str(&escape_html(&autolink_markup(body)));
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
            if line.trim_matches(|ch: char| ch.is_whitespace() || ch == '\u{200b}').is_empty() {
                line.to_string()
            } else {
                wrap_line_emphasis(line)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    let mut marker = None;
    let mut count = 0usize;
    for ch in trimmed.chars() {
        if ch == ' ' || ch == '\t' {
            continue;
        }
        if !matches!(ch, '-' | '*' | '_') || marker.is_some_and(|m| m != ch) {
            return false;
        }
        marker = Some(ch);
        count += 1;
    }
    count >= 3
}

fn unordered_list_item(line: &str) -> Option<(String, &str)> {
    let mut indent = 0usize;
    let mut byte_idx = 0usize;
    for ch in line.chars() {
        match ch {
            ' ' => {
                indent += 1;
                byte_idx += 1;
            }
            '\t' => {
                indent += 4;
                byte_idx += 1;
            }
            _ => break,
        }
    }
    let rest = &line[byte_idx..];
    let body = rest
        .strip_prefix("- ")
        .or_else(|| rest.strip_prefix("* "))
        .or_else(|| rest.strip_prefix("+ "))?;
    let bullet = if indent == 0 {
        "\u{2022} ".to_string()
    } else {
        format!("{}\u{25e6} ", " ".repeat(indent))
    };
    Some((bullet, body))
}

fn link_dest_end(s: &str, start: usize) -> Option<usize> {
    if !s[start..].starts_with("](") {
        return None;
    }
    let mut depth = 1usize;
    let mut pos = start + 2;
    while pos < s.len() {
        let ch = s[pos..].chars().next()?;
        pos += ch.len_utf8();
        match ch {
            '\\' => {
                if let Some(esc) = s[pos..].chars().next() {
                    pos += esc.len_utf8();
                }
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(pos);
                }
            }
            _ => {}
        }
    }
    None
}

fn strip_strong_markers(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pos = 0;
    while pos < s.len() {
        if let Some(end) = link_dest_end(s, pos) {
            out.push_str(&s[pos..end]);
            pos = end;
            continue;
        }
        let ch = s[pos..].chars().next().unwrap_or(' ');
        if ch == '\\' {
            out.push(ch);
            pos += 1;
            if let Some(esc) = s[pos..].chars().next() {
                out.push(esc);
                pos += esc.len_utf8();
            }
            continue;
        }
        if s[pos..].starts_with("**") {
            pos += 2;
            continue;
        }
        out.push(ch);
        pos += ch.len_utf8();
    }
    out
}

fn sanitize_emphasis_stars(s: &str) -> String {
    struct Star {
        pos: usize,
        left: bool,
        right: bool,
        paired: bool,
    }
    let mut stars: Vec<Star> = Vec::new();
    let mut pos = 0;
    let mut prev = ' ';
    while pos < s.len() {
        if let Some(end) = link_dest_end(s, pos) {
            prev = ')';
            pos = end;
            continue;
        }
        let ch = s[pos..].chars().next().unwrap_or(' ');
        if ch == '\\' {
            pos += 1;
            if let Some(esc) = s[pos..].chars().next() {
                prev = esc;
                pos += esc.len_utf8();
            } else {
                prev = '\\';
            }
            continue;
        }
        if ch == '*' {
            let run = s[pos..].bytes().take_while(|&b| b == b'*').count();
            let after_pos = pos + run;
            let after = s[after_pos..].chars().next().unwrap_or(' ');
            if run % 2 == 1 {
                let after_ws = after.is_whitespace();
                let before_ws = prev.is_whitespace();
                let after_p = after.is_ascii_punctuation();
                let before_p = prev.is_ascii_punctuation();
                let left = !after_ws && (!after_p || before_ws || before_p);
                let right = !before_ws && (!before_p || after_ws || after_p);
                let star_pos = if right && !left { pos + run - 1 } else { pos };
                stars.push(Star {
                    pos: star_pos,
                    left,
                    right,
                    paired: false,
                });
            }
            prev = '*';
            pos = after_pos;
            continue;
        }
        prev = ch;
        pos += ch.len_utf8();
    }
    let mut open: Vec<usize> = Vec::new();
    for idx in 0..stars.len() {
        if stars[idx].right && !open.is_empty() {
            let opener = open.pop().expect("non-empty stack");
            stars[opener].paired = true;
            stars[idx].paired = true;
        } else if stars[idx].left {
            open.push(idx);
        }
    }
    let unpaired: Vec<usize> = stars
        .into_iter()
        .filter(|star| !star.paired)
        .map(|star| star.pos)
        .collect();
    if unpaired.is_empty() {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + unpaired.len());
    let mut cursor = 0;
    for star_pos in unpaired {
        out.push_str(&s[cursor..star_pos]);
        out.push('\\');
        out.push('*');
        cursor = star_pos + 1;
    }
    out.push_str(&s[cursor..]);
    out
}

fn push_wrapped_chunk(chunk: &str, marker: &str, strip_bold: bool, out: &mut String) {
    let lead = chunk.len() - chunk.trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{200b}').len();
    let trail_start = chunk.trim_end_matches(|ch: char| ch.is_whitespace() || ch == '\u{200b}').len();
    if lead >= trail_start {
        out.push_str(chunk);
        return;
    }
    let inner = &chunk[lead..trail_start];
    let cleaned = if strip_bold {
        sanitize_emphasis_stars(&strip_strong_markers(inner))
    } else {
        sanitize_emphasis_stars(inner)
    };
    if cleaned.is_empty() {
        out.push_str(&chunk[..lead]);
        out.push_str(&chunk[trail_start..]);
        return;
    }
    out.push_str(&chunk[..lead]);
    out.push_str(marker);
    if cleaned.starts_with('*') {
        out.push('\u{200b}');
    }
    out.push_str(&cleaned);
    if cleaned.ends_with('*') {
        out.push('\u{200b}');
    }
    out.push_str(marker);
    out.push_str(&chunk[trail_start..]);
}

fn push_heading_chunk(chunk: &str, theme: &Resolved, out: &mut String) {
    let stripped = strip_strong_markers(chunk);
    let lead = stripped.len()
        - stripped
            .trim_start_matches(|ch: char| ch.is_whitespace() || ch == '\u{200b}')
            .len();
    let trail_start = stripped
        .trim_end_matches(|ch: char| ch.is_whitespace() || ch == '\u{200b}')
        .len();
    if lead >= trail_start {
        out.push_str(&stripped);
        return;
    }
    out.push_str(&stripped[..lead]);
    open_font(theme, Role::TranscriptHeadingInk, out);
    push_wrapped_chunk(&stripped[lead..trail_start], "**", false, out);
    out.push_str("</font>");
    out.push_str(&stripped[trail_start..]);
}

fn next_markdown_link(s: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    while let Some(open_rel) = s[pos..].find('[') {
        let open = pos + open_rel;
        if let Some(close_br_rel) = s[open..].find("](") {
            let close_br = open + close_br_rel;
            if let Some(end) = link_dest_end(s, close_br) {
                return Some((open, end));
            }
        }
        pos = open + 1;
    }
    None
}

fn push_heading_line(raw: &str, theme: &Resolved, out: &mut String) {
    let highlighted = highlight_inline_code(&escape_html(&autolink_markup(raw)), theme);
    let mut pos = 0;
    while pos < highlighted.len() {
        let slice = &highlighted[pos..];
        let font_rel = slice.find("<font ");
        let link_with_font = next_markdown_link(slice)
            .filter(|&(l_start, l_end)| slice[l_start..l_end].contains("<font "));
        if let Some((l_start, l_end)) = link_with_font {
            if font_rel.is_none_or(|f| l_start <= f) {
                if l_start > 0 {
                    push_heading_chunk(&slice[..l_start], theme, out);
                }
                // A label carrying an inline-code `<font>` cannot stay a link:
                // Slint reads that `</font>` as closing the heading's own tag.
                // The label's words stay; the destination is dropped.
                let label = &slice[l_start + 1..l_start + slice[l_start..l_end].find("](").unwrap_or(l_end - l_start - 1)];
                push_heading_plain_with_fonts(label, theme, out);
                pos += l_end;
                continue;
            }
        }
        let Some(f_rel) = font_rel else {
            push_heading_chunk(slice, theme, out);
            break;
        };
        if f_rel > 0 {
            push_heading_chunk(&slice[..f_rel], theme, out);
        }
        let tag_start = pos + f_rel;
        let Some(end_rel) = highlighted[tag_start..].find("</font>") else {
            push_heading_chunk(&highlighted[tag_start..], theme, out);
            break;
        };
        let tag_end = tag_start + end_rel + "</font>".len();
        out.push_str(&highlighted[tag_start..tag_end]);
        pos = tag_end;
    }
}

fn push_heading_plain_with_fonts(slice: &str, theme: &Resolved, out: &mut String) {
    let mut pos = 0;
    while pos < slice.len() {
        let Some(f_rel) = slice[pos..].find("<font ") else {
            push_heading_chunk(&slice[pos..], theme, out);
            break;
        };
        if f_rel > 0 {
            push_heading_chunk(&slice[pos..pos + f_rel], theme, out);
        }
        let tag_start = pos + f_rel;
        let Some(end_rel) = slice[tag_start..].find("</font>") else {
            push_heading_chunk(&slice[tag_start..], theme, out);
            break;
        };
        let tag_end = tag_start + end_rel + "</font>".len();
        out.push_str(&slice[tag_start..tag_end]);
        pos = tag_end;
    }
}

fn wrap_line_with(line: &str, marker: &str, out: &mut String) {
    let mut pos = 0;
    while pos < line.len() {
        let next_open = line[pos..].find("<font ");
        let next_close = line[pos..].find("</font>");
        let next_tag = match (next_open, next_close) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        };
        let Some(rel) = next_tag else {
            push_wrapped_chunk(&line[pos..], marker, false, out);
            break;
        };
        if rel > 0 {
            push_wrapped_chunk(&line[pos..pos + rel], marker, false, out);
        }
        let tag_start = pos + rel;
        let Some(gt_rel) = line[tag_start..].find('>') else {
            push_wrapped_chunk(&line[tag_start..], marker, false, out);
            break;
        };
        let tag_end = tag_start + gt_rel + 1;
        out.push_str(&line[tag_start..tag_end]);
        pos = tag_end;
    }
}

fn wrap_line_emphasis(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    wrap_line_with(line, "*", &mut out);
    out
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
            for (index, line) in body.trim().split('\n').enumerate() {
                if index > 0 {
                    out.push('\n');
                }
                if line.trim().is_empty() {
                    continue;
                }
                open_font(theme, Role::TranscriptInlineCodeInk, &mut out);
                out.push_str("**");
                if line.starts_with(' ') || line.starts_with('\t') {
                    out.push('\u{200b}');
                }
                escape_code_span(line, &mut out);
                if line.ends_with(' ') || line.ends_with('\t') {
                    out.push('\u{200b}');
                }
                out.push_str("**</font>");
            }
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
    Table(Table),
}

/// A GFM pipe table kept as its source and its cells, so the Sheet can draw
/// columns instead of an ASCII cage and Copy can return the markdown.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Table {
    pub source: String,
    pub header: Vec<String>,
    pub aligns: Vec<u8>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub(crate) fn stretches(&self) -> Vec<f32> {
        (0..self.header.len())
            .map(|column| {
                let widest = self
                    .rows
                    .iter()
                    .filter_map(|row| row.get(column))
                    .chain(std::iter::once(&self.header[column]))
                    .map(|cell| cell.chars().count())
                    .max()
                    .unwrap_or(1);
                (widest as f32 / 16.0).clamp(0.4, 3.0)
            })
            .collect()
    }

    pub(crate) fn cells(&self, theme: &Resolved) -> Vec<TableCellData> {
        let stretches = self.stretches();
        let mut cells = Vec::new();
        for (column, source) in self.header.iter().enumerate() {
            cells.push(TableCellData {
                row: -1,
                column: column as i32,
                text: table_cell_markup(source, true, theme),
                align: self.aligns[column],
                header: true,
                stretch: stretches[column],
            });
        }
        for (row, sources) in self.rows.iter().enumerate() {
            for (column, source) in sources.iter().enumerate() {
                cells.push(TableCellData {
                    row: row as i32,
                    column: column as i32,
                    text: table_cell_markup(source, false, theme),
                    align: self.aligns[column],
                    header: false,
                    stretch: stretches[column],
                });
            }
        }
        cells
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableCellData {
    pub row: i32,
    pub column: i32,
    pub text: String,
    pub align: u8,
    pub header: bool,
    pub stretch: f32,
}

fn table_cell_markup(source: &str, header: bool, theme: &Resolved) -> String {
    let markup = markdown_markup(source, theme);
    if !header {
        return markup;
    }
    let mut out = String::new();
    for (index, line) in markup.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        wrap_line_with(&strip_strong_markers(line), "**", &mut out);
    }
    out
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


pub(crate) fn parse_table(raw: &[&str]) -> Option<Table> {
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
    Some(Table {
        source: raw.join("\n"),
        header,
        aligns,
        rows,
    })
}


pub(crate) fn table_from_source(source: &str) -> Option<Table> {
    let lines: Vec<&str> = source.lines().collect();
    parse_table(&lines)
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
    if is_horizontal_rule(trimmed) {
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
            match parse_table(&raw) {
                Some(table) => out.push(Block::Table(table)),
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
    let lines: Vec<&str> = markup.split('\n').collect();
    let last_non_empty = lines.iter().rposition(|l| !l.trim().is_empty());
    let mut out = String::with_capacity(markup.len() + 32);
    let mut seen_non_empty = false;
    let mut emitted_blank = false;
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        if line.trim().is_empty() {
            let has_more = last_non_empty.is_some_and(|last| index < last);
            if seen_non_empty && has_more && !emitted_blank {
                out.push('\u{200b}');
                emitted_blank = true;
            }
            continue;
        }
        seen_non_empty = true;
        emitted_blank = false;
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 0 {
            out.push('\u{200b}');
            out.push_str(&line[..indent]);
        }
        let rest = &line[indent..];
        let first = rest.chars().next();
        let star_is_block = first == Some('*')
            && (rest == "*"
                || rest.starts_with("* ")
                || rest.starts_with("*\t")
                || (rest.chars().all(|ch| ch == '*' || ch == ' ' || ch == '\t')
                    && rest.chars().filter(|&ch| ch == '*').count() >= 3));
        if indent == 0
            && (star_is_block
                || matches!(first, Some('#' | '>' | '-' | '+' | '=' | '~' | '|' | '0'..='9')))
        {
            out.push('\u{200b}');
        }
        out.push_str(rest);
    }
    out
}
