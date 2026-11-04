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
        full,
        tip,
        status: tag.word().into(),
        done: tag == RowTag::Done,
        file: row.session_file.clone().into(),
    }
}

/// When the session was last used, as the search popup says it: a day the reader
/// is likely to think in, so today and yesterday by name and older days counted.
pub(crate) fn age_label(row: &SessionRow) -> SharedString {
    let stamp = if row.last_activity.is_empty() {
        &row.modified
    } else {
        &row.last_activity
    };
    let Some(seconds) = stamp_seconds(stamp) else {
        return SharedString::new();
    };
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // A stamp in the future is the clock disagreeing with itself, not a session
    // used later today: read it as today rather than as a negative age.
    match (now - seconds).max(0) / 86_400 {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        days => format!("{days}d").into(),
    }
}

/// A `YYYY-MM-DDTHH:MM:SS` stamp as seconds since the epoch, or `None` when the
/// stamp is too short to read. The zone suffix is ignored, so a stamp one zone
/// away from the machine only moves the day boundary by hours.
fn stamp_seconds(stamp: &str) -> Option<i64> {
    if stamp.len() < 19 {
        return None;
    }
    let num = |range: std::ops::Range<usize>| {
        stamp.get(range).and_then(|part| part.parse::<i64>().ok())
    };
    let year = num(0..4)?;
    let month = num(5..7)?;
    let day = num(8..10)?;
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let second = num(17..19)?;
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

/// Days since 1970-01-01 for a civil date, by Howard Hinnant's `days_from_civil`,
/// which is exact for every date the calendar has.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
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
            Some(old) if !moved_between_groups(old, entry) => old.at,
            Some(old) => old.at + if old.seam > entry.seam { slide } else { -slide },
            None => entry.at,
        })
        .collect()
}

/// Whether an entry slid to another group since the column was last drawn. This
/// is the only place the question is answered: `from_y` drops such an entry in
/// from the side, and `app.slint` fades it in over `arrived` while a fold or a
/// poll is in flight. A check on the distance before and after would answer
/// differently for a row that moved to a group that lands it at the same y.
pub(crate) fn moved_between_groups(previous: &SessionEntry, entry: &SessionEntry) -> bool {
    previous.seam != entry.seam
}

/// Whether any entry slid to another group. What the window asks after a poll,
/// before it decides which animation to run.
pub(crate) fn any_moved(previous: &[SessionEntry], entries: &[SessionEntry]) -> bool {
    let before: std::collections::HashMap<String, &SessionEntry> = previous
        .iter()
        .map(|entry| (entry_key(entry), entry))
        .collect();
    entries
        .iter()
        .any(|entry| before.get(&entry_key(entry)).is_some_and(|old| moved_between_groups(old, entry)))
}

/// The column as one rebuild writes it: the entries the window draws, the y
/// each one started from, how tall the column was and is, and whether a row or a
/// group's border has moved at all.
///
/// Fold and poll go through here, so a fold state change and a row arriving from
/// the daemon produce the same four numbers, and neither path can drift from
/// the other.
pub(crate) struct Column {
    pub(crate) previous: Vec<SessionEntry>,
    pub(crate) entries: Vec<SessionEntry>,
    /// aligned with `entries`: the y each one starts from
    pub(crate) from_y: Vec<f32>,
    pub(crate) was_height: f32,
    pub(crate) height: f32,
}

impl Column {
    /// Whether anything moved or changed: the first fill (when `previous` is
    /// empty), a row added or removed, a row between groups, an entry at a new
    /// y, or a row's visible fields updated.
    pub(crate) fn changed(&self) -> bool {
        self.entries.len() != self.previous.len()
            || self
                .entries
                .iter()
                .zip(&self.from_y)
                .any(|(entry, from)| (entry.at - from).abs() > 0.5)
            || self
                .entries
                .iter()
                .zip(&self.previous)
                .any(|(a, b)| {
                    a.is_header != b.is_header
                        || a.label != b.label
                        || a.count != b.count
                        || a.folded != b.folded
                        || a.item.id != b.item.id
                        || a.item.name != b.item.name
                        || a.item.status != b.item.status
                        || a.item.done != b.item.done
                })
    }
}

/// Borrowed rows per group, in the order the window draws them.
pub(crate) type Groups<'a> = (&'a [SessionItem], &'a [SessionItem], &'a [SessionItem]);

/// Rebuild the column for `folded` (active, idle, done), given what each group
/// held before and that same group's fold state then.
pub(crate) fn column(
    groups: Groups<'_>,
    folded: [bool; 3],
    was: Groups<'_>,
    was_folded: [bool; 3],
) -> Column {
    let (active, idle, done) = groups;
    let first_fill = was.0.len() + was.1.len() + was.2.len() == 0
        && folded == was_folded;
    let previous = if first_fill {
        Vec::new()
    } else {
        session_entries(was.0, was.1, was.2, was_folded[0], was_folded[1], was_folded[2])
    };
    let entries = session_entries(active, idle, done, folded[0], folded[1], folded[2]);
    let from_y = if first_fill {
        // nothing has been drawn yet: every entry starts where it is
        entries.iter().map(|entry| entry.at).collect()
    } else {
        from_y(&previous, &entries, 16.0)
    };
    Column {
        was_height: entries_height(&previous),
        height: entries_height(&entries),

        previous,
        entries,
        from_y,
    }
}
/// Copy the rows the window holds into the three groups `column` takes. The
/// window's models are the source, so a rebuild never needs the daemon.
pub(crate) fn groups_of(
    active: &slint::ModelRc<SessionItem>,
    idle: &slint::ModelRc<SessionItem>,
    done: &slint::ModelRc<SessionItem>,
) -> (Vec<SessionItem>, Vec<SessionItem>, Vec<SessionItem>) {
    (
        active.iter().collect(),
        idle.iter().collect(),
        done.iter().collect(),
    )
}

/// Write one rebuild to the window: the entries, the y each starts from, the
/// two heights, and the order to animate in. A first fill has nothing to slide
/// from, so it sets the entries and leaves the animation at rest.
pub(crate) fn apply(ui: &AppWindow, column: Column, fold_index: i32, fold_open: bool) {
    if column.previous.is_empty() {
        ui.set_session_entries(slint::ModelRc::from(std::rc::Rc::new(
            slint::VecModel::from(column.entries),
        )));
        ui.set_sessions_height(column.height);
        return;
    }
    let saved_cy = ui.invoke_get_sidebar_cy();
    ui.set_prev_sessions_height(column.was_height);
    ui.set_sessions_height(column.height);
    ui.set_fold_index(fold_index);
    ui.set_fold_open(fold_open);
    ui.set_session_entries(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(column.entries),
    )));
    ui.set_splice_at(slint::ModelRc::from(std::rc::Rc::new(
        slint::VecModel::from(column.from_y),
    )));
    ui.invoke_trigger_list_anim();
    ui.invoke_restore_sidebar_cy(saved_cy);
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
