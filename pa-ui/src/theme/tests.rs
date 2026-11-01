//! What the theme must promise, asserted through its own interface: the map a
//! file resolves to, the message a bad file gets, and the pairs a theme leaves
//! unreadable.
//!
//! Resolution is pure — text in, a complete map out — so every case but the
//! command line's runs with no window, no file on disk and no UI source.

use super::*;

/// The roles a file moved, beside the roles whose names claim a part of the
/// window: a file that names one part must move exactly that part and nothing
/// else, and the two lists are what says so.
fn moved_and_claimed(
    theme: &Resolved,
    claimed: impl Fn(&str) -> bool,
) -> (Vec<&'static str>, Vec<&'static str>) {
    let shipped = Resolved::shipped();
    let moved = Role::ALL
        .iter()
        .filter(|role| theme.get(**role) != shipped.get(**role))
        .map(|role| role.name())
        .collect();
    let named = Role::ALL.iter().map(|role| role.name()).filter(|name| claimed(name)).collect();
    (moved, named)
}

/// The title bar as the window drew it before there was a theme: the values
/// read off `ui/chrome.slint` and `ui/icons.slint`, not out of this crate.
#[test]
fn the_shipped_theme_draws_todays_title_bar() {
    let theme = Resolved::shipped();
    let on_background = |role| theme.get(role).over(theme.background());

    // The marks come out as the one opaque colour each was drawn in.
    assert_eq!(theme.get(Role::TitleBarGlyph), Rgba::hex("#d7d7d7"));
    assert_eq!(theme.get(Role::TitleBarFoldGlyph), Rgba::hex("#6f6f6f"));
    assert_eq!(theme.get(Role::TitleBarFoldGlyphHover), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::TitleBarCloseFillHover), Rgba::hex("#c42b1c"));
    assert_eq!(theme.get(Role::TitleBarCloseInkHover), Rgba::hex("#ffffff"));
    // The hover chips are veils, so they keep their alpha and are laid on the
    // window behind them.
    assert_eq!(on_background(Role::TitleBarCaptionFillHover), Rgba::hex("#222222"));
    assert_eq!(on_background(Role::TitleBarFoldFillHover), Rgba::hex("#2b2b2b"));
}

/// The sidebar as the window drew it before there was a theme: the values read
/// off `ui/sidebar.slint` and the column in `ui/app.slint`, not out of this
/// crate.
#[test]
fn the_shipped_theme_draws_todays_sidebar() {
    let theme = Resolved::shipped();

    assert_eq!(theme.get(Role::SidebarSurface), Rgba::hex("#111111"));
    // A row at rest draws no fill of its own: the column runs through it.
    assert_eq!(theme.get(Role::SidebarRowFill).a, 0, "a resting row is not a fill");
    assert_eq!(theme.get(Role::SidebarRowFillHover), Rgba::hex("#181818"));
    assert_eq!(theme.get(Role::SidebarRowFillSelected), Rgba::hex("#1d1d1d"));
    assert_eq!(theme.get(Role::SidebarNameInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::SidebarNameInkSelected), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::SidebarNameInkEditing), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::SidebarAgeInk), Rgba::hex("#585858"));
    // The three tags are the one part of the sidebar whose value no palette
    // entry can describe, so the window keeps them and a theme names them.
    assert_eq!(theme.get(Role::TagWorking), Rgba::hex("#a18e41"));
    assert_eq!(theme.get(Role::TagParked), Rgba::hex("#4a4d44"));
    assert_eq!(theme.get(Role::TagIdle), Rgba::hex("#6f9f5f"));
}

/// The glyphs as the window drew them before there was a theme: the values read
/// off `ui/icons.slint`, and the tints six UI sources passed beside them, not out
/// of this crate.
///
/// A glyph defaults to the role of the part it rests in, and a part that draws
/// the same shape somewhere else names its own — the two folds, the pen in the
/// folded strip, the caret in the model menu and on the chip. Both roles are here.
#[test]
fn the_shipped_theme_draws_todays_glyphs() {
    let theme = Resolved::shipped();

    // The sidebar: its fold in the header, the pen on New Session in the header
    // and in the folded strip, the magnifier both carry, and the pencil a row
    // offers to rename it.
    assert_eq!(theme.get(Role::SidebarFoldGlyph), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::SidebarFoldGlyphHover), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::SidebarNewSessionMark), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::SidebarNewSessionMarkHover), Rgba::hex("#e5e5e5"));
    // The strip's own pen reads a step above the button's: folded, the mark is
    // the whole control.
    assert_eq!(theme.get(Role::SidebarStripNewSessionMark), Rgba::hex("#d7d7d7"));
    assert_eq!(theme.get(Role::SidebarStripNewSessionMarkHover), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::SidebarSearchMark), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::SidebarRowRenameMark), Rgba::hex("#8b8b8b"));

    // The transcript: the caret on a row that opens — a thinking row and a cell
    // draw the same shape — the chevron on the chip that walks back to the
    // newest row, and the alert inside a failed cell's badge.
    assert_eq!(theme.get(Role::TranscriptCaretMark), Rgba::hex("#7b7b7b"));
    assert_eq!(theme.get(Role::TranscriptJumpMark), Rgba::hex("#b2b2b2"));
    assert_eq!(theme.get(Role::TranscriptCellBadgeMark), Rgba::hex("#f3ded8"));

    // The panels the caret and the tick are drawn in: the model menu's own
    // caret, its tick beside the running model, and the tick the ladder draws
    // on its current rung.
    assert_eq!(theme.get(Role::ModelMenuCaretMark), Rgba::hex("#7b7b7b"));
    assert_eq!(theme.get(Role::ModelMenuCheckMark), Rgba::hex("#a18e41"));
    assert_eq!(theme.get(Role::EffortLadderCheckMark), Rgba::hex("#a18e41"));

    // And the caret beside the composer's chip, which opens the model menu.
    assert_eq!(theme.get(Role::ComposerChipCaretMark), Rgba::hex("#7b7b7b"));
}

/// The brand mark's six fills as the artwork drew them before they were roles:
/// the wing and the stem, the three shadows they cast, and the blade over them
/// all.
///
/// Five of the six are the brand tone mixed towards the column the mark is cut
/// from, and the shares are the artwork's own proportions, so the values here are
/// what those proportions come to on the field the window ships.
#[test]
fn the_shipped_theme_draws_todays_brand_mark() {
    let theme = Resolved::shipped();

    assert_eq!(theme.get(Role::BrandMarkBlade), Rgba::hex("#ffffff"));
    assert_eq!(theme.get(Role::BrandMarkWing), Rgba::hex("#aeaeae"));
    assert_eq!(theme.get(Role::BrandMarkWingInnerShadow), Rgba::hex("#838383"));
    assert_eq!(theme.get(Role::BrandMarkWingShadow), Rgba::hex("#5a5a5a"));
    assert_eq!(theme.get(Role::BrandMarkStem), Rgba::hex("#636363"));
    assert_eq!(theme.get(Role::BrandMarkStemShadow), Rgba::hex("#333333"));
}

