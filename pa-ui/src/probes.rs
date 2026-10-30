//! Probes: ignored measurements. Run with --ignored; they report numbers and assert nothing.

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
        let markup = transcript::markdown_markup("Prose with some inline code and bold and a link.");

        time("markdown_markup, one paragraph", 2000, || {
            transcript::markdown_markup("Prose with some inline code and bold and a link.")
        });
        time("StyledText::from_markdown, one row", 2000, || {
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
