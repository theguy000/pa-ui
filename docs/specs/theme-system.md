# Theme system: one switchable surface for colour and components

## Problem Statement

The window has no idea what its colours mean. Ninety distinct colour literals are written by hand at roughly 180 call sites across ten UI sources: the same panel grey appears in seven files, primary ink in twenty places, hover fills in eight. Nothing declares that `#2c2f26` is a hairline or that `#a18e41` is the accent, so there is nowhere to say "these are the window's surfaces" — and two consumers keep their own private palettes entirely outside the UI language (the composer's canvas text and the transcript's markup).

The reader who wants a second look therefore has to find every literal by hand, with no way to know one was missed, no way to see the result without rebuilding, and no way to be told that the new scheme left the sidebar rows unreadable.

## Solution

The window names its colours once, as a small set of roles with meaning (`transcript-surface`, `composer-send-fill-hover`, `danger-ink`, `tag-working`), and every component asks for a role by name. A theme is a short file that sets the handful of base values; the rest resolves from them, and any role can be overridden by name. The default theme ships embedded, so the window looks the same on a machine with no theme file at all.

Selecting a theme is one setting. Wrong names are refused with a message that says what the file said and what was legal, an unreadable colour combination is reported, and the window still opens — on the default theme — rather than ever showing a broken surface.

Two themes ship as evidence the system works: the current look, and a monochrome one. The monochrome pass already running in the working tree becomes that second theme instead of an unmaintainable edit.

## User Stories

1. As a reader, I want the window to look exactly as it does today after the colour roles land, so that a refactor I cannot see does not cost me the design I chose.
2. As a reader, I want to choose a theme, so that the window matches the rest of my desktop.
3. As a reader, I want the window to open on the default theme when my theme file is missing or malformed, so that a typo never leaves me with an unusable window.
4. As a reader, I want warnings, failures and stopped turns to stay recognisable in every theme, so that a danger colour I have learned never becomes indistinguishable from a surface.
5. As a reader, I want the kind of a tag — a working session, a parked one, an idle one, the running model's effort — to stay distinguishable in every theme, so that status does not depend on noticing a hue I cannot see.
6. As a reader, I want a light theme to be possible at all, so that working in daylight is not limited to a dark window.
7. As a reader, I want the theme to cover what the app draws itself, including code syntax colour in the transcript and the text the composer draws straight to a canvas, so that no corner of the window stays on the old palette.
8. As a reader, I want the app's glyphs to take their colour from the theme, so that icons do not glow at the wrong weight against a new background.
9. As a reader, I want the brand mark to remain recognisable, so that its artwork tones are theme roles rather than fixed values that fight the chrome.
10. As a theme author, I want a theme to be one small file, so that I can write one in a sitting.
11. As a theme author, I want to set only the base values and let the rest resolve, so that a coherent theme takes a dozen lines rather than a hundred.
12. As a theme author, I want to override any single role by name, so that the one surface that looks wrong with my base values can be fixed without touching the others.
13. As a theme author, I want a misspelled role name to be refused with the name I used and the name I probably meant, so that a silent typo does not ship.
14. As a theme author, I want to be told when body text on a surface reads below a legibility floor, so that I find out before the reader does.
15. As a theme author, I want a way to check a theme without launching the window, so that I can iterate from a script.
16. As a theme author, I want the checked-in themes to be checked the same way on every build, so that a theme cannot rot unnoticed.
17. As a theme author, I want a worked second theme to copy from, so that I am not inventing the file shape from prose.
18. As a contributor, I want exactly one place that holds colour values, so that I never have to search for where a colour came from.
19. As a contributor, I want the build to fail when I paste a colour literal into a component, so that the single-owner rule holds without anyone policing it.
20. As a contributor, I want the build to fail when a component asks for a role that does not exist, so that a rename cannot half-land.
21. As a contributor, I want adding a role to be one edit in one table plus its consumers, so that a new state costs minutes.
22. As a contributor, I want to add a component that only talks in roles and variant names, so that a new surface does not leak a palette.
23. As a contributor, I want repeated button, panel and tag shapes to be one component each with a variant switch, so that a theme change reaches every one of them at once.
24. As a contributor, I want the two Rust-side colour consumers to read roles like everything else, so that "the theme" means one thing.
25. As a contributor, I want a renamed role to break the build rather than the rendering, so that refactors are safe.
26. As a contributor, I want the role table to carry the meaning of each role in a comment, so that the next person knows whether `line-quiet` is a border or a divider.
27. As a reviewer, I want the diff for the colour extraction to be mechanical, so that "nothing changed visually" is checkable by looking at the screenshots rather than the whole file.
28. As a reviewer, I want the monochrome look expressed as a theme file, so that the review of that experiment is a file diff and not a hundred scattered literals.
29. As a maintainer, I want a screenshot of the default theme before and after the extraction, so that the claim of no visual change is evidence and not a promise.
30. As a maintainer, I want the theme layer to stay out of layout, spacing and type, so that the change stays reviewable and the next layer can copy its shape.
31. As a maintainer, I want an unusable theme to fail loudly in the log and quietly in the window, so that the app never renders a half-applied palette or an error banner.
32. As a future theme author, I want the resolved theme applied through a single entry point, so that live switching can be added later without touching components.

