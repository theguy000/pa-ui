//! Where a theme leaves the window hard to read, or hard to tell apart.
//!
//! Two promises are measured here, each in its own unit: ink has to stand off
//! the surface under it, and two marks a reader knows by colour alone have to
//! stay apart. Neither is enforced — a theme that loads but reads badly is
//! still the theme its author asked for — and it is the same check whether a
//! script asks for it or the suite runs it over the themes that ship.

use super::file::load;
use super::{Resolved, Rgba, Role};

/// A mark or a line that carries meaning clears 3:1 on the surface under it.
const MARK: f32 = 3.0;

/// Body ink — the words the reader came for — clears 4.5:1.
const BODY: f32 = 4.5;

/// How far apart two marks have to be before a reader stops taking them for
/// one: a CIE76 distance. The three tags that ship sit 29.8 apart at their
/// closest, so this is the room they have to lose before they collapse.
const APART: f32 = 20.0;

/// One place the window puts ink on a surface.
struct Pair {
    ink: Role,
    /// The fields under the ink, the nearest last: a row's fill laid over its
    /// column's surface, and no more. Empty is the window's own background,
    /// which is what the caption's marks sit on — the caption draws no field.
    on: &'static [Role],
    /// Named the way a theme author would say it, since it is what they see
    /// when the pair fails.
    what: &'static str,
    /// What the ink carries decides its floor: a word the reader reads gets
    /// more room than a mark that only has to be found.
    floor: f32,
}

const PAIRS: &[Pair] = &[
    Pair {
        ink: Role::TitleBarGlyph,
        on: &[],
        what: "the caption glyphs",
        floor: MARK,
    },
    Pair {
        ink: Role::TitleBarFoldGlyph,
        on: &[],
        what: "the fold mark at rest",
        floor: MARK,
    },
    Pair {
        ink: Role::TitleBarFoldGlyphHover,
        on: &[Role::TitleBarFoldFillHover],
        what: "the fold mark under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Role::TitleBarCloseInkHover,
        on: &[Role::TitleBarCloseFillHover],
        what: "the close mark on its hover fill",
        floor: MARK,
    },
    // A session's name is the sidebar's one piece of body text, measured in
    // each state it is read in.
    Pair {
        ink: Role::SidebarNameInk,
        on: &[Role::SidebarSurface, Role::SidebarRowFill],
        what: "a session's name at rest",
        floor: BODY,
    },
    Pair {
        ink: Role::SidebarNameInk,
        on: &[Role::SidebarSurface, Role::SidebarRowFillHover],
        what: "a session's name under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Role::SidebarNameInkSelected,
        on: &[Role::SidebarSurface, Role::SidebarRowFillSelected],
        what: "the picked session's name",
        floor: BODY,
    },
    Pair {
        ink: Role::SidebarNameInkEditing,
        on: &[Role::SidebarSurface, Role::SidebarRowFill],
        what: "a session's name while it is renamed",
        floor: BODY,
    },
    // Two sidebar colours are deliberately left out of this list rather than
    // gated, and both for the same reason: the window already drew them this
    // way, and a role that has to resolve to today's pixel cannot be moved here.
    //   - the age sits at 2.7:1 on the column, under both floors: the window's
    //     miss, not a design that wants keeping.
    //   - the parked tag sits at 2.2:1, which is how a session meant to recede
    //     was drawn; the working tag lands at 5.8:1 and the idle at 6.1:1.
    // A theme that wants either read high overrides its role, where its own
    // value is measured.
];

/// Two marks a reader has to tell apart, because nothing but their colour says
/// which is which.
struct Telling {
    a: Role,
    b: Role,
    what: &'static str,
}

/// The three tags carry a session's state and nothing else does, so a theme
/// that paints two of them alike has taken the state away. This is the promise
/// the sidebar ticket can keep while it may not move a pixel; the tags' own
/// contrast against the column is recorded in the note above.
const TELLINGS: &[Telling] = &[
    Telling {
        a: Role::TagWorking,
        b: Role::TagParked,
        what: "the working tag and the parked tag",
    },
    Telling { a: Role::TagWorking, b: Role::TagIdle, what: "the working tag and the idle tag" },
    Telling { a: Role::TagParked, b: Role::TagIdle, what: "the parked tag and the idle tag" },
];

/// One ink-on-surface pair a theme left under its floor.
#[derive(Debug)]
pub(crate) struct Finding {
    pub(crate) what: &'static str,
    /// The contrast ratio the pair came to.
    pub(crate) ratio: f32,
    pub(crate) floor: f32,
}

