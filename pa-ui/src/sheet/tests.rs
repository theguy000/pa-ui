//! What the sheet must promise, asserted through its own interface.

use super::*;
use crate::transcript::{self, Row};
use std::cell::RefCell;
use std::rc::Rc;

/// The second adapter: every order written down, so the redraw discipline can
/// be asserted instead of hoped for.
struct Recording(Rc<RefCell<Vec<Order>>>);

impl Redraw for Recording {
    fn reset(&mut self, rows: Vec<TRow>) {
        self.0.borrow_mut().push(Order::Reset { rows: rows.len() });
    }
    fn set(&mut self, at: usize, _row: TRow) {
        self.0.borrow_mut().push(Order::Set { at });
    }
    fn insert(&mut self, at: usize, _row: TRow) {
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
            virtual_state: None,
            drawn: Box::new(Recording(orders.clone())),
        }),
        vstate: Rc::new(RefCell::new(VirtualSlintRows::new(Rc::new(slint::VecModel::default())))),
        paging: RefCell::new(Paging::new(2)),
    };
    (sheet, orders)
}

fn row(message: &str, kind: i32, title: &str, id: &str, body: &str) -> Row {
    Row {
        kind,
        id: id.to_string(),
        title: title.to_string(),
        meta: String::new(),
        body: body.to_string(),
        code: String::new(),
        output: String::new(),
        has_code: false,
        has_output: false,
        has_body: false,
        expanded: false,
        failed: false,
        reason: String::new(),
        image: String::new(),
        message: message.to_string(),
        elapsed: String::new(),
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