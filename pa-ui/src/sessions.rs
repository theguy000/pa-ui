//! The sidebar: session roster, group folding, UUID rebinding across daemon restarts,
//! session context menu, search popup, and theme picker.

use crate::*;

#[cfg(test)]
mod tests;

/// The tag a session row carries: what `sidebar.slint` draws, and what decides
/// which group the row sits in.
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
    pub(crate) fn word(self) -> &'static str {
        match self {
            RowTag::Active => "active",
            RowTag::Idle => "idle",
            RowTag::Done => "done",
        }
    }

    pub(crate) fn rank(self) -> u8 {
        match self {
            RowTag::Active => 0,
            RowTag::Idle => 1,
            RowTag::Done => 2,
        }
    }
}

pub(crate) fn tag_of(row: &SessionRow) -> RowTag {
    match row.roster_status() {
        "Running" => RowTag::Active,
        "Idle" => RowTag::Idle,
        _ => RowTag::Done,
    }
}

pub(crate) fn to_item(row: &SessionRow) -> SessionItem {
    let full = display_name(row);
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

pub(crate) fn age_label(row: &SessionRow) -> SharedString {
    let stamp = if row.last_activity.is_empty() {
        &row.modified
    } else {
        &row.last_activity
    };
    let now = SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    age_label_at(stamp, now)
}

fn age_label_at(stamp: &str, now: i64) -> SharedString {
    let Some(seconds) = stamp_seconds(stamp) else {
        return SharedString::new();
    };
    match (now - seconds).max(0) / 86_400 {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        days => format!("{days}d").into(),
    }
}

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
    let separators: [(usize, u8); 5] = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators
        .iter()
        .any(|(at, want)| stamp.as_bytes().get(*at) != Some(want))
    {
        return None;
    }
    let hour = num(11..13)?;
    let minute = num(14..16)?;
    let second = num(17..19)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 60
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub(crate) fn is_top_level(row: &SessionRow) -> bool {
    row.depth == 0 && row.parent.is_empty() && row.parent_active.is_empty()
}

fn anonymous_empty(row: &SessionRow) -> bool {
    let name = row.name.trim();
    row.messages == 0
        && (name.is_empty() || name == "(no messages)")
        && !row.is_resident()
        && is_top_level(row)
}

fn sidebar_row_visible(row: &SessionRow, selected: &str) -> bool {
    !anonymous_empty(row) || row.id == selected
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SessionOrder {
    #[default]
    LastUsed,
    Name,
    Manual,
}

impl SessionOrder {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::LastUsed => "Last used",
            Self::Name => "Name",
            Self::Manual => "Manual",
        }
    }

    pub(crate) fn parse(value: &str) -> Option<Self> {
        match value {
            "Last used" => Some(Self::LastUsed),
            "Name" => Some(Self::Name),
            "Manual" => Some(Self::Manual),
            _ => None,
        }
    }

    pub(crate) fn from_setting(value: &str) -> Self {
        Self::parse(value).unwrap_or_else(|| {
            eprintln!("[settings] unsupported session order {value:?}; using Last used");
            Self::LastUsed
        })
    }
}

#[cfg(test)]
pub(crate) fn session_items(rows: &[SessionRow]) -> Vec<SessionItem> {
    session_items_with_order(rows, SessionOrder::LastUsed, &[])
}

pub(crate) fn ordered_rows<'a>(
    rows: &'a [SessionRow],
    order: SessionOrder,
    manual: &[String],
) -> Vec<&'a SessionRow> {
    let mut manual_pos: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    if order == SessionOrder::Manual {
        for (index, uuid) in manual.iter().enumerate() {
            if !uuid.is_empty() {
                manual_pos.entry(uuid.as_str()).or_insert(index);
            }
        }
    }
    let manual_rank = |row: &SessionRow| -> usize {
        if row.session_id.is_empty() {
            usize::MAX
        } else {
            manual_pos
                .get(row.session_id.as_str())
                .copied()
                .unwrap_or(usize::MAX)
        }
    };
    let mut top: Vec<&SessionRow> = rows.iter().filter(|row| is_top_level(row)).collect();
    top.sort_by(|left, right| {
        let groups = tag_of(left).rank().cmp(&tag_of(right).rank());
        let by_recency = || {
            recency(right)
                .cmp(&recency(left))
                .then_with(|| display_name(left).cmp(&display_name(right)))
                .then_with(|| left.id.cmp(&right.id))
        };
        match order {
            SessionOrder::LastUsed => groups.then_with(by_recency),
            SessionOrder::Name => groups
                .then_with(|| {
                    display_name(left)
                        .to_lowercase()
                        .cmp(&display_name(right).to_lowercase())
                })
                .then_with(|| recency(right).cmp(&recency(left)))
                .then_with(|| left.id.cmp(&right.id)),
            SessionOrder::Manual => groups
                .then_with(|| manual_rank(left).cmp(&manual_rank(right)))
                .then_with(by_recency),
        }
    });
    top
}

pub(crate) fn session_items_with_order(
    rows: &[SessionRow],
    order: SessionOrder,
    manual: &[String],
) -> Vec<SessionItem> {
    ordered_rows(rows, order, manual)
        .into_iter()
        .map(to_item)
        .collect()
}

pub(crate) fn seed_manual_order(
    rows: &[SessionRow],
    order: SessionOrder,
    manual: &mut Vec<String>,
) -> bool {
    if !manual.is_empty() {
        return false;
    }
    let seeded: Vec<String> = ordered_rows(rows, order, &[])
        .into_iter()
        .filter(|row| !row.session_id.is_empty())
        .map(|row| row.session_id.clone())
        .collect();
    if seeded.is_empty() {
        return false;
    }
    *manual = seeded;
    true
}

pub(crate) fn reconcile_manual_order(rows: &[SessionRow], manual: &mut Vec<String>) -> bool {
    let valid: std::collections::HashSet<&str> = rows
        .iter()
        .filter(|row| is_top_level(row) && !row.session_id.is_empty())
        .map(|row| row.session_id.as_str())
        .collect();
    let mut seen = std::collections::HashSet::with_capacity(manual.len());
    let before = manual.len();
    manual.retain(|uuid| valid.contains(uuid.as_str()) && seen.insert(uuid.clone()));
    manual.len() != before
}

pub(crate) fn apply_manual_move(
    rows: &[SessionRow],
    manual: &[String],
    source_id: &str,
    target_id: &str,
) -> Option<Vec<String>> {
    let top: Vec<&SessionRow> = rows.iter().filter(|row| is_top_level(row)).collect();
    let source = top.iter().copied().find(|row| row.id == source_id)?;
    let target = top.iter().copied().find(|row| row.id == target_id)?;
    let group = tag_of(source);
    if group != tag_of(target) {
        return None;
    }
    let mut group_uuids: Vec<String> = Vec::new();
    let mut seen_group = std::collections::HashSet::new();
    for row in ordered_rows(rows, SessionOrder::Manual, manual) {
        if tag_of(row) == group
            && !row.session_id.is_empty()
            && seen_group.insert(row.session_id.clone())
        {
            group_uuids.push(row.session_id.clone());
        }
    }
    let from = group_uuids.iter().position(|uuid| uuid == &source.session_id)?;
    let to = group_uuids.iter().position(|uuid| uuid == &target.session_id)?;
    if from == to {
        return None;
    }
    let moved = group_uuids.remove(from);
    group_uuids.insert(to, moved);

    let mut out: Vec<String> = manual
        .iter()
        .filter(|uuid| !uuid.is_empty() && !seen_group.contains(uuid.as_str()))
        .cloned()
        .collect();
    out.extend(group_uuids);
    Some(out)
}

fn drop_target_at(entries: &[SessionEntry], row_height: f32, target_y: f32) -> Option<String> {
    entries
        .iter()
        .find(|entry| {
            !entry.is_header
                && !entry.folded
                && target_y >= entry.at
                && target_y < entry.at + row_height
        })
        .map(|entry| entry.item.id.to_string())
}

