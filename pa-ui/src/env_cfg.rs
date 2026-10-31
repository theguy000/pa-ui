//! Flags read once.
//!
//! `std::env::var` takes a lock on the process environment, and the window was
//! asking for `PA_DEBUG` on every poll tick, every event frame, every rebuilt
//! message and every row conversion. The answers cannot change while the app
//! runs, so they are taken once here.

use std::sync::OnceLock;

pub struct Config {
    pub debug: bool,
    pub dump: Option<String>,
    pub demo_markdown: bool,
    pub queue_preset: bool,
    pub menu: bool,
    pub canvas: bool,
}

static CONFIG: OnceLock<Config> = OnceLock::new();

#[must_use]
pub fn cfg() -> &'static Config {
    CONFIG.get_or_init(|| Config {
        debug: std::env::var("PA_DEBUG").is_ok(),
        dump: std::env::var("PA_DUMP").ok(),
        demo_markdown: std::env::var("PA_DEMO").as_deref() == Ok("markdown"),
        queue_preset: std::env::var("PA_QUEUE").is_ok(),
        menu: std::env::var("PA_MENU").is_ok(),
        canvas: std::env::var("PA_CANVAS").is_ok(),
    })
}
