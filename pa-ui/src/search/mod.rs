//! The search popup's matching: which sessions a typed query keeps, in which
//! order, and which letters of each name the row shows as the match.
//!
//! Names only. A query is matched against a session's name letter by letter, in
//! order, gaps allowed, so `msc` finds a session called "Mxs Mints cat". A run of
//! letters found side by side scores higher than the same letters scattered, so
//! the tightest match sorts first, and nothing is dropped for scoring low: a
//! reader who typed the letters meant them.
//!
//! A row is drawn twice: the query matches the session's whole name, so a match
//! past the tail of what the row has room for is still found, and the name is
//! then cut to what the row shows. The offsets come back as character positions
//! into the title the row was drawn from, and the row marks the ones it shows.

use crate::*;

pub(crate) mod keys;

/// Every session a query keeps, best match first.
///
/// An empty query keeps every session in the order given, which is the order the
/// daemon lists them in: the popup opens on the whole column rather than on a
/// blank panel. The match offsets come back as character indices into the same
/// name the row draws, `elide` of the whole title, so a row can mark the letters
/// it was found by.
pub(crate) fn matches<'a>(
    rows: &'a [SessionRow],
    query: &str,
) -> Vec<(&'a SessionRow, Vec<usize>)> {
    let needle: Vec<char> = query.trim().chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return rows.iter().map(|row| (row, Vec::new())).collect();
    }
    let drawn = drawn_names(rows);
    let mut kept: Vec<(&SessionRow, Vec<usize>, u32)> = rows
        .iter()
        .zip(&drawn)
        .filter_map(|(row, drawn)| {
            let (span, offsets) = narrowest_span(drawn, &needle)?;
            let begins_word = is_word_start(drawn, offsets[0]);
            let tie_break = (u8::from(!begins_word), drawn.chars().count() as u32);
            Some((row, offsets, sort_key(span, tie_break)))
        })
        .collect();
    // A stable sort keeps the daemon's order between sessions that sort the same,
    // so a session does not jump when two of them match equally well.
    kept.sort_by_key(|(_, _, key)| *key);
    kept.into_iter()
        .map(|(row, offsets, _)| (row, offsets))
        .collect()
}

/// One name per session, as the popup's row draws it before the cut: the whole
/// title, or the name the sidebar gives a session that has none. The query is
/// matched against this string and the row draws this string, so the offsets can
/// never point at a character the row does not show.
fn drawn_names(rows: &[SessionRow]) -> Vec<String> {
    rows.iter()
        .map(|row| {
            let name = row.name.trim();
            if name.is_empty() || name == "(no messages)" {
                sessions::display_name(row).to_string()
            } else {
                name.to_string()
            }
        })
        .collect()
}

/// How many characters of a name a row has room for beside its date, counting
/// the ellipsis a cut name ends in.
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
///
/// The name must be the one the offsets were found in. The row passes what it
/// draws, so a mark past the cut falls off the end instead of landing on the
/// wrong letter.
pub(crate) fn glyphs(name: &str, offsets: &[usize]) -> Vec<SearchGlyph> {
    name.chars()
        .enumerate()
        .map(|(index, character)| SearchGlyph {
            text: character.to_string().into(),
            matched: offsets.contains(&index),
        })
        .collect()
}

/// Whether a match that starts at `at` begins a word: the string's first
/// character, or a character just after a space.
fn is_word_start(hay: &str, at: usize) -> bool {
    at == 0 || hay.chars().nth(at - 1).is_some_and(char::is_whitespace)
}

/// Sorting the kept matches: the narrowest span first, then a match that begins
/// a word, then the shorter name.
///
/// The span comes in steps of four and the word bonus is worth three, so the
/// bonus beats a span one character wider but never a span two wider, and the
/// name length only orders matches that are already equal on both.
fn sort_key(span: usize, tie_break: (u8, u32)) -> u32 {
    let (not_word_start, name_length) = tie_break;
    (span as u32 * 4 - 3 * (1 - not_word_start as u32)) * 1_000 + name_length.min(999)
}