/// Every glyph the palette makes follows the inks it is made of, and the mark
/// follows the brand tone and the field under it: a reader who moves the ink and
/// the brand sees the whole window's marks move without naming one of them.
#[test]
fn a_theme_that_moves_the_inks_repaints_the_glyphs_and_the_mark() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020",
                          "quiet-ink": "#3a3a3a", "brand": "#1a1a1a" } }"##,
    )
    .expect("a file that names only base values");
    let shipped = Resolved::shipped();

    for role in [
        Role::SidebarFoldGlyph,
        Role::SidebarFoldGlyphHover,
        Role::SidebarNewSessionMark,
        Role::SidebarNewSessionMarkHover,
        Role::SidebarStripNewSessionMark,
        Role::SidebarStripNewSessionMarkHover,
        Role::SidebarSearchMark,
        Role::SidebarRowRenameMark,
        Role::TranscriptCaretMark,
        Role::TranscriptJumpMark,
        Role::ModelMenuCaretMark,
        Role::ComposerChipCaretMark,
        Role::BrandMarkBlade,
        Role::BrandMarkWing,
        Role::BrandMarkWingInnerShadow,
        Role::BrandMarkWingShadow,
        Role::BrandMarkStem,
        Role::BrandMarkStemShadow,
    ] {
        assert_ne!(theme.get(role), shipped.get(role), "{} did not follow", role.name());
    }

    // Three glyphs are the exception every part with a hue has: the alert inside
    // a failed cell's badge and the two ticks that mark what is running are drawn
    // in colours no palette word describes, so they stay values a theme names
    // outright rather than being mixed out of the ink. The check measures each of
    // them where it is drawn.
    for role in [Role::TranscriptCellBadgeMark, Role::ModelMenuCheckMark, Role::EffortLadderCheckMark]
    {
        assert_eq!(theme.get(role), shipped.get(role), "{} is not made of the inks", role.name());
    }
}

/// The mark is one drawing cut from the column, so a theme that turns the window
/// over turns the mark over with it: the blade goes dark, the shadows go light,
/// and every tone keeps its distance from the one it is laid on.
#[test]
fn an_inverted_theme_keeps_the_brand_marks_tones_apart() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#1a1a1a",
                          "quiet-ink": "#3a3a3a", "brand": "#1a1a1a" } }"##,
    )
    .expect("a file that turns the window light");

    let named: Vec<&str> = separation(&theme).iter().map(|clash| clash.what).collect();
    assert!(!named.iter().any(|what| what.starts_with("the mark's")), "{named:?}");

    // Turned over rather than merely paler: the tone that was the lightest is
    // now the darkest, and the shadow that was the darkest is now the lightest.
    let blade = theme.get(Role::BrandMarkBlade);
    let shadow = theme.get(Role::BrandMarkStemShadow);
    assert!(blade.r < shadow.r, "{blade:?} {shadow:?}");

    // And the two glyphs no palette word describes pay for it where the reader
    // can hear about it: the gold of the tick that says what is running stays
    // gold, so on a light panel the ratio check names it. A theme that wants the
    // two read high names them — the same trade the sidebar's tags make.
    let findings: Vec<&str> = contrast(&theme).iter().map(|finding| finding.what).collect();
    assert!(findings.contains(&"the tick beside the running model"), "{findings:?}");
}

/// The other side of the mark's bargain: it is artwork cut from the field, not ink
/// laid on it, so a file that moves only the inks leaves it exactly where it was.
/// What turns the mark over is the field it is cut from and the brand tone, which
/// is why an inversion that names both leaves the drawing whole.
#[test]
fn a_theme_that_moves_only_the_inks_leaves_the_mark_alone() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "ink": "#f0f0f0", "quiet-ink": "#d0d0d0" } }"##,
    )
    .expect("a file that moves only the inks");
    let shipped = Resolved::shipped();

    for role in [
        Role::BrandMarkBlade,
        Role::BrandMarkWing,
        Role::BrandMarkWingInnerShadow,
        Role::BrandMarkWingShadow,
        Role::BrandMarkStem,
        Role::BrandMarkStemShadow,
    ] {
        assert_eq!(theme.get(role), shipped.get(role), "{} follows the field", role.name());
    }
}

/// The other half of the same promise: a theme that turns the window light and
/// leaves the brand tone white mixes every shadow into the field it is cut from,
/// and the mark flattens into a shape nobody can read. Nothing refuses the file —
/// a theme is the theme its author asked for — and the check names all four
/// places the drawing came apart.
#[test]
fn a_theme_that_leaves_the_brand_tone_behind_flattens_the_mark() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020" } }"##,
    )
    .expect("a file that turns the window light");

    let named: Vec<&str> = separation(&theme).iter().map(|clash| clash.what).collect();
    let flattened: Vec<&&str> =
        named.iter().filter(|what| what.starts_with("the mark's")).collect();

    assert_eq!(flattened.len(), 4, "{named:?}");
}

/// A file may name a single glyph's role: it moves that glyph and leaves the rest
/// of the window where it was, which is how a reader fixes the one mark that
/// looks wrong under their field.
#[test]
fn a_theme_that_names_a_glyphs_role_moves_that_glyph_alone() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1, "roles": { "transcript-caret-mark": "#c81e1e" } }"##,
    )
    .expect("a file that names one glyph");

    let (moved, claimed) = moved_and_claimed(&theme, |name| name == "transcript-caret-mark");

    assert_eq!(moved, claimed);
}

/// The transcript as the window drew it before there was a theme: the values
/// read off `ui/transcript.slint` and the column and failure reason in
/// `ui/app.slint`, not out of this crate.
#[test]
fn the_shipped_theme_draws_todays_transcript() {
    let theme = Resolved::shipped();

    // The field the rows are drawn on, and the field the reader's own message
    // gets: both a lift of the window's ink over its background.
    assert_eq!(theme.get(Role::TranscriptSurface), Rgba::hex("#131313"));
    assert_eq!(theme.get(Role::TranscriptMessageFill), Rgba::hex("#292929"));
    assert_eq!(theme.get(Role::TranscriptMessageInk), Rgba::hex("#e5e5e5"));

    // Prose, and the ink a row steps back to.
    assert_eq!(theme.get(Role::TranscriptProseInk), Rgba::hex("#d1d1d1"));
    assert_eq!(theme.get(Role::TranscriptLinkInk), Rgba::hex("#8f8f8f"));
    assert_eq!(theme.get(Role::TranscriptNoticeInk), Rgba::hex("#4c4c4c"));

    // A cell in each state it is drawn in.
    assert_eq!(theme.get(Role::TranscriptCellFill), Rgba::hex("#151515"));
    assert_eq!(theme.get(Role::TranscriptCellFillHover), Rgba::hex("#191919"));
    assert_eq!(theme.get(Role::TranscriptCellTitleInk), Rgba::hex("#8b8b8b"));
    assert_eq!(theme.get(Role::TranscriptCellMetaInk), Rgba::hex("#4c4c4c"));
    assert_eq!(theme.get(Role::TranscriptCellSeparatorInk), Rgba::hex("#ffffff"));
    assert_eq!(theme.get(Role::TranscriptCellCodeInk), Rgba::hex("#d1d1d1"));
    assert_eq!(theme.get(Role::TranscriptCellOutputInk), Rgba::hex("#909090"));

    // And the two panels a row opens, which sit below the field they open on.
    assert_eq!(theme.get(Role::TranscriptRowFillExpanded), Rgba::hex("#101010"));
    assert_eq!(theme.get(Role::TranscriptCellEdgeExpanded), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::TranscriptThinkingEdgeExpanded), Rgba::hex("#1f1f1f"));
    assert_eq!(theme.get(Role::TranscriptCellRule), Rgba::hex("#1f1f1f"));

    // A thinking row: it draws no fill of its own until the pointer arrives.
    assert_eq!(theme.get(Role::TranscriptThinkingFill).a, 0, "a resting thinking row is not a fill");
    assert_eq!(theme.get(Role::TranscriptThinkingFillHover), Rgba::hex("#1b1b1b"));
    assert_eq!(theme.get(Role::TranscriptThinkingLabelInk), Rgba::hex("#6b6b6b"));
    assert_eq!(theme.get(Role::TranscriptThinkingInk), Rgba::hex("#7f7f7f"));

    // A fenced block, the table's text, and the field behind a picture.
    assert_eq!(theme.get(Role::TranscriptCodeFill), Rgba::hex("#0e0e0e"));
    assert_eq!(theme.get(Role::TranscriptCodeInk), Rgba::hex("#b7b7b7"));
    assert_eq!(theme.get(Role::TranscriptCodeLabelInk), Rgba::hex("#585858"));
    assert_eq!(theme.get(Role::TranscriptTableInk), Rgba::hex("#aaaaaa"));
    assert_eq!(theme.get(Role::TranscriptImageFill), Rgba::hex("#1b1b1b"));

    // The transcript's own chrome, and the chip that walks back to the floor.
    assert_eq!(theme.get(Role::TranscriptScrollThumb), Rgba::hex("#313131"));
    assert_eq!(theme.get(Role::TranscriptScrollThumbHover), Rgba::hex("#454545"));
    assert_eq!(theme.get(Role::TranscriptScrollThumbPressed), Rgba::hex("#585858"));
    assert_eq!(theme.get(Role::TranscriptJumpFill), Rgba::hex("#1c1c1c"));
    assert_eq!(theme.get(Role::TranscriptJumpFillHover), Rgba::hex("#2f2f2f"));
    assert_eq!(theme.get(Role::TranscriptJumpEdge), Rgba::hex("#363636"));
}

