//! Persistent desktop settings and UI bindings for the Settings popup.

use crate::*;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const VERSION: i64 = 1;

#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Settings {
    pub theme: String,
    pub thread_notifications: String,
    pub in_app_notifications: bool,
    pub session_order: String,
    pub confirm_quit: bool,
    pub confirm_delete_session: bool,
    pub queue_key: String,
    pub reopen_last_session: bool,
    pub last_session: String,
    pub interface_size: String,
    pub code_font: String,
    pub row_spacing: String,
    pub show_timestamps: bool,
    pub reduce_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: "default".to_string(),
            thread_notifications: "Off".to_string(),
            in_app_notifications: true,
            session_order: "Last used".to_string(),
            confirm_quit: false,
            confirm_delete_session: true,
            queue_key: "Ctrl+Enter".to_string(),
            reopen_last_session: true,
            last_session: String::new(),
            interface_size: "Default".to_string(),
            code_font: "Cascadia Code".to_string(),
            row_spacing: "Comfortable".to_string(),
            show_timestamps: false,
            reduce_motion: false,
        }
    }
}

pub(crate) const THREAD_NOTIFICATION_OPTIONS: &[&str] = &["Off", "Only when unfocused", "Always"];
pub(crate) const SESSION_ORDER_OPTIONS: &[&str] = &["Last used", "Name", "Manual"];
pub(crate) const QUEUE_KEY_OPTIONS: &[&str] = &["Ctrl+Enter", "Alt+Enter"];
pub(crate) const INTERFACE_SIZE_OPTIONS: &[&str] = &["Compact", "Default", "Large"];
pub(crate) const CODE_FONT_OPTIONS: &[&str] = &["Cascadia Code", "Consolas", "JetBrains Mono"];
pub(crate) const ROW_SPACING_OPTIONS: &[&str] = &["Comfortable", "Compact"];

fn valid_option<'a>(options: &[&'a str], candidate: &str, fallback: &'a str) -> String {
    options
        .iter()
        .copied()
        .find(|item| *item == candidate)
        .unwrap_or(fallback)
        .to_string()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct SectionFilter {
    pub notifications: bool,
    pub sidebar: bool,
    pub confirmations: bool,
    pub behavior: bool,
    pub theme: bool,
    pub typography: bool,
    pub density: bool,
}

impl SectionFilter {
    pub(crate) fn for_query(query: &str) -> Self {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return Self {
                notifications: true,
                sidebar: true,
                confirmations: true,
                behavior: true,
                theme: true,
                typography: true,
                density: true,
            };
        }
        let matches = |words: &str| words.contains(&q);
        Self {
            notifications: matches("notifications thread alerts in-app toast focus"),
            sidebar: matches("sidebar session order last used manual"),
            confirmations: matches("confirmations confirm quit quitting delete deleting session"),
            behavior: matches("behavior queue with ctrl+enter alt+enter reopen last session"),
            theme: matches("theme appearance default mono obsidian ember paper colour color"),
            typography: matches("typography interface size compact default large code font cascadia consolas jetbrains"),
            density: matches("density row spacing comfortable compact show timestamps reduce motion"),
        }
    }

    pub(crate) fn apply_to_ui(self, ui: &AppWindow) {
        ui.set_settings_show_notifications(self.notifications);
        ui.set_settings_show_sidebar(self.sidebar);
        ui.set_settings_show_confirmations(self.confirmations);
        ui.set_settings_show_behavior(self.behavior);
        ui.set_settings_show_theme(self.theme);
        ui.set_settings_show_typography(self.typography);
        ui.set_settings_show_density(self.density);
        if !query_matches_page(ui.get_settings_page().as_str(), self) {
            if self.theme || self.typography || self.density {
                ui.set_settings_page("appearance".into());
            } else if self.notifications || self.sidebar || self.confirmations || self.behavior {
                ui.set_settings_page("general".into());
            }
        }
    }
}

fn query_matches_page(page: &str, filter: SectionFilter) -> bool {
    if page == "appearance" {
        filter.theme || filter.typography || filter.density
    } else {
        filter.notifications || filter.sidebar || filter.confirmations || filter.behavior
    }
}

