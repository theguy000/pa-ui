use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub dark_mode: bool,
    pub sidebar_open: bool,
    pub right_open: bool,
    pub active_closed: bool,
    pub idle_closed: bool,
    pub done_closed: bool,
    pub model_thinking: BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            sidebar_open: true,
            right_open: false,
            active_closed: false,
            idle_closed: false,
            done_closed: false,
            model_thinking: BTreeMap::new(),
        }
    }
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(home.join(".prime").join("agent").join("pa-gpui-settings.json"))
}

impl AppSettings {
    pub fn load() -> Self {
        let Some(path) = settings_path() else {
            return Self::default();
        };
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = settings_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, json);
        }
    }

    pub fn model_thinking_for(&self, provider: &str, model_id: &str) -> Option<&str> {
        let key = format!("{provider}/{model_id}");
        self.model_thinking.get(&key).map(String::as_str)
    }

    pub fn set_model_thinking(&mut self, provider: &str, model_id: &str, level: &str) {
        if provider.is_empty() || model_id.is_empty() || level.is_empty() {
            return;
        }
        let key = format!("{provider}/{model_id}");
        if self.model_thinking.get(&key).map(String::as_str) == Some(level) {
            return;
        }
        self.model_thinking.insert(key, level.to_string());
        self.save();
    }
}
