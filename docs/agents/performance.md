 # Performance

The window's responsiveness is measured, not guessed. The measurements live in
`pa-ui/src/probes.rs` as `#[ignore]`d `lag_probe` tests, so the normal test run
stays timing-free.

## Running a probe

On a machine with a display (the bench runner uses Xvfb):

```sh
SLINT_DEBUG_PERFORMANCE=refresh_lazy,console \
  cargo test --profile fast -p pa-ui -- --ignored --exact probes::lag_probe::scroll_across_pages --nocapture
```

The probe prints one line: wall time, process CPU as a percentage of one core,
driver ticks, the worst callback time, and how many transcript and sidebar rows
are mounted. Slint's performance log prints the frame rate.

`scroll_across_pages` reads a deterministic 40-Turn transcript (160 messages) and
scrolls up across the paging threshold. `stream_delta` appends a 23-byte delta every 30 ms
to a 20 KB answer. `spinner` and `settled_idle` are the two ends of the busy
window.

CPU is read from `/proc`; on other platforms the probe prints wall time and
frames only.

## The dev profile optimizes dependencies

`Cargo.toml` sets `opt-level = 2` for `[profile.dev.package."*"]`. Slint's layout
and renderer are dependencies, and unoptimized they made a `cargo run` window
3-9x more expensive than the same code in `--profile fast`: scrolling cost 119.6%
of a core instead of 18.8%, and a streamed delta cost 22 ms instead of about 2 ms.

The trade-off: the first build after a dependency change is slower, and
backtraces inside dependencies have less usable line information. Workspace
crates keep the dev profile's debug assertions and backtraces.
