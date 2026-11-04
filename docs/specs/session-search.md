# Session search: find one session by its title

## Problem Statement

The sidebar lists every session in the order the daemon returns them, under a
filter that matches on a substring of the name. A reader with forty sessions
scrolls. The column also only shows the rows that fit in it, and the filter sits
inside the column's header where it competes with the header's own controls.

Finding a session the reader half remembers the name of takes scrolling, then
reading every row, then maybe giving up and opening a few.

## Solution

A popup over the middle of the window, reached from the magnifier in the
sidebar's header or with Ctrl+K. It lists every session in full, matches titles
as the reader types so that letters found in order with gaps allowed still count
(`msc` finds "Mefenamic Acid for Period Pain"), sorts the tightest match first,
and opens the session on Return or a click. Escape, a click outside the panel,
the panel's cross, or picking a session closes it, and the caret goes back to the
composer.

The popup is not a wider sidebar filter. It draws its own rows, takes its list
once at open so the two-second poll cannot reorder it under the arrow keys, and
marks the letters a query matched in the row, so a loose match still says why the
row is there.

## User Stories

1. As a reader, I want to open a search from anywhere in the window with Ctrl+K, so that I do not have to find the sidebar first.
2. As a reader, I want the magnifier in the sidebar's header to open the same search, so that the mouse has a way in.
3. As a reader, I want to search with letters in order rather than an exact substring, so that a query I half remember still finds the session.
4. As a reader, I want the tightest match at the top, so that the session I meant is the one under the first Return.
5. As a reader, I want no matching session dropped for scoring low, so that a wide match is still findable.
6. As a reader, I want the letters I typed marked in the row, so that I can see why each row survived.
7. As a reader, I want an empty field to list every session, so that the popup works as a plain list of sessions.
8. As a reader, I want to see when each session was last used, so that two similarly named sessions can be told apart.
9. As a reader, I want Up and Down to move the highlight and wrap at either end, so that I can reach both ends without guessing.
10. As a reader, I want Return to open the highlighted session, so that the common case never needs the mouse.
11. As a reader, I want Escape to close the popup without stopping a running turn, so that dismissing a search costs nothing.
12. As a reader, I want the caret to leave the popup when it closes, so that the next keystroke goes to the composer.
13. As a reader, I want picking a session to close any open menu or queue as well, so that nothing is left open over the session I chose.
14. As a reader, I want the list not to reorder while I am arrowing through it, so that a poll never moves the row under my finger.
15. As a reader, I want the popup centred and clear of the title bar, so that it reads as an overlay rather than as part of a column.
16. As a reader, I want a narrow window to keep the whole panel on screen, so that no column of it has dead clicks.
17. As a reader, I want the popup to take its colours from the theme, so that it matches the window in every theme.
18. As a theme author, I want the popup's inks declared as roles with contrast pairs, so that a theme that hides a row is reported.
19. As a contributor, I want the matching, eliding and glyph splitting in one testable module, so that the rules can be changed without launching the window.
20. As a contributor, I want a snapshot hook that opens the popup with a query and seeded rows, so that the popup can be looked at on a machine whose daemon has no sessions.

## Implementation Decisions

