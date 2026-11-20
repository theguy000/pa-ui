use gpui_kit::component::text::TextViewState;
use gpui_kit::*;
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    User,
    Assistant,
    Thinking,
    ToolCall,
    Summary,
    Custom,
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    pub call_id: Option<String>,
    pub kind: ItemKind,
    pub variant: CardVariant,
    pub verb: SharedString,
    pub title: SharedString,
    pub meta: SharedString,
    pub body_md: Option<Entity<TextViewState>>,
    pub raw_body: String,
    pub output: Option<String>,
    pub expanded: bool,
    pub active: bool,
    pub failed: bool,
}

pub fn build_items(messages: &[Value], cx: &mut App) -> Vec<TranscriptItem> {
    // First pass: index toolResult messages by toolCallId
    let mut results: HashMap<String, (String, bool)> = HashMap::new();
    for msg in messages {
        if msg.get("role").and_then(Value::as_str) == Some("toolResult") {
            let cid = msg
                .get("toolCallId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !cid.is_empty() {
                let text = extract_text(msg);
                let is_err = msg.get("isError").and_then(Value::as_bool).unwrap_or(false);
                results.insert(cid, (text, is_err));
            }
        }
    }

    let mut items = Vec::new();
    let mut next_id = 0usize;

    for msg in messages {
        let role = msg.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "user" => {
                let text = extract_text(msg);
                if text.trim().is_empty() {
                    continue;
                }
                let md = cx.new(|cx| TextViewState::markdown(&text, cx));
                items.push(TranscriptItem {
                    id: next_id,
                    call_id: None,
                    kind: ItemKind::User,
                    variant: CardVariant::None,
                    verb: SharedString::default(),
                    title: "You".into(),
                    meta: SharedString::default(),
                    body_md: Some(md),
                    raw_body: text,
                    output: None,
                    expanded: true,
                    active: false,
                    failed: false,
                });
                next_id += 1;
            }
            "assistant" => {
                if let Some(err) = msg.get("errorMessage").and_then(Value::as_str) {
                    if !err.trim().is_empty() {
                        items.push(TranscriptItem {
                            id: next_id,
                            call_id: None,
                            kind: ItemKind::Error,
                            variant: CardVariant::None,
                            verb: SharedString::default(),
                            title: "Error".into(),
                            meta: SharedString::default(),
                            body_md: None,
                            raw_body: err.to_string(),
                            output: None,
                            expanded: true,
                            active: false,
                            failed: true,
                        });
                        next_id += 1;
                    }
                }
                if let Some(parts) = msg.get("content").and_then(Value::as_array) {
                    for part in parts {
                        let ptype = part.get("type").and_then(Value::as_str).unwrap_or_default();
                        match ptype {
                            "thinking" => {
                                let t = part
                                    .get("thinking")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .trim();
                                if t.is_empty() {
                                    continue;
                                }
                                items.push(TranscriptItem {
                                    id: next_id,
                                    call_id: None,
                                    kind: ItemKind::Thinking,
                                    variant: CardVariant::None,
                                    verb: "Thought".into(),
                                    title: SharedString::default(),
                                    meta: SharedString::default(),
                                    body_md: None,
                                    raw_body: t.to_string(),
                                    output: None,
                                    expanded: false,
                                    active: false,
                                    failed: false,
                                });
                                next_id += 1;
                            }
                            "text" => {
                                let t = part
                                    .get("text")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .trim();
                                if t.is_empty() {
                                    continue;
                                }
                                let md = cx.new(|cx| TextViewState::markdown(t, cx));
                                items.push(TranscriptItem {
                                    id: next_id,
                                    call_id: None,
                                    kind: ItemKind::Assistant,
                                    variant: CardVariant::None,
                                    verb: SharedString::default(),
                                    title: SharedString::default(),
                                    meta: SharedString::default(),
                                    body_md: Some(md),
                                    raw_body: t.to_string(),
                                    output: None,
                                    expanded: true,
                                    active: false,
                                    failed: false,
                                });
                                next_id += 1;
                            }
                            "toolCall" => {
                                let cid = part
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .map(str::to_string);
                                let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                                let args = part.get("arguments").cloned().unwrap_or(Value::Null);
                                let (variant, verb, summary, code) = format_tool_call(name, &args);
                                let (output, failed) = cid
                                    .as_deref()
                                    .and_then(|id| results.get(id))
                                    .map(|(out, err)| (Some(out.clone()), *err))
                                    .unwrap_or((None, false));

                                items.push(TranscriptItem {
                                    id: next_id,
                                    call_id: cid,
                                    kind: ItemKind::ToolCall,
                                    variant,
                                    verb: verb.into(),
                                    title: summary.into(),
                                    meta: SharedString::default(),
                                    body_md: None,
                                    raw_body: code,
                                    output,
                                    expanded: false,
                                    active: false,
                                    failed,
                                });
                                next_id += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
            "compactionSummary" | "branchSummary" => {
                let summary = msg
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .trim();
                let tokens = msg.get("tokensBefore").and_then(Value::as_u64);
                let meta = tokens
                    .map(|n| format!("{n} tokens before"))
                    .unwrap_or_default();
                items.push(TranscriptItem {
                    id: next_id,
                    call_id: None,
                    kind: ItemKind::Summary,
                    variant: CardVariant::Harness,
                    verb: SharedString::default(),
                    title: if role == "compactionSummary" {
                        "Context compacted".into()
                    } else {
                        "Branch summary".into()
                    },
                    meta: meta.into(),
                    body_md: None,
                    raw_body: summary.to_string(),
                    output: None,
                    expanded: false,
                    active: false,
                    failed: false,
                });
                next_id += 1;
            }
            "custom" => {
                let ctype = msg
                    .get("customType")
                    .and_then(Value::as_str)
                    .unwrap_or("notice");
                let text = extract_text(msg);
                let details = msg.get("details").unwrap_or(&Value::Null);
                let (variant, title, meta, failed) = classify_custom(ctype, &text, details);
                items.push(TranscriptItem {
                    id: next_id,
                    call_id: None,
                    kind: ItemKind::Custom,
                    variant,
                    verb: SharedString::default(),
                    title: title.into(),
                    meta: meta.into(),
                    body_md: None,
                    raw_body: text,
                    output: None,
                    expanded: false,
                    active: false,
                    failed,
                });
                next_id += 1;
            }
            _ => {}
        }
    }

    items
}

pub fn append_text_delta(items: &mut Vec<TranscriptItem>, delta: &str, cx: &mut App) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = items.last_mut() {
        if last.kind == ItemKind::Assistant && last.active {
            last.raw_body.push_str(delta);
            if let Some(md) = last.body_md.as_ref() {
                md.update(cx, |state, cx| state.push_str(delta, cx));
            }
            return;
        }
    }
    let next_id = items.last().map_or(0, |i| i.id + 1);
    let md = cx.new(|cx| TextViewState::markdown(delta, cx));
    items.push(TranscriptItem {
        id: next_id,
        call_id: None,
        kind: ItemKind::Assistant,
        variant: CardVariant::None,
        verb: SharedString::default(),
        title: SharedString::default(),
        meta: SharedString::default(),
        body_md: Some(md),
        raw_body: delta.to_string(),
        output: None,
        expanded: true,
        active: true,
        failed: false,
    });
}

pub fn append_thinking_delta(items: &mut Vec<TranscriptItem>, delta: &str, cx: &mut App) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = items.last_mut() {
        if last.kind == ItemKind::Thinking && last.active {
            last.raw_body.push_str(delta);
            if last.expanded {
                if let Some(md) = last.body_md.as_ref() {
                    md.update(cx, |state, cx| state.push_str(delta, cx));
                }
            }
            return;
        }
    }
    let next_id = items.last().map_or(0, |i| i.id + 1);
    items.push(TranscriptItem {
        id: next_id,
        call_id: None,
        kind: ItemKind::Thinking,
        variant: CardVariant::None,
        verb: "Thinking".into(),
        title: SharedString::default(),
        meta: SharedString::default(),
        body_md: None,
        raw_body: delta.to_string(),
        output: None,
        expanded: false,
        active: true,
        failed: false,
    });
}

pub fn start_tool_call(items: &mut Vec<TranscriptItem>, call_id: String, name: &str, args: &Value) {
    // Mark any prior active thinking/text as settled
    for it in items.iter_mut() {
        if it.active && matches!(it.kind, ItemKind::Thinking | ItemKind::Assistant) {
            it.active = false;
            if it.kind == ItemKind::Thinking {
                it.verb = "Thought".into();
            }
        }
    }
    let (variant, verb, summary, code) = format_tool_call(name, args);
    let next_id = items.last().map_or(0, |i| i.id + 1);
    items.push(TranscriptItem {
        id: next_id,
        call_id: Some(call_id),
        kind: ItemKind::ToolCall,
        variant,
        verb: verb.into(),
        title: summary.into(),
        meta: SharedString::default(),
        body_md: None,
        raw_body: code,
        output: None,
        expanded: false,
        active: true,
        failed: false,
    });
}

pub fn finish_tool_call(
    items: &mut Vec<TranscriptItem>,
    call_id: &str,
    output: String,
    is_error: bool,
    cx: &mut App,
) {
    if let Some(item) = items
        .iter_mut()
        .rev()
        .find(|i| i.call_id.as_deref() == Some(call_id))
    {
        item.active = false;
        item.failed = is_error;
        item.output = Some(output);
        if item.expanded {
            let formatted = format_expanded_body(item);
            item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
        }
    }
}

pub fn settle_all(items: &mut [TranscriptItem]) {
    for it in items.iter_mut() {
        if it.active {
            it.active = false;
            if it.kind == ItemKind::Thinking {
                it.verb = "Thought".into();
            }
        }
    }
}

pub fn format_expanded_body(item: &TranscriptItem) -> String {
    match item.kind {
        ItemKind::ToolCall => {
            let lang = match item.variant {
                CardVariant::Python => "python",
                CardVariant::Bash => "bash",
                _ => "",
            };
            let mut md = format!("```{lang}\n{}\n```", item.raw_body.trim_end());
            if let Some(out) = item.output.as_deref() {
                let trimmed = out.trim();
                if !trimmed.is_empty() {
                    let clipped: String = trimmed.chars().take(12_000).collect();
                    md.push_str(&format!("\n\n**Output**\n```\n{clipped}\n```"));
                }
            }
            md
        }
        _ => item.raw_body.clone(),
    }
}

fn format_tool_call(name: &str, args: &Value) -> (CardVariant, String, String, String) {
    let lower = name.to_ascii_lowercase();
    let (variant, verb) = if lower.contains("python") || lower.contains("repl") {
        (CardVariant::Python, "Python".to_string())
    } else if lower.contains("bash") || lower.contains("shell") {
        (CardVariant::Bash, "Bash".to_string())
    } else {
        (CardVariant::ToolOther, name.to_string())
    };

    let code = args
        .get("code")
        .or_else(|| args.get("command"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| serde_json::to_string_pretty(args).unwrap_or_default());

    let summary = code
        .lines()
        .find(|l| {
            let t = l.trim();
            !t.is_empty() && !t.starts_with('#')
        })
        .or_else(|| code.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .chars()
        .take(88)
        .collect::<String>();

    (variant, verb, summary, code)
}

fn classify_custom(ctype: &str, text: &str, details: &Value) -> (CardVariant, String, String, bool) {
    match ctype {
        "bash_done" | "background_shell_done" => {
            let exit_code = details.get("exitCode").and_then(Value::as_i64);
            let meta = exit_code.map(|c| format!("exit {c}")).unwrap_or_default();
            let failed = exit_code.is_some_and(|c| c != 0);
            (
                CardVariant::BashDone,
                "Background shell finished".to_string(),
                meta,
                failed,
            )
        }
        "agent_message" => (
            CardVariant::AgentMessage,
            "Agent message".to_string(),
            String::new(),
            false,
        ),
        "provider_retry_outcome" => {
            let failed = details.get("success").and_then(Value::as_bool) == Some(false);
            (
                CardVariant::ProviderRetry,
                "Provider retry".to_string(),
                if failed { "failed" } else { "recovered" }.to_string(),
                failed,
            )
        }
        "session_slash_command_result" | "auto_refinement" | "self_refinement" => (
            CardVariant::Harness,
            if ctype == "session_slash_command_result" {
                "Refinement".to_string()
            } else {
                ctype.replace('_', " ")
            },
            String::new(),
            false,
        ),
        _ => {
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or(ctype)
                .trim()
                .chars()
                .take(72)
                .collect::<String>();
            (CardVariant::Harness, first, String::new(), false)
        }
    }
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
