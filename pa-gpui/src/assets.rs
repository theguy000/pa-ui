use gpui_kit::assets::Assets as KitAssets;
use gpui_kit::*;
use std::borrow::Cow;

pub const PRIME_AGENT_MARK_DARK_PATH: &str = "icons/prime-agent-mark-dark.svg";
pub const PRIME_AGENT_MARK_LIGHT_PATH: &str = "icons/prime-agent-mark-light.svg";
pub const ICON_PYTHON_PATH: &str = "icons/pa-python.svg";
pub const ICON_PYTHON_L_PATH: &str = "icons/pa-python-l.svg";
pub const ICON_PYTHON_M_PATH: &str = "icons/pa-python-m.svg";
pub const ICON_PYTHON_R_PATH: &str = "icons/pa-python-r.svg";
pub const ICON_BASH_PATH: &str = "icons/pa-bash.svg";
pub const ICON_BASH_CHEVRON_PATH: &str = "icons/pa-bash-chevron.svg";
pub const ICON_BASH_CURSOR_PATH: &str = "icons/pa-bash-cursor.svg";
pub const ICON_THINKING_PATH: &str = "icons/pa-thinking.svg";
pub const ICON_SUBAGENTS_PATH: &str = "icons/pa-subagents.svg";
pub const ICON_HEARTBEAT_PATH: &str = "icons/pa-heartbeat.svg";
pub const ICON_SHELLS_PATH: &str = "icons/pa-shells.svg";
pub const ICON_SEND_PATH: &str = "icons/pa-send.svg";
pub const ICON_SWATCH_BOOK_PATH: &str = "icons/pa-swatch-book.svg";
pub const ICON_PROVIDERS_PATH: &str = "icons/pa-providers.svg";
pub const ICON_CLOSE_PATH: &str = "icons/pa-close.svg";
pub const ICON_GIT_BRANCH_PATH: &str = "icons/pa-git-branch.svg";

// Exact pa-ui shipped dark-theme 6-facet brand mark (pa-ui/ui/icons.slint + theme/tests.rs)
const PRIME_AGENT_MARK_DARK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="13.5 16 74 74" width="64" height="64"><path fill="#aeaead" d="M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z"/><path fill="#828381" d="M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z"/><path fill="#595a58" d="M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z"/><path fill="#626361" d="M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z"/><path fill="#323330" d="M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z"/><path fill="#ffffff" d="M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z"/></svg>"##;

const PRIME_AGENT_MARK_LIGHT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="13.5 16 74 74" width="64" height="64"><path fill="#5a5a5c" d="M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z"/><path fill="#7d7d80" d="M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z"/><path fill="#a1a1a6" d="M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z"/><path fill="#8f8f94" d="M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z"/><path fill="#c4c4c9" d="M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z"/><path fill="#111113" d="M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z"/></svg>"##;

// Lucide code-xml (</>) — exact pa-ui IconToolPython (full + per-stroke layers for ripple)
const ICON_PYTHON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m6 8-4 4 4 4"/><path d="m14.5 4-5 16"/><path d="m18 16 4-4-4-4"/></svg>"##;
const ICON_PYTHON_L_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m6 8-4 4 4 4"/></svg>"##;
const ICON_PYTHON_M_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m14.5 4-5 16"/></svg>"##;
const ICON_PYTHON_R_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m18 16 4-4-4-4"/></svg>"##;

// Lucide terminal (>_) — exact pa-ui IconToolBash (full + chevron/cursor layers for blink)
const ICON_BASH_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m4 17 6-6-6-6"/><path d="M12 19h8"/></svg>"##;
const ICON_BASH_CHEVRON_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="m4 17 6-6-6-6"/></svg>"##;
const ICON_BASH_CURSOR_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.85" stroke-linecap="round" stroke-linejoin="round"><path d="M12 19h8"/></svg>"##;

// Exact pa-ui IconThinking flowing wave
const ICON_THINKING_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 13.5c1.5-4.5 3.6-4.5 4.6 0s2.7 4.5 4.1 0 2.7-4.5 4.1 0c1 3 2.5 3.2 4.2.5"/></svg>"##;

// Exact pa-ui IconSubagents (branching tree: 1 parent over 2 children)
const ICON_SUBAGENTS_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M 12 3 A 2.5 2.5 0 0 1 12 8 A 2.5 2.5 0 0 1 12 3 Z M 6.5 16 A 2.5 2.5 0 0 1 6.5 21 A 2.5 2.5 0 0 1 6.5 16 Z M 17.5 16 A 2.5 2.5 0 0 1 17.5 21 A 2.5 2.5 0 0 1 17.5 16 Z M 12 8 L 12 12 M 6.5 16 L 6.5 14.5 A 2.5 2.5 0 0 1 9 12 L 15 12 A 2.5 2.5 0 0 1 17.5 14.5 L 17.5 16"/></svg>"##;

