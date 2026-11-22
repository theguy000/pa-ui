//! Slash command module: token extraction, Tab completion, color decoration ranges, and dispatch.
//!
//! Deep in-process module: callers hand it the composer string and caret byte offset
//! and receive the active `/token`, matching commands, completed string + caret,
//! byte ranges to color-coat as command tokens, or the dispatch action on submit.

use pa_client::commands::Command;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SlashAction {
    SendMessage,
    SendDaemonCommand,
    Model,
    Settings,
    NewChat,
    Rename(String),
    Export,
    Copy,
    Quit,
    NotWired(&'static str),
}

pub fn is_supported_slash_cmd(cmd: &Command) -> bool {
    cmd.session_side
        || matches!(
            cmd.name,
            "model" | "effort" | "settings" | "new" | "name" | "export" | "copy" | "quit"
        )
}

pub fn slash_dispatch(text: &str) -> SlashAction {
    let trimmed = text.trim();
    let Some(body) = trimmed.strip_prefix('/') else {
        return SlashAction::SendMessage;
    };
    let name = body.split_whitespace().next().unwrap_or("");
    let rest = body[name.len()..].trim().to_string();
    let Some(command) = pa_client::commands::find(name) else {
        return SlashAction::SendMessage;
    };
    if command.session_side {
        return SlashAction::SendDaemonCommand;
    }
    match name {
        "model" | "effort" => SlashAction::Model,
        "settings" => SlashAction::Settings,
        "new" => SlashAction::NewChat,
        "name" => SlashAction::Rename(rest),
        "export" => SlashAction::Export,
        "copy" => SlashAction::Copy,
        "quit" => SlashAction::Quit,
        _ => SlashAction::NotWired(command.name),
    }
}

/// Find the whitespace-delimited `/prefix` token surrounding `caret` (UTF-8 byte offset).
/// Returns `(byte_range, prefix_after_slash)` when the token starts with `/`.
pub fn slash_token_at(text: &str, caret: usize) -> Option<(Range<usize>, &str)> {
    let caret = caret.min(text.len());
    let start = text[..caret]
        .char_indices()
        .rev()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(0, |(idx, ch)| idx + ch.len_utf8());
    let end = text[caret..]
        .char_indices()
        .find(|(_, ch)| ch.is_whitespace())
        .map_or(text.len(), |(idx, _)| caret + idx);
    let token = text.get(start..end)?;
    let prefix = token.strip_prefix('/')?;
    Some((start..end, prefix))
}

/// Supported slash commands matching `prefix` (with or without leading `/`).
pub fn matching_slash_commands(prefix: &str) -> Vec<&'static Command> {
    pa_client::commands::filter(prefix)
        .into_iter()
        .filter(|cmd| is_supported_slash_cmd(cmd))
        .collect()
}

/// Replace the `/prefix` token at `caret` with `/{cmd_name} ` and return `(new_text, new_caret)`.
pub fn complete_slash_token(text: &str, caret: usize, cmd_name: &str) -> Option<(String, usize)> {
    let (range, _) = slash_token_at(text, caret)?;
    let after = &text[range.end..];
    let needs_space = !after.starts_with(char::is_whitespace);
    let replacement = if needs_space {
        format!("/{cmd_name} ")
    } else {
        format!("/{cmd_name}")
    };
    let mut out = String::with_capacity(text.len() - range.len() + replacement.len());
    out.push_str(&text[..range.start]);
    out.push_str(&replacement);
    let new_caret = if needs_space {
        range.start + replacement.len()
    } else {
        // Step past the existing single whitespace after the token.
        let ws_len = after.chars().next().map_or(0, char::len_utf8);
        range.start + replacement.len() + ws_len
    };
    out.push_str(after);
    Some((out, new_caret))
}

/// UTF-8 byte ranges of all whitespace-delimited `/command` words in `text` that should be
/// color-coated in the composer (known commands as well as an active `/prefix` that matches
/// at least one supported slash command).
pub fn slash_command_ranges(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut word_start: Option<usize> = None;

    for (idx, ch) in text.char_indices() {
        if ch.is_whitespace() {
            if let Some(start) = word_start.take() {
                if is_colorable_slash_word(&text[start..idx]) {
                    ranges.push(start..idx);
                }
            }
        } else if word_start.is_none() {
            word_start = Some(idx);
        }
    }
    if let Some(start) = word_start {
        if is_colorable_slash_word(&text[start..]) {
            ranges.push(start..text.len());
        }
    }
    ranges
}

fn is_colorable_slash_word(word: &str) -> bool {
    let Some(name) = word.strip_prefix('/') else {
        return false;
    };
    if name.is_empty() {
        return true;
    }
    pa_client::commands::find(name).is_some()
        || name.starts_with("skill:")
        || name.starts_with("template:")
        || !matching_slash_commands(name).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slash_dispatch_routes_window_and_daemon_commands_without_leaking() {
        assert_eq!(slash_dispatch("/compact"), SlashAction::SendDaemonCommand);
        assert_eq!(slash_dispatch("/refine"), SlashAction::SendDaemonCommand);
        assert_eq!(slash_dispatch("/model"), SlashAction::Model);
        assert_eq!(
            slash_dispatch("/name sprint 2"),
            SlashAction::Rename("sprint 2".into())
        );
        assert_eq!(
            slash_dispatch("/system-prompt"),
            SlashAction::NotWired("system-prompt")
        );
        assert_eq!(slash_dispatch("hello"), SlashAction::SendMessage);
    }

    #[test]
    fn slash_token_at_and_tab_completion_replace_token_and_advance_caret() {
        assert_eq!(slash_token_at("/co", 3), Some((0..3, "co")));
        assert_eq!(slash_token_at("please /ref now", 10), Some((7..11, "ref")));
        assert_eq!(slash_token_at("plain text", 5), None);

        let (done, caret) = complete_slash_token("/co", 3, "compact").unwrap();
        assert_eq!(done, "/compact ");
        assert_eq!(caret, "/compact ".len());

        let (mid, mid_caret) = complete_slash_token("run /re please", 7, "refine").unwrap();
        assert_eq!(mid, "run /refine please");
        assert_eq!(mid_caret, "run /refine ".len());
    }

    #[test]
    fn slash_command_ranges_identifies_command_words_for_color_coating() {
        let text = "/compact and /model but not /unknownxyz or prose";
        let ranges = slash_command_ranges(text);
        let words: Vec<&str> = ranges.iter().map(|r| &text[r.clone()]).collect();
        assert_eq!(words, vec!["/compact", "/model"]);
    }
}
