# Model catalog invalidation

## Purpose
The daemon refreshes its model catalog in the background and broadcasts `model_catalog_changed`. The desktop must not keep serving its first cached snapshot after that.

## Behavior
- `event_frame` forwards `model_catalog_changed`.
- The UI clears `ModelMenu.loaded`.
- If a session is selected, request the catalog once immediately; otherwise the next menu open requests it.
- No timer and no periodic refresh.

## Acceptance
- `ModelMenu::invalidate` clears the loaded flag.
- Event handling clears the cache and requests once for the selected session.
- `cargo test -p pa-client -p pa-ui` passes.
