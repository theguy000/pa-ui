//! What the sheet must promise, asserted through its own interface.

use super::*;
use crate::transcript::{self, Row};
use std::cell::RefCell;
use std::rc::Rc;

/// The second adapter: every order written down, so the redraw discipline can
/// be asserted instead of hoped for.
struct Recording(Rc<RefCell<Vec<Order>>>);

impl Redraw for Recording {
    fn reset(&mut self, rows: &[Row]) {
        self.0.borrow_mut().push(Order::Reset { rows: rows.len() });
    }
    fn set(&mut self, at: usize, _row: &Row) {
        self.0.borrow_mut().push(Order::Set { at });
    }
    fn insert(&mut self, at: usize, _row: &Row) {
        self.0.borrow_mut().push(Order::Insert { at });
    }
    fn remove(&mut self, at: usize) {
        self.0.borrow_mut().push(Order::Remove { at });
    }
}

fn sheet() -> (Sheet, Rc<RefCell<Vec<Order>>>) {
    let orders = Rc::new(RefCell::new(Vec::new()));
    let sheet = Sheet {
        inner: RefCell::new(Inner {
            rows: Vec::new(),
            last_prepend_px: 0.0,
            index: std::collections::HashMap::new(),
            open: None,
            after_open_end: None,
            virtual_state: None,
            drawn: Box::new(Recording(orders.clone())),
        }),
        vstate: Rc::new(RefCell::new(VirtualSlintRows::new(Rc::new(slint::VecModel::default())))),
        paging: RefCell::new(Paging::new(2)),
        clipboard: RefCell::new(Rc::new(crate::sessions::copy_to_clipboard)),
    };
    (sheet, orders)
}

fn row(message: &str, kind: i32, title: &str, id: &str, body: &str) -> Row {
    Row {
        kind,
        id: id.to_string(),
        verb: String::new(),
        title: title.to_string(),
        meta: String::new(),
        custom_kind: String::new(),
        badge_secondary: String::new(),
        body: body.to_string(),
        table: None,
        raw_code: String::new(),
        code: String::new(),
        output: String::new(),
        has_code: false,
        has_output: false,
        has_body: false,
        expanded: false,
        failed: false,
        running: false,
        image: String::new(),
        message: message.to_string(),
        elapsed: String::new(),
        timestamp: String::new(),
    }
}

fn messages(sheet: &Sheet) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for row in sheet.rows().iter() {
        if out.last() != Some(&row.message) {
            out.push(row.message.clone());
        }
    }
    out
}

#[test]
fn an_error_is_one_generic_row_in_the_sheet() {
    let (sheet, orders) = sheet();
    sheet.replace(vec![row("m1", transcript::KIND_AGENT, "", "", "working")]);

    sheet.show_error("Background shell command failed: exit 1");

    let rows = sheet.rows();
    assert_eq!(rows.len(), 2, "the error is one extra row, not a panel");
    assert_eq!(rows[1].kind, transcript::KIND_ERROR);
    assert!(rows[1].body.contains("Background shell command failed: exit 1"));
    assert!(matches!(orders.borrow().last(), Some(Order::Insert { at: 1 })));
}

#[test]
fn an_error_after_an_open_answer_stays_after_that_answer() {
    let (sheet, orders) = sheet();
    sheet.replace(vec![
        row("m1", transcript::KIND_AGENT, "", "", "first"),
        row("m2", transcript::KIND_AGENT, "", "", "live"),
    ]);
    sheet.answer_began("m2");

    sheet.show_error("Background shell command failed: exit 1");

    assert_eq!(sheet.rows()[2].kind, transcript::KIND_ERROR);
    assert!(matches!(orders.borrow().last(), Some(Order::Insert { at: 2 })));
}

#[test]
fn an_open_row_stays_open_when_the_poll_rebuilds_it() {
    let (sheet, _) = sheet();
    let code = row("m1", transcript::KIND_CODE, "rust", "", "fn work() {}");
    let cell = row("m2", transcript::KIND_CELL, "shell", "call-7", "ok");
    sheet.replace(vec![code.clone(), cell.clone()]);
    sheet.toggle(0);
    assert!(sheet.rows()[0].expanded);

    sheet.merge(vec![code, cell]);

    assert!(sheet.rows()[0].expanded, "the open code block was collapsed by the poll");
    assert!(!sheet.rows()[1].expanded, "a row the reader never opened starts closed");
}