/// What a failure is drawn in: a warmed red the palette has no word for, and the
/// fields it is read on. The reason the badge opens is drawn in the failed
/// cell's own field, so it adds only its edge and its ink.
#[test]
fn the_shipped_theme_draws_todays_failure() {
    let theme = Resolved::shipped();

    assert_eq!(theme.get(Role::TranscriptCellFillFailed), Rgba::hex("#1d1412"));
    assert_eq!(theme.get(Role::TranscriptCellFillFailedHover), Rgba::hex("#241713"));
    assert_eq!(theme.get(Role::TranscriptCellInkFailed), Rgba::hex("#c98a7f"));
    assert_eq!(theme.get(Role::TranscriptCellBadgeFill), Rgba::hex("#6b342b"));
    assert_eq!(theme.get(Role::TranscriptCellBadgeFillHover), Rgba::hex("#8d4235"));
    assert_eq!(theme.get(Role::TranscriptReasonEdge), Rgba::hex("#6b342b"));
    assert_eq!(theme.get(Role::TranscriptReasonInk), Rgba::hex("#e0b6ad"));
}

/// The composer as the window drew it before there was a theme: the values read
/// off `ui/composer.slint`, `ui/draft-editor.slint` and the composer band in
/// `ui/app.slint`, not out of this crate.
#[test]
fn the_shipped_theme_draws_todays_composer() {
    let theme = Resolved::shipped();

    // The box the reader writes in, and the bar on its edge that grows it.
    assert_eq!(theme.get(Role::ComposerSurface), Rgba::hex("#101010"));
    assert_eq!(theme.get(Role::ComposerEdge), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::ComposerHandleFill), Rgba::hex("#353535"));
    assert_eq!(theme.get(Role::ComposerHandleFillHover), Rgba::hex("#4c4c4c"));
    assert_eq!(theme.get(Role::ComposerHandleFillDragging), Rgba::hex("#6d6d6d"));

    // The draft, its caret and the field behind what the reader has selected.
    assert_eq!(theme.get(Role::ComposerDraftInk), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::ComposerDraftCaret), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::ComposerDraftSelectionFill), Rgba::hex("#3d3d3d"));
    assert_eq!(theme.get(Role::ComposerDraftSelectionInk), Rgba::hex("#f0f0f0"));

    // The command word, and whatever the draft is carrying.
    assert_eq!(theme.get(Role::ComposerCommandInk), Rgba::hex("#d8c06a"));
    assert_eq!(theme.get(Role::ComposerAttachmentFill), Rgba::hex("#191919"));
    assert_eq!(theme.get(Role::ComposerAttachmentEdge), Rgba::hex("#2b2b2b"));
    assert_eq!(theme.get(Role::ComposerAttachmentEdgeHover), Rgba::hex("#3c3c3c"));
    assert_eq!(theme.get(Role::ComposerAttachmentRemoveFill), Rgba::hex("#0c0c0c"));
    assert_eq!(theme.get(Role::ComposerAttachmentRemoveInk), Rgba::hex("#d6d6d6"));

    // The plus, and the line under the draft.
    assert_eq!(theme.get(Role::ComposerAttachFill), Rgba::hex("#1a1a1a"));
    assert_eq!(theme.get(Role::ComposerAttachFillHover), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::ComposerAttachInk), Rgba::hex("#8c8c8c"));
    assert_eq!(theme.get(Role::ComposerAttachInkHover), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::ComposerStatusLineInk), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::ComposerStatusLineInkBad), Rgba::hex("#c98a7f"));

    // The ring, and the card that opens off it.
    assert_eq!(theme.get(Role::ComposerRingTickLit), Rgba::hex("#c9a86a"));
    assert_eq!(theme.get(Role::ComposerRingTickUnlit), Rgba::hex("#353535"));
    assert_eq!(theme.get(Role::ComposerCardSurface), Rgba::hex("#181818"));
    assert_eq!(theme.get(Role::ComposerCardEdge), Rgba::hex("#2b2b2b"));
    assert_eq!(theme.get(Role::ComposerCardShadow), Rgba::hex("#00000066"));
    assert_eq!(theme.get(Role::ComposerCardLabelInk), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::ComposerCardPercentInk), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::ComposerCardBarTrack), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::ComposerCardBarFill), Rgba::hex("#8f8f8f"));
    assert_eq!(theme.get(Role::ComposerCardUsedInk), Rgba::hex("#8b8b8b"));
    assert_eq!(theme.get(Role::ComposerCardRule), Rgba::hex("#1d1d1d"));
    assert_eq!(theme.get(Role::ComposerCardLedgerInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::ComposerCardCostInk), Rgba::hex("#d7d7d7"));

    // The chip that names the model.
    assert_eq!(theme.get(Role::ComposerChipFill), Rgba::hex("#1a1a1a"));
    assert_eq!(theme.get(Role::ComposerChipFillHover), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::ComposerChipEdge), Rgba::hex("#242424"));
    assert_eq!(theme.get(Role::ComposerChipEdgeHover), Rgba::hex("#353535"));
    assert_eq!(theme.get(Role::ComposerChipInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::ComposerChipInkHover), Rgba::hex("#e5e5e5"));

    // The one control, in both of the jobs it does.
    assert_eq!(theme.get(Role::ComposerSendFill), Rgba::hex("#e6e6e6"));
    assert_eq!(theme.get(Role::ComposerSendFillHover), Rgba::hex("#f2f2f2"));
    assert_eq!(theme.get(Role::ComposerSendMark), Rgba::hex("#131313"));
    assert_eq!(theme.get(Role::ComposerStopFill), Rgba::hex("#a2543f"));
    assert_eq!(theme.get(Role::ComposerStopFillHover), Rgba::hex("#b8614c"));
    assert_eq!(theme.get(Role::ComposerStopMark), Rgba::hex("#f0f0f0"));
}

