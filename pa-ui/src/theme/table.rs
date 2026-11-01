//! The window's colour vocabulary.
//!
//! Every colour the window draws is a role: a name a component asks for, and a
//! value this table decides. A role resolves from the palette under it, so a
//! theme that sets a handful of base values moves the window, and a theme that
//! wants one place to differ names that role and nothing else.
//!
//! A colour value belongs in this file, or in a theme the reader wrote. A
//! component asks for a role.

/// One colour: eight bits a channel, plus the alpha a veil needs.
///
/// Resolution works in these, so the theme can be resolved and mixed in a test
/// with no window open. `to_slint` is the only step that knows Slint exists.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// A colour written the way a theme file writes one: `#rrggbb`, or
    /// `#rrggbbaa` when it carries its own alpha.
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        let byte = |at: usize| u8::from_str_radix(digits.get(at..at + 2)?, 16).ok();
        Some(match digits.len() {
            6 => Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: 255 },
            8 => Self { r: byte(0)?, g: byte(2)?, b: byte(4)?, a: byte(6)? },
            _ => return None,
        })
    }

    /// A literal written in this file. A bad one is a mistake in this crate, not
    /// in a theme a reader wrote, so it panics where a theme file returns.
    pub(crate) fn hex(text: &str) -> Self {
        Self::parse(text).expect("the table writes well-formed colours")
    }

    /// The same colour at another alpha.
    pub(crate) fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// This colour darkened by `amount` out of 255: a field that sits below the
    /// one it is drawn on. Laid over black, which is the floor there is.
    pub(crate) fn shaded(self, amount: u8) -> Self {
        self.with_alpha(255 - amount).over(Self { r: 0, g: 0, b: 0, a: 255 })
    }

    /// This colour laid over `behind`, which must be opaque: what a veil comes
    /// out as, and what contrast is measured against.
    pub(crate) fn over(self, behind: Rgba) -> Rgba {
        let weight = u32::from(self.a);
        let rest = 255 - weight;
        let blend = |over: u8, under: u8| {
            ((u32::from(over) * weight + u32::from(under) * rest + 127) / 255) as u8
        };
        Rgba { r: blend(self.r, behind.r), g: blend(self.g, behind.g), b: blend(self.b, behind.b), a: 255 }
    }

    /// The colour the renderer draws: same channels, alpha included.
    pub(crate) fn to_slint(self) -> slint::Color {
        slint::Color::from_argb_u8(self.a, self.r, self.g, self.b)
    }

    /// `share` of this colour and the rest of `other`, both kept opaque.
    pub(crate) fn mix(self, other: Rgba, share: f32) -> Rgba {
        let lerp = |a: u8, b: u8| {
            (f32::from(a) * share + f32::from(b) * (1.0 - share)).round() as u8
        };
        Rgba {
            r: lerp(self.r, other.r),
            g: lerp(self.g, other.g),
            b: lerp(self.b, other.b),
            a: lerp(self.a, other.a),
        }
    }
}

/// The base values a theme sets: the only colours everything else is made of.
///
/// Each ships as the value the window already draws with, so the default theme
/// is today's window rather than a second design. A base value joins this list
/// when a role needs it, which is why the list is short: every one of these
/// moves something the reader can see.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Palette {
    /// The window's own background, under everything: the sidebars, the
    /// caption, the column the transcript sits in.
    Background,
    /// Body ink: the words the reader came for.
    Ink,
    /// Ink that steps back: secondary labels, a resting glyph.
    QuietInk,
    /// A stop, a failure, a delete.
    Danger,
}

impl Palette {
    pub(crate) const ALL: [Palette; 4] =
        [Palette::Background, Palette::Ink, Palette::QuietInk, Palette::Danger];

    /// The name a theme file uses.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Palette::Background => "background",
            Palette::Ink => "ink",
            Palette::QuietInk => "quiet-ink",
            Palette::Danger => "danger",
        }
    }

    /// The value the window ships with: one of the colours it drew before the
    /// theme existed, unchanged.
    fn shipped(self) -> Rgba {
        Rgba::hex(match self {
            Palette::Background => "#111111",
            Palette::Ink => "#e5e5e5",
            Palette::QuietInk => "#b0b0b0",
            Palette::Danger => "#c42b1c",
        })
    }
}