fn recency(row: &SessionRow) -> i64 {
    let stamp = if row.last_activity.is_empty() {
        &row.modified
    } else {
        &row.last_activity
    };
    stamp_seconds(stamp).unwrap_or(0)
}

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

fn tag_word_rank(word: &str) -> u8 {
    match word {
        "active" => RowTag::Active.rank(),
        "idle" => RowTag::Idle.rank(),
        _ => RowTag::Done.rank(),
    }
}

#[cfg(test)]
pub(crate) fn session_entries(
    active: &[SessionItem],
    idle: &[SessionItem],
    done: &[SessionItem],
    active_folded: bool,
    idle_folded: bool,
    done_folded: bool,
) -> Vec<SessionEntry> {
    session_entries_at(
        active,
        idle,
        done,
        active_folded,
        idle_folded,
        done_folded,
        ROW_HEIGHT as f32,
    )
}

pub(crate) fn session_entries_at(
    active: &[SessionItem],
    idle: &[SessionItem],
    done: &[SessionItem],
    active_folded: bool,
    idle_folded: bool,
    done_folded: bool,
    row_height: f32,
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
            let row_target = header_at + HEADER_HEIGHT as f32 + number as f32 * row_height;
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
            at += rows.len() as f32 * row_height;
        }
    }
    entries
}

pub(crate) fn entry_key(entry: &SessionEntry) -> String {
    if entry.is_header {
        return format!("hdr:{}", entry.label);
    }
    let identity = if !entry.item.uuid.is_empty() {
        &entry.item.uuid
    } else if !entry.item.id.is_empty() {
        &entry.item.id
    } else {
        &entry.item.file
    };
    format!("row:{identity}")
}

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

pub(crate) fn moved_between_groups(previous: &SessionEntry, entry: &SessionEntry) -> bool {
    previous.seam != entry.seam
}

pub(crate) fn any_moved(previous: &[SessionEntry], entries: &[SessionEntry]) -> bool {
    let before: std::collections::HashMap<String, &SessionEntry> = previous
        .iter()
        .map(|entry| (entry_key(entry), entry))
        .collect();
    entries.iter().any(|entry| {
        before
            .get(&entry_key(entry))
            .is_some_and(|old| moved_between_groups(old, entry))
    })
}

pub(crate) struct Column {
    pub(crate) previous: Vec<SessionEntry>,
    pub(crate) entries: Vec<SessionEntry>,
    pub(crate) from_y: Vec<f32>,
    pub(crate) was_height: f32,
    pub(crate) height: f32,
}