/// The composer is made of the base values like every other part: a file that
/// moves the window's colours carries the box, the draft and the card with it,
/// which is what makes a light composer possible without naming every role.
#[test]
fn a_theme_that_moves_the_base_values_moves_the_composer_with_them() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020" } }"##,
    )
    .expect("a file that names only base values");

    let shipped = Resolved::shipped();
    // The box sits *below* its band, so on a light window it steps further from
    // the ink rather than towards it.
    let field = theme.get(Role::ComposerSurface);
    assert!(field.r > shipped.get(Role::ComposerSurface).r, "{field:?}");
    assert_eq!(theme.get(Role::ComposerDraftInk), Rgba::hex("#202020"));
    assert_eq!(theme.get(Role::ComposerChipInkHover), Rgba::hex("#202020"));
    // The chip and the boxes in it are a lift of ink over the background, so on a
    // light window they step darker than the field and stay light: the lift is
    // the same few per cent of the ink, which is now the dark end.
    let chip = theme.get(Role::ComposerChipFillHover);
    assert!(chip.r < 0xf5 && chip.r > 0x20, "{chip:?}");
}

/// The one control does two opposite jobs, so a theme that paints the stop like
/// the send has taken the difference away: nothing but the fill says which of
/// the two the click will do. The check names the pair.
#[test]
fn a_theme_that_paints_the_stop_like_the_send_is_reported() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": { "composer-stop-fill": "#e6e6e6",
                        "composer-stop-fill-hover": "#f2f2f2" } }"##,
    )
    .expect("a theme that loads");

    let named: Vec<&str> = separation(&theme).iter().map(|clash| clash.what).collect();

    assert_eq!(named, ["the send disc and the stop disc"]);
}

/// A file that names the composer's colours repaints the composer and moves
/// nothing else in the map.
#[test]
fn a_theme_that_names_the_composer_roles_moves_only_those() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": {
               "composer-surface": "#1a1226",
               "composer-edge": "#3a2a52",
               "composer-handle-fill": "#3a2a52",
               "composer-handle-fill-hover": "#4a3a63",
               "composer-handle-fill-dragging": "#5f4d7d",
               "composer-draft-ink": "#eee6ff",
               "composer-draft-caret": "#eee6ff",
               "composer-draft-selection-fill": "#4a3a63",
               "composer-draft-selection-ink": "#ffffff",
               "composer-command-ink": "#e0b6ad",
               "composer-attachment-fill": "#241a33",
               "composer-attachment-edge": "#3a2a52",
               "composer-attachment-edge-hover": "#4a3a63",
               "composer-attachment-remove-fill": "#140f1c",
               "composer-attachment-remove-ink": "#ded2f2",
               "composer-attach-fill": "#241a33",
               "composer-attach-fill-hover": "#2f2242",
               "composer-attach-ink": "#9a8fb0",
               "composer-attach-ink-hover": "#cfc0e8",
               "composer-status-line-ink": "#8a7fa0",
               "composer-status-line-ink-bad": "#ff9f8f",
               "composer-ring-tick-lit": "#c9a7ff",
               "composer-ring-tick-unlit": "#3a2a52",
               "composer-card-surface": "#1c1526",
               "composer-card-edge": "#3a2a52",
               "composer-card-shadow": "#00000099",
               "composer-card-label-ink": "#9a8fb0",
               "composer-card-percent-ink": "#eee6ff",
               "composer-card-bar-track": "#3a2a52",
               "composer-card-bar-fill": "#c9a7ff",
               "composer-card-used-ink": "#b0a3c8",
               "composer-card-rule": "#33254a",
               "composer-card-ledger-ink": "#cfc0e8",
               "composer-card-cost-ink": "#eee6ff",
               "composer-chip-fill": "#241a33",
               "composer-chip-fill-hover": "#2f2242",
               "composer-chip-edge": "#3a2a52",
               "composer-chip-edge-hover": "#4a3a63",
               "composer-chip-ink": "#cfc0e8",
               "composer-chip-ink-hover": "#eee6ff",
               "composer-chip-caret-mark": "#a99bc4",
               "composer-send-fill": "#c9a7ff",
               "composer-send-fill-hover": "#ded2f2",
               "composer-send-mark": "#140f1c",
               "composer-stop-fill": "#ff9f8f",
               "composer-stop-fill-hover": "#ffd0c4",
               "composer-stop-mark": "#140f1c" } }"##,
    )
    .expect("a file that repaints the composer");

    // The file names every colour the composer has, and nothing else moves.
    let (moved, claimed) = moved_and_claimed(&theme, |name| name.starts_with("composer-"));
    assert_eq!(moved, claimed);
}

/// The transcript is made of the base values like every other part: a file that
/// moves the field and the ink carries the sheet of rows with it, which is what
/// makes a light transcript possible without naming every role by hand.
#[test]
fn a_theme_that_moves_the_base_values_moves_the_transcript_with_them() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020" } }"##,
    )
    .expect("a file that names only base values");

    let shipped = Resolved::shipped();
    for field in [
        Role::TranscriptSurface,
        Role::TranscriptCellFill,
        Role::TranscriptRowFillExpanded,
        Role::TranscriptCodeFill,
    ] {
        assert_ne!(theme.get(field), shipped.get(field), "{} follows the field", field.name());
        assert!(theme.get(field).r > 0x80, "{} is a field of a light window", field.name());
    }
    for ink in [Role::TranscriptProseInk, Role::TranscriptMessageInk, Role::TranscriptCodeInk] {
        assert_ne!(theme.get(ink), shipped.get(ink), "{} follows the ink", ink.name());
        assert!(theme.get(ink).r < 0x80, "{} is ink on a light window", ink.name());
    }
}

/// The failure keeps the colours it is drawn in, because the palette has no word
/// for them: a theme that wants a failed cell to read on its own field has to
/// name the roles, and the check tells it which when it does not.
#[test]
fn the_theme_that_names_only_base_values_leaves_the_failure_where_it_was() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020", "danger": "#7a1f18" } }"##,
    )
    .expect("a file that names only base values");

    assert_eq!(theme.get(Role::TranscriptCellFillFailed), Rgba::hex("#1d1412"));
    assert_eq!(theme.get(Role::TranscriptCellInkFailed), Rgba::hex("#c98a7f"));
}

/// A failed cell is distinguishable because of its field's tint and the red its
/// name is written in. A theme that paints both like a quiet cell has taken the
/// failure away, and is told which of the two it lost.
#[test]
fn a_theme_that_paints_a_failed_cell_like_a_quiet_one_is_reported() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": { "transcript-cell-fill-failed": "#151515",
                        "transcript-cell-ink-failed": "#8b8b8b" } }"##,
    )
    .expect("a theme that loads");

    let named: Vec<&str> = separation(&theme).iter().map(|clash| clash.what).collect();

    assert_eq!(named.len(), 2, "{named:?}");
    assert!(named.contains(&"a failed cell's field and a quiet cell's field"), "{named:?}");
    assert!(named.contains(&"a failed cell's name and a quiet cell's name"), "{named:?}");
}

