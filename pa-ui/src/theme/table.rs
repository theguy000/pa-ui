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

    /// The colour written the way a theme file writes one: `#rrggbb`, or
    /// `#rrggbbaa` when it carries an alpha of its own.
    ///
    /// The markup builder is the one consumer that cannot be handed a
    /// `slint::Color`: a `<font>` tag takes its colour as text.
    pub(crate) fn text(self) -> String {
        let mut out = format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b);
        if self.a != 255 {
            out.push_str(&format!("{:02x}", self.a));
        }
        out
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
    /// The brand mark's strongest tone: the blade at the top. It is the mark's
    /// own colour rather than the window's, which is why it is a base value and
    /// not the ink: the mark ships white because it was cut for a dark tile, and
    /// a theme that turns the window light turns the mark over by naming it.
    Brand,
}

impl Palette {
    pub(crate) const ALL: [Palette; 5] =
        [Palette::Background, Palette::Ink, Palette::QuietInk, Palette::Danger, Palette::Brand];

    /// The name a theme file uses.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Palette::Background => "background",
            Palette::Ink => "ink",
            Palette::QuietInk => "quiet-ink",
            Palette::Danger => "danger",
            Palette::Brand => "brand",
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
            Palette::Brand => "#ffffff",
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
// prefix is the role's name in a theme file. The caption, the sidebar, the brand
// mark, the transcript, the composer, the panels that open over them and the
// status strip are the parts the table has reached so far — and a mark drawn
// inside one of them is named for the part it is drawn in rather than for the
// shape of the glyph, because that is what a theme author goes looking for. The
// one exception is the brand mark, which is a single drawing: its fills are named
// for the shapes they paint, because moving one of them on its own takes the
// drawing apart and there is no part to blame it on.
#[allow(clippy::enum_variant_names)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// The window's own field, under everything: the caption paints none of its
    /// own and is read against this, the right column shows it while it is blank,
    /// and the sidebar's field ships the same value so the column reads as part of
    /// the window rather than as a slab laid on it. Its own role all the same: a
    /// theme that wants a lighter column than window, or a window one shade off
    /// its column, has to be able to say so.
    WindowSurface,
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
    /// The column's own fold, at the right of its header, at rest. The mark is
    /// the whole of what says the control folds the column, so it is read on the
    /// column it sits on rather than on a field of its own.
    SidebarFoldGlyph,
    /// That mark with the pointer on it — and the one the folded strip draws on
    /// the control that opens the column again, which draws its arrow only while
    /// the pointer is on it and so is always drawn in this state.
    SidebarFoldGlyphHover,
    /// The pen on the New Session button at rest: quiet ink, because the label
    /// beside it carries the words.
    SidebarNewSessionMark,
    /// That pen with the pointer on the button.
    SidebarNewSessionMarkHover,
    /// The same pen in the column's folded strip, where the mark is the whole
    /// control and the label has nowhere to go. It therefore reads a step above
    /// the button's quiet ink, at the weight the caption's own marks do.
    SidebarStripNewSessionMark,
    /// That pen with the pointer on the strip's control. Its own role rather than
    /// the button's hover: a theme that moves the strip's pen would have it spring
    /// back to the button's colour under the pointer.
    SidebarStripNewSessionMarkHover,
    /// The magnifier in the column's header, and again in its folded strip.
    /// Drawn and inert until search exists, and read at the quiet ink both draw.
    SidebarSearchMark,
    /// The product's name beside the brand mark, at the top of the column. The one
    /// piece of lettering in the column that is not a session's, so it is read at
    /// body ink while the names under it step back.
    SidebarWordmarkInk,
    /// The pencil a session row offers to rename it.
    SidebarRowRenameMark,
    /// The chip that pencil sits on, at rest: a lift of ink over the column, so
    /// the control reads as something to press rather than as a bare mark. The
    /// chip draws no edge, so its fill is the whole of what says it is there.
    SidebarRowRenameFill,
    /// That chip with the pointer on it. Opaque, and its own role rather than a
    /// quiet button's veil: the chip only appears under a pointer that is already
    /// on the row, so what it is laid on is the row's own fill and not the column,
    /// and a veil would land a shade lighter on a picked row than on a quiet one.
    SidebarRowRenameFillHover,
    /// The label on the column's New Session button at rest. The button carries a
    /// field of its own, so its words are read on that field rather than on the
    /// column, and the pointer brightens them.
    SidebarNewSessionInk,
    /// That label with the pointer on the button.
    SidebarNewSessionInkHover,
    /// The pill the window shows while the column is folded, saying how to open
    /// it again. The lightest field the window draws, because it floats over the
    /// transcript's sheet rather than sitting on the column.
    SidebarOpenTipFill,
    /// The words on that pill: the near-black the window puts on its lightest
    /// fills.
    SidebarOpenTipInk,
    /// The shadow that lifts the pill off the sheet it floats over.
    SidebarOpenTipShadow,
    // The brand mark. Its six fills are the artwork's own tones rather than roles
    // another part could borrow: the mark is one drawing, and a tone moved on its
    // own takes the drawing apart.
    //
    // Five of the six are the palette's brand tone mixed towards the field the
    // mark is cut from, so a theme that turns the window over turns the mark over
    // with it: the blade goes dark, the shadows go light, and every tone keeps its
    // distance from the one it is drawn on. The shares are the artwork's own
    // proportions, read off the six values it shipped with.
    /// The blade across the top of the mark: the brand tone itself.
    BrandMarkBlade,
    /// The lower wing the mark's stem hangs from.
    BrandMarkWing,
    /// The shadow the stem casts across the wing's inner edge.
    BrandMarkWingInnerShadow,
    /// The shadow the blade casts on the wing.
    BrandMarkWingShadow,
    /// The extended vertical stem.
    BrandMarkStem,
    /// The shadow the blade casts on the stem.
    BrandMarkStemShadow,
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
    /// A heading line inside prose: the one line of a message the renderer draws
    /// bold, and read a step above the prose around it because there is no size
    /// step inside one text run to say it is a heading.
    TranscriptHeadingInk,
    /// A word the reader's message wrapped in backticks, inside a line of prose.
    /// The renderer's own box around it hugs the glyphs with no padding and no
    /// styling hook, so the span is drawn as a colour instead: a step back from
    /// the prose, which is what makes it read as a symbol and not as emphasis.
    TranscriptInlineCodeInk,
    /// The caret that says a row opens, in the header of a thinking row and of a
    /// cell. One role although two kinds of row draw it: it says the same thing
    /// and it is read on the same sheet.
    TranscriptCaretMark,
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
    /// The alert inside that badge: the failure's own warm white, which the badge
    /// is opened for. No palette entry describes it, so — like the failed cell's
    /// fill and its ink — it is a fixed value a theme names outright.
    TranscriptCellBadgeMark,
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
    /// A python keyword in highlighted code. The three token roles below are
    /// drawn by one tokeniser, which paints the code inside a cell and a fenced
    /// block alike, so a theme that recolours a keyword recolours it in both
    /// places.
    TranscriptCodeKeywordInk,
    /// A string literal in that code.
    TranscriptCodeStringInk,
    /// A comment in that code: the furthest back of the three, because it is the
    /// part of the code a reader skips until they want it.
    TranscriptCodeCommentInk,
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
    /// The chevron on that chip, drawn on the chip's own fill.
    TranscriptJumpMark,
    /// The composer's own field. A shade below the window's background, so the
    /// box reads as a well in the band it sits on — and that band is the
    /// transcript's sheet, which is why the two fields are separate roles.
    ComposerSurface,
    /// The hairline round that box.
    ComposerEdge,
    /// The bar on the box's top edge that grows it, at rest.
    ComposerHandleFill,
    /// That bar with the pointer on it, which is the whole of what says the bar
    /// is a control.
    ComposerHandleFillHover,
    /// That bar while the reader is dragging it.
    ComposerHandleFillDragging,
    /// The words the reader is writing. The one place in the composer where ink
    /// is body text on a field.
    ComposerDraftInk,
    /// The caret a canvas-mode draft paints for itself, because the words it
    /// edits are a picture. A native field draws its own, and Slint gives that
    /// one no colour to set.
    ComposerDraftCaret,
    /// The field behind the words the reader has selected.
    ComposerDraftSelectionFill,
    /// Those words, read on that field. Whiter than the body ink, which the
    /// palette has no word for, so a theme names it.
    ComposerDraftSelectionInk,
    /// A command word inside a draft the canvas paints. Its own role rather than
    /// the `composer-command-ink` the token the slash menu puts in front of a
    /// draft is drawn in: the canvas's word sits in the line the reader is
    /// writing, and the window drew it a step behind the prose around it, where
    /// that token is gold.
    ComposerDraftCommandInk,
    /// The field behind the words the canvas paints as selected. Its own role
    /// although it says what the native field's selection fill says: the canvas
    /// lays it down as a rectangle in a buffer of its own — opaque, so the words
    /// keep their own ink — and the window drew it a step lighter than the fill
    /// the text input is handed.
    ComposerDraftSelectionWash,
    /// The command word a draft starts with: the token the slash menu put in
    /// front of the words. A gold no palette entry describes, so — like the
    /// sidebar's tags — it is a fixed value a theme names outright.
    ComposerCommandInk,
    /// The tile a picture the draft carries is drawn on. `image-fit: cover`
    /// leaves it showing at the edges.
    ComposerAttachmentFill,
    /// That tile's edge.
    ComposerAttachmentEdge,
    /// That edge with the pointer on the tile, which is the whole of what says
    /// the tile is also the control that removes it.
    ComposerAttachmentEdgeHover,
    /// The disc behind the × that removes a picture. It appears only while the
    /// pointer is on the tile.
    ComposerAttachmentRemoveFill,
    /// The ×'s two strokes.
    ComposerAttachmentRemoveInk,
    /// The plus that attaches a picture, at rest.
    ComposerAttachFill,
    /// That plus with the pointer on it.
    ComposerAttachFillHover,
    /// The plus mark at rest.
    ComposerAttachInk,
    /// The plus mark with the pointer on it.
    ComposerAttachInkHover,
    /// The line under the draft that says what the composer is doing.
    ComposerStatusLineInk,
    /// The same line when it carries a failure. Its own role although it ships
    /// the failed cell's red: it is read on the composer's field, and a theme
    /// that moves one of the two has already decided about the other. A fixed
    /// value, because the palette's danger is a red the window never drew a
    /// status line in, so a theme names this role rather than moving `danger`.
    ComposerStatusLineInkBad,
    /// A tick of the context ring the session has reached. The ring is a mark
    /// rather than a fill: it is drawn as 24 hairlines round a circle, so a
    /// theme can move it without moving anything else the composer draws.
    ComposerRingTickLit,
    /// A tick the session has not reached, which is the rest of the dial.
    ComposerRingTickUnlit,
    /// The field of the card the ring opens: how close the session is to
    /// forgetting, at the scale of numbers rather than of ticks.
    ComposerCardSurface,
    /// That card's edge, which is what makes it read as a panel over the window
    /// rather than as part of it.
    ComposerCardEdge,
    /// The shadow that lifts the card off the window. Black at an alpha, so a
    /// theme with a light background names this role rather than inheriting it.
    ComposerCardShadow,
    /// The card's small labels: the word over its bar and the name of each
    /// ledger line.
    ComposerCardLabelInk,
    /// The share of the context used, which is the number the card is opened
    /// for.
    ComposerCardPercentInk,
    /// The track of the bar that answers "how close am I" before a single
    /// number is read.
    ComposerCardBarTrack,
    /// The part of that track the session has used.
    ComposerCardBarFill,
    /// The words under the bar: what is left of the context, and how much of
    /// it the session still has.
    ComposerCardUsedInk,
    /// The hairline that parts the card's three questions — the window, the
    /// counts, the money — so they are read as three and not as one column.
    ComposerCardRule,
    /// A ledger's value, flush right in its column.
    ComposerCardLedgerInk,
    /// What the session, and the subagents under it, have cost. The one ledger
    /// the card draws at full strength, because it is the one the reader came
    /// to the card to check.
    ComposerCardCostInk,
    /// The chip that names the running model, at rest.
    ComposerChipFill,
    /// That chip with the pointer on it, or with its menu open: the resting
    /// state is what the old hover state was, because a chip that appears only
    /// under the pointer reads as decoration rather than as a control.
    ComposerChipFillHover,
    /// That chip's edge at rest.
    ComposerChipEdge,
    /// That chip's edge while it is pointed at or open.
    ComposerChipEdgeHover,
    /// The model's name at rest.
    ComposerChipInk,
    /// The model's name while the chip is pointed at or open.
    ComposerChipInkHover,
    /// The caret beside that name, which says the chip opens the model menu. Read
    /// on the chip's own fill in both states the chip has, and its own role rather
    /// than the transcript's: a theme that repaints the cards the reader opens
    /// must reach this one too.
    ComposerChipCaretMark,
    /// What the control is filled with while it sends. The lightest thing in
    /// the composer: with no hue to spend, the primary action spends contrast
    /// and carries the near-black arrow below. A value above the palette's ink,
    /// which the palette has no word for, so a theme names it.
    ComposerSendFill,
    /// That fill with the pointer on it.
    ComposerSendFillHover,
    /// The arrow on that fill. Its own role because the arrow is the one mark a
    /// theme has to move when it moves the disc: the send fill ships lighter
    /// than anything else in the window, so a theme that darkens the disc and
    /// leaves this one behind draws a dark mark on it.
    ComposerSendMark,
    /// The square the send arrow becomes as the control turns into a stop.
    ComposerStopMark,
    /// The field of a panel that opens over the window: the command list, the
    /// model menu, the ladder that drops off one of its rows, and the queue card.
    /// One role for all four, because they are one shape and a theme that raises
    /// the panel surface raises every panel — which is the whole of what makes
    /// four panels cost one role rather than four. A panel here is a surface that
    /// opens over the window and closes again; it is not a region of the window's
    /// layout, which is what the glossary's ban on "side panel" keeps apart.
    PanelSurface,
    /// The hairline round that panel: the whole of what parts it from the fields
    /// it hangs over, since a panel and the sheet under it are the same grey.
    PanelEdge,
    /// The shadow that lifts a panel off the window. Black at an alpha, so a
    /// theme with a light background names this role rather than inheriting it.
    PanelShadow,
    /// The shadow under the ladder, which is shallower than a panel's: it is
    /// drawn on another panel, and what is under it has already been lifted.
    PanelShadowNested,
    /// The row the pointer is on, in any of the four panels. A resting row draws
    /// nothing of its own, so the panel runs through it.
    PanelRowFillHover,
    /// The row a panel fills to say which one is in force: the model this session
    /// runs. A panel state rather than a model menu's own, although the model
    /// menu is the only one that fills a row today — the ladder marks its rung
    /// with a check instead, which is this shape's other way of saying it.
    PanelRowFillCurrent,
    /// The edge of the card that names a session, which hangs off a row beside
    /// the column. Its own role although it ships a shade under the panels'
    /// edge: it is drawn against the window rather than against another panel,
    /// and it is dimmer because it is nearer the reader than a menu is.
    SessionCardEdge,
    /// The words on that card.
    SessionCardInk,
    /// A command's name at rest: quiet ink, because the list is a column to scan.
    SlashMenuRowInk,
    /// That name while the row is the one the arrows or the pointer is on. The
    /// keyboard's row is drawn exactly as the pointer's, because there is one
    /// highlight and it is never in two places.
    SlashMenuRowInkHover,
    /// The argument a command takes, which only the highlighted row shows: read
    /// at the ink a link gets, one step behind the name beside it.
    SlashMenuHintInk,
    /// What a command does, on the highlighted row. The quietest word the list
    /// carries, because it is there for the row the reader is already on.
    SlashMenuDescriptionInk,
    /// A provider's name. The menu is opened to pick one of the models under it,
    /// so its headers are scanned rather than read: a step under the ink a
    /// model's own name gets.
    ModelMenuHeaderInk,
    /// A provider the reader is not signed in to. Nothing behind the header can
    /// be picked, so the name steps back to what the window says beside it.
    ModelMenuHeaderInkQuiet,
    /// The caret in front of a provider's name, which says the header opens and
    /// closes the models under it.
    ModelMenuCaretMark,
    /// The figure beside a header, and the words that stand in its place. The
    /// window draws it at 2.5:1 on the panel and this table keeps that because
    /// no pixel may move here; `contrast.rs` records the miss rather than
    /// measuring it.
    ModelMenuCountInk,
    /// A model's name at rest.
    ModelMenuRowInk,
    /// The running model's name, at the ink body text gets: it is the one row
    /// the menu exists to confirm.
    ModelMenuRowInkCurrent,
    /// The tick beside that name, which marks the row as well as tinting it. Gold,
    /// the same gold the working tag and the running model's level are drawn in,
    /// because the three say the same thing: this is what is running. No palette
    /// entry describes it, so a theme names it outright.
    ModelMenuCheckMark,
    /// The context window beside a model's name, for every row but the running
    /// one — which carries its level there instead.
    ModelMenuDetailInk,
    /// How hard the running model is thinking, right of its name: the gold the
    /// sidebar's working tag is drawn in, because it says the same thing.
    ModelMenuEffortInk,
    /// A rung of the ladder at rest.
    EffortLadderRowInk,
    /// The rung the model is on: the ink body text gets, beside the check that
    /// marks it.
    EffortLadderRowInkCurrent,
    /// The tick beside that rung. Its own role although it ships the model menu's
    /// gold: the ladder hangs off the menu, and a theme that repaints one of the
    /// two panels must reach the tick drawn on it.
    EffortLadderCheckMark,
    /// The count of waiting messages over the queue card's first lane.
    QueueCardCountInk,
    /// The name of a lane that carries nothing to count — "runs next" over the
    /// second one. A label rather than a figure, so it steps back from the count
    /// above it.
    QueueCardLaneInk,
    /// The action that empties both lanes.
    QueueCardClearInk,
    /// That action with the pointer on it, at the ink a waiting message's own
    /// row is written in.
    QueueCardClearInkHover,
    /// One waiting message, in the row that opens it for editing.
    QueueRowInk,
    /// The fill of a button inside a waiting row: editing it, pulling it into
    /// the running turn, or deleting it. A resting surface rather than a hover
    /// only one, because a control found only by pointing at it is hidden.
    QueueRowControlFill,
    /// That button under the pointer.
    QueueRowControlFillHover,
    /// The pencil and the bin, at rest.
    QueueRowControlMark,
    /// The pencil under the pointer.
    QueueRowControlMarkHover,
    /// The arrow that pulls a follow-up message into the turn already running.
    /// It is read at the ink a link gets rather than the body ink its neighbours
    /// get: it is the one control a row offers that moves a message between the
    /// lanes, and the window drew it a step back.
    QueueRowPromoteMark,
    /// That arrow under the pointer.
    QueueRowPromoteMarkHover,
    /// The bin under the pointer, in the warmth the transcript's failure is
    /// drawn in: the one control a waiting row carries that throws something
    /// away, and the only one that changes colour as it is pointed at.
    QueueRowDeleteMark,
    /// The field of the strip that opens above the composer: a step above the
    /// window's background and below the box it sits behind.
    StatusStripSurface,
    /// The outline of the strip's rounded top. Its own role although it ships
    /// the panel's edge: the strip draws its fill inset by exactly this stroke,
    /// so a theme that wants a lighter outline moves this one and keeps the
    /// panels' edges where they were.
    StatusStripEdge,
    /// The spinner and the word beside it that name what the turn is doing: one
    /// role although it paints a mark and a word, because they say one thing,
    /// and a theme that moved one alone would leave a spinner turning in one
    /// colour beside a label in another.
    StatusStripWorkingInk,
    /// How long the turn has been running. The window draws it at 2.5:1 on the
    /// strip — it recedes behind the word it follows — and this table keeps that
    /// because no pixel may move here; `contrast.rs` records the miss.
    StatusStripElapsedInk,
    /// The pill that opens the queue, at rest: a veil at no strength, because at
    /// rest the pill is part of the strip. A theme that wants the queue always
    /// visible names this role.
    StatusStripQueueFill,
    /// The pill the pointer is on, and the pill whose card is open. One state
    /// rather than two, because the card outlives the pointer: a pill that
    /// dropped back to nothing under its own open panel would read as closed.
    StatusStripQueueFillHover,
    /// How many messages are waiting: the number the pill is opened to answer,
    /// so it is read at body strength.
    StatusStripQueueCountInk,
    /// That number with the pointer on it. Its own role although the fill under
    /// it stays lit while the card is open: the window brightens the words for
    /// the pill being pointed at and for nothing else.
    StatusStripQueueCountInkHover,
    /// The word beside the number, which names what is counted.
    StatusStripQueueLabelInk,
    /// That word with the pointer on it.
    StatusStripQueueLabelInkHover,
    // The field a control paints, in the three characters a control comes in.
    // These are named for the shape and not for the part because the shape is
    // what they belong to: a quiet control is quiet wherever it is drawn, and the
    // column already draws five of them. A part whose field is genuinely its own
    // — the disc that sends, the pill that opens the column, a waiting row's
    // buttons — names that role at the call site instead, and the shape, the
    // states and the hit area are shared either way.
    /// A quiet control at rest: a veil at no strength, because a control with no
    /// field reads as part of the column under it until the pointer arrives.
    ButtonQuietFill,
    /// That control with the pointer on it.
    ButtonQuietFillHover,
    /// The field a part's main action paints at rest: lifted off its column, so
    /// the one control the part exists for is found without being pointed at.
    ButtonPrimaryFill,
    /// That field with the pointer on it.
    ButtonPrimaryFillHover,
    /// The field the control that stops a turn paints. Warm, and apart from the
    /// send disc's: the two say opposite things, and a theme has to be able to
    /// move one without the other.
    ButtonDangerFill,
    /// That field with the pointer on it.
    ButtonDangerFillHover,
}