/// Where a role's value comes from when a theme file does not name it.
pub(crate) enum Derivation {
    /// The palette entry itself.
    Palette(Palette),
    /// The palette entry at an alpha, drawn over what is behind it: a hover
    /// chip, which has to let the surface through.
    Veil(Palette, u8),
    /// The palette entry darkened by an amount out of 255: the body a row opens
    /// and the card a code block is, both of which sit *below* the field they
    /// are drawn on. A mix towards the ink would move such a field the wrong
    /// way; a shade darkens it, and goes on darkening it on a light theme, so
    /// the field keeps reading as a field rather than as a slab of ink.
    Shade(Palette, u8),
    /// `share` of the first palette entry and the rest of the second, worked out
    /// when the theme resolves. A mark made this way is one opaque colour, so it
    /// draws exactly as the value it comes to while still moving with the
    /// palette it is mixed from.
    Mix(Palette, Palette, f32),
    /// A value the palette has no word for, kept because the window drew it.
    Fixed(Rgba),
}

/// One colour the window draws, named for the part it paints.
///
/// The names are the theme global's property names and the keys a theme file
/// overrides, so this list is the window's colour glossary: the test in
/// `tests.rs` holds it and `ui/theme.slint` to the same set.
// Every variant is prefixed with the part of the window it paints, because that
// prefix is the role's name in a theme file. The caption, the sidebar and the
// transcript are the parts the table has reached so far.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// The minimise, maximise and close marks at rest: ink stepped back a
    /// little, so a row of three does not shout from the corner.
    TitleBarGlyph,
    /// A caption button with the pointer on it: a faint veil of ink over the
    /// window behind it, the chip the native caption draws.
    TitleBarCaptionFillHover,
    /// The close button with the pointer on it. The only caption that warns.
    TitleBarCloseFillHover,
    /// The close mark on that fill, in white: the one mark the window draws at
    /// full strength, because a warning fill is where it has to be found.
    TitleBarCloseInkHover,
    /// The right column's fold, with the pointer on it.
    TitleBarFoldFillHover,
    /// The fold mark at rest: quiet ink stepped back further, because it is
    /// chrome nobody is using.
    TitleBarFoldGlyph,
    /// The fold mark once the pointer arrives and the column says what it does.
    TitleBarFoldGlyphHover,
    /// The sidebar's own field, folded or open. The row fills are not made of
    /// it — they are a lift of ink over the palette's background — so a theme
    /// that moves this role alone keeps the rows it shipped with, and moves
    /// them with `sidebar-row-fill` and company.
    SidebarSurface,
    /// A session row at rest: a veil at no strength, because a resting row
    /// draws nothing of its own and the column runs through it. A theme that
    /// wants the rows to read as cards names this role.
    SidebarRowFill,
    /// The row the pointer is on.
    SidebarRowFillHover,
    /// The row the reader has picked: the session the transcript is showing.
    SidebarRowFillSelected,
    /// A session's name at rest — quiet ink, because the names are a column to
    /// scan rather than the words the reader came for.
    SidebarNameInk,
    /// The picked session's name, at the ink body text gets, read against the
    /// picked row's fill.
    SidebarNameInkSelected,
    /// The name in the row being renamed. Its own role although it ships the
    /// picked name's value: it is read against the row's own field and not the
    /// picked fill, so a light sidebar has to move the two apart.
    SidebarNameInkEditing,
    /// How long ago a session last moved. The window's own value reads at
    /// 2.7:1 on the column, under any legibility floor, and this table keeps it
    /// because a pixel may not move here; `contrast.rs` records the miss instead
    /// of measuring it.
    SidebarAgeInk,
    /// A session at work. The three tags are told apart by colour alone, so
    /// `contrast.rs` holds them to a separation floor. Their own contrast
    /// against the column is recorded there, not gated: the parked tag, at
    /// 2.2:1, is drawn as a session meant to recede.
    TagWorking,
    /// A session that is not live: it is on the sidebar, but nothing is running
    /// in it.
    TagParked,
    /// A session that is live and between turns, ready for the next prompt.
    TagIdle,
    /// The field the transcript is drawn on: the window's own background lifted
    /// by a step of ink, so the sheet of rows reads above the columns beside it.
    /// The composer floats inside the same field, so a theme that moves this
    /// moves the strip around that box and never the box itself.
    TranscriptSurface,
    /// The agent's words, read at body strength on the transcript's surface.
    TranscriptProseInk,
    /// The field around the message the reader sent, which sets their own turn
    /// apart from the agent's.
    TranscriptMessageFill,
    /// The words inside it: the window's ink at full strength, because the
    /// reader is reading back what they wrote.
    TranscriptMessageInk,
    /// A link inside prose, wherever the prose is: quieter than the words around
    /// it and still told apart from them.
    TranscriptLinkInk,
    /// The line a notice row carries. A notice is the quietest thing the
    /// transcript says, and it is drawn to recede.
    TranscriptNoticeInk,
    /// The field of a cell at rest: a lift of ink over the transcript, one step
    /// up from the column so the cell reads as a thing on the sheet.
    TranscriptCellFill,
    /// The cell the pointer is on, which steps up again — and the cell whose
    /// failure reason is open, which the row holds in the same state.
    TranscriptCellFillHover,
    /// The name of what ran, in the cell's header. Read at the ink a secondary
    /// label gets: the cell is a thing to scan and open, not the words the reader
    /// came for.
    TranscriptCellTitleInk,
    /// The figures beside that name: what a cell was given, how long it took,
    /// and — in a thinking row's header — how long that row ran.
    TranscriptCellMetaInk,
    /// The `·` that parts a cell's meta from its elapsed time.
    TranscriptCellSeparatorInk,
    /// The code a cell opened: read at the strength of prose, because it is what
    /// the reader opened the cell to read. Its own role although it ships the
    /// prose's value, because it is read on the cell's panel, not on the column.
    TranscriptCellCodeInk,
    /// What the cell printed.
    TranscriptCellOutputInk,
    /// The line between a cell's code and its output, so the two are not read as
    /// one script.
    TranscriptCellRule,
    /// The edge of the body a cell opens. It and the field under it are the only
    /// two colours an opened cell adds to the sheet.
    TranscriptCellEdgeExpanded,
    /// The edge of the body a thinking row opens, its own role because the
    /// window drew the two frames a step apart and this one may not be moved.
    TranscriptThinkingEdgeExpanded,
    /// A cell that failed, at rest. Not the danger colour thinned: the value is a
    /// warmed red the palette has no word for, so — like the sidebar's tags — it
    /// stays a fixed value a theme names outright.
    TranscriptCellFillFailed,
    /// That cell under the pointer, or with its reason open.
    TranscriptCellFillFailedHover,
    /// What a failed cell says: its name in the header and what it printed, both
    /// in the same warmed red on the cell's own field.
    TranscriptCellInkFailed,
    /// The badge that says a cell failed before the header is read, and opens the
    /// reason.
    TranscriptCellBadgeFill,
    /// The badge with the pointer on it.
    TranscriptCellBadgeFillHover,
    /// The edge of the reason the badge opens, which is drawn at the window's
    /// root so no row paints over it. Its field is the failed cell's own, so the
    /// popup adds this edge and the ink below and nothing else.
    TranscriptReasonEdge,
    /// The reason itself.
    TranscriptReasonInk,
    /// The field a row opens onto: a thinking row's text, and a cell's code and
    /// output. A shade below the window's background, which is where a body
    /// belonging to the row above it sits.
    TranscriptRowFillExpanded,
    /// A thinking row at rest: a veil at no strength, because resting it draws
    /// nothing of its own and the column runs through it. A theme that wants the
    /// thinking rows to read as strips names this role.
    TranscriptThinkingFill,
    /// The thinking row with the pointer on it, which is the whole of what says
    /// the row opens and closes.
    TranscriptThinkingFillHover,
    /// The word the thinking row carries.
    TranscriptThinkingLabelInk,
    /// The thinking itself, once it is opened: the agent's working read at a
    /// step below the words it finally said.
    TranscriptThinkingInk,
    /// The card a fenced block is drawn on, below every other field in the
    /// transcript.
    TranscriptCodeFill,
    /// The code inside it, at body strength.
    TranscriptCodeInk,
    /// The language named in the block's corner. The quietest label the
    /// transcript has, and drawn to recede.
    TranscriptCodeLabelInk,
    /// The box-drawn table's text.
    TranscriptTableInk,
    /// The field behind an attached picture. `image-fit: contain` leaves it
    /// showing wherever the picture does not fill the box.
    TranscriptImageFill,
    /// The transcript's scrollbar at rest.
    TranscriptScrollThumb,
    /// That thumb under the pointer.
    TranscriptScrollThumbHover,
    /// That thumb while it is being dragged.
    TranscriptScrollThumbPressed,
    /// The chip that walks the reader back to the newest row, over the
    /// transcript's floor.
    TranscriptJumpFill,
    /// That chip under the pointer.
    TranscriptJumpFillHover,
    /// Its edge, which is the whole of what makes it read as a control while it
    /// is not being pointed at.
    TranscriptJumpEdge,
}

