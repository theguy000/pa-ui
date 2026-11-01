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
use std::sync::{PoisonError, RwLock};

/// The theme the two parts that paint their own colours read: the transcript's
/// markup builder and the composer's canvas. Neither can be handed a
/// `slint::Color`, so neither goes through the global below.
///
/// Written by `apply` and read here, so the one entry point reaches everything:
/// a reload that lands later has nothing else to update, and no component has to
/// change. Unset — a test that only resolves files, a theme author's check —
/// reads the theme the window ships with.
static CURRENT: RwLock<Option<Resolved>> = RwLock::new(None);

/// The theme the window is drawn in, or the shipped one before anything applied.
///
/// A copy rather than a borrow: the map is a fixed number of four-byte colours,
/// and it is read once per poll or per draft, not once per glyph.
pub(crate) fn current() -> Resolved {
    // A poisoned lock is a panic in another thread; the value it guards cannot
    // be left half-written here, so it is still the theme the window drew with.
    let held = CURRENT.read().unwrap_or_else(PoisonError::into_inner);
    held.clone().unwrap_or_else(Resolved::shipped)
}

/// Assign every role to the window's theme global, and hand the same theme to
/// the code that paints its own colours.
///
/// The single entry point: nothing else in the window knows how a colour gets
/// into Slint or into a `<font>` tag, so a reload that lands later has one place
/// to reach and no component has to change.
pub(crate) fn apply(ui: &AppWindow, theme: &Resolved) {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Some(theme.clone());
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
        Role::SidebarFoldGlyph => global.set_sidebar_fold_glyph(value),
        Role::SidebarFoldGlyphHover => global.set_sidebar_fold_glyph_hover(value),
        Role::SidebarNewSessionMark => global.set_sidebar_new_session_mark(value),
        Role::SidebarNewSessionMarkHover => global.set_sidebar_new_session_mark_hover(value),
        Role::SidebarStripNewSessionMark => global.set_sidebar_strip_new_session_mark(value),
        Role::SidebarStripNewSessionMarkHover => {
            global.set_sidebar_strip_new_session_mark_hover(value)
        }
        Role::SidebarSearchMark => global.set_sidebar_search_mark(value),
        Role::SidebarRowRenameMark => global.set_sidebar_row_rename_mark(value),
        Role::SidebarRowRenameFill => global.set_sidebar_row_rename_fill(value),
        Role::SidebarRowRenameFillHover => global.set_sidebar_row_rename_fill_hover(value),
        Role::SidebarNewSessionInk => global.set_sidebar_new_session_ink(value),
        Role::SidebarNewSessionInkHover => global.set_sidebar_new_session_ink_hover(value),
        Role::SidebarOpenTipFill => global.set_sidebar_open_tip_fill(value),
        Role::SidebarOpenTipInk => global.set_sidebar_open_tip_ink(value),
        Role::SidebarOpenTipShadow => global.set_sidebar_open_tip_shadow(value),
        Role::BrandMarkBlade => global.set_brand_mark_blade(value),
        Role::BrandMarkWing => global.set_brand_mark_wing(value),
        Role::BrandMarkWingInnerShadow => global.set_brand_mark_wing_inner_shadow(value),
        Role::BrandMarkWingShadow => global.set_brand_mark_wing_shadow(value),
        Role::BrandMarkStem => global.set_brand_mark_stem(value),
        Role::BrandMarkStemShadow => global.set_brand_mark_stem_shadow(value),
        Role::TranscriptSurface => global.set_transcript_surface(value),
        Role::TranscriptProseInk => global.set_transcript_prose_ink(value),
        Role::TranscriptMessageFill => global.set_transcript_message_fill(value),
        Role::TranscriptMessageInk => global.set_transcript_message_ink(value),
        Role::TranscriptLinkInk => global.set_transcript_link_ink(value),
        Role::TranscriptNoticeInk => global.set_transcript_notice_ink(value),
        Role::TranscriptHeadingInk => global.set_transcript_heading_ink(value),
        Role::TranscriptInlineCodeInk => global.set_transcript_inline_code_ink(value),
        Role::TranscriptCaretMark => global.set_transcript_caret_mark(value),
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
        Role::TranscriptCellBadgeMark => global.set_transcript_cell_badge_mark(value),
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
        Role::TranscriptCodeKeywordInk => global.set_transcript_code_keyword_ink(value),
        Role::TranscriptCodeStringInk => global.set_transcript_code_string_ink(value),
        Role::TranscriptCodeCommentInk => global.set_transcript_code_comment_ink(value),
        Role::TranscriptTableInk => global.set_transcript_table_ink(value),
        Role::TranscriptImageFill => global.set_transcript_image_fill(value),
        Role::TranscriptScrollThumb => global.set_transcript_scroll_thumb(value),
        Role::TranscriptScrollThumbHover => global.set_transcript_scroll_thumb_hover(value),
        Role::TranscriptScrollThumbPressed => global.set_transcript_scroll_thumb_pressed(value),
        Role::TranscriptJumpFill => global.set_transcript_jump_fill(value),
        Role::TranscriptJumpFillHover => global.set_transcript_jump_fill_hover(value),
        Role::TranscriptJumpEdge => global.set_transcript_jump_edge(value),
        Role::TranscriptJumpMark => global.set_transcript_jump_mark(value),
        Role::ComposerSurface => global.set_composer_surface(value),
        Role::ComposerEdge => global.set_composer_edge(value),
        Role::ComposerHandleFill => global.set_composer_handle_fill(value),
        Role::ComposerHandleFillHover => global.set_composer_handle_fill_hover(value),
        Role::ComposerHandleFillDragging => global.set_composer_handle_fill_dragging(value),
        Role::ComposerDraftInk => global.set_composer_draft_ink(value),
        Role::ComposerDraftCaret => global.set_composer_draft_caret(value),
        Role::ComposerDraftSelectionFill => global.set_composer_draft_selection_fill(value),
        Role::ComposerDraftSelectionInk => global.set_composer_draft_selection_ink(value),
        Role::ComposerDraftCommandInk => global.set_composer_draft_command_ink(value),
        Role::ComposerDraftSelectionWash => global.set_composer_draft_selection_wash(value),
        Role::ComposerCommandInk => global.set_composer_command_ink(value),
        Role::ComposerAttachmentFill => global.set_composer_attachment_fill(value),
        Role::ComposerAttachmentEdge => global.set_composer_attachment_edge(value),
        Role::ComposerAttachmentEdgeHover => global.set_composer_attachment_edge_hover(value),
        Role::ComposerAttachmentRemoveFill => global.set_composer_attachment_remove_fill(value),
        Role::ComposerAttachmentRemoveInk => global.set_composer_attachment_remove_ink(value),
        Role::ComposerAttachFill => global.set_composer_attach_fill(value),
        Role::ComposerAttachFillHover => global.set_composer_attach_fill_hover(value),
        Role::ComposerAttachInk => global.set_composer_attach_ink(value),
        Role::ComposerAttachInkHover => global.set_composer_attach_ink_hover(value),
        Role::ComposerStatusLineInk => global.set_composer_status_line_ink(value),
        Role::ComposerStatusLineInkBad => global.set_composer_status_line_ink_bad(value),
        Role::ComposerRingTickLit => global.set_composer_ring_tick_lit(value),
        Role::ComposerRingTickUnlit => global.set_composer_ring_tick_unlit(value),
        Role::ComposerCardSurface => global.set_composer_card_surface(value),
        Role::ComposerCardEdge => global.set_composer_card_edge(value),
        Role::ComposerCardShadow => global.set_composer_card_shadow(value),
        Role::ComposerCardLabelInk => global.set_composer_card_label_ink(value),
        Role::ComposerCardPercentInk => global.set_composer_card_percent_ink(value),
        Role::ComposerCardBarTrack => global.set_composer_card_bar_track(value),
        Role::ComposerCardBarFill => global.set_composer_card_bar_fill(value),
        Role::ComposerCardUsedInk => global.set_composer_card_used_ink(value),
        Role::ComposerCardRule => global.set_composer_card_rule(value),
        Role::ComposerCardLedgerInk => global.set_composer_card_ledger_ink(value),
        Role::ComposerCardCostInk => global.set_composer_card_cost_ink(value),
        Role::ComposerChipFill => global.set_composer_chip_fill(value),
        Role::ComposerChipFillHover => global.set_composer_chip_fill_hover(value),
        Role::ComposerChipEdge => global.set_composer_chip_edge(value),
        Role::ComposerChipEdgeHover => global.set_composer_chip_edge_hover(value),
        Role::ComposerChipInk => global.set_composer_chip_ink(value),
        Role::ComposerChipInkHover => global.set_composer_chip_ink_hover(value),
        Role::ComposerChipCaretMark => global.set_composer_chip_caret_mark(value),
        Role::ComposerSendFill => global.set_composer_send_fill(value),
        Role::ComposerSendFillHover => global.set_composer_send_fill_hover(value),
        Role::ComposerSendMark => global.set_composer_send_mark(value),
        Role::ComposerStopMark => global.set_composer_stop_mark(value),
        Role::ButtonQuietFill => global.set_button_quiet_fill(value),
        Role::ButtonQuietFillHover => global.set_button_quiet_fill_hover(value),
        Role::ButtonPrimaryFill => global.set_button_primary_fill(value),
        Role::ButtonPrimaryFillHover => global.set_button_primary_fill_hover(value),
        Role::ButtonDangerFill => global.set_button_danger_fill(value),
        Role::ButtonDangerFillHover => global.set_button_danger_fill_hover(value),
        Role::PanelSurface => global.set_panel_surface(value),
        Role::PanelEdge => global.set_panel_edge(value),
        Role::PanelShadow => global.set_panel_shadow(value),
        Role::PanelShadowNested => global.set_panel_shadow_nested(value),
        Role::PanelRowFillHover => global.set_panel_row_fill_hover(value),
        Role::PanelRowFillCurrent => global.set_panel_row_fill_current(value),
        Role::SessionCardEdge => global.set_session_card_edge(value),
        Role::SessionCardInk => global.set_session_card_ink(value),
        Role::SlashMenuRowInk => global.set_slash_menu_row_ink(value),
        Role::SlashMenuRowInkHover => global.set_slash_menu_row_ink_hover(value),
        Role::SlashMenuHintInk => global.set_slash_menu_hint_ink(value),
        Role::SlashMenuDescriptionInk => global.set_slash_menu_description_ink(value),
        Role::ModelMenuHeaderInk => global.set_model_menu_header_ink(value),
        Role::ModelMenuHeaderInkQuiet => global.set_model_menu_header_ink_quiet(value),
        Role::ModelMenuCaretMark => global.set_model_menu_caret_mark(value),
        Role::ModelMenuCountInk => global.set_model_menu_count_ink(value),
        Role::ModelMenuRowInk => global.set_model_menu_row_ink(value),
        Role::ModelMenuRowInkCurrent => global.set_model_menu_row_ink_current(value),
        Role::ModelMenuCheckMark => global.set_model_menu_check_mark(value),
        Role::ModelMenuDetailInk => global.set_model_menu_detail_ink(value),
        Role::ModelMenuEffortInk => global.set_model_menu_effort_ink(value),
        Role::EffortLadderRowInk => global.set_effort_ladder_row_ink(value),
        Role::EffortLadderRowInkCurrent => global.set_effort_ladder_row_ink_current(value),
        Role::EffortLadderCheckMark => global.set_effort_ladder_check_mark(value),
        Role::QueueCardCountInk => global.set_queue_card_count_ink(value),
        Role::QueueCardLaneInk => global.set_queue_card_lane_ink(value),
        Role::QueueRowInk => global.set_queue_row_ink(value),
        Role::QueueCardClearInk => global.set_queue_card_clear_ink(value),
        Role::QueueCardClearInkHover => global.set_queue_card_clear_ink_hover(value),
        Role::QueueRowControlFill => global.set_queue_row_control_fill(value),
        Role::QueueRowControlFillHover => global.set_queue_row_control_fill_hover(value),
        Role::QueueRowControlMark => global.set_queue_row_control_mark(value),
        Role::QueueRowControlMarkHover => global.set_queue_row_control_mark_hover(value),
        Role::QueueRowPromoteMark => global.set_queue_row_promote_mark(value),
        Role::QueueRowPromoteMarkHover => global.set_queue_row_promote_mark_hover(value),
        Role::QueueRowDeleteMark => global.set_queue_row_delete_mark(value),
        Role::StatusStripSurface => global.set_status_strip_surface(value),
        Role::StatusStripEdge => global.set_status_strip_edge(value),
        Role::StatusStripWorkingInk => global.set_status_strip_working_ink(value),
        Role::StatusStripElapsedInk => global.set_status_strip_elapsed_ink(value),
        Role::StatusStripQueueFill => global.set_status_strip_queue_fill(value),
        Role::StatusStripQueueFillHover => global.set_status_strip_queue_fill_hover(value),
        Role::StatusStripQueueCountInk => global.set_status_strip_queue_count_ink(value),
        Role::StatusStripQueueCountInkHover => {
            global.set_status_strip_queue_count_ink_hover(value)
        }
        Role::StatusStripQueueLabelInk => global.set_status_strip_queue_label_ink(value),
        Role::StatusStripQueueLabelInkHover => {
            global.set_status_strip_queue_label_ink_hover(value)
        }
    }
}


/// A second theme for the tests that have to see one theme follow another: the
/// window turned over, with both inks moved so a role made of either follows it.
/// Not a bundled theme — the two consumers' tests are the only callers.
#[cfg(test)]
pub(crate) fn light() -> Resolved {
    from_text(
        "light.json",
        r##"{ "name": "light", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020", "quiet-ink": "#5a5a5a" } }"##,
    )
    .expect("a theme that turns the window light")
}