//! The search popup: opening it, matching as the reader types, picking a
//! session out of it, and closing it again.
//!
//! The window holds the field's text; nothing here writes it back. Writing the
//! text from inside the callback the field's own edit raised would stop the
//! field after its first character.

use crate::*;

/// Opening: one row per session, in the order the daemon lists them, and frozen
/// there until the reader types again. The window's two-second poll keeps the
/// sidebar current; this list deliberately does not follow it, so nothing
/// reorders under the arrow keys while the popup is open.
///
/// The rows are matched with whatever is already in the field, which is nothing
/// when the magnifier or Ctrl+K opened it.
pub(crate) fn opened(ui: &AppWindow, sessions: &Rc<RefCell<Vec<SessionRow>>>) {
    let sessions = sessions.clone();
    let weak = ui.as_weak();
    ui.on_search_open_requested(move || {
        let Some(ui) = weak.upgrade() else { return };
        open(&ui, &sessions.borrow());
    });
}

/// Opening, once the window is in hand: the rows for whatever the field already
/// holds, the highlight on the best of them, and the popup up. The snapshot hook
/// comes through here too, so the frame it writes is the frame the key opens.
pub(crate) fn open(ui: &AppWindow, sessions: &[SessionRow]) {
    let query = ui.get_search_query();
    ui.set_search_rows(match_rows(sessions, query.as_str()));
    ui.set_search_index(0);
    ui.set_search_field_typed(false);
    ui.set_search_open(true);
    // The caret goes into the field in the same step that opens the popup: a
    // frame later the chain from the key the scope receives to the leaf that
    // holds the text is not built, and a focus asked for then never reaches the
    // letters.
    ui.set_search_focus_tick(ui.get_search_focus_tick() + 1);
}

/// A character arrived in the field: the rows narrow to what it matches, and the
/// highlight falls back to the best match, because the row it was on may be gone.
pub(crate) fn typed(ui: &AppWindow, rows: &Rc<RefCell<Vec<SessionRow>>>) {
    let weak = ui.as_weak();
    let rows = rows.clone();
    ui.on_search_typed(move |query| {
        let Some(ui) = weak.upgrade() else { return };
        // the field already holds the text: only the rows catch up with it, and
        // they are set before the highlight that indexes into them
        let found = match_rows(&rows.borrow(), query.as_str());
        ui.set_search_index(0);
        ui.set_search_rows(found);
    });
}

/// Picking a session: the two things the sidebar's own click does, plus the
/// popup closing. `selected-id` is what the poll loop watches, so setting it is
/// what loads the transcript. Focus goes back to the words the reader was
/// writing, because picking a session is not the end of the turn.
pub(crate) fn picked(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_search_picked(move |id| {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_model_menu(false);
        ui.set_queue_open(false);
        ui.set_selected_id(id);
        ui.set_hover_title(SharedString::new());
        ui.set_search_field_typed(false);
        ui.set_search_open(false);
        ui.invoke_focus_composer();
    });
}

/// Closing without a pick. Escape lands here as well as the cross and a click
/// outside, because the popup answers all three the same way. The window raises
/// this only while the popup is open and closes it in the same call, which is
/// what makes it safe to bind `closed` straight to the callback.
pub(crate) fn closed(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_search_closed(move || {
        let Some(ui) = weak.upgrade() else { return };
        ui.set_search_field_typed(false);
        ui.set_search_open(false);
        ui.invoke_focus_composer();
    });
}

/// The rows the popup draws for one query: each name cut to what a row has room
/// for and drawn as the glyphs the row is built from. The glyphs are looked up
/// in the cut name itself, which is the string the popup draws, so a mark can
/// never land on a character the row does not show.
fn match_rows(sessions: &[SessionRow], query: &str) -> ModelRc<SearchRow> {
    let rows: Vec<SearchRow> = search::matches(sessions, query)
        .into_iter()
        .map(|(row, offsets)| {
            let name = search::elide(&sessions::display_name(row));
            SearchRow {
                id: row.id.as_str().into(),
                age: sessions::age_label(row),
                glyphs: ModelRc::from(Rc::new(VecModel::from(search::glyphs(&name, &offsets)))),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

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

    /// The popup opens on every session, so a cache with rows in it can never
    /// draw "No sessions yet". Matching the cache is the whole of what opening
    /// does to it, so an empty popup means an empty cache, not a lost match.
    #[test]
    fn opening_on_a_populated_cache_lists_every_session() {
        let rows = [
            daemon_row("1", "prime-agent"),
            daemon_row("2", "hello"),
            daemon_row("3", "do some random tool calls"),
        ];
        assert_eq!(match_rows(&rows, "").row_count(), 3);
    }

    /// Rows are drawn by session id, which is what a pick hands back.
    #[test]
    fn each_row_carries_the_sessions_id() {
        let rows = [daemon_row("1", "prime-agent")];
        let entries = match_rows(&rows, "");
        assert_eq!(entries.row_data(0).expect("a row").id.as_str(), "1");
    }

    /// The session with no title still appears, named the way the sidebar names
    /// it: the folder's name, and the id so two such sessions stay apart. The row
    /// carries one glyph for each character of that name, and no more.
    #[test]
    fn an_unnamed_session_still_draws_a_name() {
        let mut row = daemon_row("abc", "");
        row.cwd = "C:\\work\\prime-agent".to_string();
        let name = sessions::display_name(&row);
        let entries = match_rows(&[row], "");
        let glyphs = entries.row_data(0).expect("a row").glyphs;
        assert_eq!(glyphs.row_count(), name.chars().count());
        let drawn: String = (0..glyphs.row_count())
            .map(|index| glyphs.row_data(index).expect("a glyph").text.to_string())
            .collect();
        assert_eq!(drawn, "prime-agent · abc");
    }

    /// A name the row cuts keeps the marks on the letters it still shows, and a
    /// mark past the cut is dropped rather than moved onto the wrong letter.
    #[test]
    fn a_cut_name_marks_only_the_letters_it_shows() {
        let row = daemon_row("1", "alpha beta gamma delta epsilon zeta eta theta iota");
        let entries = match_rows(&[row], "beta");
        let glyphs = entries.row_data(0).expect("a row").glyphs;
        let marked: String = (0..glyphs.row_count())
            .filter(|index| glyphs.row_data(*index).expect("a glyph").matched)
            .map(|index| glyphs.row_data(index).expect("a glyph").text.to_string())
            .collect();
        assert_eq!(marked, "beta");
    }

    /// An empty cache is the one case that draws "No sessions yet", and it must
    /// stay distinguishable from a query that matched nothing.
    #[test]
    fn an_empty_cache_draws_no_rows() {
        assert_eq!(match_rows(&[], "").row_count(), 0);
    }
}
