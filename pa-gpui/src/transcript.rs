use base64::Engine as _;
use gpui_kit::component::text::TextViewState;
use gpui_kit::*;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ItemKind {
    User,
    Assistant,
    Thinking,
    ToolCall,
    Summary,
    Custom,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum CardVariant {
    None,
    Python,
    Bash,
    ToolOther,
    BashDone,
    AgentMessage,
    ProviderRetry,
    Harness,
}

pub struct TranscriptItem {
    pub id: usize,
    pub entry_id: Option<String>,
    pub call_id: Option<String>,
    pub kind: ItemKind,
    pub variant: CardVariant,
    pub verb: SharedString,
    pub title: SharedString,
    pub meta: SharedString,
    pub body_md: Option<Entity<TextViewState>>,
    pub raw_body: String,
    pub output: Option<String>,
    pub images: Vec<Arc<Image>>,
    pub expanded: bool,
    pub active: bool,
    pub failed: bool,
    pub fingerprint: u64,
}

impl TranscriptItem {
    pub fn compute_fingerprint(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.entry_id.hash(&mut h);
        self.call_id.hash(&mut h);
        self.kind.hash(&mut h);
        self.variant.hash(&mut h);
        self.verb.as_ref().hash(&mut h);
        self.title.as_ref().hash(&mut h);
        self.meta.as_ref().hash(&mut h);
        self.raw_body.hash(&mut h);
        self.output.hash(&mut h);
        self.images.len().hash(&mut h);
        self.failed.hash(&mut h);
        h.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BashInfo {
    first: String,
    count: usize,
    lines: usize,
}

#[derive(Debug, Clone)]
struct ToolResultInfo {
    text: String,
    failed: bool,
    bash: Option<BashInfo>,
    duration_ms: Option<u64>,
}

const BASH_DOMINANCE_DIVISOR: usize = 2;

fn parse_bash_commands(details: Option<&Value>) -> Option<BashInfo> {
    let record = details?.get("bashCommands")?;
    let first = record.get("first")?.as_str()?.trim().to_string();
    if first.is_empty() {
        return None;
    }
    let number = |key: &str| {
        record
            .get(key)
            .and_then(Value::as_u64)
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(1)
    };
    Some(BashInfo {
        first,
        count: number("count"),
        lines: number("lines"),
    })
}

fn parse_duration_ms(details: Option<&Value>) -> Option<u64> {
    let d = details?;
    d.get("durationMs")
        .or_else(|| d.get("duration_ms"))
        .and_then(Value::as_u64)
}

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

fn strip_bash_cell_magic(code: &str) -> Option<&str> {
    let trimmed = code.trim_start();
    let rest = trimmed.strip_prefix("%%bash")?;
    if let Some(nl) = rest.find('\n') {
        Some(&rest[nl + 1..])
    } else {
        Some("")
    }
}

fn input_line_count(code: &str) -> usize {
    let body = strip_bash_cell_magic(code).unwrap_or(code);
    body.lines().filter(|l| !l.trim().is_empty()).count()
}

fn extract_inline_bash_preview(code: &str) -> Option<String> {
    let meaningful: Vec<&str> = code
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    if meaningful.is_empty() {
        return None;
    }
    let bash_lines = meaningful
        .iter()
        .filter(|l| l.contains("bash("))
        .count();
    if bash_lines == 0 || bash_lines * BASH_DOMINANCE_DIVISOR < meaningful.len() {
        return None;
    }
    for line in &meaningful {
        if let Some(idx) = line.find("bash(") {
            let after = line[idx + 5..].trim_start();
            for quote in ["\"\"\"", "'''", "\"", "'"] {
                if let Some(qrest) = after
                    .strip_prefix(quote)
                    .or_else(|| after.strip_prefix(&format!("f{quote}")))
                    .or_else(|| after.strip_prefix(&format!("r{quote}")))
                {
                    let inner = qrest.split(quote).next().unwrap_or(qrest).trim();
                    if !inner.is_empty() {
                        return Some(clip_line(inner, 72));
                    }
                }
            }
            return Some(clip_line(line, 72));
        }
    }
    None
}

fn clip_line(s: &str, limit: usize) -> String {
    let single: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if single.chars().count() <= limit {
        single
    } else {
        let mut out: String = single.chars().take(limit.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

pub fn build_items(messages: &[Value]) -> Vec<TranscriptItem> {
    let mut results: HashMap<String, ToolResultInfo> = HashMap::new();
    for msg in messages {
        if msg.get("role").and_then(Value::as_str) == Some("toolResult") {
            if let Some(cid) = msg.get("toolCallId").and_then(Value::as_str) {
                let text = extract_text(msg);
                let is_err = msg
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
                    || text.trim_start().starts_with("Traceback");
                let details = msg.get("details");
                results.insert(
                    cid.to_string(),
                    ToolResultInfo {
                        text,
                        failed: is_err,
                        bash: parse_bash_commands(details),
                        duration_ms: parse_duration_ms(details),
                    },
                );
            }
        }
    }

    let mut items = Vec::new();
    let mut next_id = 0usize;

    for msg in messages {
        let entry_id = msg
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let role = msg.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "user" => {
                let text = extract_text(msg);
                let images = extract_images(msg);
                if text.trim().is_empty() && images.is_empty() {
                    continue;
                }
                items.push(TranscriptItem {
                    id: next_id,
                    entry_id,
                    call_id: None,
                    kind: ItemKind::User,
                    variant: CardVariant::None,
                    verb: SharedString::default(),
                    title: "You".into(),
                    meta: SharedString::default(),
                    body_md: None,
                    raw_body: text,
                    output: None,
                    images,
                    expanded: true,
                    active: false,
                    failed: false,
                    fingerprint: 0,
                });
                next_id += 1;
            }
            "assistant" => {
                if let Some(err) = msg.get("errorMessage").and_then(Value::as_str) {
                    if !err.trim().is_empty() {
                        items.push(TranscriptItem {
                            id: next_id,
                            entry_id: entry_id.clone(),
                            call_id: None,
                            kind: ItemKind::Error,
                            variant: CardVariant::None,
                            verb: SharedString::default(),
                            title: "Error".into(),
                            meta: SharedString::default(),
                            body_md: None,
                            raw_body: err.to_string(),
                            output: None,
                            images: Vec::new(),
                            expanded: true,
                            active: false,
                            failed: true,
                            fingerprint: 0,
                        });
                        next_id += 1;
                    }
                }
                let Some(blocks) = msg.get("content").and_then(Value::as_array) else {
                    continue;
                };
                for block in blocks {
                    let kind = block.get("type").and_then(Value::as_str).unwrap_or_default();
                    match kind {
                        "thinking" => {
                            let thought = block
                                .get("thinking")
                                .or_else(|| block.get("text"))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim();
                            if thought.is_empty() {
                                continue;
                            }
                            items.push(TranscriptItem {
                                id: next_id,
                                entry_id: entry_id.clone(),
                                call_id: None,
                                kind: ItemKind::Thinking,
                                variant: CardVariant::None,
                                verb: SharedString::default(),
                                title: "Thought".into(),
                                meta: SharedString::default(),
                                body_md: None,
                                raw_body: thought.to_string(),
                                output: None,
                                images: Vec::new(),
                                expanded: false,
                                active: false,
                                failed: false,
                                fingerprint: 0,
                            });
                            next_id += 1;
                        }
                        "text" => {
                            let text = block
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .trim();
                            if text.is_empty() {
                                continue;
                            }
                            items.push(TranscriptItem {
                                id: next_id,
                                entry_id: entry_id.clone(),
                                call_id: None,
                                kind: ItemKind::Assistant,
                                variant: CardVariant::None,
                                verb: SharedString::default(),
                                title: "Assistant".into(),
                                meta: SharedString::default(),
                                body_md: None,
                                raw_body: text.to_string(),
                                output: None,
                                images: Vec::new(),
                                expanded: true,
                                active: false,
                                failed: false,
                                fingerprint: 0,
                            });
                            next_id += 1;
                        }
                        "toolCall" => {
                            let cid = block
                                .get("id")
                                .and_then(Value::as_str)
                                .map(str::to_string);
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("tool");
                            let args = block.get("arguments").unwrap_or(&Value::Null);
                            let res = cid.as_deref().and_then(|id| results.get(id));
                            let (variant, verb, summary, raw_code) = classify_tool_call(
                                name,
                                args,
                                res.and_then(|r| r.bash.as_ref()),
                            );
                            let output = res.map(|r| r.text.clone());
                            let failed = res.is_some_and(|r| r.failed);
                            let meta = if failed {
                                "failed".to_string()
                            } else {
                                format_duration(res.and_then(|r| r.duration_ms))
                            };
                            items.push(TranscriptItem {
                                id: next_id,
                                entry_id: entry_id.clone(),
                                call_id: cid,
                                kind: ItemKind::ToolCall,
                                variant,
                                verb: verb.into(),
                                title: summary.into(),
                                meta: meta.into(),
                                body_md: None,
                                raw_body: raw_code,
                                output,
                                images: Vec::new(),
                                expanded: false,
                                active: false,
                                failed,
                                fingerprint: 0,
                            });
                            next_id += 1;
                        }
                        _ => {}
                    }
                }
            }
            "compactionSummary" | "branchSummary" => {
                let summary = msg
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let (title, meta, body) = if role == "compactionSummary" {
                    let tokens = msg.get("tokensBefore").and_then(Value::as_u64);
                    let retained = msg.get("retainedMessageCount").and_then(Value::as_u64);
                    let meta = [
                        tokens.map(|n| format!("{n} tokens before")),
                        retained.map(|n| format!("{n} retained")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" · ");
                    let focus = msg
                        .get("customInstructions")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    let body = if focus.is_empty() {
                        summary.to_string()
                    } else if summary.is_empty() {
                        format!("**Focus:** {focus}")
                    } else {
                        format!("**Focus:** {focus}\n\n{summary}")
                    };
                    ("Context compacted", meta, body)
                } else {
                    let from = msg.get("fromId").and_then(Value::as_str).unwrap_or_default();
                    let meta = if from.is_empty() {
                        String::new()
                    } else {
                        format!("from {from}")
                    };
                    ("Branch summary", meta, summary.to_string())
                };
                items.push(TranscriptItem {
                    id: next_id,
                    entry_id,
                    call_id: None,
                    kind: ItemKind::Summary,
                    variant: CardVariant::Harness,
                    verb: SharedString::default(),
                    title: title.into(),
                    meta: meta.into(),
                    body_md: None,
                    raw_body: body,
                    output: None,
                    images: Vec::new(),
                    expanded: false,
                    active: false,
                    failed: false,
                    fingerprint: 0,
                });
                next_id += 1;
            }
            _ if role == "custom" || msg.get("customType").is_some() => {
                if msg.get("display").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                let ctype = msg
                    .get("customType")
                    .and_then(Value::as_str)
                    .unwrap_or("notice");
                let text = extract_text(msg);
                if ctype == "session_slash_command" {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    items.push(TranscriptItem {
                        id: next_id,
                        entry_id,
                        call_id: None,
                        kind: ItemKind::User,
                        variant: CardVariant::None,
                        verb: SharedString::default(),
                        title: "You".into(),
                        meta: SharedString::default(),
                        body_md: None,
                        raw_body: trimmed.to_string(),
                        output: None,
                        images: Vec::new(),
                        expanded: true,
                        active: false,
                        failed: false,
                        fingerprint: 0,
                    });
                    next_id += 1;
                    continue;
                }
                let details = msg.get("details").unwrap_or(&Value::Null);
                let (variant, title, meta, failed) = classify_custom(ctype, &text, details);
                items.push(TranscriptItem {
                    id: next_id,
                    entry_id,
                    call_id: None,
                    kind: ItemKind::Custom,
                    variant,
                    verb: SharedString::default(),
                    title: title.into(),
                    meta: meta.into(),
                    body_md: None,
                    raw_body: text,
                    output: None,
                    images: Vec::new(),
                    expanded: false,
                    active: false,
                    failed,
                    fingerprint: 0,
                });
                next_id += 1;
            }
            _ => {}
        }
    }

    for item in &mut items {
        item.fingerprint = item.compute_fingerprint();
    }

    items
}

pub fn hydrate_markdown(item: &mut TranscriptItem, cx: &mut App) {
    if item.body_md.is_some() {
        return;
    }
    let visible = matches!(item.kind, ItemKind::User | ItemKind::Assistant) || item.expanded;
    if !visible || item.raw_body.trim().is_empty() {
        return;
    }
    let formatted = format_expanded_body(item);
    item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
}

/// Find the oldest prefix `0..drop_count` that can be unloaded while keeping at
/// least `keep_items` tail rows and landing on an item with a known `entry_id`
/// cursor so older history can be re-paged seamlessly if the user scrolls back up.
pub fn compute_tail_trim(
    items: &[TranscriptItem],
    max_items: usize,
    keep_items: usize,
) -> Option<(usize, String)> {
    if items.len() <= max_items {
        return None;
    }
    let target_drop = items.len().saturating_sub(keep_items);
    if target_drop == 0 {
        return None;
    }
    // Walk backwards from `target_drop` to a clean message boundary (an item whose
    // `entry_id` differs from the preceding item's `entry_id`, so we never split
    // a multi-block assistant message across the unloaded boundary).
    for ix in (1..=target_drop).rev() {
        if let Some(eid) = items[ix].entry_id.as_deref() {
            if items[ix - 1].entry_id.as_deref() != Some(eid) {
                return Some((ix, eid.to_string()));
            }
        }
    }
    None
}

/// Find where a freshly fetched tail page overlaps `existing`, returning
/// `(existing_start, incoming_start)` so we can splice only the changed suffix
/// without losing loaded history or measured heights in `ListState`.
pub fn find_tail_merge_start(
    existing: &[TranscriptItem],
    incoming: &[TranscriptItem],
    first_entry: &str,
) -> Option<(usize, usize)> {
    if existing.is_empty() || incoming.is_empty() {
        return None;
    }
    // 1. Anchor by `first_entry` (the oldest message id in the incoming page).
    if !first_entry.is_empty() {
        if let Some(ex_start) = existing
            .iter()
            .position(|it| it.entry_id.as_deref() == Some(first_entry))
        {
            let mut e = ex_start;
            let mut n = 0;
            while e < existing.len()
                && n < incoming.len()
                && existing[e].fingerprint == incoming[n].fingerprint
            {
                e += 1;
                n += 1;
            }
            return Some((e, n));
        }
    }
    // 2. Fallback: anchor by the first incoming item's fingerprint.
    let first_fp = incoming[0].fingerprint;
    if first_fp != 0 {
        if let Some(ex_start) = existing.iter().position(|it| it.fingerprint == first_fp) {
            let mut e = ex_start;
            let mut n = 0;
            while e < existing.len()
                && n < incoming.len()
                && existing[e].fingerprint == incoming[n].fingerprint
            {
                e += 1;
                n += 1;
            }
            return Some((e, n));
        }
    }
    // 3. New-session fallback: if existing starts with optimistic (entry_id: None) items
    // whose first user/assistant message matches incoming[0], merge in-place instead of
    // resetting the list and losing measured heights.
    if existing[0].entry_id.is_none()
        && existing[0].kind == incoming[0].kind
        && existing[0].raw_body == incoming[0].raw_body
    {
        return Some((0, 0));
    }
    None
}

fn settle_thinking(items: &mut [TranscriptItem]) {
    for it in items.iter_mut().rev() {
        if it.kind == ItemKind::Thinking && it.active {
            it.active = false;
            it.title = "Thought".into();
        }
    }
}

pub fn append_text_delta(items: &mut Vec<TranscriptItem>, delta: &str, cx: &mut App) {
    if delta.is_empty() {
        return;
    }
    settle_thinking(items);
    if let Some(last) = items.last_mut() {
        if last.kind == ItemKind::Assistant {
            last.raw_body.push_str(delta);
            last.active = true;
            if let Some(md) = &last.body_md {
                md.update(cx, |state, cx| state.push_str(delta, cx));
            } else {
                let raw = last.raw_body.clone();
                last.body_md = Some(cx.new(|cx| TextViewState::markdown(&raw, cx)));
            }
            return;
        }
    }
    let next_id = items.last().map_or(0, |i| i.id + 1);
    let md = cx.new(|cx| TextViewState::markdown(delta, cx));
    items.push(TranscriptItem {
        id: next_id,
        entry_id: None,
        call_id: None,
        kind: ItemKind::Assistant,
        variant: CardVariant::None,
        verb: SharedString::default(),
        title: "Assistant".into(),
        meta: SharedString::default(),
        body_md: Some(md),
        raw_body: delta.to_string(),
        output: None,
        images: Vec::new(),
        expanded: true,
        active: true,
        failed: false,
        fingerprint: 0,
    });
}

pub fn append_thinking_delta(items: &mut Vec<TranscriptItem>, delta: &str, cx: &mut App) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = items.last_mut() {
        if last.kind == ItemKind::Thinking {
            last.raw_body.push_str(delta);
            last.active = true;
            last.title = "Thinking…".into();
            if last.expanded {
                if let Some(md) = &last.body_md {
                    md.update(cx, |state, cx| state.push_str(delta, cx));
                }
            }
            return;
        }
    }
    let next_id = items.last().map_or(0, |i| i.id + 1);
    items.push(TranscriptItem {
        id: next_id,
        entry_id: None,
        call_id: None,
        kind: ItemKind::Thinking,
        variant: CardVariant::None,
        verb: SharedString::default(),
        title: "Thinking…".into(),
        meta: SharedString::default(),
        body_md: None,
        raw_body: delta.to_string(),
        output: None,
        images: Vec::new(),
        expanded: false,
        active: true,
        failed: false,
        fingerprint: 0,
    });
}

pub fn start_tool_call(
    items: &mut Vec<TranscriptItem>,
    call_id: String,
    name: &str,
    args: &Value,
) {
    settle_thinking(items);
    let (variant, verb, summary, raw_code) = classify_tool_call(name, args, None);
    let next_id = items.last().map_or(0, |i| i.id + 1);
    items.push(TranscriptItem {
        id: next_id,
        entry_id: None,
        call_id: Some(call_id),
        kind: ItemKind::ToolCall,
        variant,
        verb: verb.into(),
        title: summary.into(),
        meta: "running".into(),
        body_md: None,
        raw_body: raw_code,
        output: None,
        images: Vec::new(),
        expanded: false,
        active: true,
        failed: false,
        fingerprint: 0,
    });
}

pub fn finish_tool_call(
    items: &mut [TranscriptItem],
    call_id: &str,
    output: String,
    is_error: bool,
    details: Option<&Value>,
    cx: &mut App,
) -> Option<usize> {
    let (ix, item) = items
        .iter_mut()
        .enumerate()
        .rev()
        .find(|(_, i)| i.kind == ItemKind::ToolCall && i.call_id.as_deref() == Some(call_id))?;
    let bash = parse_bash_commands(details);
    let dur = parse_duration_ms(details);
    if bash.is_some() {
        let args = serde_json::json!({"code": item.raw_body});
        let (variant, verb, summary, _) = classify_tool_call("python_repl", &args, bash.as_ref());
        item.variant = variant;
        item.verb = verb.into();
        item.title = summary.into();
    }
    item.active = false;
    item.failed = is_error || output.trim_start().starts_with("Traceback");
    item.output = if output.trim().is_empty() {
        None
    } else {
        Some(output)
    };
    item.meta = if item.failed {
        "failed".into()
    } else {
        format_duration(dur).into()
    };
    if item.expanded {
        let formatted = format_expanded_body(item);
        item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
    }
    Some(ix)
}

pub fn settle_all(items: &mut [TranscriptItem]) {
    for it in items.iter_mut() {
        it.active = false;
        if it.kind == ItemKind::Thinking {
            it.title = "Thought".into();
        }
        if it.kind == ItemKind::ToolCall && it.meta == "running" {
            it.meta = SharedString::default();
        }
    }
}

pub fn format_expanded_body(item: &TranscriptItem) -> String {
    match item.kind {
        ItemKind::ToolCall => {
            let lang = match item.variant {
                CardVariant::Python => "python",
                CardVariant::Bash => "bash",
                _ => "text",
            };
            let mut md = format!("```{lang}\n{}\n```", item.raw_body);
            if let Some(out) = &item.output {
                if !out.trim().is_empty() {
                    md.push_str("\n\n**Output**\n```text\n");
                    md.push_str(out.trim_end());
                    md.push_str("\n```");
                }
            }
            md
        }
        _ => item.raw_body.clone(),
    }
}

fn classify_tool_call(
    name: &str,
    args: &Value,
    bash: Option<&BashInfo>,
) -> (CardVariant, String, String, String) {
    let code = args
        .get("code")
        .or_else(|| args.get("command"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(args).unwrap_or_else(|_| args.to_string())
        });

    let trimmed_code = code.trim_end();

    if name.eq_ignore_ascii_case("bash") {
        let title = bash
            .map(|b| clip_line(&b.first, 72))
            .unwrap_or_else(|| summary_line(trimmed_code));
        return (CardVariant::Bash, "bash".to_string(), title, code);
    }

    if name == "python_repl" {
        let dominant =
            bash.filter(|b| b.lines * BASH_DOMINANCE_DIVISOR >= input_line_count(trimmed_code));
        if let Some(b) = dominant {
            let mut title = clip_line(&b.first, 72);
            if b.count > 1 {
                title.push_str(&format!(" · +{} more", b.count - 1));
            }
            return (CardVariant::Bash, "bash".to_string(), title, code);
        }
        if let Some(body) = strip_bash_cell_magic(trimmed_code) {
            return (
                CardVariant::Bash,
                "bash".to_string(),
                summary_line(body),
                code,
            );
        }
        if let Some(bash_preview) = extract_inline_bash_preview(trimmed_code) {
            return (CardVariant::Bash, "bash".to_string(), bash_preview, code);
        }
        return (
            CardVariant::Python,
            "python".to_string(),
            summary_line(trimmed_code),
            code,
        );
    }

    (
        CardVariant::ToolOther,
        name.to_string(),
        summary_line(trimmed_code),
        code,
    )
}

fn summary_line(code: &str) -> String {
    let line = code
        .lines()
        .map(str::trim)
        .map(|l| l.trim_end_matches(':'))
        .find(|l| !l.is_empty() && !l.starts_with('#'))
        .unwrap_or("empty cell");
    clip_line(line, 72)
}

fn parse_bracket_exit_code(header: &str) -> Option<i64> {
    let idx = header.find("exit:")?;
    let rest = &header[idx + 5..];
    let digits: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse::<i64>().ok()
}

fn classify_custom(ctype: &str, text: &str, details: &Value) -> (CardVariant, String, String, bool) {
    let first_line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();

    if matches!(ctype, "bash_done" | "bash-done" | "background_shell_done")
        || first_line.starts_with("[bash-done")
    {
        let exit_code = details
            .get("exitCode")
            .and_then(Value::as_i64)
            .or_else(|| parse_bracket_exit_code(first_line));
        let meta = exit_code.map(|c| format!("exit {c}")).unwrap_or_default();
        let failed = exit_code.is_some_and(|c| c != 0);
        let title = if first_line.starts_with("[bash-done") {
            let cmd = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("Command:"))
                .map(str::trim)
                .unwrap_or("");
            if cmd.is_empty() {
                "Background shell finished".to_string()
            } else {
                clip_line(cmd, 72)
            }
        } else {
            "Background shell finished".to_string()
        };
        return (CardVariant::BashDone, title, meta, failed);
    }

    if matches!(ctype, "agent_message" | "agent-message")
        || first_line.starts_with("[agent-message")
    {
        return (
            CardVariant::AgentMessage,
            "Agent message".to_string(),
            String::new(),
            false,
        );
    }

    if matches!(ctype, "provider_retry_outcome" | "provider-retry")
        || first_line.starts_with("[provider-retry")
    {
        let failed = details.get("success").and_then(Value::as_bool) == Some(false);
        return (
            CardVariant::ProviderRetry,
            "Provider retry".to_string(),
            if failed { "failed" } else { "recovered" }.to_string(),
            failed,
        );
    }

    if ctype == "session_slash_command" {
        let label = text
            .trim()
            .split_whitespace()
            .next()
            .unwrap_or("Command")
            .to_string();
        return (CardVariant::Harness, label, String::new(), false);
    }

    if ctype == "session_slash_command_result" {
        let cmd = details
            .pointer("/command/name")
            .or_else(|| details.get("command"))
            .or_else(|| details.get("name"))
            .and_then(Value::as_str)
            .map(|s| s.trim_start_matches('/'));
        let label = match cmd {
            Some("compact") => "Compaction",
            Some("refine") => "Refinement",
            Some("goal") => "Goal",
            Some("autonomous") => "Autonomous mode",
            _ => "Command result",
        };
        let failed = details.get("success").and_then(Value::as_bool) == Some(false);
        return (
            CardVariant::Harness,
            label.to_string(),
            if failed { "failed".into() } else { String::new() },
            failed,
        );
    }

    if ctype == "compaction_outcome" {
        let outcome = details.get("outcome").and_then(Value::as_str).unwrap_or("");
        let failed = matches!(outcome, "failed" | "cancelled");
        return (
            CardVariant::Harness,
            "Compaction".to_string(),
            outcome.to_string(),
            failed,
        );
    }

    if ctype == "refinement_outcome" {
        if let Some(edits) = details.get("edits").and_then(Value::as_array) {
            let total = edits.len();
            let applied = edits
                .iter()
                .filter(|e| e.get("applied").and_then(Value::as_bool) == Some(true))
                .count();
            let (label, failed) = if total == 0 {
                ("Refinement", false)
            } else if applied == 0 {
                ("Refinement failed", true)
            } else if applied < total {
                ("Refinement partial", false)
            } else {
                ("Refinement", false)
            };
            let meta = if total == 0 {
                String::new()
            } else {
                format!("{applied}/{total} edits")
            };
            return (CardVariant::Harness, label.to_string(), meta, failed);
        }
        let failed = details.get("success").and_then(Value::as_bool) == Some(false)
            || details.get("error").is_some();
        return (
            CardVariant::Harness,
            "Refinement".to_string(),
            if failed { "failed".into() } else { String::new() },
            failed,
        );
    }

    if ctype == "model_prompt_error" {
        return (
            CardVariant::Harness,
            "Model prompt error".to_string(),
            "failed".to_string(),
            true,
        );
    }

    if matches!(ctype, "auto_refinement" | "self_refinement") {
        return (
            CardVariant::Harness,
            ctype.replace('_', " "),
            String::new(),
            false,
        );
    }

    let first = if first_line.is_empty() {
        ctype.to_string()
    } else {
        clip_line(first_line, 72)
    };
    (CardVariant::Harness, first, String::new(), false)
}

fn extract_text(msg: &Value) -> String {
    if let Some(s) = msg.get("content").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        return arr
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
    }
    String::new()
}

fn extract_images(msg: &Value) -> Vec<Arc<Image>> {
    let mut out = Vec::new();
    let Some(arr) = msg.get("content").and_then(Value::as_array) else {
        return out;
    };
    for part in arr {
        if part.get("type").and_then(Value::as_str) != Some("image") {
            continue;
        }
        let mime = part
            .get("mimeType")
            .and_then(Value::as_str)
            .unwrap_or("image/png");
        let Some(data) = part.get("data").and_then(Value::as_str) else {
            continue;
        };
        let format = match mime {
            "image/jpeg" | "image/jpg" => ImageFormat::Jpeg,
            "image/gif" => ImageFormat::Gif,
            "image/webp" => ImageFormat::Webp,
            _ => ImageFormat::Png,
        };
        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
            out.push(Arc::new(Image::from_bytes(format, bytes)));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[::core::prelude::v1::test]
    fn classify_tool_call_distinguishes_python_and_bash_dominance() {
        let (v_py, verb_py, sum_py, _) = classify_tool_call(
            "python_repl",
            &json!({"code": "x = 1\ny = 2\nprint(x + y)"}),
            None,
        );
        assert_eq!(v_py, CardVariant::Python);
        assert_eq!(verb_py, "python");
        assert_eq!(sum_py, "x = 1");

        let bash_info = BashInfo {
            first: "git status -sb".into(),
            count: 2,
            lines: 2,
        };
        let (v_dom, verb_dom, sum_dom, _) = classify_tool_call(
            "python_repl",
            &json!({"code": "r = await bash('git status -sb')\nprint(r.output)"}),
            Some(&bash_info),
        );
        assert_eq!(v_dom, CardVariant::Bash);
        assert_eq!(verb_dom, "bash");
        assert_eq!(sum_dom, "git status -sb · +1 more");

        let (v_live, verb_live, sum_live, _) = classify_tool_call(
            "python_repl",
            &json!({"code": "await bash('cargo test -p pa-gpui')"}),
            None,
        );
        assert_eq!(v_live, CardVariant::Bash);
        assert_eq!(verb_live, "bash");
        assert_eq!(sum_live, "cargo test -p pa-gpui");
    }

    #[::core::prelude::v1::test]
    fn classify_custom_identifies_daemon_cards() {
        let (v_bd, _, meta_bd, fail_bd) =
            classify_custom("bash_done", "", &json!({"exitCode": 1}));
        assert_eq!(v_bd, CardVariant::BashDone);
        assert_eq!(meta_bd, "exit 1");
        assert!(fail_bd);

        let (v_brk, title_brk, meta_brk, fail_brk) = classify_custom(
            "notice",
            "[bash-done pid:42 exit:101]\n\nCommand: \"cargo build\"",
            &Value::Null,
        );
        assert_eq!(v_brk, CardVariant::BashDone);
        assert_eq!(title_brk, "\"cargo build\"");
        assert_eq!(meta_brk, "exit 101");
        assert!(fail_brk);

        let (v_pr, _, meta_pr, fail_pr) =
            classify_custom("provider_retry_outcome", "", &json!({"success": true}));
        assert_eq!(v_pr, CardVariant::ProviderRetry);
        assert_eq!(meta_pr, "recovered");
        assert!(!fail_pr);

        let (_, title_sr, _, fail_sr) = classify_custom(
            "session_slash_command_result",
            "Refined 2 items",
            &json!({"command": {"name": "refine"}, "success": true}),
        );
        assert_eq!(title_sr, "Refinement");
        assert!(!fail_sr);

        let (_, title_co, meta_co, fail_co) = classify_custom(
            "compaction_outcome",
            "Compacted",
            &json!({"outcome": "failed"}),
        );
        assert_eq!(title_co, "Compaction");
        assert_eq!(meta_co, "failed");
        assert!(fail_co);

        let (_, title_ro, meta_ro, fail_ro) = classify_custom(
            "refinement_outcome",
            "Done",
            &json!({"edits": [{"applied": true}, {"applied": false}]}),
        );
        assert_eq!(title_ro, "Refinement partial");
        assert_eq!(meta_ro, "1/2 edits");
        assert!(!fail_ro);
    }

    #[::core::prelude::v1::test]
    fn start_tool_call_settles_active_thinking() {
        let mut items = vec![TranscriptItem {
            id: 0,
            entry_id: None,
            call_id: None,
            kind: ItemKind::Thinking,
            variant: CardVariant::None,
            verb: SharedString::default(),
            title: "Thinking…".into(),
            meta: SharedString::default(),
            body_md: None,
            raw_body: "planning".into(),
            output: None,
            images: Vec::new(),
            expanded: false,
            active: true,
            failed: false,
            fingerprint: 0,
        }];
        start_tool_call(&mut items, "c1".into(), "python_repl", &json!({"code": "1+1"}));
        assert!(!items[0].active);
        assert_eq!(items[0].title.as_ref(), "Thought");
        assert!(items[1].active);
    }

    #[::core::prelude::v1::test]
    fn build_items_keeps_all_cards_collapsed_and_supports_window_trim_and_tail_merge() {
        let msgs = vec![
            json!({"id": "e1", "role": "compactionSummary", "summary": "Long summary", "tokensBefore": 5000}),
            json!({"id": "e2", "role": "custom", "customType": "session_slash_command", "content": "/compact"}),
            json!({"id": "e3", "role": "custom", "customType": "compaction_outcome", "content": "Compacted", "details": {"outcome": "completed"}}),
            json!({"id": "e4", "role": "user", "content": "hi"}),
            json!({"id": "e5", "role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm"},
                {"type": "text", "text": "hello"}
            ]}),
        ];
        let items = build_items(&msgs);
        assert_eq!(items.len(), 6);
        // All card rows start collapsed with zero eager TextViewState entities.
        assert!(!items[0].expanded); // Summary
        assert!(items[1].kind == ItemKind::User && items[1].expanded); // /compact as user message
        assert!(!items[2].expanded); // Custom compaction_outcome
        assert!(items[3].kind == ItemKind::User && items[3].expanded);
        assert!(!items[4].expanded); // Thinking
        assert!(items[5].kind == ItemKind::Assistant && items[5].expanded);
        assert!(items.iter().all(|it| it.body_md.is_none()));

        // Tail trim lands cleanly on an entry_id boundary without splitting multi-block assistant e5.
        let (drop_count, next_cursor) = compute_tail_trim(&items, 4, 2).expect("should trim");
        assert_eq!(drop_count, 4);
        assert_eq!(next_cursor, "e5");

        // Tail merge finds exact overlap without resetting loaded prefix.
        let tail = build_items(&[
            msgs[3].clone(),
            msgs[4].clone(),
            json!({"id": "e6", "role": "user", "content": "next"}),
        ]);
        let (ex_at, in_at) = find_tail_merge_start(&items, &tail, "e4").expect("overlap");
        assert_eq!(ex_at, 6);
        assert_eq!(in_at, 3);
    }

    #[::core::prelude::v1::test]
    fn format_duration_matches_pa_ui() {
        assert_eq!(format_duration(Some(250)), "250ms");
        assert_eq!(format_duration(Some(12340)), "12.3s");
        assert_eq!(format_duration(Some(304000)), "5m 04s");
    }
}
