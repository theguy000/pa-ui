
use super::*;

mod build_tests {
    use super::*;
    use serde_json::json;

    fn cell_and_result(output: &str) -> Vec<Row> {
        let messages = vec![
            json!({
                "role": "assistant",
                "content": [{
                    "type": "toolCall",
                    "id": "call_1",
                    "name": "python_repl",
                    "arguments": {"code": "print(rows[3][\"name\"])"}
                }]
            }),
            json!({
                "role": "toolResult",
                "toolCallId": "call_1",
                "toolName": "python_repl",
                "content": [{"text": output}]
            }),
        ];
        build(&messages, 300).0
    }

    #[test]
    fn traceback_marks_the_cell_failed_with_its_reason() {
        let rows = cell_and_result(
            "Traceback (most recent call last):\n  File \"<cell-3>\", line 1\n    raise KeyError\nKeyError: 'name'",
        );
        assert_eq!(rows.len(), 1);
        assert!(rows[0].failed);
        assert_eq!(rows[0].reason, "KeyError: 'name'");
    }

    #[test]
    fn clean_output_is_not_a_failure() {
        let rows = cell_and_result("2");
        assert!(!rows[0].failed);
        assert!(rows[0].reason.is_empty());
    }

    #[test]
    fn pasted_entities_are_decoded_not_shown() {
        assert_eq!(decode_entities("&#x20;a &amp; b"), " a & b");
        assert_eq!(decode_entities("caf&#233;"), "café");
        assert!(!markdown_markup("&#x20;    x").contains("&#x20;"));
        assert!(!plain_markup("a&nbsp;b").contains("&nbsp;"));
    }

    #[test]
    fn code_markup_survives_the_parser() {
        let markup = highlight_python("def f():\n    # four space indent\n    return 'x'");
        assert!(StyledText::from_markdown(&markup).is_ok(), "parser rejected: {markup}");
    }
}


#[cfg(test)]
mod strong_tests {
    use super::*;

    #[test]
    fn a_span_across_a_line_break_is_closed_and_reopened() {
        assert_eq!(
            balance_strong("one **two\nthree** four"),
            "one **two**\n**three** four"
        );
    }


    #[test]
    fn a_message_splits_into_prose_code_and_table() {
        let text = "before\n\n```python\nprint(1)\n```\n\n| a | b |\n|---|--:|\n| 1 | 2 |\n\nafter";
        let blocks = split_blocks(text);
        assert_eq!(blocks.len(), 4, "prose, code, table, prose");
        assert!(matches!(blocks[0], Block::Prose(_)));
        assert!(matches!(blocks[1], Block::Code { .. }));
        assert!(matches!(blocks[2], Block::Table(_)));
        assert!(matches!(blocks[3], Block::Prose(_)));
    }

    #[test]
    fn an_unclosed_fence_still_makes_a_code_row() {
        let blocks = split_blocks("look:\n```rust\nfn main() {}");
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            Block::Code { lang, body } => {
                assert_eq!(lang, "rust");
                assert_eq!(body, "fn main() {}");
            }
            other => panic!("expected code, got {}", matches!(other, Block::Prose(_))),
        }
    }

    #[test]
    fn a_pipe_line_without_a_delimiter_row_stays_prose() {
        let blocks = split_blocks("a | b\nnot a table");
        assert_eq!(blocks.len(), 1);
        assert!(matches!(blocks[0], Block::Prose(_)));
    }

    #[test]
    fn tables_pad_columns_and_honour_right_alignment() {
        let table = render_table(&["| model | cost |", "|:--|--:|", "| a | $1.50 |", "| longer | $2 |"])
            .expect("a valid table");
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 6, "top, header, rule, two rows, bottom");
        assert!(lines[0].starts_with('\u{250c}') && lines[0].ends_with('\u{2510}'));
        assert!(lines[1].contains("\u{2502} model"));
        // right aligned: the shorter price keeps its padding on the left
        assert!(lines[4].contains("  $2 \u{2502}"), "{:?}", lines[4]);
        assert!(lines.iter().all(|line| line.chars().count() == lines[0].chars().count()));
    }

    #[test]
    fn a_wide_table_gives_up_width_instead_of_wrapping() {
        let wide = "x".repeat(60);
        let table = render_table(&[
            &format!("| {wide} | {wide} | {wide} |"),
            "|---|---|---|",
            &format!("| {wide} | {wide} | {wide} |"),
        ])
        .expect("a valid table");
        let width = table.lines().next().map_or(0, |line| line.chars().count());
        assert!(width <= 92, "table stayed {width} wide");
        assert!(table.contains('\u{2026}'), "overflow elides, it does not wrap");
    }

    #[test]
    fn balanced_lines_are_left_alone() {
        assert_eq!(balance_strong("**a** and **b**"), "**a** and **b**");
        assert_eq!(balance_strong("plain"), "plain");
    }

    #[test]
    fn an_odd_line_closes_where_it_ends() {
        assert_eq!(balance_strong("a **b\nc"), "a **b**\n**c**");
    }

    #[test]
    fn code_spans_and_escapes_do_not_count() {
        assert_eq!(balance_strong("`a**b` c"), "`a**b` c");
        assert_eq!(balance_strong(r"a \*\* b"), r"a \*\* b");
    }

    #[test]
    fn a_span_opened_by_a_heading_stays_closed() {
        // headings become bold inside the accent colour, so the pass sees
        // balanced markers and must not add any
        assert_eq!(
            markdown_markup("# Title"),
            "<font color=\"#c8b158\">**Title**</font>"
        );
    }
}


