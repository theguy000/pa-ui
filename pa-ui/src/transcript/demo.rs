use super::*;

/// Two synthetic rows (clean + failed) so the failure affordance can be seen
/// without waiting for a real traceback.
#[must_use]
pub fn demo_rows() -> Vec<Row> {
    let theme = crate::theme::current();
    let mut ok = Row::new(KIND_CELL);
    ok.title = "python · print(1 + 1)".to_string();
    ok.meta = "1 line".to_string();
    ok.code = highlight_python("print(1 + 1)", &theme);
    ok.output = plain_markup("2");
    ok.has_code = true;
    ok.has_output = true;

    let mut failed = Row::new(KIND_CELL);
    failed.title = "python · rows = [r for r in data]".to_string();
    failed.meta = "2 lines".to_string();
    failed.code = highlight_python("rows = [r for r in data]\nprint(rows[3][\"name\"])\n", &theme);
    failed.output = plain_markup(
        "Traceback (most recent call last):\n  File \"<cell-9>\", line 2, in <module>\n    print(rows[3][\"name\"])\n          ~~~~^^^\nKeyError: 'name'",
    );
    failed.has_code = true;
    failed.has_output = true;
    failed.failed = true;
    failed.reason = "KeyError: 'name'".to_string();
    vec![ok, failed]
}



/// A scratch message covering every block and inline style the transcript can
/// draw - `PA_DEMO=markdown` splices it in so the rendering can be looked at
/// without writing a message into a real session.
#[must_use]
pub fn markdown_demo_rows() -> Vec<Row> {
    let text = "## receipts and throughput\n\n\
        Some **bold**, some *italic*, a word of `inline code`, and a [link](https://example.com).\n\
        A second line of the same paragraph, long enough to reach the wrap and show\n\
        how the prose column ends.\n\n\
        ```python\n\
        def total(rows: list[dict]) -> float:\n    \"\"\"Sum the priced rows.\"\"\"\n    \
        return sum(row[\"cost\"] for row in rows if row.get(\"cost\"))\n\n\
        # a comment, a string, and a number\n\
        rate = 0.0025 * 1450\n\
        print(f\"total: {total([]):.2f}\", rate)\n\
        ```\n\n\
        | model | in | out | cost |\n\
        |:--|--:|--:|--:|\n\
        | deepseek-v4.1-flash | 462k | 39k | $0.00 |\n\
        | claude-4.5-sonnet | 12.4k | 2.1k | $0.19 |\n\n\
        A closing paragraph under the table, so the spacing on both edges shows.\n";
    let message = serde_json::json!({
        "role": "assistant",
        "timestamp": 0,
        "content": [{ "type": "text", "text": text }],
    });
    build(std::slice::from_ref(&message), 1).0
}
