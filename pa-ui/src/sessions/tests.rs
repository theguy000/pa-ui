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

    // Folding Idle closes its gap: its header stays, its row is drawn at the
    // header, and the Done header takes the row's place.
    let idle_folded = session_entries(&a, &i, &d, false, true, false);
    assert_eq!(
        idle_folded[2].at,
        (COLUMN_TOP + HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    let idle_row = &idle_folded[3];
    assert!(idle_row.folded);
    // A folded row is drawn at its group's header and settles nowhere: `target`
    // is where it would sit with the group open, below the header.
    assert_eq!(idle_row.at, idle_folded[2].at);
    assert_eq!(
        idle_row.target,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(
        idle_folded[4].at,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + ROW_HEIGHT) as f32
    );
    assert_eq!(
        idle_folded[5].at,
        (COLUMN_TOP + 2 * HEADER_HEIGHT + 2 * ROW_HEIGHT) as f32
    );
    assert_eq!(
        entries_height(&idle_folded),
        (COLUMN_TOP + 3 * HEADER_HEIGHT + 2 * ROW_HEIGHT) as f32
    );
}

/// A row that changes group is the one case the fade and the slide disagree
/// about when each works it out alone: one rule answers it once.
#[test]
fn a_row_that_changes_group_has_moved_and_one_that_stays_has_not() {
    let row = |id: &str, fields: serde_json::Value| {
        let mut item = resident(fields);
        item.id = id.into();
        item
    };
    let busy = || serde_json::json!({"activeSessionId": "a", "sessionId": "ua",
                                     "lifecycle": "live", "activity": "working"});
    let quiet = || serde_json::json!({"activeSessionId": "a", "sessionId": "ua",
                                      "lifecycle": "live", "activity": "idle"});
    let before = session_entries(&[row("a", busy())], &[], &[], false, false, false);
    // the turn ends: same row, now under Idle
    let after = session_entries(&[], &[row("a", quiet())], &[], false, false, false);
    let old = before.iter().find(|entry| !entry.is_header).expect("the row");
    let new = after.iter().find(|entry| !entry.is_header).expect("the row");
    assert_eq!(old.item.uuid, new.item.uuid);
    assert!(any_moved(&before, &after));
    assert!(moved_between_groups(old, new));
    assert!(!moved_between_groups(&before[0], &after[0]), "headers keep their seam");
    // and it slides in from the side of the group it left: Active sits above
    // Idle, so the row starts `slide` px above where it sat, heading down
    let from = from_y(&before, &after, 16.0);
    let start = after.iter().position(|entry| !entry.is_header).expect("the row");
    assert_eq!(from[start], old.at - 16.0);
    // a row that stays put has not moved, even if its group's y shifted
    assert!(!any_moved(&after, &after));
}

/// Fold and poll build the column the same way, so a fold moves the entries the
/// last poll drew instead of rebuilding them from lists nothing refreshed.
#[test]
fn a_fold_grows_the_column_by_the_group_it_closes() {
    let a = vec![resident(serde_json::json!({
        "activeSessionId": "a1", "sessionId": "ua1", "lifecycle": "live", "activity": "working"
    }))];
    let i = vec![resident(serde_json::json!({
        "activeSessionId": "i1", "sessionId": "ui1", "lifecycle": "live", "activity": "idle"
    }))];
    let folded = column((&a, &i, &[]), [false, true, false], (&a, &i, &[]), [false, false, false]);
    assert_eq!(
        folded.was_height - folded.height,
        ROW_HEIGHT as f32,
        "closing Idle takes one row off the column"
    );
    assert!(folded.changed());
    // the idle row starts where it was drawn, not at the y it now occupies
    let at = folded.entries[3].at;
    assert!(folded.from_y[3] > at);
}

#[test]
fn first_poll_and_subsequent_polls_compare_against_was_not_current() {
    let i = vec![resident(serde_json::json!({
        "activeSessionId": "i1", "sessionId": "ui1", "lifecycle": "live", "activity": "idle", "name": "First"
    }))];
    // First poll: `was` is empty, so `previous` is empty and `changed()` must be true.
    let first = column((&[], &i, &[]), [false, false, false], (&[], &[], &[]), [false, false, false]);
    assert!(first.previous.is_empty());
    assert!(first.changed(), "first poll must populate the sidebar");

    // Second poll with a new session: `was` has 1 row, `groups` has 2 rows.
    let i2 = vec![
        i[0].clone(),
        resident(serde_json::json!({
            "activeSessionId": "i2", "sessionId": "ui2", "lifecycle": "live", "activity": "idle", "name": "Second"
        })),
    ];
    let second = column((&[], &i2, &[]), [false, false, false], (&[], &i, &[]), [false, false, false]);
    assert!(second.changed(), "adding a session must mark the column changed");

    // Rename in place: same length and positions, different name.
    let mut renamed = i2.clone();
    renamed[0].name = "Renamed".into();
    let third = column((&[], &renamed, &[]), [false, false, false], (&[], &i2, &[]), [false, false, false]);
    assert!(third.changed(), "renaming a session must mark the column changed");
}

/// The popup's age column reads the daemon's stamp, and the two helpers are the
/// only place the window turns a date into a number: a day off would show a
/// session as used yesterday that the daemon saved today.
#[test]
fn a_stamp_reads_as_the_seconds_the_popup_counts_days_from() {
    // 2026-11-03T00:00:00Z is 20760 days after the epoch, by the same
    // arithmetic every calendar library uses.
    assert_eq!(stamp_seconds("2026-11-03T00:00:00Z"), Some(20_760 * 86_400));
    assert_eq!(
        stamp_seconds("1970-01-01T00:00:00Z"),
        Some(0),
        "the epoch is the zero the rest are measured from"
    );
    assert_eq!(stamp_seconds("2024-02-29T12:34:56Z"), Some(1_709_210_096));
    assert_eq!(stamp_seconds("2026-11-03"), None, "a date alone is not a stamp");
    assert_eq!(stamp_seconds(""), None);
    assert_eq!(
        stamp_seconds("20261103T000000000Z"),
        None,
        "a stamp without its separators is not a date"
    );
    assert_eq!(
        stamp_seconds("2026-13-03T00:00:00Z"),
        None,
        "a thirteenth month is not a date"
    );
}

/// The label counts forward from the clock it is given, so a stamp from a
/// second ago, a day ago and ten days ago read as the three words a reader
/// scans a column for.
#[test]
fn the_age_is_today_yesterday_or_the_days_since() {
    let now = 20_760 * 86_400 + 3600;
    assert_eq!(age_label_at("2026-11-03T00:59:00Z", now), "Today");
    assert_eq!(age_label_at("2026-11-02T00:59:00Z", now), "Yesterday");
    assert_eq!(age_label_at("2026-10-24T00:59:00Z", now), "10d");
}

/// A stamp the machine's clock has not reached yet is the clock disagreeing with
/// itself: the popup says today rather than a negative age.
#[test]
fn a_future_stamp_reads_as_today() {
    let row = row(serde_json::json!({
        "activeSessionId": "f1", "sessionId": "uf1", "lifecycle": "live",
        "lastActivityAt": "2999-01-01T00:00:00Z",
    }));
    assert_eq!(age_label(&row), "Today");
}

#[test]
fn a_row_with_no_readable_stamp_has_no_age() {
    let row = row(serde_json::json!({
        "activeSessionId": "n1", "sessionId": "un1", "lifecycle": "live",
    }));
    assert_eq!(age_label(&row), "");
}


fn daemon_row(id: &str, name: &str) -> SessionRow {
    SessionRow {
        id: id.to_string(),
        session_id: id.to_string(),
        name: name.to_string(),
        cwd: "C:\\work".to_string(),
        activity: "idle".to_string(),
        lifecycle: "ready".to_string(),
        last_activity: "2026-11-03T00:00:00Z".to_string(),
        ..SessionRow::default()
    }
}

#[test]
fn opening_on_a_populated_cache_lists_every_session() {
    use slint::Model;
    let rows = [
        daemon_row("1", "prime-agent"),
        daemon_row("2", "hello"),
        daemon_row("3", "do some random tool calls"),
    ];
    assert_eq!(match_rows(&rows, "").row_count(), 3);
}

#[test]
fn each_row_carries_the_sessions_id() {
    use slint::Model;
    let rows = [daemon_row("1", "prime-agent")];
    let entries = match_rows(&rows, "");
    assert_eq!(entries.row_data(0).expect("a row").id.as_str(), "1");
}

#[test]
fn an_unnamed_session_still_draws_a_name() {
    use slint::Model;
    let mut row = daemon_row("abc", "");
    row.cwd = "C:\\work\\prime-agent".to_string();
    let name = display_name(&row);
    let entries = match_rows(&[row], "");
    let glyphs = entries.row_data(0).expect("a row").glyphs;
    assert_eq!(glyphs.row_count(), name.chars().count());
    let drawn: String = (0..glyphs.row_count())
        .map(|index| glyphs.row_data(index).expect("a glyph").text.to_string())
        .collect();
    assert_eq!(drawn, "prime-agent \u{00b7} abc");
}

#[test]
fn a_cut_name_marks_only_the_letters_it_shows() {
    use slint::Model;
    let row = daemon_row("1", "alpha beta gamma delta epsilon zeta eta theta iota");
    let entries = match_rows(&[row], "beta");
    let glyphs = entries.row_data(0).expect("a row").glyphs;
    let marked: String = (0..glyphs.row_count())
        .filter(|index| glyphs.row_data(*index).expect("a glyph").matched)
        .map(|index| glyphs.row_data(index).expect("a glyph").text.to_string())
        .collect();
    assert_eq!(marked, "beta");
}

#[test]
fn an_empty_cache_draws_no_rows() {
    use slint::Model;
    assert_eq!(match_rows(&[], "").row_count(), 0);
}

#[test]
fn rebind_finds_new_session_id_by_uuid_across_daemon_restart() {
    let mut r = daemon_row("resident-9", "my chat");
    r.session_id = "uuid-stable".to_string();
    let rows = [r];
    assert_eq!(
        resolve_rebound_id(&rows, "saved-old", Some("uuid-stable")),
        Some("resident-9".to_string())
    );
    assert_eq!(
        resolve_rebound_id(&rows, "resident-9", Some("uuid-stable")),
        None
    );
}

#[test]
fn family_cost_sums_spawned_children_with_parent_session_id() {
    let mut parent = daemon_row("s1", "parent");
    parent.session_id = "uuid-parent".to_string();
    parent.cost = 1.25;
    let mut child1 = daemon_row("s2", "child1");
    child1.parent = "uuid-parent".to_string();
    child1.cost = 0.50;
    let mut child2 = daemon_row("s3", "child2");
    child2.parent = "uuid-parent".to_string();
    child2.cost = 0.25;
    let (total, has_children) = compute_family_cost(&[parent, child1, child2], "s1", 1.25);
    assert!(has_children);
    assert!((total - 2.00).abs() < 1e-6);
}

fn subagent(active: &str, session: &str, parent: &str, status: &str) -> SessionRow {
    let mut fields = serde_json::json!({
        "activeSessionId": active, "sessionId": session,
        "parentSessionId": parent, "lifecycle": "live",
    });
    if let Some(object) = fields.as_object_mut() {
        match status {
            "running" => {
                object.insert("workerState".into(), serde_json::json!("ready"));
                object.insert("activity".into(), serde_json::json!("working"));
                object.insert("isSessionActive".into(), serde_json::json!(true));
            }
            "idle" => {
                object.insert("workerState".into(), serde_json::json!("ready"));
                object.insert("activity".into(), serde_json::json!("idle"));
            }
            _ => {}
        }
    }
    row(fields)
}

#[test]
fn a_subagent_is_not_a_sidebar_session() {
    let child = subagent("child", "uuid-child", "uuid-parent", "running");
    let rows = vec![row(serde_json::json!({
        "activeSessionId": "parent", "sessionId": "uuid-parent",
        "lifecycle": "live", "workerState": "ready", "activity": "idle"
    })), child];
    let items = session_items(&rows);
    assert_eq!(items.len(), 1, "the child belongs under its parent");
    assert_eq!(items[0].id, "parent");
}
