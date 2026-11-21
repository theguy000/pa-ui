use gpui_kit::assets::Assets as KitAssets;
use gpui_kit::*;
use std::borrow::Cow;

pub const PRIME_AGENT_MARK_DARK_PATH: &str = "icons/prime-agent-mark-dark.svg";
pub const PRIME_AGENT_MARK_LIGHT_PATH: &str = "icons/prime-agent-mark-light.svg";
pub const ICON_PYTHON_PATH: &str = "icons/pa-python.svg";
pub const ICON_BASH_PATH: &str = "icons/pa-bash.svg";
pub const ICON_THINKING_PATH: &str = "icons/pa-thinking.svg";

// Exact pa-ui shipped dark-theme 6-facet brand mark (pa-ui/ui/icons.slint + theme/tests.rs)
const PRIME_AGENT_MARK_DARK_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"13.5 16 74 74\" width=\"64\" height=\"64\">\
<path fill=\"#aeaead\" d=\"M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z\"/>\
<path fill=\"#828381\" d=\"M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z\"/>\
<path fill=\"#595a58\" d=\"M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z\"/>\
<path fill=\"#626361\" d=\"M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z\"/>\
<path fill=\"#323330\" d=\"M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z\"/>\
<path fill=\"#ffffff\" d=\"M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z\"/>\
</svg>";

const PRIME_AGENT_MARK_LIGHT_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"13.5 16 74 74\" width=\"64\" height=\"64\">\
<path fill=\"#5a5a5c\" d=\"M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z\"/>\
<path fill=\"#7d7d80\" d=\"M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z\"/>\
<path fill=\"#a1a1a6\" d=\"M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z\"/>\
<path fill=\"#8f8f94\" d=\"M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z\"/>\
<path fill=\"#c4c4c9\" d=\"M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z\"/>\
<path fill=\"#111113\" d=\"M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z\"/>\
</svg>";

// Lucide code-xml (</>) — exact pa-ui IconToolPython
const ICON_PYTHON_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.85\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"m6 8-4 4 4 4\"/>\
<path d=\"m14.5 4-5 16\"/>\
<path d=\"m18 16 4-4-4-4\"/>\
</svg>";

// Lucide terminal (>_) — exact pa-ui IconToolBash
const ICON_BASH_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.85\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"m4 17 6-6-6-6\"/>\
<path d=\"M12 19h8\"/>\
</svg>";

// Exact pa-ui IconThinking flowing wave
const ICON_THINKING_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.8\" stroke-linecap=\"round\" stroke-linejoin=\"round\">\
<path d=\"M3.5 13.5c1.5-4.5 3.6-4.5 4.6 0s2.7 4.5 4.1 0 2.7-4.5 4.1 0c1 3 2.5 3.2 4.2.5\"/>\
</svg>";

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            PRIME_AGENT_MARK_DARK_PATH => Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_DARK_SVG.as_bytes()))),
            PRIME_AGENT_MARK_LIGHT_PATH => Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_LIGHT_SVG.as_bytes()))),
            ICON_PYTHON_PATH => Ok(Some(Cow::Borrowed(ICON_PYTHON_SVG.as_bytes()))),
            ICON_BASH_PATH => Ok(Some(Cow::Borrowed(ICON_BASH_SVG.as_bytes()))),
            ICON_THINKING_PATH => Ok(Some(Cow::Borrowed(ICON_THINKING_SVG.as_bytes()))),
            _ => KitAssets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        KitAssets.list(path)
    }
}