#[cfg(test)]
mod image_tests {
    use super::*;

    #[test]
    fn an_image_block_becomes_its_own_row() {
        let message = serde_json::json!({"role": "user", "content": [
            {"type": "text", "text": "look at this"},
            {"type": "image", "data": "QUJD", "mimeType": "image/png"},
        ]});
        let (rows, _) = build(std::slice::from_ref(&message), 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, KIND_USER);
        assert_eq!(rows[1].kind, KIND_IMAGE);
        assert_eq!(rows[1].image, "QUJD");
    }

    #[test]
    fn an_image_only_message_is_still_a_row() {
        let message = serde_json::json!({"role": "user", "content": [
            {"type": "image", "data": "QUJD", "mimeType": "image/png"},
        ]});
        let (rows, _) = build(std::slice::from_ref(&message), 1);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, KIND_IMAGE);
    }
}





#[cfg(test)]
mod golden_preview_tests {
    /// The corpus the TUI checks itself against, recorded from the TypeScript
    /// tools. Not a resemblance check: every case has to match exactly, the
    /// language as well as the text, because this shell is meant to be the same
    /// shell.
    #[test]
    fn every_golden_case_matches_the_tui_exactly() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/golden/preview.json")).expect("corpus");
        let mut failures: Vec<String> = Vec::new();
        for case in corpus["cases"].as_array().expect("cases") {
            let input = case["input"].as_str().unwrap_or_default();
            let bash = crate::preview::preview_bash_command(input);
            let ipython = crate::preview::preview_ipython_code(input);
            let language = |preview: crate::preview::CodePreviewLanguage| match preview {
                crate::preview::CodePreviewLanguage::Bash => "bash",
                crate::preview::CodePreviewLanguage::Python => "python",
            };
            let ours_bash = serde_json::json!({
                "language": language(bash.language),
                "text": bash.text,
            });
            if ours_bash != case["bash"] {
                failures.push(format!("bash {input:.40}: {ours_bash} != {}", case["bash"]));
            }
            let ours_ipython = serde_json::json!({
                "language": language(ipython.language),
                "text": ipython.text,
            });
            if ours_ipython != case["ipython"] {
                failures.push(format!("ipython {input:.40}: {ours_ipython} != {}", case["ipython"]));
            }
        }
        assert!(
            failures.is_empty(),
            "{} cases differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }

    /// The title line as it is actually built, for the shapes the corpus does
    /// not cover: the label plus the preview text.
    #[test]
    fn titles_are_the_label_and_the_preview() {
        let args = serde_json::json!({ "code": "cd '/repo' && npm run test -- --watch" });
        let row = crate::transcript::tool_cell("call_0_x", &args);
        assert_eq!(row.title, "python \u{00b7} cd '/repo' && npm run test -- --watch");

        let args = serde_json::json!({ "code": "out = '/repo/out.txt'\nout.write_text('hello')" });
        let row = crate::transcript::tool_cell("call_0_x", &args);
        assert_eq!(row.title, "python \u{00b7} write /repo/out.txt");
    }
}


#[cfg(test)]
mod duration_tests {
    use super::*;

    #[test]
    fn durations_read_the_way_the_tui_writes_them() {
        assert_eq!(format_duration(Some(2)), "2ms");
        assert_eq!(format_duration(Some(999)), "999ms");
        assert_eq!(format_duration(Some(1_000)), "1.0s");
        assert_eq!(format_duration(Some(12_345)), "12.3s");
        assert_eq!(format_duration(Some(65_000)), "1m 05s");
        assert_eq!(format_duration(None), "");
    }

    #[test]
    fn the_timing_trails_the_line_count() {
        let code = "r = await sh(\"ls\")";
        let args = serde_json::json!({ "code": code });
        let mut row = tool_cell("call_0_x", &args);
        apply_header(&mut row, code, None, Some(2));
        assert_eq!(row.meta, "1 line");
        assert_eq!(row.elapsed, "2ms");
    }

    #[test]
    fn the_details_field_is_read_from_either_spelling() {
        let camel = serde_json::json!({ "durationMs": 1500 });
        let snake = serde_json::json!({ "duration_ms": 1500 });
        assert_eq!(duration_ms(Some(&camel)), Some(1500));
        assert_eq!(duration_ms(Some(&snake)), Some(1500));
        assert_eq!(duration_ms(None), None);
    }
}


#[cfg(test)]
mod header_parity_tests {
    use super::*;

