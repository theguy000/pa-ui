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
        Role::SidebarSurface => global.set_sidebar_surface(value),
        Role::SidebarRowFill => global.set_sidebar_row_fill(value),
        Role::SidebarRowFillHover => global.set_sidebar_row_fill_hover(value),
        Role::SidebarRowFillSelected => global.set_sidebar_row_fill_selected(value),
        Role::SidebarNameInk => global.set_sidebar_name_ink(value),
        Role::SidebarNameInkSelected => global.set_sidebar_name_ink_selected(value),
        Role::SidebarNameInkEditing => global.set_sidebar_name_ink_editing(value),
        Role::SidebarAgeInk => global.set_sidebar_age_ink(value),
        Role::TagWorking => global.set_tag_working(value),
        Role::TagParked => global.set_tag_parked(value),
        Role::TagIdle => global.set_tag_idle(value),
        Role::TranscriptSurface => global.set_transcript_surface(value),
        Role::TranscriptProseInk => global.set_transcript_prose_ink(value),
        Role::TranscriptMessageFill => global.set_transcript_message_fill(value),
        Role::TranscriptMessageInk => global.set_transcript_message_ink(value),
        Role::TranscriptLinkInk => global.set_transcript_link_ink(value),
        Role::TranscriptNoticeInk => global.set_transcript_notice_ink(value),
        Role::TranscriptCellFill => global.set_transcript_cell_fill(value),
        Role::TranscriptCellFillHover => global.set_transcript_cell_fill_hover(value),
        Role::TranscriptCellTitleInk => global.set_transcript_cell_title_ink(value),
        Role::TranscriptCellMetaInk => global.set_transcript_cell_meta_ink(value),
        Role::TranscriptCellSeparatorInk => global.set_transcript_cell_separator_ink(value),
        Role::TranscriptCellCodeInk => global.set_transcript_cell_code_ink(value),
        Role::TranscriptCellOutputInk => global.set_transcript_cell_output_ink(value),
        Role::TranscriptCellRule => global.set_transcript_cell_rule(value),
        Role::TranscriptCellEdgeExpanded => global.set_transcript_cell_edge_expanded(value),
        Role::TranscriptThinkingEdgeExpanded => global.set_transcript_thinking_edge_expanded(value),
        Role::TranscriptCellFillFailed => global.set_transcript_cell_fill_failed(value),
        Role::TranscriptCellFillFailedHover => global.set_transcript_cell_fill_failed_hover(value),
        Role::TranscriptCellInkFailed => global.set_transcript_cell_ink_failed(value),
        Role::TranscriptCellBadgeFill => global.set_transcript_cell_badge_fill(value),
        Role::TranscriptCellBadgeFillHover => {
            global.set_transcript_cell_badge_fill_hover(value)
        }
        Role::TranscriptReasonEdge => global.set_transcript_reason_edge(value),
        Role::TranscriptReasonInk => global.set_transcript_reason_ink(value),
        Role::TranscriptRowFillExpanded => global.set_transcript_row_fill_expanded(value),
        Role::TranscriptThinkingFill => global.set_transcript_thinking_fill(value),
        Role::TranscriptThinkingFillHover => global.set_transcript_thinking_fill_hover(value),
        Role::TranscriptThinkingLabelInk => global.set_transcript_thinking_label_ink(value),
        Role::TranscriptThinkingInk => global.set_transcript_thinking_ink(value),
        Role::TranscriptCodeFill => global.set_transcript_code_fill(value),
        Role::TranscriptCodeInk => global.set_transcript_code_ink(value),
        Role::TranscriptCodeLabelInk => global.set_transcript_code_label_ink(value),
        Role::TranscriptTableInk => global.set_transcript_table_ink(value),
        Role::TranscriptImageFill => global.set_transcript_image_fill(value),
        Role::TranscriptScrollThumb => global.set_transcript_scroll_thumb(value),
        Role::TranscriptScrollThumbHover => global.set_transcript_scroll_thumb_hover(value),
        Role::TranscriptScrollThumbPressed => global.set_transcript_scroll_thumb_pressed(value),
        Role::TranscriptJumpFill => global.set_transcript_jump_fill(value),
        Role::TranscriptJumpFillHover => global.set_transcript_jump_fill_hover(value),
        Role::TranscriptJumpEdge => global.set_transcript_jump_edge(value),
    }
}
