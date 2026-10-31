//! Code-preview extraction for tool-call cards (`ipython` collapsed line).
//! The pa-tui dependency boundary (pa-types only) keeps the session engine
//! out of the UI, so the display-side preview algorithm lives here; the
//! TypeScript golden corpus test lives here too.

pub mod bash;
pub mod cell;

pub use cell::parse_ipython_bash_cell;
pub mod python;

pub use bash::{preview_bash_command, CodePreviewLanguage};
pub use python::preview_ipython_code;