impl Settings {
    pub(crate) fn to_json(&self) -> String {
        let value = json!({
            "version": VERSION,
            "theme": self.theme,
            "thread-notifications": self.thread_notifications,
            "in-app-notifications": self.in_app_notifications,
            "session-order": self.session_order,
            "confirm-quit": self.confirm_quit,
            "confirm-delete-session": self.confirm_delete_session,
            "queue-key": self.queue_key,
            "reopen-last-session": self.reopen_last_session,
            "last-session": self.last_session,
            "interface-size": self.interface_size,
            "code-font": self.code_font,
            "row-spacing": self.row_spacing,
            "show-timestamps": self.show_timestamps,
            "reduce-motion": self.reduce_motion,
        });
        serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
    }

    pub(crate) fn from_json(text: &str) -> Self {
        let mut out = Self::default();
        let text = text.trim_start_matches('\u{feff}');
        let Ok(Value::Object(map)) = serde_json::from_str::<Value>(text) else {
            return out;
        };
        apply_fields(&mut out, &map);
        out
    }

    pub(crate) fn load_from(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .map(|text| Self::from_json(&text))
            .unwrap_or_default()
    }

    pub(crate) fn save_to(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
            }
        }
        std::fs::write(path, self.to_json()).map_err(|err| err.to_string())
    }

    pub(crate) fn toggle(&mut self, key: &str) -> bool {
        match key {
            "in-app-notifications" => self.in_app_notifications = !self.in_app_notifications,
            "confirm-quit" => self.confirm_quit = !self.confirm_quit,
            "confirm-delete-session" => self.confirm_delete_session = !self.confirm_delete_session,
            "reopen-last-session" => self.reopen_last_session = !self.reopen_last_session,
            "show-timestamps" => self.show_timestamps = !self.show_timestamps,
            "reduce-motion" => self.reduce_motion = !self.reduce_motion,
            _ => return false,
        }
        true
    }

    pub(crate) fn choose(&mut self, key: &str, value: &str) -> bool {
        match key {
            "thread-notifications" => {
                self.thread_notifications = valid_option(THREAD_NOTIFICATION_OPTIONS, value, "Off");
            }
            "session-order" => {
                self.session_order = valid_option(SESSION_ORDER_OPTIONS, value, "Last used");
            }
            "queue-key" => {
                self.queue_key = valid_option(QUEUE_KEY_OPTIONS, value, "Ctrl+Enter");
            }
            "interface-size" => {
                self.interface_size = valid_option(INTERFACE_SIZE_OPTIONS, value, "Default");
            }
            "code-font" => {
                self.code_font = valid_option(CODE_FONT_OPTIONS, value, "Cascadia Code");
            }
            "row-spacing" => {
                self.row_spacing = valid_option(ROW_SPACING_OPTIONS, value, "Comfortable");
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn apply_to_ui(&self, ui: &AppWindow) {
        ui.set_setting_thread_notifications(self.thread_notifications.clone().into());
        ui.set_setting_in_app_notifications(self.in_app_notifications);
        ui.set_setting_session_order(self.session_order.clone().into());
        ui.set_setting_confirm_quit(self.confirm_quit);
        ui.set_setting_confirm_delete_session(self.confirm_delete_session);
        ui.set_setting_queue_key(self.queue_key.clone().into());
        ui.set_setting_reopen_last_session(self.reopen_last_session);
        ui.set_setting_interface_size(self.interface_size.clone().into());
        ui.set_setting_code_font(self.code_font.clone().into());
        ui.set_setting_row_spacing(self.row_spacing.clone().into());
        ui.set_setting_show_timestamps(self.show_timestamps);
        ui.set_setting_reduce_motion(self.reduce_motion);
    }
}

fn apply_fields(out: &mut Settings, map: &Map<String, Value>) {
    if let Some(theme) = map.get("theme").and_then(Value::as_str) {
        let trimmed = theme.trim();
        if !trimmed.is_empty() {
            out.theme = trimmed.to_string();
        }
    }
    if let Some(v) = map.get("thread-notifications").and_then(Value::as_str) {
        out.thread_notifications = valid_option(THREAD_NOTIFICATION_OPTIONS, v, "Off");
    }
    if let Some(v) = map.get("in-app-notifications").and_then(Value::as_bool) {
        out.in_app_notifications = v;
    }
    if let Some(v) = map.get("session-order").and_then(Value::as_str) {
        out.session_order = valid_option(SESSION_ORDER_OPTIONS, v, "Last used");
    }
    if let Some(v) = map.get("confirm-quit").and_then(Value::as_bool) {
        out.confirm_quit = v;
    }
    if let Some(v) = map.get("confirm-delete-session").and_then(Value::as_bool) {
        out.confirm_delete_session = v;
    }
    if let Some(v) = map.get("queue-key").and_then(Value::as_str) {
        out.queue_key = valid_option(QUEUE_KEY_OPTIONS, v, "Ctrl+Enter");
    }
    if let Some(v) = map.get("reopen-last-session").and_then(Value::as_bool) {
        out.reopen_last_session = v;
    }
    if let Some(v) = map.get("last-session").and_then(Value::as_str) {
        out.last_session = v.to_string();
    }
    if let Some(v) = map.get("interface-size").and_then(Value::as_str) {
        out.interface_size = valid_option(INTERFACE_SIZE_OPTIONS, v, "Default");
    }
    if let Some(v) = map.get("code-font").and_then(Value::as_str) {
        out.code_font = valid_option(CODE_FONT_OPTIONS, v, "Cascadia Code");
    }
    if let Some(v) = map.get("row-spacing").and_then(Value::as_str) {
        out.row_spacing = valid_option(ROW_SPACING_OPTIONS, v, "Comfortable");
    }
    if let Some(v) = map.get("show-timestamps").and_then(Value::as_bool) {
        out.show_timestamps = v;
    }
    if let Some(v) = map.get("reduce-motion").and_then(Value::as_bool) {
        out.reduce_motion = v;
    }
}

pub(crate) fn default_settings_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("PA_SETTINGS_FILE") {
        let trimmed = explicit.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let trimmed = appdata.trim();
        if !trimmed.is_empty() {
            return Some(
                PathBuf::from(trimmed)
                    .join("PrimeAgent")
                    .join("settings.json"),
            );
        }
    }
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return Some(
                PathBuf::from(trimmed)
                    .join(".prime-agent")
                    .join("settings.json"),
            );
        }
    }
    None
}

