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

pub(crate) fn session_items(rows: &[SessionRow]) -> Vec<SessionItem> {
    let mut top: Vec<&SessionRow> = rows.iter().filter(|row| is_top_level(row)).collect();
    top.sort_by(|left, right| {
        tag_of(left)
            .rank()
            .cmp(&tag_of(right).rank())
            .then_with(|| recency(right).cmp(&recency(left)))
            .then_with(|| display_name(left).cmp(&display_name(right)))
            .then_with(|| left.id.cmp(&right.id))
    });
    top.into_iter().map(to_item).collect()
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
    let (active, idle, done) = groups;
    let first_fill = was.0.len() + was.1.len() + was.2.len() == 0 && folded == was_folded;
    let previous = if first_fill {
        Vec::new()
    } else {
        session_entries(was.0, was.1, was.2, was_folded[0], was_folded[1], was_folded[2])
    };
    let entries = session_entries(active, idle, done, folded[0], folded[1], folded[2]);
    let from_y = if first_fill {
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

pub(crate) fn open_search(ui: &AppWindow, sessions: &[SessionRow]) {
    // The search popup and the share card are the two surfaces that open over
    // the whole window; only one of them is ever up.
    ui.set_share_open(false);
    ui.set_settings_menu(false);
    let query = ui.get_search_query();
    ui.set_search_rows(match_rows(sessions, query.as_str()));
    ui.set_search_index(0);
    ui.set_search_field_typed(false);
    ui.set_search_open(true);
    ui.set_search_focus_tick(ui.get_search_focus_tick() + 1);
}

/// Stateful Sidebar module owning the session roster, fold state, UUID rebinding,
/// context menu confirmation, and search popup.
pub(crate) struct Sidebar {
    pub cache: Rc<RefCell<Vec<SessionRow>>>,
    pub selected_uuid: RefCell<Option<String>>,
    groups: Rc<RefCell<(Vec<SessionItem>, Vec<SessionItem>, Vec<SessionItem>)>>,
    settings: Rc<crate::settings::SettingsStore>,
}

impl Sidebar {
    pub fn new(ui: &AppWindow, job_tx: &Sender<Job>) -> Rc<Self> {
        let sidebar = Rc::new(Self {
            cache: Rc::new(RefCell::new(Vec::new())),
            selected_uuid: RefCell::new(None),
            groups: Rc::new(RefCell::new((Vec::new(), Vec::new(), Vec::new()))),
            settings: crate::settings::SettingsStore::new(crate::settings::default_settings_path()),
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

    pub fn family_cost(&self, session: &str, own_cost: f64) -> (f64, bool) {
        if let Ok(rows) = self.cache.try_borrow() {
            compute_family_cost(&rows, session, own_cost)
        } else {
            (own_cost, false)
        }
    }

    pub fn seed(&self, ui: &AppWindow, rows: Vec<SessionRow>) {
        let (active, idle, done) = groups_from(&rows);
        let col = column(
            (&active, &idle, &done),
            [false, false, false],
            (&[], &[], &[]),
            [false, false, false],
        );
        apply(ui, col, -1, false);
        ui.set_active_items(ModelRc::from(Rc::new(VecModel::from(active.clone()))));
        ui.set_idle_items(ModelRc::from(Rc::new(VecModel::from(idle.clone()))));
        ui.set_done_items(ModelRc::from(Rc::new(VecModel::from(done.clone()))));
        ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(session_items(&rows)))));
        *self.groups.borrow_mut() = (active, idle, done);
        *self.cache.borrow_mut() = rows;
    }

    /// Returns `Some(rebound_id)` if the active selection was re-keyed by the daemon.
    pub fn apply_roster(&self, ui: &AppWindow, rows: Vec<SessionRow>) -> Option<String> {
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
        let items: Vec<SessionItem> = session_items(&rows);
        let (active, idle, done) = session_groups(&items);
        let folded = [
            ui.get_active_folded(),
            ui.get_idle_folded(),
            ui.get_done_folded(),
        ];
        let col = {
            let prev = self.groups.borrow();
            column(
                (&active, &idle, &done),
                folded,
                (&prev.0, &prev.1, &prev.2),
                folded,
            )
        };
        let (previous_active, previous_idle, previous_done) = self.groups.borrow().clone();
        let active_changed = active != previous_active;
        let idle_changed = idle != previous_idle;
        let done_changed = done != previous_done;
        *self.groups.borrow_mut() = (active.clone(), idle.clone(), done.clone());
        if active_changed {
            ui.set_active_items(ModelRc::from(Rc::new(VecModel::from(active))));
        }
        if idle_changed {
            ui.set_idle_items(ModelRc::from(Rc::new(VecModel::from(idle))));
        }
        if done_changed {
            ui.set_done_items(ModelRc::from(Rc::new(VecModel::from(done))));
        }

        let any_group_move = any_moved(&col.previous, &col.entries);
        if col.changed() {
            apply(ui, col, if any_group_move { -2 } else { -1 }, false);
        }
        let visible_first = items.first().cloned();
        if active_changed || idle_changed || done_changed {
            ui.set_sessions(ModelRc::from(Rc::new(VecModel::from(items))));
        }

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

    fn bind_ui(&self, ui: &AppWindow, job_tx: &Sender<Job>) {
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
            let groups = self.groups.clone();
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
                let held = groups.borrow();
                let (active, idle, done) = if held.0.is_empty() && held.1.is_empty() && held.2.is_empty() {
                    groups_of(
                        &ui.get_active_items(),
                        &ui.get_idle_items(),
                        &ui.get_done_items(),
                    )
                } else {
                    held.clone()
                };
                drop(held);
                let col = column((&active, &idle, &done), now, (&active, &idle, &done), was);
                apply(&ui, col, index as i32, !now[index]);
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
                ui.on_setting_chosen(move |key, val| {
                    let Some(ui) = weak.upgrade() else { return };
                    let changed = {
                        let mut state = store.state.borrow_mut();
                        let ok = state.choose(key.as_str(), val.as_str());
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
            let sessions = self.cache.clone();
            let weak = ui.as_weak();
            ui.on_search_open_requested(move || {
                let Some(ui) = weak.upgrade() else { return };
                open_search(&ui, &sessions.borrow());
            });
        }
        {
            let weak = ui.as_weak();
            let rows = self.cache.clone();
            ui.on_search_typed(move |query| {
                let Some(ui) = weak.upgrade() else { return };
                let found = match_rows(&rows.borrow(), query.as_str());
                ui.set_search_index(0);
                ui.set_search_rows(found);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_search_picked(move |id| {
                let Some(ui) = weak.upgrade() else { return };
                ui.set_model_menu(false);
                ui.set_queue_open(false);
                let changed = ui.get_selected_id() != id;
                ui.set_selected_id(id.clone());
                if changed {
                    ui.invoke_selection_picked(id);
                }
                ui.set_hover_title(SharedString::new());
                ui.set_search_field_typed(false);
                ui.set_search_open(false);
                ui.invoke_focus_composer();
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