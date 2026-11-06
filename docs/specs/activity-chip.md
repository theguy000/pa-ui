# Activity chip

**Status:** implemented

## Problem Statement

The composer's activity chip showed three permanent zeros and still carried a
click affordance. The sidebar listed spawned subagent Sessions as ordinary Idle
rows, so the idle count included work that belongs under its parent Session. A
reader could not tell how many subagents were running, how many heartbeats
belonged to the selected Session, or how many shell commands were running.

## Solution

A display-only activity chip at the left of the composer's bottom tool row. It
shows three centred icon/count pairs, with no separators and no click target:

- running subagent descendants of the selected Session
- the Session-scoped heartbeat total (active and paused)
- running shell commands in the selected Session

Subagent Sessions no longer appear as sidebar rows or search results. Counts refresh while a
Session is selected and reset when the reader selects another Session. Opening
the subagent, heartbeat, or shell views is not part of this surface.

## Counts

The TUI's activity dock is the parity source.

- **Subagents** are direct and nested roster descendants whose roster status is
  `running`. The parent linkage is the durable session id, with the active
  session id as the fallback. A Session never counts itself.
- **Heartbeats** are the `heartbeats_list` rows whose durable session id or live
  active session id matches the selected Session. Paused jobs count.
- **Shells** are the `list_kernel_bash` rows whose status is `running`. A Session
  without a running kernel has zero live shells.

The worker reads all three on the activity cadence. A failed read keeps the last
good number instead of clearing it. The window's outbox tags every reply with the
attach generation, so a Session the reader has left cannot write into the new
Session's chip.

## Layout

The chip is a non-interactive Rectangle in the composer's existing tool row,
before the flexible spacer. It reuses the composer chip fill/edge and the
composer card's muted/live inks. The three icons are the selected monoline marks:
branching tree, pulse wave, and framed prompt.

## Testing

- Pure roster counting covers direct, nested, idle, inactive, missing, and
  self-referential cases.
- Pure heartbeat and shell parsing covers durable/live scoping, empty
  identities, paused jobs, finished shells, and nameless rows.
- The window harness covers selected-Session ownership, failed-half retention,
  and selection reset.
- The ignored live-daemon smoke test reads the real roster, heartbeat catalog,
  and kernel registry.
