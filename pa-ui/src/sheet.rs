//! The sheet: the rows the reader sees, and the list that draws them.
//!
//! Before this module the rows existed twice - a store the window reasoned about
//! and a Slint model beside it - and every caller that changed one had to
//! remember the other. The rule "both must move together" was a comment. Here it
//! is a field: nothing outside can address a row by index except through the
//! methods below, and each of those keeps the two in step by construction.
//!
//! The one idea that makes the surface small: an answer still being written is
//! not a special kind of thing. It is a message whose rows keep changing, so a
//! merge already knows how to patch it in place. The sheet only has to remember
//! that one message is provisional, because new rows from the store belong
//! above it, not below.

use crate::*;
use std::cell::Ref;

/// One order to whatever draws the sheet. The production adapter turns these
/// into Slint list mutations one for one; the test adapter writes them down, so
/// "one changed row costs one redraw" can be asserted instead of hoped for.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Reset { rows: usize },
    Set { at: usize },
    Insert { at: usize },
    Remove { at: usize },
}

pub(crate) trait Redraw {
    fn reset(&mut self, rows: Vec<TRow>);
    fn set(&mut self, at: usize, row: TRow);
    fn insert(&mut self, at: usize, row: TRow);
    fn remove(&mut self, at: usize);
}

/// The production adapter: the Slint list the window draws.
struct SlintRows(Rc<VecModel<TRow>>);

impl Redraw for SlintRows {
    fn reset(&mut self, rows: Vec<TRow>) {
        self.0.set_vec(rows);
    }
    fn set(&mut self, at: usize, row: TRow) {
        self.0.set_row_data(at, row);
    }
    fn insert(&mut self, at: usize, row: TRow) {
        self.0.insert(at, row);
    }
    fn remove(&mut self, at: usize) {
        self.0.remove(at);
    }
}

pub(crate) struct Sheet {
    inner: RefCell<Inner>,
}

struct Inner {
    rows: Vec<Row>,
    /// which message each key's rows occupy, so a poll patches what changed and
    /// never re-reads the transcript
    index: std::collections::HashMap<String, std::ops::Range<usize>>,
    /// the message whose rows are still being written, if any
    open: Option<String>,
    /// where the notice sits while a stretch of history is unloaded
    gap: Option<usize>,
    drawn: Box<dyn Redraw>,
}

impl Sheet {
    /// The one Slint fact in the interface: the list the window shows. The
    /// caller installs it once; everything after that is rows in, orders out.
    pub fn new() -> (Self, ModelRc<TRow>) {
        let model = Rc::new(VecModel::<TRow>::default());
        let sheet = Self {
            inner: RefCell::new(Inner {
                rows: Vec::new(),
                index: std::collections::HashMap::new(),
                open: None,
                gap: None,
                drawn: Box::new(SlintRows(model.clone())),
            }),
        };
        (sheet, ModelRc::from(model))
    }