pub(crate) struct SettingsStore {
    pub state: RefCell<Settings>,
    path: Option<PathBuf>,
}

impl SettingsStore {
    pub(crate) fn new(path: Option<PathBuf>) -> Rc<Self> {
        let mut initial = path
            .as_deref()
            .map(Settings::load_from)
            .unwrap_or_default();
        if let Some(env_theme) = crate::env_cfg::cfg().theme.as_deref() {
            if !env_theme.is_empty() {
                initial.theme = env_theme.to_string();
            }
        }
        Rc::new(Self {
            state: RefCell::new(initial),
            path,
        })
    }

    pub(crate) fn persist(&self) -> Result<(), String> {
        match self.path.as_deref() {
            Some(path) => self.state.borrow().save_to(path),
            None => Ok(()),
        }
    }
}

pub(crate) fn open_settings(ui: &AppWindow) {
    ui.set_model_menu(false);
    ui.set_queue_open(false);
    ui.set_menu_id("".into());
    ui.set_search_field_typed(false);
    ui.set_search_open(false);
    ui.set_share_open(false);
    ui.set_settings_query("".into());
    ui.set_settings_open_choice("".into());
    SectionFilter::for_query("").apply_to_ui(ui);
    ui.set_settings_menu(true);
    ui.set_settings_focus_tick(ui.get_settings_focus_tick() + 1);
}

