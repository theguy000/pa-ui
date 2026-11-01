//! Where a theme leaves the window hard to read.
//!
//! The role table says which pairs are ink on a surface; this measures each
//! pair and names the ones that fall below their floor. It never refuses a
//! theme — a theme that loads but reads badly is still the theme its author
//! asked for — and it is the same check whether a script asks for it or the
//! suite runs it over the theme that ships.

use super::file::load;
use super::{Resolved, Rgba, Role};

/// A mark or a line that carries meaning clears 3:1. Body text clears 4.5:1,
/// and a decorative hairline is not a pair at all, because nobody reads it.
/// Nothing in the caption is body text, so every pair below is a mark; the
/// floor a pair carries grows into a field the day one of them is body text.
const FLOOR: f32 = 3.0;

/// One place the window puts ink on a surface.
struct Pair {
    ink: Role,
    /// The surface it sits on. `None` is the window's own background, which is
    /// what the caption's marks sit on: the caption draws no field of its own.
    on: Option<Role>,
    /// Named the way a theme author would say it, since it is what they see
    /// when the pair fails.
    what: &'static str,
}

const PAIRS: &[Pair] = &[
    Pair { ink: Role::TitleBarGlyph, on: None, what: "the caption glyphs" },
    Pair { ink: Role::TitleBarFoldGlyph, on: None, what: "the fold mark at rest" },
    Pair {
        ink: Role::TitleBarFoldGlyphHover,
        on: Some(Role::TitleBarFoldFillHover),
        what: "the fold mark under the pointer",
    },
    Pair {
        ink: Role::TitleBarCloseInkHover,
        on: Some(Role::TitleBarCloseFillHover),
        what: "the close mark on its hover fill",
    },
];

/// One pair a theme left below its floor.
#[derive(Debug)]
pub(crate) struct Finding {
    pub(crate) what: &'static str,
    pub(crate) ratio: f32,
}

/// Every pair this theme leaves under its floor. Empty is the answer a theme
/// author wants, and the only answer the theme that ships may give.
pub(crate) fn contrast(theme: &Resolved) -> Vec<Finding> {
    PAIRS
        .iter()
        .filter_map(|pair| {
            let on = pair.on.map_or_else(|| theme.background(), |role| theme.get(role));
            let ratio = ratio(theme.get(pair.ink), on, theme.background());
            (ratio < FLOOR).then_some(Finding { what: pair.what, ratio })
        })
        .collect()
}

/// The WCAG contrast ratio of ink on the surface behind it.
///
/// A surface that draws nothing of its own — the caption, which lets the
/// sidebar's header run through it — is read on the window's background, and so
/// is an ink that carries its own alpha. That is what the reader sees.
fn ratio(ink: Rgba, on: Rgba, background: Rgba) -> f32 {
    let luminance = |colour: Rgba| {
        let channel = |value: u8| {
            let value = f32::from(value) / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(colour.r) + 0.7152 * channel(colour.g) + 0.0722 * channel(colour.b)
    };
    let on = on.over(background);
    let ink = luminance(ink.over(on));
    let on = luminance(on);
    (ink.max(on) + 0.05) / (ink.min(on) + 0.05)
}

const FLAG: &str = "--check-theme";

/// `pa-ui --check-theme <file>`: the report for one theme file, and the code a
/// script reads, without a window ever being opened.
///
/// `None` when the command line is not a check, which is when the window runs.
/// Otherwise the whole answer — text and code — so the only untested part is
/// the printing of it.
///
/// The codes tell the two failures apart on purpose: a file this window cannot
/// read is not the same news as a file that reads badly, and only the first one
/// is a refusal.
pub(crate) fn check_cli(mut args: impl Iterator<Item = String>) -> Option<(i32, String)> {
    if args.next().as_deref() != Some(FLAG) {
        return None;
    }
    let usage = format!("usage: pa-ui {FLAG} <theme file>\n");
    let Some(path) = args.next() else {
        return Some((2, usage));
    };
    if args.next().is_some() {
        return Some((2, usage));
    }

    let theme = match load(&path) {
        Ok(theme) => theme,
        Err(refusal) => return Some((2, format!("{refusal}\n"))),
    };

    let findings = contrast(&theme);
    if findings.is_empty() {
        let pairs = PAIRS.len();
        return Some((0, format!("theme `{path}`: {pairs} ink-on-surface pairs clear their floor\n")));
    }
    let subject = match findings.len() {
        1 => "1 ink-on-surface pair is".to_string(),
        count => format!("{count} ink-on-surface pairs are"),
    };
    let mut report = format!("theme `{path}`: {subject} under its floor\n");
    for finding in &findings {
        report.push_str(&format!("  {}: {:.1}:1\n", finding.what, finding.ratio));
    }
    Some((1, report))
}
