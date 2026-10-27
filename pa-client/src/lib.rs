//! Daemon wire client, on top of `pa-tui`'s transport.
//!
//! This crate used to own a hand-rolled pipe reader: one thread, `PeekNamedPipe`,
//! 8ms naps. It could not keep up with the daemon's event queue (which drops
//! frames for a slow client), so turns arrived truncated. The connection now
//! comes from `pa_tui::daemon_client` - the client the terminal UI ships with -
//! and this crate keeps only the typed helpers the UI calls.

mod wire;

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::Duration;

pub const DAEMON_PIPE: &str = r"\\.\pipe\prime-agent-daemon";
pub const PROTOCOL_NAME: &str = "prime-agent.daemon";
pub const PROTOCOL_VERSION: u64 = 7;

pub type Frame = Value;

/// Handle to the daemon: commands go in, frames come out.
///
/// The transport is `pa-tui`'s own client — the same one the terminal UI ships
/// with — rather than a second implementation. That matters beyond tidiness:
/// the daemon hands each connection its events through a bounded queue and
/// drops frames for a client that falls behind, so the reader has to be a real
/// async task that is always draining. Reimplementing that by hand is how the
/// window ended up stopping mid-turn.
pub struct Client {
    rt: tokio::runtime::Runtime,
    inner: std::sync::Arc<wire::daemon_client::DaemonClient>,
    frames: Mutex<Receiver<Frame>>,
    frames_tx: Sender<Frame>,
    backlog: Mutex<VecDeque<Frame>>,
    next_id: AtomicU64,
    dead: std::sync::Arc<AtomicBool>,
    /// The daemon's greeting, exactly as received.
    pub hello: Frame,
}

impl Client {
    /// Connect and start the event pump.
    ///
    /// # Errors
    /// Fails when the daemon cannot be reached or refuses the handshake.
    pub fn connect() -> std::io::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let (client, mut events) = rt
            .block_on(wire::daemon_client::DaemonClient::connect(
                std::path::Path::new(DAEMON_PIPE),
            ))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        let hello = client.hello().clone();
        let client = std::sync::Arc::new(client);

        let (frame_tx, frame_rx) = channel::<Frame>();
        let frame_tx_clone = frame_tx.clone();
        let dead = std::sync::Arc::new(AtomicBool::new(false));
        let pump_dead = std::sync::Arc::clone(&dead);
        rt.spawn(async move {
            // Drain continuously: the daemon drops events for a connection that
            // stops reading, so this loop never sleeps on the consumer's behalf.
            while let Some(event) = events.recv().await {
                let Some(frame) = event_frame(event) else {
                    continue;
                };
                if frame_tx.send(frame).is_err() {
                    break;
                }
            }
            pump_dead.store(true, Ordering::Relaxed);
        });

