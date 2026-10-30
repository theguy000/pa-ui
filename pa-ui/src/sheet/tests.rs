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
            index: std::collections::HashMap::new(),
            open: None,
            gap: None,
            drawn: Box::new(Recording(orders.clone())),
        }),
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
        expanded: false,
        failed: false,
        reason: String::new(),
        image: String::new(),
        mime: String::new(),
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
fn an_open_row_stays_open_when_the_poll_rebuilds_it() {
    let (sheet, _) = sheet();
    let code = row("m1", transcript::KIND_CODE, "rust", "", "fn work() {}");
    let cell = row("m2", transcript::KIND_CELL, "shell", "call-7", "ok");
    sheet.replace(vec![code.clone(), cell.clone()]);
    sheet.toggle(0);
    assert!(sheet.rows()[0].expanded);

    // the same two rows arrive again, collapsed, exactly as a poll delivers them
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

    // the turn closes and the store's copy of the same message arrives, longer
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

    // the reader switches chats while that answer is being written
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

    // a message the store had not published yet arrives mid-answer
    sheet.merge(vec![row("m2", transcript::KIND_AGENT, "", "", "older news")]);

    assert_eq!(messages(&sheet), ["m1", "m2", "live"], "the answer drifted above newer rows");
}

fn pair(message: &str, title: &str) -> Vec<Row> {
    vec![
        row(message, transcript::KIND_AGENT, title, "", &format!("{title} one")),
        row(message, transcript::KIND_AGENT, title, "", &format!("{title} two")),
    ]
}

fn three_messages() -> Vec<Row> {
    let mut rows = pair("m1", "first");
    rows.extend(pair("m2", "second"));
    rows.extend(pair("m3", "newest"));
    rows
}

#[test]
fn a_sheet_inside_its_budget_keeps_everything() {
    let (sheet, _) = sheet();
    sheet.replace(three_messages());
    let before = sheet.rows().len();

    let gone = sheet.narrow("m3", 10, "older history is not loaded");

    assert_eq!(gone, 0);
    assert_eq!(sheet.rows().len(), before);
    assert_eq!(messages(&sheet), ["m1", "m2", "m3"]);
}

#[test]
fn narrowing_unloads_the_history_nearest_the_newest_page() {
    let (sheet, _) = sheet();
    sheet.replace(three_messages());

    let gone = sheet.narrow("m3", 1, "older history is not loaded");

    assert_eq!(gone, 2, "the newest stretch of history did not go");
    assert_eq!(messages(&sheet), ["m1", "", "m3"], "the hole is not where it was");
    assert_eq!(sheet.rows()[2].body, "older history is not loaded");
    assert_eq!(sheet.rows()[3].message, "m3", "the newest page moved");
}

#[test]
fn the_newest_page_is_never_unloaded() {
    let (sheet, _) = sheet();
    sheet.replace(three_messages());

    sheet.narrow("m3", 0, "older history is not loaded");

    assert_eq!(messages(&sheet), ["", "m3"]);
    assert_eq!(sheet.rows().len(), 3);
}

#[test]
fn filling_the_hole_puts_the_page_back_where_it_was() {
    let (sheet, _) = sheet();
    sheet.replace(three_messages());
    sheet.narrow("m3", 1, "older history is not loaded");

    let put_back = sheet.fill(pair("m2", "second"));

    assert_eq!(put_back, 2);
    assert_eq!(messages(&sheet), ["m1", "m2", "m3"], "the page did not go back in the hole");
    assert!(!sheet.rows().iter().any(|row| row.body == "older history is not loaded"));
}

#[test]
fn narrowing_twice_leaves_one_notice_not_two() {
    let (sheet, _) = sheet();
    sheet.replace(three_messages());

    sheet.narrow("m3", 1, "older history is not loaded");
    sheet.narrow("m3", 0, "older history is not loaded");

    assert_eq!(messages(&sheet), ["", "m3"]);
    let notices = sheet.rows().iter().filter(|row| row.body == "older history is not loaded").count();
    assert_eq!(notices, 1, "a second notice stacked on the first");
}

#[test]
fn the_hole_costs_one_order_to_make_and_one_to_fill() {
    let (sheet, orders) = sheet();
    sheet.replace(three_messages());
    orders.borrow_mut().clear();

    sheet.narrow("m3", 1, "older history is not loaded");

    // two rows out, one notice in
    assert_eq!(*orders.borrow(), vec![Order::Remove { at: 3 }, Order::Remove { at: 2 }, Order::Insert { at: 2 }]);
}
