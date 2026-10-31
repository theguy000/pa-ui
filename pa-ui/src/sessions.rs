//! The rail's sessions: session rows, the rail's filter, and the name a session is shown under.

use crate::*;

pub(crate) fn to_item(row: &SessionRow) -> SessionItem {
    let status = if row.lifecycle != "live" {
        "parked"
    } else if row.activity == "working" || row.is_streaming {
        "working"
    } else {
        "idle"
    };
    let full = display_name(row);
    // every row gets a card: consistency was asked for explicitly, twice
    let tip = full.clone();
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
        status: status.into(),
        scroll: marquee_shift(&trim_to(&display_name(row), 90)),
    }
}


/// The rail's rows for a query: a session matches if its name or either id
/// carries the text, case aside. An empty query keeps everything.
pub(crate) fn session_items(rows: &[SessionRow], query: &str) -> Vec<SessionItem> {
    let needle = query.trim().to_lowercase();
    rows.iter()
        .filter(|row| {
            needle.is_empty()
                || display_name(row).to_lowercase().contains(&needle)
                || row.id.to_lowercase().contains(&needle)
                || row.session_id.to_lowercase().contains(&needle)
        })
        .map(to_item)
        .collect()
}


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
