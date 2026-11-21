use gpui_kit::assets::Assets as KitAssets;
use gpui_kit::*;
use std::borrow::Cow;

pub const PRIME_AGENT_MARK_DARK_PATH: &str = "icons/prime-agent-mark-dark.svg";
pub const PRIME_AGENT_MARK_LIGHT_PATH: &str = "icons/prime-agent-mark-light.svg";

const PRIME_AGENT_MARK_DARK_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"13.5 16 74 74\" width=\"64\" height=\"64\">\
<path fill=\"#cfcfc8\" d=\"M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z\"/>\
<path fill=\"#6d6d66\" d=\"M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z\"/>\
<path fill=\"#7c7c75\" d=\"M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z\"/>\
<path fill=\"#e7e5df\" d=\"M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z\"/>\
<path fill=\"#8e8e86\" d=\"M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z\"/>\
<path fill=\"#ffffff\" d=\"M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z\"/>\
</svg>";

const PRIME_AGENT_MARK_LIGHT_SVG: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"13.5 16 74 74\" width=\"64\" height=\"64\">\
<path fill=\"#52524c\" d=\"M 51.59 37.56 L 43.2 43.19 L 43.2 61.5 L 73 41.5 Z\"/>\
<path fill=\"#8c8c84\" d=\"M 43.2 43.19 L 43.2 61.5 L 45.4 60.02 L 45.4 41.72 Z\"/>\
<path fill=\"#787870\" d=\"M 51.59 37.56 L 48.32 39.76 L 69.73 43.70 L 73 41.5 Z\"/>\
<path fill=\"#2c2d28\" d=\"M 28 16.5 L 28 89.5 L 43.2 79.3 L 43.2 27.14 Z\"/>\
<path fill=\"#696962\" d=\"M 28 33.22 L 28 36.02 L 43.2 38.82 L 43.2 36.02 Z\"/>\
<path fill=\"#141512\" d=\"M 28 16.5 L 28 33.22 L 73 41.5 L 50 16.5 Z\"/>\
</svg>";

pub struct AppAssets;

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == PRIME_AGENT_MARK_DARK_PATH {
            return Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_DARK_SVG.as_bytes())));
        }
        if path == PRIME_AGENT_MARK_LIGHT_PATH {
            return Ok(Some(Cow::Borrowed(PRIME_AGENT_MARK_LIGHT_SVG.as_bytes())));
        }
        KitAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        KitAssets.list(path)
    }
}
