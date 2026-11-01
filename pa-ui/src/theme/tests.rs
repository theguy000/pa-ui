//! What the theme must promise, asserted through its own interface: the map a
//! file resolves to, the message a bad file gets, and the pairs a theme leaves
//! unreadable.
//!
//! Resolution is pure — text in, a complete map out — so every case but the
//! command line's runs with no window, no file on disk and no UI source.

use super::*;

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
/// ink steps *towards* the field rather than away from it, and disappears. That
/// is a role to override, not a file to refuse, and the author is told which.
#[test]
fn a_light_palette_still_resolves_and_the_check_names_what_it_left_dim() {
    let theme = from_text(
        "mine.json",
        r##"{ "name": "mine", "version": 1,
             "palette": { "background": "#f5f5f5", "ink": "#202020", "danger": "#7a1f18" } }"##,
    )
    .expect("a file that names only base values");

    let findings = contrast(&theme);

    assert_eq!(findings.len(), 1, "{findings:?}");
    assert_eq!(findings[0].what, "the fold mark at rest");
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
