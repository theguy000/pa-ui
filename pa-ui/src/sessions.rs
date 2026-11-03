//! The sidebar's sessions: session rows, the sidebar's filter, and the name a session is shown under.

use crate::*;

#[cfg(test)]
mod tests;

/// The tag a session row carries: what `sidebar.slint` draws, and what decides
/// which group the row sits in.
///
/// The three words are the daemon's three states, which are the only states
/// there are: `Running`, `Idle` and `Inactive` from `classify_session_roster_status`.
/// The window shows them as active, idle and done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum RowTag {
    /// A resident session with a turn in flight, or a message waiting on it.
    Active,
    /// A resident session between turns.
    Idle,
    /// No worker holds it: set aside, whichever lifecycle the scan stamped on it.
    Done,
}

impl RowTag {
    /// The word `sidebar.slint` compares against, and the group the row sits
    /// under: active first, then idle, then the done rows.
    pub(crate) fn word(self) -> &'static str {
        match self {
            RowTag::Active => "active",
            RowTag::Idle => "idle",
            RowTag::Done => "done",
        }
    }

    /// The order the groups are drawn in.
    pub(crate) fn rank(self) -> u8 {
        match self {
            RowTag::Active => 0,
            RowTag::Idle => 1,
            RowTag::Done => 2,
        }
    }
}

/// The row's tag, read straight from the daemon's status formula.
///
/// `list` answers every saved session with `lifecycle: "live"` once it has a
/// message, so lifecycle cannot say whether anything is running: 53 of this
/// machine's "live" rows had no worker at all and read as idle beside the TUI's
/// single idle session. `workerState` is what a worker actually stamps on the
/// rows it holds, so residency comes from that and the busy/idle split is the
/// daemon's own.
pub(crate) fn tag_of(row: &SessionRow) -> RowTag {
    match row.roster_status() {
        "Running" => RowTag::Active,
        "Idle" => RowTag::Idle,
        _ => RowTag::Done,
    }
}

pub(crate) fn to_item(row: &SessionRow) -> SessionItem {
    let full = display_name(row);
    // every row gets a card: consistency was asked for explicitly, twice
    let tip = full.clone();
    let tag = tag_of(row);
    SessionItem {
        id: row.id.clone().into(),
        uuid: row.session_id.clone().into(),
        name: trim_to(&full, 90).into(),
        age: age_of(if row.last_activity.is_empty() {
            &row.modified
        } else {
            &row.last_activity
        })
        .into(),
        full,
        tip,
        status: tag.word().into(),
        done: tag == RowTag::Done,
        file: row.session_file.clone().into(),
    }
}

/// Every session, as the sidebar draws it, grouped: active rows first, then
/// idle, then done, each group kept in the daemon's order.
pub(crate) fn session_items(rows: &[SessionRow]) -> Vec<SessionItem> {
    let mut items: Vec<SessionItem> = rows.iter().map(to_item).collect();
    items.sort_by_key(|item| tag_word_rank(&item.status));
    items
}

/// The three groups as the window hands them to the column: each holding the
/// rows of that group in drawing order, taken from the one sorted list so a
/// group and its rows cannot disagree.
pub(crate) fn session_groups(
    items: &[SessionItem],
) -> (Vec<SessionItem>, Vec<SessionItem>, Vec<SessionItem>) {
    let take = |tag: RowTag| {
        items
            .iter()
            .filter(|item| tag_word_rank(&item.status) == tag.rank())
            .cloned()
            .collect::<Vec<SessionItem>>()
    };
    (take(RowTag::Active), take(RowTag::Idle), take(RowTag::Done))
}

/// A row's word back to its tag. The word is what the window holds, since
/// `SessionItem`'s status crosses into Slint as a string.
fn tag_word_rank(word: &str) -> u8 {
    match word {
        "active" => RowTag::Active.rank(),
        "idle" => RowTag::Idle.rank(),
        _ => RowTag::Done.rank(),
    }
}

