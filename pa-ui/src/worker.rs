//! The background worker: one thread owns the pipe, the window hands it jobs and reads back messages.

use crate::*;

pub(crate) enum Job {
    Rename { session: String, name: String },
    /// rename a row no worker holds, by its file: `rename` would start a worker
    RenameSaved { session_path: String, name: String },
    /// stop a live session, the TUI's ctrl+x: it becomes a done row
    MarkDone { session: String },
    /// delete a done row's file; the daemon refuses one a worker holds
    DeleteSaved { session_path: String },
    /// ask the daemon to export a session and the GitHub CLI to share it, the
    /// TUI's `/share`
    Share { session: String },
    /// check the unauthenticated GitHub quota the share viewer will spend
    CheckViewerQuota,
    Attach { session: String, generation: u64 },
    Tail { session: String, fresh: bool },
    /// the page before the one already loaded, walked from a row id
    LoadOlder {
        session: String,
        before: String,
        limit: usize,
    },
    Prompt {
        session: String,
        message: String,
        images: Vec<serde_json::Value>,
    },
    Steer {
        session: String,
        message: String,
        images: Vec<serde_json::Value>,
    },
    Abort { session: String },
    Queue { session: String },
    ModelCatalog { session: String },
    QueueAct {
        session: String,
        lane: String,
        slot: usize,
        text: String,
        action: String,
    },
    /// An edit of a parked message: replaces it where it stands, so the queue
    /// keeps its order instead of sending the message to the back.
    QueueEdit {
        session: String,
        lane: String,
        slot: usize,
        expected: String,
        text: String,
        images: Vec<serde_json::Value>,
    },
    Stats { session: String },
    /// one live activity read for the selected Session: roster descendants,
    /// scoped heartbeats, and running shell commands
    ActivityRead { session: String, session_id: String },
    /// the session's own slash commands (templates and skills)
    Commands { session: String },
    SetModel { session: String, provider: String, model: String },
    /// the running model and the level it thinks at, straight from `get_state`
    SessionState { session: String },
    /// a level picked off a model's ladder: switching models first when the row
    /// was not the running one
    SetLevel { session: String, provider: String, model: String, level: String, switch: bool },
    /// start a fresh chat beside the running one (the TUI's `/new`)
    NewChat,
    /// ask for a full `list` now: a terminal Turn must not leave a stale
    /// roster row holding the Status strip up until the fallback poll.
    RefreshSessions,
}


pub(crate) enum Msg {
    Sessions(Result<Vec<SessionRow>, String>),
    /// the supervisor re-keyed the attached session: follow the new active id
    SessionBinding { previous: String, active: String },
    Transcript { session: String, rows: Vec<crate::transcript::Row>, skipped: usize, messages: Vec<serde_json::Value> },
    Tail {
        session: String,
        rows: Vec<crate::transcript::Row>,
        skipped: usize,
        messages: Vec<serde_json::Value>,
        last_entry: String,
        /// id of the oldest row in this page: where a walk backwards starts
        first_entry: String,
        /// whether the store has rows older than this page
        more: bool,
        /// the turn just closed: reconcile for real instead of appending
        fresh: bool,
    },
    /// an older page, to be put above what is already shown
    Older {
        session: String,
        rows: Vec<crate::transcript::Row>,
        messages: Vec<serde_json::Value>,
        first_entry: String,
        more: bool,
    },
    /// one `session_event` from the attached session, forwarded verbatim
    Event { session: String, event: serde_json::Value },
    PromptResult { session: Option<String>, error: Option<String> },
    /// the daemon refused a prompt: hand the text back to the composer
    PromptRejected { session: Option<String>, text: String },
    /// the pipe came back: the old attachment is gone, re-attach
    Reconnected,
    Queue { steering: Vec<String>, follow_up: Vec<String> },
    /// the parked row went away before its edit landed: the text goes out as a
    /// new prompt rather than being lost
    Prompt { session: String, message: String, images: Vec<serde_json::Value> },
    /// the session's own slash commands, on top of the builtin table
    ExtraCommands(Vec<(String, String, String)>),
    /// the daemon's model catalog, as the menu builds it
    ModelCatalog(Vec<pa_client::ModelEntry>),
    QueueEdit { text: String },
    /// a `create` came back: the id of the brand new session
    NewChat(String),
    Stats { session: String, stats: pa_client::SessionStats },
    /// one activity refresh; a failed part is `None` so the window keeps its
    /// last good number
    ActivityCounts {
        session: String,
        subagents: Option<usize>,
        heartbeats: Option<usize>,
        shells: Option<usize>,
    },
    State { state: pa_client::SessionState },
    Activity { label: String },
    /// a share finished: the viewer URL the card shows, copies and opens, or
    /// the reason it could not be made
    Shared(Result<crate::share::Shared, String>),
    /// the public GitHub API quota, before the card opens the viewer
    ViewerQuota(Result<crate::share::ViewerQuota, String>),
    Error { session: Option<String>, message: String },
    /// The daemon closed this Session. The transcript stays readable, but the
    /// turn is over and the close reason is an Error row in that Sheet.
    SessionClosed { session: String, reason: String },
}

/// The worker's outbox. Every reply carries the generation it was produced
/// under, so the window can tell an answer it is still waiting for from an
/// answer to a question it has already given up on.
pub(crate) struct Out {
    tx: Sender<(u64, Msg)>,
    generation: u64,
}

impl Out {
    fn send(&self, message: Msg) {
        let _ = self.tx.send((self.generation, message));
    }
}

impl Msg {
    /// Whether this reply is about one chat's contents. Such a reply is only an
    /// answer while the window is still attached to the chat that asked; what
    /// comes back after the reader has left is noise, and a session id cannot
    /// tell it apart from a reply sent before a re-attach of the same chat.
    #[must_use]
    pub(crate) fn is_session_scoped(&self) -> bool {
        matches!(
            self,
            Msg::Transcript { .. }
                | Msg::Tail { .. }
                | Msg::Older { .. }
                | Msg::Event { .. }
                | Msg::Stats { .. }
                | Msg::ActivityCounts { .. }
                | Msg::State { .. }
                | Msg::ModelCatalog(_)
                | Msg::ExtraCommands(_)
                | Msg::Queue { .. }
        )
    }
}

