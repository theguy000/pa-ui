# Activity chip

**Status:** implemented

## Problem Statement

Subagent Sessions are intentionally hidden from the Left sidebar, and a reader
needs a lightweight way inside the desktop window to see running and recent
subagents, heartbeats, and shells for the selected Session, control them, and
inspect long transcripts or shell output without crowding the main conversation.

## Solution

An interactive three-segment Activity chip at the left of the composer's bottom
tool row paired with a borderless, `pa-tui`-inspired **TUI Composer Sheet**
(`Style B`) inside the top of the composer card and dynamic per-Session tabs in
the **Right sidebar**:

- **Subagents:** direct and nested descendants of the selected Session
- **Heartbeats:** Session-scoped heartbeats (active and paused)
- **Shells:** kernel bash commands in the selected Session

Clicking any segment toggles the TUI Composer Sheet directly to that view (the
three chip segments serve as the tabs; clicking the active segment closes the
sheet). All three views are strictly scoped to the selected Session and reset
when switching Sessions.

## Counts

Each segment tracks `(running, total)` for the selected Session:

- When `running > 0`, the segment displays the **running/active** count in gold.
- When `running == 0`, the segment displays the **total** count for this
  Session in muted ink (`0` only when none exist).

Scoping rules:

- **Subagents** are direct and nested roster descendants (plus direct
  `get_rlm_children` entries). Running children sort first; nested children
  indent with `↳`. A Session never counts itself.
- **Heartbeats** are the `heartbeats_list` rows whose durable session id or
  live active session id matches the selected Session. Active jobs count as
  running; paused jobs count toward the Session total.
- **Shells** are the `list_kernel_bash` rows for the selected Session, sorted
  running-first. A Session without a running kernel reports `(0, 0)`.

Updates are event-driven (attach, sheet open, drill-in, manual refresh, and
daemon push events such as `roster_update`, `heartbeats_changed`, and
`tool_execution_end`) with no periodic polling timers. Stale replies from a
previously selected Session are discarded.

## Layout & Interactions

- **TUI Composer Sheet (`Style B`):** Mounts seamlessly inside the top of the
  composer card above the input area, separated by a hairline rule. Uses dim
  column headers, a `›` selection marker with soft full-row wash, `●` / `◐` /
  `○` status dots, and in-place drill-in (`← back` / `Esc`) with `› Action`
  rows.
- **Heartbeats:** Table (`Interval | Instruction | Next run | Status`),
  drill-in metadata pairs (`created`, `delivery`, `schedule`, `next run`,
  `runs`, `prompt`), `Pause`/`Resume`/`Edit`/`Stop`, and an inline
  `+ set heartbeat` / `edit heartbeat` editor.
- **Shells:** Table (`Command | Duration | PID | Status`), drill-in tail
  preview, `Refresh tail`, `Kill`, and `Open in Right sidebar`.
- **Subagents:** Indented descendant table (`Subagent | Activity | Duration |
  Status`), read-only drill-in preview, `Stop` (records `"Cancelled by user"`),
  `Delete`, and `Open in Right sidebar` (read-only transcript tab alongside the
  active Session, with no steer input).
- **Right sidebar:** All-dynamic per-Session tabs (`Diff`, `Subagent`, `Shell`).
  Closing the last tab auto-collapses the Right sidebar; `+ Diff` reopens `Diff`
  when closed.

## Testing

- Pure `pa-client` tests cover `(running, total)` display rules, direct and
  nested subagent tree construction, Session-scoped heartbeat parsing/sorting,
  running-first shell parsing, and subagent message extraction.
- Pure `pa-ui` tests cover the `RightSidebarModel` tab lifecycle and the
  `ActivitySheetModel` state machine (3-segment toggle, drill-in/`Esc`,
  inline heartbeat editor, and stale-Session reply rejection).