        Ok(Self {
            rt,
            inner: client,
            frames: Mutex::new(frame_rx),
            frames_tx: frame_tx_clone,
            backlog: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            dead,
            hello,
        })
    }

    /// Whether the daemon connection is still usable.
    pub fn alive(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }

    /// Fire a command and return a local id; the daemon's verdict arrives later
    /// as a `response` frame carrying that id.
    ///
    /// # Errors
    /// Fails when the command does not match the typed wire enum.
    pub fn send(&self, ty: &str, payload: Value) -> std::io::Result<String> {
        let id = format!("ui-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let command = typed_command(ty, payload)?;
        let client = std::sync::Arc::clone(&self.inner);
        let tx = self.frame_sender();
        let reply_id = id.clone();
        let reply_ty = ty.to_string();
        self.rt.spawn(async move {
            let frame = match client
                .request_with_timeout(command, LONG_REQUEST_TIMEOUT_MS)
                .await
            {
                Ok(response) => {
                    let mut value = serde_json::to_value(response).unwrap_or(Value::Null);
                    if let Value::Object(map) = &mut value {
                        map.insert("id".into(), Value::String(reply_id.clone()));
                        map.insert("type".into(), Value::String("response".into()));
                        map.insert("command".into(), Value::String(reply_ty));
                    }
                    value
                }
                Err(error) => json!({
                    "type": "response",
                    "id": reply_id,
                    "command": reply_ty,
                    "success": false,
                    "error": error.to_string(),
                }),
            };
            let _ = tx.send(frame);
        });
        Ok(id)
    }

    /// Send a command and wait for its response.
    ///
    /// # Errors
    /// Fails on a transport error or when the deadline passes.
    pub fn request(
        &self,
        ty: &str,
        payload: Value,
        timeout: Duration,
    ) -> std::io::Result<Frame> {
        let command = typed_command(ty, payload)?;
        let millis = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let response = self
            .rt
            .block_on(self.inner.request_with_timeout(command, millis))
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(serde_json::to_value(response).unwrap_or(Value::Null))
    }

    /// Next frame, backlog first. `None` on timeout.
    pub fn recv(&self, timeout: Duration) -> Option<Frame> {
        if let Some(frame) = self.backlog.lock().expect("backlog").pop_front() {
            return Some(frame);
        }
        let rx = self.frames.lock().expect("frames");
        match rx.recv_timeout(timeout) {
            Ok(frame) => Some(frame),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.dead.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    fn frame_sender(&self) -> Sender<Frame> {
        // a synthetic reply is delivered through the same channel the pump uses
        self.frames_tx.clone()
    }
}

/// The daemon's per-command deadline for anything the UI fires and forgets.
const LONG_REQUEST_TIMEOUT_MS: u64 = 600_000;

/// One command as the typed enum the supervisor parses, so a renamed field is
/// reported here instead of silently doing nothing.
fn typed_command(ty: &str, payload: Value) -> std::io::Result<pa_types::daemon::DaemonCommand> {
    let mut command = payload;
    if let Value::Object(map) = &mut command {
        map.insert("type".into(), Value::String(ty.to_string()));
    }
    match serde_json::from_value::<pa_types::daemon::DaemonCommand>(command.clone()) {
        Ok(command) => Ok(command),
        Err(error) => {
            let report = format!("command {ty} does not match DaemonCommand: {error}");
            if std::env::var("PA_STRICT").is_ok() {
                return Err(std::io::Error::other(report));
            }
            eprintln!("[wire] {report}");
            serde_json::from_value(command)
                .map_err(|_| std::io::Error::other(format!("command {ty} is not on the wire")))
        }
    }
}

/// The wire shape the rest of this crate expects, from one typed event.
fn event_frame(event: wire::daemon_client::DaemonClientEvent) -> Option<Frame> {
    match event {
        wire::daemon_client::DaemonClientEvent::SessionEvent {
            active_session_id,
            event,
            meta_sequence,
        } => Some(json!({
            "type": "session_event",
            "activeSessionId": active_session_id,
            "event": event,
            "meta": {"sequence": meta_sequence},
        })),
        wire::daemon_client::DaemonClientEvent::SessionClosed {
            active_session_id,
            reason,
        } => Some(json!({
            "type": "session_closed",
            "activeSessionId": active_session_id,
            "reason": reason,
        })),
        _ => None,
    }
}

/// The session rows the left rail renders.
#[derive(Debug, Clone, Default)]
pub struct SessionRow {
    pub id: String,
    /// Stable across daemon restarts, unlike `id` (which is re-keyed).
    pub session_id: String,
    pub name: String,
    pub cwd: String,
    pub activity: String,
    pub lifecycle: String,
    pub model: String,
    pub thinking: String,
    pub messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub is_streaming: bool,
    pub is_running_tools: bool,
    pub is_bash_running: bool,
    pub is_compacting: bool,
    pub is_quota_parked: bool,
    pub worker_state: String,
    pub has_subagents: bool,
    pub last_activity: String,
    pub modified: String,
}

impl SessionRow {
    /// What the agent is doing right now, in the TUI's vocabulary.
    ///
    /// The summary only distinguishes coarse states; once stream events are
    /// consumed these labels get refined to Thinking / Writing / Writing code.
    #[must_use]
    pub fn live_state(&self) -> &'static str {
        if self.lifecycle == "archived" {
            return "archived";
        }
        if self.is_quota_parked {
            return "Parked";
        }
        if self.is_compacting {
            return "Compacting";
        }
        if self.is_bash_running {
            return "Executing";
        }
        if self.is_running_tools {
            return "Executing";
        }
        if self.is_streaming {
            return "Thinking";
        }
        if self.activity == "working" {
            return "Waiting";
        }
        "idle"
    }

    #[must_use]
    pub fn from_value(value: &Value) -> Self {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        let usage = value.get("usage").cloned().unwrap_or(Value::Null);
        let usage_num = |key: &str| usage.get(key).and_then(Value::as_u64).unwrap_or(0);
        let named = text("sessionName");
        let name = if named.is_empty() {
            text("firstMessage").chars().take(60).collect::<String>()
        } else {
            named
        };
        Self {
            id: text("activeSessionId"),
            session_id: text("sessionId"),
            name,
            cwd: text("cwd"),
            activity: text("activity"),
            lifecycle: text("lifecycle"),
            model: text("model"),
            thinking: text("thinkingLevel"),
            messages: number("messageCount"),
            input_tokens: usage_num("inputTokens"),
            output_tokens: usage_num("outputTokens"),
            cost: usage.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
            is_streaming: value.get("isStreaming").and_then(Value::as_bool).unwrap_or(false),
            is_running_tools: value
                .get("isRunningTools")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_bash_running: value
                .get("isBashRunning")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_compacting: value
                .get("isCompacting")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            is_quota_parked: value
                .get("isQuotaParked")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            worker_state: text("workerState"),
            has_subagents: value
                .get("hasRunningSubagents")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            last_activity: text("lastActivityAt"),
            modified: text("modified"),
        }
    }
}

/// Attach to a session and return its transcript messages (newest snapshot).
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn attach_messages(client: &Client, session: &str) -> std::io::Result<Vec<Value>> {
    attach_as(client, session, None, None)
}

/// `attach` under a client identity.
///
/// The supervisor routes `session_event` frames to the *connections* attached to
/// a session, and a connection is named by its `clientId`. Attaching anonymously
/// gets the snapshot and nothing else.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn attach_as(
    client: &Client,
    session: &str,
    client_id: Option<&str>,
    resume: Option<(String, u64)>,
) -> std::io::Result<Vec<Value>> {
    let mut payload = json!({"activeSessionId": session});
    if let (Some(id), Value::Object(map)) = (client_id, &mut payload) {
        map.insert("clientId".into(), Value::String(id.to_string()));
        map.insert("capabilities".into(), json!(["slim_attach"]));
    }
    // resume where this connection left off, so the daemon replays what we missed
    if let (Some((generation, sequence)), Value::Object(map)) = (resume, &mut payload) {
        map.insert(
            "resumeCursor".into(),
            json!({
                "activeSessionId": session,
                "generation": generation,
                "sequence": sequence,
                "eventSequence": sequence,
            }),
        );
    }
    let response = client.request("attach", payload, Duration::from_secs(20))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "attach failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let messages = response
        .pointer("/data/snapshot/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(messages)
}

/// `get_messages` → every message the daemon has *right now*.
///
/// Unlike the `attach` snapshot, which is frozen at attach time, this reads the
/// live session store, so it shows tool calls from the turn in flight.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn get_messages(client: &Client, session: &str) -> std::io::Result<Vec<Value>> {
    get_messages_within(client, session, Duration::from_secs(20))
}

