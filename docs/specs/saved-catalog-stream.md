# Streamed saved-session catalog

## Purpose
Load the saved-session catalog progressively from the daemon's streaming scan instead of polling `list`.

## Flow
- On the first attach with no cached catalog, send one `list_saved_sessions`
  request addressed by `activeSessionId`.
- A later session switch reuses the cached catalog; it does not start another
  scan.
- A daemon reconnect discards the cached catalog, so the next attach scans the
  catalog from the new daemon.
- `session_list_item` frames upsert into a pending list; send a snapshot every 8 rows.
- The terminal response's `data.sessions` is authoritative and replaces the streamed list.
- A new request replaces the pending request.
- Catalog-changing actions request a fresh stream once.
- Errors keep the last good catalog.

## Rules
- No periodic `list` refresh.
- Live roster rows still win in the merged sidebar.
- One-shot requests are triggered by the first attach or a user action, never a
  timer.

## Acceptance
- Streamed items upsert by session identity.
- Switching sessions does not rescan the catalog or rebuild Done.
- Terminal response replaces streamed rows.
- `cargo test -p pa-client -p pa-ui` passes.
