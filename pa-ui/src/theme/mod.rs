//! The window's colours, and the file that can change them.
//!
//! One table names every colour the window draws and decides what each one is;
//! components ask for a role and never for a value. The table is compiled in, so
//! the window has a complete theme before it touches the disk, and a theme file
//! is a sparse patch laid over it: the base values it sets, and any role it
//! names outright.

mod contrast;
mod file;
mod table;
#[cfg(test)]
mod tests;

pub(crate) use contrast::*;
pub(crate) use file::*;
pub(crate) use table::*;

use crate::{AppWindow, Theme};
use slint::ComponentHandle;

/// Assign every role to the window's theme global.
///
/// The single entry point: nothing else in the window knows how a colour gets
/// into Slint, so a reload that lands later has one place to reach and no
/// component has to change.
pub(crate) fn apply(ui: &AppWindow, theme: &Resolved) {
    let global = ui.global::<Theme>();
    for role in Role::ALL {
        set(&global, role, theme.get(role).to_slint());
    }
}

/// The setter behind each role.
///
/// The match is exhaustive on purpose: a role with nowhere to go does not
/// compile, so the table cannot grow a colour the global has never heard of.
fn set(global: &Theme<'_>, role: Role, value: slint::Color) {
    match role {
        Role::TitleBarGlyph => global.set_title_bar_glyph(value),
        Role::TitleBarCaptionFillHover => global.set_title_bar_caption_fill_hover(value),
        Role::TitleBarCloseFillHover => global.set_title_bar_close_fill_hover(value),
        Role::TitleBarCloseInkHover => global.set_title_bar_close_ink_hover(value),
        Role::TitleBarFoldFillHover => global.set_title_bar_fold_fill_hover(value),
        Role::TitleBarFoldGlyph => global.set_title_bar_fold_glyph(value),
        Role::TitleBarFoldGlyphHover => global.set_title_bar_fold_glyph_hover(value),
    }
}