/// `get_messages`, with an explicit response deadline.
///
/// The poll loop is a single thread: a tail that blocks for the default 20 s
/// also blocks the session list, the activity frames and the queue, and the UI
/// looks hung. Tails use a short deadline so a lost response costs a heartbeat,
/// not the whole client.
///
/// # Errors
/// Fails when the response does not arrive in time.
pub fn get_messages_within(
    client: &Client,
    session: &str,
    timeout: Duration,
) -> std::io::Result<Vec<Value>> {
    let response = client.request(
        "get_messages",
        json!({"activeSessionId": session}),
        timeout,
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "get_messages failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(response
        .pointer("/data/messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default())
}

/// The newest `limit` messages of the branch, by page cursor.
///
/// The typed `TranscriptPage::End` the terminal UI uses: the daemon resolves it
/// against the branch instead of shipping the whole session on every poll. The
/// returned cursor is a content fingerprint — it changes when the tail moves,
/// even if the row count does not.
///
/// # Errors
/// Fails on a transport error, or when the branch moved under the cursor.
pub fn get_tail(
    client: &Client,
    session: &str,
    limit: usize,
) -> std::io::Result<(Vec<Value>, String, bool)> {
    let response = client.request(
        "get_messages",
        json!({
            "activeSessionId": session,
            "page": {"from": "end", "limit": limit},
        }),
        Duration::from_secs(10),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "get_messages failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let data = response.get("data");
    let messages = data
        .and_then(|data| data.get("messages"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let last = data
        .and_then(|data| data.get("lastEntryId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let more = data
        .and_then(|data| data.get("more"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((messages, last, more))
}

/// `prompt` → hand a message to the session's agent.
///
/// Deliberately does **not** wait for the response: when the session already has
/// a turn in flight the daemon queues the message and answers only once it is
/// admitted (which can be minutes). The UI must not block on that — the turn's
/// progress arrives as `session_event` frames anyway.
///
/// # Errors
/// Fails only on transport errors (a closed pipe).
pub fn send_prompt(client: &Client, session: &str, message: &str) -> std::io::Result<String> {
    client.send(
        "prompt",
        json!({
            "activeSessionId": session,
            "message": message,
            // without this the daemon refuses a prompt sent mid-turn instead
            // of holding it: `queueIfBusy: false` never queues
            "queueIfBusy": true,
            "streamingBehavior": "followUp",
        }),
    )
}

/// `abort` → stop the running turn.
///
/// Fire-and-forget like [`send_prompt`]: the outcome shows up as a `stop`
/// event on the attached connection, not as a response.
///
/// # Errors
/// Fails only on transport errors (a closed pipe).
pub fn abort(client: &Client, session: &str) -> std::io::Result<String> {
    client.send("abort", json!({"activeSessionId": session}))
}

/// `prompt` with an explicit streaming behaviour: `steer` folds the message into
/// the running turn instead of queueing behind it.
///
/// # Errors
/// Fails only on transport errors (a closed pipe).
pub fn steer_prompt(
    client: &Client,
    session: &str,
    message: &str,
) -> std::io::Result<String> {
    client.send(
        "prompt",
        json!({
            "activeSessionId": session,
            "message": message,
            "streamingBehavior": "steer",
            "queueIfBusy": true,
        }),
    )
}

/// `get_queue` → the messages stacked behind the running turn.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn get_queue(client: &Client, session: &str) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let response = client.request(
        "get_queue",
        json!({"activeSessionId": session}),
        Duration::from_secs(10),
    )?;
    let texts = |key: &str| -> Vec<String> {
        response
            .pointer(&format!("/data/{key}"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| match item {
                        Value::String(text) => text.clone(),
                        other => other
                            .get("preview")
                            .or_else(|| other.get("message"))
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    Ok((texts("steering"), texts("followUp")))
}

/// Edit/delete/re-lane one waiting message.
///
/// The daemon addresses a queue row by `lane` + `slot` + the text the client
/// last saw, so a row that moved under an update is rejected rather than
/// silently rewriting somebody else's message. `mutation` is `delete`, `move`,
/// or `replace` (a replace that names the other lane moves it there).
///
/// # Errors
/// Fails on a transport error; a rejected mutation returns `"rejected"`.
pub fn mutate_queue(
    client: &Client,
    session: &str,
    lane: &str,
    slot: usize,
    expected: &str,
    mutation: Value,
) -> std::io::Result<String> {
    let response = client.request(
        "mutate_queued_message",
        json!({
            "activeSessionId": session,
            "lane": lane,
            "index": slot,
            "expectedText": expected,
            "mutation": mutation,
        }),
        Duration::from_secs(10),
    )?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "mutate_queued_message failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    Ok(response
        .pointer("/data/status")
        .and_then(Value::as_str)
        .unwrap_or("rejected")
        .to_string())
}

/// A session's live numbers, as the right panel shows them.
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub context_tokens: u64,
    pub context_window: u64,
    pub percent: f64,
    pub messages: u64,
    pub tool_calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read: u64,
    pub cost: f64,
}

impl SessionStats {
    #[must_use]
    pub fn from_value(value: &Value) -> Self {
        let number = |pointer: &str| {
            value
                .pointer(pointer)
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        Self {
            context_tokens: number("/contextUsage/tokens"),
            context_window: number("/contextUsage/contextWindow"),
            percent: value
                .pointer("/contextUsage/percent")
                .and_then(Value::as_f64)
                .unwrap_or(0.0),
            messages: number("/totalMessages"),
            tool_calls: number("/toolCalls"),
            input_tokens: number("/tokens/input"),
            output_tokens: number("/tokens/output"),
            cache_read: number("/tokens/cacheRead"),
            cost: value.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
        }
    }
}

/// `get_session_stats` → context usage and token totals.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn session_stats(client: &Client, session: &str) -> std::io::Result<SessionStats> {
    let response = client.request(
        "get_session_stats",
        json!({"activeSessionId": session}),
        Duration::from_secs(10),
    )?;
    Ok(response.get("data").map(SessionStats::from_value).unwrap_or_default())
}

/// `set_model` → switch the session's provider/model.
///
/// # Errors
/// Fails only on transport errors (a rejected model shows up in the status).
pub fn set_model(
    client: &Client,
    session: &str,
    provider: &str,
    model_id: &str,
) -> std::io::Result<String> {
    client.send(
        "set_model",
        json!({"activeSessionId": session, "provider": provider, "modelId": model_id}),
    )
}

/// One agent stream event → the TUI's loader vocabulary.
///
/// Mirrors `track_stream_activity` in pa-tui: the event *kind*, not the delta,
/// is what names the state.
#[must_use]
pub fn activity_label(event: &Value) -> Option<&'static str> {
    match event.get("type").and_then(Value::as_str)? {
        "thinking_start" | "thinking_delta" => Some("Thinking"),
        "text_start" | "text_delta" => Some("Writing"),
        "toolcall_start" | "toolcall_delta" => Some("Writing code"),
        "tool_result" | "toolcall_end" => Some("Waiting"),
        "message_start" => Some("Thinking"),
        "message_end" | "stop" => Some("Waiting"),
        _ => None,
    }
}

/// `list` → every session the daemon knows.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn list_sessions(client: &Client) -> std::io::Result<Vec<SessionRow>> {
    let response = client.request("list", json!({"all": true}), Duration::from_secs(10))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "list failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let rows = response
        .pointer("/data/sessions")
        .and_then(Value::as_array)
        .map(|sessions| sessions.iter().map(SessionRow::from_value).collect())
        .unwrap_or_default();
    Ok(rows)
}

pub use std::sync::Arc as Shared;

#[cfg(test)]
mod wire_tests {
    use super::*;

    fn parses(value: Value) -> bool {
        serde_json::from_value::<pa_types::daemon::DaemonCommand>(value).is_ok()
    }

    #[test]
    fn every_command_we_send_exists_on_the_wire() {
        assert!(parses(json!({"type": "attach", "activeSessionId": "s", "clientId": "c",
                              "capabilities": ["slim_attach"]})));
        assert!(parses(json!({"type": "prompt", "activeSessionId": "s", "message": "m"})));
        assert!(parses(json!({"type": "prompt", "activeSessionId": "s", "message": "m",
                              "streamingBehavior": "steer"})));
        assert!(parses(json!({"type": "abort", "activeSessionId": "s"})));
        assert!(parses(json!({"type": "get_messages", "activeSessionId": "s"})));
        assert!(parses(json!({"type": "get_queue", "activeSessionId": "s"})));
        assert!(parses(json!({"type": "get_session_stats", "activeSessionId": "s"})));
        assert!(parses(json!({"type": "get_rlm_children", "activeSessionId": "s"})));
        assert!(parses(json!({"type": "rename", "activeSessionId": "s", "name": "n"})));
        assert!(parses(json!({"type": "set_model", "activeSessionId": "s",
                              "provider": "openrouter", "modelId": "x"})));
        assert!(parses(json!({"type": "clear_queue", "activeSessionId": "s"})));
    }

    #[test]
    fn a_renamed_field_is_caught() {
        assert!(!parses(json!({"type": "prompt", "session": "s", "message": "m"})));
    }
}
