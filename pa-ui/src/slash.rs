//! Slash commands: which token the caret is in, what the menu offers, and what accepting one does.

use crate::*;

/// Whether a bare word, typed after a slash, names a command this session has.
///
/// Builtins come from the copied table; the session's own entries may carry a
/// `skill:` prefix while the typed word does not.
pub(crate) fn is_command_word(extra: &[(String, String, String)], name: &str) -> bool {
    if pa_client::commands::find(name).is_some() {
        return true;
    }
    extra.iter().any(|(known, _, _)| {
        let bare = known.rsplit(':').next().unwrap_or(known.as_str());
        known == name || bare == name
    })
}


/// The menu's rows: the builtin table first, then whatever this session adds on
/// top of it, both narrowed by the same prefix.
pub(crate) fn slash_rows(prefix: &str, extra: &[(String, String, String)]) -> Vec<SlashItem> {
    let needle = prefix.trim_start_matches('/').to_lowercase();
    let mut rows: Vec<SlashItem> = pa_client::commands::filter(prefix)
        .iter()
        .map(|command| SlashItem {
            name: command.name.into(),
            hint: command.hint.into(),
            description: command.description.into(),
        })
        .collect();
    for (name, description, hint) in extra {
        let bare = name.strip_prefix("skill:").unwrap_or(name);
        if !name.to_lowercase().starts_with(&needle) && !bare.to_lowercase().starts_with(&needle) {
            continue;
        }
        rows.push(SlashItem {
            name: name.as_str().into(),
            hint: hint.as_str().into(),
            description: description.as_str().into(),
        });
    }
    rows
}


/// The `/command` token under the caret, as byte range `[start, end)`.
///
/// The menu belongs to whichever word the caret sits in: the first or the fifth;
/// so a command can be started between words, not only at the start of a message.
pub(crate) fn slash_token_at(text: &str, caret: usize) -> (usize, usize) {
    let caret = caret.min(text.len());
    let start = text[..caret]
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let end = text[caret..]
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(text.len(), |(index, _)| caret + index);
    (start, end)
}



/// What the window should do with a message that starts with a slash.
///
/// The daemon's table splits commands in two: the agent runs `compact`,
/// `refine`, `goal` and `autonomous` itself, so those are sent as written. Every
/// other builtin is a *window* action: sending `/system-prompt` down the pipe
/// gave the agent a chat message that read like a command and did nothing.
pub(crate) fn slash_dispatch(text: &str) -> SlashAction {
    let trimmed = text.trim();
    let Some(body) = trimmed.strip_prefix('/') else {
        return SlashAction::Send;
    };
    let name = body.split_whitespace().next().unwrap_or("");
    let rest = body[name.len()..].trim().to_string();
    let Some(command) = pa_client::commands::find(name) else {
        // not a builtin: skills and prompt templates are the agent's to run
        return SlashAction::Send;
    };
    if command.session_side {
        return SlashAction::Send;
    }
    match name {
        "model" => SlashAction::Model,
        "effort" | "thinking" => SlashAction::EffortHint,
        "new" | "clear" => SlashAction::NewChat,
        "quit" => SlashAction::Quit,
        "name" => SlashAction::Rename(rest),
        _ => SlashAction::NotWired(command.name),
    }
}


/// The decision above, as a value so the send path stays readable.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SlashAction {
    Send,
    Model,
    EffortHint,
    NewChat,
    Quit,
    Rename(String),
    NotWired(&'static str),
}



#[cfg(test)]
mod slash_menu_tests {
    use super::slash_token_at;

    #[test]
    fn the_menu_belongs_to_the_word_after_the_slash() {
        assert_eq!(slash_token_at("/mo", 3), (0, 3));
        let text = "please /ref now";
        assert_eq!(slash_token_at(text, 10), (7, 11));
        assert_eq!(slash_token_at("x /y", 2), (2, 4));
    }

    #[test]
    fn the_agent_runs_its_own_commands() {
        use super::{slash_dispatch, SlashAction};
        // the four session commands go down the pipe as written
        assert_eq!(slash_dispatch("/compact"), SlashAction::Send);
        assert_eq!(slash_dispatch("/refine tighten the notes"), SlashAction::Send);
        // a skill is not a builtin: it is the agent's to run too
        assert_eq!(slash_dispatch("/skill:review the diff"), SlashAction::Send);
        assert_eq!(slash_dispatch("just prose"), SlashAction::Send);
    }

    #[test]
    fn window_commands_never_leave_as_words() {
        use super::{slash_dispatch, SlashAction};
        assert_eq!(slash_dispatch("/model"), SlashAction::Model);
        assert_eq!(slash_dispatch("/new"), SlashAction::NewChat);
        assert_eq!(slash_dispatch("/quit"), SlashAction::Quit);
        assert_eq!(
            slash_dispatch("/name plan the week"),
            SlashAction::Rename("plan the week".to_string())
        );
        // the one that was silently sent to the agent and did nothing
        assert_eq!(
            slash_dispatch("/system-prompt"),
            SlashAction::NotWired("system-prompt")
        );
    }
}


#[cfg(test)]
mod slash_rows_tests {
    use super::slash_rows;

    #[test]
    fn the_table_comes_first_and_the_sessions_own_follow() {
        let extra = vec![
            (
                "skill:review".to_string(),
                "review the diff".to_string(),
                "your request".to_string(),
            ),
            (
                "template:standup".to_string(),
                "daily standup".to_string(),
                "[day]".to_string(),
            ),
        ];
        let all = slash_rows("", &extra);
        assert!(all.len() == 43, "41 builtins + 2 session commands, got {}", all.len());
        assert_eq!(all.last().map(|row| row.name.to_string()), Some("template:standup".to_string()));
    }

    #[test]
    fn a_prefix_narrows_both_sources() {
        let extra = vec![(
            "skill:compact-review".to_string(),
            "x".to_string(),
            "your request".to_string(),
        )];
        let rows = slash_rows("compact", &extra);
        let names: Vec<String> = rows.iter().map(|row| row.name.to_string()).collect();
        assert!(names.contains(&"compact".to_string()));
        // the skill matches on its bare word too, so a typed `compact` finds it
        assert!(names.contains(&"skill:compact-review".to_string()));
    }

    #[test]
    fn compact_and_refine_are_in_the_menu() {
        let names: Vec<String> = slash_rows("", &[])
            .iter()
            .map(|row| row.name.to_string())
            .collect();
        assert!(names.contains(&"compact".to_string()));
        assert!(names.contains(&"refine".to_string()));
    }
}