    /// What the reader sees. Tests assert here rather than on the drawn list:
    /// the list is an adapter, so it may not be the only way to observe.
    pub fn rows(&self) -> Ref<'_, Vec<Row>> {
        Ref::map(self.inner.borrow(), |inner| &inner.rows)
    }

    /// A whole chat arrived - an attach, a new chat, a reconnect. The only call
    /// that deletes: whatever was on the sheet is not this chat.
    pub fn replace(&self, rows: Vec<Row>) {
        let mut inner = self.inner.borrow_mut();
        inner.index.clear();
        inner.open = None;
        inner.gap = None;
        for (offset, row) in rows.iter().enumerate() {
            let key = row.message.clone();
            if key.is_empty() {
                continue;
            }
            match inner.index.get_mut(&key) {
                Some(range) => range.end = offset + 1,
                None => {
                    inner.index.insert(key, offset..offset + 1);
                }
            }
        }
        inner.drawn.reset(rows.iter().map(to_slint).collect());
        inner.rows = rows;
    }

    /// The newest page arrived, or the answer as it stands. A message already on
    /// the sheet is patched where it stands; one it has never seen is placed
    /// above an open answer, so the answer keeps writing at the bottom.
    pub fn merge(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        for (key, range) in group_by_message(&page) {
            let fresh = &page[range];
            if key.is_empty() {
                inner.place(fresh);
                continue;
            }
            match inner.index.get(&key).cloned() {
                Some(current) => inner.patch(&key, current, fresh),
                None => inner.place_keyed(&key, fresh),
            }
        }
    }

    /// A page above what is on the sheet. Messages already present are dropped
    /// from the page rather than drawn twice.
    pub fn prepend(&self, page: Vec<Row>) {
        if page.is_empty() {
            return;
        }
        let mut inner = self.inner.borrow_mut();
        let fresh: Vec<(String, std::ops::Range<usize>)> = group_by_message(&page)
            .into_iter()
            .filter(|(key, _)| !inner.index.contains_key(key))
            .collect();
        let total: usize = fresh.iter().map(|(_, range)| range.len()).sum();
        if total == 0 {
            return;
        }
        for range in inner.index.values_mut() {
            range.start += total;
            range.end += total;
        }
        let mut at = 0usize;
        for (key, range) in fresh {
            let len = range.len();
            for row in &page[range] {
                inner.rows.insert(at, row.clone());
                inner.drawn.insert(at, to_slint(row));
                at += 1;
            }
            inner.index.insert(key, at - len..at);
        }
    }

    /// This message's rows are provisional: the store has not carried it yet.
    pub fn answer_began(&self, key: &str) {
        if !key.is_empty() {
            self.inner.borrow_mut().open = Some(key.to_string());
        }
    }

    /// The answer is finished. Its rows stay exactly where they are; the store's
    /// copy of the same message replaces them when it arrives.
    pub fn answer_ended(&self) {
        self.inner.borrow_mut().open = None;
    }

    /// The reader opened or closed one row.
    pub fn toggle(&self, at: usize) {
        let mut inner = self.inner.borrow_mut();
        let Some(row) = inner.rows.get_mut(at) else { return };
        row.expanded = !row.expanded;
        let drawn = to_slint(row);
        inner.drawn.set(at, drawn);
    }

    /// Unload the stretch of history nearest the newest page, keeping at most
    /// keep_rows rows above it, and leave one notice row where it was.
    ///
    /// The budget is rows because rows are what the window pays for: one row
    /// measured costs about a millisecond and a half to lay out, whatever the
    /// message it came from.
    ///
    /// A session of any length must not cost the window more than a window. The
    /// rows the reader is looking at stay; the rows between them and the newest
    /// page - which nobody is reading - go, and the notice stands for them until
    /// a page fills the hole. Returns the rows that went.
    pub fn narrow(&self, newest_page: &str, keep: usize, note: &str) -> usize {
        let mut inner = self.inner.borrow_mut();
        let Some(from) = inner.index.get(newest_page).map(|range| range.start) else {
            return 0;
        };
        // the notice is not history: it is the hole itself
        let seam = inner.gap.unwrap_or(from);
        let mut drop_from = seam;
        while drop_from > keep {
            let key = inner.rows[drop_from - 1].message.clone();
            if key.is_empty() {
                drop_from -= 1;
                continue;
            }
            let Some(range) = inner.index.get(&key).cloned() else {
                break;
            };
            if range.end != drop_from {
                break;
            }
            drop_from = range.start;
        }
        if drop_from == seam {
            if crate::env_cfg::cfg().debug {
                eprintln!(
                    "[narrow] nothing to do: held={seam} keep={keep} rows={}",
                    inner.rows.len()
                );
            }
            return 0;
        }
        let stop = if inner.gap.is_some() { seam + 1 } else { seam };
        for at in (drop_from..stop).rev() {
            inner.rows.remove(at);
            inner.drawn.remove(at);
        }
        let row = notice(note);
        inner.rows.insert(drop_from, row.clone());
        inner.drawn.insert(drop_from, to_slint(&row));
        inner.gap = Some(drop_from);
        // a hole is rare; reading the keys back is cheaper than tracking them
        inner.index = reindex(&inner.rows);
        if crate::env_cfg::cfg().debug {
            eprintln!(
                "[narrow] held={seam} keep={keep} dropped {} rows; sheet now {} rows",
                stop - drop_from,
                inner.rows.len()
            );
        }
        stop - drop_from
    }

    /// Put a page back where the notice stands, and retire the notice.
    pub fn fill(&self, rows: Vec<Row>) -> usize {
        let mut inner = self.inner.borrow_mut();
        let Some(at) = inner.gap else {
            return 0;
        };
        inner.rows.remove(at);
        inner.drawn.remove(at);
        let held = rows.len();
        for (offset, row) in rows.iter().enumerate() {
            inner.rows.insert(at + offset, row.clone());
            inner.drawn.insert(at + offset, to_slint(row));
        }
        inner.gap = None;
        inner.index = reindex(&inner.rows);
        held
    }

    /// The daemon refused the prompt: the row drawn for it goes back out.
    pub fn take_back_prompt(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        let echo = serde_json::json!({
            "role": "user",
            "content": [{ "type": "text", "text": text }],
        });
        let Some(candidate) = transcript::build(std::slice::from_ref(&echo), 1).0.into_iter().next()
        else {
            return;
        };
        let mut inner = self.inner.borrow_mut();
        let Some(last) = inner.rows.last() else { return };
        if last.kind != candidate.kind || last.body != candidate.body {
            return;
        }
        let at = inner.rows.len() - 1;
        let key = inner.rows[at].message.clone();
        inner.rows.remove(at);
        inner.drawn.remove(at);
        let spread = inner.index.get(&key).map(|range| (range.start, range.end));
        match spread {
            Some((start, end)) if end > start + 1 => {
                if let Some(range) = inner.index.get_mut(&key) {
                    range.end -= 1;
                }
            }
            Some(_) => {
                inner.index.remove(&key);
            }
            None => {}
        }
    }
}

