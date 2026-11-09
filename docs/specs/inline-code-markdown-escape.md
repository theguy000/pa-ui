Status: Implemented in #42.

# Inline code with Markdown markers

## Problem Statement

The desktop transcript sends prose through `markup.rs::markdown_markup` and
then through Slint's `StyledText::from_markdown`. Inline code is lifted out of
the Markdown first: `highlight_inline_code` replaces a backtick span with bold,
theme-coloured text.

The code body is inserted raw. If it contains Markdown metacharacters, Slint
reads them as markup. A span containing `**` emits:

```text
- **<font color="#8f9088">**</font>** spans ...
```

`balance_strong` then counts the code's `**` as emphasis and appends closing
markers across following lines. Slint rejects the row with:

```text
HTML tag </font> overlaps with markdown formatting
```

`styled()` falls back to `StyledText::from_plain_text`, so the reader sees the
generated `**` and `<font>` tags literally for the whole row. Inline code
containing `*`, `_`, `[`, `]`, a backtick, or a backslash can corrupt the row
the same way.

## Solution

Add a Markdown-only escape for the inline code body. It backslash-escapes the
characters that open or close Markdown constructs, then the existing
`**<font>...</font>**` wrapper is emitted around the escaped text. The escape
must not run `escape_html` again: the body already carries HTML entities from
the prose pass, and re-escaping would display `&lt;` literally.

## Requirements

- Inline code renders its body literally.
- Markdown markers inside a code span do not affect emphasis, links, or
  `balance_strong` on the surrounding row.
- A code span containing `**`, `*`, `_`, `[`, `]`, backtick, backslash, or `~`
  produces markup Slint parses.
- Raw `<`, `>`, and `&` from the model remain text; the probe text
  `<font color="#8f9088">` stays literal.
- Existing inline-code colour, bold weight, and theme roles are unchanged.
- Rows that are genuinely malformed still take the plain-text fallback.

## Non-goals

- No CommonMark parser or renderer replacement.
- No TUI change; its inline parser already treats code bodies literally.
- No new visual treatment for inline code.

## Acceptance

- Regression test: a prose row containing the code span `**` passes
  `StyledText::from_markdown` and the code body is escaped in the emitted
  markup.
- Regression test: code spans with the other Markdown markers parse.
- `cargo test -p pa-ui --bin pa-ui`: 240 passed, 0 failed, 4 ignored.
- `cargo build --profile fast -p pa-ui`: passes.
- Mutation: removing the body escape makes the inline-code parse test fail.