#[test]
fn a_merge_never_deletes_a_message_it_was_not_given() {
    let (sheet, _) = sheet();
    sheet.replace(vec![
        row("m1", transcript::KIND_CODE, "rust", "", "one"),
        row("m2", transcript::KIND_CODE, "sql", "", "two"),
    ]);
    sheet.merge(vec![row("m1", transcript::KIND_CODE, "rust", "", "one, edited")]);
    assert_eq!(sheet.rows().len(), 2, "the unmentioned message was dropped");
    assert_eq!(sheet.rows()[1].title, "sql");
}

#[test]
fn the_store_copy_replaces_the_answer_where_it_stands() {
    let (sheet, _) = sheet();
    sheet.replace(vec![row("m1", transcript::KIND_USER, "", "", "hi")]);
    sheet.answer_began("m2");
    sheet.merge(vec![row("m2", transcript::KIND_AGENT, "", "", "half an answer")]);
    assert_eq!(sheet.rows().len(), 2);

    sheet.answer_ended();
    sheet.merge(vec![
        row("m2", transcript::KIND_AGENT, "", "", "the whole answer"),
        row("m2", transcript::KIND_CODE, "rust", "", "fn x() {}"),
    ]);

    assert_eq!(messages(&sheet), ["m1", "m2"], "the answer was drawn twice");
    assert_eq!(sheet.rows().len(), 3);
    assert_eq!(sheet.rows()[2].body, "fn x() {}");
}

#[test]
fn a_prepended_page_survives_a_later_merge() {
    let (sheet, _) = sheet();
    sheet.replace(vec![row("m2", transcript::KIND_CODE, "rust", "", "second")]);
    sheet.prepend(vec![row("m1", transcript::KIND_CODE, "rust", "", "first")]);
    assert_eq!(messages(&sheet), ["m1", "m2"]);

    sheet.merge(vec![row("m2", transcript::KIND_CODE, "rust", "", "second, edited")]);

    assert_eq!(messages(&sheet), ["m1", "m2"], "the older page was dropped by the merge");
    assert_eq!(sheet.rows()[1].body, "second, edited");
}

#[test]
fn one_changed_row_costs_exactly_one_order() {
    let (sheet, orders) = sheet();
    sheet.replace(vec![
        row("m1", transcript::KIND_USER, "", "", "hi"),
        row("m2", transcript::KIND_CODE, "rust", "", "one"),
        row("m2", transcript::KIND_CODE, "sql", "", "two"),
    ]);
    orders.borrow_mut().clear();

    sheet.merge(vec![
        row("m1", transcript::KIND_USER, "", "", "hi"),
        row("m2", transcript::KIND_CODE, "rust", "", "one"),
        row("m2", transcript::KIND_CODE, "sql", "", "two, edited"),
    ]);

    assert_eq!(*orders.borrow(), vec![Order::Set { at: 2 }], "a changed row was removed and re-inserted");
}

#[test]
fn a_poll_that_changes_nothing_draws_nothing() {
    let (sheet, orders) = sheet();
    let page = vec![
        row("m1", transcript::KIND_USER, "", "", "hi"),
        row("m2", transcript::KIND_CODE, "rust", "", "one"),
    ];
    sheet.replace(page.clone());
    orders.borrow_mut().clear();

    sheet.merge(page);

    assert!(orders.borrow().is_empty(), "an unchanged poll still told the list to redraw");
}

#[test]
fn leaving_a_chat_mid_answer_does_not_touch_the_new_one() {
    let (sheet, _) = sheet();
    sheet.replace(vec![
        row("a1", transcript::KIND_USER, "", "", "ask a"),
        row("a2", transcript::KIND_AGENT, "", "", "answer a"),
    ]);
    sheet.answer_began("a3");
    sheet.merge(vec![row("a3", transcript::KIND_AGENT, "", "", "still writing")]);

    let b = vec![
        row("b1", transcript::KIND_USER, "", "", "ask b"),
        row("b2", transcript::KIND_AGENT, "", "", "answer b"),
        row("b3", transcript::KIND_AGENT, "", "", "answer b, more"),
    ];
    sheet.replace(b.clone());
    sheet.merge(b.clone());

    assert_eq!(messages(&sheet), ["b1", "b2", "b3"], "the stale answer ate rows from the new chat");
}

