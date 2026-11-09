# Session binding

## Purpose
Follow a session when the supervisor rebinds it to a new `activeSessionId` after a worker replacement, instead of continuing to address the old worker.

## Wire
`session_binding` carries:

- `previousActiveSessionId`
- `activeSessionId`

`event_frame` maps every recognized `DaemonClientEvent` explicitly. There is no catch-all; a future event is a compile error.

## UI behavior
- If the selected session is `previousActiveSessionId`, set the selection to `activeSessionId`.
- Force the normal attach path to run for the new id: clear the loaded marker; the next tick clears the sheet and attaches.
- Ignore bindings for other sessions.
- No polling is added.

## Failure
An empty or unchanged id pair is ignored. If attaching the new id fails, the existing error path reports it.

## Acceptance
- Frame conversion test for `session_binding`.
- Pure selection test for matching, non-matching, empty, and unchanged ids.
- `cargo test -p pa-client -p pa-ui` passes.
