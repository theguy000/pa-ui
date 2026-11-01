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

/// How far apart the brand mark's tones have to be before the drawing still
/// reads as one cut mark rather than as a flat blob. The artwork is shading
/// rather than two meanings, so the floor is a step under the tags': the tones
/// it ships sit 16 apart at their closest, and a shadow that comes within 10 of
/// the shape it darkens has stopped shading it.
const APART_TONE: f32 = 10.0;

// The fields each part is read on, nearest last, named once because most of them
// are read on again and again.
const ON_WINDOW: &[Role] = &[];
const ON_FOLD_HOVER: &[Role] = &[Role::TitleBarFoldFillHover];
const ON_CLOSE_HOVER: &[Role] = &[Role::TitleBarCloseFillHover];
const ON_SIDEBAR: &[Role] = &[Role::SidebarSurface];
const ON_SIDEBAR_ROW: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFill];
const ON_SIDEBAR_ROW_HOVER: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFillHover];
const ON_SIDEBAR_ROW_SELECTED: &[Role] = &[Role::SidebarSurface, Role::SidebarRowFillSelected];
// The sidebar's controls, and the fields they paint: a quiet chip that only has
// a field under the pointer, the button that carries New Session, the chip a row
// offers to rename it, and the pill the folded column shows.
const ON_QUIET_CONTROL: &[Role] = &[Role::ButtonQuietFillHover];
const ON_PRIMARY_CONTROL: &[Role] = &[Role::ButtonPrimaryFill];
const ON_PRIMARY_CONTROL_HOVER: &[Role] = &[Role::ButtonPrimaryFillHover];
const ON_RENAME_CHIP: &[Role] = &[Role::SidebarRowRenameFill];
const ON_RENAME_CHIP_HOVER: &[Role] = &[Role::SidebarRowRenameFillHover];
const ON_OPEN_TIP: &[Role] = &[Role::SidebarOpenTipFill];
const ON_TRANSCRIPT: &[Role] = &[Role::TranscriptSurface];
const ON_MESSAGE: &[Role] = &[Role::TranscriptSurface, Role::TranscriptMessageFill];
const ON_CELL: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFill];
const ON_CELL_HOVER: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFillHover];
const ON_FAILED_CELL: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCellFillFailed];
const ON_FAILED_CELL_HOVER: &[Role] =
    &[Role::TranscriptSurface, Role::TranscriptCellFillFailedHover];
