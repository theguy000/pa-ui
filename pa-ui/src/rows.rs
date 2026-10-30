//! Rows: turning messages into transcript rows, and keeping the rows on screen in step with them.

use crate::*;

/// Write the built transcript to `PA_DUMP`, so a row that appears twice can be
/// counted instead of guessed at.
/// Merge a rebuild into the rows already on screen, message by message.
///
/// This is the TUI's model, transplanted: its chat holds one entry per message
/// and a rebuild replaces an entry it was given, appends one it has not seen, and
/// leaves the rest alone. A flat row list can do the same as long as every row
/// knows its message, which is what `Row.message` is for.
///
/// The grouping of the store is computed once, not once per incoming message, and
/// a message whose row count did not change is replaced in place � `set_row_data`
/// instead of a remove/insert pair � because every model mutation is a layout
/// notification and a transcript that jitters while it streams is the result.
pub(crate) fn group_ranges(rows: &[transcript::Row]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match out.last_mut() {
            Some((key, range)) if *key == row.message => range.end = index + 1,
            _ => out.push((row.message.clone(), index..index + 1)),
        }
    }
    out
}


/// Put a page of older rows above what is on screen.
///
/// Both copies of the transcript move together � the store, which is the truth
/// here, and the model Slint draws � because a prepend that touched only one of
/// them would leave the next merge operating on rows nobody can see.
pub fn prepend_rows(
    store: &mut Vec<transcript::Row>,
    older: &[transcript::Row],
    model: &Rc<VecModel<TRow>>,
) {
    if older.is_empty() {
        return;
    }
    let mut combined = Vec::with_capacity(store.len() + older.len());
    combined.extend_from_slice(older);
    combined.append(store);
    *store = combined;
    // one insertion per row: a ListView only keeps the rows it draws, so this
    // costs the rows inserted, not the transcript
    let slint_rows: Vec<TRow> = older.iter().map(to_slint).collect();
    for (offset, row) in slint_rows.into_iter().enumerate() {
        model.insert(offset, row);
    }
}


pub(crate) fn merge_rows(
    store: &mut Vec<transcript::Row>,
    incoming: Vec<transcript::Row>,
    model: &Rc<VecModel<TRow>>,
) {
    if store.is_empty() {
        let slint_rows: Vec<TRow> = incoming.iter().map(to_slint).collect();
        *store = incoming;
        model.set_vec(slint_rows);
        return;
    }
    let mut store_map: std::collections::HashMap<String, std::ops::Range<usize>> =
        group_ranges(store).into_iter().collect();
    for (key, range) in group_ranges(&incoming) {
        let fresh = &incoming[range];
        match store_map.get(&key).cloned() {
            // same shape: rewrite the rows where they stand, one notification each
            Some(current) if current.len() == fresh.len() => {
                for (offset, row) in fresh.iter().enumerate() {
                    let index = current.start + offset;
                    // a rebuilt row is collapsed by default; what the reader
                    // opened must not close under them every poll
                    let was_open = store[index].expanded;
                    store[index] = row.clone();
                    store[index].expanded = was_open;
                    model.set_row_data(index, to_slint(&store[index]));
                }
            }
            Some(current) => {
                let at = current.start.min(store.len());
                let until = current.end.min(store.len());
                let opened = opened_rows(&store[at..until]);
                for index in (at..until).rev() {
                    store.remove(index);
                    model.remove(index);
                }
                for (offset, row) in fresh.iter().enumerate() {
                    let mut row = row.clone();
                    if opened.contains(&row_identity(&row)) {
                        row.expanded = true;
                    }
                    store.insert(at + offset, row.clone());
                    model.insert(at + offset, to_slint(&row));
                }
                // the indices after this message moved: re-group once
                store_map = group_ranges(store).into_iter().collect();
            }
            None => {
                let at = store.len();
                for (offset, row) in fresh.iter().enumerate() {
                    store.insert(at + offset, row.clone());
                    model.insert(at + offset, to_slint(row));
                }
                store_map.insert(key, at..at + fresh.len());
            }
        }
    }
}


/// Stable identity of a row across a rebuild: the tool call's id when it has
/// one, otherwise the message it belongs to plus its kind and its place among
/// the rows of that kind.
pub(crate) fn row_identity(row: &transcript::Row) -> String {
    if !row.id.is_empty() {
        return format!("id:{}", row.id);
    }
    format!("{}#{}#{}", row.message, row.kind, row.title)
}