#[test]
fn rows_that_arrive_while_an_answer_is_open_land_above_it() {
    let (sheet, _) = sheet();
    sheet.replace(vec![row("m1", transcript::KIND_USER, "", "", "ask")]);
    sheet.answer_began("live");
    sheet.merge(vec![row("live", transcript::KIND_AGENT, "", "", "writing")]);

    sheet.merge(vec![row("m2", transcript::KIND_AGENT, "", "", "older news")]);

    assert_eq!(messages(&sheet), ["m1", "m2", "live"], "the answer drifted above newer rows");
}

fn settled() -> PagerState {
    PagerState {
        follows_bottom: false,
        ready: true,
        loading: false,
        has_older: true,
        content_y: -3000.0,
        content_height: 2000.0,
        height: 600.0,
    }
}

#[test]
fn a_page_is_fetched_only_at_the_top() {
    let mut state = settled();
    assert!(!should_load_older(state), "mid-transcript must not fetch");
    state.content_y = -2500.0;
    assert!(!should_load_older(state), "still below the slack band");
    state.content_y = -2200.0;
    assert!(should_load_older(state), "at the top: fetch");
    state.content_y = 0.0;
    assert!(should_load_older(state), "clamped at the very top");
}

#[test]
fn nothing_is_fetched_when_there_is_nothing_to_fetch() {
    let mut state = settled();
    state.content_y = 0.0;
    state.follows_bottom = true;
    assert!(!should_load_older(state));
    state.follows_bottom = false;
    state.has_older = false;
    assert!(!should_load_older(state));
    state.has_older = true;
    state.loading = true;
    assert!(!should_load_older(state));
    state.loading = false;
    state.ready = false;
    assert!(!should_load_older(state));
    state.ready = true;
    state.content_height = 400.0;
    assert!(!should_load_older(state));
}

#[test]
fn scroll_and_hold_guards_are_exact_opposites_when_not_dragging() {
    assert!(!may_autoscroll(true, true));
    assert!(!may_autoscroll(false, false));
    assert!(may_autoscroll(true, false));
    assert!(should_hold_offset(false, false));
    assert!(!should_hold_offset(true, false));
    assert!(!should_hold_offset(false, true));
}

#[test]
fn idle_floor_trim_evicts_prepended_pages_after_sixty_seconds_and_restores_tail_cursor() {
    let (sheet, _) = sheet();
    let tail_msgs = vec![serde_json::json!({"id": "m3", "role": "user", "timestamp": 10})];
    assert_eq!(
        sheet.accept_tail_page(&tail_msgs, "cursor-m3", "cursor-m3", true, true, false),
        Some(true)
    );
    sheet.replace(vec![
        row("m3", transcript::KIND_USER, "", "", "newest"),
        row("m4", transcript::KIND_AGENT, "", "", "reply"),
    ]);

    // Reader scrolls up and loads an older page
    let cursor = sheet.check_load_older(false, true, -100.0, 2000.0, 600.0);
    assert_eq!(cursor.as_deref(), Some("cursor-m3"));
    let (_, has_older) = sheet.complete_older_page(
        vec![row("m1", transcript::KIND_USER, "", "", "oldest")],
        false,
        "cursor-m1".to_string(),
        false,
    );
    assert!(!has_older);
    assert_eq!(messages(&sheet), ["m1", "m3", "m4"]);

    // Reader returns to floor; before 60s, nothing is trimmed
    let t0 = SystemTime::now();
    assert_eq!(sheet.check_idle_floor_trim(true, t0), None);
    assert_eq!(
        sheet.check_idle_floor_trim(true, t0 + Duration::from_secs(30)),
        None
    );

    // At 60s, rows above m3 are trimmed and `has_older` / `oldest_cursor` restore to `cursor-m3`
    let trimmed = sheet.check_idle_floor_trim(true, t0 + Duration::from_secs(60));
    assert_eq!(trimmed, Some((1, true)));
    assert_eq!(messages(&sheet), ["m3", "m4"]);
    assert!(sheet.has_older());
}

/// Non-contiguous blocks that share a fallback key (such as `:0` notices) must
/// not merge into one straddling range in `index` and walk intermediate blocks
/// below zero on `merge`.
#[test]
fn non_contiguous_blocks_with_the_same_key_do_not_straddle_or_underflow_on_merge() {
    let (sheet, orders) = sheet();
    let page = vec![
        row(":0", transcript::KIND_CUSTOM_MESSAGE, "retry", "", "first custom"),
        row("m1", transcript::KIND_USER, "", "", "hello"),
        row("m2", transcript::KIND_AGENT, "", "", "world"),
        row(":0", transcript::KIND_CUSTOM_MESSAGE, "retry", "", "second custom"),
    ];
    sheet.replace(page.clone());
    orders.borrow_mut().clear();

    sheet.merge(page);

    let bodies: Vec<String> = sheet.rows().iter().map(|r| r.body.clone()).collect();
    assert_eq!(
        bodies,
        ["first custom", "hello", "world", "second custom"],
        "merging a page with repeated non-contiguous keys must preserve every block"
    );
    assert!(
        orders.borrow().is_empty(),
        "an unchanged poll with repeated non-contiguous keys must not redraw"
    );
}

