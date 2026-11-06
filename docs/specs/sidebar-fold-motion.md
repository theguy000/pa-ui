# Sidebar fold motion

A group header folds its rows by sliding the flat `SessionEntry` list between
two layouts. Small groups also fade their rows. Large groups slide only.

## Why large groups do not fade every row

Slint/FemtoVG renders every item whose opacity is not exactly `1.0` into its own
offscreen layer. Fading a group therefore creates one layer per row. At 150
rows, the Done fold was creating 150 layers from the row opacity and another
150 from the three-dots button's `opacity: 0`, and the window dropped to **1-3
fps**.

The fix is to stop asking the renderer for per-row layers:

- The row fade is used only when the folding group has at most 12 rows. Larger
  groups ride the slide alone, stay visible for the whole slide, and hide or
  show at the fold's endpoint rather than at an opacity threshold.
- The three-dots button stays mounted, so its 22px slot still takes the
  pointer, but it uses transparent fill/tint at rest instead of `opacity: 0`.
  Its existing background colour animation supplies the fade.

`clip: true` on a row does not create a layer when the row's opacity is `1.0`,
so the row clip is retained.

## Reproduce

```text
PROBE_ROWS=150 PROBE_SECONDS=8 PROBE_TOGGLE_MS=300 \
SLINT_DEBUG_PERFORMANCE=refresh_lazy,console \
cargo run --profile fast -p pa-ui --example sidebar_anim_probe
```

Before the fix this reported 1-3 fps and 150-304 layers created. After the fix
it reports roughly 130 fps and zero layers.

## Regression check

`probes::sidebar_fold_motion::large_folds_skip_the_per_row_fade_and_small_folds_keep_it`
opens a window, so it is ignored by the normal suite. Run it with:

```text
cargo test -p pa-ui large_folds_skip -- --ignored
```
