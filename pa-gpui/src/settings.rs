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
    pub live_radar_closed: bool,
    pub collapsed_workspaces: Vec<String>,
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
            live_radar_closed: false,
            collapsed_workspaces: Vec::new(),
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

    pub fn is_workspace_collapsed(&self, key: &str) -> bool {
        self.collapsed_workspaces.iter().any(|k| k == key)
    }

    pub fn toggle_workspace_collapsed(&mut self, key: &str) {
        if let Some(pos) = self.collapsed_workspaces.iter().position(|k| k == key) {
            self.collapsed_workspaces.remove(pos);
        } else {
            self.collapsed_workspaces.push(key.to_string());
        }
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
        let settings = AppSettings {
            right_open: true,
            diff_width: 560.0,
            diff_scope: DiffScope::Branch,
            diff_split: true,
            diff_wrap: true,
            diff_jump_collapsed: true,
            ..Default::default()
        };

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


fn agent_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("PRIME_AGENT_CODING_AGENT_DIR").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(PathBuf::from(home).join(".prime").join("agent"))
}

/// Path to `~/.prime/agent/settings.json` (shared with the daemon).
pub fn agent_settings_path() -> Option<PathBuf> {
    Some(agent_dir()?.join("settings.json"))
}

/// Default worktree root (`~/.prime/agent/worktrees`).
pub fn default_worktree_root() -> String {
    agent_dir()
        .map(|d| d.join("worktrees").to_string_lossy().into_owned())
        .unwrap_or_else(|| "~/.prime/agent/worktrees".to_string())
}

/// Read `worktreeRoot` from `~/.prime/agent/settings.json`, falling back to default.
pub fn load_agent_worktree_root() -> String {
    if let Some(path) = agent_settings_path() {
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) {
                if let Some(root) = val
                    .get("worktreeRoot")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    return root.to_string();
                }
            }
        }
    }
    default_worktree_root()
}

/// Atomically update `worktreeRoot` in `~/.prime/agent/settings.json` without touching other keys.
pub fn save_agent_worktree_root(root: Option<&str>) -> std::io::Result<String> {
    let Some(path) = agent_settings_path() else {
        return Err(std::io::Error::other("home directory unavailable"));
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut obj = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let effective = match root.map(str::trim).filter(|s| !s.is_empty()) {
        Some(custom) => {
            obj.insert(
                "worktreeRoot".to_string(),
                serde_json::Value::String(custom.to_string()),
            );
            custom.to_string()
        }
        None => {
            obj.remove("worktreeRoot");
            default_worktree_root()
        }
    };
    let tmp = path.with_extension("json.tmp");
    let body = serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, &path)?;
    Ok(effective)
}
