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

/// How far apart two fields have to be before a row still reads as the one that
/// failed. A field is not a mark: the tint that tells a failed cell from a quiet
/// one is meant to be seen before the name in it is read, and the window drew it
/// 5 apart. Anything less and the row has become a quiet cell with a red line.
const APART_TINT: f32 = 3.0;

// The fields each part is read on, nearest last, named once because most of them
// are read on again and again.
const ON_WINDOW: &[Role] = &[];
const ON_FOLD_HOVER: &[Role] = &[Role::TitleBarFoldFillHover];
const ON_CLOSE_HOVER: &[Role] = &[Role::TitleBarCloseFillHover];
const ON_SIDEBAR: &[Role] = &[Role::SidebarSurface];
const ON_SIDEBAR_ROW: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFill];
const ON_SIDEBAR_ROW_HOVER: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFillHover];
const ON_SIDEBAR_ROW_SELECTED: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFillSelected];
const ON_TRANSCRIPT: &[Role] = &[Role::TranscriptSurface];
const ON_MESSAGE: &[Role] = &[Role::TranscriptSurface, Role::TranscriptMessageFill];
const ON_CELL: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFill];
const ON_CELL_HOVER: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFillHover];
const ON_FAILED_CELL: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFillFailed];
const ON_FAILED_CELL_HOVER: &[Role] =
    &[Role::TranscriptSurface, Role::TranscriptCellFillFailedHover];
const ON_OPENED_ROW: &[Role] = &[Role::TranscriptSurface, Role::TranscriptRowFillExpanded];
const ON_CODE_CARD: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCodeFill];
// The composer's own fields: the box the reader writes in, the chip that names
// the model, and the card the context ring opens.
const ON_COMPOSER: &[Role] = &[Role::ComposerSurface];
const ON_SELECTION: &[Role] = &[Role::ComposerSurface, Role::ComposerDraftSelectionFill];
const ON_ATTACHMENT_REMOVE: &[Role] = &[
    Role::ComposerSurface,
    Role::ComposerAttachmentFill,
    Role::ComposerAttachmentRemoveFill,
];
const ON_ATTACH: &[Role] = &[Role::ComposerSurface, Role::ComposerAttachFill];
const ON_ATTACH_HOVER: &[Role] = &[Role::ComposerSurface, Role::ComposerAttachFillHover];
const ON_CARD: &[Role] = &[Role::ComposerCardSurface];
const ON_BAR_TRACK: &[Role] = &[Role::ComposerCardSurface, Role::ComposerCardBarTrack];
const ON_CHIP: &[Role] = &[Role::ComposerSurface, Role::ComposerChipFill];
const ON_CHIP_HOVER: &[Role] = &[Role::ComposerSurface, Role::ComposerChipFillHover];
const ON_SEND_FILL: &[Role] = &[Role::ComposerSurface, Role::ComposerSendFill];
const ON_STOP_FILL: &[Role] = &[Role::ComposerSurface, Role::ComposerStopFill];

/// One colour as the reader sees it: a role, and the fields it is drawn on.
struct Mark {
    role: Role,
    /// The fields under it, nearest last: a row's fill laid over its column's
    /// surface, and no more. `ON_WINDOW` is empty because there is nothing under
    /// the window: the caption draws no field and its marks sit on that.
    on: &'static [Role],
}

/// One place the window puts ink on a surface.
struct Pair {
    ink: Mark,
    /// Named the way a theme author would say it, since it is what they see
    /// when the pair fails.
    what: &'static str,
    /// What the ink carries decides its floor: a word the reader reads gets
    /// more room than a mark that only has to be found.
    floor: f32,
}

