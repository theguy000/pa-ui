Status: Implemented in #42.

# Inline code accent

## Problem Statement

Inline code in the transcript is drawn with `TranscriptInlineCodeInk`, which
ships as a quiet mix (`#8f9088`). The status strip's busy word is drawn with
`StatusStripWorkingInk` (`#c9a86a`). Two accents that should say "the same
working language" instead read as unrelated colours.

## Solution

The inline-code role resolves from the status strip's working role when the
theme file does not name it. A file that names `transcript-inline-code-ink`
explicitly still wins. Both role names remain in the theme global and the file
schema.

## Requirements

- With the shipped theme, `TranscriptInlineCodeInk` equals
  `StatusStripWorkingInk`.
- A theme that changes `status-strip-working-ink` changes inline code with it.
- A theme that names `transcript-inline-code-ink` keeps that value.
- Inline code still uses `Role::TranscriptInlineCodeInk` in the renderer; no
  literal colour enters `markup.rs`.
- Existing inline-code escaping and Markdown behaviour are unchanged.

## Non-goals

- No TUI change.
- No new theme role name.
- No change to the status strip's own roles.

## Acceptance

- Theme test: shipped inline code equals the status working ink.
- Theme test: moving `status-strip-working-ink` moves inline code.
- Theme test: an explicit inline-code override wins.
- `cargo test -p pa-ui --bin pa-ui`: all tests pass.
- `cargo build --profile fast -p pa-ui`: passes.
