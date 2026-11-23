use crate::diff_model::DIFF_DEFAULT_WIDTH;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffScope {
    #[default]
    Uncommitted,
    Branch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub dark_mode: bool,
    pub sidebar_open: bool,
    pub right_open: bool,
    pub diff_width: f32,
    pub diff_scope: DiffScope,
    pub diff_split: bool,
    pub diff_wrap: bool,
    pub diff_jump_collapsed: bool,
    pub active_closed: bool,
    pub idle_closed: bool,
    pub done_closed: bool,
    pub last_folder: Option<String>,
    pub recent_folders: Vec<String>,
    pub model_thinking: BTreeMap<String, String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            dark_mode: true,
            sidebar_open: true,
            right_open: false,
            diff_width: DIFF_DEFAULT_WIDTH,
            diff_scope: DiffScope::Uncommitted,
            diff_split: false,
            diff_wrap: false,
            diff_jump_collapsed: false,
            active_closed: false,
            idle_closed: false,
            done_closed: false,
            last_folder: None,
            recent_folders: Vec::new(),
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

    #[cfg(test)]
    pub fn record_recent_folder(&mut self, folder: &str) {
        self.record_recent_project(folder, folder);
    }

    pub fn record_recent_project(&mut self, project_root: &str, active_cwd: &str) {
        let project = project_root.trim();
        let active = active_cwd.trim();
        if !active.is_empty() {
            self.last_folder = Some(active.to_string());
        }
        if project.is_empty() {
            return;
        }
        self.recent_folders.retain(|existing| {
            #[cfg(windows)]
            {
                !existing.eq_ignore_ascii_case(project)
            }
            #[cfg(not(windows))]
            {
                existing != project
            }
        });
        self.recent_folders.insert(0, project.to_string());
        self.recent_folders.truncate(8);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn diff_preferences_round_trip_through_json() {
        let mut settings = AppSettings::default();
        settings.right_open = true;
        settings.diff_width = 560.0;
        settings.diff_scope = DiffScope::Branch;
        settings.diff_split = true;
        settings.diff_wrap = true;
        settings.diff_jump_collapsed = true;

        let json = serde_json::to_string(&settings).expect("serialize settings");
        let parsed: AppSettings = serde_json::from_str(&json).expect("deserialize settings");
        assert!(parsed.right_open);
        assert_eq!(parsed.diff_width, 560.0);
        assert_eq!(parsed.diff_scope, DiffScope::Branch);
        assert!(parsed.diff_split);
        assert!(parsed.diff_wrap);
        assert!(parsed.diff_jump_collapsed);
    }

    #[::core::prelude::v1::test]
    fn legacy_settings_json_fills_diff_defaults() {
        let legacy = r#"{"dark_mode":false,"sidebar_open":false,"right_open":true}"#;
        let parsed: AppSettings = serde_json::from_str(legacy).expect("deserialize legacy");
        assert!(!parsed.dark_mode);
        assert!(!parsed.sidebar_open);
        assert!(parsed.right_open);
        assert_eq!(parsed.diff_width, DIFF_DEFAULT_WIDTH);
        assert_eq!(parsed.diff_scope, DiffScope::Uncommitted);
        assert!(!parsed.diff_split);
        assert!(!parsed.diff_wrap);
        assert!(!parsed.diff_jump_collapsed);
        assert_eq!(parsed.last_folder, None);
        assert!(parsed.recent_folders.is_empty());
    }

    #[::core::prelude::v1::test]
    fn recent_folders_deduplicate_and_cap_at_eight() {
        let mut settings = AppSettings::default();
        for i in 0..10 {
            settings.record_recent_folder(&format!("/repo/p{i}"));
        }
        assert_eq!(settings.last_folder.as_deref(), Some("/repo/p9"));
        assert_eq!(settings.recent_folders.len(), 8);
        assert_eq!(settings.recent_folders[0], "/repo/p9");

        settings.record_recent_folder("/repo/p5");
        assert_eq!(settings.last_folder.as_deref(), Some("/repo/p5"));
        assert_eq!(settings.recent_folders[0], "/repo/p5");
        assert_eq!(
            settings
                .recent_folders
                .iter()
                .filter(|p| p.as_str() == "/repo/p5")
                .count(),
            1
        );
    }
}