impl Column {
    pub(crate) fn changed(&self) -> bool {
        self.entries.len() != self.previous.len()
            || self
                .entries
                .iter()
                .zip(&self.from_y)
                .any(|(entry, from)| (entry.at - from).abs() > 0.5)
            || self.entries.iter().zip(&self.previous).any(|(a, b)| {
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

pub(crate) type Groups<'a> = (&'a [SessionItem], &'a [SessionItem], &'a [SessionItem]);

#[cfg(test)]
pub(crate) fn groups_from(
    rows: &[SessionRow],
) -> (Vec<SessionItem>, Vec<SessionItem>, Vec<SessionItem>) {
    session_groups(&session_items(rows))
}

pub(crate) fn column(
    groups: Groups<'_>,
    folded: [bool; 3],
    was: Groups<'_>,
    was_folded: [bool; 3],
) -> Column {
    column_at(groups, folded, was, was_folded, ROW_HEIGHT as f32)
}

pub(crate) fn column_at(
    groups: Groups<'_>,
    folded: [bool; 3],
    was: Groups<'_>,
    was_folded: [bool; 3],
    row_height: f32,
) -> Column {
    let (active, idle, done) = groups;
    let first_fill = was.0.len() + was.1.len() + was.2.len() == 0 && folded == was_folded;
    let previous = if first_fill {
        Vec::new()
    } else {
        session_entries_at(
            was.0,
            was.1,
            was.2,
            was_folded[0],
            was_folded[1],
            was_folded[2],
            row_height,
        )
    };
    let entries = session_entries_at(
        active,
        idle,
        done,
        folded[0],
        folded[1],
        folded[2],
        row_height,
    );
    let from_y = if first_fill {
        entries.iter().map(|entry| entry.at).collect()
    } else {
        from_y(&previous, &entries, 16.0)
    };
    Column {
        was_height: entries_height_at(&previous, row_height),
        height: entries_height_at(&entries, row_height),
        previous,
        entries,
        from_y,
    }
}

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

const INITIAL_VIEWPORT_HEIGHT: f32 = 732.0;
const OVERSCAN_ROWS: usize = 6;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarModelOp {
    Reset,
    Set { at: usize },
    Insert { at: usize },
    Remove { at: usize },
}

fn lcs_matches(old: &[SessionEntry], new: &[SessionEntry]) -> Vec<(usize, usize)> {
    let old_keys: Vec<String> = old.iter().map(entry_key).collect();
    let new_keys: Vec<String> = new.iter().map(entry_key).collect();
    let m = old_keys.len();
    let n = new_keys.len();
    let mut dp = vec![0u16; (m + 1) * (n + 1)];
    let stride = n + 1;
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i * stride + j] = if old_keys[i] == new_keys[j] {
                1 + dp[(i + 1) * stride + (j + 1)]
            } else {
                dp[(i + 1) * stride + j].max(dp[i * stride + (j + 1)])
            };
        }
    }
    let mut out = Vec::with_capacity(dp[0] as usize);
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        if old_keys[i] == new_keys[j] {
            out.push((i, j));
            i += 1;
            j += 1;
        } else if dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

#[cfg(test)]
pub(crate) fn entries_height(entries: &[SessionEntry]) -> f32 {
    entries_height_at(entries, ROW_HEIGHT as f32)
}

pub(crate) fn entries_height_at(entries: &[SessionEntry], row_height: f32) -> f32 {
    entries
        .iter()
        .filter(|entry| entry.is_header || !entry.folded)
        .map(|entry| {
            entry.at
                + if entry.is_header {
                    HEADER_HEIGHT as f32
                } else {
                    row_height
                }
        })
        .fold(COLUMN_TOP as f32, f32::max)
}

fn row_height_for(ui: &AppWindow) -> f32 {
    ui.global::<crate::Appearance>().get_session_row_height()
}

pub(crate) const HEADER_HEIGHT: i32 = 32;
pub(crate) const ROW_HEIGHT: i32 = 32;
pub(crate) const COLUMN_TOP: i32 = 88;

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

/// Put the last copied id on the system clipboard.
pub(crate) fn copy_to_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|error| format!("no clipboard: {error}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|error| error.to_string())
}

/// Pure UUID rebinding resolution: when `selected` is missing from `rows`, looks up
/// a row sharing `selected_uuid` and returns its new `id`.
pub(crate) fn resolve_rebound_id(
    rows: &[SessionRow],
    selected: &str,
    selected_uuid: Option<&str>,
) -> Option<String> {
    if selected.is_empty() || rows.iter().any(|row| row.id == selected) {
        return None;
    }
    let uuid = selected_uuid?;
    rows.iter()
        .find(|row| row.session_id == uuid)
        .map(|row| row.id.clone())
}

/// Pure family cost calculation across spawned child sessions in the roster.
pub(crate) fn compute_family_cost(rows: &[SessionRow], session: &str, own_cost: f64) -> (f64, bool) {
    let mut children_cost = 0.0f64;
    let mut has_children = false;
    let own = rows.iter().find(|row| row.id == session);
    let own_sid = own
        .map(|row| row.session_id.as_str())
        .filter(|sid| !sid.is_empty());
    let own_active = own.map(|row| row.id.as_str()).filter(|id| !id.is_empty());
    for row in rows.iter().filter(|row| {
        own_sid.is_some_and(|sid| row.parent == sid)
            || own_active.is_some_and(|id| row.parent_active == id)
    }) {
        has_children = true;
        children_cost += row.cost;
    }
    (own_cost + children_cost, has_children)
}

/// The items a session row's menu offers, in the order they are drawn.
pub(crate) fn menu_items(done: bool, confirming: bool) -> Vec<SessionMenuItem> {
    if done {
        let delete = if confirming {
            menu_item(
                "delete",
                "Delete for good",
                "this removes the session file",
                true,
            )
        } else {
            menu_item("delete", "Delete", "", true)
        };
        vec![
            menu_item("rename", "Rename", "", false),
            menu_item("copy-id", "Copy id", "", false),
            delete,
        ]
    } else {
        vec![
            menu_item("done", "Mark done", "", false),
            menu_item("rename", "Rename", "", false),
            menu_item("share", "Share as gist", "", false),
            menu_item("copy-id", "Copy id", "", false),
        ]
    }
}

fn menu_item(key: &str, label: &str, note: &str, separated: bool) -> SessionMenuItem {
    SessionMenuItem {
        key: key.into(),
        label: label.into(),
        danger: separated,
        note: note.into(),
        separated,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MenuAction {
    MarkDone,
    Rename,
    Share,
    CopyId,
    Delete,
    Nothing,
}

pub(crate) fn menu_action(key: &str, done: bool) -> MenuAction {
    match key {
        "done" if !done => MenuAction::MarkDone,
        "rename" => MenuAction::Rename,
        "share" if !done => MenuAction::Share,
        "copy-id" => MenuAction::CopyId,
        "delete" if done => MenuAction::Delete,
        _ => MenuAction::Nothing,
    }
}

fn current_item(ui: &AppWindow, id: &str) -> Option<SessionItem> {
    ui.get_sessions().iter().find(|item| item.id == id)
}

const MOCK_THEMES: [(&str, &str); 5] = [
    ("default", "Default"),
    ("mono", "Mono"),
    ("obsidian", "Obsidian"),
    ("ember", "Ember"),
    ("paper", "Paper"),
];

fn populate_themes(ui: &AppWindow, active: &str) {
    let rows: Vec<ThemeItem> = MOCK_THEMES
        .iter()
        .map(|(id, label)| ThemeItem {
            id: (*id).into(),
            label: (*label).into(),
            current: *id == active,
        })
        .collect();
    ui.set_theme_items(ModelRc::from(Rc::new(VecModel::from(rows))));
}

/// The rows the search popup draws for one query.
pub(crate) fn match_rows(sessions: &[SessionRow], query: &str) -> ModelRc<SearchRow> {
    let top_level: Vec<SessionRow> = sessions
        .iter()
        .filter(|row| is_top_level(row))
        .cloned()
        .collect();
    let rows: Vec<SearchRow> = search::matches(&top_level, query)
        .into_iter()
        .map(|(row, offsets)| {
            let name = search::elide(&display_name(row));
            SearchRow {
                id: row.id.as_str().into(),
                age: age_label(row),
                glyphs: ModelRc::from(Rc::new(VecModel::from(search::glyphs(&name, &offsets)))),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

pub(crate) fn match_rows_with_order(
    sessions: &[SessionRow],
    query: &str,
    order: SessionOrder,
    manual: &[String],
) -> ModelRc<SearchRow> {
    let ordered: Vec<SessionRow> = ordered_rows(sessions, order, manual)
        .into_iter()
        .cloned()
        .collect();
    match_rows(&ordered, query)
}

fn open_search_with_order(
    ui: &AppWindow,
    sessions: &[SessionRow],
    order: SessionOrder,
    manual: &[String],
) {
    // The search popup and the share card are the two surfaces that open over
    // the whole window; only one of them is ever up.
    ui.set_share_open(false);
    ui.set_settings_menu(false);
    let query = ui.get_search_query();
    ui.set_search_rows(match_rows_with_order(
        sessions,
        query.as_str(),
        order,
        manual,
    ));
    ui.set_search_index(0);
    ui.set_search_field_typed(false);
    ui.set_search_open(true);
    ui.set_search_focus_tick(ui.get_search_focus_tick() + 1);
}

struct SidebarViewport {
    groups: (Vec<SessionItem>, Vec<SessionItem>, Vec<SessionItem>),
    all_entries: Vec<SessionEntry>,
    all_from_y: Vec<f32>,
    mounted_entries: Vec<SessionEntry>,
    mounted_from_y: Vec<f32>,
    row_height: f32,
    scroll_y: f32,
    viewport_h: f32,
    animating_fold: Option<usize>,
    animating_splice: bool,
    fold_generation: u64,
    fold_timer: Option<slint::Timer>,
    entries_model: Rc<VecModel<SessionEntry>>,
    splice_model: Rc<VecModel<f32>>,
    #[cfg(test)]
    last_ops: Vec<SidebarModelOp>,
}

impl SidebarViewport {
    fn new() -> Self {
        Self {
            groups: (Vec::new(), Vec::new(), Vec::new()),
            all_entries: Vec::new(),
            all_from_y: Vec::new(),
            mounted_entries: Vec::new(),
            mounted_from_y: Vec::new(),
            row_height: ROW_HEIGHT as f32,
            scroll_y: 0.0,
            viewport_h: INITIAL_VIEWPORT_HEIGHT,
            animating_fold: None,
            animating_splice: false,
            fold_generation: 0,
            fold_timer: None,
            entries_model: Rc::new(VecModel::default()),
            splice_model: Rc::new(VecModel::default()),
            #[cfg(test)]
            last_ops: Vec::new(),
        }
    }

    fn visible_window(&self, include_splice: bool) -> (Vec<SessionEntry>, Vec<f32>) {
        let rh = self.row_height.max(1.0);
        let effective_vh = self.viewport_h;
        let scroll_top = (-self.scroll_y).max(0.0);
        let overscan = OVERSCAN_ROWS as f32 * rh;
        let win_top = (COLUMN_TOP as f32 + scroll_top - overscan).max(COLUMN_TOP as f32);
        let win_bottom = COLUMN_TOP as f32 + scroll_top + effective_vh + overscan;

        let mut entries = Vec::new();
        let mut from_y = Vec::new();
        for (i, entry) in self.all_entries.iter().enumerate() {
            let animating_group =
                !entry.is_header && self.animating_fold == Some(entry.seam as usize);
            if !entry.is_header && entry.folded && !animating_group {
                continue;
            }
            let h = if entry.is_header {
                HEADER_HEIGHT as f32
            } else {
                rh
            };
            let fy = if include_splice && i < self.all_from_y.len() {
                self.all_from_y[i]
            } else {
                entry.at
            };
            let (y_min, y_max) = if animating_group {
                (entry.target - 18.0, entry.target + h)
            } else if include_splice {
                (entry.at.min(fy), entry.at.max(fy) + h)
            } else {
                (entry.at, entry.at + h)
            };
            if y_max >= win_top && y_min <= win_bottom {
                entries.push(entry.clone());
                if include_splice {
                    from_y.push(fy);
                }
            }
        }
        (entries, from_y)
    }
}

enum EntriesPatch {
    None,
    Reset(Vec<SessionEntry>),
    Diff {
        removes: Vec<usize>,
        steps: Vec<PatchStep>,
    },
}

enum PatchStep {
    Set { at: usize, entry: SessionEntry },
    Keep,
    Insert { at: usize, entry: SessionEntry },
}

fn plan_entries_patch(old: &[SessionEntry], new: &[SessionEntry]) -> EntriesPatch {
    if old == new {
        return EntriesPatch::None;
    }
    if old.is_empty() || new.is_empty() {
        return EntriesPatch::Reset(new.to_vec());
    }
    let matches = lcs_matches(old, new);
    if matches.is_empty() || matches.len() * 4 < new.len() {
        return EntriesPatch::Reset(new.to_vec());
    }
    let mut keep_old = vec![false; old.len()];
    for &(oi, _) in &matches {
        keep_old[oi] = true;
    }
    let removes: Vec<usize> = (0..old.len()).rev().filter(|&i| !keep_old[i]).collect();
    let mut steps = Vec::with_capacity(new.len());
    let mut mi = 0usize;
    for (ni, new_entry) in new.iter().enumerate() {
        if mi < matches.len() && matches[mi].1 == ni {
            let oi = matches[mi].0;
            if old[oi] != *new_entry {
                steps.push(PatchStep::Set {
                    at: ni,
                    entry: new_entry.clone(),
                });
            } else {
                steps.push(PatchStep::Keep);
            }
            mi += 1;
        } else {
            steps.push(PatchStep::Insert {
                at: ni,
                entry: new_entry.clone(),
            });
        }
    }
    EntriesPatch::Diff { removes, steps }
}

/// Stateful Sidebar module owning the session roster, fold state, UUID rebinding,
/// context menu confirmation, and search popup.
pub(crate) struct Sidebar {
    pub cache: Rc<RefCell<Vec<SessionRow>>>,
    pub selected_uuid: RefCell<Option<String>>,
    viewport: Rc<RefCell<SidebarViewport>>,
    settings: Rc<crate::settings::SettingsStore>,
    catalog_ready: std::cell::Cell<bool>,
}

impl Sidebar {
    pub fn new(ui: &AppWindow, job_tx: &Sender<Job>) -> Rc<Self> {
        let vp = SidebarViewport::new();
        ui.set_session_entries(ModelRc::from(vp.entries_model.clone()));
        ui.set_splice_at(ModelRc::from(vp.splice_model.clone()));
        let sidebar = Rc::new(Self {
            cache: Rc::new(RefCell::new(Vec::new())),
            selected_uuid: RefCell::new(None),
            viewport: Rc::new(RefCell::new(vp)),
            settings: crate::settings::SettingsStore::new(crate::settings::default_settings_path()),
            catalog_ready: std::cell::Cell::new(false),
        });
        sidebar.bind_ui(ui, job_tx);
        sidebar
    }

    /// The durable session id of a row, for session-scoped reads (heartbeats).
    pub(crate) fn session_id_for(&self, selected: &str) -> String {
        uuid_of(&self.cache.borrow(), selected)
            .unwrap_or_default()
            .to_string()
    }

    pub fn track_selection(&self, selected: &str) {
        if let Some(uuid) = uuid_of(&self.cache.borrow(), selected).map(str::to_string) {
            *self.selected_uuid.borrow_mut() = Some(uuid.clone());
            if !uuid.is_empty() {
                let changed = {
                    let mut state = self.settings.state.borrow_mut();
                    if state.last_session == uuid {
                        false
                    } else {
                        state.last_session = uuid;
                        true
                    }
                };
                if changed {
                    if let Err(error) = self.settings.persist() {
                        eprintln!("[settings] {error}");
                    }
                }
            }
        }
    }

    pub fn forget_selection(&self) {
        *self.selected_uuid.borrow_mut() = None;
    }

    pub fn has_running_top_level(&self, settled: Option<&str>) -> bool {
        self.cache.borrow().iter().any(|row| {
            settled != Some(row.id.as_str())
                && is_top_level(row)
                && tag_of(row) == RowTag::Active
        })
    }

    pub fn family_cost(&self, session: &str, own_cost: f64) -> (f64, bool) {
        if let Ok(rows) = self.cache.try_borrow() {
            compute_family_cost(&rows, session, own_cost)
        } else {
            (own_cost, false)
        }
    }

    pub fn set_catalog_ready(&self, ready: bool) {
        self.catalog_ready.set(ready);
    }

    fn order(&self) -> SessionOrder {
        self.settings.state.borrow().session_order
    }

    pub fn open_search(&self, ui: &AppWindow) {
        let manual = self.settings.state.borrow().manual_session_order.clone();
        open_search_with_order(ui, &self.cache.borrow(), self.order(), &manual);
    }

    fn sync_mounted_window(&self) {
        let (patch, new_from_y, splice_changed, entries_model, splice_model) = {
            let mut w = self.viewport.borrow_mut();
            let include_splice = w.animating_splice;
            let (new_entries, new_from_y) = w.visible_window(include_splice);
            let patch = plan_entries_patch(&w.mounted_entries, &new_entries);
            let splice_changed = w.mounted_from_y != new_from_y;
            if matches!(patch, EntriesPatch::None) && !splice_changed {
                return;
            }
            w.mounted_entries = new_entries;
            w.mounted_from_y = new_from_y.clone();
            (
                patch,
                new_from_y,
                splice_changed,
                w.entries_model.clone(),
                w.splice_model.clone(),
            )
        };

        if new_from_y.is_empty() {
            if splice_model.row_count() > 0 {
                splice_model.set_vec(Vec::new());
            }
        } else if splice_changed {
            splice_model.set_vec(new_from_y);
        }

        #[cfg(test)]
        let mut recorded_ops = Vec::new();
        match patch {
            EntriesPatch::None => {}
            EntriesPatch::Reset(entries) => {
                #[cfg(test)]
                recorded_ops.push(SidebarModelOp::Reset);
                entries_model.set_vec(entries);
            }
            EntriesPatch::Diff { removes, steps } => {
                for at in removes {
                    #[cfg(test)]
                    recorded_ops.push(SidebarModelOp::Remove { at });
                    entries_model.remove(at);
                }
                for step in steps {
                    match step {
                        PatchStep::Set { at, entry } => {
                            #[cfg(test)]
                            recorded_ops.push(SidebarModelOp::Set { at });
                            entries_model.set_row_data(at, entry);
                        }
                        PatchStep::Insert { at, entry } => {
                            #[cfg(test)]
                            recorded_ops.push(SidebarModelOp::Insert { at });
                            entries_model.insert(at, entry);
                        }
                        PatchStep::Keep => {}
                    }
                }
            }
        }
        #[cfg(test)]
        if !recorded_ops.is_empty() {
            self.viewport.borrow_mut().last_ops = recorded_ops;
        }
    }

    fn settle_fold(&self, expected_gen: Option<u64>) {
        {
            let mut w = self.viewport.borrow_mut();
            if expected_gen.is_some_and(|g| g != w.fold_generation) {
                return;
            }
            if !w.animating_splice && w.animating_fold.is_none() && w.splice_model.row_count() == 0 {
                return;
            }
            w.animating_fold = None;
            w.animating_splice = false;
            w.fold_timer = None;
            w.all_from_y.clear();
        }
        self.sync_mounted_window();
    }

    fn apply_column(
        self: &Rc<Self>,
        ui: &AppWindow,
        column: Column,
        fold_index: i32,
        fold_open: bool,
        is_fold_toggle: bool,
    ) {
        let previous_empty = column.previous.is_empty();
        let fold_duration = ui.global::<crate::Appearance>().get_fold_duration();
        let row_h = row_height_for(ui);
        let vh = ui.get_dbg_sidebar_h();
        let saved_cy = ui.invoke_get_sidebar_cy();
        let is_group_move = fold_index == -2;
        let start_anim = !previous_empty && fold_duration > 0 && (is_fold_toggle || is_group_move);

        {
            let mut w = self.viewport.borrow_mut();
            w.all_entries = column.entries;
            w.all_from_y = column.from_y;
            w.row_height = row_h;
            if vh > 1.0 {
                w.viewport_h = vh;
            }
            if is_fold_toggle && fold_duration <= 0 {
                w.animating_fold = None;
                w.animating_splice = false;
                w.fold_timer = None;
                w.fold_generation = w.fold_generation.wrapping_add(1);
            } else if start_anim {
                w.fold_generation = w.fold_generation.wrapping_add(1);
                if is_fold_toggle {
                    w.animating_fold = Some(fold_index as usize);
                }
                w.animating_splice = true;
                let timer = slint::Timer::default();
                let sb_w = Rc::downgrade(self);
                let gen = w.fold_generation;
                timer.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(fold_duration as u64 + 32),
                    move || {
                        if let Some(sb) = sb_w.upgrade() {
                            sb.settle_fold(Some(gen));
                        }
                    },
                );
                w.fold_timer = Some(timer);
            } else if !is_fold_toggle {
                w.animating_fold = None;
                w.animating_splice = false;
                w.fold_timer = None;
                w.fold_generation = w.fold_generation.wrapping_add(1);
            }
        }

        if previous_empty {
            ui.set_sessions_height(column.height);
            self.sync_mounted_window();
            return;
        }
        ui.set_prev_sessions_height(column.was_height);
        ui.set_sessions_height(column.height);
        ui.set_fold_index(fold_index);
        ui.set_fold_open(fold_open);
        self.sync_mounted_window();
        if start_anim {
            ui.invoke_trigger_list_anim();
        }
        ui.invoke_restore_sidebar_cy(saved_cy);
    }

    fn apply_items(
        self: &Rc<Self>,
        ui: &AppWindow,
        items: Vec<SessionItem>,
        folded: [bool; 3],
        selected: i32,
    ) -> Option<SessionItem> {
        let (active, idle, done) = session_groups(&items);
        let prev = self.viewport.borrow().groups.clone();
        let active_changed = active != prev.0;
        let idle_changed = idle != prev.1;
        let done_changed = done != prev.2;
        let col = column_at(
            (&active, &idle, &done),
            folded,
            (&prev.0, &prev.1, &prev.2),
            folded,
            row_height_for(ui),
        );
        let selected = if selected < 0 && any_moved(&col.previous, &col.entries) {
            -2
        } else {
            selected
        };
        if col.changed() {
            self.apply_column(ui, col, selected, false, false);
        }
        self.viewport.borrow_mut().groups = (active.clone(), idle.clone(), done.clone());
        if active_changed {
            ui.set_active_items(ModelRc::from(Rc::new(VecModel::from(active))));
        }
        if idle_changed {
            ui.set_idle_items(ModelRc::from(Rc::new(VecModel::from(idle))));
        }
        if done_changed {
            ui.set_done_items(ModelRc::from(Rc::new(VecModel::from(done))));
        }
        let first = items.first().cloned();
        if active_changed || idle_changed || done_changed {
            ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(items))));
        }
        first
    }

    fn project_roster(self: &Rc<Self>, ui: &AppWindow, rows: &[SessionRow]) -> Option<SessionItem> {
        let order = self.order();
        let manual = self.settings.state.borrow().manual_session_order.clone();
        let items = session_items_with_order(rows, order, &manual);
        let folded = [
            ui.get_active_folded(),
            ui.get_idle_folded(),
            ui.get_done_folded(),
        ];
        self.apply_items(ui, items, folded, -1)
    }

    pub fn seed(self: &Rc<Self>, ui: &AppWindow, rows: Vec<SessionRow>) {
        let manual = self.settings.state.borrow().manual_session_order.clone();
        let items = session_items_with_order(&rows, self.order(), &manual);
        self.apply_items(ui, items, [false, false, false], 0);
        *self.cache.borrow_mut() = rows;
    }

    /// Returns `Some(rebound_id)` if the active selection was re-keyed by the daemon.
    pub fn apply_roster(self: &Rc<Self>, ui: &AppWindow, rows: Vec<SessionRow>) -> Option<String> {
        let selected = ui.get_selected_id().to_string();
        let selected_uuid = self.selected_uuid.borrow().clone();
        let rows: Vec<SessionRow> = rows
            .into_iter()
            .filter(|row| {
                sidebar_row_visible(row, &selected)
                    || selected_uuid
                        .as_deref()
                        .is_some_and(|uuid| row.session_id == uuid)
            })
            .collect();
        *self.cache.borrow_mut() = rows.clone();
        if self.catalog_ready.get() {
            let changed = {
                let mut state = self.settings.state.borrow_mut();
                let pruned = reconcile_manual_order(&rows, &mut state.manual_session_order);
                let seeded = state.session_order == SessionOrder::Manual
                    && seed_manual_order(&rows, SessionOrder::LastUsed, &mut state.manual_session_order);
                pruned || seeded
            };
            if changed {
                if let Err(error) = self.settings.persist() {
                    eprintln!("[settings] {error}");
                }
            }
        }
        let visible_first = self.project_roster(ui, &rows);

        if ui.get_selected_id().is_empty() && !ui.get_draft_chat() {
            let preferred = std::env::var("PA_SELECT").ok();
            let reopen_last = {
                let state = self.settings.state.borrow();
                if state.reopen_last_session && !state.last_session.is_empty() {
                    Some(state.last_session.clone())
                } else {
                    None
                }
            };
            let chosen = preferred
                .as_deref()
                .and_then(|wanted| rows.iter().find(|row| row.id == wanted))
                .or_else(|| {
                    reopen_last
                        .as_deref()
                        .and_then(|uuid| rows.iter().find(|row| row.session_id == uuid))
                })
                .or_else(|| {
                    visible_first
                        .as_ref()
                        .and_then(|item| rows.iter().find(|row| row.id == item.id.as_str()))
                });
            if let Some(first) = chosen {
                ui.set_selected_id(first.id.clone().into());
                *self.selected_uuid.borrow_mut() = Some(first.session_id.clone());
            }
        }

        let selected = ui.get_selected_id().to_string();
        let mut rebound_out = None;
        let current_uuid = self.selected_uuid.borrow().clone();
        if let Some(rebound) = resolve_rebound_id(&rows, &selected, current_uuid.as_deref()) {
            if crate::env_cfg::cfg().debug {
                eprintln!("[rebind] {selected} -> {rebound}");
            }
            ui.set_selected_id(rebound.clone().into());
            rebound_out = Some(rebound);
        }

        let selected = ui.get_selected_id().to_string();
        if let Some(uuid) = uuid_of(&rows, &selected) {
            *self.selected_uuid.borrow_mut() = Some(uuid.to_string());
        }
        rebound_out
    }

    fn bind_ui(self: &Rc<Self>, ui: &AppWindow, job_tx: &Sender<Job>) {
        {
            let jobs = job_tx.clone();
            let window = ui.as_weak();
            ui.on_rename_requested(move |session, name| {
                let session = session.to_string();
                if session.is_empty() || name.trim().is_empty() {
                    return;
                }
                let saved = window
                    .upgrade()
                    .and_then(|ui| current_item(&ui, &session))
                    .filter(|item| item.done)
                    .map(|item| item.file.to_string());
                if let Some(session_path) = saved {
                    let _ = jobs.send(Job::RenameSaved {
                        session_path,
                        name: name.trim().to_string(),
                    });
                    return;
                }
                let _ = jobs.send(Job::Rename {
                    session,
                    name: name.trim().to_string(),
                });
            });
        }

        {
            let jobs = job_tx.clone();
            let window = ui.as_weak();
            let confirming: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

            let open_window = ui.as_weak();
            let open_confirming = confirming.clone();
            ui.on_session_menu_opened(move |id| {
                let id = id.to_string();
                let Some(ui) = open_window.upgrade() else {
                    return;
                };
                let Some(item) = current_item(&ui, &id) else {
                    return;
                };
                let mut confirming = open_confirming.borrow_mut();
                if confirming.as_deref() != Some(id.as_str()) {
                    *confirming = None;
                }
                let items = menu_items(item.done, confirming.is_some());
                ui.set_menu_items(ModelRc::from(Rc::new(VecModel::from(items))));
            });

            let close_confirming = confirming.clone();
            ui.on_menu_closed(move || {
                *close_confirming.borrow_mut() = None;
            });

            let picked_confirming = confirming;
            ui.on_session_menu_picked(move |id, key| {
                let id = id.to_string();
                let key = key.to_string();
                let Some(ui) = window.upgrade() else { return };
                let mut confirming = picked_confirming.borrow_mut();
                let done = match current_item(&ui, &id) {
                    Some(item) => item.done,
                    None if confirming.as_deref() == Some(id.as_str()) => true,
                    None => return,
                };
                match menu_action(&key, done) {
                    MenuAction::Delete
                        if ui.get_setting_confirm_delete_session()
                            && confirming.as_deref() != Some(id.as_str()) =>
                    {
                        *confirming = Some(id);
                        let items = menu_items(true, true);
                        ui.set_menu_items(ModelRc::from(Rc::new(VecModel::from(items))));
                        return;
                    }
                    MenuAction::Delete => {
                        *confirming = None;
                        let path = current_item(&ui, &id)
                            .map(|item| item.file.to_string())
                            .unwrap_or_default();
                        if !path.is_empty() {
                            let _ = jobs.send(Job::DeleteSaved { session_path: path });
                        }
                        ui.set_menu_id("".into());
                        return;
                    }
                    MenuAction::MarkDone => {
                        *confirming = None;
                        let _ = jobs.send(Job::MarkDone { session: id });
                    }
                    MenuAction::Rename => {
                        *confirming = None;
                        let Some(item) = current_item(&ui, &id) else {
                            return;
                        };
                        ui.set_menu_id("".into());
                        ui.set_editing_id(id.into());
                        ui.set_draft(item.full);
                        return;
                    }
                    MenuAction::Share => {
                        *confirming = None;
                        // The card opens before the work starts: the GitHub CLI
                        // can take a moment, and a menu that closes onto nothing
                        // reads as a click that did not land.
                        ui.set_search_field_typed(false);
                        ui.set_search_open(false);
                        ui.set_settings_menu(false);
                        ui.set_share_pending(true);
                        ui.set_share_url("".into());
                        ui.set_share_error("".into());
                        ui.set_share_action_error("".into());
                        ui.set_share_copied(false);
                        ui.set_share_open(true);
                        let _ = jobs.send(Job::Share { session: id });
                    }
                    MenuAction::CopyId => {
                        *confirming = None;
                        let Some(item) = current_item(&ui, &id) else {
                            return;
                        };
                        let _ = copy_to_clipboard(&item.uuid);
                    }
                    MenuAction::Nothing => {
                        *confirming = None;
                    }
                }
                ui.set_menu_id("".into());
            });
        }

        {
            let window = ui.as_weak();
            let sidebar = self.clone();
            let before: Rc<RefCell<[bool; 3]>> = Rc::new(RefCell::new([
                ui.get_active_folded(),
                ui.get_idle_folded(),
                ui.get_done_folded(),
            ]));
            ui.on_fold_changed(move || {
                let Some(ui) = window.upgrade() else { return };
                let now = [
                    ui.get_active_folded(),
                    ui.get_idle_folded(),
                    ui.get_done_folded(),
                ];
                let was = before.replace(now);
                let Some(index) = (0..3).find(|index| now[*index] != was[*index]) else {
                    return;
                };
                let (active, idle, done) = {
                    let w = sidebar.viewport.borrow();
                    if w.groups.0.is_empty() && w.groups.1.is_empty() && w.groups.2.is_empty() {
                        groups_of(
                            &ui.get_active_items(),
                            &ui.get_idle_items(),
                            &ui.get_done_items(),
                        )
                    } else {
                        w.groups.clone()
                    }
                };
                let col = column_at(
                    (&active, &idle, &done),
                    now,
                    (&active, &idle, &done),
                    was,
                    row_height_for(&ui),
                );
                sidebar.apply_column(&ui, col, index as i32, !now[index], true);
            });
        }

        {
            let sidebar = self.clone();
            ui.on_fold_settled(move || {
                sidebar.settle_fold(None);
            });
        }

        {
            let sidebar = self.clone();
            ui.on_sidebar_viewport_changed(move |cy, vh| {
                let changed = {
                    let Ok(mut w) = sidebar.viewport.try_borrow_mut() else {
                        return;
                    };
                    let mut moved = (w.scroll_y - cy).abs() > 0.5;
                    w.scroll_y = cy;
                    if vh > 1.0 && (w.viewport_h - vh).abs() > 0.5 {
                        w.viewport_h = vh;
                        moved = true;
                    }
                    moved
                };
                if changed {
                    sidebar.sync_mounted_window();
                }
            });
        }

        {
            let store = self.settings.clone();
            {
                let initial = store.state.borrow();
                initial.apply_to_ui(ui);
                populate_themes(ui, &initial.theme);
                if crate::env_cfg::cfg().theme.is_none() && initial.theme != "default" {
                    let (resolved, refusal) = crate::theme::select(Some(initial.theme.as_str()));
                    if refusal.is_none() {
                        crate::theme::apply(ui, &resolved);
                    }
                }
            }

            {
                let weak = ui.as_weak();
                let store = store.clone();
                ui.on_theme_picked(move |id| {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_settings_open_choice("".into());
                    let id_str = id.as_str();
                    populate_themes(&ui, id_str);
                    {
                        let mut state = store.state.borrow_mut();
                        state.theme = id_str.to_string();
                    }
                    if let Err(error) = store.persist() {
                        eprintln!("[settings] {error}");
                    }
                    let selection = if id_str == "default" {
                        None
                    } else {
                        Some(id_str)
                    };
                    let (resolved, refusal) = crate::theme::select(selection);
                    if refusal.is_none() {
                        crate::theme::apply(&ui, &resolved);
                    }
                });
            }
            {
                let weak = ui.as_weak();
                let store = store.clone();
                ui.on_setting_toggled(move |key| {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_settings_open_choice("".into());
                    let changed = {
                        let mut state = store.state.borrow_mut();
                        let ok = state.toggle(key.as_str());
                        if ok {
                            state.apply_to_ui(&ui);
                        }
                        ok
                    };
                    if changed {
                        if let Err(error) = store.persist() {
                            eprintln!("[settings] {error}");
                        }
                    }
                });
            }
            {
                let weak = ui.as_weak();
                let store = store.clone();
                let sidebar = self.clone();
                ui.on_setting_chosen(move |key, val| {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_settings_open_choice("".into());
                    let changed = {
                        let mut state = store.state.borrow_mut();
                        let prev_order = state.session_order;
                        let ok = state.choose(key.as_str(), val.as_str());
                        if ok && key == "session-order" && state.session_order == SessionOrder::Manual {
                            let rows = sidebar.cache.borrow();
                            seed_manual_order(&rows, prev_order, &mut state.manual_session_order);
                        }
                        if ok {
                            state.apply_to_ui(&ui);
                        }
                        ok
                    };
                    if changed {
                        if key.as_str() == "interface-size" {
                            let (active, idle, done) = groups_of(
                                &ui.get_active_items(),
                                &ui.get_idle_items(),
                                &ui.get_done_items(),
                            );
                            let folded = [
                                ui.get_active_folded(),
                                ui.get_idle_folded(),
                                ui.get_done_folded(),
                            ];
                            let col = column_at(
                                (&active, &idle, &done),
                                folded,
                                (&[], &[], &[]),
                                folded,
                                row_height_for(&ui),
                            );
                            sidebar.apply_column(&ui, col, -1, false, false);
                        }
                        if let Err(error) = store.persist() {
                            eprintln!("[settings] {error}");
                        }
                        if key == "session-order" {
                            let rows = sidebar.cache.borrow();
                            sidebar.project_roster(&ui, &rows);
                        }
                    }
                });
            }
            {
                let weak = ui.as_weak();
                ui.on_settings_toggle_requested(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    if ui.get_settings_menu() {
                        crate::settings::close_settings(&ui);
                    } else {
                        crate::settings::open_settings(&ui);
                    }
                });
            }
            {
                let weak = ui.as_weak();
                ui.on_settings_search_edited(move |query| {
                    let Some(ui) = weak.upgrade() else { return };
                    ui.set_settings_open_choice("".into());
                    crate::settings::SectionFilter::for_query(query.as_str()).apply_to_ui(&ui);
                });
            }
            {
                let weak = ui.as_weak();
                ui.on_settings_closed(move || {
                    let Some(ui) = weak.upgrade() else { return };
                    crate::settings::close_settings(&ui);
                });
            }
        }

        {
            let sidebar = self.clone();
            let weak = ui.as_weak();
            ui.on_search_open_requested(move || {
                let Some(ui) = weak.upgrade() else { return };
                sidebar.open_search(&ui);
            });
        }
        {
            let weak = ui.as_weak();
            let sidebar = self.clone();
            ui.on_search_typed(move |query| {
                let Some(ui) = weak.upgrade() else { return };
                let manual = sidebar.settings.state.borrow().manual_session_order.clone();
                let found = match_rows_with_order(
                    &sidebar.cache.borrow(),
                    query.as_str(),
                    sidebar.order(),
                    &manual,
                );
                ui.set_search_index(0);
                ui.set_search_rows(found);
            });
        }
        fn select_from_overlay(ui: &AppWindow, id: SharedString) {
            ui.set_model_menu(false);
            ui.set_queue_open(false);
            ui.set_settings_menu(false);
            ui.set_share_open(false);
            let changed = ui.get_selected_id() != id;
            ui.set_selected_id(id.clone());
            if changed {
                ui.invoke_selection_picked(id);
            }
            ui.set_hover_title(SharedString::new());
            ui.set_search_field_typed(false);
            ui.set_search_open(false);
            ui.invoke_focus_composer();
        }
        {
            let weak = ui.as_weak();
            let sidebar = self.clone();
            ui.on_session_dropped(move |source, target_y| {
                let Some(ui) = weak.upgrade() else { return };
                if sidebar.order() != SessionOrder::Manual {
                    return;
                }
                let target = {
                    let w = sidebar.viewport.borrow();
                    drop_target_at(&w.all_entries, row_height_for(&ui), target_y)
                };
                let Some(target) = target else {
                    return;
                };
                let rows = sidebar.cache.borrow();
                let updated = {
                    let state = sidebar.settings.state.borrow();
                    apply_manual_move(&rows, &state.manual_session_order, source.as_str(), &target)
                };
                if let Some(mut next) = updated {
                    if sidebar.catalog_ready.get() {
                        reconcile_manual_order(&rows, &mut next);
                    }
                    sidebar.settings.state.borrow_mut().manual_session_order = next;
                    if let Err(error) = sidebar.settings.persist() {
                        eprintln!("[settings] {error}");
                    }
                    sidebar.project_roster(&ui, &rows);
                }
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_search_picked(move |id| {
                let Some(ui) = weak.upgrade() else { return };
                select_from_overlay(&ui, id);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_notice_picked(move |id| {
                let Some(ui) = weak.upgrade() else { return };
                crate::notify::clear(&ui);
                if id.is_empty() {
                    return;
                }
                select_from_overlay(&ui, id);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_notice_dismissed(move || {
                let Some(ui) = weak.upgrade() else { return };
                crate::notify::clear(&ui);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_search_closed(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_search_field_typed(false);
                ui.set_search_open(false);
                ui.invoke_focus_composer();
            });
        }
        ui.on_opens_search(|text, control, search_open| {
            search::keys::opens_search(text.as_str(), control, search_open)
        });

        {
            let weak = ui.as_weak();
            ui.on_share_copy_requested(move || {
                let Some(ui) = weak.upgrade() else { return };
                let url = ui.get_share_url().to_string();
                if url.is_empty() {
                    return;
                }
                match copy_to_clipboard(&url) {
                    Ok(()) => {
                        ui.set_share_copied(true);
                        ui.set_share_action_error("".into());
                    }
                    Err(error) => {
                        ui.set_share_copied(false);
                        ui.set_share_action_error(format!("could not copy: {error}").into());
                    }
                }
            });
        }
        {
            // The viewer is a browser page that spends GitHub's unauthenticated
            // quota. Check that quota first, on the worker, so an exhausted
            // network gets a sentence that says so instead of a blank viewer.
            let jobs = job_tx.clone();
            ui.on_share_open_link_requested(move || {
                let _ = jobs.send(Job::CheckViewerQuota);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_share_dismissed(move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_share_open(false);
                ui.set_settings_menu(false);
                ui.invoke_focus_composer();
            });
        }
    }
}

#[cfg(test)]
pub(crate) mod virtual_sidebar_tests {
    use super::*;
    use std::time::Duration;

    fn make_roster(active: usize, idle: usize, done: usize) -> Vec<SessionRow> {
        let mut rows = Vec::with_capacity(active + idle + done);
        let mut sec = 59u32;
        let mut min = 59u32;
        let mut hour = 23u32;
        let mut next_stamp = || {
            let s = format!("2026-11-10T{hour:02}:{min:02}:{sec:02}Z");
            if sec > 0 {
                sec -= 1;
            } else {
                sec = 59;
                if min > 0 {
                    min -= 1;
                } else {
                    min = 59;
                    hour = hour.saturating_sub(1);
                }
            }
            s
        };
        for i in 0..active {
            rows.push(SessionRow::from_value(&serde_json::json!({
                "activeSessionId": format!("a-{i:03}"),
                "sessionId": format!("ua-{i:03}"),
                "firstMessage": format!("Active session {i:03}"),
                "workerState": "ready",
                "activity": "working",
                "lastActivityAt": next_stamp(),
            })));
        }
        for i in 0..idle {
            rows.push(SessionRow::from_value(&serde_json::json!({
                "activeSessionId": format!("i-{i:03}"),
                "sessionId": format!("ui-{i:03}"),
                "firstMessage": format!("Idle session {i:03}"),
                "workerState": "ready",
                "activity": "idle",
                "lastActivityAt": next_stamp(),
            })));
        }
        for i in 0..done {
            rows.push(SessionRow::from_value(&serde_json::json!({
                "activeSessionId": format!("d-{i:03}"),
                "sessionId": format!("ud-{i:03}"),
                "firstMessage": format!("Done session {i:03}"),
                "lifecycle": "archived",
                "lastActivityAt": next_stamp(),
            })));
        }
        rows
    }

    fn setup_window() -> (AppWindow, Rc<Sidebar>, crossbeam_channel::Receiver<Job>, std::path::PathBuf) {
        let path = std::env::var("PA_SETTINGS_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::temp_dir().join(format!("pa-ui-sidebar-vtest-{}.json", std::process::id()))
            });
        let _ = std::fs::remove_file(&path);
        let ui = AppWindow::new().expect("window");
        let (job_tx, job_rx) = crossbeam_channel::unbounded::<Job>();
        let sidebar = Sidebar::new(&ui, &job_tx);
        (ui, sidebar, job_rx, path)
    }

    fn seeded_large_roster_bounds_mounted_session_entries_to_viewport_and_overscan() {
        let (ui, sidebar, _jobs, path) = setup_window();
        let rows = make_roster(10, 20, 125);
        sidebar.seed(&ui, rows);

        let entries = ui.get_session_entries();
        let mounted = entries.row_count();
        assert!(mounted > 0 && mounted <= 64, "expected bounded mounted entries, got {mounted}");

        let expected_h = (COLUMN_TOP + 3 * HEADER_HEIGHT + 155 * ROW_HEIGHT) as f32;
        assert_eq!(ui.get_sessions_height(), expected_h);

        let mounted_list: Vec<SessionEntry> = (0..mounted).filter_map(|i| entries.row_data(i)).collect();
        assert!(mounted_list.iter().any(|e| e.is_header && e.label == "Active"));
        assert!(mounted_list.iter().any(|e| !e.is_header && e.item.id == "a-000"));
        assert!(!mounted_list.iter().any(|e| !e.is_header && e.item.id == "d-124"));
        let _ = std::fs::remove_file(&path);
    }

    fn collapsing_a_group_omits_folded_rows_from_mounted_slice_once_animation_completes() {
        let (ui, sidebar, _jobs, path) = setup_window();
        let rows = make_roster(5, 5, 145);
        sidebar.seed(&ui, rows);

        let done_count_mounted = || {
            let entries = ui.get_session_entries();
            (0..entries.row_count())
                .filter_map(|i| entries.row_data(i))
                .filter(|e| !e.is_header && e.seam == 2)
                .count()
        };
        assert!(done_count_mounted() > 0, "Done rows should start visible in viewport");
        assert!(ui.get_session_entries().row_count() <= 64);

        ui.invoke_setting_toggled("reduce-motion".into());
        ui.invoke_group_toggled("Done".into());
        assert_eq!(done_count_mounted(), 0, "instant collapse must omit folded Done rows immediately");
        assert!(
            (0..ui.get_session_entries().row_count())
                .filter_map(|i| ui.get_session_entries().row_data(i))
                .any(|e| e.is_header && e.label == "Done")
        );

        ui.invoke_group_toggled("Done".into());
        assert!(done_count_mounted() > 0);
        ui.invoke_setting_toggled("reduce-motion".into());
        assert!(crate::Appearance::get(&ui).get_fold_duration() > 0);

        ui.show().expect("show window");
        ui.invoke_restore_sidebar_cy(-600.0);
        let cy_before_fold = ui.invoke_get_sidebar_cy();
        assert_eq!(cy_before_fold, -600.0);
        ui.invoke_group_toggled("Done".into());
        assert_eq!(ui.invoke_get_sidebar_cy(), -600.0);
        assert!(
            ui.get_session_entries().row_count() <= 64,
            "mounted count must remain bounded during fold animation"
        );

        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::SingleShot, Duration::from_millis(320), || {
            let _ = slint::quit_event_loop();
        });
        slint::run_event_loop().expect("event loop");
        drop(timer);
        let _ = ui.hide();

        assert_eq!(
            done_count_mounted(),
            0,
            "completed fold animation must omit folded Done rows from mounted slice"
        );
        let _ = std::fs::remove_file(&path);
    }

    fn scrolling_updates_mounted_slice_while_preserving_height_selection_menu_and_reorder_coordinates() {
        let (ui, sidebar, _jobs, path) = setup_window();
        let rows = make_roster(10, 20, 125);
        sidebar.apply_roster(&ui, rows);
        ui.invoke_setting_chosen("session-order".into(), "Manual".into());

        let expected_h = (COLUMN_TOP + 3 * HEADER_HEIGHT + 155 * ROW_HEIGHT) as f32;
        assert_eq!(ui.get_sessions_height(), expected_h);

        ui.set_selected_id("d-080".into());
        ui.invoke_restore_sidebar_cy(-3200.0);

        assert_eq!(ui.get_sessions_height(), expected_h);
        let entries = ui.get_session_entries();
        let mounted = entries.row_count();
        assert!(mounted > 0 && mounted <= 64, "mounted slice must stay bounded after scroll: {mounted}");

        let mounted_list: Vec<SessionEntry> = (0..mounted).filter_map(|i| entries.row_data(i)).collect();
        assert!(!mounted_list.iter().any(|e| !e.is_header && e.item.id == "a-000"));
        let d080 = mounted_list
            .iter()
            .find(|e| !e.is_header && e.item.id == "d-080")
            .expect("d-080 should be mounted near scroll offset -3200");
        let expected_d080_at = (COLUMN_TOP + 3 * HEADER_HEIGHT + (10 + 20 + 80) * ROW_HEIGHT) as f32;
        assert_eq!(d080.at, expected_d080_at);
        assert_eq!(ui.invoke_row_y(0), mounted_list[0].at);
        assert_eq!(ui.get_selected_id().as_str(), "d-080");
        assert!(!d080.item.full.is_empty());

        ui.invoke_restore_sidebar_cy(-3328.0);
        let scroll_ops = sidebar.viewport.borrow().last_ops.clone();
        assert!(!scroll_ops.is_empty());
        assert!(
            !scroll_ops.contains(&SidebarModelOp::Reset),
            "smooth scroll must patch mounted slice incrementally: {scroll_ops:?}"
        );

        ui.set_menu_id("d-080".into());
        ui.invoke_session_menu_opened("d-080".into());
        assert!(ui.get_menu_items().row_count() >= 3);

        let d082_at = mounted_list
            .iter()
            .find(|e| !e.is_header && e.item.id == "d-082")
            .map(|e| e.at)
            .expect("d-082 mounted");
        ui.invoke_session_dropped("d-080".into(), d082_at + 16.0);
        let after_entries = ui.get_session_entries();
        let moved_at = (0..after_entries.row_count())
            .filter_map(|i| after_entries.row_data(i))
            .find(|e| !e.is_header && e.item.id == "d-080")
            .map(|e| e.at)
            .expect("d-080 still mounted after manual drop");
        assert_eq!(moved_at, d082_at);
        assert_eq!(ui.get_sessions_height(), expected_h);
        let _ = std::fs::remove_file(&path);
    }

    fn routine_roster_updates_patch_mounted_vecmodel_incrementally_without_swapping_modelrc() {
        let (ui, sidebar, _jobs, path) = setup_window();
        let mut rows = make_roster(10, 20, 125);
        sidebar.apply_roster(&ui, rows.clone());

        let model_before = ui.get_session_entries();
        let count_before = model_before.row_count();
        assert!(count_before > 0);

        rows[1].name = "Renamed active session".to_string();
        sidebar.apply_roster(&ui, rows.clone());

        let rename_ops = sidebar.viewport.borrow().last_ops.clone();
        assert_eq!(rename_ops, vec![SidebarModelOp::Set { at: 2 }]);

        let updated = (0..model_before.row_count())
            .filter_map(|i| model_before.row_data(i))
            .find(|e| !e.is_header && e.item.id == "a-001")
            .expect("a-001 still in mounted model_before without swapping ModelRc");
        assert_eq!(updated.item.name.as_str(), "Renamed active session");

        rows.insert(
            0,
            SessionRow::from_value(&serde_json::json!({
                "activeSessionId": "a-new",
                "sessionId": "ua-new",
                "firstMessage": "Brand new active",
                "workerState": "ready",
                "activity": "working",
                "lastActivityAt": "2026-11-11T00:00:00Z",
            })),
        );
        sidebar.apply_roster(&ui, rows);

        let insert_ops = sidebar.viewport.borrow().last_ops.clone();
        assert!(!insert_ops.is_empty());
        assert!(!insert_ops.contains(&SidebarModelOp::Reset), "routine insert must patch incrementally: {insert_ops:?}");

        let has_new = (0..model_before.row_count())
            .filter_map(|i| model_before.row_data(i))
            .any(|e| !e.is_header && e.item.id == "a-new");
        assert!(has_new, "same ModelRc must be patched incrementally with inserted row");
        assert!(model_before.row_count() <= 64);
        let _ = std::fs::remove_file(&path);
    }

    pub(crate) fn run_all() {
        seeded_large_roster_bounds_mounted_session_entries_to_viewport_and_overscan();
        collapsing_a_group_omits_folded_rows_from_mounted_slice_once_animation_completes();
        scrolling_updates_mounted_slice_while_preserving_height_selection_menu_and_reorder_coordinates();
        routine_roster_updates_patch_mounted_vecmodel_incrementally_without_swapping_modelrc();
    }
}