/// A file that names the transcript's colours repaints the transcript and moves
/// nothing else in the map.
#[test]
fn a_theme_that_names_the_transcript_roles_moves_only_those() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": {
               "transcript-surface": "#1a1226",
               "transcript-prose-ink": "#eee6ff",
               "transcript-message-fill": "#2b1f3d",
               "transcript-message-ink": "#ffffff",
               "transcript-link-ink": "#c9a7ff",
               "transcript-notice-ink": "#8a7fa0",
               "transcript-caret-mark": "#a99bc4",
               "transcript-cell-fill": "#241a33",
               "transcript-cell-fill-hover": "#2f2242",
               "transcript-cell-title-ink": "#cfc0e8",
               "transcript-cell-meta-ink": "#8a7fa0",
               "transcript-cell-separator-ink": "#ffe9f4",
               "transcript-cell-code-ink": "#eee6ff",
               "transcript-cell-output-ink": "#b9a9d6",
               "transcript-cell-rule": "#3a2a52",
               "transcript-cell-edge-expanded": "#3a2a52",
               "transcript-thinking-edge-expanded": "#33254a",
               "transcript-cell-fill-failed": "#3d1f24",
               "transcript-cell-fill-failed-hover": "#4a262c",
               "transcript-cell-ink-failed": "#ff9f8f",
               "transcript-cell-badge-fill": "#8a3428",
               "transcript-cell-badge-fill-hover": "#a5402f",
               "transcript-cell-badge-mark": "#ffe9e2",
               "transcript-reason-edge": "#8a3428",
               "transcript-reason-ink": "#ffd0c4",
               "transcript-row-fill-expanded": "#140f1c",
               "transcript-thinking-fill": "#1c1526",
               "transcript-thinking-fill-hover": "#251c33",
               "transcript-thinking-label-ink": "#9a8fb0",
               "transcript-thinking-ink": "#b0a3c8",
               "transcript-code-fill": "#120d1a",
               "transcript-code-ink": "#ded2f2",
               "transcript-code-label-ink": "#7d7193",
               "transcript-table-ink": "#c8bae0",
               "transcript-image-fill": "#241a33",
               "transcript-scroll-thumb": "#4a3a63",
               "transcript-scroll-thumb-hover": "#5f4d7d",
               "transcript-scroll-thumb-pressed": "#7a6699",
               "transcript-jump-fill": "#251c33",
               "transcript-jump-fill-hover": "#3a2c50",
               "transcript-jump-edge": "#4a3a63",
               "transcript-jump-mark": "#d8ccee" } }"##,
    )
    .expect("a file that repaints the transcript");

    // The file names every colour the transcript has, and nothing else moves.
    let (moved, claimed) = moved_and_claimed(&theme, |name| name.starts_with("transcript-"));
    assert_eq!(moved, claimed);
}

/// The transcript's prose is the one thing a reader is here to read, so a theme
/// that paints it into its own field is told, by name, which pair it buried.
#[test]
fn a_theme_that_buries_the_transcripts_prose_is_reported() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": { "transcript-prose-ink": "#202020" } }"##,
    )
    .expect("a theme that loads");

    let findings = contrast(&theme);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].what, "the transcript's prose");
}

/// A file that names the sidebar's colours repaints the sidebar and moves
/// nothing else in the map.
#[test]
fn a_theme_that_names_the_sidebar_roles_moves_only_those() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": {
               "sidebar-surface": "#f5f5f5",
               "sidebar-row-fill": "#eeeeee",
               "sidebar-row-fill-hover": "#e4e4e4",
               "sidebar-row-fill-selected": "#dadada",
               "sidebar-name-ink": "#303030",
               "sidebar-name-ink-selected": "#101010",
               "sidebar-name-ink-editing": "#101010",
               "sidebar-age-ink": "#6a6a6a",
               "tag-working": "#8a6d00",
               "tag-parked": "#8a8a8a",
               "tag-idle": "#2f7d3a",
               "sidebar-fold-glyph": "#4a4a4a",
               "sidebar-fold-glyph-hover": "#101010",
               "sidebar-new-session-mark": "#4a4a4a",
               "sidebar-new-session-mark-hover": "#101010",
               "sidebar-strip-new-session-mark": "#303030",
               "sidebar-strip-new-session-mark-hover": "#101010",
               "sidebar-search-mark": "#4a4a4a",
               "sidebar-row-rename-mark": "#5a5a5a" } }"##,
    )
    .expect("a file that repaints the sidebar");

    // The file names every colour the sidebar has, and nothing else moves.
    let (moved, claimed) = moved_and_claimed(&theme, |name| {
        name.starts_with("sidebar-") || name.starts_with("tag-")
    });
    assert_eq!(moved, claimed);

    assert_eq!(theme.get(Role::SidebarSurface), Rgba::hex("#f5f5f5"));
    assert_eq!(theme.get(Role::TagIdle), Rgba::hex("#2f7d3a"));
}

/// The sidebar is made of the base values like every other part: a file that
/// moves the window's own colours carries the column and its rows with it,
/// which is what makes a light theme possible without naming every sidebar
/// role by hand.
#[test]
fn a_theme_that_moves_the_base_values_moves_the_sidebar_with_them() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020" } }"##,
    )
    .expect("a file that names only base values");

    assert_eq!(theme.get(Role::SidebarSurface), Rgba::hex("#f5f5f5"));
    assert_eq!(theme.get(Role::SidebarNameInkSelected), Rgba::hex("#202020"));
    // The row fills are a mix of ink and background, so on a light window a
    // hover steps darker than the field rather than lighter.
    let hover = theme.get(Role::SidebarRowFillHover);
    assert!(hover.r < 0xf5 && hover.r > 0x20, "{hover:?}");
}

/// The three tags are told apart by colour alone, so a theme that paints two of
/// them alike has taken a meaning away. The check names the pair.
#[test]
fn a_theme_that_paints_the_working_tag_like_the_idle_one_is_reported() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1, "roles": { "tag-idle": "#a18e41" } }"##,
    )
    .expect("a theme that loads");

    let clashes = separation(&theme);

    assert_eq!(clashes.len(), 1, "{clashes:?}");
    assert_eq!(clashes[0].what, "the working tag and the idle tag");
    assert!(clashes[0].apart < 0.5, "{clashes:?}");
}

/// Not only an exact copy: three greys a few steps apart, the way a monochrome
/// theme collapses the tags when nobody looks, are three clashes and not none.
#[test]
fn three_tags_that_barely_differ_are_three_findings() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": { "tag-working": "#9a9a9a", "tag-parked": "#8a8a8a", "tag-idle": "#7a7a7a" } }"##,
    )
    .expect("a theme that loads");

    let named: Vec<&str> = separation(&theme).iter().map(|clash| clash.what).collect();

    assert_eq!(named.len(), 3, "{named:?}");
    assert!(named.contains(&"the working tag and the parked tag"), "{named:?}");
    assert!(named.contains(&"the working tag and the idle tag"), "{named:?}");
    assert!(named.contains(&"the parked tag and the idle tag"), "{named:?}");
}

/// A theme that only moves base values: every role it never mentions still
/// comes out with a value, and the ones made of the base follow it.
#[test]
fn a_sparse_palette_resolves_every_role_it_does_not_mention() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020", "danger": "#7a1f18" } }"##,
    )
    .expect("a file that names only base values");

    // The base values moved what is made of them: the marks, the chips, the
    // close button's fill, and the background they all sit on.
    assert_eq!(theme.background(), Rgba::hex("#f5f5f5"));
    assert_eq!(theme.get(Role::TitleBarFoldGlyphHover), Rgba::hex("#202020"));
    assert_eq!(theme.get(Role::TitleBarCloseFillHover), Rgba::hex("#7a1f18"));
    let glyph = theme.get(Role::TitleBarGlyph).over(theme.background());
    assert!(glyph.r < 0x60, "the caption glyphs follow the ink: {glyph:?}");
}