#[test]
fn a_quote_row_estimates_its_narrower_column() {
    let body = "a".repeat(140);
    let agent = row("m", transcript::KIND_AGENT, "", "", &body);
    let quote = row("m", transcript::KIND_QUOTE, "", "", &body);
    assert!(estimate_row_height(&quote) > estimate_row_height(&agent));
    assert_eq!(estimate_row_height(&quote), (3.0 * 22.5) + 12.0);
}

#[test]
fn to_slint_copies_row_fields_and_timestamp() {
    let mut r = row("m1", transcript::KIND_CELL, "ls -la", "id1", "");
    r.verb = "bash".to_string();
    r.failed = true;
    r.running = true;
    r.elapsed = "2s".to_string();
    r.timestamp = "18:25".to_string();
    let trow = to_slint(&r);
    assert_eq!(trow.kind, transcript::KIND_CELL);
    assert_eq!(trow.verb.as_str(), "bash");
    assert_eq!(trow.title.as_str(), "ls -la");
    assert!(trow.failed);
    assert!(trow.running);
    assert_eq!(trow.elapsed.as_str(), "2s");
    assert_eq!(trow.timestamp.as_str(), "18:25");

    r.running = false;
    assert!(!to_slint(&r).running);
}

#[test]
fn answer_ended_settles_running_cells_and_trailing_thinking() {
    let (sheet, _) = sheet();
    let mut think = row("m1", transcript::KIND_THINKING, "Thinking", "", "hmm");
    think.running = true;
    let mut cell = row("m1", transcript::KIND_CELL, "print(1)", "c1", "");
    cell.verb = "python".to_string();
    cell.running = true;

    sheet.answer_began("m1");
    sheet.merge(vec![think, cell]);
    assert!(sheet.rows()[0].running);
    assert!(sheet.rows()[1].running);

    sheet.answer_ended();
    assert!(!sheet.rows()[0].running);
    assert_eq!(sheet.rows()[0].title, "Thinking");
    assert!(!sheet.rows()[1].running);
}

#[test]
fn expanded_trailing_thinking_persists_when_replaced_by_store_rows() {
    let (sheet, _) = sheet();
    let mut live = crate::live::Live::new("m-live".to_string());
    live.push_thinking("considering options");
    sheet.answer_began("m-live");
    sheet.merge(live.rows());

    assert!(sheet.rows()[0].running);
    assert_eq!(sheet.rows()[0].title, "Thinking");

    sheet.toggle(0);
    assert!(sheet.rows()[0].expanded);

    let store_msgs = vec![serde_json::json!({
        "id": "m-live",
        "role": "assistant",
        "content": [{ "type": "thinking", "thinking": "considering options" }]
    })];
    let (store_rows, _) = transcript::build(&store_msgs, 10);
    sheet.answer_ended();
    sheet.merge(store_rows);

    assert!(!sheet.rows()[0].running);
    assert_eq!(sheet.rows()[0].title, "Thinking");
    assert!(sheet.rows()[0].expanded);
}

#[test]
fn cell_height_estimate_reflects_30px_card_and_expanded_well() {
    let mut r = row("m1", transcript::KIND_CELL, "print(1)", "id1", "");
    r.verb = "python".to_string();
    r.raw_code = "print(1)".to_string();
    r.code = "print(1)".to_string();
    r.has_code = true;
    assert_eq!(estimate_row_height(&r), 38.0);

    r.expanded = true;
    let code_only = estimate_row_height(&r);
    assert_eq!(code_only, 72.5);

    r.output = "1".to_string();
    r.has_output = true;
    let code_and_output = estimate_row_height(&r);
    assert_eq!(code_and_output, 103.0);

    r.has_output = false;
    r.raw_code = "x".repeat(160);
    assert!(estimate_row_height(&r) > code_only);
}

