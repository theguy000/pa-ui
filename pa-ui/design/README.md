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

The rule: **`assets/` is build input, `design/` is authoring source.** A new file
belongs in `design/` unless `build.rs` or the app reads it at build time.