impl Role {
    pub(crate) const ALL: [Role; 187] = [
        Role::WindowSurface,
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
        Role::SidebarFoldGlyph,
        Role::SidebarFoldGlyphHover,
        Role::SidebarNewSessionMark,
        Role::SidebarNewSessionMarkHover,
        Role::SidebarStripNewSessionMark,
        Role::SidebarStripNewSessionMarkHover,
        Role::SidebarSearchMark,
        Role::SidebarWordmarkInk,
        Role::SidebarRowRenameMark,
        Role::SidebarRowRenameFill,
        Role::SidebarRowRenameFillHover,
        Role::SidebarNewSessionInk,
        Role::SidebarNewSessionInkHover,
        Role::SidebarOpenTipFill,
        Role::SidebarOpenTipInk,
        Role::SidebarOpenTipShadow,
        Role::BrandMarkBlade,
        Role::BrandMarkWing,
        Role::BrandMarkWingInnerShadow,
        Role::BrandMarkWingShadow,
        Role::BrandMarkStem,
        Role::BrandMarkStemShadow,
        Role::TranscriptSurface,
        Role::TranscriptProseInk,
        Role::TranscriptMessageFill,
        Role::TranscriptMessageInk,
        Role::TranscriptLinkInk,
        Role::TranscriptNoticeInk,
        Role::TranscriptHeadingInk,
        Role::TranscriptInlineCodeInk,
        Role::TranscriptCaretMark,
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
        Role::TranscriptCellBadgeMark,
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
        Role::TranscriptCodeKeywordInk,
        Role::TranscriptCodeStringInk,
        Role::TranscriptCodeCommentInk,
        Role::TranscriptTableInk,
        Role::TranscriptImageFill,
        Role::TranscriptScrollThumb,
        Role::TranscriptScrollThumbHover,
        Role::TranscriptScrollThumbPressed,
        Role::TranscriptJumpFill,
        Role::TranscriptJumpFillHover,
        Role::TranscriptJumpEdge,
        Role::TranscriptJumpMark,
        Role::ComposerSurface,
        Role::ComposerEdge,
        Role::ComposerHandleFill,
        Role::ComposerHandleFillHover,
        Role::ComposerHandleFillDragging,
        Role::ComposerDraftInk,
        Role::ComposerDraftCaret,
        Role::ComposerDraftSelectionFill,
        Role::ComposerDraftSelectionInk,
        Role::ComposerDraftCommandInk,
        Role::ComposerDraftSelectionWash,
        Role::ComposerCommandInk,
        Role::ComposerAttachmentFill,
        Role::ComposerAttachmentEdge,
        Role::ComposerAttachmentEdgeHover,
        Role::ComposerAttachmentRemoveFill,
        Role::ComposerAttachmentRemoveInk,
        Role::ComposerAttachFill,
        Role::ComposerAttachFillHover,
        Role::ComposerAttachInk,
        Role::ComposerAttachInkHover,
        Role::ComposerStatusLineInk,
        Role::ComposerStatusLineInkBad,
        Role::ComposerRingTickLit,
        Role::ComposerRingTickUnlit,
        Role::ComposerCardSurface,
        Role::ComposerCardEdge,
        Role::ComposerCardShadow,
        Role::ComposerCardLabelInk,
        Role::ComposerCardPercentInk,
        Role::ComposerCardBarTrack,
        Role::ComposerCardBarFill,
        Role::ComposerCardUsedInk,
        Role::ComposerCardRule,
        Role::ComposerCardLedgerInk,
        Role::ComposerCardCostInk,
        Role::ComposerChipFill,
        Role::ComposerChipFillHover,
        Role::ComposerChipEdge,
        Role::ComposerChipEdgeHover,
        Role::ComposerChipInk,
        Role::ComposerChipInkHover,
        Role::ComposerChipCaretMark,
        Role::ComposerSendFill,
        Role::ComposerSendFillHover,
        Role::ComposerSendMark,
        Role::ComposerStopMark,
        Role::PanelSurface,
        Role::PanelEdge,
        Role::PanelShadow,
        Role::PanelShadowNested,
        Role::PanelRowFillHover,
        Role::PanelRowFillCurrent,
        Role::SessionCardEdge,
        Role::SessionCardInk,
        Role::SlashMenuRowInk,
        Role::SlashMenuRowInkHover,
        Role::SlashMenuHintInk,
        Role::SlashMenuDescriptionInk,
        Role::ModelMenuHeaderInk,
        Role::ModelMenuHeaderInkQuiet,
        Role::ModelMenuCaretMark,
        Role::ModelMenuCountInk,
        Role::ModelMenuRowInk,
        Role::ModelMenuRowInkCurrent,
        Role::ModelMenuCheckMark,
        Role::ModelMenuDetailInk,
        Role::ModelMenuEffortInk,
        Role::EffortLadderRowInk,
        Role::EffortLadderRowInkCurrent,
        Role::EffortLadderCheckMark,
        Role::QueueCardCountInk,
        Role::QueueCardLaneInk,
        Role::QueueCardClearInk,
        Role::QueueCardClearInkHover,
        Role::QueueRowInk,
        Role::QueueRowControlFill,
        Role::QueueRowControlFillHover,
        Role::QueueRowControlMark,
        Role::QueueRowControlMarkHover,
        Role::QueueRowPromoteMark,
        Role::QueueRowPromoteMarkHover,
        Role::QueueRowDeleteMark,
        Role::StatusStripSurface,
        Role::StatusStripEdge,
        Role::StatusStripWorkingInk,
        Role::StatusStripElapsedInk,
        Role::StatusStripQueueFill,
        Role::StatusStripQueueFillHover,
        Role::StatusStripQueueCountInk,
        Role::StatusStripQueueCountInkHover,
        Role::StatusStripQueueLabelInk,
        Role::StatusStripQueueLabelInkHover,
        Role::ButtonQuietFill,
        Role::ButtonQuietFillHover,
        Role::ButtonPrimaryFill,
        Role::ButtonPrimaryFillHover,
        Role::ButtonDangerFill,
        Role::ButtonDangerFillHover,
    ];