## Implementation Decisions

**Three layers, resolved once at startup.**

- *Palette*: the dozen or so values an author sets — window background, surface, raised surface, ink, quiet ink, line, accent, danger, success, warning, plus the two or three brand tones. Required by name; missing entries inherit the default theme.
- *Roles*: the semantic names components use. Each role has either a derivation from the palette (a fixed mix of two palette entries, or a palette entry at a fixed alpha) or an explicit default value. A theme file may override any role by name, and an override always wins over its derivation.
- *Components*: reference roles only. No colour arithmetic in the UI layer — no lighten, no blend, no alpha maths. Every state a component can be in has a role, so the theme is the only place that decides what a hovered danger button looks like.

**Vocabulary.** Role names are built from the glossary's own words for the window's parts — `sidebar`, `composer`, `transcript`, `row`, `cell`, `lane`, `status-strip`, `title-bar` — followed by the part and then the property, with states as a suffix: `sidebar-row-fill-hover`, `cell-surface-failed`, `composer-send-fill`, `tag-working`. The generic families (`ink`, `surface`, `line`, `accent`, `danger`, `ok`, `warn`, `brand`) are reserved for values that genuinely apply everywhere. Names banned by the glossary (rail, pane, list, view) are banned here too, because the role table doubles as the window's colour glossary.

**Theme file.** JSON, one object, with a name, a schema version, a sparse palette block and a sparse role-override block. JSON because the crate already parses it and it needs no new dependency; the parser walks the document and validates, so no derive is required. Unknown top-level keys, unknown palette entries and unknown role names are hard errors carrying the file, the offending name, and the closest legal name. Values must parse as six- or eight-digit hex with an optional alpha.

**Loading order.** The default theme is compiled in, so the app has a complete role map before it touches the disk. A theme is then selected by an environment variable naming either a bundled theme or a path, loaded and overlaid on the default. Any syntax error, unknown name or unreadable file leaves the already-complete default applied and writes one line to the log; nothing is shown in the window, which must never surface daemon or configuration state.

**The bridge.** The resolved map is applied through a single entry point that assigns each role to the UI's theme global. That global is `in-out`, even though switching at runtime is out of scope, so a later reload needs no component changes. The role table is an enum whose setter match is exhaustive, so a role can never exist without a setter; the remaining seam — enum versus the properties declared in the UI's theme source — is closed by a test rather than by code generation, which keeps the theme source readable.

**Legibility checking.** The role table declares which pairs are ink-on-surface (transcript body, composer text, sidebar row, menu item, ledger value, and the small-caps labels). A check computes the contrast ratio for each pair and reports those under the floor — 4.5:1 for body text, 3:1 for meaning-bearing marks and lines; decorative hairlines are exempt. A standalone check invocation prints the report for a file without launching the window; the same check runs against bundled themes in the test suite. Failing contrast is a warning, never a refusal to load.

**The two Rust-side consumers.** The transcript's markup builder and the composer's canvas text currently hold private colour constants. Both take their colours from the resolved theme instead: the markup builder's colour helper receives a role rather than a hex string, and the canvas takes its normal, command and selection colours from the theme when it is constructed. This makes the theme the only owner of colour in the crate.

