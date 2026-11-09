# Event-driven activity chip

## Purpose
Keep the subagent, heartbeat, and shell counts current without a periodic data refresh.

## Events
- Roster snapshot / `roster_update`: subagent count for the attached session.
- `heartbeats_changed`: re-read the scoped heartbeat count once.
- `bash_start` / `bash_end` on the attached session: re-read the running shell count once.
- Attach: one initial activity read covers all three.

## Rules
- No data-refresh timer. The UI tick may still repaint animation and elapsed time.
- A failed partial read sends `None`, preserving the last good count.
- Counts are scoped to the attached session.

## Acceptance
- The 2-second `ActivityRead` timer is gone.
- Roster updates change the subagent count.
- Heartbeat and shell counts update from their events.
- Raw roster reconciliation covers changed, removed, and resync.
- `cargo test -p pa-client -p pa-ui` passes.
