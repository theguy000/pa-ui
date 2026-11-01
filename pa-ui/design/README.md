# pa-ui/design — authoring sources

Nothing here is compiled, embedded or shipped. This folder *produces* what
`pa-ui/assets/` and `pa-ui/ui/icons.slint` consume.

- **`logo-designer.html`** — source of truth for the app icon. Self-contained, no
  network: it holds the geometry (`drawIcon1`), the palettes (`PALETTES.dark` /
  `PALETTES.white`) and the two parameters, `stemExt` and `weight`. Move the
  sliders, hit *Download .ICO (7 sizes)* / *.PNG (256)*, save over
  `pa-ui/assets/icon.ico` and `pa-ui/assets/icon.png`, then `cargo build`.
- **`candidates/`** — earlier explorations, raster only, superseded. Kept for history.

Shipped settings: `PALETTES.dark`, `stemExt = 0.100` (+10.0%), `weight = 0.152`.

## Translating `drawIcon1` to Slint (`PrimeAgentMark` in `ui/icons.slint`)

When porting the canvas geometry (`W = 100`, `stemExt = 0.100`, `weight = 0.152`) into
Slint `Path` elements for `PrimeAgentMark`, keep four invariants in mind:

1. **Vertical wing thickness is `th / cos(angLow)`, not `th * cos(angLow)`:**
   With `slopeLow = 20 / 29.8 = 0.67114`, `wingVy = 15.2 / cos(atan(slopeLow)) = 18.31`,
   so the inner wing corner sits at `y = 61.5 - 18.31 = 43.19` and meets the top blade's
   bottom edge at `(51.59, 37.56)`. Using `th * cos(angLow)` makes the lower wing too thin.
2. **Pre-clip `source-atop` shadows to each surface:**
   `logo-designer.html` uses Canvas `globalCompositeOperation = 'source-atop'` to draw the
   top blade's drop shadow (`shOff = 2.8`) across `underCan`. Slint `Path` has no composite
   clipping, so an unclipped shadow quad from `x = 28` to `x = 73` paints a dark bar across
   the empty triangular cutout between the stem (`x = 43.2`) and the wing (`x = 51.59`).
   Split the blade shadow into two opaque pre-clipped polygons: one on the stem
   (`#333339`, `x = 28..43.2`) and one on the wing (`#595a60`).
3. **Crop the viewbox to the mark bounds (`74x74`), not the padded `100x100` tile:**
   In `logo-designer.html`, the `100x100` canvas includes outer tile margins around the mark
   (`x: 28..73`, `y: 16.5..89.5`). Using `viewbox: 0 0 100 100` inside an `18x18` icon
   shrinks the glyph to `8.1px x 13.1px`, causing sub-pixel aliasing on the top blade edge.
   Use `viewbox-x: 13.5; viewbox-y: 16.0; viewbox-width: 74; viewbox-height: 74` on all
   sub-paths so the bare mark fills its `18x18` box cleanly.
4. **Wind every polygon so the accumulated area is positive, or the renderer calls it a hole:**
   Slint's FemtoVG backend (`i-slint-renderer-femtovg`) computes polygon winding as
   `self.area += (p.x - self.prev.x) * (p.y + self.prev.y);` and marks any path with
   `area < 0` as `Solidity::Hole` instead of `Solidity::Solid`. All 6 polygons in
   `PrimeAgentMark` were wound clockwise (`area < 0`), so FemtoVG treated every shape as a
   hole, inverting its vertex normals and extruding the 0.5px anti-aliasing fringe
   inward/backward at acute corners — the missing corner pixel at top-left and the split
   tip on the right. Both look like geometry bugs and are not: reverse the point order
   (`M 28 16.5 L 28 89.5 …`, not `M 28 16.5 L 43.2 27.14 …`) and the same coordinates
   render solid.

## Why the components look the way they do

Reasoning that does not fit in a one-line comment in the source lives here.

**The mark's tones.** `PrimeAgentMark`'s six fills are brand roles, not artwork
values. `brand-mark-blade` is the brand tone itself; the other five are that tone
mixed toward the window's background at fixed ratios, so a theme that turns the
window over turns the mark over with it and the shadows keep their distance from
the shapes they darken. The ratios live on the roles in `src/theme/table.rs` and
the tones they resolve to are asserted in that table's tests.

**The marquee's travel.** `MarqueeText` measures with a probe `Text` drawn in the
string's own font. The sidebar used to compute the shift in Rust — 6.6px a
character against 178px of sidebar — which is a guess about a font and can only
be right at one size. The component owns *how* the text travels; the caller owns
*when*: the sidebar sets `active` on row hover and holds the tail, because
letting go of the row should send the name home, while a failed cell's reason
loops, because a popup has nowhere else to send the reader.

**The send/stop morph.** The two marks are both always present and cross over:
the arrow shrinks and turns, the square grows out of the middle and turns the
other way. Slint cannot interpolate one path into another, so the motion is
scale, rotation and opacity on one clock. The two fields are named rather than
taken from a `ButtonKind` because the same control changes job while it is on
screen.

**The follow latch.** Once the reader scrolls up, following stays off through any
number of layout passes. Only three gestures re-arm it: the jump control, sending,
or scrolling down onto the floor. A measurement must never re-arm it — content
growing at the bottom rewrites the offset, and for one frame the distance to the
floor reads as zero. `scrolled` distinguishes the two, because it fires only for a
user drag or wheel.

## The rule: **`assets/` is build input, `design/` is authoring source.** A new file
belongs in `design/` unless `build.rs` or the app reads it at build time.