**Components.** The extraction is proportional, not a rewrite. A shared primitive is created where the same role pair already repeats three or more times: a button (fill and ink for rest, hover, pressed, disabled, with variants for primary, quiet, danger), a panel (surface, line, shadow — the four menus), and a tag (fill and ink for working, parked, idle, current). Everything else keeps its current structure and only swaps literals for roles. Variants select roles inside the primitive; the values stay in the theme. Glyph defaults become role names, and the brand mark's fill values become brand roles so the artwork is themeable without becoming a hex dump.

**What the tests enforce.** Two repo-level checks run on every build: no colour literal may appear in the UI sources outside the theme source, and every role referenced by a component must be declared in the theme source with a setter behind it. Both read the sources as text, which is the same technique the transcript tests already use to assert on emitted markup.

**Sequencing.** (1) Introduce the role table, the theme global and the default theme, and swap every literal for a role with no visual change. (2) Move the markup and canvas consumers onto roles. (3) Extract the three primitives. (4) Ship the monochrome theme, seeded from the working tree's monochrome pass, including its rule that danger and status tags keep their hue while surfaces, ink and chrome go neutral.

## Testing Decisions

A good test here asserts something a theme author or a reader could observe: the role map a file resolves to, the message a bad file produces, the markup string a cell emits, the frame a window draws. It does not assert on the order entries were inserted, on intermediate structs, or on the exact wording of a message beyond the parts that carry meaning (the offending name, the suggestion).

**Seam one — resolution, the highest seam.** Theme resolution is pure: text in, complete role map or a named error out. It is tested without a window, without a disk and without the UI sources. Cases that matter: a sparse theme inherits every gap from the default; a role override beats its derivation; an unknown role name is refused and suggests the nearest legal one; a batch of legal-but-awful values is reported by the contrast check while the map still resolves. Prior art in this codebase: the transcript's module tests and image decoding tests, which call a pure function and assert on its output.

**Seam two — the source contract.** One test reads the UI sources as text and holds three invariants: the properties declared on the theme global equal the role table exactly; every `Theme.<name>` reference resolves; no hex colour appears outside the theme source. Prior art: the transcript markup tests already parse emitted `<font color=…>` strings, and the golden preview tests assert on generated text rather than on internals.

**Seam three — the Rust consumers.** The markup builder and the composer canvas are tested against a non-default theme: the same cell, rendered under two themes, must emit different colours and identical text. The canvas colour choice is observable through the buffer it produces, which is already covered by existing caret and buffer tests.

**Fidelity check, not a CI test.** The default theme must render identically to today's window. That is verified with the project's visual probe — one screenshot before the extraction, one after, compared by eye and by a pixel diff with a tolerance for antialiasing. It stays a manual, agent-run check because there is no headless renderer in CI and a golden-image comparison across machines would fail for reasons unrelated to the theme.

**Bundled themes are fixtures.** The default and the monochrome theme both load in the suite: one proves the derivation path, the other proves that a theme which leaves danger and tags coloured resolves without touching anything else.

## Out of Scope

- Switching themes at runtime or reloading a theme file while the window is open. The bridge is shaped so this is additive, but no watcher, no reload, no UI for it.
- Following the operating system's light/dark preference automatically.
- Spacing, radius, type scale and motion as theme tokens. They have the same seam and the same shape; they are the next layer, not this one.
- Per-session, per-workspace or per-theme font choices.
- A theme browser, importer, or any marketplace affordance; a theme is a path or a bundled name.
- The executable's icon and taskbar artwork.
- Redesigning any surface while extracting it. The extraction is deliberately mechanical.

## Further Notes

The working tree currently carries an uncommitted monochrome pass over every UI source plus the two Rust colour consumers. That pass is the best available evidence for which literals are really roles, and it is also the second theme: seed the role table from it, then express the same result as a theme file and let the literals go back to their original values.

The role table will land somewhere around sixty to seventy entries. That size is the point — it is the whole colour vocabulary of the window in one readable place — but it is also the review cost. Keeping each role to one line with a one-line comment on meaning, and keeping the extraction free of behavioural change, is what makes that review possible.

The existing `Type` global is the precedent for how a theme global is declared, imported and read; following it means contributors meet nothing new.

Publishing: this repository has no issue tracker configured, so this spec lives here as a file. Running `/setup-matt-pocock-skills` would wire the tracker and the triage labels, after which this can be filed with `ready-for-agent`.
