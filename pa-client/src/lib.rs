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
use std::sync::atomic::{AtomicU64, Ordering};
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
            hello,
        })
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
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
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
        if pending(&file) > 0 {
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

/// Bytes waiting on the pipe (0 when the handle is gone).
fn pending(file: &std::fs::File) -> u32 {
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
    if ok == 0 { 0 } else { available }
}

/// Read one newline-delimited frame, if one is available within `timeout`.
fn read_frame(file: &mut std::fs::File, buffer: &mut Vec<u8>, timeout: Duration) -> Option<Frame> {
    if let Some(position) = buffer.iter().position(|byte| *byte == b'\n') {
        let line: Vec<u8> = buffer.drain(..=position).collect();
        return serde_json::from_slice(&line[..line.len() - 1]).ok();
    }
    let deadline = Instant::now() + timeout;
    loop {
        if pending(file) == 0 {
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(4));
            continue;
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
    attach_as(client, session, None)
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
) -> std::io::Result<Vec<Value>> {
    let mut payload = json!({"activeSessionId": session});
    if let (Some(id), Value::Object(map)) = (client_id, &mut payload) {
        map.insert("clientId".into(), Value::String(id.to_string()));
        map.insert("capabilities".into(), json!(["slim_attach"]));
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