const PAIRS: &[Pair] = &[
    Pair {
        ink: Mark { role: Role::TitleBarGlyph, on: ON_WINDOW },
        what: "the caption glyphs",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::TitleBarFoldGlyph, on: ON_WINDOW },
        what: "the fold mark at rest",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::TitleBarFoldGlyphHover, on: ON_FOLD_HOVER },
        what: "the fold mark under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::TitleBarCloseInkHover, on: ON_CLOSE_HOVER },
        what: "the close mark on its hover fill",
        floor: MARK,
    },
    // A session's name is the sidebar's one piece of body text, measured in
    // each state it is read in.
    Pair {
        ink: Mark { role: Role::SidebarNameInk, on: ON_SIDEBAR_ROW },
        what: "a session's name at rest",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SidebarNameInk, on: ON_SIDEBAR_ROW_HOVER },
        what: "a session's name under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SidebarNameInkSelected, on: ON_SIDEBAR_ROW_SELECTED },
        what: "the picked session's name",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SidebarNameInkEditing, on: ON_SIDEBAR_ROW },
        what: "a session's name while it is renamed",
        floor: BODY,
    },
    // The transcript. Its rows are what the reader came for, so the words in
    // them are held to the body floor on whichever field they are read on — a
    // cell's name on the cell, the code it opened and what it printed inside it.
    Pair {
        ink: Mark { role: Role::TranscriptProseInk, on: ON_TRANSCRIPT },
        what: "the transcript's prose",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptMessageInk, on: ON_MESSAGE },
        what: "the message the reader sent",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptLinkInk, on: ON_TRANSCRIPT },
        what: "a link in prose",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellTitleInk, on: ON_CELL },
        what: "a cell's name at rest",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellTitleInk, on: ON_CELL_HOVER },
        what: "a cell's name under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellCodeInk, on: ON_OPENED_ROW },
        what: "the code a cell opened",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellOutputInk, on: ON_OPENED_ROW },
        what: "what a cell printed",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellSeparatorInk, on: ON_CELL },
        what: "the dot that parts a cell's meta from its elapsed time",
        floor: MARK,
    },
    // A failure is read in the same two states as a quiet cell, and its field is
    // the one a theme is most likely to leave behind.
    Pair {
        ink: Mark { role: Role::TranscriptCellInkFailed, on: ON_FAILED_CELL },
        what: "a failed cell's words",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCellInkFailed, on: ON_FAILED_CELL_HOVER },
        what: "a failed cell's words under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptReasonInk, on: ON_FAILED_CELL },
        what: "the reason a failed cell opens",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptThinkingLabelInk, on: ON_TRANSCRIPT },
        what: "the label a thinking row carries",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::TranscriptThinkingInk, on: ON_OPENED_ROW },
        what: "the text inside an opened thinking row",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCodeInk, on: ON_CODE_CARD },
        what: "a fenced block's code",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::TranscriptTableInk, on: ON_TRANSCRIPT },
        what: "a table's text",
        floor: BODY,
    },
    // The composer. The draft is the reader's own words and the one place there
    // body text is written rather than read back, so it is held to the body floor
    // on both the field it is typed on and the field a selection lays over it.
    Pair {
        ink: Mark { role: Role::ComposerDraftInk, on: ON_COMPOSER },
        what: "the draft the reader is writing",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerDraftSelectionInk, on: ON_SELECTION },
        what: "the words the reader has selected",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerCommandInk, on: ON_COMPOSER },
        what: "the command a draft starts with",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerStatusLineInk, on: ON_COMPOSER },
        what: "the composer's status line",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerStatusLineInkBad, on: ON_COMPOSER },
        what: "the status line when something went wrong",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerAttachInk, on: ON_ATTACH },
        what: "the plus that attaches a picture",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerAttachInkHover, on: ON_ATTACH_HOVER },
        what: "the plus that attaches a picture, under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerAttachmentRemoveInk, on: ON_ATTACHMENT_REMOVE },
        what: "the × that removes a picture",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerChipInk, on: ON_CHIP },
        what: "the model the chip names",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerChipInkHover, on: ON_CHIP_HOVER },
        what: "the model the chip names, pointed at or open",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerStopMark, on: ON_STOP_FILL },
        what: "the square on the stop disc",
        floor: MARK,
    },
    // The card the ring opens: the numbers are what it is opened for, so the
    // figure and every ledger value are held to the body floor, and the small
    // labels and the bar to the mark floor.
    Pair {
        ink: Mark { role: Role::ComposerCardPercentInk, on: ON_CARD },
        what: "the share of the context used",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerCardUsedInk, on: ON_CARD },
        what: "what is left of the context",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerCardLedgerInk, on: ON_CARD },
        what: "a ledger value",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerCardCostInk, on: ON_CARD },
        what: "what the session has cost",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ComposerCardLabelInk, on: ON_CARD },
        what: "the card's small labels",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerCardBarFill, on: ON_BAR_TRACK },
        what: "the bar that says how full the context is",
        floor: MARK,
    },
    // The one control is a disc rather than a mark, and what it stands on is the
    // composer's own field. Held to the mark floor because the disc is the whole
    // of what says the control is there: a send disc mixed into the box it sits
    // in leaves a button that cannot be found.
    Pair {
        ink: Mark { role: Role::ComposerSendFill, on: ON_COMPOSER },
        what: "the send disc on the composer's field",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerStopFill, on: ON_COMPOSER },
        what: "the stop disc on the composer's field",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerSendMark, on: ON_SEND_FILL },
        what: "the arrow on the send disc",
        floor: MARK,
    },
    // The colours below are recorded rather than checked, and all for the same
    // reason: the window already drew them this way and a role that has to
    // resolve to today's pixel cannot be moved here. A theme that wants any of
    // them read high overrides its role, where its own value is measured.
    //   - the sidebar's age sits at 2.7:1 on the column, under both floors: the
    //     window's miss, not a design that wants keeping.
    //   - the parked tag sits at 2.2:1, which is how a session meant to recede
    //     was drawn; the working tag lands at 5.8:1 and the idle at 6.1:1.
    //   - a cell's meta and elapsed figures sit at 2.1:1 on the cell, and the
    //     label a thinking row carries at 3.2:1 once its own hover chip is under
    //     it: both are the dimmest ink the transcript has.
    //   - a notice line sits at 2.2:1 on the column.
    //   - the language in a block's corner sits at 2.7:1 on the code card.
    //   - the failure badge is a disc of the failure's red on the failed cell's
    //     field, 1.9:1, and the jump chip's edge is 1.5:1 on the column. One is
    //     a mark with a glyph drawn inside it and the other a line whose whole
    //     job is to say "control": neither is a word.
    // The transcript's other hairlines and its scrollbar's thumb are no ink on a
    // surface at all, so no floor of this kind describes them.
    //
    // And one thing no floor here can see: the failure's six colours are fixed
    // values, so a theme that moves the field and the ink and stops there keeps
    // a failed cell drawn for a dark window. The separation below still holds —
    // a near-black cell is a long way from a light one — so nothing refuses it;
    // naming the failure roles is how a theme makes its own.
];

