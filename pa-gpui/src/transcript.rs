use gpui_kit::component::text::TextViewState;
use gpui_kit::*;
use serde_json::Value;

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

pub struct TranscriptItem {
    pub id: usize,
    pub kind: ItemKind,
    pub title: SharedString,
    pub meta: SharedString,
    pub body_md: Option<Entity<TextViewState>>,
    pub raw_body: String,
    pub expanded: bool,
    pub failed: bool,
}

pub fn build_items(messages: &[Value], cx: &mut App) -> Vec<TranscriptItem> {
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
                    kind: ItemKind::User,
                    title: "You".into(),
                    meta: SharedString::default(),
                    body_md: Some(md),
                    raw_body: text,
                    expanded: true,
                    failed: false,
                });
                next_id += 1;
            }
            "assistant" => {
                if let Some(err) = msg.get("errorMessage").and_then(Value::as_str) {
                    if !err.trim().is_empty() {
                        items.push(TranscriptItem {
                            id: next_id,
                            kind: ItemKind::Error,
                            title: "Error".into(),
                            meta: SharedString::default(),
                            body_md: None,
                            raw_body: err.to_string(),
                            expanded: true,
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
                                let t = part.get("thinking").and_then(Value::as_str).unwrap_or_default().trim();
                                if t.is_empty() {
                                    continue;
                                }
                                items.push(TranscriptItem {
                                    id: next_id,
                                    kind: ItemKind::Thinking,
                                    title: "Thought".into(),
                                    meta: SharedString::default(),
                                    body_md: None,
                                    raw_body: t.to_string(),
                                    expanded: false,
                                    failed: false,
                                });
                                next_id += 1;
                            }
                            "text" => {
                                let t = part.get("text").and_then(Value::as_str).unwrap_or_default().trim();
                                if t.is_empty() {
                                    continue;
                                }
                                let md = cx.new(|cx| TextViewState::markdown(t, cx));
                                items.push(TranscriptItem {
                                    id: next_id,
                                    kind: ItemKind::Assistant,
                                    title: SharedString::default(),
                                    meta: SharedString::default(),
                                    body_md: Some(md),
                                    raw_body: t.to_string(),
                                    expanded: true,
                                    failed: false,
                                });
                                next_id += 1;
                            }
                            "toolCall" => {
                                let name = part.get("name").and_then(Value::as_str).unwrap_or("tool");
                                let args = part.get("arguments").cloned().unwrap_or(Value::Null);
                                let code = args
                                    .get("code")
                                    .or_else(|| args.get("command"))
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                                    .unwrap_or_else(|| serde_json::to_string_pretty(&args).unwrap_or_default());
                                let first_line = code
                                    .lines()
                                    .find(|l| !l.trim().is_empty())
                                    .unwrap_or("")
                                    .trim()
                                    .chars()
                                    .take(72)
                                    .collect::<String>();
                                let title = if first_line.is_empty() {
                                    name.to_string()
                                } else {
                                    format!("{name} · {first_line}")
                                };
                                items.push(TranscriptItem {
                                    id: next_id,
                                    kind: ItemKind::ToolCall,
                                    title: title.into(),
                                    meta: SharedString::default(),
                                    body_md: None,
                                    raw_body: code,
                                    expanded: false,
                                    failed: false,
                                });
                                next_id += 1;
                            }
                            _ => {}
                        }
                    }
                }
            }
            "compactionSummary" | "branchSummary" => {
                let summary = msg.get("summary").and_then(Value::as_str).unwrap_or_default().trim();
                let tokens = msg.get("tokensBefore").and_then(Value::as_u64);
                let meta = tokens.map(|n| format!("{n} tokens before")).unwrap_or_default();
                items.push(TranscriptItem {
                    id: next_id,
                    kind: ItemKind::Summary,
                    title: if role == "compactionSummary" {
                        "Context compacted".into()
                    } else {
                        "Branch summary".into()
                    },
                    meta: meta.into(),
                    body_md: None,
                    raw_body: summary.to_string(),
                    expanded: false,
                    failed: false,
                });
                next_id += 1;
            }
            "custom" => {
                let ctype = msg.get("customType").and_then(Value::as_str).unwrap_or("notice");
                let text = extract_text(msg);
                items.push(TranscriptItem {
                    id: next_id,
                    kind: ItemKind::Custom,
                    title: ctype.replace('_', " ").into(),
                    meta: SharedString::default(),
                    body_md: None,
                    raw_body: text,
                    expanded: false,
                    failed: false,
                });
                next_id += 1;
            }
            _ => {}
        }
    }

    items
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
