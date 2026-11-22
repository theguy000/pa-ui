//! Dev-only error log shown from the titlebar Info button instead of the composer.

use std::path::Path;

const MAX_DEV_ERRORS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevErrorEntry {
    pub(crate) id: usize,
    pub(crate) source: String,
    pub(crate) message: String,
}

impl DevErrorEntry {
    pub(crate) fn format_for_copy(&self) -> String {
        format!("[{}] {}", self.source, self.message)
    }
}

pub(crate) fn is_dev_environment_for(
    debug_assertions: bool,
    pa_dev: Option<&str>,
    has_cargo: bool,
    exe_path: Option<&Path>,
) -> bool {
    if let Some(val) = pa_dev.map(str::trim).filter(|s| !s.is_empty()) {
        return !matches!(val.to_ascii_lowercase().as_str(), "0" | "false" | "off");
    }
    if debug_assertions || has_cargo {
        return true;
    }
    let Some(exe) = exe_path else {
        return false;
    };
    let mut saw_profile_dir = false;
    for comp in exe.parent().into_iter().flat_map(|p| p.components()).rev() {
        let name = comp.as_os_str().to_string_lossy();
        if !saw_profile_dir {
            saw_profile_dir = matches!(name.as_ref(), "debug" | "fast" | "release");
        } else if name == "target" {
            return true;
        } else {
            saw_profile_dir = matches!(name.as_ref(), "debug" | "fast" | "release");
        }
    }
    false
}

pub(crate) fn is_dev_environment() -> bool {
    let pa_dev = std::env::var("PA_DEV").ok();
    let has_cargo = std::env::var_os("CARGO").is_some();
    let exe_path = std::env::current_exe().ok();
    is_dev_environment_for(
        cfg!(debug_assertions),
        pa_dev.as_deref(),
        has_cargo,
        exe_path.as_deref(),
    )
}

pub(crate) struct DevErrorLog {
    enabled: bool,
    next_id: usize,
    entries: Vec<DevErrorEntry>,
}

impl DevErrorLog {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            next_id: 1,
            entries: Vec::new(),
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub(crate) fn entries(&self) -> &[DevErrorEntry] {
        &self.entries
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn push(&mut self, source: impl Into<String>, message: impl Into<String>) {
        if !self.enabled {
            return;
        }
        let message = message.into().trim().to_string();
        if message.is_empty() {
            return;
        }
        let source = source.into();
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        self.entries.insert(
            0,
            DevErrorEntry {
                id,
                source,
                message,
            },
        );
        if self.entries.len() > MAX_DEV_ERRORS {
            self.entries.truncate(MAX_DEV_ERRORS);
        }
    }

    pub(crate) fn dismiss(&mut self, id: usize) {
        self.entries.retain(|entry| entry.id != id);
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(crate) fn format_all(&self) -> String {
        self.entries
            .iter()
            .map(DevErrorEntry::format_for_copy)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[::core::prelude::v1::test]
    fn dev_environment_detection_respects_env_cargo_and_target_paths() {
        let installed = PathBuf::from(r"C:\Program Files\Prime Agent\pa-gpui.exe");
        let fast_target = PathBuf::from(r"C:\Users\dev\repo\target\fast\pa-gpui.exe");

        assert!(!is_dev_environment_for(
            false,
            None,
            false,
            Some(&installed)
        ));
        assert!(is_dev_environment_for(
            false,
            None,
            false,
            Some(&fast_target)
        ));
        assert!(is_dev_environment_for(true, None, false, Some(&installed)));
        assert!(is_dev_environment_for(false, None, true, Some(&installed)));
        assert!(!is_dev_environment_for(
            true,
            Some("0"),
            true,
            Some(&fast_target)
        ));
        assert!(is_dev_environment_for(
            false,
            Some("1"),
            false,
            Some(&installed)
        ));
    }

    #[::core::prelude::v1::test]
    fn dev_error_log_records_dismisses_clears_and_formats_only_when_enabled() {
        let mut disabled = DevErrorLog::new(false);
        disabled.push("daemon", "boom");
        assert!(disabled.entries().is_empty());

        let mut log = DevErrorLog::new(true);
        log.push("daemon", "first error");
        log.push("slash", "  /foo is not available  ");
        assert_eq!(log.len(), 2);
        assert_eq!(log.entries()[0].source, "slash");
        assert_eq!(log.entries()[0].message, "/foo is not available");
        assert_eq!(
            log.format_all(),
            "[slash] /foo is not available\n[daemon] first error"
        );

        let first_id = log.entries()[0].id;
        log.dismiss(first_id);
        assert_eq!(log.len(), 1);
        assert_eq!(log.entries()[0].source, "daemon");

        log.clear();
        assert!(log.entries().is_empty());
    }
}
