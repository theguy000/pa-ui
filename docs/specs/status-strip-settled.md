Status: Implemented in PR #45.

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
  `turn_start`, `agent_start`, `compaction_start`, and `auto_retry_start` open
  the strip for a Turn the engine starts without this window's composer, as the
  TUI's loader does. Switching to another Session also drops the settlement, so
  a Turn that started while the reader was away is not hidden on their return.
- `Msg::Activity` only changes the label of a strip that is already up. It never
  starts the strip by itself.
- Each successful `list` snapshot reconciles the cached roster: rows covered by
  the snapshot are removed from the cache so the fresh snapshot wins. Roster
  rows that are not in the snapshot (live-only rows) are kept.
- The run's `agent_end` asks the worker for a fresh list, so a stale cached row
  is corrected instead of waiting for the next 30-second fallback. `turn_end`
  alone does not: a tool-heavy run has one per assistant cycle, and a full
  `list` scans the whole session directory.

## Requirements

- A roster snapshot that still says `Executing` after `turn_end` does not
  reopen the strip.
- An activity frame alone does not open the strip.
- A new prompt or user message lets the next roster snapshot open the strip.
- A `turn_start`, `agent_start`, `compaction_start`, or `auto_retry_start`
  after a settlement opens the strip for that Turn.
- A normal composer send releases the settlement when its `agent_start`
  arrives, not only the queue-edit fallback.
- An `agent_end` asks for one fresh list; a `turn_end` does not. Duplicate
  refresh requests within one second coalesce.
- An idle roster snapshot releases the settlement.
- Switching Sessions clears the settlement; returning to a Session where a new
  Turn started shows that Turn's running state.
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
- No TUI loader redesign: the new handlers follow the TUI's existing start
  points.

## Residual risk

A session switch discards the settlement because events for a hidden Session are
not delivered to the window. The attach path forces a fresh list; until that
list lands, a cached running roster row can open the strip briefly. Roster
pushes carry no sequence, so the client cannot totally order them against a
list response; the fresh snapshot narrows the window but does not close it.

## Acceptance

- Regression tests for the three status paths.
- Worker test for list reconciliation.
- `cargo test -p pa-ui --bin pa-ui`: all tests pass.
- `cargo build --profile fast -p pa-ui --bin pa-ui`: passes.
- Mutation: removing the settlement guard reopens the strip in the regression
  test; removing reconciliation leaves the stale row in place; removing the
  `agent_end` refresh leaves the refresh test without a job.