/// The narrowest span that carries the query's letters in order, with the
/// offsets the letters were found at.
///
/// Each start is tried where the query's first letter appears, and each run
/// takes its letters at the earliest place they fall after that. Trying every
/// start rather than only the first is what lets `msc` find "Mxs Mints cat": the
/// m of "Mxs" alone cannot reach an s, and the m of "Mints" can.
fn narrowest_span(hay: &str, needle: &[char]) -> Option<(usize, Vec<usize>)> {
    let lower: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    let mut best: Option<(usize, Vec<usize>)> = None;
    for from in (0..lower.len()).filter(|index| lower[*index] == needle[0]) {
        let mut offsets = Vec::with_capacity(needle.len());
        // the run has already spent its first letter on the start itself
        let mut at = from;
        offsets.push(from);
        at += 1;
        let mut complete = true;
        for want in &needle[1..] {
            match lower[at..].iter().position(|c| c == want) {
                Some(found) => {
                    offsets.push(at + found);
                    at += found + 1;
                }
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if !complete {
            // a start past this one cannot reach the letter that failed, so no
            // later start can complete either
            break;
        }
        let width = offsets[offsets.len() - 1] - from + 1;
        if best.as_ref().is_none_or(|(best, _)| width < *best) {
            best = Some((width, offsets));
        }
    }
    best
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

    /// A start where the query's first letter alone is enough to fail is not the
    /// only start tried: nothing after the m of "Mxs" is an s, and the m of
    /// "Mints" beside it has one.
    #[test]
    fn a_start_that_cannot_finish_does_not_hide_a_later_one() {
        let rows = [row("Mxs Mints cat")];
        assert_eq!(names(&rows, "msc"), ["Mxs Mints cat"]);
    }

    /// The narrowest span wins even where a different start would also match:
    /// the first m of "Mix Msc Code" runs to the s of "Msc", while the m of
    /// "Msc" finds its s and c side by side.
    #[test]
    fn the_every_start_tried_is_the_narrowest_one() {
        let rows = [row("Mix Msc Code")];
        let found = matches(&rows, "msc");
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].1,
            vec![4, 5, 6],
            "the m of Msc is the narrow start"
        );
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

    /// Name length is a tie-break, not part of the score: a longer name with a
    /// snug match still sorts above a shorter name whose letters are scattered.
    #[test]
    fn a_narrow_span_beats_a_short_name() {
        // The short name scatters the query: its m, s and c sit eighteen
        // characters apart. The long name finds them side by side in "Msc".
        let rows = [
            row("Music Scene, a closer look"),
            row("Ace Mica Spore Cat and a much longer tail here, with an Msc inside"),
        ];
        assert_eq!(
            names(&rows, "msc"),
            [
                "Ace Mica Spore Cat and a much longer tail here, with an Msc inside",
                "Music Scene, a closer look"
            ],
            "the snug span wins even though its name is longer"
        );
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
        let name = elide(&rows[0].name);
        let glyphs = glyphs(&name, &found[0].1);
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

    /// The row draws the name it matched, so a run of spaces in a title cannot
    /// move the marks: the glyphs are looked up in the drawn string, not in a
    /// shortened copy of it.
    #[test]
    fn offsets_point_at_the_letters_the_row_draws() {
        let rows = [row("Plan   the   launch")];
        let found = matches(&rows, "launch");
        assert_eq!(found.len(), 1);
        let name = elide(&rows[0].name);
        let shown = glyphs(&name, &found[0].1);
        let marked: String = shown
            .iter()
            .filter(|glyph| glyph.matched)
            .map(|glyph| glyph.text.to_string())
            .collect();
        assert_eq!(marked, "launch");
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
        let name = elide(&long.name);
        let shown = glyphs(&name, offsets);
        assert_eq!(drawn(&shown), name);
        assert!(shown.iter().all(|glyph| !glyph.matched));
    }
}
