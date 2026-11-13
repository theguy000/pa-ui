//! Probes: ignored measurements plus cheap source contracts. Run measurements
//! with --ignored; the contracts run with the normal suite.

#[cfg(test)]
mod efficiency_probe {
    use crate::sheet::Sheet;
    use crate::transcript;
    use serde_json::{json, Value};
    use std::time::Instant;

    /// A session shaped like a real one: prose, code blocks, tool calls with
    /// results, and thinking.
    fn session(messages: usize) -> Vec<Value> {
        let mut out = Vec::new();
        for index in 0..messages {
            match index % 4 {
                0 => out.push(json!({
                    "role": "user",
                    "timestamp": index,
                    "content": [{"type": "text", "text": format!("message {index}: fix the thing that broke yesterday, and keep the rest as it is")}]
                })),
                1 | 2 => {
                    let call = format!("call-{index}");
                    out.push(json!({
                        "role": "assistant",
                        "timestamp": index,
                        "content": [
                            {"type": "thinking", "thinking": "the store is the authority\nand the events are advisory\n".repeat(3)},
                            {"type": "text", "text": format!("# heading {index}\n\nProse with some inline code and bold and a link.\n\n```rust\nfn work() -> u32 {{\n    // a comment\n    41 + 1\n}}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n")},
                            {"type": "toolCall", "id": call, "name": "bash", "arguments": {"command": "cargo test -p pa-ui"}}
                        ]
                    }));
                    out.push(json!({
                        "role": "toolResult",
                        "timestamp": index,
                        "toolCallId": call,
                        "content": [{"text": format!("test result: ok. {index} passed\n")}],
                        "details": {"command": "cargo test -p pa-ui", "durationMs": 1200}
                    }));
                }
                _ => out.push(json!({
                    "role": "assistant",
                    "timestamp": index,
                    "content": [{"type": "text", "text": "A short answer.\n\n".repeat(4)}]
                })),
            }
        }
        out
    }

    fn time<T>(label: &str, runs: usize, mut body: impl FnMut() -> T) -> T {
        let start = Instant::now();
        let mut out = None;
        for _ in 0..runs {
            out = Some(body());
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        eprintln!("[bench] {label:44} {each:8.3} ms/run  ({runs} runs)");
        out.expect("a run")
    }

    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn how_much_a_poll_costs() {
        let messages = session(300);
        let (rows, _) = time("build, cold cache (300 msgs)", 3, || {
            crate::transcript::reset_cache();
            transcript::build(&messages, 300)
        });
        eprintln!("[bench] rows per page: {}", rows.len());

        let (rows, _) = time("build, warm cache (300 msgs)", 20, || {
            transcript::build(&messages, 300)
        });

        let (sheet, _list) = Sheet::new();
        sheet.replace(rows.clone());
        eprintln!("[bench] rows on the sheet: {}", sheet.rows().len());

        let unchanged = rows.clone();
        time("a poll where nothing changed", 20, || sheet.merge(unchanged.clone()));

        let mut changed = rows.clone();
        let last = changed.len() - 1;
        changed[last].body.push_str(" and one more sentence");
        time("a poll where one message changed", 20, || sheet.merge(changed.clone()));

        let older: Vec<_> = rows.iter().take(60).cloned().collect();
        time("a page of older rows", 20, || {
            sheet.replace(rows.clone());
            sheet.prepend(older.clone());
        });
    }
}

#[cfg(test)]
mod efficiency_parts {
    use crate::sheet::to_slint;
    use crate::transcript;
    use serde_json::json;
    use std::time::Instant;

    fn time<T>(label: &str, runs: usize, mut body: impl FnMut() -> T) -> T {
        let start = Instant::now();
        let mut out = None;
        for _ in 0..runs {
            out = Some(body());
        }
        let each = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        eprintln!("[bench] {label:44} {each:8.3} ms/run  ({runs} runs)");
        out.expect("a run")
    }

    /// The parts the poll spends its time in, measured separately.
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn where_the_poll_goes() {
        let message = json!({
            "role": "assistant",
            "timestamp": 7,
            "content": [
                {"type": "text", "text": "# heading\n\nProse with some inline code and bold.\n\n```rust\nfn work() -> u32 { 41 + 1 }\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n"},
                {"type": "toolCall", "id": "call-1", "name": "bash", "arguments": {"command": "cargo test"}}
            ]
        });
        let markup = transcript::markdown_markup(
            "Prose with some inline code and bold and a link.",
            &crate::theme::current(),
        );

        time("markdown_markup, one paragraph", 2000, || {
            transcript::markdown_markup(
                "Prose with some inline code and bold and a link.",
                &crate::theme::current(),
            )
        });
        let _ = time("StyledText::from_markdown, one row", 2000, || {
            slint::StyledText::from_markdown(&markup)
        });
        time("message.to_string() (fingerprint input)", 2000, || message.to_string());
        time("one message -> rows (cold rows, warm cache)", 2000, || {
            let (rows, _) = transcript::build(std::slice::from_ref(&message), 1);
            rows
        });
        let (rows, _) = transcript::build(std::slice::from_ref(&message), 1);
        time("to_slint, that message's rows", 2000, || {
            rows.iter().map(to_slint).collect::<Vec<_>>()
        });
        eprintln!("[bench] rows for that message: {}", rows.len());
    }
}

#[cfg(test)]
mod efficiency_stream {
    use crate::sheet::to_slint;
    use crate::transcript;
    use serde_json::json;
    use std::time::Instant;

