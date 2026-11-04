//! The search popup's matching: which sessions a typed query keeps, in which
//! order, and which letters of each name the row shows as the match.
//!
//! Names only. A query is matched against a session's name letter by letter, in
//! order, gaps allowed, so `msc` finds "Mefenamic Acid for Period Pain". A run of
//! letters found side by side scores higher than the same letters scattered, so
//! the tightest match sorts first, and nothing is dropped for scoring low: a
//! reader who typed the letters meant them.
//!
//! A row is drawn twice: the query matches the session's whole name, so a match
//! past the tail of what the row has room for is still found, and the name is
//! then cut to what the row shows, with the letters that fit drawn as matched.

use crate::*;

pub(crate) mod keys;

/// Every session a query keeps, best match first.
///
/// An empty query keeps every session in the order given, which is the order the
/// daemon lists them in: the popup opens on the whole column rather than on a
/// blank panel. The match offsets come back as character indices into `name`, so
/// a row can draw the letters it was found by.
pub(crate) fn matches<'a>(rows: &'a [SessionRow], query: &str) -> Vec<(&'a SessionRow, Vec<usize>)> {
    let needle: Vec<char> = query.trim().chars().flat_map(|c| c.to_lowercase()).collect();
    if needle.is_empty() {
        return rows.iter().map(|row| (row, Vec::new())).collect();
    }
    let mut kept: Vec<(&SessionRow, Vec<usize>, i64)> = rows
        .iter()
        .filter_map(|row| {
            let (score, offsets) = score(&name_of(row), &needle)?;
            Some((row, offsets, score))
        })
        .collect();
    // A stable sort keeps the daemon's order between sessions that score the
    // same, so a session does not jump when two of them match equally well.
    kept.sort_by_key(|(_, _, score)| -score);
    kept.into_iter().map(|(row, offsets, _)| (row, offsets)).collect()
}

/// The name a query is matched against: the title the session was given, whole,
/// rather than the sidebar's shortened form of it. A title longer than the
/// sidebar's cut is still the title, and a reader who remembers its ending should
/// find the session by it. The offsets are still character positions into the
/// title as the sidebar draws it, which is the same string while the title fits.
fn name_of(row: &SessionRow) -> String {
    let name = row.name.trim();
    if name.is_empty() || name == "(no messages)" {
        return sessions::display_name(row).to_string();
    }
    name.to_string()
}

/// The letters a query matched in a name, or nothing if it matched none of them.
///
/// Every match starts at the first place the query's opening letters appear and
/// the rest are taken where they fall, which is the earliest match there is. The
/// score is the width of the span those letters cover, so the letters side by
/// side sort first, and a match that begins a word gets a nudge ahead of one that
/// starts mid-word.
fn score(name: &str, needle: &[char]) -> Option<(i64, Vec<usize>)> {
    let hay: Vec<char> = name.chars().flat_map(|c| c.to_lowercase()).collect();
    let from = hay.iter().position(|c| *c == needle[0])?;
    let mut offsets = Vec::with_capacity(needle.len());
    let mut at = from;
    for want in needle {
        let found = hay[at..].iter().position(|c| c == want)?;
        offsets.push(at + found);
        at += found + 1;
    }
    // Side-by-side letters cover a narrower span than the same letters spread out,
    // which is the whole of what the scoring measures.
    let first = offsets[0];
    let begins_word = first == 0 || hay[first - 1].is_whitespace();
    // A match is as wide as the span it covers, and the span is what it costs the
    // reader to recognise the row. So a scattered match is scored by its width,
    // and a name's own length only breaks a tie between two spans of one size.
    let span = offsets[offsets.len() - 1] - first + 1;
    let score = if begins_word { 3 } else { 0 } - span as i64 * 4
        - (hay.len().min(80) as i64 / 20);
    Some((score, offsets))
}

/// How much of a name a row has room for beside its date. Long enough for any
/// title a session is given in full, and short enough that a row draws a row of
/// glyphs rather than a page of them.
const ROOM: usize = 46;

/// A name cut to the room a row has for it, with an ellipsis where the tail went
/// so the reader can see the name continues.
pub(crate) fn elide(name: &str) -> String {
    let characters: Vec<char> = name.chars().collect();
    if characters.len() <= ROOM {
        return name.to_string();
    }
    // one character spared for the ellipsis, so the cut name is no longer than
    // the room the row offered it
    characters[..ROOM - 1].iter().collect::<String>() + "\u{2026}"
}

