//! Model catalog & thinking-level ladder module.

use crate::Client;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub provider: String,
    /// The provider has credentials, so this model can actually be selected.
    pub configured: bool,
    pub context_window: u64,
    /// The thinking levels this model accepts, weakest first (`off` for a model
    /// with no thinking surface). Empty when the catalog says nothing at all.
    pub thinking_levels: Vec<String>,
    /// The model takes image input.
    pub supports_images: bool,
}

/// The thinking-level ladder, weakest to strongest — the daemon's
/// `EXTENDED_THINKING_LEVELS`, whose wire names are the level strings.
pub const THINKING_LEVELS: [&str; 7] = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

fn supports_thinking(reasoning: bool, map: Option<&Value>) -> bool {
    reasoning
        || map
            .and_then(Value::as_object)
            .is_some_and(|map| map.values().any(|value| !value.is_null()))
}

fn takes_images(model: &Value) -> bool {
    let list = model
        .get("modalities")
        .and_then(|modalities| modalities.get("input"))
        .or_else(|| model.get("input"))
        .and_then(Value::as_array);
    list.is_some_and(|list| list.iter().any(|item| item.as_str() == Some("image")))
}

pub(crate) fn supported_levels(reasoning: bool, map: Option<&Value>) -> Vec<String> {
    if !supports_thinking(reasoning, map) {
        return vec!["off".to_string()];
    }
    let map = map.and_then(Value::as_object);
    THINKING_LEVELS
        .iter()
        .filter(|level| match map.and_then(|map| map.get(**level)) {
            None => !matches!(**level, "xhigh" | "max"),
            Some(Value::Null) => false,
            Some(_) => true,
        })
        .map(|level| (*level).to_string())
        .collect()
}

/// Where a requested level lands on a model's own ladder: the level itself when
/// the model takes it, else the nearest higher rung, else the nearest lower one.
#[must_use]
pub fn clamp_level(levels: &[String], level: &str) -> String {
    if levels.iter().any(|available| available == level) {
        return level.to_string();
    }
    let Some(index) = THINKING_LEVELS
        .iter()
        .position(|candidate| *candidate == level)
    else {
        return levels.first().cloned().unwrap_or_default();
    };
    for candidate in THINKING_LEVELS.iter().skip(index) {
        if levels.iter().any(|available| available == candidate) {
            return (*candidate).to_string();
        }
    }
    for candidate in THINKING_LEVELS[..index].iter().rev() {
        if levels.iter().any(|available| available == candidate) {
            return (*candidate).to_string();
        }
    }
    levels.first().cloned().unwrap_or_default()
}

/// A level's display name: the wire words are lowercase, the menu is not.
#[must_use]
pub fn thinking_level_label(level: &str) -> &str {
    match level {
        "off" => "Off",
        "minimal" => "Minimal",
        "low" => "Low",
        "medium" => "Medium",
        "high" => "High",
        "xhigh" => "X-High",
        "max" => "Max",
        other => other,
    }
}

/// Every model the daemon knows, with the providers that are signed in.
///
/// # Errors
/// Fails on a transport error or a refused request.
pub fn model_catalog(client: &Client, session: &str) -> std::io::Result<Vec<ModelEntry>> {
    let response = client.request(
        "get_model_catalog",
        json!({"activeSessionId": session}),
        Duration::from_secs(20),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "get_model_catalog failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let data = response.get("data");
    let configured: Vec<String> = data
        .and_then(|data| data.get("configuredProviders"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let entries = data
        .and_then(|data| data.get("models"))
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|model| {
                    let provider = model.get("provider").and_then(Value::as_str)?.to_string();
                    let id = model.get("id").and_then(Value::as_str)?.to_string();
                    let name = model
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(&id)
                        .to_string();
                    let reasoning = model
                        .get("reasoning")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    Some(ModelEntry {
                        supports_images: takes_images(model),
                        configured: configured.contains(&provider),
                        context_window: model
                            .get("contextWindow")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        thinking_levels: supported_levels(
                            reasoning,
                            model.get("thinkingLevelMap"),
                        ),
                        id,
                        name,
                        provider,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(entries)
}