    fn growing(chars: usize) -> serde_json::Value {
        let paragraph = "The store is the authority and the events are advisory, so a poll reconciles rather than trusts. ";
        let mut text = String::new();
        while text.len() < chars {
            text.push_str(paragraph);
            if text.len() % 900 < paragraph.len() {
                text.push_str("\n\n```rust\nfn step() -> u32 { 1 }\n```\n\n");
            }
        }
        json!({"role": "assistant", "timestamp": 1, "content": [{"type": "text", "text": text}]})
    }

    /// What one streamed delta costs when the message being written is long.
    #[test]
    #[ignore = "a measurement, not an assertion"]
    fn the_cost_of_one_more_character() {
        for chars in [2_000usize, 6_000, 20_000] {
            let message = growing(chars);
            let runs = 50;
            let start = Instant::now();
            let mut rows = 0;
            for _ in 0..runs {
                // a delta invalidates the message, so this is always a cold build
                crate::transcript::reset_cache();
                let (built, _) = transcript::build(std::slice::from_ref(&message), 1);
                rows = built.len();
                let _: Vec<_> = built.iter().map(to_slint).collect();
            }
            let each = start.elapsed().as_secs_f64() * 1000.0 / runs as f64;
            eprintln!("[bench] one delta, {chars:>6} chars -> {rows:>2} rows   {each:8.3} ms");
        }
    }
}

#[cfg(test)]
mod sidebar_fold_motion {
    //! The fold's fade is deliberately skipped for large groups: every row
    //! opacity becomes its own render layer. `examples/sidebar_anim_probe.rs`
    //! reproduces the frame-rate difference; this locks the policy.

    use crate::{AppWindow, SessionEntry, SessionItem};
    use slint::{ModelRc, VecModel};
    use std::rc::Rc;

    fn done_items(count: usize) -> ModelRc<SessionItem> {
        ModelRc::from(Rc::new(VecModel::from(
            (0..count)
                .map(|_| SessionItem::default())
                .collect::<Vec<_>>(),
        )))
    }

    fn one_entry() -> ModelRc<SessionEntry> {
        ModelRc::from(Rc::new(VecModel::from(vec![SessionEntry {
            is_header: false,
            label: "".into(),
            count: 0,
            folded: false,
            item: SessionItem::default(),
            at: 0.0,
            target: 0.0,
            seam: 2,
        }])))
    }

    #[test]
    #[ignore = "opens a window; run alone with --ignored"]
    fn large_folds_skip_the_per_row_fade_and_small_folds_keep_it() {
        let ui = AppWindow::new().expect("window");
        ui.set_fold_index(2);
        ui.set_done_items(done_items(13));
        assert_eq!(ui.invoke_fold_size(), 13);
        assert_eq!(ui.invoke_row_fade(0), 1.0);

        ui.set_fold_open(true);
        ui.set_fold_flip(true);
        ui.set_fold_mix(0.5);
        ui.set_splice_at(ModelRc::from(Rc::new(VecModel::from(vec![0.0f32]))));
        ui.set_session_entries(one_entry());
        ui.set_done_items(done_items(12));
        assert_eq!(ui.invoke_fold_size(), 12);
        // The fade is allowed to start at zero; the point is that a small fold
        // reads the row's own opacity instead of the forced 1.0 large folds use.
        assert!(ui.invoke_row_fade(0) < 1.0);
    }
}


#[cfg(test)]
mod activity_chip_contract {
    //! The chip is a readout, not a control, and it belongs at the left edge of
    //! the composer row. This source-level contract guards the two layout
    //! regressions the prototype caught: centring the row and reintroducing a
    //! click surface.

    #[test]
    fn the_activity_chip_stays_left_aligned_and_display_only() {
        let app = include_str!("../ui/app.slint");
        let composer = include_str!("../ui/composer.slint");
        let start = composer
            .find("export component ActivityChip")
            .expect("activity chip");
        let end = composer[start..]
            .find("export component Chip")
            .map(|offset| start + offset)
            .expect("next composer chip");
        let chip = &composer[start..end];

        assert!(chip.contains("inherits Rectangle"), "the chip is not a control");
        assert!(!chip.contains("TouchArea"), "the chip grew a hit area");
        assert!(!chip.contains("callback"), "the chip grew a callback");
        assert!(!chip.contains("clicked"), "the chip grew a click handler");
        assert!(!chip.contains("Panel"), "the chip grew a popover");
        assert!(chip.contains("height: 24px"), "the chip no longer matches the tool row");
        assert!(
            chip.contains("padding-top: 5.5px") && chip.contains("padding-bottom: 5.5px"),
            "the icon/count line is no longer vertically centred in the tag"
        );
        let compact: String = app.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(
            compact.contains("HorizontalLayout{spacing:8px;ActivityChip{"),
            "the composer row was centred or the chip moved off the left edge"
        );
        assert!(
            compact.contains("Rectangle{x:12px;y:parent.height-8px-25px;width:parent.width-24px;height:25px;background:transparent;HorizontalLayout{spacing:8px;ActivityChip{"),
            "composer dock row must remain pinned to the bottom of composer-box"
        );
    }

    #[test]
    fn changed_scroll_range_ignores_large_transient_shrink_but_keeps_small_chrome_clamp() {
        let app = include_str!("../ui/app.slint");
        let compact: String = app.chars().filter(|ch| !ch.is_whitespace()).collect();
        assert!(
            compact.contains("root.prev-scroll-range-root.scroll-range<=120px"),
            "changed scroll-range must only clamp small UI chrome shrinks, not large layout re-measure drops"
        );
    }
}
