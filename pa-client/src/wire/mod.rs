//! Vendored from `pa-tui` (the terminal UI's own daemon client).
//!
//! Copied rather than depended on: `pa-tui` drags ratatui and its
//! `unicode-width` requirement into a Slint workspace, which cannot resolve
//! against the compiler's own `annotate-snippets` pin. The two modules here are
//! the terminal UI's transport verbatim (only `crate::` paths adjusted), so the
//! app speaks to the daemon exactly as the TUI does - including the async event
//! pump that keeps up with the daemon's bounded, dropping event queue.

pub mod daemon_client;
pub mod direct_transport;