    fn title(code: &str, details: Option<serde_json::Value>) -> String {
        let args = serde_json::json!({ "code": code });
        let mut row = tool_cell("call_0_x", &args);
        let info = bash_commands(details.as_ref());
        apply_header(&mut row, code, info.as_ref(), None);
        row.title
    }

    /// `bash_dominated_cell_renders_as_bash` in the TUI's tool_card tests: bash
    /// accounts for 4 of 5 lines, so the cell reads as bash and previews the
    /// command, with the count of the rest.
    #[test]
    fn a_bash_dominated_cell_reads_as_bash() {
        let code = "r = await sh(\"\"\"\ncd crates/pa-tui\ncargo test -p pa-tui\n\"\"\")\nprint(r.output)";
        let details = serde_json::json!({
            "status": "ok",
            "durationMs": 2,
            "bashCommands": {
                "first": "\ncd crates/pa-tui\ncargo test -p pa-tui\n",
                "count": 3,
                "lines": 4,
            },
        });
        assert_eq!(title(code, Some(details)), "bash \u{00b7} cargo test -p pa-tui \u{00b7} +2 more");
    }

    /// `bash_minority_cell_stays_python`: one bash line among four is not
    /// dominance, so the cell keeps the python label and the python preview.
    #[test]
    fn a_bash_minority_cell_stays_python() {
        let code = "async def sh(cmd):\n    return await bash(cmd)\nr = await sh(\"ls\")\nprint(r.pid)";
        let details = serde_json::json!({
            "status": "ok",
            "durationMs": 5,
            "stdout": "0\n",
            "bashCommands": { "first": "ls", "count": 1, "lines": 1 },
        });
        let line = title(code, Some(details));
        assert_eq!(line, "python \u{00b7} r = await sh(\"ls\")");
        assert!(!line.contains("bash \u{00b7}"));
    }

    /// Nothing reported yet: the header still has to say something useful, and
    /// the label comes from the code alone.
    #[test]
    fn a_running_cell_previews_its_own_code() {
        assert_eq!(title("out = '/repo/out.txt'\nout.write_text('hi')", None), "python \u{00b7} write /repo/out.txt");
    }

    /// A `%%bash` cell is bash, label and all.
    #[test]
    fn a_bash_cell_magic_is_bash() {
        assert_eq!(title("%%bash\necho in-bash-cell", None), "bash \u{00b7} echo in-bash-cell");
    }
}


#[cfg(test)]
mod markup_tests {
    use super::{markdown_markup, plain_markup, styled};
    use slint::StyledText;

    /// The renderer refuses a document whole when one span is malformed, and
    /// then the row prints as plain text. These are the shapes that did it.
    fn parses(text: &str) -> bool {
        StyledText::from_markdown(&markdown_markup(text)).is_ok()
    }

    #[test]
    fn indented_text_is_not_a_code_block() {
        // four spaces used to be folded back to a plain space before parsing,
        // which made CommonMark call it an indented code block - unsupported,
        // so the whole row fell back to plain text
        assert!(parses("first line\n    indented continuation"));
        assert!(parses("    leading indent on the first line"));
        assert!(parses("a\n\n        eight spaces deep"));
    }

    #[test]
    fn inline_code_inside_bold_does_not_overlap() {
        assert!(parses("**bold with `code` inside**"));
        assert!(parses("## heading with `code`"));
        assert!(parses("### another `one` and `two`"));
        assert!(parses("`plain code` in prose"));
        assert!(parses("**a** and `b` and **c**"));
    }

    #[test]
    fn a_message_that_mentions_a_tag_draws_it() {
        // a handover line talking about `</font>` used to emit a real closing tag
        assert!(parses("the `</font>` overlap is fixed"));
        assert!(parses("write <font color=\"#fff\"> by hand"));
        assert!(parses("**bold `</font>` bold**"));
        assert!(parses("a < b && c > d"));
        let drawn = markdown_markup("the `</font>` overlap");
        assert!(drawn.contains("&lt;/font&gt;"), "got {drawn}");
    }

    #[test]
    fn bold_running_over_a_line_break_is_balanced() {
        assert!(parses("**starts here\nand ends here**"));
        assert!(parses("**open\nstill open"));
    }

    #[test]
    fn plain_rows_still_parse() {
        assert!(StyledText::from_markdown(&plain_markup("just words\n    and indented"))
            .is_ok());
    }

    #[test]
    fn styled_never_returns_a_failed_parse() {
        for sample in [
            "**bold `code` bold**",
            "- a dash line\n- another",
            "> quoted words",
            "1. numbered\n2. list",
            "| a | b |\n|---|---|\n| 1 | 2 |",
            "text with <b>html</b> in it",
            "text with a &amp; entity and `code`",
        ] {
            let _ = styled(&markdown_markup(sample));
        }
    }
}