type RosterRows = Vec<(String, SessionRow)>;

/// One roster entry as the sidebar's flat session row. The roster summary is
/// the same shape `list` sends; its status carries the worker presence, which
/// `SessionRow::from_value` reads from `workerState`.
fn roster_session_row(entry: &serde_json::Value) -> Option<(String, SessionRow)> {
    let agent_id = entry
        .get("agentId")
        .and_then(serde_json::Value::as_str)
        .filter(|id| !id.is_empty())?
        .to_string();
    let mut summary = entry.get("summary")?.clone();
    let status = entry
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if summary.get("workerState").and_then(serde_json::Value::as_str).is_none()
        && matches!(status, "running" | "idle")
    {
        if let Some(object) = summary.as_object_mut() {
            object.insert("workerState".to_string(), serde_json::json!("ready"));
        }
    }
    let row = SessionRow::from_value(&summary);
    (!row.id.is_empty() || !row.session_id.is_empty()).then_some((agent_id, row))
}

fn roster_rows(entries: &[serde_json::Value]) -> RosterRows {
    entries.iter().filter_map(roster_session_row).collect()
}

/// Apply one `roster_update` push: a resync replaces the store, `changed`
/// upserts by agent id, `removed` deletes by agent id.
fn apply_roster_update(roster: &mut RosterRows, frame: &serde_json::Value) {
    if frame
        .get("resync")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        roster.clear();
    }
    if let Some(changed) = frame.get("changed").and_then(serde_json::Value::as_array) {
        for entry in changed {
            let Some((id, row)) = roster_session_row(entry) else {
                continue;
            };
            if let Some(position) = roster.iter().position(|(known, _)| known == &id) {
                roster[position] = (id, row);
            } else {
                roster.push((id, row));
            }
        }
    }
    if let Some(removed) = frame.get("removed").and_then(serde_json::Value::as_array) {
        for id in removed.iter().filter_map(serde_json::Value::as_str) {
            roster.retain(|(known, _)| known != id);
        }
    }
}

fn same_session(left: &SessionRow, right: &SessionRow) -> bool {
    (!right.session_id.is_empty() && left.session_id == right.session_id)
        || (!right.id.is_empty() && left.id == right.id)
}

/// Apply one roster push to the raw entries used for running-subagent counts.
fn apply_roster_entries(entries: &mut Vec<serde_json::Value>, frame: &serde_json::Value) {
    if frame
        .get("resync")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        entries.clear();
    }
    if let Some(changed) = frame.get("changed").and_then(serde_json::Value::as_array) {
        for entry in changed {
            let Some(id) = entry.get("agentId").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if let Some(position) = entries.iter().position(|known| {
                known.get("agentId").and_then(serde_json::Value::as_str) == Some(id)
            }) {
                entries[position] = entry.clone();
            } else {
                entries.push(entry.clone());
            }
        }
    }
    if let Some(removed) = frame.get("removed").and_then(serde_json::Value::as_array) {
        entries.retain(|entry| {
            let id = entry.get("agentId").and_then(serde_json::Value::as_str);
            !removed.iter().any(|gone| gone.as_str() == id)
        });
    }
}

fn activity_row<'a>(
    attached: &str,
    roster: &'a RosterRows,
    saved: &'a [SessionRow],
) -> Option<&'a SessionRow> {
    roster
        .iter()
        .find(|(_, row)| row.id == attached)
        .map(|(_, row)| row)
        .or_else(|| saved.iter().find(|row| row.id == attached))
}

fn activity_subagents(
    entries: &[serde_json::Value],
    attached: &str,
    roster: &RosterRows,
    saved: &[SessionRow],
) -> Option<usize> {
    let row = activity_row(attached, roster, saved)?;
    Some(pa_client::running_subagents(
        entries,
        attached,
        &row.session_id,
    ))
}

/// Merge the saved catalog with the live roster. Roster rows win for a shared
/// session; saved-only rows keep their archived/done state.
fn merged_sessions(saved: &[SessionRow], roster: &RosterRows) -> Vec<SessionRow> {
    let mut rows = saved.to_vec();
    for (_, live) in roster {
        let found = rows.iter_mut().find(|row| same_session(row, live));
        match found {
            Some(row) => *row = live.clone(),
            None => rows.push(live.clone()),
        }
    }
    rows
}

/// A full `list` is a fresher snapshot than a cached roster row. Drop the
/// cached rows the snapshot covers so the fresh state wins; keep live-only
/// rows (a Session the catalog has not reported yet) for the next merge.
fn reconcile_roster_with_saved(roster: &mut RosterRows, saved: &[SessionRow]) {
    roster.retain(|(_, live)| !saved.iter().any(|row| same_session(row, live)));
}

fn apply_list_snapshot(
    roster: &mut RosterRows,
    saved: &mut Vec<SessionRow>,
    rows: Vec<SessionRow>,
) {
    *saved = rows;
    reconcile_roster_with_saved(roster, saved);
}

/// A stable number for a page cursor, so the poll can tell "same page" quickly.
pub(crate) fn page_cursor(last_entry: &str) -> u64 {
    last_entry
        .bytes()
        .fold(0u64, |hash, byte| hash.wrapping_mul(31).wrapping_add(u64::from(byte)))
}


/// The one wording for a model change that does not land. Both the model job
/// and the effort job's prerequisite switch report the same operation.
fn model_change_error(provider: &str, model: &str, error: &impl std::fmt::Display) -> String {
    format!("Could not change the model to {provider}/{model}: {error}")
}

/// A worker that panics used to vanish without a word: no events, no replies,
/// no poll - a window that keeps painting a transcript nobody is updating. The
/// work happens behind a catch so a panic in one frame is loud.
pub(crate) fn worker(jobs: Receiver<Job>, out: Sender<(u64, Msg)>, client_id: String) {
    let reporter = out.clone();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        worker_inner(jobs, out, client_id);
    }));
    if outcome.is_err() {
        eprintln!("[worker] panicked - the window is now static");
        let _ = reporter.send((0, Msg::Error { session: None, message: "engine stopped \u{2014} reopen the window".into() }));
    }
}


