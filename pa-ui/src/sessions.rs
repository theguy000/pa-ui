//! The sidebar's sessions: session rows, the sidebar's filter, and the name a session is shown under.

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
    let Some(seconds) = parse_iso_seconds(stamp) else {
        return SharedString::new();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    // A stamp in the future is the clock disagreeing with itself, not a session
    // used later today: read it as today rather than as a negative age.
    match (now - seconds).max(0) / 86_400 {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        days => format!("{days}d").into(),
    }
}


/// Every session, as the sidebar draws it, in the order the daemon gave them.
pub(crate) fn session_items(rows: &[SessionRow]) -> Vec<SessionItem> {
    rows.iter().map(to_item).collect()
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