const ON_OPENED_ROW: &[Role] = &[Role::TranscriptSurface, Role::TranscriptRowFillExpanded];
const ON_CODE_CARD: &[Role] = &[Role::TranscriptSurface, Role::TranscriptCodeFill];
// The chip that walks back to the newest row, and the badge a failed cell
// carries: a mark drawn on a fill that is itself laid on a field.
const ON_JUMP: &[Role] = &[Role::TranscriptSurface, Role::TranscriptJumpFill];
const ON_BADGE: &[Role] = &[
    Role::TranscriptSurface,
    Role::TranscriptCellFillFailed,
    Role::TranscriptCellBadgeFill,
];
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
const ON_STOP_FILL: &[Role] = &[Role::ComposerSurface, Role::ButtonDangerFill];
// The four panels that open over the window, each word read on the panel it is
// drawn on and — where a row carries it — on the fill that row takes.
const ON_PANEL: &[Role] = &[Role::PanelSurface];
const ON_PANEL_ROW: &[Role] = &[Role::PanelSurface, Role::PanelRowFillHover];
const ON_PANEL_ROW_CURRENT: &[Role] = &[Role::PanelSurface, Role::PanelRowFillCurrent];
// A waiting row's controls: a fill laid on the panel, and a mark drawn on that
// fill rather than on the panel.
const ON_ROW_CONTROL: &[Role] = &[Role::PanelSurface, Role::QueueRowControlFill];
const ON_ROW_CONTROL_HOVER: &[Role] = &[Role::PanelSurface, Role::QueueRowControlFillHover];
// The strip, and the pill the queue is opened from: one field laid on another.
const ON_STRIP: &[Role] = &[Role::StatusStripSurface];
const ON_STRIP_PILL: &[Role] = &[Role::StatusStripSurface, Role::StatusStripQueueFillHover];

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
    // The column's controls draw a mark and nothing else, so the mark is read on
    // the column itself: there is no field between them. The strip's pen is the
    // same story — its chip is transparent until the pointer arrives, so folded
    // the mark rests on the column too.
    Pair {
        ink: Mark { role: Role::SidebarFoldGlyph, on: ON_SIDEBAR },
        what: "the sidebar's fold mark at rest",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarSearchMark, on: ON_SIDEBAR },
        what: "the sidebar's magnifier",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarStripNewSessionMark, on: ON_SIDEBAR },
        what: "the pen on the folded strip",
        floor: MARK,
    },
    // The mark's two lit tones are what says it is there at all, so both are read
    // on the column it is cut from. Its shadows are not offered to this floor —
    // a shadow drawn as bright as the field would not be a shadow — and the note
    // under these pairs says what is measured instead.
    Pair {
        ink: Mark { role: Role::BrandMarkBlade, on: ON_SIDEBAR },
        what: "the brand mark's blade",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::BrandMarkWing, on: ON_SIDEBAR },
        what: "the brand mark's wing",
        floor: MARK,
    },
    // The column's controls paint a field of their own, so a mark that sits on
    // one is read on it rather than on the column: the chip that lights under the
    // pointer, the button that carries New Session, the corner chip a row offers
    // to rename it, and the pill the folded column shows. The two filled buttons
    // say what they do in words as well, and words are body text on the field
    // they are drawn on.
    Pair {
        ink: Mark { role: Role::SidebarFoldGlyphHover, on: ON_QUIET_CONTROL },
        what: "the fold mark on its chip under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarStripNewSessionMarkHover, on: ON_QUIET_CONTROL },
        what: "the pen on the folded strip's chip under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarNewSessionMark, on: ON_PRIMARY_CONTROL },
        what: "the pen on New Session at rest",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarNewSessionMarkHover, on: ON_PRIMARY_CONTROL_HOVER },
        what: "the pen on New Session under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarNewSessionInk, on: ON_PRIMARY_CONTROL },
        what: "the words on New Session at rest",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SidebarNewSessionInkHover, on: ON_PRIMARY_CONTROL_HOVER },
        what: "the words on New Session under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SidebarRowRenameMark, on: ON_RENAME_CHIP },
        what: "the pencil a row offers, at rest",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarRowRenameMark, on: ON_RENAME_CHIP_HOVER },
        what: "the pencil a row offers, with the pointer on its chip",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::SidebarOpenTipInk, on: ON_OPEN_TIP },
        what: "the words on the pill that opens the column",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SessionCardInk, on: ON_PANEL },
        what: "the words on the card that names a session",
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
    // The caret beside that name is the whole of what says the row opens, so it is
    // read on the cell — and, in a thinking row, on the sheet the row runs across,
    // because a resting thinking row draws no fill of its own.
    Pair {
        ink: Mark { role: Role::TranscriptCaretMark, on: ON_CELL },
        what: "the caret on a cell",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::TranscriptCaretMark, on: ON_TRANSCRIPT },
        what: "the caret on a thinking row",
        floor: MARK,
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
    // The badge is opened for what the alert inside it says, and the alert is a
    // mark on the badge's own red rather than on the cell's field.
    Pair {
        ink: Mark { role: Role::TranscriptCellBadgeMark, on: ON_BADGE },
        what: "the alert inside a failed cell's badge",
        floor: MARK,
    },
    // The chip that walks the reader back to the newest row floats over the sheet
    // on a fill of its own; the chevron is the whole of what says it goes down.
    Pair {
        ink: Mark { role: Role::TranscriptJumpMark, on: ON_JUMP },
        what: "the chevron that walks back to the newest row",
        floor: MARK,
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
        ink: Mark { role: Role::ComposerChipCaretMark, on: ON_CHIP },
        what: "the caret on the composer's chip",
        floor: MARK,
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
        ink: Mark { role: Role::ButtonDangerFill, on: ON_COMPOSER },
        what: "the stop disc on the composer's field",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ComposerSendMark, on: ON_SEND_FILL },
        what: "the arrow on the send disc",
        floor: MARK,
    },
    // The four panels that open over the window. A command's name and a model's
    // name are what the reader is choosing between, so both are held to the body
    // floor in every state a row is read in; the sentences, labels and figures
    // around them are held to the mark floor, because they are read only after
    // the name they qualify has been found.
    Pair {
        ink: Mark { role: Role::SlashMenuRowInk, on: ON_PANEL },
        what: "a command's name",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SlashMenuRowInkHover, on: ON_PANEL_ROW },
        what: "the command the pointer or the arrows are on",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SlashMenuHintInk, on: ON_PANEL_ROW },
        what: "the argument a command takes",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::SlashMenuDescriptionInk, on: ON_PANEL_ROW },
        what: "what a command does",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::ModelMenuRowInk, on: ON_PANEL },
        what: "a model's name at rest",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ModelMenuRowInkCurrent, on: ON_PANEL_ROW_CURRENT },
        what: "the running model's name",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ModelMenuEffortInk, on: ON_PANEL_ROW_CURRENT },
        what: "the level the running model is at",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ModelMenuHeaderInk, on: ON_PANEL },
        what: "a provider's name",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::ModelMenuHeaderInkQuiet, on: ON_PANEL },
        what: "a provider the reader is not signed in to",
        floor: MARK,
    },
    // The caret that opens a provider's models is drawn beside its header rather
    // than on a fill of its own.
    Pair {
        ink: Mark { role: Role::ModelMenuCaretMark, on: ON_PANEL },
        what: "the caret in front of a provider's name",
        floor: MARK,
    },
    // The two panels that mark the running model draw it twice — a name in the
    // body ink and a tick in the gold the window marks what is live with — and
    // the tick is the half a theme is likeliest to leave behind.
    Pair {
        ink: Mark { role: Role::ModelMenuCheckMark, on: ON_PANEL_ROW_CURRENT },
        what: "the tick beside the running model",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::EffortLadderCheckMark, on: ON_PANEL_ROW },
        what: "the tick beside the rung the model is on",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::EffortLadderRowInk, on: ON_PANEL },
        what: "a rung of the ladder",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::EffortLadderRowInkCurrent, on: ON_PANEL_ROW },
        what: "the rung the model is on",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::QueueCardCountInk, on: ON_PANEL },
        what: "how many messages are waiting",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::QueueCardLaneInk, on: ON_PANEL },
        what: "the name of a lane with nothing to count",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueRowInk, on: ON_PANEL_ROW },
        what: "a waiting message",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::QueueCardClearInk, on: ON_PANEL },
        what: "the action that empties both lanes",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueCardClearInkHover, on: ON_PANEL_ROW },
        what: "the action that empties both lanes, under the pointer",
        floor: BODY,
    },
    // A waiting row's three controls are marks on a fill of their own, which is
    // the only thing that says they are controls: the row draws no outline.
    Pair {
        ink: Mark { role: Role::QueueRowControlMark, on: ON_ROW_CONTROL },
        what: "the pencil and the bin on a waiting row",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueRowControlMarkHover, on: ON_ROW_CONTROL_HOVER },
        what: "the pencil on a waiting row, under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueRowPromoteMark, on: ON_ROW_CONTROL },
        what: "the arrow that pulls a message into the running turn",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueRowPromoteMarkHover, on: ON_ROW_CONTROL_HOVER },
        what: "that arrow under the pointer",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::QueueRowDeleteMark, on: ON_ROW_CONTROL_HOVER },
        what: "the bin on a waiting row, under the pointer",
        floor: MARK,
    },
    // The strip. The word that says the turn is working and the count the queue
    // pill carries are the two things a reader glances at, so both are held to
    // the body floor on the strip and on the pill when it is lit; the word
    // beside the count names what is counted and is held to the mark floor.
    Pair {
        ink: Mark { role: Role::StatusStripWorkingInk, on: ON_STRIP },
        what: "the word that says what the turn is doing",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::StatusStripQueueCountInk, on: ON_STRIP },
        what: "how many messages the pill counts",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::StatusStripQueueCountInkHover, on: ON_STRIP_PILL },
        what: "how many messages the pill counts, under the pointer",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::StatusStripQueueLabelInk, on: ON_STRIP },
        what: "the word beside the count on the pill",
        floor: MARK,
    },
    Pair {
        ink: Mark { role: Role::StatusStripQueueLabelInkHover, on: ON_STRIP_PILL },
        what: "the word beside the count, under the pointer",
        floor: MARK,
    },
    // The fill stays lit while the card is open and the words do not, so the two
    // resting inks are read on it as well: the pointer leaving a pill whose card
    // is still open is the one state where a theme can bury the count without
    // touching a colour either of the pairs above has seen.
    Pair {
        ink: Mark { role: Role::StatusStripQueueCountInk, on: ON_STRIP_PILL },
        what: "how many messages the pill counts, with its card open",
        floor: BODY,
    },
    Pair {
        ink: Mark { role: Role::StatusStripQueueLabelInk, on: ON_STRIP_PILL },
        what: "the word beside the count, with the pill's card open",
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
    //   - a provider's count and a model's context window sit at 2.5:1 on the
    //     panel they are drawn on, and the strip's elapsed time at 2.5:1 on the
    //     strip: the three figures the window draws to recede behind the words
    //     they qualify.
    //   - the failure badge is a disc of the failure's red on the failed cell's
    //     field, 1.9:1, and the jump chip's edge is 1.5:1 on the column. One is
    //     a mark with a glyph drawn inside it and the other a line whose whole
    //     job is to say "control": neither is a word.
    // The transcript's other hairlines and its scrollbar's thumb are no ink on a
    // surface at all, so no floor of this kind describes them.
    //
    // And one thing no floor here can see: the failure's six colours are fixed
    // values, so a theme that moves the field and the ink and stops there keeps a
    // failed cell drawn for a dark window. The separation below still holds — a
    // near-black cell is a long way from a light one — so nothing refuses it;
    // naming the failure roles is how a theme makes its own.
    //
    // The brand mark is artwork rather than ink, and only its blade and its wing
    // are held to a floor on the column (18.9:1 and 8.5:1): its shadows sit at
    // 2.7:1 and 1.5:1, which is what a shadow is. What has to survive is the
    // drawing, and that is measured as distance, below.
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
        b: Mark { role: Role::ButtonDangerFill, on: ON_COMPOSER },
        what: "the send disc and the stop disc",
        floor: APART,
    },
    // A waiting row's three controls are told from the row by their fill alone:
    // they draw no outline, and the code that draws them says why — the fill
    // sits a clear step above both the panel and the row's own hover. A theme
    // that tints the panel and leaves the fills behind turns three buttons into
    // three patches of nothing without moving a word. The tint floor is the
    // failed cell's, because a control is a field rather than a mark.
    Telling {
        a: Mark { role: Role::QueueRowControlFill, on: ON_PANEL },
        b: Mark { role: Role::PanelSurface, on: ON_WINDOW },
        what: "a waiting row's buttons and the panel behind them",
        floor: APART_TINT,
    },
    Telling {
        a: Mark { role: Role::QueueRowControlFill, on: ON_PANEL_ROW },
        b: Mark { role: Role::PanelRowFillHover, on: ON_PANEL },
        what: "a waiting row's buttons and the row they sit on",
        floor: APART_TINT,
    },
    // A panel's rows are told from the panel by their fill alone — a resting row
    // draws nothing of its own, so the highlight is the whole of what says which
    // row the pointer or the arrows are on, and the fill is the whole of what
    // says which model the session runs. A theme that moves the panel surface
    // and leaves those fills at the value they were drawn at is the theme this
    // ticket promises will work; one that paints the two alike has taken every
    // menu's highlight away at once and moved no word to say so.
    Telling {
        a: Mark { role: Role::PanelRowFillHover, on: ON_PANEL },
        b: Mark { role: Role::PanelSurface, on: ON_WINDOW },
        what: "the row the pointer is on and the panel behind it",
        floor: APART_TINT,
    },
    Telling {
        a: Mark { role: Role::PanelRowFillCurrent, on: ON_PANEL },
        b: Mark { role: Role::PanelSurface, on: ON_WINDOW },
        what: "the row a panel marks as current and the panel behind it",
        floor: APART_TINT,
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
    // The brand mark is one drawing in six tones, and each tone is there to stand
    // off the shape it is laid on: the blade off the wing, the stem's shadow off
    // the wing, the blade's shadow off the wing and off the stem. A theme that
    // turns the window over turns all six with it, because five of them are made
    // of the brand tone and the field; a theme that turns the field and leaves the
    // brand tone behind is the one this set exists to catch, and it catches it as
    // four findings rather than as a mark nobody can read. The floor is the tones'
    // own because these are four readings of one artwork, not four promises.
    Telling {
        a: Mark { role: Role::BrandMarkBlade, on: ON_SIDEBAR },
        b: Mark { role: Role::BrandMarkWing, on: ON_SIDEBAR },
        what: "the mark's blade and the wing under it",
        floor: APART_TONE,
    },
    Telling {
        a: Mark { role: Role::BrandMarkWingInnerShadow, on: ON_SIDEBAR },
        b: Mark { role: Role::BrandMarkWing, on: ON_SIDEBAR },
        what: "the mark's stem shadow and the wing it darkens",
        floor: APART_TONE,
    },
    Telling {
        a: Mark { role: Role::BrandMarkWingShadow, on: ON_SIDEBAR },
        b: Mark { role: Role::BrandMarkWing, on: ON_SIDEBAR },
        what: "the mark's blade shadow and the wing it darkens",
        floor: APART_TONE,
    },
    Telling {
        a: Mark { role: Role::BrandMarkStemShadow, on: ON_SIDEBAR },
        b: Mark { role: Role::BrandMarkStem, on: ON_SIDEBAR },
        what: "the mark's blade shadow and the stem it darkens",
        floor: APART_TONE,
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