/// Which rows of a range the reader had opened.
pub(crate) fn opened_rows(rows: &[transcript::Row]) -> std::collections::HashSet<String> {
    rows.iter()
        .filter(|row| row.expanded)
        .map(row_identity)
        .collect()
}


pub(crate) fn dump_rows(rows: &[Row]) {
    let Some(path) = crate::env_cfg::cfg().dump.clone() else {
        return;
    };
    let dump: String = rows
        .iter()
        .map(|row| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\n",
                row.kind,
                row.id,
                row.title.replace('\n', " "),
                row.meta.replace('\n', " "),
                row.elapsed.replace('\n', " "),
                row.body.replace('\n', " ").chars().take(90).collect::<String>()
            )
        })
        .collect();
    let _ = std::fs::write(path, dump);
}


/// Every change to the row model, logged in order: the shape of a send is a
/// sequence of appends, rebuilds and patches, and a jump is one of them landing
/// in the wrong order. Only compiled in at PA_DEBUG.
/// Walk the open message's range back to what it looks like now: the old rows
/// go, the new ones take their place. Called for every frame, because the range
/// is the message - there is no second copy to keep in step.
/// Write the open turn into the model without rebuilding its rows.
///
/// This used to remove every row of the live turn and insert them again on each
/// streamed delta. Every one of those was a model mutation, and a mutation makes
/// the list relayout � and its relayout rewrites the scroll offset from an
/// average row height. That is what moved the view under a reader who had not
/// scrolled. A delta now changes the text of the rows that exist; rows are only
/// inserted when the turn actually grows a new one (a thinking row starting, a
/// tool cell opening, a code block splitting off).
pub(crate) fn sync_live(
    live: &mut Option<live::Live>,
    store: &Rc<RefCell<Vec<transcript::Row>>>,
    model: &Rc<VecModel<TRow>>,
) {
    let Some(turn) = live.as_mut() else {
        return;
    };
    let fresh = turn.rows();
    let mut rows_store = store.borrow_mut();
    let start = turn.start.min(rows_store.len());
    let before = turn.rows.min(rows_store.len().saturating_sub(start));
    let after = fresh.len();

    // 1. the rows both sides have: change the data, not the element
    for offset in 0..before.min(after) {
        let index = start + offset;
        let row = &fresh[offset];
        if &rows_store[index] == row {
            continue;
        }
        rows_store[index] = row.clone();
        model.set_row_data(index, to_slint(row));
    }

    // 2. the turn grew: append what is new, in order
    if after > before {
        for offset in before..after {
            let index = start + offset;
            let row = &fresh[offset];
            rows_store.insert(index, row.clone());
            model.insert(index, to_slint(row));
        }
    }

    // 3. the turn shrank (a partial block folded back into prose)
    if before > after {
        for index in (start + after..start + before).rev() {
            if index < rows_store.len() {
                rows_store.remove(index);
                model.remove(index);
            }
        }
    }

    turn.rows = after;
}


/// Drop the open range: the store speaks for that message from here on.
pub(crate) fn close_live(
    live: &mut Option<live::Live>,
    store: &Rc<RefCell<Vec<transcript::Row>>>,
    model: &Rc<VecModel<TRow>>,
) {
    let _ = &store;
    if let Some(turn) = live.take() {
        let mut rows_store = store.borrow_mut();
        let end = (turn.start + turn.rows).min(rows_store.len());
        let begin = turn.start.min(end);
        for index in (begin..end).rev() {
            rows_store.remove(index);
            model.remove(index);
        }
    }
}


pub(crate) fn trace_rows(what: &str, rows: &Rc<RefCell<Vec<transcript::Row>>>) {
    if !crate::env_cfg::cfg().debug {
        return;
    }
    let rows = rows.borrow();
    let tail: Vec<String> = rows
        .iter()
        .rev()
        .take(2)
        .rev()
        .map(|row| {
            let body: String = row.body.chars().take(18).collect();
            format!("{}:{:?}", row.kind, body)
        })
        .collect();
    eprintln!("[rows] {what:24} n={:<4} tail={:?}", rows.len(), tail);
}


