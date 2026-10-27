//! Daemon wire client.
//!
//! Two things the wire forces:
//! 1. The supervisor accepts only `type: "command"` envelopes.
//! 2. A synchronous Windows pipe handle cannot serve a blocking reader thread
//!    and a writer at once (they deadlock), so one thread owns the pipe and
//!    polls with `PeekNamedPipe` before reading.

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const DAEMON_PIPE: &str = r"\\.\pipe\prime-agent-daemon";
pub const PROTOCOL_NAME: &str = "prime-agent.daemon";
pub const PROTOCOL_VERSION: u64 = 7;

pub type Frame = Value;

/// Handle to the daemon: commands go in, frames come out.
pub struct Client {
    out: Sender<String>,
    frames: Mutex<Receiver<Frame>>,
    backlog: Mutex<VecDeque<Frame>>,
    next_id: AtomicU64,
    dead: AtomicBool,
    pub hello: Frame,
}

impl Client {
    /// Connect, read the hello frame, and start the poller thread.
    ///
    /// # Errors
    /// Fails when the pipe cannot be opened or the hello never arrives.
    pub fn connect() -> std::io::Result<Self> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(DAEMON_PIPE)?;
        let mut buffer = Vec::new();
        let hello = read_frame(&mut file, &mut buffer, Duration::from_secs(5))
            .ok_or_else(|| std::io::Error::other("no daemon hello"))?;

        let (out_tx, out_rx) = channel::<String>();
        let (frame_tx, frame_rx) = channel::<Frame>();
        std::thread::spawn(move || poll_loop(file, buffer, out_rx, frame_tx));

        Ok(Self {
            out: out_tx,
            frames: Mutex::new(frame_rx),
            backlog: Mutex::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            dead: AtomicBool::new(false),
            hello,
        })
    }

    /// Whether the daemon connection is still usable.
    ///
    /// The poller thread ends when the pipe breaks (daemon restart, shutdown),
    /// and nothing else surfaces that: frames just stop arriving.
    pub fn alive(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }

    /// Send one command; returns its id.
    ///
    /// # Errors
    /// Fails when the poller thread is gone.
    pub fn send(&self, ty: &str, payload: Value) -> std::io::Result<String> {
        let id = format!("ui-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        let mut command = payload;
        if let Value::Object(map) = &mut command {
            map.insert("id".into(), Value::String(id.clone()));
            map.insert("type".into(), Value::String(ty.to_string()));
        }
        let frame = json!({
            "type": "command",
            "id": id,
            "protocol": {"name": PROTOCOL_NAME, "version": PROTOCOL_VERSION},
            "command": command,
        });
        // every command is checked against the enum the supervisor parses, so a
        // renamed field fails here instead of silently doing nothing
        match serde_json::from_value::<pa_types::daemon::DaemonCommand>(command.clone()) {
            Ok(_) => {}
            Err(error) => {
                let report = format!("command {ty} does not match DaemonCommand: {error}");
                if std::env::var("PA_STRICT").is_ok() {
                    return Err(std::io::Error::other(report));
                }
                eprintln!("[wire] {report}");
            }
        }
        let mut line = frame.to_string();
        line.push('\n');
        self.out
            .send(line)
            .map_err(|_| std::io::Error::other("daemon poller stopped"))?;
        Ok(id)
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
                // the poller stopped: the pipe died with it
                self.dead.store(true, Ordering::Relaxed);
                None
            }
        }
    }

    /// Send a command and wait for the response with its id.
    ///
    /// # Errors
    /// Fails when the response does not arrive within `timeout`.
    pub fn request(&self, ty: &str, payload: Value, timeout: Duration) -> std::io::Result<Frame> {
        let id = self.send(ty, payload)?;
        let deadline = Instant::now() + timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let Some(frame) = self.recv(left) else {
                return Err(std::io::Error::other("timed out waiting for response"));
            };
            let is_response = frame.get("type").and_then(Value::as_str) == Some("response");
            let matches = frame.get("id").and_then(Value::as_str) == Some(id.as_str());
            if is_response && matches {
                return Ok(frame);
            }
            self.backlog.lock().expect("backlog").push_back(frame);
        }
    }
}

fn poll_loop(
    mut file: std::fs::File,
    mut buffer: Vec<u8>,
    commands: Receiver<String>,
    frames: Sender<Frame>,
) {
    loop {
        loop {
            match commands.try_recv() {
                Ok(line) => {
                    if file.write_all(line.as_bytes()).and_then(|()| file.flush()).is_err() {
                        return;
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        // a failed peek means the daemon closed the pipe: stop the poller so
        // the frames channel disconnects and the worker reconnects
        let Some(waiting) = pending(&file) else { return };
        if waiting > 0 {
            while let Some(frame) = read_frame(&mut file, &mut buffer, Duration::from_millis(50)) {
                if frames.send(frame).is_err() {
                    return;
                }
            }
        } else {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
}

/// Bytes waiting on the pipe, or `None` when the handle is gone.
///
/// The distinction matters: a daemon restart shows up here as a failed peek.
/// Reporting that as "0 bytes waiting" made a dead pipe look exactly like a
/// quiet one, so the app stayed attached to a corpse and never reconnected.
fn pending(file: &std::fs::File) -> Option<u32> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::Pipes::PeekNamedPipe;
    let mut available = 0u32;
    let ok = unsafe {
        PeekNamedPipe(
            file.as_raw_handle() as _,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            &mut available,
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

/// Read one newline-delimited frame, if one is available within `timeout`.
fn read_frame(file: &mut std::fs::File, buffer: &mut Vec<u8>, timeout: Duration) -> Option<Frame> {
    if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=position).collect();
        return serde_json::from_slice(&line[..line.len() - 1]).ok();
    }
    let deadline = Instant::now() + timeout;
    loop {
        match pending(file) {
            None => return None,
            Some(0) => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(4));
                continue;
            }
            Some(_) => {}
        }
        let mut chunk = [0u8; 65536];
        match file.read(&mut chunk) {
            Ok(0) | Err(_) => return None,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
                    let line: Vec<u8> = buffer.drain(..=position).collect();
                    return serde_json::from_slice(&line[..line.len() - 1]).ok();
                }
            }
        }
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
    let response = client.request(
        "get_messages",
        json!({"activeSessionId": session}),
        Duration::from_secs(20),
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
    client.send("prompt", json!({"activeSessionId": session, "message": message}))
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