impl Inner {
    /// Place rows the sheet has never seen, above an open answer if there is
    /// one: the answer is the newest thing on the sheet, and stays there.
    fn landing(&self) -> usize {
        self.open
            .as_ref()
            .and_then(|key| self.index.get(key))
            .map_or(self.rows.len(), |range| range.start)
    }

    fn insert_rows(&mut self, at: usize, rows: &[Row]) {
        for (offset, row) in rows.iter().enumerate() {
            self.rows.insert(at + offset, row.clone());
            self.drawn.insert(at + offset, to_slint(row));
        }
    }

    fn place(&mut self, rows: &[Row]) {
        let at = self.landing();
        self.insert_rows(at, rows);
        self.shift_from(at, rows.len() as isize);
    }

    fn place_keyed(&mut self, key: &str, rows: &[Row]) {
        let at = self.landing();
        self.insert_rows(at, rows);
        self.shift_from(at, rows.len() as isize);
        self.index.insert(key.to_string(), at..at + rows.len());
    }

    /// Every message that started at or after a position moved by a delta.
    fn shift_from(&mut self, at: usize, delta: isize) {
        if delta == 0 {
            return;
        }
        for range in self.index.values_mut() {
            if range.start >= at {
                range.start = (range.start as isize + delta) as usize;
                range.end = (range.end as isize + delta) as usize;
            }
        }
    }

    /// A message already on the sheet: write only what differs, and only as many
    /// orders as the change needs - one per changed row, one insert for a block
    /// that grew, one remove for a block that shrank.
    fn patch(&mut self, key: &str, current: std::ops::Range<usize>, fresh: &[Row]) {
        let opened: std::collections::HashSet<String> = self.rows[current.clone()]
            .iter()
            .filter(|row| row.expanded)
            .map(row_identity)
            .collect();
        let adjusted: Vec<Row> = fresh
            .iter()
            .map(|row| {
                let mut row = row.clone();
                row.expanded = opened.contains(&row_identity(&row));
                row
            })
            .collect();

        let start = current.start;
        let had = current.len();
        let common = had.min(adjusted.len());
        for offset in 0..common {
            let at = start + offset;
            if self.rows[at] == adjusted[offset] {
                continue;
            }
            self.rows[at] = adjusted[offset].clone();
            self.drawn.set(at, to_slint(&adjusted[offset]));
        }
        if adjusted.len() > had {
            self.insert_rows(start + had, &adjusted[had..]);
            self.shift_from(start + 1, (adjusted.len() - had) as isize);
            self.index.insert(key.to_string(), start..start + adjusted.len());
        } else if adjusted.len() < had {
            for at in (start + adjusted.len()..start + had).rev() {
                self.rows.remove(at);
                self.drawn.remove(at);
            }
            self.shift_from(start + 1, -((had - adjusted.len()) as isize));
            self.index.insert(key.to_string(), start..start + adjusted.len());
        }
    }
}

/// The unit of every call is a whole message: a message's rows are contiguous,
/// and its key is how a rebuild finds them again.
fn group_by_message(rows: &[Row]) -> Vec<(String, std::ops::Range<usize>)> {
    let mut out: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match out.last_mut() {
            Some((key, range)) if *key == row.message => range.end = index + 1,
            _ => out.push((row.message.clone(), index..index + 1)),
        }
    }
    out
}

/// A row that stands for rows that are not loaded.
fn notice(note: &str) -> Row {
    let mut row = Row {
        kind: transcript::KIND_NOTICE,
        id: String::new(),
        title: String::new(),
        meta: String::new(),
        body: note.to_string(),
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
        message: String::new(),
    };
    row.expanded = false;
    row
}

/// Every message key and the rows it holds, read off a finished row list.
fn reindex(rows: &[Row]) -> std::collections::HashMap<String, std::ops::Range<usize>> {
    let mut out: std::collections::HashMap<String, std::ops::Range<usize>> =
        std::collections::HashMap::new();
    for (offset, row) in rows.iter().enumerate() {
        if row.message.is_empty() {
            continue;
        }
        match out.get_mut(&row.message) {
            Some(range) => range.end = offset + 1,
            None => {
                out.insert(row.message.clone(), offset..offset + 1);
            }
        }
    }
    out
}

/// Stable identity of a row across a rebuild: the tool call's id when it has
/// one, otherwise the message it belongs to plus its kind and its title.
fn row_identity(row: &Row) -> String {
    if !row.id.is_empty() {
        return format!("id:{}", row.id);
    }
    format!("{}#{}#{}", row.message, row.kind, row.title)
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

/// Write the rows out where a duplicate can be counted instead of guessed at.
pub(crate) fn dump(rows: &[Row]) {
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

/// The last two rows, by kind, once per interesting moment. Only at PA_DEBUG.
pub(crate) fn trace(what: &str, rows: &[Row]) {
    if !crate::env_cfg::cfg().debug {
        return;
    }
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

#[cfg(test)]
mod tests;