/// Two marks a reader has to tell apart, because nothing but their colour says
/// which is which.
struct Telling {
    a: Mark,
    b: Mark,
    /// They are named as two sides rather than one with a shared field because
    /// the fields differ: a failed cell and a quiet one are told apart by what
    /// is on them.
    what: &'static str,
    /// What the pair carries decides its floor: a state the reader has to know
    /// gets more room than a tint the reader has to notice.
    floor: f32,
}

/// The three tags carry a session's state and nothing else does, so a theme
/// that paints two of them alike has taken the state away. This is the promise
/// the sidebar ticket can keep while it may not move a pixel; the tags' own
/// contrast against the column is recorded in the note above.
const TELLINGS: &[Telling] = &[
    Telling {
        a: Mark { role: Role::TagWorking, on: ON_SIDEBAR },
        b: Mark { role: Role::TagParked, on: ON_SIDEBAR },
        what: "the working tag and the parked tag",
        floor: APART,
    },
    Telling {
        a: Mark { role: Role::TagWorking, on: ON_SIDEBAR },
        b: Mark { role: Role::TagIdle, on: ON_SIDEBAR },
        what: "the working tag and the idle tag",
        floor: APART,
    },
    Telling {
        a: Mark { role: Role::TagParked, on: ON_SIDEBAR },
        b: Mark { role: Role::TagIdle, on: ON_SIDEBAR },
        what: "the parked tag and the idle tag",
        floor: APART,
    },
    // A cell that failed says so twice: its field is warmed and its name is
    // written in the failure's red. Both are measured in each state the row is
    // read in, because a theme that greys one and leaves the other is one
    // careless role away from a transcript where nothing says which cell failed.
    Telling {
        a: Mark { role: Role::TranscriptCellFillFailed, on: ON_TRANSCRIPT },
        b: Mark { role: Role::TranscriptCellFill, on: ON_TRANSCRIPT },
        what: "a failed cell's field and a quiet cell's field",
        floor: APART_TINT,
    },
    Telling {
        a: Mark { role: Role::TranscriptCellFillFailedHover, on: ON_TRANSCRIPT },
        b: Mark { role: Role::TranscriptCellFillHover, on: ON_TRANSCRIPT },
        what: "a failed cell's field and a quiet cell's field under the pointer",
        floor: APART_TINT,
    },
    Telling {
        a: Mark { role: Role::TranscriptCellInkFailed, on: ON_FAILED_CELL },
        b: Mark { role: Role::TranscriptCellTitleInk, on: ON_CELL },
        what: "a failed cell's name and a quiet cell's name",
        floor: APART,
    },
    // The composer's one control does two opposite jobs. The arrow that becomes a
    // square says which while it is being pressed; the fill has to say it first,
    // to a reader who is only looking. The two states are never on the screen at
    // once, so the pair is compared the way the reader compares it: this fill
    // against the one they saw a moment ago.
    Telling {
        a: Mark { role: Role::ComposerSendFill, on: ON_COMPOSER },
        b: Mark { role: Role::ComposerStopFill, on: ON_COMPOSER },
        what: "the send disc and the stop disc",
        floor: APART,
    },
    // The ring is a bar bent round a circle, and its lit ticks are the only
    // thing that says how much of the context is gone. A theme that paints them
    // into the ticks it never reached leaves a circle with nothing to read.
    Telling {
        a: Mark { role: Role::ComposerRingTickLit, on: ON_COMPOSER },
        b: Mark { role: Role::ComposerRingTickUnlit, on: ON_COMPOSER },
        what: "the context ring's lit ticks and the ones it never reached",
        floor: APART,
    },
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
    pub(crate) floor: f32,
}

/// The ink-on-surface pairs this theme leaves under their floor. Empty is the
/// answer a theme author wants, and the only answer the theme that ships may
/// give.
pub(crate) fn contrast(theme: &Resolved) -> Vec<Finding> {
    PAIRS
        .iter()
        .filter_map(|pair| {
            let ratio = ratio(theme.get(pair.ink.role), surface(theme, pair.ink.on));
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
    // Each side read as the reader sees it: a colour carrying an alpha is laid
    // on the fields under it, in the order the window draws them.
    let seen = |mark: &Mark| theme.get(mark.role).over(surface(theme, mark.on));
    TELLINGS
        .iter()
        .filter_map(|telling| {
            let apart = distance(seen(&telling.a), seen(&telling.b));
            (apart < telling.floor).then_some(Clash {
                what: telling.what,
                apart,
                floor: telling.floor,
            })
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
        let marks = TELLINGS.len();
        return Some((
            0,
            format!(
                "theme `{path}`: {pairs} ink-on-surface pairs and {marks} separations clear their floors\n"
            ),
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
        report.push_str(&format!(
            "  {}: {:.1} apart, floor {:.0}\n",
            clash.what, clash.apart, clash.floor
        ));
    }
    Some((1, report))
}