impl Role {
    pub(crate) const ALL: [Role; 57] = [
        Role::TitleBarGlyph,
        Role::TitleBarCaptionFillHover,
        Role::TitleBarCloseFillHover,
        Role::TitleBarCloseInkHover,
        Role::TitleBarFoldFillHover,
        Role::TitleBarFoldGlyph,
        Role::TitleBarFoldGlyphHover,
        Role::SidebarSurface,
        Role::SidebarRowFill,
        Role::SidebarRowFillHover,
        Role::SidebarRowFillSelected,
        Role::SidebarNameInk,
        Role::SidebarNameInkSelected,
        Role::SidebarNameInkEditing,
        Role::SidebarAgeInk,
        Role::TagWorking,
        Role::TagParked,
        Role::TagIdle,
        Role::TranscriptSurface,
        Role::TranscriptProseInk,
        Role::TranscriptMessageFill,
        Role::TranscriptMessageInk,
        Role::TranscriptLinkInk,
        Role::TranscriptNoticeInk,
        Role::TranscriptCellFill,
        Role::TranscriptCellFillHover,
        Role::TranscriptCellTitleInk,
        Role::TranscriptCellMetaInk,
        Role::TranscriptCellSeparatorInk,
        Role::TranscriptCellCodeInk,
        Role::TranscriptCellOutputInk,
        Role::TranscriptCellRule,
        Role::TranscriptCellEdgeExpanded,
        Role::TranscriptThinkingEdgeExpanded,
        Role::TranscriptCellFillFailed,
        Role::TranscriptCellFillFailedHover,
        Role::TranscriptCellInkFailed,
        Role::TranscriptCellBadgeFill,
        Role::TranscriptCellBadgeFillHover,
        Role::TranscriptReasonEdge,
        Role::TranscriptReasonInk,
        Role::TranscriptRowFillExpanded,
        Role::TranscriptThinkingFill,
        Role::TranscriptThinkingFillHover,
        Role::TranscriptThinkingLabelInk,
        Role::TranscriptThinkingInk,
        Role::TranscriptCodeFill,
        Role::TranscriptCodeInk,
        Role::TranscriptCodeLabelInk,
        Role::TranscriptTableInk,
        Role::TranscriptImageFill,
        Role::TranscriptScrollThumb,
        Role::TranscriptScrollThumbHover,
        Role::TranscriptScrollThumbPressed,
        Role::TranscriptJumpFill,
        Role::TranscriptJumpFillHover,
        Role::TranscriptJumpEdge,
    ];