- Matching lives in `pa-ui/src/search/mod.rs`, which has no Slint code: `matches` returns each session with the character offsets the query hit, `elide` cuts a name to the row's room, and `glyphs` turns a name and offsets into the per-character list the row draws. The window's callbacks are in `pa-ui/src/interactive/search.rs`.
- The query matches the session's full title, not the sidebar's 80-character cut, so a match past that cut still finds the session. The row draws `elide(display_name(row))`: 46 characters plus an ellipsis. Offsets still point at the drawn characters because the cut only ever removes the tail.
- Score is the width of the matched span, narrower first, with a three-point bonus when the match starts a word and a small tie-break on name length. The match starts at the first occurrence of the query's first letter. Equal scores keep the daemon's order through a stable sort.
- A row is a list of `SearchGlyph { text, matched }` rather than one `Text`, because Slint cannot colour a range of a string without styling all of it. `MatchedName` in `ui/search-name.slint` lays the glyphs out.
- `SearchPopup` in `ui/search.slint` has no placeholder property on its `TextInput`: a `Text` drawn under the field renders the placeholder until the field is typed into.
- Placement: the panel is 400 wide and 45px tall plus up to eight 32px rows plus 8px, centred on both axes and never closer than 48px to the top edge or past the window's left edge. Slint cannot read an element's size in `changed`, so the window reads a sibling rectangle placed from the panel's own `x`, `y`, `width` and `height`, and `changed width` / `changed height` on that rectangle set the root's `search-x` and `search-y`. The panel and the four strips around it that close it on a click read the same two numbers.
- The panel carries a transparent `TouchArea` over itself, so a click on the panel never reaches the rows beneath the pointer and steals the highlight from the arrows.
- `Ctrl+K` is handled by `root.search-key(event)`, called from the transcript's key handler and the composer's. The toolkit reports the chord as `\u{b}`, and the function accepts that, `k` and `K`.
- Opening reads `ui.get_search_query()` before setting `search_open`. Rust sets `search-entries` and `search-index` and then `search_open`; nothing writes `search-query` from Rust while the popup is open, because writing it from inside the `edited` callback would stop the field after its first character.
- `picked` sets `selected-id` and clears `model-menu` and `queue-open`; `picked` and `closed` both call `focus-composer()`.
- Theme roles `search-row-ink`, `search-row-ink-highlighted`, `search-match-ink`, `search-age-ink` are added to `Role::ALL` (now 194) and to the ink-on-surface pairs in `pa-ui/src/theme/contrast.rs`. `search-match-ink` is fixed at `#e0cd7c`; `search-age-ink` is `quiet-ink` mixed toward the background at 0.5786. The magnifier, in the column's header and in the folded strip, takes the existing `sidebar-search-mark-hover` under the pointer.
- The age column reads `last_activity`, or `modified` when the row has none. `sessions::age_label` turns the stamp into seconds with `stamp_seconds` and `days_from_civil`, kept in `pa-ui/src/sessions.rs` because no other part of the window formats a date any more.

## Testing Decisions

- The matcher is tested only through `matches`, `elide` and `glyphs`, which are the module's public surface. Tests cover an empty and a whitespace query, subsequence matching, order not running backwards, case folding, a snug match sorting before a scattered one, the offsets pointing at the characters hit, a titleless session matching the name the sidebar gives it, a short name drawn whole, a long one cut with an ellipsis, and a match past the cut still finding the session.
- `interactive::search` has a test that a titleless session's row carries as many glyphs as the sidebar's name has characters.
- The theme suite's shipped-theme run covers the new roles: a shipped theme that fails a pair's floor fails `the_shipped_theme_clears_every_floor_it_declares`.
- What is not tested automatically: the popup's layout, its keys and its focus. These were checked in snapshots via `PA_SNAPSHOT`.

## Out of Scope

- Matching message bodies. A query only looks at titles.
- Pinning, grouping or filtering by project inside the popup.
- Replacing the sidebar's own filter.
- A scrim behind the popup, and animation on open or close.

## Further Notes

Three environment variables exist for looking at the popup without a daemon, and all are read only when `PA_SNAPSHOT` is also set:

- `PA_SEARCH`: open the popup with this query already in the field. An empty value opens it on the whole list.
- `PA_SEARCH_SEED`: seed the sidebar and the popup with `|`-separated titles.
- `PA_SNAPSHOT`: write the window as a raw frame, width and height as little-endian `i32` followed by RGBA.

Run with `PA_SNAPSHOT=<path> PA_SEARCH=m PA_SEARCH_SEED="Mixing the master|Music and Sound Cycles" PA_THEME=mono`.
