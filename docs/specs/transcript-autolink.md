# Transcript autolink

## Purpose
Bare web addresses in transcript prose render as underlined, clickable links.

## Behavior
- `http(s)://` and `www.` URLs become Markdown links before markup escaping.
- Trailing sentence punctuation stays outside the link; balanced parentheses remain part of it.
- A `www.` address gets an `http://` href.
- Inline code spans and existing Markdown link labels/destinations are untouched.
- Quote rows use the same pass and have a click handler.
- Fenced code, tool output, thinking, and tables are literal text and are not autolinked.

## Acceptance
- Bare URL, `www.`, punctuation, code-span, and existing-link cases are covered by tests.
- The Markdown-link path still underlines and opens through Slint's `StyledText`.
- `cargo test -p pa-ui` passes.