/// The four panels that open over the window as the window drew them before
/// there was a theme: the values read off `ui/menus.slint`, the queue card in
/// `ui/app.slint` and the waiting rows in `ui/transcript.slint`, not out of this
/// crate.
#[test]
fn the_shipped_theme_draws_todays_menus() {
    let theme = Resolved::shipped();

    // The shape all four share: a field a step over the window's background, the
    // hairline that parts it from the sheet, and the shadow that lifts it.
    assert_eq!(theme.get(Role::PanelSurface), Rgba::hex("#181818"));
    assert_eq!(theme.get(Role::PanelEdge), Rgba::hex("#2e2e2e"));
    assert_eq!(theme.get(Role::PanelShadow), Rgba::hex("#00000080"));
    // The ladder hangs off the model menu rather than off the window, and its
    // shadow is the shallower one.
    assert_eq!(theme.get(Role::PanelShadowNested), Rgba::hex("#00000060"));
    assert_eq!(theme.get(Role::PanelRowFillHover), Rgba::hex("#202020"));
    assert_eq!(theme.get(Role::PanelRowFillCurrent), Rgba::hex("#242424"));

    // The command list: a name in the two states its row has, the argument only
    // the highlighted row shows, and the sentence that says what it does.
    assert_eq!(theme.get(Role::SlashMenuRowInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::SlashMenuRowInkHover), Rgba::hex("#c1c1c1"));
    assert_eq!(theme.get(Role::SlashMenuHintInk), Rgba::hex("#8f8f8f"));
    assert_eq!(theme.get(Role::SlashMenuDescriptionInk), Rgba::hex("#6d6d6d"));

    // The model menu: a provider signed in and not, the figure beside it, a
    // model in both states its row has, and the two things the right of a row
    // can carry.
    assert_eq!(theme.get(Role::ModelMenuHeaderInk), Rgba::hex("#d8d8d8"));
    assert_eq!(theme.get(Role::ModelMenuHeaderInkQuiet), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::ModelMenuCountInk), Rgba::hex("#585858"));
    assert_eq!(theme.get(Role::ModelMenuRowInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::ModelMenuRowInkCurrent), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::ModelMenuDetailInk), Rgba::hex("#585858"));
    // The level the running model carries is the gold the sidebar's working tag
    // is drawn in: they say the same thing.
    assert_eq!(theme.get(Role::ModelMenuEffortInk), Rgba::hex("#a18e41"));

    // The ladder's rungs.
    assert_eq!(theme.get(Role::EffortLadderRowInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::EffortLadderRowInkCurrent), Rgba::hex("#e5e5e5"));

    // The queue card: the count over the first lane, the second lane's own name,
    // a waiting message, and the action that empties both.
    assert_eq!(theme.get(Role::QueueCardCountInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::QueueCardLaneInk), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::QueueRowInk), Rgba::hex("#d8d8d8"));
    assert_eq!(theme.get(Role::QueueCardClearInk), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::QueueCardClearInkHover), Rgba::hex("#d8d8d8"));

    // A waiting row's three controls: a fill in the two states it has, and the
    // marks on it. The bin is the one mark that changes colour rather than
    // weight as the pointer arrives.
    assert_eq!(theme.get(Role::QueueRowControlFill), Rgba::hex("#2e2e2e"));
    assert_eq!(theme.get(Role::QueueRowControlFillHover), Rgba::hex("#3a3a3a"));
    assert_eq!(theme.get(Role::QueueRowControlMark), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::QueueRowControlMarkHover), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::QueueRowPromoteMark), Rgba::hex("#8f8f8f"));
    assert_eq!(theme.get(Role::QueueRowPromoteMarkHover), Rgba::hex("#cdcdcd"));
    assert_eq!(theme.get(Role::QueueRowDeleteMark), Rgba::hex("#f3ded8"));
}

/// The status strip as the window drew it before there was a theme: the values
/// read off `ui/status-strip.slint` and the `busy-h` it is drawn at in
/// `ui/app.slint`, not out of this crate.
#[test]
fn the_shipped_theme_draws_todays_status_strip() {
    let theme = Resolved::shipped();

    // The strip is an outline with a fill inset by exactly its stroke, which is
    // why the two are separate roles although the panels' edge ships this value.
    assert_eq!(theme.get(Role::StatusStripEdge), Rgba::hex("#2e2e2e"));
    assert_eq!(theme.get(Role::StatusStripSurface), Rgba::hex("#191919"));

    // What the turn is doing, and how long it has been at it.
    assert_eq!(theme.get(Role::StatusStripWorkingInk), Rgba::hex("#c9a86a"));
    assert_eq!(theme.get(Role::StatusStripElapsedInk), Rgba::hex("#585858"));

    // The pill that opens the queue: at rest it draws nothing of its own, and
    // the number and the word beside it brighten together.
    assert_eq!(theme.get(Role::StatusStripQueueFill).a, 0, "a resting pill is not a fill");
    assert_eq!(theme.get(Role::StatusStripQueueFillHover), Rgba::hex("#202020"));
    assert_eq!(theme.get(Role::StatusStripQueueCountInk), Rgba::hex("#b0b0b0"));
    assert_eq!(theme.get(Role::StatusStripQueueCountInkHover), Rgba::hex("#e5e5e5"));
    assert_eq!(theme.get(Role::StatusStripQueueLabelInk), Rgba::hex("#6d6d6d"));
    assert_eq!(theme.get(Role::StatusStripQueueLabelInkHover), Rgba::hex("#b0b0b0"));
}

/// The panels and the strip are made of the base values like every other part: a
/// file that moves the window's colours carries them with it, which is what
/// makes a light set of menus possible without naming every role by hand.
#[test]
fn a_theme_that_moves_the_base_values_moves_the_panels_and_the_strip_with_them() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020" } }"##,
    )
    .expect("a file that names only base values");

    let shipped = Resolved::shipped();
    for field in [
        Role::PanelSurface,
        Role::PanelEdge,
        Role::StatusStripSurface,
        Role::StatusStripEdge,
    ] {
        assert_ne!(theme.get(field), shipped.get(field), "{} follows the field", field.name());
        assert!(theme.get(field).r > 0x80, "{} is a field of a light window", field.name());
    }
    // The running model's name and the strip's word for the turn follow the ink;
    // the rows are a lift of it over the background, so on a light window they
    // step darker than the panel rather than lighter.
    assert_eq!(theme.get(Role::ModelMenuRowInkCurrent), Rgba::hex("#202020"));
    let hover = theme.get(Role::PanelRowFillHover);
    assert!(hover.r < 0xf5 && hover.r > 0x20, "{hover:?}");
}

/// A file that names the panels' colours repaints the four panels and moves
/// nothing else in the map — not the strip, which is a surface of its own.
#[test]
fn a_theme_that_names_the_panel_roles_moves_only_those() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": {
               "panel-surface": "#1a1226",
               "panel-edge": "#3a2a52",
               "panel-shadow": "#00000099",
               "panel-shadow-nested": "#0000004d",
               "panel-row-fill-hover": "#2f2242",
               "panel-row-fill-current": "#3a2c50",
               "slash-menu-row-ink": "#cfc0e8",
               "slash-menu-row-ink-hover": "#eee6ff",
               "slash-menu-hint-ink": "#c9a7ff",
               "slash-menu-description-ink": "#9a8fb0",
               "model-menu-header-ink": "#eee6ff",
               "model-menu-header-ink-quiet": "#8a7fa0",
               "model-menu-caret-mark": "#a99bc4",
               "model-menu-count-ink": "#9a8fb0",
               "model-menu-row-ink": "#cfc0e8",
               "model-menu-row-ink-current": "#ffffff",
               "model-menu-check-mark": "#c9a7ff",
               "model-menu-detail-ink": "#9a8fb0",
               "model-menu-effort-ink": "#c9a7ff",
               "effort-ladder-row-ink": "#cfc0e8",
               "effort-ladder-row-ink-current": "#ffffff",
               "effort-ladder-check-mark": "#c9a7ff",
               "queue-card-count-ink": "#cfc0e8",
               "queue-card-lane-ink": "#9a8fb0",
               "queue-row-ink": "#eee6ff",
               "queue-card-clear-ink": "#9a8fb0",
               "queue-card-clear-ink-hover": "#eee6ff",
               "queue-row-control-fill": "#3a2a52",
               "queue-row-control-fill-hover": "#5f4d7d",
               "queue-row-control-mark": "#cfc0e8",
               "queue-row-control-mark-hover": "#ffffff",
               "queue-row-promote-mark": "#c9a7ff",
               "queue-row-promote-mark-hover": "#eee6ff",
               "queue-row-delete-mark": "#ff9f8f" } }"##,
    )
    .expect("a file that repaints the panels");

    // The file names every colour the four panels have, and nothing else moves.
    let (moved, claimed) = moved_and_claimed(&theme, |name| {
        name.starts_with("panel-")
            || name.starts_with("slash-menu-")
            || name.starts_with("model-menu-")
            || name.starts_with("effort-ladder-")
            || name.starts_with("queue-card-")
            || name.starts_with("queue-row-")
    });
    assert_eq!(moved, claimed);
}