pub(crate) fn worker_inner(jobs: Receiver<Job>, tx: Sender<(u64, Msg)>, client_id: String) {
    let mut out = Out {
        tx,
        generation: 0,
    };
    let mut client: Option<Client> = None;

    // Reads go over a connection that never attaches: once a connection is
    // attached to a session the daemon serves its store view, which lags the
    // live agent by whole turns. A bare connection always answers with the
    // current messages (verified: attached 106 frozen while a fresh one read
    // 108 and kept moving).

    let mut last_list = SystemTime::now() - Duration::from_secs(60);
    let mut attached: Option<String> = None;
    // prompts awaiting a daemon verdict: a fire-and-forget send would otherwise
    // swallow a rejection and the message would vanish with no trace
    let mut pending_prompts: Vec<(String, String)> = Vec::new();
    let mut pending_renames: Vec<(String, String)> = Vec::new();
    let mut roster: RosterRows = Vec::new();
    let mut roster_entries: Vec<serde_json::Value> = Vec::new();
    let mut saved_rows: Vec<SessionRow> = Vec::new();
    let mut pending_roster: Option<String> = None;
    let mut roster_subscribed = false;
    let mut roster_live = false;
    let mut roster_failed = false;
    let mut resume: Option<(String, u64)> = None;
    // the outage line is already on the status strip for this stretch of failure
    let mut offline = false;
    let window_rows: usize = std::env::var("PA_ROWS").ok().and_then(|v| v.parse().ok()).unwrap_or(WINDOW_ROWS);

    loop {
        if client.is_none() {
            match Client::connect() {
                Ok(connected) => {
                    client = Some(connected);
                    // a new connection has no attachments: the UI re-attaches
                    // with the resume cursor so the daemon replays the gap
                    attached = None;
                    resume = None;
                    offline = false;
                    roster.clear();
                    roster_entries.clear();
                    pending_roster = None;
                    roster_subscribed = false;
                    roster_live = false;
                    roster_failed = false;
                    out.send(Msg::Reconnected);
                }
                Err(error) => {
                    // Reported once per outage: the retry is every two seconds,
                    // and the same line repeated is not news. The loop then runs
                    // on to the jobs below rather than starting over, so a
                    // prompt typed while the daemon is away is still answered
                    // and the list keeps its rows instead of freezing.
                    if !offline {
                        out.send(Msg::Error { session: None, message: error.to_string() });
                        offline = true;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            }
        }
        let Some(active) = client.as_ref() else {
            // No connection: still let the clock run. `continue` here spun the
            // loop with nothing to do, so `last_list` never aged and no list was
            // ever asked for once a connection came back.
            std::thread::sleep(Duration::from_millis(150));
            continue;
        };
        // the daemon may have restarted under us; without this the app keeps
        // reading a dead pipe and the transcript freezes until it is restarted
        if !active.alive() {
            client = None;
            attached = None;
            offline = false;
            out.send(Msg::Error { session: None, message: "Daemon connection lost; reconnecting\u{2026}".into() });
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }

        if !roster_subscribed && !roster_failed && pending_roster.is_none() {
            match active.send("roster_subscribe", serde_json::json!({})) {
                Ok(id) => pending_roster = Some(id),
                Err(_) => roster_failed = true,
            }
        }

        while let Ok(job) = jobs.try_recv() {
            match job {
                Job::RefreshSessions => {
                    // A list scans the whole session directory. Coalesce a
                    // duplicate request that arrives while the last snapshot is
                    // still fresh.
                    if last_list.elapsed().unwrap_or_default() >= Duration::from_secs(1) {
                        last_list = SystemTime::now() - Duration::from_secs(60);
                    }
                }
                Job::Rename { session, name } => {
                    match active.send(
                        "rename",
                        serde_json::json!({"activeSessionId": session, "name": name}),
                    ) {
                        Ok(id) => pending_renames.push((id, session)),
                        Err(error) => out.send(Msg::Error {
                            session: Some(session),
                            message: error.to_string(),
                        }),
                    }
                    last_list = SystemTime::now() - Duration::from_secs(60);
                }
                Job::RenameSaved { session_path, name } => {
                    match pa_client::rename_saved_session(active, &session_path, &name) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error { session: None, message: error.to_string() }),
                    }
                }
                Job::MarkDone { session } => {
                    // the TUI's ctrl+x: the worker archives the session on its
                    // way out, and the row moves to done on the next poll
                    match pa_client::mark_done(active, &session) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error {
                            session: Some(session),
                            message: error.to_string(),
                        }),
                    }
                }
                Job::DeleteSaved { session_path } => {
                    match pa_client::delete_saved_session(active, &session_path) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error { session: None, message: error.to_string() }),
                    }
                }
                Job::Share { session } => {
                    out.send(Msg::Shared(crate::share::share_as_gist(active, &session)));
                }
                Job::CheckViewerQuota => {
                    out.send(Msg::ViewerQuota(crate::share::viewer_quota()));
                }
                Job::Attach {
                    session,
                    generation,
                } => {
                    // Every reply after this carries the window's generation,
                    // so an answer to a question the window no longer has can
                    // be dropped before it reaches any state.
                    out.generation = generation;
                    if attached.as_deref() != Some(session.as_str()) {
                        resume = None;
                    }
                    match attach_as(active, &session, Some(&client_id), resume.clone()) {
                        Ok(fallback) => {
                            attached = Some(session.clone());
                            let messages = pa_client::get_page(active, &session, None, WINDOW_ROWS)
                                .map(|page| page.messages)
                                .unwrap_or(fallback);
                            let (rows, skipped) = crate::transcript::build(&messages, window_rows);
                            out.send(Msg::Transcript { session, rows, skipped, messages });
                            last_list = SystemTime::now() - Duration::from_secs(60);
                        }
                        Err(error) => {
                            out.send(Msg::Error {
                                session: Some(session),
                                message: error.to_string(),
                            });
                        }
                    }
                }
                Job::SetModel {
                    session,
                    provider,
                    model,
                } => {
                    let outcome = set_model(active, &session, &provider, &model);
                    if crate::env_cfg::cfg().debug {
                        eprintln!("[model] {provider}/{model} -> {outcome:?}");
                    }
                    // a provider that is not signed in fails here: say so rather
                    // than leaving the chip naming a model the session is not on
                    if let Err(error) = outcome {
                        out.send(Msg::Error {
                            session: Some(session.clone()),
                            message: model_change_error(&provider, &model, &error),
                        });
                    }
                }
                Job::NewChat => match create_session(active) {
                    Ok(id) => {
                        if crate::env_cfg::cfg().debug {
                            eprintln!("[new] created {id}");
                        }
                        out.send(Msg::NewChat(id));
                    }
                    Err(error) => {
                        out.send(Msg::Error {
                            session: None,
                            message: format!("Could not start a new Session: {error}"),
                        });
                    }
                },
                Job::QueueEdit {
                    session,
                    lane,
                    slot,
                    expected,
                    text,
                    images,
                } => {
                    let mut mutation =
                        serde_json::json!({"type": "replace", "text": text.clone()});
                    if !images.is_empty() {
                        // the daemon keeps the parked attachments unless a new
                        // list is present
                        mutation["images"] = serde_json::Value::Array(images.clone());
                    }
                    let outcome = mutate_queue(active, &session, &lane, slot, &expected, mutation);
                    if crate::env_cfg::cfg().debug {
                        eprintln!("[queue] replace {lane}[{slot}] -> {outcome:?}");
                    }
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        out.send(Msg::Queue { steering, follow_up });
                    }
                    match outcome {
                        Ok(status) if status == "applied" => {}
                        // the row moved or went away under us: send it as a new
                        // message rather than lose what was typed
                        other => {
                            if crate::env_cfg::cfg().debug {
                                eprintln!("[queue] replace rejected ({other:?}); sending as new");
                            }
                            out.send(Msg::Prompt { session, message: text, images });
                        }
                    }
                }
                Job::Commands { session } => {
                    let rows = get_commands(active, &session).unwrap_or_default();
                    if crate::env_cfg::cfg().debug {
                        eprintln!("[commands] {} extra command(s)", rows.len());
                    }
                    out.send(Msg::ExtraCommands(rows));
                }
                Job::Stats { session } => {
                    if let Ok(stats) = session_stats(active, &session) {
                        out.send(Msg::Stats { session, stats });
                    }
                }
                Job::ActivityRead {
                    session,
                    session_id,
                } => {
                    let subagents = pa_client::list_roster(active).ok().map(|roster| {
                        pa_client::running_subagents(&roster, &session, &session_id)
                    });
                    let heartbeats =
                        pa_client::scoped_heartbeat_count(active, &session, &session_id).ok();
                    let shells = pa_client::running_shells(active, &session).ok();
                    if subagents.is_some() || heartbeats.is_some() || shells.is_some() {
                        out.send(Msg::ActivityCounts {
                            session,
                            subagents,
                            heartbeats,
                            shells,
                        });
                    }
                }
                Job::SessionState { session } => {
                    if let Ok(state) = session_state(active, &session) {
                        if crate::env_cfg::cfg().debug {
                            eprintln!(
                                "[state] {}/{} at {}",
                                state.provider, state.model_id, state.thinking_level
                            );
                        }
                        out.send(Msg::State { state });
                    }
                }
                Job::SetLevel {
                    session,
                    provider,
                    model,
                    level,
                    switch,
                } => {
                    // a level belongs to a model, so the switch goes first: the
                    // daemon clamps the level to whatever model is running
                    let switched = if switch {
                        set_model(active, &session, &provider, &model)
                    } else {
                        Ok(String::new())
                    };
                    if let Err(error) = switched {
                        out.send(Msg::Error {
                            session: Some(session.clone()),
                            message: model_change_error(&provider, &model, &error),
                        });
                        continue;
                    }
                    match set_thinking_level(active, &session, &level) {
                        Ok(_) => {
                            if crate::env_cfg::cfg().debug {
                                eprintln!("[model] {provider}/{model} effort -> {level}");
                            }
                            if let Ok(state) = session_state(active, &session) {
                                out.send(Msg::State { state });
                            }
                        }
                        Err(error) => {
                            out.send(Msg::Error {
                                session: Some(session.clone()),
                                message: format!("Could not change the effort to {level}: {error}"),
                            });
                        }
                    }
                }
                Job::ModelCatalog { session } => {
                    match model_catalog(active, &session) {
                        Ok(catalog) => {
                            if crate::env_cfg::cfg().debug {
                                eprintln!("[models] {} entries", catalog.len());
                            }
                            out.send(Msg::ModelCatalog(catalog));
                        }
                        Err(error) => {
                            out.send(Msg::Error {
                                session: Some(session.clone()),
                                message: format!("Could not load the Model menu: {error}"),
                            });
                        }
                    }
                }
                Job::Queue { session } => {
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        out.send(Msg::Queue { steering, follow_up });
                    }
                }
                Job::QueueAct {
                    session,
                    lane,
                    slot,
                    text,
                    action,
                } => {
                    // `clear` is not a mutation of one row: it empties both lanes
                    if action == "clear" {
                        let outcome = clear_queue(active, &session);
                        if crate::env_cfg::cfg().debug {
                            eprintln!("[queue] clear -> {outcome:?}");
                        }
                        if let Ok((steering, follow_up)) = get_queue(active, &session) {
                            out.send(Msg::Queue { steering, follow_up });
                        }
                        continue;
                    }
                    let mutation = match action.as_str() {
                        // a follow-up promoted to the steering lane is the same
                        // text re-laned: the daemon delivers it inside the turn
                        // that is already running
                        "promote" => serde_json::json!({
                            "type": "replace",
                            "lane": "steering",
                            "text": text,
                        }),
                        // editing pulls the message back into the composer, so
                        // the queue entry goes away first
                        _ => serde_json::json!({"type": "delete"}),
                    };
                    let outcome = mutate_queue(active, &session, &lane, slot, &text, mutation);
                    if crate::env_cfg::cfg().debug {
                        eprintln!("[queue] {action} {lane}[{slot}] -> {outcome:?}");
                    }
                    match outcome {
                        Ok(status) if status == "applied" => {
                            if action == "edit" {
                                out.send(Msg::QueueEdit { text: text.clone() });
                            }
                            // the lanes as the daemon now has them
                            if let Ok((steering, follow_up)) = get_queue(active, &session) {
                                out.send(Msg::Queue { steering, follow_up });
                            }
                        }
                        Ok(_) => {
                            out.send(Msg::Error {
                                session: Some(session.clone()),
                                message: "Could not change the parked message: it changed already"
                                    .into(),
                            });
                        }
                        Err(error) => {
                            out.send(Msg::Error {
                                session: Some(session.clone()),
                                message: format!("Could not change the parked message: {error}"),
                            });
                        }
                    }
                }
                Job::Abort { session } => {
                    // `stop` = abort_and_send_queued when the daemon has it, so the
                    // queue keeps flowing behind the stopped run
                    if let Err(error) = stop(active, &session) {
                        out.send(Msg::PromptResult {
                            session: Some(session.clone()),
                            error: Some(error.to_string()),
                        });
                    }
                }
                Job::Steer { session, message, images } => {
                    let outcome = steer_prompt(active, &session, &message, &images);
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[steer] session={session} message={message:?} images={} outcome={outcome:?}",
                            images.len()
                        );
                    }
                    if let Err(error) = outcome {
                        out.send(Msg::PromptResult {
                            session: Some(session.clone()),
                            error: Some(error.to_string()),
                        });
                    }
                }
                Job::Prompt { session, message, images } => {
                    // A prompt names the chat it belongs to. If this connection
                    // is bound to another chat - or to none - bind first, then
                    // send: sending into a session the connection is not
                    // attached to is how a message can be accepted and then go
                    // nowhere. The TUI does the same rebind-and-send.
                    if attached.as_deref() != Some(session.as_str()) {
                        resume = None;
                        match attach_as(active, &session, Some(&client_id), None) {
                            Ok(messages) => {
                                attached = Some(session.clone());
                                let (rows, skipped) = crate::transcript::build(&messages, window_rows);
                                out.send(Msg::Transcript {
                                    session: session.clone(),
                                    rows,
                                    skipped,
                                    messages,
                                });
                            }
                            Err(error) => {
                                if crate::env_cfg::cfg().debug {
                                    eprintln!("[prompt] bind before send failed: {error}");
                                }
                                out.send(Msg::PromptRejected {
                                    session: Some(session.clone()),
                                    text: message,
                                });
                                out.send(Msg::PromptResult {
                                    session: Some(session.clone()),
                                    error: Some(format!(
                                        "the window could not reach this chat: {error}"
                                    )),
                                });
                                continue;
                            }
                        }
                    }
                    // The daemon's verdict, awaited: an accepted message and a
                    // refused one used to look identical from here, so a refusal
                    // was silence. This is what the TUI's submit does.
                    let outcome = send_prompt_checked(active, &session, &message, &images);
                    if crate::env_cfg::cfg().debug {
                        eprintln!(
                            "[prompt] session={session} message={message:?} images={} outcome={outcome:?}",
                            images.len()
                        );
                    }
                    // Ask what the daemon actually queued. This is the only
                    // source for those lanes: the window may guess that a turn
                    // is running, but it may not guess that a message is waiting.
                    if let Ok((steering, follow_up)) = get_queue(active, &session) {
                        out.send(Msg::Queue { steering, follow_up });
                    }
                    match outcome {
                        Ok(()) => {
                            out.send(Msg::PromptResult {
                                session: Some(session.clone()),
                                error: None,
                            });
                        }
                        Err(reason) => {
                            // The text goes back to the composer and the reason
                            // is said plainly, instead of the message vanishing.
                            out.send(Msg::PromptRejected {
                                session: Some(session.clone()),
                                text: message,
                            });
                            out.send(Msg::PromptResult {
                                session: Some(session.clone()),
                                error: Some(reason),
                            });
                        }
                    }
                }
                Job::LoadOlder {
                    session,
                    before,
                    limit,
                } => {
                    match pa_client::get_page(active, &session, Some(&before), limit) {
                        Ok(page) => {
                            let rows = crate::transcript::build(&page.messages, page.messages.len()).0;
                            out.send(Msg::Older {
                                session: session.clone(),
                                rows,
                                messages: page.messages,
                                first_entry: page.first_entry,
                                more: page.more,
                            });
                        }
                        Err(error) => {
                            if crate::env_cfg::cfg().debug {
                                eprintln!("[older] {session} FAILED: {error}");
                            }
                            // the reader must not be left waiting on a page that
                            // will never arrive
                            out.send(Msg::Older {
                                session: session.clone(),
                                rows: Vec::new(),
                                messages: Vec::new(),
                                first_entry: String::new(),
                                more: false,
                            });
                        }
                    }
                }
                Job::Tail { session, fresh } => {
                    // Event frames are droppable: the daemon publishes them with
                    // try_send into a bounded per-connection queue, so a client
                    // that falls behind simply loses message_update/message_end -
                    // silently, with no notice. That is why a turn can stop
                    // half-written. The store is the authority.
                    //
                    // Which store view you get depends on the connection: a
                    // long-lived one keeps answering from the snapshot it fell
                    // out of step with (measured: store at 18 while it returned
                    // 17 for 30s), while a freshly opened connection always
                    // answers with current messages. So a forced sync opens a new
                    // connection and drops it; ordinary polls reuse one.
                    // Every read opens its own connection: a connection that has
                    // been used once keeps answering with the snapshot it saw at
                    // connect time (measured 102 -> 102 across nine seconds
                    // while fresh connections advanced 96 -> 108). The cost is a
                    // local pipe open per poll; the alternative is a window that
                    // stops one turn short.
                    // one tail page, not the whole branch: the typed cursor the
                    // terminal UI uses, so a long session costs the same as a
                    // short one
                    let read = pa_client::get_page(active, &session, None, WINDOW_ROWS);
                    match read {
                    Ok(page) => {
                        let (rows, skipped) = crate::transcript::build(&page.messages, window_rows);
                        out.send(Msg::Tail {
                            session,
                            rows,
                            skipped,
                            messages: page.messages,
                            last_entry: page.last_entry,
                            first_entry: page.first_entry,
                            more: page.more,
                            fresh,
                        });
                    }
                    Err(error) => {
                        if crate::env_cfg::cfg().debug {
                            eprintln!("[tail] {session} FAILED: {error}");
                        }
                    }
                    }
                }
            }
        }

        // drain live frames even before a session is attached: the roster
        // subscription is independent of attachment, and holding its response
        // or pushes until an attach would leave the sidebar on the poll.
        while let Some(frame) = active.recv(Duration::from_millis(0)) {
            let frame_type = frame
                .get("type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("?");
            if crate::env_cfg::cfg().debug {
                let event_type = frame
                    .get("event")
                    .and_then(|event| event.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("");
                eprintln!("[frame] {frame_type} {event_type}");
                if crate::env_cfg::cfg().dump.is_some() {
                    // chars, not bytes: slicing a String at byte 700 splits
                    // whatever multi-byte character sits there and panics the
                    // reader thread, which is the thread every event and every
                    // reply arrives on - the window goes deaf and silent
                    let text: String = frame.to_string().chars().take(700).collect();
                    eprintln!("[dump] {text}");
                }
            }
            if frame_type == "session_closed" {
                let session = frame
                    .get("activeSessionId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if session.is_empty() {
                    continue;
                }
                let reason = frame
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                out.send(Msg::SessionClosed { session, reason });
                continue;
            }
            if frame_type == "response" {
                let id = frame.get("id").and_then(serde_json::Value::as_str).unwrap_or("");
                if pending_roster.as_deref() == Some(id) {
                    pending_roster = None;
                    let accepted =
                        frame.get("success").and_then(serde_json::Value::as_bool) == Some(true);
                    roster_subscribed = accepted;
                    roster_failed = !accepted;
                    if accepted {
                        if let Some(entries) = frame
                            .pointer("/data/roster")
                            .and_then(serde_json::Value::as_array)
                        {
                            roster_entries = entries.clone();
                            roster = roster_rows(entries);
                            out.send(Msg::Sessions(Ok(merged_sessions(&saved_rows, &roster))));
                            // The subscription snapshot can be older than the
                            // last list, and an idle flip may already have been
                            // content-diffed away. Take a full list now so the
                            // two sources agree before the first poll window.
                            last_list = SystemTime::now() - Duration::from_secs(60);
                            if let Some(session) = attached.as_deref() {
                                if let Some(subagents) = activity_subagents(
                                    &roster_entries,
                                    session,
                                    &roster,
                                    &saved_rows,
                                ) {
                                    out.send(Msg::ActivityCounts {
                                        session: session.to_string(),
                                        subagents: Some(subagents),
                                        heartbeats: None,
                                        shells: None,
                                    });
                                }
                            }
                        }
                    }
                    continue;
                }
                if let Some(position) = pending_prompts.iter().position(|(open, _)| open == id) {
                    let (_, text) = pending_prompts.remove(position);
                    if frame.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
                        let reason = frame
                            .get("error")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("rejected by the daemon");
                        out.send(Msg::PromptRejected { session: None, text });
                        out.send(Msg::Error { session: None, message: reason.to_string() });
                    }
                } else if let Some(position) =
                    pending_renames.iter().position(|(open, _)| open == id)
                {
                    let (_, session) = pending_renames.remove(position);
                    if frame.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
                        let reason = frame
                            .get("error")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("rename rejected by the daemon");
                        out.send(Msg::Error {
                            session: Some(session),
                            message: reason.to_string(),
                        });
                    }
                }
                continue;
            }
            if frame_type == "roster_update" {
                roster_live = true;
                apply_roster_update(&mut roster, &frame);
                apply_roster_entries(&mut roster_entries, &frame);
                out.send(Msg::Sessions(Ok(merged_sessions(&saved_rows, &roster))));
                if let Some(session) = attached.as_deref() {
                    if let Some(subagents) =
                        activity_subagents(&roster_entries, session, &roster, &saved_rows)
                    {
                        out.send(Msg::ActivityCounts {
                            session: session.to_string(),
                            subagents: Some(subagents),
                            heartbeats: None,
                            shells: None,
                        });
                    }
                }
                continue;
            }
            if frame_type == "heartbeats_changed" {
                if let Some(session) = attached.clone() {
                    if let Some(row) = activity_row(&session, &roster, &saved_rows) {
                        if let Ok(heartbeats) =
                            pa_client::scoped_heartbeat_count(active, &session, &row.session_id)
                        {
                            out.send(Msg::ActivityCounts {
                                session,
                                subagents: None,
                                heartbeats: Some(heartbeats),
                                shells: None,
                            });
                        }
                    }
                }
                continue;
            }
            if frame_type == "session_binding" {
                let previous = frame
                    .get("previousActiveSessionId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let active = frame
                    .get("activeSessionId")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if !previous.is_empty() && !active.is_empty() && previous != active {
                    out.send(Msg::SessionBinding { previous, active });
                }
                continue;
            }
            if frame_type != "session_event" {
                continue;
            }
            let session = frame
                .get("activeSessionId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if Some(session.to_string()) != attached {
                continue;
            }
            if let Some(meta) = frame.get("meta") {
                let generation = meta
                    .get("generation")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let sequence = meta
                    .get("sequence")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0);
                if !generation.is_empty() && sequence > 0 {
                    resume = Some((generation.to_string(), sequence));
                }
            }
            if let Some(event) = frame.get("event") {
                out.send(Msg::Event {
                    session: session.to_string(),
                    event: event.clone(),
                });
                if let Some(label) = pa_client::activity_label(event) {
                    out.send(Msg::Activity { label: label.to_string() });
                }
                let event_type = event
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if matches!(event_type, "bash_start" | "bash_end")
                    && attached.as_deref() == Some(session)
                {
                    if let Ok(shells) = pa_client::running_shells(active, session) {
                        out.send(Msg::ActivityCounts {
                            session: session.to_string(),
                            subagents: None,
                            heartbeats: None,
                            shells: Some(shells),
                        });
                    }
                }
            }
        }

        let list_interval = if roster_live { 30_000 } else { 2_000 };
        if last_list.elapsed().unwrap_or_default() > Duration::from_millis(list_interval) {
            let result = list_sessions(active);
            match result {
                Ok(rows) => {
                    apply_list_snapshot(&mut roster, &mut saved_rows, rows);
                    out.send(Msg::Sessions(Ok(merged_sessions(&saved_rows, &roster))));
                }
                Err(error) => {
                    client = None;
                    out.send(Msg::Sessions(Err(error.to_string())));
                }
            }
            last_list = SystemTime::now();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        activity_subagents, apply_list_snapshot, apply_roster_entries, apply_roster_update,
        merged_sessions, reconcile_roster_with_saved, Msg,
    };
    use crate::SessionRow;

    // --- the session menu, at the seam between the row and the commands ------

    // The menu lives behind a window's callback, so its decisions are tested
    // through the functions that hold them, not through the UI.

    fn keys(items: &[crate::SessionMenuItem]) -> Vec<String> {
        items.iter().map(|item| item.key.to_string()).collect()
    }

    #[test]
    fn a_live_row_offers_done_and_never_delete() {
        let items = crate::sessions::menu_items(false, false);
        let keys = keys(&items);
        assert_eq!(keys, ["done", "rename", "share", "copy-id"]);
        assert!(
            !keys.iter().any(|key| key == "delete"),
            "a live row must never offer the action that destroys its file: {keys:?}"
        );
        assert_eq!(items[0].label.as_str(), "Mark done");
    }

    #[test]
    fn a_done_row_offers_delete_and_neither_done_nor_share() {
        let items = crate::sessions::menu_items(true, false);
        let keys = keys(&items);
        assert_eq!(keys, ["rename", "copy-id", "delete"]);
        assert!(
            !keys.iter().any(|key| key == "done"),
            "a done row is already set aside: {keys:?}"
        );
        // `export_html` needs a worker, and a done row has none
        assert!(
            !keys.iter().any(|key| key == "share"),
            "a done row has no worker to export it: {keys:?}"
        );
        // Delete sits below the separator: it acts on the file, not the session
        let delete = items.iter().find(|item| item.key == "delete").expect("delete");
        assert!(delete.separated);
    }

    #[test]
    fn delete_asks_once_before_it_acts() {
        let armed = crate::sessions::menu_items(true, true);
        let delete = armed.iter().find(|item| item.key == "delete").expect("delete");
        assert_eq!(delete.label.as_str(), "Delete for good");
        // and it is still the same key: the second press is the act
        assert_eq!(delete.key.as_str(), "delete");
        assert!(delete.danger);
        assert_eq!(delete.note.as_str(), "this removes the session file");
        let unarmed = crate::sessions::menu_items(true, false);
        let delete = unarmed.iter().find(|item| item.key == "delete").expect("delete");
        assert_eq!(delete.label.as_str(), "Delete");
        assert!(delete.note.is_empty());
    }

    #[test]
    fn every_menu_row_says_what_it_would_do() {
        for done in [false, true] {
            for item in crate::sessions::menu_items(done, false) {
                assert!(item.note.is_empty(), "{} has unexpected subtext {:?}", item.key, item.note);
            }
        }
    }

    #[test]
    fn a_live_row_cannot_be_talked_into_deleting_and_a_done_row_cannot_start() {
        use crate::sessions::{menu_action, MenuAction};
        assert_eq!(menu_action("delete", false), MenuAction::Nothing);
        assert_eq!(menu_action("done", true), MenuAction::Nothing);
        assert_eq!(menu_action("share", true), MenuAction::Nothing);
        assert_eq!(menu_action("share", false), MenuAction::Share);
        assert_eq!(menu_action("delete", true), MenuAction::Delete);
        assert_eq!(menu_action("done", false), MenuAction::MarkDone);
        // every key a row offers is one some row can execute
        for done in [false, true] {
            for item in crate::sessions::menu_items(done, false) {
                assert_ne!(
                    menu_action(&item.key, done),
                    MenuAction::Nothing,
                    "{} offered on done={done} does nothing",
                    item.key
                );
            }
        }
    }

    fn tail() -> Msg {
        Msg::Tail {
            session: String::new(),
            rows: Vec::new(),
            skipped: 0,
            messages: Vec::new(),
            last_entry: String::new(),
            first_entry: String::new(),
            more: false,
            fresh: false,
        }
    }

    #[test]
    fn the_fence_covers_the_replies_about_one_chat() {
        // every kind of read a chat answers
        assert!(tail().is_session_scoped());
        assert!(Msg::Transcript { session: String::new(), rows: Vec::new(), skipped: 0, messages: Vec::new() }.is_session_scoped());
        assert!(Msg::Older {
            session: String::new(),
            rows: Vec::new(),
            messages: Vec::new(),
            first_entry: String::new(),
            more: false,
        }
        .is_session_scoped());
        assert!(Msg::Event { session: String::new(), event: serde_json::Value::Null }.is_session_scoped());
        assert!(Msg::ExtraCommands(Vec::new()).is_session_scoped());
        assert!(Msg::ModelCatalog(Vec::new()).is_session_scoped());
        assert!(Msg::Stats {
            session: String::new(),
            stats: pa_client::SessionStats::default(),
        }
        .is_session_scoped());
        assert!(Msg::State {
            state: pa_client::SessionState::default(),
        }
        .is_session_scoped());
    }

    #[test]
    fn the_fence_never_swallows_what_belongs_to_the_composer() {
        // these are about the words on screen, not about a chat's contents: a
        // reader who switches chats still needs the refusal of their own
        // prompt, the reconnection notice, and the fresh chat's id
        assert!(!Msg::Reconnected.is_session_scoped());
        assert!(!Msg::PromptResult {
            session: None,
            error: None
        }
        .is_session_scoped());
        assert!(!Msg::PromptRejected {
            session: None,
            text: String::new()
        }
        .is_session_scoped());
        assert!(!Msg::NewChat(String::new()).is_session_scoped());
        assert!(!Msg::QueueEdit { text: String::new() }.is_session_scoped());
        assert!(!Msg::Sessions(Ok(Vec::new())).is_session_scoped());
        assert!(!Msg::Error { session: None, message: String::new() }.is_session_scoped());
        assert!(!Msg::SessionClosed {
            session: String::new(),
            reason: String::new(),
        }
        .is_session_scoped());
        assert!(!Msg::Activity { label: String::new() }.is_session_scoped());
    }

    fn saved_row(name: &str) -> SessionRow {
        SessionRow::from_value(&serde_json::json!({
            "activeSessionId": "a1",
            "sessionId": "s1",
            "sessionFile": "/tmp/s1.jsonl",
            "sessionName": name,
            "lifecycle": "live",
            "activity": "idle",
        }))
    }

    fn roster_entry_on(agent: &str, session: &str, name: &str) -> serde_json::Value {
        serde_json::json!({
            "agentId": agent,
            "status": "running",
            "summary": {
                "activeSessionId": agent,
                "sessionId": session,
                "sessionFile": format!("/tmp/{session}.jsonl"),
                "sessionName": name,
                "lifecycle": "live",
                "activity": "idle",
            },
        })
    }

    fn roster_entry(name: &str) -> serde_json::Value {
        roster_entry_on("a1", "s1", name)
    }

    #[test]
    fn roster_push_updates_a_name_and_replaces_the_saved_row() {
        let mut roster = Vec::new();
        apply_roster_update(
            &mut roster,
            &serde_json::json!({"changed": [roster_entry("old")]}),
        );
        assert_eq!(merged_sessions(&[saved_row("old")], &roster)[0].name, "old");

        apply_roster_update(
            &mut roster,
            &serde_json::json!({"changed": [roster_entry("new")]}),
        );
        let rows = merged_sessions(&[saved_row("old")], &roster);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "new");
    }

    #[test]
    fn a_fresh_list_clears_a_stale_running_roster_row() {
        let mut stale = roster_entry_on("a1", "s1", "stale");
        stale["summary"]["activity"] = serde_json::json!("working");
        stale["summary"]["isStreaming"] = serde_json::json!(true);
        stale["summary"]["isSessionActive"] = serde_json::json!(true);
        stale["summary"]["isRunningTools"] = serde_json::json!(true);
        let mut roster = Vec::new();
        apply_roster_update(
            &mut roster,
            &serde_json::json!({"changed": [stale]}),
        );
        assert_eq!(
            merged_sessions(&[saved_row("fresh")], &roster)[0].live_state(),
            "Executing"
        );

        // The full list says the Session is idle. It must win over the cached
        // running row, or every poll leaves the strip up for good.
        let mut saved = vec![saved_row("old")];
        apply_list_snapshot(&mut roster, &mut saved, vec![saved_row("fresh")]);
        let rows = merged_sessions(&saved, &roster);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "fresh");
        assert_eq!(rows[0].live_state(), "idle");
    }

    #[test]
    fn reconciliation_keeps_a_live_row_the_catalog_has_not_reported() {
        let mut roster = Vec::new();
        apply_roster_update(
            &mut roster,
            &serde_json::json!({"changed": [roster_entry_on("a2", "s2", "live-only")]}),
        );
        let saved = vec![saved_row("saved")];
        reconcile_roster_with_saved(&mut roster, &saved);
        let rows = merged_sessions(&saved, &roster);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.name == "live-only"));
    }

    #[test]
    fn roster_push_removes_an_agent_and_resyncs_the_store() {
        let mut roster = Vec::new();
        apply_roster_update(
            &mut roster,
            &serde_json::json!({"changed": [roster_entry("one")]}),
        );
        apply_roster_update(
            &mut roster,
            &serde_json::json!({"resync": true, "changed": [roster_entry_on("a2", "s2", "two")]}),
        );
        assert_eq!(roster.len(), 1);
        assert_eq!(roster[0].0, "a2");
        assert_eq!(roster[0].1.name, "two");

        apply_roster_update(&mut roster, &serde_json::json!({"removed": ["a2"]}));
        assert!(roster.is_empty());
    }

    #[test]
    fn raw_roster_tracks_changed_removed_and_resync() {
        let mut entries = vec![serde_json::json!({"agentId": "a1"})];
        apply_roster_entries(
            &mut entries,
            &serde_json::json!({"changed": [{"agentId": "a2"}]}),
        );
        assert_eq!(entries.len(), 2);

        apply_roster_entries(&mut entries, &serde_json::json!({"removed": ["a1"]}));
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["agentId"], "a2");

        apply_roster_entries(
            &mut entries,
            &serde_json::json!({"resync": true, "changed": [{"agentId": "a3"}]}),
        );
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["agentId"], "a3");
    }

    #[test]
    fn activity_count_follows_the_attached_roster_family() {
        let entries = vec![
            serde_json::json!({"agentId": "root", "status": "idle", "summary": {
                "activeSessionId": "root", "sessionId": "uuid-root",
                "rlmDepth": 0, "runtimeKind": "top-level", "sessionFile": "/root.jsonl"
            }}),
            serde_json::json!({"agentId": "child", "status": "running", "summary": {
                "activeSessionId": "child", "sessionId": "uuid-child",
                "parentActiveSessionId": "root", "parentSessionId": "uuid-root",
                "rlmDepth": 1, "runtimeKind": "subagent"
            }}),
        ];
        let roster = vec![("root".to_string(), SessionRow {
            id: "root".to_string(),
            session_id: "uuid-root".to_string(),
            ..SessionRow::default()
        })];

        assert_eq!(activity_subagents(&entries, "root", &roster, &[]), Some(1));
    }
}