    /// The name a theme file and the theme global both use.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::TitleBarGlyph => "title-bar-glyph",
            Role::TitleBarCaptionFillHover => "title-bar-caption-fill-hover",
            Role::TitleBarCloseFillHover => "title-bar-close-fill-hover",
            Role::TitleBarCloseInkHover => "title-bar-close-ink-hover",
            Role::TitleBarFoldFillHover => "title-bar-fold-fill-hover",
            Role::TitleBarFoldGlyph => "title-bar-fold-glyph",
            Role::TitleBarFoldGlyphHover => "title-bar-fold-glyph-hover",
            Role::SidebarSurface => "sidebar-surface",
            Role::SidebarRowFill => "sidebar-row-fill",
            Role::SidebarRowFillHover => "sidebar-row-fill-hover",
            Role::SidebarRowFillSelected => "sidebar-row-fill-selected",
            Role::SidebarNameInk => "sidebar-name-ink",
            Role::SidebarNameInkSelected => "sidebar-name-ink-selected",
            Role::SidebarNameInkEditing => "sidebar-name-ink-editing",
            Role::SidebarAgeInk => "sidebar-age-ink",
            Role::TagWorking => "tag-working",
            Role::TagParked => "tag-parked",
            Role::TagIdle => "tag-idle",
            Role::TranscriptSurface => "transcript-surface",
            Role::TranscriptProseInk => "transcript-prose-ink",
            Role::TranscriptMessageFill => "transcript-message-fill",
            Role::TranscriptMessageInk => "transcript-message-ink",
            Role::TranscriptLinkInk => "transcript-link-ink",
            Role::TranscriptNoticeInk => "transcript-notice-ink",
            Role::TranscriptCellFill => "transcript-cell-fill",
            Role::TranscriptCellFillHover => "transcript-cell-fill-hover",
            Role::TranscriptCellTitleInk => "transcript-cell-title-ink",
            Role::TranscriptCellMetaInk => "transcript-cell-meta-ink",
            Role::TranscriptCellSeparatorInk => "transcript-cell-separator-ink",
            Role::TranscriptCellCodeInk => "transcript-cell-code-ink",
            Role::TranscriptCellOutputInk => "transcript-cell-output-ink",
            Role::TranscriptCellRule => "transcript-cell-rule",
            Role::TranscriptCellEdgeExpanded => "transcript-cell-edge-expanded",
            Role::TranscriptThinkingEdgeExpanded => "transcript-thinking-edge-expanded",
            Role::TranscriptCellFillFailed => "transcript-cell-fill-failed",
            Role::TranscriptCellFillFailedHover => "transcript-cell-fill-failed-hover",
            Role::TranscriptCellInkFailed => "transcript-cell-ink-failed",
            Role::TranscriptCellBadgeFill => "transcript-cell-badge-fill",
            Role::TranscriptCellBadgeFillHover => "transcript-cell-badge-fill-hover",
            Role::TranscriptReasonEdge => "transcript-reason-edge",
            Role::TranscriptReasonInk => "transcript-reason-ink",
            Role::TranscriptRowFillExpanded => "transcript-row-fill-expanded",
            Role::TranscriptThinkingFill => "transcript-thinking-fill",
            Role::TranscriptThinkingFillHover => "transcript-thinking-fill-hover",
            Role::TranscriptThinkingLabelInk => "transcript-thinking-label-ink",
            Role::TranscriptThinkingInk => "transcript-thinking-ink",
            Role::TranscriptCodeFill => "transcript-code-fill",
            Role::TranscriptCodeInk => "transcript-code-ink",
            Role::TranscriptCodeLabelInk => "transcript-code-label-ink",
            Role::TranscriptTableInk => "transcript-table-ink",
            Role::TranscriptImageFill => "transcript-image-fill",
            Role::TranscriptScrollThumb => "transcript-scroll-thumb",
            Role::TranscriptScrollThumbHover => "transcript-scroll-thumb-hover",
            Role::TranscriptScrollThumbPressed => "transcript-scroll-thumb-pressed",
            Role::TranscriptJumpFill => "transcript-jump-fill",
            Role::TranscriptJumpFillHover => "transcript-jump-fill-hover",
            Role::TranscriptJumpEdge => "transcript-jump-edge",
        }
    }

    /// The alphas and shares below are the ones that land on the pixel the
    /// window drew before there was a theme: the caption marks at `#d7d7d7` and
    /// `#6f6f6f`, the chips at `#222222` and `#2b2b2b` over the background, the
    /// session rows at `#181818` and `#1d1d1d`, the age at `#585858`, and the
    /// transcript a shade at a time from `#0e0e0e` to `#292929`.
    ///
    /// A share is not the only reading of a value. A surface is a lift of the
    /// palette's ink over its background, and an ink that steps back is the
    /// quiet ink faded towards the background — which is why the rows and the
    /// words in them are made from different palette entries and still move
    /// together when a theme moves both.
    fn derivation(self) -> Derivation {
        match self {
            Role::TitleBarGlyph => Derivation::Mix(Palette::Ink, Palette::Background, 0.933),
            Role::TitleBarCaptionFillHover => Derivation::Veil(Palette::Ink, 0x14),
            Role::TitleBarCloseFillHover => Derivation::Palette(Palette::Danger),
            Role::TitleBarCloseInkHover => Derivation::Fixed(Rgba::hex("#ffffff")),
            Role::TitleBarFoldFillHover => Derivation::Veil(Palette::Ink, 0x1f),
            Role::TitleBarFoldGlyph => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.592),
            Role::TitleBarFoldGlyphHover => Derivation::Palette(Palette::Ink),
            Role::SidebarSurface => Derivation::Palette(Palette::Background),
            Role::SidebarRowFill => Derivation::Veil(Palette::Background, 0x00),
            Role::SidebarRowFillHover => Derivation::Mix(Palette::Ink, Palette::Background, 0.033),
            Role::SidebarRowFillSelected => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.057)
            }
            Role::SidebarNameInk => Derivation::Palette(Palette::QuietInk),
            Role::SidebarNameInkSelected => Derivation::Palette(Palette::Ink),
            Role::SidebarNameInkEditing => Derivation::Palette(Palette::Ink),
            Role::SidebarAgeInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.447),
            Role::TagWorking => Derivation::Fixed(Rgba::hex("#a18e41")),
            Role::TagParked => Derivation::Fixed(Rgba::hex("#4a4d44")),
            Role::TagIdle => Derivation::Fixed(Rgba::hex("#6f9f5f")),
            Role::TranscriptSurface => Derivation::Mix(Palette::Ink, Palette::Background, 0.0094),
            Role::TranscriptProseInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.9057),
            Role::TranscriptMessageFill => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.1132)
            }
            Role::TranscriptMessageInk => Derivation::Palette(Palette::Ink),
            Role::TranscriptLinkInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7925),
            Role::TranscriptNoticeInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.3711)
            }
            Role::TranscriptCellFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0189),
            Role::TranscriptCellFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0377)
            }
            Role::TranscriptCellTitleInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7673)
            }
            Role::TranscriptCellMetaInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.3711)
            }
            Role::TranscriptCellSeparatorInk => Derivation::Fixed(Rgba::hex("#ffffff")),
            Role::TranscriptCellCodeInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.9057),
            Role::TranscriptCellOutputInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7987)
            }
            Role::TranscriptCellRule => Derivation::Mix(Palette::Ink, Palette::Background, 0.066),
            Role::TranscriptCellEdgeExpanded => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0896)
            }
            Role::TranscriptThinkingEdgeExpanded => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.066)
            }
            Role::TranscriptCellFillFailed => Derivation::Fixed(Rgba::hex("#1d1412")),
            Role::TranscriptCellFillFailedHover => Derivation::Fixed(Rgba::hex("#241713")),
            Role::TranscriptCellInkFailed => Derivation::Fixed(Rgba::hex("#c98a7f")),
            Role::TranscriptCellBadgeFill => Derivation::Fixed(Rgba::hex("#6b342b")),
            Role::TranscriptCellBadgeFillHover => Derivation::Fixed(Rgba::hex("#8d4235")),
            Role::TranscriptReasonEdge => Derivation::Fixed(Rgba::hex("#6b342b")),
            Role::TranscriptReasonInk => Derivation::Fixed(Rgba::hex("#e0b6ad")),
            Role::TranscriptRowFillExpanded => Derivation::Shade(Palette::Background, 0x0a),
            Role::TranscriptThinkingFill => Derivation::Veil(Palette::Background, 0x00),
            Role::TranscriptThinkingFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0472)
            }
            Role::TranscriptThinkingLabelInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.566)
            }
            Role::TranscriptThinkingInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.6918)
            }
            Role::TranscriptCodeFill => Derivation::Shade(Palette::Background, 0x28),
            Role::TranscriptCodeInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.783),
            Role::TranscriptCodeLabelInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.4465)
            }
            Role::TranscriptTableInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.9623)
            }
            Role::TranscriptImageFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0472),
            Role::TranscriptScrollThumb => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.1509)
            }
            Role::TranscriptScrollThumbHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.2453)
            }
            Role::TranscriptScrollThumbPressed => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.3349)
            }
            Role::TranscriptJumpFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0519),
            Role::TranscriptJumpFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.1415)
            }
            Role::TranscriptJumpEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.1745),
        }
    }
}