/// A name as the popup's row draws it: one glyph per character, the offsets the
/// query matched marked. Slint has no way to colour a range of a string without
/// styling the whole of it, so the row is a run of Texts and this is their list.
pub(crate) fn glyphs(name: &str, offsets: &[usize]) -> Vec<SearchGlyph> {
    name.chars()
        .enumerate()
        .map(|(index, character)| SearchGlyph {
            text: character.to_string().into(),
            matched: offsets.contains(&index),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str) -> SessionRow {
        SessionRow {
            id: name.to_string(),
            session_id: name.to_string(),
            name: name.to_string(),
            ..SessionRow::default()
        }
    }

    fn names(rows: &[SessionRow], query: &str) -> Vec<String> {
        matches(rows, query)
            .into_iter()
            .map(|(row, _)| row.name.clone())
            .collect()
    }

    /// The row draws one Text per character, so what a row says is what its
    /// glyphs spell, in order.
    fn drawn(glyphs: &[SearchGlyph]) -> String {
        glyphs.iter().map(|glyph| glyph.text.to_string()).collect()
    }
    /// The popup opens on everything, and leaves the choice of order to the
    /// daemon rather than reordering a column the reader already knows.
    #[test]
    fn an_empty_query_keeps_every_session_in_the_order_given() {
        let rows = [row("Beta"), row("Alpha")];
        assert_eq!(names(&rows, ""), ["Beta", "Alpha"]);
        assert_eq!(names(&rows, "   "), ["Beta", "Alpha"]);
    }

    #[test]
    fn a_query_keeps_only_the_names_that_carry_its_letters_in_order() {
        // M, then the s of "MeSC", then the c that follows it.
        let rows = [row("MeSC Session"), row("Urban Roots"), row("Valheim")];
        assert_eq!(names(&rows, "msc"), ["MeSC Session"]);
        assert_eq!(names(&rows, "ess"), ["MeSC Session"]);
        // The letters are taken where they fall, so a query whose letters fall in
        // the other order finds nothing rather than reaching backwards.
        assert!(names(&rows, "csm").is_empty());
    }

    #[test]
    fn case_does_not_decide_a_match() {
        let rows = [row("GoogLe AI Studio Rate Limits")];
        assert_eq!(names(&rows, "google"), ["GoogLe AI Studio Rate Limits"]);
        assert_eq!(names(&rows, "GOOGLE"), ["GoogLe AI Studio Rate Limits"]);
    }

    /// Two names carry the same query somewhere in the middle, so neither opens
    /// with it: the one whose letters sit side by side comes first, because the
    /// narrower span is the one the reader most likely meant.
    #[test]
    fn a_snug_match_sorts_before_the_same_letters_scattered() {
        let rows = [row("Ace Mica Spore Cat"), row("_msc")];
        // "Ace Mica Spore Cat" finds M, S and C fourteen characters apart; "_msc"
        // finds them side by side.
        assert_eq!(names(&rows, "msc"), ["_msc", "Ace Mica Spore Cat"]);
    }

    /// The row draws the letters it matched, so the offsets have to point at the
    /// characters the query hit, not at the whole name.
    #[test]
    fn the_offsets_are_the_characters_the_query_hit() {
        let rows = [row("Urban Roots")];
        let hits: Vec<(String, Vec<usize>)> = matches(&rows, "ur")
            .into_iter()
            .map(|(row, offsets)| (row.name.clone(), offsets))
            .collect();
        assert_eq!(hits, [("Urban Roots".to_string(), vec![0, 1])]);

        let rows = [row("Fixing a Broken Windows Key")];
        let found = matches(&rows, "wkey");
        let (_, offsets) = found.first().expect("a match");
        let shown: String = offsets
            .iter()
            .map(|index| "Fixing a Broken Windows Key".chars().nth(*index).unwrap())
            .collect();
        assert_eq!(shown.to_lowercase(), "wkey");
    }

    /// A name is matched as the sidebar shows it, so a session the daemon never
    /// named is found by the name the reader can actually see.
    #[test]
    fn a_session_with_no_title_is_matched_by_the_name_the_sidebar_shows() {
        let mut row = row("");
        row.id = "abc123".to_string();
        row.cwd = "C:\\work\\prime-agent".to_string();
        let rows = [row];
        let found = matches(&rows, "prime");
        assert_eq!(found.len(), 1);
    }

    /// A short name is drawn as the daemon named it, punctuation and case alike.
    #[test]
    fn a_name_that_has_room_is_drawn_whole() {
        let rows = [row("Urban Roots")];
        let found = matches(&rows, "");
        let glyphs = glyphs(&found[0].0.name, &found[0].1);
        assert_eq!(drawn(&glyphs), "Urban Roots");
    }

    /// A long one is cut and says so, and the cut lands inside the room a row has
    /// beside its date.
    #[test]
    fn a_name_too_long_for_a_row_is_cut_and_marked() {
        let long = "a".repeat(200);
        let cut = elide(&long);
        assert_eq!(cut.chars().count(), 46);
        assert!(cut.ends_with('\u{2026}'));
    }

    /// The query is matched against the whole name, so a match past the row's
    /// cut still finds the session, and the row marks whatever of it it shows.
    #[test]
    fn a_match_past_the_cut_still_finds_the_session() {
        let mut long = row(&"xy ".repeat(40));
        long.name.push_str(" and the Garden");
        let rows = [long];
        let found = matches(&rows, "garden");
        assert_eq!(found.len(), 1);
        let (long, offsets) = (&found[0].0, &found[0].1);
        let name = elide(&sessions::display_name(long));
        let shown = glyphs(&name, offsets);
        assert_eq!(drawn(&shown), name);
        assert!(shown.iter().all(|glyph| !glyph.matched));
    }
}

