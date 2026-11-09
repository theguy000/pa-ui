# Quote blocks match the TUI

Status: Implemented in #59.

## Problem

`markdown_markup` strips `> ` from a line and writes `›` in prose ink. The body stays
plain, a wrapped line loses the marker, and the reader sees what looks like unrendered
Markdown. The TUI draws every quoted line with a `│ ` border, muted italic body, and
inline code in its own colour.

## Behaviour

A blockquote becomes its own transcript row:

- one continuous 2px quote bar on the left, in quote-edge ink
- quoted body in quote ink, italic
- inline code, emphasis, and links keep their own styling
- no literal `>` or `›` reaches the screen
- the bar spans the quote text box, including wrapped lines

## Parsing

`split_blocks` recognises a quote before table detection:

- a quote line is optional leading spaces, `>`, then an optional one space
- consecutive quote lines join one quote block
- a plain paragraph line without `>` continues the quote (lazy continuation)
- a heading, rule, list, fence, table, or blank line ends the quote
- leading empty quote lines are dropped; an all-empty quote is dropped

A quote block is `Block::Quote(body)`. `transcript/mod.rs` turns it into
`KIND_QUOTE`; the body goes through `quote_markup` without the `>` markers.

`markdown_markup` keeps a direct-call path: a `> ` line renders a `│ ` bar in
quote-edge ink and the body in quote ink, so a caller that bypasses `split_blocks`
never shows a raw marker.

## Rendering

`pa-ui/ui/transcript.slint` draws `KIND_QUOTE` as a bar plus a `StyledText`:

- bar: x 24px, 2px wide, y 6px, height row minus 12px, rounded 1px
- text: x 36px, width parent minus 60px, italic from `quote_markup`
- row height: text preferred height plus 12px

New theme roles:

- `transcript-quote-ink`: `Mix(QuietInk, Background, 0.80)`
- `transcript-quote-edge`: `Mix(QuietInk, Background, 0.35)`

Both are added to `Role::ALL`, the theme names, and `ui/theme.slint`.

## Tests

- `split_blocks` keeps quote lines, lazy continuations, and blank-line endings
- a quote row is `KIND_QUOTE` and its body has no `>` marker
- transcript markup puts no `>` or `›` in quote output
- the shipped theme gives quote ink and edge their derived values
- a quote row uses its narrower sheet estimate
- a theme override moves each role independently
- mutation: removing quote parsing or the bar role fails a test

## Non-goals

- nested quote depths beyond one visible bar
- custom-message and summary bodies keep the weaker inline quote path
- changing TUI quote rendering
- changing daemon message content