/// The strip is its own surface, so a file that names it repaints the strip and
/// leaves the four panels — and every other part — where they were.
#[test]
fn a_theme_that_names_the_status_strip_roles_moves_only_those() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": {
               "status-strip-surface": "#1a1226",
               "status-strip-edge": "#3a2a52",
               "status-strip-working-ink": "#c9a7ff",
               "status-strip-elapsed-ink": "#9a8fb0",
               "status-strip-queue-fill": "#2f2242",
               "status-strip-queue-fill-hover": "#3a2c50",
               "status-strip-queue-count-ink": "#cfc0e8",
               "status-strip-queue-count-ink-hover": "#ffffff",
               "status-strip-queue-label-ink": "#9a8fb0",
               "status-strip-queue-label-ink-hover": "#cfc0e8" } }"##,
    )
    .expect("a file that repaints the strip");

    // The file names every colour the strip has, and nothing else moves.
    let (moved, claimed) = moved_and_claimed(&theme, |name| name.starts_with("status-strip-"));
    assert_eq!(moved, claimed);

    assert_eq!(theme.get(Role::StatusStripSurface), Rgba::hex("#1a1226"));
    // The panels are not the strip: the same file leaves their surface alone.
    assert_eq!(theme.get(Role::PanelSurface), Resolved::shipped().get(Role::PanelSurface));
}

/// The ticket's second promise, in the unit that can keep it: the strip's tags —
/// the word that says what the turn is doing, and the count and the word the
/// queue pill carries — are read on the strip, so a theme that paints any of
/// them into it is told which one it buried.
#[test]
fn a_theme_that_paints_the_strips_tags_into_the_strip_is_reported() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "roles": { "status-strip-working-ink": "#191919",
                        "status-strip-queue-count-ink": "#191919",
                        "status-strip-queue-label-ink": "#191919" } }"##,
    )
    .expect("a theme that loads");

    let mut named: Vec<&str> = contrast(&theme).iter().map(|finding| finding.what).collect();
    named.sort_unstable();

    assert_eq!(
        named,
        [
            "how many messages the pill counts",
            "how many messages the pill counts, with its card open",
            "the word beside the count on the pill",
            "the word beside the count, with the pill's card open",
            "the word that says what the turn is doing"
        ]
    );
}

/// A waiting row's controls draw no outline: the fill is the whole of what says
/// they are controls. A theme that tints the panel — or the row's own hover —
/// and leaves the fills behind is told, in both rows it can happen on.
#[test]
fn a_theme_that_paints_a_waiting_rows_buttons_like_the_row_is_reported() {
    let painted = |fill: &str| {
        let theme = from_text(
            "mine.json",
            &format!(
                r##"{{ "name": "mine", "version": 1,
                       "roles": {{ "queue-row-control-fill": "{fill}" }} }}"##
            ),
        )
        .expect("a theme that loads");
        separation(&theme).into_iter().map(|clash| clash.what).collect::<Vec<&str>>()
    };

    // The panel's own field.
    assert_eq!(painted("#181818"), ["a waiting row's buttons and the panel behind them"]);
    // The fill a row takes when the pointer is on it.
    assert_eq!(painted("#202020"), ["a waiting row's buttons and the row they sit on"]);
}

/// A base value carries a theme far, and the places it does not carry are the
/// ones the check names: on a light background the fold mark's veil of quiet
/// ink steps *towards* the field rather than away from it, and the session
/// names — made of quiet ink, which this file never moved — go with it. The
/// transcript's secondary ink is made of the same quiet ink faded towards the
/// field, so it stays pale on a pale column while its prose, made of the ink,
/// follows the file all the way. The composer is the same story twice over: the
/// chip's name, the plus and the line under the draft are quiet ink faded
/// towards a field that has just gone light, and the gold of a command word and
/// the white of a selection are fixed values no palette entry moves. The four
/// panels and the strip tell it again — every word made of the quiet ink stays
/// pale on fields that have gone light, and the gold of the running model's
/// level and of the strip's spinner stays gold. The glyphs tell it a third time:
/// the column's fold and its magnifier are quiet ink and the carets are made of
/// the ink, so they stay legible; and the brand mark, whose blade and wing are
/// the brand tone this file never moved, washes into the column it is cut from —
/// which is the distance check's finding, not a ratio's, but the blade and the
/// wing are still read on the column and still named here. Those are roles to
/// override, not a file to refuse, and the author is told which.
#[test]
fn a_light_palette_still_resolves_and_the_check_names_what_it_left_dim() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020", "danger": "#7a1f18" } }"##,
    )
    .expect("a file that names only base values");

    let mut named: Vec<&str> = contrast(&theme).iter().map(|finding| finding.what).collect();
    named.sort_unstable();

    assert_eq!(
        named,
        [
            "a cell's name at rest",
            "a cell's name under the pointer",
            "a command's name",
            "a ledger value",
            "a link in prose",
            "a model's name at rest",
            "a provider the reader is not signed in to",
            "a rung of the ladder",
            "a session's name at rest",
            "a session's name under the pointer",
            "a table's text",
            "how many messages are waiting",
            "how many messages the pill counts",
            "how many messages the pill counts, with its card open",
            "the action that empties both lanes",
            "the argument a command takes",
            "the arrow that pulls a message into the running turn",
            "the bar that says how full the context is",
            "the bin on a waiting row, under the pointer",
            "the brand mark's blade",
            "the brand mark's wing",
            "the card's small labels",
            "the caret in front of a provider's name",
            "the caret on the composer's chip",
            "the command a draft starts with",
            "the composer's status line",
            "the dot that parts a cell's meta from its elapsed time",
            "the fold mark at rest",
            "the label a thinking row carries",
            "the level the running model is at",
            "the model the chip names",
            "the name of a lane with nothing to count",
            "the pencil and the bin on a waiting row",
            "the plus that attaches a picture",
            "the plus that attaches a picture, under the pointer",
            "the send disc on the composer's field",
            "the sidebar's fold mark at rest",
            "the sidebar's magnifier",
            "the status line when something went wrong",
            "the text inside an opened thinking row",
            "the tick beside the rung the model is on",
            "the tick beside the running model",
            "the word beside the count on the pill",
            "the word beside the count, under the pointer",
            "the word beside the count, with the pill's card open",
            "the word that says what the turn is doing",
            "the words the reader has selected",
            "what a cell printed",
            "what a command does",
            "what is left of the context"
        ]
    );
}

/// Touching one role by name: the palette would have given the close button the
/// theme's danger, and the file says otherwise for that button alone.
#[test]
fn a_role_override_beats_the_value_the_palette_derives() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "danger": "#20c040" },
             "roles": { "title-bar-close-fill-hover": "#123456" } }"##,
    )
    .expect("a file that overrides one role");

    assert_eq!(theme.get(Role::TitleBarCloseFillHover), Rgba::hex("#123456"));
}