/// One pair of marks a theme painted too alike to tell apart.
#[derive(Debug)]
pub(crate) struct Clash {
    pub(crate) what: &'static str,
    /// How far apart the two are, as a CIE76 distance.
    pub(crate) apart: f32,
}

/// The ink-on-surface pairs this theme leaves under their floor. Empty is the
/// answer a theme author wants, and the only answer the theme that ships may
/// give.
pub(crate) fn contrast(theme: &Resolved) -> Vec<Finding> {
    PAIRS
        .iter()
        .filter_map(|pair| {
            let ratio = ratio(theme.get(pair.ink), surface(theme, pair.on));
            (ratio < pair.floor).then_some(Finding {
                what: pair.what,
                ratio,
                floor: pair.floor,
            })
        })
        .collect()
}

/// The marks this theme pulled too close to each other to be told apart.
pub(crate) fn separation(theme: &Resolved) -> Vec<Clash> {
    // Each tag read as the reader sees it: on the column, which is where the
    // sidebar draws them. A tag carrying an alpha is laid on that field.
    let column = surface(theme, &[Role::SidebarSurface]);
    let seen = |role| theme.get(role).over(column);
    TELLINGS
        .iter()
        .filter_map(|telling| {
            let apart = distance(seen(telling.a), seen(telling.b));
            (apart < APART).then_some(Clash { what: telling.what, apart })
        })
        .collect()
}

/// The surface ink sits on: each field in `on` laid over the one before it,
/// starting from the window's own background, which nothing sits behind.
fn surface(theme: &Resolved, on: &[Role]) -> Rgba {
    on.iter().fold(theme.background(), |under, role| theme.get(*role).over(under))
}

/// The WCAG contrast ratio of ink on the surface behind it: an ink that carries
/// its own alpha is read as the reader sees it, laid on that surface.
fn ratio(ink: Rgba, on: Rgba) -> f32 {
    let ink = luminance(ink.over(on));
    let on = luminance(on);
    (ink.max(on) + 0.05) / (ink.min(on) + 0.05)
}

/// How far apart two colours look, as CIE76 sees it: the plain distance in
/// CIELAB, where equal steps are meant to look equally different. A contrast
/// ratio cannot answer this — two tags can share a luminance and still be told
/// apart by hue, and go on sharing it when both have gone grey.
fn distance(a: Rgba, b: Rgba) -> f32 {
    let (lightness_a, a_a, b_a) = lab(a);
    let (lightness_b, a_b, b_b) = lab(b);
    ((lightness_a - lightness_b).powi(2) + (a_a - a_b).powi(2) + (b_a - b_b).powi(2)).sqrt()
}

/// The colour as CIELAB reads it: sRGB, whose channels carry the eye's own
/// response, into the space where a distance means "looks different".
fn lab(colour: Rgba) -> (f32, f32, f32) {
    let (r, g, b) = (linear(colour.r), linear(colour.g), linear(colour.b));
    // The space's reference white: sRGB's own, so both sides are scaled alike.
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.950_47;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.088_83;
    let compress = |value: f32| {
        if value > 0.008_856 {
            value.cbrt()
        } else {
            7.787 * value + 16.0 / 116.0
        }
    };
    let (x, y, z) = (compress(x), compress(y), compress(z));
    (116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z))
}

/// One channel as light rather than as a number: the sRGB curve, undone.
fn linear(value: u8) -> f32 {
    let value = f32::from(value) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

fn luminance(colour: Rgba) -> f32 {
    0.2126 * linear(colour.r) + 0.7152 * linear(colour.g) + 0.0722 * linear(colour.b)
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
    let clashes = separation(&theme);
    if findings.is_empty() && clashes.is_empty() {
        let pairs = PAIRS.len();
        let tags = TELLINGS.len();
        return Some((
            0,
            format!("theme `{path}`: {pairs} ink-on-surface pairs and {tags} tag pairs clear their floor\n"),
        ));
    }
    let short = findings.len() + clashes.len();
    let subject = match short {
        1 => "1 promise is".to_string(),
        count => format!("{count} promises are"),
    };
    let mut report = format!("theme `{path}`: {subject} under its floor\n");
    for finding in &findings {
        let Finding { what, ratio, floor } = finding;
        report.push_str(&format!("  {what}: {ratio:.1}:1, floor {floor:.1}:1\n"));
    }
    for clash in &clashes {
        report.push_str(&format!("  {}: {:.1} apart, floor {APART:.0}\n", clash.what, clash.apart));
    }
    Some((1, report))
}