    /// The name a theme file and the theme global both use.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Role::WindowSurface => "window-surface",
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
            Role::SidebarFoldGlyph => "sidebar-fold-glyph",
            Role::SidebarFoldGlyphHover => "sidebar-fold-glyph-hover",
            Role::SidebarNewSessionMark => "sidebar-new-session-mark",
            Role::SidebarNewSessionMarkHover => "sidebar-new-session-mark-hover",
            Role::SidebarStripNewSessionMark => "sidebar-strip-new-session-mark",
            Role::SidebarStripNewSessionMarkHover => "sidebar-strip-new-session-mark-hover",
            Role::SidebarSearchMark => "sidebar-search-mark",
            Role::SidebarWordmarkInk => "sidebar-wordmark-ink",
            Role::SidebarRowRenameMark => "sidebar-row-rename-mark",
            Role::SidebarRowRenameFill => "sidebar-row-rename-fill",
            Role::SidebarRowRenameFillHover => "sidebar-row-rename-fill-hover",
            Role::SidebarNewSessionInk => "sidebar-new-session-ink",
            Role::SidebarNewSessionInkHover => "sidebar-new-session-ink-hover",
            Role::SidebarOpenTipFill => "sidebar-open-tip-fill",
            Role::SidebarOpenTipInk => "sidebar-open-tip-ink",
            Role::SidebarOpenTipShadow => "sidebar-open-tip-shadow",
            Role::BrandMarkBlade => "brand-mark-blade",
            Role::BrandMarkWing => "brand-mark-wing",
            Role::BrandMarkWingInnerShadow => "brand-mark-wing-inner-shadow",
            Role::BrandMarkWingShadow => "brand-mark-wing-shadow",
            Role::BrandMarkStem => "brand-mark-stem",
            Role::BrandMarkStemShadow => "brand-mark-stem-shadow",
            Role::TranscriptSurface => "transcript-surface",
            Role::TranscriptProseInk => "transcript-prose-ink",
            Role::TranscriptMessageFill => "transcript-message-fill",
            Role::TranscriptMessageInk => "transcript-message-ink",
            Role::TranscriptLinkInk => "transcript-link-ink",
            Role::TranscriptNoticeInk => "transcript-notice-ink",
            Role::TranscriptHeadingInk => "transcript-heading-ink",
            Role::TranscriptInlineCodeInk => "transcript-inline-code-ink",
            Role::TranscriptCaretMark => "transcript-caret-mark",
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
            Role::TranscriptCellBadgeMark => "transcript-cell-badge-mark",
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
            Role::TranscriptCodeKeywordInk => "transcript-code-keyword-ink",
            Role::TranscriptCodeStringInk => "transcript-code-string-ink",
            Role::TranscriptCodeCommentInk => "transcript-code-comment-ink",
            Role::TranscriptTableInk => "transcript-table-ink",
            Role::TranscriptImageFill => "transcript-image-fill",
            Role::TranscriptScrollThumb => "transcript-scroll-thumb",
            Role::TranscriptScrollThumbHover => "transcript-scroll-thumb-hover",
            Role::TranscriptScrollThumbPressed => "transcript-scroll-thumb-pressed",
            Role::TranscriptJumpFill => "transcript-jump-fill",
            Role::TranscriptJumpFillHover => "transcript-jump-fill-hover",
            Role::TranscriptJumpEdge => "transcript-jump-edge",
            Role::TranscriptJumpMark => "transcript-jump-mark",
            Role::ComposerSurface => "composer-surface",
            Role::ComposerEdge => "composer-edge",
            Role::ComposerHandleFill => "composer-handle-fill",
            Role::ComposerHandleFillHover => "composer-handle-fill-hover",
            Role::ComposerHandleFillDragging => "composer-handle-fill-dragging",
            Role::ComposerDraftInk => "composer-draft-ink",
            Role::ComposerDraftCaret => "composer-draft-caret",
            Role::ComposerDraftSelectionFill => "composer-draft-selection-fill",
            Role::ComposerDraftSelectionInk => "composer-draft-selection-ink",
            Role::ComposerDraftCommandInk => "composer-draft-command-ink",
            Role::ComposerDraftSelectionWash => "composer-draft-selection-wash",
            Role::ComposerCommandInk => "composer-command-ink",
            Role::ComposerAttachmentFill => "composer-attachment-fill",
            Role::ComposerAttachmentEdge => "composer-attachment-edge",
            Role::ComposerAttachmentEdgeHover => "composer-attachment-edge-hover",
            Role::ComposerAttachmentRemoveFill => "composer-attachment-remove-fill",
            Role::ComposerAttachmentRemoveInk => "composer-attachment-remove-ink",
            Role::ComposerAttachFill => "composer-attach-fill",
            Role::ComposerAttachFillHover => "composer-attach-fill-hover",
            Role::ComposerAttachInk => "composer-attach-ink",
            Role::ComposerAttachInkHover => "composer-attach-ink-hover",
            Role::ComposerStatusLineInk => "composer-status-line-ink",
            Role::ComposerStatusLineInkBad => "composer-status-line-ink-bad",
            Role::ComposerRingTickLit => "composer-ring-tick-lit",
            Role::ComposerRingTickUnlit => "composer-ring-tick-unlit",
            Role::ComposerCardSurface => "composer-card-surface",
            Role::ComposerCardEdge => "composer-card-edge",
            Role::ComposerCardShadow => "composer-card-shadow",
            Role::ComposerCardLabelInk => "composer-card-label-ink",
            Role::ComposerCardPercentInk => "composer-card-percent-ink",
            Role::ComposerCardBarTrack => "composer-card-bar-track",
            Role::ComposerCardBarFill => "composer-card-bar-fill",
            Role::ComposerCardUsedInk => "composer-card-used-ink",
            Role::ComposerCardRule => "composer-card-rule",
            Role::ComposerCardLedgerInk => "composer-card-ledger-ink",
            Role::ComposerCardCostInk => "composer-card-cost-ink",
            Role::ComposerChipFill => "composer-chip-fill",
            Role::ComposerChipFillHover => "composer-chip-fill-hover",
            Role::ComposerChipEdge => "composer-chip-edge",
            Role::ComposerChipEdgeHover => "composer-chip-edge-hover",
            Role::ComposerChipInk => "composer-chip-ink",
            Role::ComposerChipInkHover => "composer-chip-ink-hover",
            Role::ComposerChipCaretMark => "composer-chip-caret-mark",
            Role::ComposerSendFill => "composer-send-fill",
            Role::ComposerSendFillHover => "composer-send-fill-hover",
            Role::ComposerSendMark => "composer-send-mark",
            Role::ComposerStopMark => "composer-stop-mark",
            Role::PanelSurface => "panel-surface",
            Role::PanelEdge => "panel-edge",
            Role::PanelShadow => "panel-shadow",
            Role::PanelShadowNested => "panel-shadow-nested",
            Role::PanelRowFillHover => "panel-row-fill-hover",
            Role::PanelRowFillCurrent => "panel-row-fill-current",
            Role::SessionCardEdge => "session-card-edge",
            Role::SessionCardInk => "session-card-ink",
            Role::SlashMenuRowInk => "slash-menu-row-ink",
            Role::SlashMenuRowInkHover => "slash-menu-row-ink-hover",
            Role::SlashMenuHintInk => "slash-menu-hint-ink",
            Role::SlashMenuDescriptionInk => "slash-menu-description-ink",
            Role::ModelMenuHeaderInk => "model-menu-header-ink",
            Role::ModelMenuHeaderInkQuiet => "model-menu-header-ink-quiet",
            Role::ModelMenuCaretMark => "model-menu-caret-mark",
            Role::ModelMenuCountInk => "model-menu-count-ink",
            Role::ModelMenuRowInk => "model-menu-row-ink",
            Role::ModelMenuRowInkCurrent => "model-menu-row-ink-current",
            Role::ModelMenuCheckMark => "model-menu-check-mark",
            Role::ModelMenuDetailInk => "model-menu-detail-ink",
            Role::ModelMenuEffortInk => "model-menu-effort-ink",
            Role::EffortLadderRowInk => "effort-ladder-row-ink",
            Role::EffortLadderRowInkCurrent => "effort-ladder-row-ink-current",
            Role::EffortLadderCheckMark => "effort-ladder-check-mark",
            Role::QueueCardCountInk => "queue-card-count-ink",
            Role::QueueCardLaneInk => "queue-card-lane-ink",
            Role::QueueCardClearInk => "queue-card-clear-ink",
            Role::QueueCardClearInkHover => "queue-card-clear-ink-hover",
            Role::QueueRowInk => "queue-row-ink",
            Role::QueueRowControlFill => "queue-row-control-fill",
            Role::QueueRowControlFillHover => "queue-row-control-fill-hover",
            Role::QueueRowControlMark => "queue-row-control-mark",
            Role::QueueRowControlMarkHover => "queue-row-control-mark-hover",
            Role::QueueRowPromoteMark => "queue-row-promote-mark",
            Role::QueueRowPromoteMarkHover => "queue-row-promote-mark-hover",
            Role::QueueRowDeleteMark => "queue-row-delete-mark",
            Role::StatusStripSurface => "status-strip-surface",
            Role::StatusStripEdge => "status-strip-edge",
            Role::StatusStripWorkingInk => "status-strip-working-ink",
            Role::StatusStripElapsedInk => "status-strip-elapsed-ink",
            Role::StatusStripQueueFill => "status-strip-queue-fill",
            Role::StatusStripQueueFillHover => "status-strip-queue-fill-hover",
            Role::StatusStripQueueCountInk => "status-strip-queue-count-ink",
            Role::StatusStripQueueCountInkHover => "status-strip-queue-count-ink-hover",
            Role::StatusStripQueueLabelInk => "status-strip-queue-label-ink",
            Role::StatusStripQueueLabelInkHover => "status-strip-queue-label-ink-hover",
            Role::ButtonQuietFill => "button-quiet-fill",
            Role::ButtonQuietFillHover => "button-quiet-fill-hover",
            Role::ButtonPrimaryFill => "button-primary-fill",
            Role::ButtonPrimaryFillHover => "button-primary-fill-hover",
            Role::ButtonDangerFill => "button-danger-fill",
            Role::ButtonDangerFillHover => "button-danger-fill-hover",
        }
    }

    /// The alphas and shares below are the ones that land on the pixel the
    /// window drew before there was a theme: the caption marks at `#d7d7d7` and
    /// `#6f6f6f`, the chips at `#222222` and `#2b2b2b` over the background, the
    /// session rows at `#181818` and `#1d1d1d`, the age at `#585858`, the
    /// transcript a shade at a time from `#0e0e0e` to `#292929`, the composer
    /// from `#0c0c0c` to `#e5e5e5`, the panels that open over the window from
    /// `#181818` to `#3a3a3a`, and the strip from `#191919` to `#c9a86a`.
    ///
    /// A share is not the only reading of a value. A surface is a lift of the
    /// palette's ink over its background, and an ink that steps back is the
    /// quiet ink faded towards the background — which is why the rows and the
    /// words in them are made from different palette entries and still move
    /// together when a theme moves both.
    fn derivation(self) -> Derivation {
        match self {
            Role::WindowSurface => Derivation::Palette(Palette::Background),
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
            Role::SidebarFoldGlyph => Derivation::Palette(Palette::QuietInk),
            Role::SidebarFoldGlyphHover => Derivation::Palette(Palette::Ink),
            Role::SidebarNewSessionMark => Derivation::Palette(Palette::QuietInk),
            Role::SidebarNewSessionMarkHover => Derivation::Palette(Palette::Ink),
            Role::SidebarStripNewSessionMark => Derivation::Mix(Palette::Ink, Palette::Background, 0.933),
            Role::SidebarStripNewSessionMarkHover => Derivation::Palette(Palette::Ink),
            Role::SidebarSearchMark => Derivation::Palette(Palette::QuietInk),
            Role::SidebarWordmarkInk => Derivation::Palette(Palette::Ink),
            Role::SidebarRowRenameMark => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7673),
            Role::SidebarRowRenameFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0566),
            Role::SidebarRowRenameFillHover => Derivation::Fixed(Rgba::hex("#2b2b2b")),
            Role::SidebarNewSessionInk => Derivation::Palette(Palette::Ink),
            Role::SidebarNewSessionInkHover => Derivation::Fixed(Rgba::hex("#ffffff")),
            Role::SidebarOpenTipFill => Derivation::Fixed(Rgba::hex("#e3e3e3")),
            Role::SidebarOpenTipInk => Derivation::Fixed(Rgba::hex("#1f1f1f")),
            Role::SidebarOpenTipShadow => Derivation::Fixed(Rgba::hex("#00000066")),
            Role::BrandMarkBlade => Derivation::Palette(Palette::Brand),
            Role::BrandMarkWing => Derivation::Mix(Palette::Brand, Palette::Background, 0.6597),
            Role::BrandMarkWingInnerShadow => {
                Derivation::Mix(Palette::Brand, Palette::Background, 0.479)
            }
            Role::BrandMarkWingShadow => Derivation::Mix(Palette::Brand, Palette::Background, 0.3067),
            Role::BrandMarkStem => Derivation::Mix(Palette::Brand, Palette::Background, 0.3445),
            Role::BrandMarkStemShadow => Derivation::Mix(Palette::Brand, Palette::Background, 0.1429),
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
            Role::TranscriptHeadingInk => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.7594)
            }
            Role::TranscriptInlineCodeInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7925)
            }
            Role::TranscriptCaretMark => Derivation::Mix(Palette::Ink, Palette::Background, 0.5),
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
            Role::TranscriptCellBadgeMark => Derivation::Fixed(Rgba::hex("#f3ded8")),
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
            Role::TranscriptCodeKeywordInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.9748)
            }
            Role::TranscriptCodeStringInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.9623)
            }
            Role::TranscriptCodeCommentInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.4969)
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
            Role::TranscriptJumpMark => Derivation::Mix(Palette::Ink, Palette::Background, 0.7594),
            Role::ComposerSurface => Derivation::Shade(Palette::Background, 0x0a),
            Role::ComposerEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.0896),
            Role::ComposerHandleFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.1698),
            Role::ComposerHandleFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.2783)
            }
            Role::ComposerHandleFillDragging => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.434)
            }
            Role::ComposerDraftInk => Derivation::Palette(Palette::Ink),
            Role::ComposerDraftCaret => Derivation::Palette(Palette::Ink),
            Role::ComposerDraftSelectionFill => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.2075)
            }
            Role::ComposerDraftSelectionInk => Derivation::Fixed(Rgba::hex("#f0f0f0")),
            Role::ComposerDraftCommandInk => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.8302)
            }
            Role::ComposerDraftSelectionWash => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.2358)
            }
            Role::ComposerCommandInk => Derivation::Fixed(Rgba::hex("#d8c06a")),
            Role::ComposerAttachmentFill => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0377)
            }
            Role::ComposerAttachmentEdge => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.1226)
            }
            Role::ComposerAttachmentEdgeHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.2028)
            }
            Role::ComposerAttachmentRemoveFill => Derivation::Shade(Palette::Background, 0x4b),
            Role::ComposerAttachmentRemoveInk => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.9292)
            }
            Role::ComposerAttachFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0425),
            Role::ComposerAttachFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0896)
            }
            Role::ComposerAttachInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7736),
            Role::ComposerAttachInkHover => Derivation::Palette(Palette::QuietInk),
            Role::ComposerStatusLineInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786),
            Role::ComposerStatusLineInkBad => Derivation::Fixed(Rgba::hex("#c98a7f")),
            Role::ComposerRingTickLit => Derivation::Fixed(Rgba::hex("#c9a86a")),
            Role::ComposerRingTickUnlit => Derivation::Mix(Palette::Ink, Palette::Background, 0.1698),
            Role::ComposerCardSurface => Derivation::Mix(Palette::Ink, Palette::Background, 0.033),
            Role::ComposerCardEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.1226),
            Role::ComposerCardShadow => Derivation::Fixed(Rgba::hex("#00000066")),
            Role::ComposerCardLabelInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786)
            }
            Role::ComposerCardPercentInk => Derivation::Palette(Palette::Ink),
            Role::ComposerCardBarTrack => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0896)
            }
            Role::ComposerCardBarFill => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7925)
            }
            Role::ComposerCardUsedInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7673)
            }
            Role::ComposerCardRule => Derivation::Mix(Palette::Ink, Palette::Background, 0.0566),
            Role::ComposerCardLedgerInk => Derivation::Palette(Palette::QuietInk),
            Role::ComposerCardCostInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.933),
            Role::ComposerChipFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0425),
            Role::ComposerChipFillHover => Derivation::Mix(Palette::Ink, Palette::Background, 0.0896),
            Role::ComposerChipEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.0896),
            Role::ComposerChipEdgeHover => Derivation::Mix(Palette::Ink, Palette::Background, 0.1698),
            Role::ComposerChipInk => Derivation::Palette(Palette::QuietInk),
            Role::ComposerChipInkHover => Derivation::Palette(Palette::Ink),
            Role::ComposerChipCaretMark => Derivation::Mix(Palette::Ink, Palette::Background, 0.5),
            Role::ComposerSendFill => Derivation::Fixed(Rgba::hex("#e6e6e6")),
            Role::ComposerSendFillHover => Derivation::Fixed(Rgba::hex("#f2f2f2")),
            Role::ComposerSendMark => Derivation::Fixed(Rgba::hex("#131313")),
            Role::ComposerStopMark => Derivation::Fixed(Rgba::hex("#f0f0f0")),
            Role::ButtonQuietFill => Derivation::Veil(Palette::Background, 0x00),
            Role::ButtonQuietFillHover => Derivation::Veil(Palette::Ink, 0x1f),
            Role::ButtonPrimaryFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.0425),
            Role::ButtonPrimaryFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0896)
            }
            Role::ButtonDangerFill => Derivation::Fixed(Rgba::hex("#a2543f")),
            Role::ButtonDangerFillHover => Derivation::Fixed(Rgba::hex("#b8614c")),
            Role::PanelSurface => Derivation::Mix(Palette::Ink, Palette::Background, 0.033),
            Role::PanelEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.1368),
            Role::PanelShadow => Derivation::Fixed(Rgba::hex("#00000080")),
            Role::PanelShadowNested => Derivation::Fixed(Rgba::hex("#00000060")),
            Role::PanelRowFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0708)
            }
            Role::PanelRowFillCurrent => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0896)
            }
            Role::SessionCardEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.1226),
            Role::SessionCardInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.9764),
            Role::SlashMenuRowInk => Derivation::Palette(Palette::QuietInk),
            Role::SlashMenuRowInkHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.8302)
            }
            Role::SlashMenuHintInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7925),
            Role::SlashMenuDescriptionInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786)
            }
            Role::ModelMenuHeaderInk => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.9387)
            }
            Role::ModelMenuHeaderInkQuiet => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786)
            }
            Role::ModelMenuCaretMark => Derivation::Mix(Palette::Ink, Palette::Background, 0.5),
            Role::ModelMenuCountInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.4465)
            }
            Role::ModelMenuRowInk => Derivation::Palette(Palette::QuietInk),
            Role::ModelMenuRowInkCurrent => Derivation::Palette(Palette::Ink),
            Role::ModelMenuCheckMark => Derivation::Fixed(Rgba::hex("#a18e41")),
            Role::ModelMenuDetailInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.4465)
            }
            Role::ModelMenuEffortInk => Derivation::Fixed(Rgba::hex("#a18e41")),
            Role::EffortLadderRowInk => Derivation::Palette(Palette::QuietInk),
            Role::EffortLadderRowInkCurrent => Derivation::Palette(Palette::Ink),
            Role::EffortLadderCheckMark => Derivation::Fixed(Rgba::hex("#a18e41")),
            Role::QueueCardCountInk => Derivation::Palette(Palette::QuietInk),
            Role::QueueCardLaneInk => Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786),
            Role::QueueRowInk => Derivation::Mix(Palette::Ink, Palette::Background, 0.9387),
            Role::QueueCardClearInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786)
            }
            Role::QueueCardClearInkHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.9387)
            }
            Role::QueueRowControlFill => Derivation::Mix(Palette::Ink, Palette::Background, 0.1368),
            Role::QueueRowControlFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.1934)
            }
            Role::QueueRowControlMark => Derivation::Palette(Palette::QuietInk),
            Role::QueueRowControlMarkHover => Derivation::Palette(Palette::Ink),
            Role::QueueRowPromoteMark => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.7925)
            }
            Role::QueueRowPromoteMarkHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.8868)
            }
            Role::QueueRowDeleteMark => Derivation::Fixed(Rgba::hex("#f3ded8")),
            Role::StatusStripSurface => Derivation::Mix(Palette::Ink, Palette::Background, 0.0377),
            Role::StatusStripEdge => Derivation::Mix(Palette::Ink, Palette::Background, 0.1368),
            Role::StatusStripWorkingInk => Derivation::Fixed(Rgba::hex("#c9a86a")),
            Role::StatusStripElapsedInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.4465)
            }
            Role::StatusStripQueueFill => Derivation::Veil(Palette::Background, 0x00),
            Role::StatusStripQueueFillHover => {
                Derivation::Mix(Palette::Ink, Palette::Background, 0.0708)
            }
            Role::StatusStripQueueCountInk => Derivation::Palette(Palette::QuietInk),
            Role::StatusStripQueueCountInkHover => Derivation::Palette(Palette::Ink),
            Role::StatusStripQueueLabelInk => {
                Derivation::Mix(Palette::QuietInk, Palette::Background, 0.5786)
            }
            Role::StatusStripQueueLabelInkHover => Derivation::Palette(Palette::QuietInk),
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