#[test]
fn an_unknown_role_is_refused_with_the_file_the_name_used_and_the_nearest_legal_one() {
    let refusal = from_text(
        "themes/mine.json",
        r##"{ "name": "mine", "version": 1, "roles": { "title-bar-gylph": "#ffffff" } }"##,
    )
    .expect_err("a role this window does not have");
    let line = refusal.to_string();

    assert!(line.contains("themes/mine.json"), "{line}");
    assert!(line.contains("title-bar-gylph"), "{line}");
    assert!(line.contains("title-bar-glyph"), "{line}");
}

#[test]
fn an_unknown_base_value_is_refused_with_the_nearest_name_in_the_palette() {
    let refusal = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1, "palette": { "backgroud": "#000000" } }"##,
    )
    .expect_err("a palette entry this window does not have");
    let line = refusal.to_string();

    assert!(line.contains("backgroud"), "{line}");
    assert!(line.contains("background"), "{line}");
}

#[test]
fn a_colour_that_is_not_hex_is_refused_with_the_name_it_sits_beside() {
    let refusal = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1, "palette": { "ink": "#12" } }"##,
    )
    .expect_err("a colour that is not a colour");

    let line = refusal.to_string();
    assert!(line.contains("palette.ink"), "{line}");
    assert!(line.contains("#12"), "{line}");
}

#[test]
fn a_file_from_another_schema_version_is_refused_rather_than_half_read() {
    let refusal = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 2, "palette": { "ink": "#ffffff" } }"##,
    )
    .expect_err("a version this window does not read");

    assert!(refusal.to_string().contains("version 2"), "{refusal}");
}

/// A file that names nothing of itself is not a theme this window can tell from
/// any other, and the log line is the only place a reader learns of it.
#[test]
fn a_file_that_does_not_name_itself_is_refused() {
    let refusal = from_text("mine.json", r##"{ "version": 1 }"##)
        .expect_err("a theme with no name");

    assert!(refusal.to_string().contains("name"), "{refusal}");
}

/// Windows writes a byte-order mark in front of a UTF-8 file as a matter of
/// course: a theme saved by Notepad or by PowerShell is still a theme.
#[test]
fn a_file_written_with_a_byte_order_mark_is_still_read() {
    let theme = from_text(
        "mine.json",
        "\u{feff}{ \"name\": \"mine\", \"version\": 1, \"roles\": { \"title-bar-glyph\": \"#ffffff\" } }",
    )
    .expect("a file with a mark in front of it");

    assert_eq!(theme.get(Role::TitleBarGlyph), Rgba::hex("#ffffff"));
}

#[test]
fn a_key_at_the_top_of_the_file_this_window_does_not_know_is_refused() {
    let refusal = from_text("mine.json", r##"{ "name": "mine", "version": 1, "colours": {} }"##)
        .expect_err("a top-level key with no meaning");

    let line = refusal.to_string();
    assert!(line.contains("colours"), "{line}");
}

/// The selection the window makes: a path it cannot read leaves the window on
/// the theme it ships with, and the refusal is what goes in the log.
#[test]
fn a_selection_that_cannot_be_read_leaves_the_shipped_theme_in_place() {
    let missing = format!("{}/no-such-theme-{}.json", std::env::temp_dir().display(), std::process::id());
    let (theme, refusal) = select(Some(&missing));

    assert_eq!(theme, Resolved::shipped());
    let refusal = refusal.expect("a selection that could not be read");
    assert!(refusal.to_string().contains("no-such-theme"), "{refusal}");
}

#[test]
fn no_selection_is_the_shipped_theme_and_nothing_to_say() {
    let (theme, refusal) = select(None);

    assert_eq!(theme, Resolved::shipped());
    assert!(refusal.is_none());
}

/// The check a theme author runs, run here on the theme the window ships with.
#[test]
fn the_shipped_theme_clears_every_floor_it_declares() {
    let findings = contrast(&Resolved::shipped());

    assert!(findings.is_empty(), "{findings:?}");
}

/// The other half of both tickets that could not move a pixel: a reader tells
/// the three tags apart and a failed cell from a quiet one in the theme that
/// ships, and in every theme the suite is given.
#[test]
fn the_shipped_themes_marks_stay_apart() {
    let clashes = separation(&Resolved::shipped());

    assert!(clashes.is_empty(), "{clashes:?}");
}

/// A theme may be ugly and still load. What it cannot do is load quietly: the
/// pair is named, so the author knows which part of the window to look at.
#[test]
fn a_theme_that_buries_the_caption_glyphs_is_reported_while_it_still_resolves() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1, "roles": { "title-bar-glyph": "#1e1e1e" } }"##,
    )
    .expect("a theme that loads");

    let findings = contrast(&theme);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].what, "the caption glyphs");
    // #1e1e1e on #111111 is 1.13:1 by the WCAG definition, worked out here
    // rather than by the code under test.
    assert!((findings[0].ratio - 1.13).abs() < 0.005, "{findings:?}");
}

/// The report the command line prints, which is also the report the check above
/// produces: one function, so a script and the suite cannot disagree.
#[test]
fn the_command_line_reports_a_file_and_says_nothing_about_a_window() {
    let path = std::env::temp_dir().join(format!("pa-ui-theme-check-{}.json", std::process::id()));
    std::fs::write(
        &path,
        r##"{ "name": "mine", "version": 1, "roles": { "title-bar-glyph": "#1e1e1e" } }"##,
    )
    .expect("a file to check");

    let (code, report) =
        check_cli(["--check-theme".to_string(), path.display().to_string()].into_iter())
            .expect("a check, not a window");

    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 1, "{report}");
    assert!(report.contains("the caption glyphs"), "{report}");
}

/// A file this window cannot read at all is a different answer from a file that
/// reads badly, because only the first one is a refusal.
#[test]
fn the_command_line_tells_a_refused_file_apart_from_a_hard_to_read_one() {
    let path = std::env::temp_dir().join(format!("pa-ui-theme-typo-{}.json", std::process::id()));
    std::fs::write(
        &path,
        r##"{ "name": "mine", "version": 1, "roles": { "title-bar-gylph": "#ffffff" } }"##,
    )
    .expect("a file to check");

    let (code, report) =
        check_cli(["--check-theme".to_string(), path.display().to_string()].into_iter())
            .expect("a check, not a window");

    let _ = std::fs::remove_file(&path);
    assert_eq!(code, 2, "{report}");
    assert!(report.contains("title-bar-glyph"), "{report}");
}

#[test]
fn the_command_line_wants_a_file_to_check() {
    let (code, report) = check_cli(["--check-theme".to_string()].into_iter()).expect("a check");

    assert_eq!(code, 2, "{report}");
    assert!(report.contains("--check-theme"), "{report}");
}

/// Anything that is not a check is the window's business.
#[test]
fn a_command_line_with_no_check_in_it_opens_the_window() {
    assert!(check_cli(std::iter::empty()).is_none());
}

/// The one seam code generation cannot close: the global's properties and the
/// role table are written twice, so a test holds them to the same set. A role
/// added to one and not the other fails here rather than in the window.
#[test]
fn the_theme_global_declares_exactly_the_roles_the_table_holds() {
    let mut declared: Vec<&str> = include_str!("../../ui/theme.slint")
        .lines()
        .filter_map(|line| line.trim().strip_prefix("in-out property <color> "))
        .map(|name| name.trim_end_matches(';'))
        .collect();
    let mut table: Vec<&str> = Role::ALL.iter().map(|role| role.name()).collect();
    declared.sort_unstable();
    table.sort_unstable();

    assert_eq!(declared, table);
}
