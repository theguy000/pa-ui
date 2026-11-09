Status: Implemented in #45.

# The status strip settles when the turn ends

## Problem Statement

The desktop Status strip can keep saying `Executing` after the engine's Turn has
ended and the TUI has returned to idle. The reader sees a spinner and an elapsed
time that keep counting.

Two paths can hold the strip up after `turn_end`:

1. `Msg::Activity` sets `busy` to true unconditionally. The worker sends one for
   every stream frame. A late frame from the finished Turn can therefore wake an
   idle strip. The TUI only changes the loader's word while a loader is already
   running (`track_stream_activity`), so it never has this path.
2. The strip also derives `busy` from the roster in `Msg::Sessions`. Roster
   deltas can be dropped when a client lags the daemon's event ring, and the
   daemon's content-diffed push does not re-send an unchanged idle row. The
   window's cached roster row then stays `isRunningTools: true`. Every successful
   `list` snapshot is merged with `merged_sessions`, which always lets the cached
   roster row win over the fresh saved row, so the stale running state survives
   every poll. The strip never clears.

## Solution

The Turn's terminal frame is the authority for the selected Session, as it is in
the TUI:

- `turn_end`, `agent_end`, and `compaction_end` clear the strip and mark the
  Session settled.
- While the selected Session is settled, a busy roster snapshot does not reopen
  the strip.
- A new prompt or a user `message_start` releases the settlement; an idle roster
  snapshot also releases it once the Session has actually been reported idle.
- `Msg::Activity` only changes the label of a strip that is already up. It never
  starts the strip by itself.
- Each successful `list` snapshot reconciles the cached roster: rows covered by
  the snapshot are removed from the cache so the fresh snapshot wins. Roster
  rows that are not in the snapshot (live-only rows) are kept.
- A terminal Turn asks the worker for a fresh list, so a stale cached row is
  corrected immediately instead of waiting for the next 30-second fallback.

## Requirements

- A roster snapshot that still says `Executing` after `turn_end` does not
  reopen the strip.
- An activity frame alone does not open the strip.
- A new prompt or user message lets the next roster snapshot open the strip.
- A stale cached roster row is cleared by the next full `list` snapshot.
- An unmatched roster row is not dropped by reconciliation.
- The TUI's own status behavior is unchanged.
- No new polling: the full list remains the existing fallback, now forced on a
  terminal Turn.

## Non-goals

- No TUI change.
- No daemon change.
- No new theme role.
- No change to the sidebar's roster display beyond removing stale cached state.

## Acceptance

- Regression tests for the three status paths.
- Worker test for list reconciliation.
- `cargo test -p pa-ui --bin pa-ui`: all tests pass.
- `cargo build --profile fast -p pa-ui --bin pa-ui`: passes.
- Mutation: removing the settlement guard reopens the strip in the regression
  test; removing reconciliation leaves the stale row in place.
