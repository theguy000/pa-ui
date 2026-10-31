# pa-ui/design — authoring sources

Nothing here is compiled, embedded or shipped. This folder *produces* what
`pa-ui/assets/` consumes.

- **`logo-designer.html`** — source of truth for the app icon. Self-contained, no
  network: it holds the geometry (`drawIcon1`), the palettes (`PALETTES.dark` /
  `PALETTES.white`) and the two parameters, `stemExt` and `weight`. Move the
  sliders, hit *Download .ICO (7 sizes)* / *.PNG (256)*, save over
  `pa-ui/assets/icon.ico` and `pa-ui/assets/icon.png`, then `cargo build`.
- **`candidates/`** — earlier explorations, raster only, superseded. Kept for history.

Shipped settings: `PALETTES.dark`, `stemExt = 0.100` (+10.0%), `weight = 0.152`.

The rule: **`assets/` is build input, `design/` is authoring source.** A new file
belongs in `design/` unless `build.rs` or the app reads it at build time.
