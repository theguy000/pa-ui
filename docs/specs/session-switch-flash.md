# Session switch flash

## Purpose
Do not leave the previous Session's Sheet on screen after the reader picks another row.

## Cause
The sidebar updates `selected-id` synchronously on a row pick. The Sheet and attach path ran only on the 120 ms poll tick, so the new row was highlighted while the old transcript remained visible until the next tick.

## Behavior
- A row pick that changes `selected-id` invokes `selection-picked`.
- The pick handler runs the normal tick immediately: clear the Sheet, reset paging, and send the attach.
- Picking the already-selected row does not invoke the callback.
- A search pick that changes the selection takes the same path.
- Programmatic selection changes still follow the existing tick path; the callback is for reader picks.

## Acceptance
- Row picks clear the previous Sheet in the same event, not on the next poll.
- Search picks do the same.
- Re-picking the current row does not clear or reattach it.
- `cargo test -p pa-client -p pa-ui` passes.