pub(crate) fn to_slint(row: &Row) -> TRow {
    TRow {
        kind: row.kind,
        title: transcript::plain(&row.title).into(),
        meta: transcript::plain(&row.meta).into(),
        body: transcript::styled(&row.body),
        code: transcript::styled(&row.code),
        output: transcript::styled(&row.output),
        has_code: row.has_code,
        has_output: row.has_output,
        expanded: row.expanded,
        failed: row.failed,
        reason: transcript::plain(&row.reason).into(),
        image: images::from_wire(&row.image, &row.mime),
        elapsed: row.elapsed.clone().into(),
    }
}


#[cfg(test)]
mod row_state_tests {
    use super::{merge_rows, row_identity, to_slint, TRow};
    use crate::transcript::{self, Row};
    use slint::{Model, VecModel};
    use std::rc::Rc;

    fn row(message: &str, kind: i32, title: &str, id: &str) -> Row {
        let mut row = transcript::Row {
            kind,
            id: id.to_string(),
            title: title.to_string(),
            meta: String::new(),
            body: format!("body of {title}"),
            code: String::new(),
            output: String::new(),
            has_code: false,
            has_output: false,
            expanded: false,
            failed: false,
            reason: String::new(),
            image: String::new(),
            mime: String::new(),
            elapsed: String::new(),
            message: message.to_string(),
        };
        row.expanded = false;
        row
    }

    fn model_of(rows: &[Row]) -> Rc<VecModel<TRow>> {
        Rc::new(VecModel::<TRow>::from(rows.iter().map(to_slint).collect::<Vec<_>>()))
    }

    #[test]
    fn an_open_row_stays_open_when_the_poll_rebuilds_it() {
        let mut store = vec![row("m1", transcript::KIND_CODE, "rust", ""),
                             row("m2", transcript::KIND_CELL, "shell", "call-7")];
        store[0].expanded = true;
        let model = model_of(&store);

        // the same two rows arrive again, collapsed, as a poll would deliver them
        let incoming = vec![row("m1", transcript::KIND_CODE, "rust", ""),
                            row("m2", transcript::KIND_CELL, "shell", "call-7")];
        merge_rows(&mut store, incoming, &model);

        assert!(store[0].expanded, "the open code block was collapsed by the poll");
        assert!(!store[1].expanded);
        assert!(model.row_data(0).unwrap().expanded);
    }

    #[test]
    fn an_open_cell_survives_the_message_growing() {
        // a message that gains a row: the group is re-inserted whole, so the
        // toggle has to be carried by identity rather than by position
        let mut store = vec![row("m1", transcript::KIND_CELL, "shell", "call-1"),
                             row("m1", transcript::KIND_CELL, "python", "call-2")];
        store[1].expanded = true;
        let model = model_of(&store);

        let incoming = vec![row("m1", transcript::KIND_CELL, "shell", "call-1"),
                            row("m1", transcript::KIND_CELL, "python", "call-2"),
                            row("m1", transcript::KIND_CODE, "rust", "")];
        merge_rows(&mut store, incoming, &model);

        assert_eq!(store.len(), 3);
        assert!(store[1].expanded, "the open python cell closed when a row was added");
        assert!(!store[2].expanded, "a row the reader never opened starts closed");
        assert!(model.row_data(1).unwrap().expanded);
    }

    #[test]
    fn a_message_the_store_does_not_mention_is_left_exactly_as_drawn() {
        // the merge appends and replaces, it never deletes: a store that is one
        // message behind must not tear rows off the transcript, and the toggle
        // on those rows belongs to the reader
        let mut store = vec![row("m1", transcript::KIND_CODE, "rust", ""),
                             row("m2", transcript::KIND_CODE, "sql", "")];
        store[1].expanded = true;
        let model = model_of(&store);
        merge_rows(&mut store, vec![row("m1", transcript::KIND_CODE, "rust", "")], &model);
        assert_eq!(store.len(), 2, "the unmentioned message was dropped");
        assert!(store[1].expanded);
        assert_eq!(model.row_count(), 2);
    }

    #[test]
    fn identity_prefers_the_call_id() {
        let mut with_id = row("m1", transcript::KIND_CELL, "shell", "call-9");
        with_id.expanded = true;
        assert_eq!(row_identity(&with_id), "id:call-9");
        let plain = row("m1", transcript::KIND_CODE, "rust", "");
        assert!(row_identity(&plain).starts_with("m1#6#rust"));
    }
}
