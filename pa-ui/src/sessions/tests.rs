//! What the sidebar's rows must say, asserted on the items the window hands the
//! list: the status word `sidebar.slint` compares, built from the daemon's wire
//! JSON.
//!
//! The rule under test is the daemon's own: a resident session is active while
//! it holds a turn and idle between turns, and anything no worker holds is done.

use super::*;

/// One `list` row as the daemon sends it.
fn row(fields: serde_json::Value) -> SessionRow {
    SessionRow::from_value(&fields)
}

fn item(fields: serde_json::Value) -> SessionItem {
    to_item(&row(fields))
}

/// A resident row: the daemon stamps `workerState` on the rows a worker holds.
fn resident(fields: serde_json::Value) -> SessionItem {
    let mut fields = fields;
    if let Some(object) = fields.as_object_mut() {
        object.insert("workerState".into(), serde_json::json!("ready"));
    }
    item(fields)
}

/// A saved row off disk: `lifecycle: live` whenever it has a message, and no
/// `workerState`, because no worker is holding it.
fn saved(fields: serde_json::Value) -> SessionItem {
    item(fields)
}

#[test]
fn a_resident_session_holding_a_turn_is_active() {
    let item = resident(serde_json::json!({
        "activeSessionId": "s1", "sessionId": "u1", "lifecycle": "live",
        "activity": "working", "isSessionActive": true, "isStreaming": true,
    }));
    assert_eq!(item.status, "active");
    assert!(!item.done);
}

/// The daemon keeps `isSessionActive` true for a parked message and for a
/// compaction, and its roster classifies on that key. A session waiting on its
/// queue is not between turns.
#[test]
fn a_resident_session_waiting_on_its_queue_is_active() {
    let item = resident(serde_json::json!({
        "activeSessionId": "s1", "sessionId": "u1", "lifecycle": "live",
        "activity": "idle", "isSessionActive": true, "isStreaming": false,
    }));
    assert_eq!(item.status, "active");
}

#[test]
fn a_resident_session_between_turns_is_idle() {
    let item = resident(serde_json::json!({
        "activeSessionId": "s1", "sessionId": "u1", "lifecycle": "live",
        "activity": "idle", "isSessionActive": false, "isStreaming": false,
    }));
    assert_eq!(item.status, "idle");
}

/// The bug this rule was rewritten for: `list {all: true}` re-sends every saved
/// session with `lifecycle: "live"` once it has a message, and the daemon writes
/// `activity: "idle"` on all of them. 53 rows of a 54-row "idle" section on this
/// machine had no worker. They are done, not idle.
#[test]
fn a_saved_session_wearing_live_is_done_not_idle() {
    let item = saved(serde_json::json!({
        "activeSessionId": "s1", "sessionId": "u1", "lifecycle": "live",
        "activity": "idle", "isSessionActive": false, "isStreaming": false,
        "messageCount": 37,
    }));
    assert_eq!(item.status, "done");
    assert!(item.done);
}

/// A stopped session comes back from the daemon as `archived`, with no worker.
#[test]
fn a_stopped_session_is_done() {
    let item = saved(serde_json::json!({
        "activeSessionId": "s1", "sessionId": "u1", "lifecycle": "archived",
        "activity": "idle", "isSessionActive": false,
    }));
    assert_eq!(item.status, "done");
    assert!(item.done);
}

/// The `list` command reaches the daemon with `all: true`, so a draft row is on
/// the wire and has to say what it is.
#[test]
fn a_row_without_a_worker_is_done_whatever_its_lifecycle() {
    for lifecycle in ["draft", "live", "archived", ""] {
        let status = saved(serde_json::json!({
            "activeSessionId": "s1", "sessionId": "u1", "lifecycle": lifecycle,
            "activity": "working", "isSessionActive": true,
        }))
        .status;
        assert_eq!(status, "done", "{lifecycle:?}");
    }
}

/// A resident row outranks a saved one however the daemon ordered them, and a
/// saved row never pairs with an active one: group order is active, idle, done.
#[test]
fn the_list_is_grouped_active_then_idle_then_done() {
    let rows = vec![
        row(
            serde_json::json!({"activeSessionId": "d", "sessionId": "u4",
                               "lifecycle": "archived"}),
        ),
        row(
            serde_json::json!({"activeSessionId": "i", "sessionId": "u3",
                               "lifecycle": "live", "workerState": "ready"}),
        ),
        row(
            serde_json::json!({"activeSessionId": "a", "sessionId": "u2",
                               "lifecycle": "live", "workerState": "ready",
                               "activity": "working"}),
        ),
        row(
            serde_json::json!({"activeSessionId": "s", "sessionId": "u1",
                               "lifecycle": "live", "activity": "idle"}),
        ),
    ];
    let items = session_items(&rows);
    let statuses: Vec<&str> = items.iter().map(|item| item.status.as_str()).collect();
    assert_eq!(statuses, ["active", "idle", "done", "done"]);
    // and the split the window draws from keeps that order: the same rows, the
    // same way, and no row in two groups
    let (active, idle, done) = session_groups(&items);
    assert_eq!(active.len(), 1);
    assert_eq!(idle.len(), 1);
    assert_eq!(done.len(), 2);
    assert_eq!(
        active
            .iter()
            .chain(&idle)
            .chain(&done)
            .map(|item| item.id.clone())
            .collect::<Vec<_>>(),
        items.iter().map(|item| item.id.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn session_entries_positions_headers_and_rows_by_fold_state() {
    let a = vec![resident(serde_json::json!({
        "activeSessionId": "a1", "sessionId": "ua1", "lifecycle": "live", "activity": "working"
    }))];
    let i = vec![resident(serde_json::json!({
        "activeSessionId": "i1", "sessionId": "ui1", "lifecycle": "live", "activity": "idle"
    }))];
    let d = vec![saved(serde_json::json!({
        "activeSessionId": "d1", "sessionId": "ud1", "lifecycle": "archived"
    }))];

    let open = session_entries(&a, &i, &d, false, false, false);
    assert_eq!(open.len(), 6);
    assert_eq!(open[0].at, COLUMN_TOP as f32);
    assert_eq!(open[1].at, (COLUMN_TOP + HEADER_HEIGHT) as f32);
    assert_eq!(open[2].at, (COLUMN_TOP + HEADER_HEIGHT + ROW_HEIGHT) as f32);
    assert_eq!(
        open[3].at,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(
        open[4].at,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + 2 * ROW_HEIGHT) as f32
    );
    assert_eq!(
        open[5].at,
        (COLUMN_TOP + 3 * HEADER_HEIGHT + 2 * ROW_HEIGHT) as f32
    );
    assert_eq!(
        entries_height(&open),
        (COLUMN_TOP + 3 * HEADER_HEIGHT + 3 * ROW_HEIGHT) as f32
    );

    let idle_folded = session_entries(&a, &i, &d, false, true, false);
    assert_eq!(
        idle_folded[2].at,
        (COLUMN_TOP + HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(idle_folded[3].at, idle_folded[2].at);
    assert!(idle_folded[3].folded);
    assert_eq!(
        idle_folded[4].at,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(
        idle_folded[5].at,
        (COLUMN_TOP + 3 * HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(
        entries_height(&idle_folded),
        (COLUMN_TOP + 3 * HEADER_HEIGHT + 2 * ROW_HEIGHT) as f32
    );
}