pub(crate) fn close_settings(ui: &AppWindow) {
    ui.set_settings_open_choice("".into());
    ui.set_settings_menu(false);
    ui.invoke_focus_composer();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_preserves_every_field() {
        let custom = Settings {
            theme: "mono".to_string(),
            thread_notifications: "Always".to_string(),
            in_app_notifications: false,
            session_order: "Name".to_string(),
            confirm_quit: true,
            confirm_delete_session: false,
            queue_key: "Ctrl+Enter".to_string(),
            reopen_last_session: false,
            last_session: "01a1c539-d4a3-72e5-9022-a1138888f7a1".to_string(),
            interface_size: "Large".to_string(),
            code_font: "JetBrains Mono".to_string(),
            row_spacing: "Compact".to_string(),
            show_timestamps: true,
            reduce_motion: true,
        };
        let encoded = custom.to_json();
        let decoded = Settings::from_json(&encoded);
        assert_eq!(decoded, custom);
    }

    #[test]
    fn settings_invalid_json_or_unknown_choice_falls_back_to_defaults() {
        assert_eq!(Settings::from_json("not json"), Settings::default());
        let with_bogus = r#"{
            "thread-notifications": "Bogus",
            "session-order": "Random",
            "queue-key": "Space",
            "interface-size": "Huge",
            "code-font": "Comic Sans",
            "row-spacing": "Tight"
        }"#;
        assert_eq!(Settings::from_json(with_bogus), Settings::default());
    }

    #[test]
    fn settings_disk_persistence_saves_and_reloads() {
        let path = std::env::temp_dir().join(format!(
            "pa-ui-settings-test-{}.json",
            std::process::id()
        ));
        let mut settings = Settings::default();
        assert!(settings.toggle("confirm-quit"));
        assert!(settings.choose("queue-key", "Alt+Enter"));
        assert!(!settings.toggle("unknown-key"));
        assert!(!settings.choose("unknown-key", "val"));
        settings.save_to(&path).expect("saved settings");
        let loaded = Settings::load_from(&path);
        let _ = std::fs::remove_file(&path);
        assert!(loaded.confirm_quit);
        assert_eq!(loaded.queue_key, "Alt+Enter");
    }

    #[test]
    fn settings_persist_reports_a_write_failure() {
        let root = std::env::temp_dir().join(format!(
            "pa-ui-settings-write-failure-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test directory");
        let blocker = root.join("not-a-directory");
        std::fs::write(&blocker, b"x").expect("blocking file");
        let store = SettingsStore::new(Some(blocker.join("settings.json")));
        assert!(store.persist().is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn settings_persist_without_a_path_is_a_noop() {
        let store = SettingsStore::new(None);
        assert!(store.persist().is_ok());
    }

    #[test]
    fn removed_options_are_not_read_from_an_old_settings_file() {
        let loaded = Settings::from_json(
            r#"{"dim-inactive-sessions": true, "restore-drafts": false, "show-session-previews": false}"#,
        );
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn settings_choose_uses_the_explicit_option() {
        let mut settings = Settings::default();
        assert!(settings.choose("thread-notifications", "Always"));
        assert_eq!(settings.thread_notifications, "Always");
        assert!(settings.choose("session-order", "Name"));
        assert_eq!(settings.session_order, "Name");
        assert!(settings.choose("code-font", "Consolas"));
        assert_eq!(settings.code_font, "Consolas");
    }

    #[test]
    fn section_filter_narrows_sections_by_query() {
        let all = SectionFilter::for_query("");
        assert!(all.notifications && all.sidebar && all.confirmations && all.behavior && all.theme && all.typography && all.density);

        let font = SectionFilter::for_query("font");
        assert!(font.typography);
        assert!(!font.notifications && !font.confirmations);

        let queue = SectionFilter::for_query("queue");
        assert!(queue.behavior);

        let notify = SectionFilter::for_query("toast");
        assert!(notify.notifications);
        assert!(!notify.theme && !notify.density);

        let order = SectionFilter::for_query("order");
        assert!(order.sidebar);

        let preview = SectionFilter::for_query("preview");
        assert!(!preview.sidebar && !preview.notifications && !preview.theme);
    }
}
