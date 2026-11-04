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
}


pub(crate) enum Msg {
    Sessions(Result<Vec<SessionRow>, String>),
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
        rows: Vec<crate::transcript::Row>,
        messages: Vec<serde_json::Value>,
        first_entry: String,
        more: bool,
    },
    /// one `session_event` from the attached session, forwarded verbatim
    Event { session: String, event: serde_json::Value },
    PromptResult { error: Option<String> },
    /// the daemon refused a prompt: hand the text back to the composer
    PromptRejected { text: String },
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
    State { state: pa_client::SessionState },
    Activity { label: String },
    /// a share finished: the gist URL to copy and the viewer URL to open, or
    /// the reason it could not be made
    Shared(Result<crate::share::Shared, String>),
    Error(String),
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
                | Msg::State { .. }
                | Msg::ModelCatalog(_)
                | Msg::ExtraCommands(_)
                | Msg::Queue { .. }
        )
    }
}

/// A stable number for a page cursor, so the poll can tell "same page" quickly.
pub(crate) fn page_cursor(last_entry: &str) -> u64 {
    last_entry
        .bytes()
        .fold(0u64, |hash, byte| hash.wrapping_mul(31).wrapping_add(u64::from(byte)))
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
        let _ = reporter.send((0, Msg::Error("engine stopped \u{2014} reopen the window".into())));
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
    let mut resume: Option<(String, u64)> = None;
    // the outage line is already on the status bar for this stretch of failure
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
                    out.send(Msg::Reconnected);
                }
                Err(error) => {
                    // Reported once per outage: the retry is every two seconds,
                    // and the same line repeated is not news. The loop then runs
                    // on to the jobs below rather than starting over, so a
                    // prompt typed while the daemon is away is still answered
                    // and the list keeps its rows instead of freezing.
                    if !offline {
                        out.send(Msg::Error(error.to_string()));
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
            out.send(Msg::Error("daemon connection lost \u{2014} reconnecting".into()));
            std::thread::sleep(Duration::from_millis(300));
            continue;
        }

        while let Ok(job) = jobs.try_recv() {
            match job {
                Job::Rename { session, name } => {
                    let _ = active.send(
                        "rename",
                        serde_json::json!({"activeSessionId": session, "name": name}),
                    );
                    last_list = SystemTime::now() - Duration::from_secs(60);
                }
                Job::RenameSaved { session_path, name } => {
                    match pa_client::rename_saved_session(active, &session_path, &name) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error(error.to_string())),
                    }
                }
                Job::MarkDone { session } => {
                    // the TUI's ctrl+x: the worker archives the session on its
                    // way out, and the row moves to done on the next poll
                    match pa_client::mark_done(active, &session) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error(error.to_string())),
                    }
                }
                Job::DeleteSaved { session_path } => {
                    match pa_client::delete_saved_session(active, &session_path) {
                        Ok(_) => last_list = SystemTime::now() - Duration::from_secs(60),
                        Err(error) => out.send(Msg::Error(error.to_string())),
                    }
                }
                Job::Share { session } => {
                    out.send(Msg::Shared(crate::share::share_as_gist(active, &session)));
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
                        Ok(messages) => {
                            attached = Some(session.clone());
                            let (rows, skipped) = crate::transcript::build(&messages, window_rows);
                            out.send(Msg::Transcript { session, rows, skipped, messages });
                            last_list = SystemTime::now() - Duration::from_secs(60);
                        }
                        Err(error) => {
                            out.send(Msg::Error(error.to_string()));
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
                        out.send(Msg::Activity {
                            label: format!("{provider}/{model}: {error}"),
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
                        out.send(Msg::Activity {
                            label: format!("new chat: {error}"),
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
                        out.send(Msg::Activity {
                            label: format!("{provider}/{model}: {error}"),
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
                            out.send(Msg::Activity {
                                label: format!("effort {level}: {error}"),
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
                            out.send(Msg::Activity {
                                label: format!("model catalog: {error}"),
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
                            out.send(Msg::Activity {
                                label: "that queue row changed already".into(),
                            });
                        }
                        Err(error) => {
                            out.send(Msg::Activity { label: error.to_string() });
                        }
                    }
                }
                Job::Abort { session } => {
                    // `stop` = abort_and_send_queued when the daemon has it, so the
                    // queue keeps flowing behind the stopped run
                    if let Err(error) = stop(active, &session) {
                        out.send(Msg::PromptResult { error: Some(error.to_string()) });
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
                        out.send(Msg::PromptResult { error: Some(error.to_string()) });
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
                                out.send(Msg::PromptRejected { text: message });
                                out.send(Msg::PromptResult {
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
                            out.send(Msg::PromptResult { error: None });
                        }
                        Err(reason) => {
                            // The text goes back to the composer and the reason
                            // is said plainly, instead of the message vanishing.
                            out.send(Msg::PromptRejected { text: message });
                            out.send(Msg::PromptResult {
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

        // drain live events for the attached session: this is what turns a
        // coarse "running tool" into the TUI's Thinking / Writing / Writing code
        if attached.is_some() {
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
                if frame_type == "response" {
                    let id = frame.get("id").and_then(serde_json::Value::as_str).unwrap_or("");
                    if let Some(position) = pending_prompts.iter().position(|(open, _)| open == id) {
                        let (_, text) = pending_prompts.remove(position);
                        if frame.get("success").and_then(serde_json::Value::as_bool) != Some(true) {
                            let reason = frame
                                .get("error")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("rejected by the daemon");
                            out.send(Msg::PromptRejected { text });
                            out.send(Msg::Error(reason.to_string()));
                        }
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
                }
            }
        }

        if last_list.elapsed().unwrap_or_default() > Duration::from_millis(2000) {
            let result = list_sessions(active);
            if result.is_err() {
                client = None;
            }
            out.send(Msg::Sessions(result.map_err(|error| error.to_string())));
            last_list = SystemTime::now();
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

#[cfg(test)]
mod tests {
    use super::Msg;

    // --- the session menu, at the seam between the row and the commands ------

    // The menu lives behind a window's callback, so its decisions are tested
    // through the functions that hold them, not through the UI.

    fn keys(items: &[crate::SessionMenuItem]) -> Vec<String> {
        items.iter().map(|item| item.key.to_string()).collect()
    }

    #[test]
    fn a_live_row_offers_done_and_never_delete() {
        let items = crate::interactive::sidebar::menu_items(false, false);
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
        let items = crate::interactive::sidebar::menu_items(true, false);
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
        let armed = crate::interactive::sidebar::menu_items(true, true);
        let delete = armed.iter().find(|item| item.key == "delete").expect("delete");
        assert_eq!(delete.label.as_str(), "Delete for good");
        // and it is still the same key: the second press is the act
        assert_eq!(delete.key.as_str(), "delete");
        assert!(delete.danger);
        assert_eq!(delete.note.as_str(), "this removes the session file");
        let unarmed = crate::interactive::sidebar::menu_items(true, false);
        let delete = unarmed.iter().find(|item| item.key == "delete").expect("delete");
        assert_eq!(delete.label.as_str(), "Delete");
        assert!(delete.note.is_empty());
    }

    #[test]
    fn every_menu_row_says_what_it_would_do() {
        for done in [false, true] {
            for item in crate::interactive::sidebar::menu_items(done, false) {
                assert!(item.note.is_empty(), "{} has unexpected subtext {:?}", item.key, item.note);
            }
        }
    }

    #[test]
    fn a_live_row_cannot_be_talked_into_deleting_and_a_done_row_cannot_start() {
        use crate::interactive::sidebar::{menu_action, MenuAction};
        assert_eq!(menu_action("delete", false), MenuAction::Nothing);
        assert_eq!(menu_action("done", true), MenuAction::Nothing);
        assert_eq!(menu_action("share", true), MenuAction::Nothing);
        assert_eq!(menu_action("share", false), MenuAction::Share);
        assert_eq!(menu_action("delete", true), MenuAction::Delete);
        assert_eq!(menu_action("done", false), MenuAction::MarkDone);
        // every key a row offers is one some row can execute
        for done in [false, true] {
            for item in crate::interactive::sidebar::menu_items(done, false) {
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
        assert!(!Msg::PromptResult { error: None }.is_session_scoped());
        assert!(!Msg::PromptRejected { text: String::new() }.is_session_scoped());
        assert!(!Msg::NewChat(String::new()).is_session_scoped());
        assert!(!Msg::QueueEdit { text: String::new() }.is_session_scoped());
        assert!(!Msg::Sessions(Ok(Vec::new())).is_session_scoped());
        assert!(!Msg::Error(String::new()).is_session_scoped());
        assert!(!Msg::Activity { label: String::new() }.is_session_scoped());
    }
}
