# Sheet error row

## Problem Statement

An operation can fail without a Row that says so. Worker errors currently travel as `Msg::Activity`, the same channel as `Thinking` and `Writing`, so the Status strip can replace the Turn's activity word with an error — or hide the error behind a held Command word. A Turn failure can also carry its reason in the assistant message's `errorMessage`, while `/compact` reports through `compaction_end`; neither had a Sheet path.

The TUI already has the right shape: `SessionUi::error_row` appends one `ChatEntry::Status` row, `⚠ Error: <message>`, in the error colour. Multi-line errors collapse to one summary line. The window needs the same plain, generic Row in its own visual language.

## Solution

Add one `KIND_ERROR` Row to the Sheet. It draws a soft-triangle warning icon, the label `Error:`, and one plain message. Long messages wrap; a multi-line error keeps its first non-empty line.

Worker errors, prompt failures, `turn_end` failures, and `compaction_end` failures all enter this one row. The Status strip, Command word, Composer, and Turn lifecycle stay independent. The row is not a card, toast, modal, or action surface.

## Requirements

- One generic error Row, in the Sheet, for every operation failure.
- The row carries the selected soft-triangle icon, an `Error:` label, and the message.
- Worker errors carry their Session target and do not enter another Session's Sheet.
- A provider failure is read from `turn_end.message.errorMessage` when the message's `stopReason` is `error`.
- A `/compact` failure is read from `compaction_end` (`errorSeverity: error`) and reads `Compacting failed: …`.
- A command Turn failure reads `Refining failed: …` or `Compacting failed: …` when that Command word is held.
- A deliberate Stop is not an error row; its aborted assistant message already records it.
- The prompt take-back removes the reader's echo before the error row is appended.
- Long text wraps in the Sheet; the row stays one transcript entry.
- The Status strip does not show errors and does not change for an error row.

## Non-goals

- No Store write, protocol change, or Session format change.
- No per-Session retention for an error that arrives after the Reader switched away; that needs a separate transient-notice store.
- No retry button or other action in the error row.

## Acceptance

- `cargo test -p pa-ui`: 219 passed; 0 failed; 3 ignored.
- `cargo build --profile fast -p pa-ui`: passes.
- The generic row is covered with an open live Message, a cross-Session error, a provider failure, an abort, and a compaction failure.
