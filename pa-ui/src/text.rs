//! Small formatting: labels, token counts, widths, and the text inside a protocol message.

/// `river_ai` reads better as `river ai`; the catalog's own names are long.
pub(crate) fn provider_label(provider: &str) -> String {
    provider.replace('_', " ")
}


/// The catalog's `name` repeats the provider ("... (river.ai)"); the group
/// header already says it.
pub(crate) fn model_label(name: &str, id: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return id.to_string();
    }
    match trimmed.rfind(" (") {
        Some(at) if trimmed.ends_with(')') => trimmed[..at].to_string(),
        _ => trimmed.to_string(),
    }
}


/// A rough size for the right-hand hint: `200k`, `1M`.
pub(crate) fn context_hint(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        String::new()
    }
}


/// 1234 ? "1.2k", 47_011_000 ? "47.0M".
pub(crate) fn tokens(value: u64) -> String {
    if value >= 1_000_000 {
        format!("{:.1}M", value as f64 / 1_000_000.0)
    } else if value >= 1_000 {
        format!("{:.0}k", value as f64 / 1_000.0)
    } else {
        value.to_string()
    }
}


/// Concatenated text of a wire message's content parts.
pub(crate) fn message_text(message: &serde_json::Value) -> String {
    message
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text"))
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}


/// Tool results arrive as a model-facing value: sometimes a string, sometimes
/// `{ output, ... }`, sometimes a list of content blocks. Take the text.
pub(crate) fn tool_output_text(result: Option<&serde_json::Value>) -> String {
    fn text_of(value: &serde_json::Value, depth: usize) -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text.clone()),
            serde_json::Value::Array(items) => {
                let parts: Vec<String> = items.iter().filter_map(|item| text_of(item, depth + 1)).collect();
                (!parts.is_empty()).then(|| parts.join("\n"))
            }
            serde_json::Value::Object(map) if depth < 4 => {
                for key in ["text", "output", "stdout", "content", "result", "message"] {
                    if let Some(inner) = map.get(key) {
                        if let Some(text) = text_of(inner, depth + 1) {
                            return Some(text);
                        }
                    }
                }
                Some(value.to_string())
            }
            _ => None,
        }
    }
    result
        .and_then(|value| text_of(value, 0))
        .unwrap_or_default()
}


pub(crate) fn trim_to(text: &str, width: usize) -> String {
    let cleaned = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.chars().count() <= width {
        return cleaned;
    }
    cleaned.chars().take(width.saturating_sub(1)).chain(['\u{2026}']).collect()
}
