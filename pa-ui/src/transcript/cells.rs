use super::*;

/// The `bashCommands` the python tool reports on its own result: the first
/// command it ran, how many it ran, and how many of the cell's lines they
/// account for. That last number is what decides whether the cell *is* a bash
/// cell or merely contains bash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BashInfo {
    pub first: String,
    pub count: usize,
    pub lines: usize,
}


#[must_use]
pub fn bash_commands(details: Option<&Value>) -> Option<BashInfo> {
    let record = details?.get("bashCommands")?;
    let first = record.get("first")?.as_str()?.to_string();
    if first.trim().is_empty() {
        return None;
    }
    let number = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or(1)
    };
    Some(BashInfo {
        count: number("count"),
        lines: number("lines"),
        first,
    })
}


/// How long a call took, in the TUI's own words: `2ms`, `12.3s`, `5m 04s`.
#[must_use]
pub fn format_duration(duration_ms: Option<u64>) -> String {
    let Some(ms) = duration_ms else {
        return String::new();
    };
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let total_seconds = ms / 1_000;
    if total_seconds < 60 {
        return format!("{total_seconds}.{}s", (ms % 1_000) / 100);
    }
    let minutes = total_seconds / 60;
    let seconds = total_seconds % 60;
    format!("{minutes}m {seconds:02}s")
}


/// The tool's own timing, beside the rest of its details.
#[must_use]
pub fn duration_ms(details: Option<&Value>) -> Option<u64> {
    let details = details?;
    details
        .get("durationMs")
        .or_else(|| details.get("duration_ms"))
        .and_then(Value::as_u64)
}


/// Non-blank lines of the cell, after a `%%bash` magic is stripped.
pub(crate) fn input_line_count(code: &str) -> usize {
    let body = crate::preview::parse_ipython_bash_cell(code)
        .map_or_else(|| code.to_string(), |cell| cell.body);
    body.lines().filter(|line| !line.trim().is_empty()).count()
}


/// A cell is a bash cell when bash accounts for half its lines or more -
/// `1/BASH_DOMINANCE_DIVISOR`, the same threshold the TUI uses.
pub(crate) const BASH_DOMINANCE_DIVISOR: usize = 2;


/// The title line of a cell, decided exactly as the TUI's collapsed line decides
/// it: the label from the dominance test, then the preview of whichever side
/// won, then how many further commands ran.
pub fn apply_header(
    row: &mut Row,
    code: &str,
    bash: Option<&BashInfo>,
    duration: Option<u64>,
) {
    let code = code.trim_end();
    let dominant = bash.filter(|bash| bash.lines * BASH_DOMINANCE_DIVISOR >= input_line_count(code));
    let preview = crate::preview::preview_ipython_code(code);
    let is_bash_cell = crate::preview::parse_ipython_bash_cell(code).is_some();
    let verb = if dominant.is_some() {
        "bash"
    } else {
        match (is_bash_cell, preview.language) {
            (true, crate::preview::CodePreviewLanguage::Python) => "bash \u{00b7} python",
            (_, crate::preview::CodePreviewLanguage::Bash) => "bash",
            (false, crate::preview::CodePreviewLanguage::Python) => "python",
        }
    };
    let text = match dominant {
        Some(bash) => crate::preview::preview_bash_command(&bash.first).text,
        None => preview.text,
    };
    let text = if text.trim().is_empty() {
        summary_line(code)
    } else {
        text
    };
    let mut title = text;
    if let Some(bash) = dominant.filter(|bash| bash.count > 1) {
        if title.is_empty() {
            title = format!("+{} more", bash.count - 1);
        } else {
            title.push_str(&format!(" \u{00b7} +{} more", bash.count - 1));
        }
    }
    row.verb = verb.to_string();
    row.title = title;
    row.elapsed = format_duration(duration);
}


#[must_use]
pub(crate) fn tool_source(args: &Value) -> std::borrow::Cow<'_, str> {
    match args.get("code").and_then(Value::as_str) {
        Some(code) if !code.is_empty() => std::borrow::Cow::Borrowed(code),
        _ => std::borrow::Cow::Owned(args.to_string()),
    }
}


/// A cell row for a tool call that has just started, shaped exactly like the one
/// the store will produce for it so the reconcile at turn end changes nothing
/// on screen. Its code is highlighted in the theme the window is drawn in, the
/// same one the store's copy of the cell is built under.
#[must_use]
pub fn tool_cell(call_id: &str, args: &Value) -> Row {
    let code = args.get("code").and_then(Value::as_str).unwrap_or_default();
    let mut row = Row::new(KIND_CELL);
    row.id = call_id.to_string();
    apply_header(&mut row, &tool_source(args), None, None);
    row.raw_code = code.to_string();
    row.code = highlight_python(code, &crate::theme::current());
    row.has_code = !code.is_empty();
    row
}


pub fn finish_cell(row: &mut Row, output: &str, failed: bool) {
    row.output = plain_markup(output);
    row.has_output = !output.trim().is_empty();
    row.failed = failed;
}


/// The one line a collapsed cell shows.
///
/// Bash cells are titled by the command their tool reported, so this is what a
/// plain python cell gets: its first meaningful statement, cut to fit.
pub(crate) fn summary_line(code: &str) -> String {
    let line = code
        .lines()
        .map(str::trim)
        .map(|line| line.trim_end_matches(':'))
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("empty cell");
    clip(line, 72)
}


pub(crate) fn clip(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    // cut at a word, the way the TUI's descriptor does: half a word reads worse
    // than a shorter label
    let trimmed = match cut.rfind(' ') {
        Some(at) if at > limit / 2 => cut[..at].to_string(),
        _ => cut,
    };
    format!("{trimmed}\u{2026}")
}

// ── markup ──────────────────────────────────────────────────────────
