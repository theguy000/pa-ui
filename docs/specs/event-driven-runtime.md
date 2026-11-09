# Event-driven window runtime

## Purpose
The window updates from daemon events and user actions, not a periodic data poll.

## Runtime
- The worker blocks on the next job or transport frame; it has no idle sleep.
- A blocking frame reader feeds the worker; connection loss wakes the select.
- Every `Msg` send also wakes the Slint event loop.
- `Tick::run` runs from a wake, a sidebar pick, or startup; a transcript scroll asks for the older page directly without running a full tick. There is no 120 ms tick.

## Data
- Attach reads the transcript, then asks for stats and the parked queue once.
- `session_action_update` updates the queue.
- `turn_end` / `agent_end` / `compaction_end` refresh stats and reconcile the transcript.
- A sequence gap, attach, user message, or turn end reads the tail; there is no tail timer.
- Roster, activity, model catalog, saved catalog, and bindings stay push-driven.

## UI-only timers
- The spinner/elapsed timer is animation, not data; it starts on busy and stops when the turn settles.
- Attachment reads wake the UI directly.
- The title bar publishes maximise/hover state from `WM_SIZE` / `WM_NCHITTEST` / `WM_MOUSELEAVE` / `WM_DPICHANGED`.
- Reconnect backoff and one-shot settle timers remain.

## Acceptance
- No queue, stats, tail, worker-idle, UI-tick, attachment, or title-bar polling remains.
- Attach requests stats and queue once.
- Queue replies are scoped to the selected session.
- `cargo test -p pa-ui` passes.

## Known risk

- If the daemon drops a terminal turn event and no later event, roster update, or sequence gap arrives, the last tail reconciliation waits for the next message. The old tail timer covered that case; this runtime relies on event delivery. It is a residual gap, not a second polling layer.
