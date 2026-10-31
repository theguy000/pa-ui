//! Vendored from `pa-tui` (the terminal UI's own daemon client).
//!
//! Copied rather than depended on: `pa-tui` drags ratatui and its
//! `unicode-width` requirement into a Slint workspace, which cannot resolve
//! against the compiler's own `annotate-snippets` pin. The two modules here are
//! the terminal UI's transport verbatim (only `crate::` paths adjusted), so the
//! app speaks to the daemon exactly as the TUI does - including the async event
//! pump that keeps up with the daemon's bounded, dropping event queue.
//!
//! # Why the lints below are allowed at the module
//!
//! Upstream, both modules sit in `pa-tui`'s public surface, so everything in
//! them is reachable and nothing is dead. Here they sit in a private module and
//! this app uses only part of that surface: request/response, the event pump,
//! and the direct transport. The rest - the update-resume frames, the reconnect
//! ladder, the peer-claim handshake - is upstream's, not ours. Deleting it
//! would make the copy diverge and the next re-sync a manual merge, so the copy
//! stays verbatim and its unused surface is declared unused here, at the one
//! place that can say so honestly.
#![expect(
    dead_code,
    unused_imports,
    reason = "vendored verbatim from pa-tui: its unused surface is upstream's"
)]

pub mod daemon_client;
pub mod direct_transport;
