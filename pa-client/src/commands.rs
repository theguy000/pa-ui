//! The builtin slash commands, copied from the daemon's own table
//! (`pa-types::slash_commands`) so the window offers exactly the commands the
//! agent has: names, one-line descriptions and argument hints are byte-identical
//! to that table.

/// One builtin command, as the menu shows it.
pub struct Command {
    /// The word after the slash.
    pub name: &'static str,
    /// The one-line description, straight from the daemon's table.
    pub description: &'static str,
    /// The argument hint (`[level]`, `<path.jsonl>`), empty when it takes none.
    pub hint: &'static str,
    pub takes_argument: bool,
}

const TABLE: &[Command] = &[
    Command { name: "settings", description: "Open settings menu", hint: "", takes_argument: false },
    Command { name: "model", description: "Select model (opens selector UI; Tab filters by typed text)", hint: "", takes_argument: false },
    Command { name: "effort", description: "Select reasoning/thinking level (opens selector UI)", hint: "[level]", takes_argument: false },
    Command { name: "fast", description: "Toggle OpenAI Fast mode", hint: "", takes_argument: false },
    Command { name: "tier", description: "Show or set the service tier", hint: "[default|flex|priority|auto]", takes_argument: false },
    Command { name: "export", description: "Export session (HTML default, or specify path: .html/.jsonl)", hint: "[path]", takes_argument: true },
    Command { name: "import", description: "Import and resume a session from a JSONL file", hint: "<path.jsonl>", takes_argument: true },
    Command { name: "share", description: "Share session as a secret GitHub gist", hint: "", takes_argument: false },
    Command { name: "copy", description: "Copy last agent message to clipboard", hint: "", takes_argument: false },
    Command { name: "btw", description: "Ask a side question without adding it to the session; replies follow up, esc returns", hint: "<question>", takes_argument: true },
    Command { name: "name", description: "Set or show the session display name", hint: "[name]", takes_argument: true },
    Command { name: "session", description: "Show session info", hint: "", takes_argument: false },
    Command { name: "system-prompt", description: "Show the exact system prompt sent to the model", hint: "", takes_argument: false },
    Command { name: "logs", description: "Show where daemon and client logs are saved", hint: "", takes_argument: false },
    Command { name: "traces", description: "Preview, upload, or configure Prime Agent traces", hint: "[status|on|off|preview|upload|upload-current|upload-all|login]", takes_argument: false },
    Command { name: "telemetry", description: "Show or change usage telemetry (pseudonymous metrics, never prompts or code)", hint: "[status|on|off]", takes_argument: true },
    Command { name: "context", description: "Show token, cost, and context usage for agent and sub-agents", hint: "", takes_argument: false },
    Command { name: "changelog", description: "Show changelog entries", hint: "", takes_argument: false },
    Command { name: "update", description: "Update to the latest Rust build (uninstalls the TypeScript version; sessions preserved)", hint: "", takes_argument: false },
    Command { name: "nightly", description: "Switch Prime Agent updates to the nightly channel (unreleased builds, may be broken)", hint: "[on|off|status]", takes_argument: true },
    Command { name: "hotkeys", description: "Show all keyboard shortcuts", hint: "", takes_argument: false },
    Command { name: "fork", description: "Create a new fork from a previous user message", hint: "", takes_argument: false },
    Command { name: "clone", description: "Duplicate the current session at the current position", hint: "", takes_argument: false },
    Command { name: "tree", description: "Navigate session tree (switch branches)", hint: "", takes_argument: false },
    Command { name: "login", description: "Configure provider authentication", hint: "", takes_argument: false },
    Command { name: "logout", description: "Remove provider authentication", hint: "", takes_argument: false },
    Command { name: "mcp", description: "Open the MCP connections menu (Tab filters by typed text)", hint: "", takes_argument: false },
    Command { name: "plugins", description: "Browse and connect external services", hint: "[search]", takes_argument: true },
    Command { name: "new", description: "Start a new session, optionally named and/or with an initial prompt", hint: r#"[--name "session name" --] [prompt]"#, takes_argument: true },
    Command { name: "compact", description: "Compact the session context; optional instructions focus the summary", hint: "[instructions]", takes_argument: false },
    Command { name: "refine", description: "Refine continual harness prompt notes, skills, subagents, and memory", hint: "", takes_argument: false },
    Command { name: "goal", description: "Set or view a persistent goal; supports pause, resume, and clear", hint: "[objective]", takes_argument: true },
    Command { name: "autonomous", description: "Set or view autonomous mode with an optional budget", hint: "[status|off|on [--max-continuations <n>] [--max-turns <n>] [--max-tokens <n>] [--timeout-ms <n>] [--gate <command>]]", takes_argument: true },
    Command { name: "rlm-max-depth", description: "Set/view the per-chat persistent RLM max depth immediately; never interrupts or queues the running turn", hint: "[<int> [--global]]", takes_argument: true },
    Command { name: "heartbeat", description: "Set or view a persistent heartbeat; delivery defaults to steer, use --follow-up to queue; supports pause, resume, stop, and clear", hint: "[status|pause|resume|stop|[every <duration>] [--steer|--follow-up] <instruction>]", takes_argument: true },
    Command { name: "heartbeats", description: "View and manage all user and agent heartbeats", hint: "", takes_argument: false },
    Command { name: "factory", description: "Show or set the agent factory opt-in gate (off by default)", hint: "[on|off|status]", takes_argument: true },
    Command { name: "resume", description: "Open the agents view, or resume a session by id or path", hint: "[id|path]", takes_argument: true },
    Command { name: "reload", description: "Reload keybindings, skills, prompts, and themes", hint: "", takes_argument: false },
    Command { name: "speed", description: "Toggle footer readout of model output tok/sec (latest response and session average)", hint: "[on|off]", takes_argument: true },
    Command { name: "quit", description: "Quit Prime Agent", hint: "", takes_argument: false },
];

/// Every builtin command, in the daemon's order.
#[must_use]
pub fn builtin() -> &'static [Command] {
    TABLE
}

/// The commands whose name starts with `prefix` (given without the slash).
///
/// An empty prefix offers everything, which is what the menu shows the moment
/// the slash is typed.
#[must_use]
pub fn filter(prefix: &str) -> Vec<&'static Command> {
    let needle = prefix.trim_start_matches('/').to_lowercase();
    TABLE
        .iter()
        .filter(|command| command.name.starts_with(&needle))
        .collect()
}

/// The command exactly named `name`, if the daemon has one.
#[must_use]
pub fn find(name: &str) -> Option<&'static Command> {
    TABLE.iter().find(|command| command.name == name)
}

#[cfg(test)]
mod tests {
    use super::{builtin, filter, find};

    #[test]
    fn the_table_is_the_daemons_table() {
        // 41 builtins today; a wrong count means the copy drifted
        assert_eq!(builtin().len(), 41);
        assert_eq!(find("model").map(|c| c.name), Some("model"));
        assert!(find("nonsense").is_none());
    }

    #[test]
    fn an_empty_prefix_offers_everything() {
        assert_eq!(filter("").len(), builtin().len());
        assert_eq!(filter("/").len(), builtin().len());
    }

    #[test]
    fn prefixes_narrow_the_list_in_table_order() {
        let names: Vec<&str> = filter("/mo").iter().map(|c| c.name).collect();
        assert_eq!(names.first(), Some(&"model"));
        assert_eq!(filter("compact").len(), 1);
        // the slash is optional and case does not matter
        assert_eq!(filter("/MODEL").len(), filter("model").len());
    }
}
