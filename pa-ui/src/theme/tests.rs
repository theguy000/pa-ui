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
               "transcript-jump-edge": "#4a3a63" } }"##,
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
               "tag-idle": "#2f7d3a" } }"##,
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

/// A base value carries a theme far, and the places it does not carry are the
/// ones the check names: on a light background the fold mark's veil of quiet
/// ink steps *towards* the field rather than away from it, and the session
/// names — made of quiet ink, which this file never moved — go with it. The
/// transcript's secondary ink is made of the same quiet ink faded towards the
/// field, so it stays pale on a pale column while its prose, made of the ink,
/// follows the file all the way. Those are roles to override, not a file to
/// refuse, and the author is told which.
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
            "a link in prose",
            "a session's name at rest",
            "a session's name under the pointer",
            "a table's text",
            "the dot that parts a cell's meta from its elapsed time",
            "the fold mark at rest",
            "the label a thinking row carries",
            "the text inside an opened thinking row",
            "what a cell printed"
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