/// The sessions column, as the window draws it: one flat list of headers and
/// rows.
///
/// The groups are drawn in the daemon's order (active, idle, done). A folded
/// group contributes its header and nothing else. Positions come from here
/// rather than from Slint's layouts so that the height of the column is a
/// number the window already has, not one a layout has to report back.
///
/// `at` is where a header stands, and where a row is drawn: the y it settles at
/// with every group open. `target` is the same number for an unfolded group;
/// inside the group a fold is moving, it is where the row would sit with that
/// group open, so the rows keep their spacing while the group's box shrinks.
/// `seam` is the group's index, which is what `fold-index` names.
pub(crate) fn session_entries(
    active: &[SessionItem],
    idle: &[SessionItem],
    done: &[SessionItem],
    active_folded: bool,
    idle_folded: bool,
    done_folded: bool,
) -> Vec<SessionEntry> {
    let mut entries = Vec::with_capacity(active.len() + idle.len() + done.len() + 3);
    let mut at: f32 = COLUMN_TOP as f32;
    for (index, label, rows, folded) in [
        (0, "Active", active, active_folded),
        (1, "Idle", idle, idle_folded),
        (2, "Done", done, done_folded),
    ] {
        let header_at = at;
        entries.push(SessionEntry {
            is_header: true,
            label: label.into(),
            count: rows.len() as i32,
            folded,
            item: SessionItem::default(),
            at: header_at,
            target: header_at,
            seam: index,
        });
        at += HEADER_HEIGHT as f32;
        for (number, row) in rows.iter().enumerate() {
            let row_target = header_at + HEADER_HEIGHT as f32 + number as f32 * ROW_HEIGHT as f32;
            let row_at = if folded { header_at } else { row_target };
            entries.push(SessionEntry {
                is_header: false,
                label: "".into(),
                count: 0,
                folded,
                item: row.clone(),
                at: row_at,
                target: row_target,
                seam: index,
            });
        }
        if !folded {
            at += rows.len() as f32 * ROW_HEIGHT as f32;
        }
    }
    entries
}

/// One key per entry, the same before and after a fold: a header by its word, a
/// row by its session's uuid. A row keeps its key when it moves between groups,
/// which is how a poll tells it slid rather than arrived.
pub(crate) fn entry_key(entry: &SessionEntry) -> String {
    if entry.is_header {
        format!("hdr:{}", entry.label)
    } else {
        format!("row:{}", entry.item.uuid)
    }
}

/// Where each entry was drawn before the column was rebuilt, keyed by
/// `entry_key`. `from_y` is aligned with `entries`, so `row-y` in `app.slint`
/// slides one row from that row's own old y, whatever order either list is in.
///
/// A row that changed group has no shared history with the row it replaced in
/// that slot, so it drops in from `slide` px on the side of the group it left:
/// `-slide` from below, `+slide` from above. Fades are `app.slint`'s business.
pub(crate) fn from_y(previous: &[SessionEntry], entries: &[SessionEntry], slide: f32) -> Vec<f32> {
    let before: std::collections::HashMap<String, &SessionEntry> = previous
        .iter()
        .map(|entry| (entry_key(entry), entry))
        .collect();
    entries
        .iter()
        .map(|entry| match before.get(&entry_key(entry)) {
            Some(old) if old.seam == entry.seam => old.at,
            Some(old) => entry.at + if old.seam > entry.seam { slide } else { -slide },
            None => entry.at,
        })
        .collect()
}

/// The column's height for a set of entries, and its bottom edge.
pub(crate) fn entries_height(entries: &[SessionEntry]) -> f32 {
    entries
        .iter()
        .filter(|entry| entry.is_header || !entry.folded)
        .map(|entry| {
            entry.at
                + if entry.is_header {
                    HEADER_HEIGHT as f32
                } else {
                    ROW_HEIGHT as f32
                }
        })
        .fold(COLUMN_TOP as f32, f32::max)
}

/// A header is 32px and a row 32px, both in `sidebar.slint` and `app.slint`:
/// the window draws its entries at these heights, so the two must agree.
pub(crate) const HEADER_HEIGHT: i32 = 32;
pub(crate) const ROW_HEIGHT: i32 = 32;
/// Where the first header goes: below the column's band, the New Session button
/// and the search row. The list is pinned to the window's top edge, so this
/// offset is the entries' own, and `app.slint` places the list at the same y.
pub(crate) const COLUMN_TOP: i32 = 88;

/// The session the selection follows: ids are re-keyed by a daemon restart,
/// the session's uuid is not.
pub(crate) fn uuid_of<'a>(rows: &'a [SessionRow], id: &str) -> Option<&'a str> {
    rows.iter()
        .find(|row| row.id == id)
        .map(|row| row.session_id.as_str())
}

pub(crate) fn display_name(row: &SessionRow) -> SharedString {
    let name = row.name.trim();
    let usable = !name.is_empty() && name != "(no messages)";
    if usable {
        return trim_to(name, 80).into();
    }
    let tail = |text: &str, width: usize| -> String {
        let count = text.chars().count();
        text.chars().skip(count.saturating_sub(width)).collect()
    };
    if let Some(last) = row.cwd.split(['\\', '/']).rfind(|part| !part.is_empty()) {
        return format!("{last} \u{00b7} {}", tail(&row.id, 6)).into();
    }
    tail(&row.id, 10).into()
}