// Exact pa-ui IconHeartbeat (monoline ECG pulse wave)
const ICON_HEARTBEAT_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M 3 12 L 7.2 12 L 9.5 6 L 14.5 18 L 16.8 12 L 21 12"/></svg>"##;

// Exact pa-ui IconShells (rounded terminal frame with >_)
const ICON_SHELLS_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"><path d="M 7 3 L 17 3 A 4 4 0 0 1 21 7 L 21 17 A 4 4 0 0 1 17 21 L 7 21 A 4 4 0 0 1 3 17 L 3 7 A 4 4 0 0 1 7 3 Z M 8 9.3 L 11.2 12 L 8 14.7 M 12.8 14.7 L 16 14.7"/></svg>"##;

// Exact pa-ui IconSend (centered 12x12 up-arrow)
const ICON_SEND_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.65" stroke-linecap="round" stroke-linejoin="round"><path d="M 6 10.2 L 6 2 M 2.4 5.6 L 6 2 L 9.6 5.6"/></svg>"##;

// Lucide swatch-book (General settings tab icon)
const ICON_SWATCH_BOOK_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M11 17a4 4 0 0 1-8 0V5a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2Z"/><path d="M16.7 13H19a2 2 0 0 1 2 2v4a2 2 0 0 1-2 2H7"/><path d="M7 17h.01"/><path d="m11 8 2.3-2.3a2.4 2.4 0 0 1 3.404.004L18.6 7.6a2.4 2.4 0 0 1 .026 3.434L9.9 19.8"/></svg>"##;

// Tri-Nodes (Providers settings tab icon)
const ICON_PROVIDERS_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="4.5"/><path d="M12 5v2.5"/><path d="M18.1 15.5 15.9 14.2"/><path d="M5.9 15.5 8.1 14.2"/><circle cx="12" cy="3.2" r="1.6"/><circle cx="19.6" cy="16.4" r="1.6"/><circle cx="4.4" cy="16.4" r="1.6"/></svg>"##;

// Symmetric 12x12 modal close X (exact 8px margin inside 28x28 button)
const ICON_CLOSE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"><path d="M2.5 2.5L9.5 9.5M9.5 2.5L2.5 9.5"/></svg>"##;

// Lucide git-branch
const ICON_GIT_BRANCH_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"##;

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            PRIME_AGENT_MARK_DARK_PATH => Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_DARK_SVG.as_bytes()))),
            PRIME_AGENT_MARK_LIGHT_PATH => Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_LIGHT_SVG.as_bytes()))),
            ICON_PYTHON_PATH => Ok(Some(Cow::Borrowed(ICON_PYTHON_SVG.as_bytes()))),
            ICON_PYTHON_L_PATH => Ok(Some(Cow::Borrowed(ICON_PYTHON_L_SVG.as_bytes()))),
            ICON_PYTHON_M_PATH => Ok(Some(Cow::Borrowed(ICON_PYTHON_M_SVG.as_bytes()))),
            ICON_PYTHON_R_PATH => Ok(Some(Cow::Borrowed(ICON_PYTHON_R_SVG.as_bytes()))),
            ICON_BASH_PATH => Ok(Some(Cow::Borrowed(ICON_BASH_SVG.as_bytes()))),
            ICON_BASH_CHEVRON_PATH => Ok(Some(Cow::Borrowed(ICON_BASH_CHEVRON_SVG.as_bytes()))),
            ICON_BASH_CURSOR_PATH => Ok(Some(Cow::Borrowed(ICON_BASH_CURSOR_SVG.as_bytes()))),
            ICON_THINKING_PATH => Ok(Some(Cow::Borrowed(ICON_THINKING_SVG.as_bytes()))),
            ICON_SUBAGENTS_PATH => Ok(Some(Cow::Borrowed(ICON_SUBAGENTS_SVG.as_bytes()))),
            ICON_HEARTBEAT_PATH => Ok(Some(Cow::Borrowed(ICON_HEARTBEAT_SVG.as_bytes()))),
            ICON_SHELLS_PATH => Ok(Some(Cow::Borrowed(ICON_SHELLS_SVG.as_bytes()))),
            ICON_SEND_PATH => Ok(Some(Cow::Borrowed(ICON_SEND_SVG.as_bytes()))),
            ICON_SWATCH_BOOK_PATH => Ok(Some(Cow::Borrowed(ICON_SWATCH_BOOK_SVG.as_bytes()))),
            ICON_PROVIDERS_PATH => Ok(Some(Cow::Borrowed(ICON_PROVIDERS_SVG.as_bytes()))),
            ICON_CLOSE_PATH => Ok(Some(Cow::Borrowed(ICON_CLOSE_SVG.as_bytes()))),
            ICON_GIT_BRANCH_PATH => Ok(Some(Cow::Borrowed(ICON_GIT_BRANCH_SVG.as_bytes()))),
            _ => KitAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        KitAssets.list(path)
    }
}
