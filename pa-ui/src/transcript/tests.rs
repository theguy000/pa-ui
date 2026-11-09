
use super::*;

#[test]
fn a_generic_error_row_keeps_only_the_first_line() {
    let row = Row::error("Background shell command failed: exit 1\n  at shell.rs:42");
    assert_eq!(row.kind, KIND_ERROR);
    assert_eq!(row.title, "Error:");
    assert!(row.body.contains("Background shell command failed: exit 1"));
    assert!(!row.body.contains("shell.rs:42"));
}

mod build_tests {
    use super::*;
    use crate::theme::Resolved;
    use serde_json::json;

    /// The theme the window ships with: these cases are about the rows, not
    /// about what colour a row came out.
    fn shipped() -> Resolved {
        Resolved::shipped()
    }

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
        assert!(!markdown_markup("&#x20;    x", &shipped()).contains("&#x20;"));
        assert!(!plain_markup("a&nbsp;b").contains("&nbsp;"));
    }

    #[test]
    fn code_markup_survives_the_parser() {
        let markup = highlight_python("def f():\n    # four space indent\n    return 'x'", &shipped());
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
    fn quote_lines_and_lazy_continuations_make_one_quote_block() {
        let blocks = split_blocks("before\n> one\n> two\nlazy\n\nafter");
        assert_eq!(blocks.len(), 3);
        match &blocks[1] {
            Block::Quote(body) => assert_eq!(body, "one\ntwo\nlazy"),
            other => panic!("expected quote, got {}", matches!(other, Block::Prose(_))),
        }
    }

    #[test]
    fn a_quote_stops_at_a_heading() {
        let blocks = split_blocks("> quoted\n# heading");
        assert_eq!(blocks.len(), 2);
        assert!(matches!(blocks[0], Block::Quote(_)));
        assert!(matches!(blocks[1], Block::Prose(_)));
    }

    #[test]
    fn an_empty_quote_does_not_swallow_the_next_paragraph() {
        let blocks = split_blocks(">\nafter");
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
        // headings become bold inside the heading's own ink, so the pass sees
        // balanced markers and must not add any
        assert_eq!(
            markdown_markup("# Title", &crate::theme::Resolved::shipped()),
            "<font color=\"#b3b2ad\">**Title**</font>"
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
mod theme_tests {
    use super::*;
    use crate::theme::{light, Resolved};

    /// The words a markup string draws: the colour spans are the whole of what a
    /// theme may change, so they are the whole of what is taken out before the
    /// two renderings are compared.
    fn words(markup: &str) -> String {
        regex::Regex::new("</?font[^>]*>")
            .expect("a font tag")
            .replace_all(markup, "")
            .into_owned()
    }

    /// The ticket's first promise: the same cell, built twice in two themes,
    /// emits the same words in other colours. The prose around it too, which is
    /// where the heading and the symbol in backticks are drawn.
    #[test]
    fn the_same_cell_under_two_themes_emits_the_same_words_in_other_colours() {
        let shipped = Resolved::shipped();
        let light = light();

        let code = "def total(rows):\n    # add them up\n    return sum(rows), 'done'";
        let here = highlight_python(code, &shipped);
        let there = highlight_python(code, &light);
        assert_ne!(here, there, "the cell has to follow the theme");
        assert_eq!(words(&here), words(&there), "only the colours may differ");

        let prose = "# the running total\n\ncall `total(rows)` after the loop";
        let here = markdown_markup(prose, &shipped);
        let there = markdown_markup(prose, &light);
        assert_ne!(here, there, "the prose has to follow the theme");
        assert_eq!(words(&here), words(&there), "only the colours may differ");
    }

    /// The other half of that promise: the theme that ships draws exactly the
    /// markup the builder drew before there was a theme, character for character.
    #[test]
    fn the_shipped_theme_emits_the_markup_the_window_drew_before_the_theme() {
        let shipped = Resolved::shipped();

        assert_eq!(
            markdown_markup("# Title", &shipped),
            "<font color=\"#b3b2ad\">**Title**</font>"
        );
        assert_eq!(
            markdown_markup("a `b` c", &shipped),
            "a **<font color=\"#c9a86a\">b</font>** c"
        );
        assert_eq!(
            highlight_python("return 'x'", &shipped),
            "<font color=\"#acada4\">return</font> <font color=\"#aaaba2\">'x'</font>"
        );
        assert_eq!(highlight_python("# why", &shipped), "<font color=\"#60615b\"># why</font>\n");
    }
}

#[cfg(test)]
mod markup_tests {
    use super::{markdown_markup, plain_markup, quote_markup, styled};
    use crate::theme::Resolved;
    use slint::StyledText;

    /// Every one of these reads the theme the window ships with: the builder
    /// takes the theme it is drawn in, and the cases here are about the markup
    /// and not about the colours it came out in.
    fn shipped() -> Resolved {
        Resolved::shipped()
    }

    /// The renderer refuses a document whole when one span is malformed, and
    /// then the row prints as plain text. These are the shapes that did it.
    fn parses(text: &str) -> bool {
        StyledText::from_markdown(&markdown_markup(text, &shipped())).is_ok()
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
    fn markdown_markers_inside_inline_code_are_escaped() {
        assert!(parses("the `**` marker"));
        assert!(parses("- `**` spans crossing line breaks are balanced."));
        assert!(parses("the `*a* _b_ [c] ~d~` span"));
        assert!(parses(r"the `a\b` span"));
        assert!(parses("the ``a`b`` span"));
        assert!(parses("``first line\n- second line``"));

        for (source, escaped) in [
            ("`**`", r"\*\*"),
            ("`*a*`", r"\*a\*"),
            ("`_b_`", r"\_b\_"),
            ("`[c]`", r"\[c\]"),
            ("`~d~`", r"\~d\~"),
            ("`a\\b`", r"a\\b"),
            ("``a`b``", r"a\`b"),
        ] {
            let drawn = markdown_markup(source, &shipped());
            assert!(drawn.contains(escaped), "{source} escaped to {drawn}");
        }

        let drawn = markdown_markup("the `**` marker", &shipped());
        let rendered = format!(
            "{:?}",
            StyledText::from_markdown(&drawn).expect("the escaped row parses")
        );
        assert!(
            rendered.contains(r#"text: "the ** marker""#),
            "the code body must render literally, got {rendered}"
        );

        let drawn = markdown_markup(r##"the `<font color="#8f9088">` span"##, &shipped());
        let rendered = format!(
            "{:?}",
            StyledText::from_markdown(&drawn).expect("a tag in a code span parses")
        );
        assert!(
            rendered.contains(r##"text: "the <font color=\"#8f9088\"> span""##),
            "the probe text must stay literal, got {rendered}"
        );
    }

    #[test]
    fn a_message_that_mentions_a_tag_draws_it() {
        // a handover line talking about `</font>` used to emit a real closing tag
        assert!(parses("the `</font>` overlap is fixed"));
        assert!(parses("write <font color=\"#fff\"> by hand"));
        assert!(parses("**bold `</font>` bold**"));
        assert!(parses("a < b && c > d"));
        let drawn = markdown_markup("the `</font>` overlap", &shipped());
        assert!(drawn.contains("&lt;/font&gt;"), "got {drawn}");
    }

    #[test]
    fn bold_running_over_a_line_break_is_balanced() {
        assert!(parses("**starts here\nand ends here**"));
        assert!(parses("**open\nstill open"));
    }

    #[test]
    fn a_quote_markup_draws_a_bar_not_a_source_marker() {
        let drawn = markdown_markup("> quoted", &shipped());
        assert_eq!(
            drawn,
            "<font color=\"#484944\">│</font> <font color=\"#909189\">quoted</font>"
        );
        assert!(!drawn.contains('›'), "got {drawn}");
        let escaped = markdown_markup("> a < b & c", &shipped());
        assert!(escaped.contains("&lt; b &amp; c"), "got {escaped}");
    }

    #[test]
    fn quote_markup_keeps_a_literal_asterisk() {
        let drawn = quote_markup("a * b", &shipped());
        let rendered = format!(
            "{:?}",
            StyledText::from_markdown(&drawn).expect("a literal asterisk quote parses")
        );
        assert!(rendered.contains(r#"text: "a * b""#), "got {rendered}");
    }

    #[test]
    fn quote_markup_keeps_inline_style_and_adds_emphasis() {
        let drawn = quote_markup("one **two** and `three`", &shipped());
        let rendered = format!(
            "{:?}",
            StyledText::from_markdown(&drawn).expect("a styled quote parses")
        );
        assert!(rendered.contains("Emphasis"), "got {rendered}");
        assert!(rendered.contains("Strong"), "got {rendered}");
        assert!(rendered.contains("Color"), "got {rendered}");
        assert!(!drawn.contains("&gt;"), "got {drawn}");
    }

    #[test]
    fn plain_rows_still_parse() {
        assert!(StyledText::from_markdown(&plain_markup("just words\n    and indented"))
            .is_ok());
    }

    #[test]
    #[should_panic(expected = "transcript markup must parse")]
    fn malformed_markup_is_not_hidden_by_a_plain_text_fallback() {
        let _ = styled("**<font color=\"#8f9088\">**</font>**");
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
            let _ = styled(&markdown_markup(sample, &shipped()));
        }
    }
}

#[cfg(test)]
mod custom_message_tests {
    use super::*;
    use serde_json::{json, Value};

    fn one(message: Value) -> Vec<Row> {
        build(&[message], 20).0
    }

    #[test]
    fn display_false_custom_rows_stay_out_of_the_transcript() {
        let rows = one(json!({
            "role": "custom",
            "customType": "ipython_state",
            "display": false,
            "content": "[python-state] internal"
        }));
        assert!(rows.is_empty());
    }

    #[test]
    fn a_slash_command_echo_keeps_the_command_as_its_title() {
        let rows = one(json!({
            "role": "custom",
            "customType": "session_slash_command",
            "display": true,
            "content": "/refine --global tighten the policy",
            "details": {"command": {"name": "refine", "args": "--global tighten the policy", "text": "/refine --global tighten the policy"}}
        }));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, KIND_CUSTOM_MESSAGE);
        assert_eq!(rows[0].title, "/refine");
        assert!(rows[0].has_body);
        assert!(rows[0].expanded);
        assert!(plain(&rows[0].body).contains("--global tighten the policy"));
    }

    #[test]
    fn a_slash_command_result_uses_a_readable_label_and_keeps_its_body() {
        let rows = one(json!({
            "role": "custom",
            "customType": "session_slash_command_result",
            "display": true,
            "content": "Refined continual harness state: 2 edits applied.",
            "details": {"command": {"name": "refine"}, "success": true}
        }));
        assert_eq!(rows[0].title, "Refinement");
        assert!(rows[0].has_body);
        assert!(rows[0].expanded);
        assert!(!rows[0].failed);
        assert!(plain(&rows[0].body).contains("2 edits applied"));
    }

    #[test]
    fn a_failed_slash_result_is_explicitly_failed() {
        let rows = one(json!({
            "role": "custom",
            "customType": "session_slash_command_result",
            "display": true,
            "content": "Command failed: the model did not return valid JSON",
            "details": {"command": {"name": "goal"}, "success": false}
        }));
        assert_eq!(rows[0].title, "Goal");
        assert!(rows[0].failed);
        assert!(plain(&rows[0].body).contains("did not return valid JSON"));
    }

    #[test]
    fn an_agent_message_keeps_the_sender_and_body() {
        let rows = one(json!({
            "role": "custom",
            "customType": "agent_message",
            "display": true,
            "content": "[agent-message from child:visual-test-child]\n\nready when you are"
        }));
        assert_eq!(rows[0].title, "Agent message");
        assert!(rows[0].has_body);
        let body = plain(&rows[0].body);
        assert!(body.contains("from child:visual-test-child"));
        assert!(body.contains("ready when you are"));
    }

    #[test]
    fn a_background_shell_completion_has_a_readable_label() {
        let rows = one(json!({
            "role": "custom",
            "customType": "async_bash_completion",
            "display": true,
            "content": "[bash-done pid:42 exit:0]\n\nCommand: \"cargo test\"",
            "details": {"exitCode": 0, "command": "cargo test", "pid": 42}
        }));
        assert_eq!(rows[0].title, "Background shell command finished");
        assert_eq!(rows[0].meta, "exit 0");
        assert!(!rows[0].failed);
        assert!(plain(&rows[0].body).contains("cargo test"));
    }

    #[test]
    fn an_unknown_custom_type_gets_a_safe_fallback_and_body() {
        let rows = one(json!({
            "role": "custom",
            "customType": "future_notice",
            "display": true,
            "content": "a future daemon message"
        }));
        assert_eq!(rows[0].title, "Custom message");
        assert!(rows[0].has_body);
        assert!(plain(&rows[0].body).contains("a future daemon message"));
    }

    #[test]
    fn a_long_custom_body_is_visible_by_default() {
        let body = "a long agent report\n".repeat(12);
        let rows = one(json!({
            "role": "custom",
            "customType": "agent_message",
            "display": true,
            "content": body
        }));
        assert!(rows[0].has_body);
        assert!(rows[0].expanded);
        assert!(plain(&rows[0].body).contains("a long agent report"));
    }

    #[test]
    fn a_slash_command_with_arguments_keeps_the_full_command_in_the_body() {
        let rows = one(json!({
            "role": "custom",
            "customType": "session_slash_command",
            "display": true,
            "content": "/goal ship the desktop transcript fix"
        }));
        assert_eq!(rows[0].title, "/goal");
        assert!(rows[0].has_body);
        assert!(plain(&rows[0].body).contains("ship the desktop transcript fix"));
    }

    #[test]
    fn a_failed_background_shell_is_marked_failed() {
        let rows = one(json!({
            "role": "custom",
            "customType": "async_bash_completion",
            "display": true,
            "content": "[bash-done pid:42 exit:1]\n\nCommand: \"cargo test\"",
            "details": {"exitCode": 1, "command": "cargo test", "pid": 42}
        }));
        assert!(rows[0].failed);
        assert_eq!(rows[0].meta, "exit 1");
    }

    #[test]
    fn a_subagent_failure_and_exit_keep_their_reason() {
        let failed = one(json!({
            "role": "custom",
            "customType": "rlm_child_failure",
            "display": true,
            "content": "[child-failed child:nested-worker]\n\nthe worktree disappeared"
        }));
        assert_eq!(failed[0].title, "Subagent failed");
        assert!(failed[0].failed);
        assert!(plain(&failed[0].body).contains("worktree disappeared"));

        let exited = one(json!({
            "role": "custom",
            "customType": "rlm_child_terminal_notice",
            "display": true,
            "content": "[child-exited: no-reply child:nested-worker]\n\nlast words"
        }));
        assert_eq!(exited[0].title, "Subagent finished");
        assert!(!exited[0].failed);
        assert!(plain(&exited[0].body).contains("last words"));
    }

    #[test]
    fn a_refinement_outcome_reads_the_applied_edit_flags() {
        let failed = one(json!({
            "role": "custom",
            "customType": "refinement_outcome",
            "display": true,
            "content": "Refinement complete: tighten the policy",
            "details": {"edits": [
                {"action": "update", "kind": "memory", "id": "m1", "applied": false, "error": "the applier rejected the plan"}
            ]}
        }));
        assert_eq!(failed[0].title, "Refinement failed");
        assert!(failed[0].failed);
        assert_eq!(failed[0].meta, "0/1 edits");

        let partial = one(json!({
            "role": "custom",
            "customType": "refinement_outcome",
            "display": true,
            "content": "Refinement complete: tighten the policy",
            "details": {"edits": [
                {"action": "update", "kind": "memory", "id": "m1", "applied": true},
                {"action": "update", "kind": "skill", "id": "s1", "applied": false}
            ]}
        }));
        assert_eq!(partial[0].title, "Refinement partial");
        assert!(!partial[0].failed);
        assert_eq!(partial[0].meta, "1/2 edits");

        let ok = one(json!({
            "role": "custom",
            "customType": "refinement_outcome",
            "display": true,
            "content": "Refinement complete: tightened the policy",
            "details": {"edits": [{"action": "update", "kind": "memory", "id": "m1", "applied": true}]}
        }));
        assert_eq!(ok[0].title, "Refinement");
        assert!(!ok[0].failed);
        assert_eq!(ok[0].meta, "1/1 edits");
    }

    #[test]
    fn a_compaction_failure_is_failed_but_a_skip_is_not() {
        let failed = one(json!({
            "role": "custom",
            "customType": "compaction_outcome",
            "display": true,
            "content": "Auto-compaction failed",
            "details": {"outcome": "failed"}
        }));
        assert!(failed[0].failed);

        let skipped = one(json!({
            "role": "custom",
            "customType": "compaction_outcome",
            "display": true,
            "content": "Nothing to compact",
            "details": {"outcome": "skipped"}
        }));
        assert!(!skipped[0].failed);
    }
}

#[cfg(test)]
mod summary_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_compaction_summary_is_visible_before_retained_messages() {
        let messages = vec![
            json!({
                "role": "compactionSummary",
                "summary": "## Plan\nThe retained work continues here.",
                "tokensBefore": 12345,
                "retainedMessageCount": 2,
                "customInstructions": "keep the plan",
                "timestamp": 10
            }),
            json!({
                "role": "user",
                "content": [{"type": "text", "text": "the retained question"}],
                "timestamp": 11
            }),
            json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "the retained answer"}],
                "timestamp": 12
            }),
        ];
        let rows = build(&messages, 20).0;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, KIND_SUMMARY);
        assert_eq!(rows[0].title, "Context compacted");
        assert_eq!(rows[0].meta, "12345 tokens before \u{00b7} 2 retained");
        assert!(rows[0].has_body);
        assert!(rows[0].expanded);
        let body = plain(&rows[0].body);
        assert!(body.contains("The retained work continues here."));
        assert!(body.contains("keep the plan"));
        assert_eq!(rows[1].kind, KIND_USER);
        assert_eq!(rows[2].kind, KIND_AGENT);
    }

    #[test]
    fn a_branch_summary_keeps_its_source_and_body() {
        let rows = build(
            &[json!({
                "role": "branchSummary",
                "summary": "## Goal\nRecover the branch context.",
                "fromId": "branch-entry-7",
                "timestamp": 20
            })],
            20,
        )
        .0;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].kind, KIND_SUMMARY);
        assert_eq!(rows[0].title, "Branch summary");
        assert_eq!(rows[0].meta, "from branch-entry-7");
        assert!(rows[0].has_body);
        assert!(rows[0].expanded);
        assert!(plain(&rows[0].body).contains("Recover the branch context."));
    }
}

#[cfg(test)]
mod quote_tests {
    use super::*;

    #[test]
    fn a_quote_is_its_own_row_without_the_marker() {
        let rows = build(
            &[serde_json::json!({
                "role": "assistant",
                "content": [{"type": "text", "text": "before\n> one\n> two\nlazy\n\nafter"}]
            })],
            20,
        )
        .0;
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].kind, KIND_AGENT);
        assert_eq!(rows[1].kind, KIND_QUOTE);
        assert_eq!(rows[1].body, "*one*\n*two*\n*lazy*");
        assert_eq!(rows[2].kind, KIND_AGENT);
    }
}