#[test]
fn expanded_cell_persists_across_live_updates_and_store_reconcile() {
    let (sheet, _) = sheet();
    let mut live = crate::live::Live::new("m-live".to_string());
    live.start_tool("call-1", serde_json::json!({ "code": "echo hi" }));
    sheet.answer_began("m-live");
    sheet.merge(live.rows());

    sheet.toggle(0);
    assert!(sheet.rows()[0].expanded);

    live.finish_tool("call-1", "hi
".to_string(), false, None, Some(12));
    sheet.merge(live.rows());
    assert!(sheet.rows()[0].expanded);
    assert!(sheet.rows()[0].has_output);

    let store_msg = serde_json::json!([
        {
            "id": "m-live",
            "role": "assistant",
            "content": [{ "type": "toolCall", "id": "call-1", "arguments": { "code": "echo hi" } }]
        },
        {
            "role": "toolResult",
            "toolCallId": "call-1",
            "content": [{ "type": "text", "text": "hi
" }],
            "details": { "durationMs": 12 }
        }
    ]);
    let (store_rows, _) = transcript::build(&serde_json::from_value::<Vec<serde_json::Value>>(store_msg).unwrap(), 10);
    sheet.answer_ended();
    sheet.merge(store_rows);

    assert!(sheet.rows()[0].expanded);
    assert_eq!(sheet.rows()[0].elapsed, "12ms");
}

#[test]
fn copy_cell_preserves_raw_code_and_invokes_clipboard_helper() {
    let (sheet, _) = sheet();
    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let copied_ref = copied.clone();
    sheet.set_clipboard_for_test(Rc::new(move |text| {
        copied_ref.borrow_mut().push(text.to_string());
        Ok(())
    }));

    let raw = "x = \"a_b\"\nprint(x) # <tag> & 'ok'";
    let msgs = vec![serde_json::json!({
        "id": "m1",
        "role": "assistant",
        "content": [
            { "type": "text", "text": "prose" },
            { "type": "toolCall", "id": "call-1", "arguments": { "code": raw } }
        ]
    })];
    let (rows, _) = transcript::build(&msgs, 10);
    assert!(rows[1].code.contains("<font"));
    assert_eq!(rows[1].raw_code, raw);

    sheet.replace(rows);
    assert!(!sheet.copy_cell(0));
    assert!(!sheet.copy_cell(99));
    assert!(copied.borrow().is_empty());

    assert!(sheet.copy_cell(1));
    assert_eq!(*copied.borrow(), vec![raw.to_string()]);

    sheet.set_clipboard_for_test(Rc::new(|_| Err("unavailable".to_string())));
    assert!(!sheet.copy_cell(1));
}

#[test]
fn copy_cell_returns_false_when_raw_code_is_empty() {
    let (sheet, _) = sheet();
    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let copied_ref = copied.clone();
    sheet.set_clipboard_for_test(Rc::new(move |text| {
        copied_ref.borrow_mut().push(text.to_string());
        Ok(())
    }));

    let mut cell = row("m1", transcript::KIND_CELL, "print(1)", "c1", "");
    cell.code = "<font color=\"#ffffff\">print(1)</font>".to_string();
    cell.has_code = true;
    sheet.replace(vec![cell]);

    assert!(!sheet.copy_cell(0));
    assert!(copied.borrow().is_empty());
}

pub(crate) fn copy_cell_callback_routes_from_app_window_to_clipboard() {
    let ui = AppWindow::new().expect("app window");
    let (sheet, model) = Sheet::new();
    let sheet = Rc::new(sheet);
    sheet.bind_ui(&ui);
    ui.set_transcript(model);

    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let copied_ref = copied.clone();
    sheet.set_clipboard_for_test(Rc::new(move |text| {
        copied_ref.borrow_mut().push(text.to_string());
        Ok(())
    }));

    let msgs = vec![serde_json::json!({
        "id": "m1",
        "role": "assistant",
        "content": [{ "type": "toolCall", "id": "call-1", "arguments": { "code": "%%bash
cargo test -p pa-ui" } }]
    })];
    let (rows, _) = transcript::build(&msgs, 10);
    sheet.replace(rows);

    assert!(ui.invoke_copy_cell(0));
    assert_eq!(*copied.borrow(), vec!["%%bash
cargo test -p pa-ui".to_string()]);
}

#[test]
fn image_height_estimate_accounts_for_thumbnail_and_padding() {
    let img = row("m1", transcript::KIND_IMAGE, "", "", "");
    assert_eq!(estimate_row_height(&img), 162.0);
}

#[test]
fn summary_and_custom_message_height_estimates_reflect_30px_card_and_expanded_well() {
    for kind in [transcript::KIND_CUSTOM_MESSAGE, transcript::KIND_SUMMARY] {
        let mut r = row("m1", kind, "Refinement", "", "Applied 1 edit");
        r.has_body = false;
        r.expanded = true;
        assert_eq!(estimate_row_height(&r), 38.0);

        r.has_body = true;
        r.expanded = false;
        assert_eq!(estimate_row_height(&r), 38.0);

        r.expanded = true;
        let one_line = estimate_row_height(&r);
        assert_eq!(one_line, 75.0);

        r.body = "word ".repeat(30);
        assert!(estimate_row_height(&r) > one_line);
    }
}

#[test]
fn copy_cell_returns_raw_source_for_code_and_table_rows() {
    let (sheet, _) = sheet();
    let copied = Rc::new(RefCell::new(Vec::<String>::new()));
    let copied_ref = copied.clone();
    sheet.set_clipboard_for_test(Rc::new(move |text| {
        copied_ref.borrow_mut().push(text.to_string());
        Ok(())
    }));

    let table = "| a | b |\n|---|--:|\n| 1 | 2 |";
    let text = format!("```python\nprint(1)\n```\n\n{table}");
    let messages = vec![serde_json::json!({
        "id": "m1",
        "role": "assistant",
        "content": [{ "type": "text", "text": text }]
    })];
    let (rows, _) = transcript::build(&messages, 10);
    assert_eq!(rows[0].kind, transcript::KIND_CODE);
    assert_eq!(rows[1].kind, transcript::KIND_TABLE);

    sheet.replace(rows);
    assert!(sheet.copy_cell(0));
    assert!(sheet.copy_cell(1));
    assert_eq!(
        *copied.borrow(),
        vec!["print(1)".to_string(), table.to_string()]
    );
}

#[test]
fn code_and_table_height_estimates_are_explicit_and_grow_with_content() {
    let mut code = row("m1", transcript::KIND_CODE, "", "", "");
    code.raw_code = "x".to_string();
    code.has_code = true;
    let short_code = estimate_row_height(&code);
    assert!(short_code > 28.0);

    code.raw_code = "x\n".repeat(12);
    assert!(estimate_row_height(&code) > short_code);

    let table = "| name | score |\n|---|---:|\n| a | 1 |";
    let mut row = row("m2", transcript::KIND_TABLE, "", "", table);
    row.raw_code = table.to_string();
    row.has_code = true;
    row.table = Some(std::sync::Arc::new(
        transcript::table_from_source(table).expect("a parsed table"),
    ));
    let short_table = estimate_row_height(&row);
    assert!(short_table > 28.0);

    let longer = format!("{table}\n| some much longer cell | 200 |");
    row.raw_code = longer.clone();
    row.table = Some(std::sync::Arc::new(
        transcript::table_from_source(&longer).expect("a parsed table"),
    ));
    assert!(estimate_row_height(&row) > short_table);
}

#[test]
fn to_slint_carries_table_cells_rows_and_column_stretch() {
    use slint::Model;

    let mut row = row("m1", transcript::KIND_TABLE, "", "", "");
    let source = "| name | score |\n|---|--:|\n| a | 1 |".to_string();
    row.raw_code = source.clone();
    row.table = Some(std::sync::Arc::new(
        transcript::table_from_source(&source).expect("a parsed table"),
    ));

    let trow = to_slint(&row);
    assert_eq!(trow.table.row_count(), 4, "one header row and one body row");
    assert_eq!(trow.table_row_indices.row_count(), 1);
    assert_eq!(trow.table_column_indices.row_count(), 2);
    assert_eq!(trow.table_widths.row_count(), 2);
    assert_eq!(trow.table.row_data(3).expect("right cell").align, 1);
}

#[test]
fn error_height_estimate_reflects_inline_card_and_wrapping() {
    let short = row("m1", transcript::KIND_ERROR, "Error:", "", "Request failed");
    let single_line = estimate_row_height(&short);
    assert_eq!(single_line, 45.5);

    let long_body = "x".repeat(130);
    let wrapped = row("m2", transcript::KIND_ERROR, "Error:", "", &long_body);
    assert!(estimate_row_height(&wrapped) > single_line);
    assert_eq!(estimate_row_height(&wrapped), (3.0 * 21.5) + 24.0);
}