/// What one theme file changes: the base values it sets, and the roles it names
/// outright. Sparse in both directions — what a file leaves out keeps the value
/// the window shipped with.
#[derive(Default)]
pub(crate) struct Overlay {
    palette: Vec<(Palette, Rgba)>,
    roles: Vec<(Role, Rgba)>,
}

impl Overlay {
    /// A base value, from the file's `palette` block.
    pub(crate) fn set_palette(&mut self, key: Palette, value: Rgba) {
        self.palette.push((key, value));
    }

    /// One role named outright, from the file's `roles` block. It beats
    /// whatever the palette would have derived for the same role.
    pub(crate) fn set_role(&mut self, role: Role, value: Rgba) {
        self.roles.push((role, value));
    }
}

/// Every role resolved: a complete map, with nothing left to look up.
#[derive(Clone, PartialEq, Eq, Debug)]
pub(crate) struct Resolved {
    palette: [Rgba; Palette::ALL.len()],
    roles: [Rgba; Role::ALL.len()],
}

impl Resolved {
    /// The theme compiled into the window. There is no file behind it, so there
    /// is nothing to fail: a selection that cannot be read leaves exactly this.
    pub(crate) fn shipped() -> Self {
        Self::with_overlay(&Overlay::default())
    }

    /// The shipped theme with one file's values on top.
    ///
    /// The whole patch arrives at once and is applied in one pass — palette,
    /// then every derivation, then the roles the file named — so a file can
    /// never leave half of itself behind.
    pub(crate) fn with_overlay(patch: &Overlay) -> Self {
        let blank = Rgba { r: 0, g: 0, b: 0, a: 255 };
        let mut palette = [blank; Palette::ALL.len()];
        for key in Palette::ALL {
            palette[key as usize] = key.shipped();
        }
        for &(key, value) in &patch.palette {
            palette[key as usize] = value;
        }

        let mut roles = [blank; Role::ALL.len()];
        for role in Role::ALL {
            roles[role as usize] = derive(&palette, role);
        }
        // An override always wins over the derivation it would otherwise have.
        for &(role, value) in &patch.roles {
            roles[role as usize] = value;
        }
        Self { palette, roles }
    }

    /// One role's colour.
    pub(crate) fn get(&self, role: Role) -> Rgba {
        self.roles[role as usize]
    }

    /// The window's own background: what everything is drawn on, and what a part
    /// that draws no field of its own — the caption — is read against. Nothing
    /// sits behind the window, so a background carrying an alpha is read as if
    /// it were solid.
    pub(crate) fn background(&self) -> Rgba {
        self.palette[Palette::Background as usize].over(Rgba { r: 0, g: 0, b: 0, a: 255 })
    }
}

fn derive(palette: &[Rgba; Palette::ALL.len()], role: Role) -> Rgba {
    match role.derivation() {
        Derivation::Palette(key) => palette[key as usize],
        Derivation::Veil(key, alpha) => palette[key as usize].with_alpha(alpha),
        Derivation::Shade(key, amount) => palette[key as usize].shaded(amount),
        Derivation::Mix(from, to, share) => palette[from as usize].mix(palette[to as usize], share),
        Derivation::Fixed(value) => value,
    }
}
