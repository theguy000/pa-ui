//! Daemon wire client, on top of `pa-tui`'s transport.
//!
//! This crate used to own a hand-rolled pipe reader: one thread, `PeekNamedPipe`,
//! 8ms naps. It could not keep up with the daemon's event queue (which drops
//! frames for a slow client), so turns arrived truncated. The connection now
//! comes from `pa_tui::daemon_client` - the client the terminal UI ships with -
//! and this crate keeps only the typed helpers the UI calls.

mod activity;
mod models;
mod wire;

pub mod commands;
pub mod workspace;

pub use activity::*;
pub use models::*;
pub use pa_types::daemon::{
    BranchInfo, HarnessEntryWire, HarnessKindWire, HarnessScopeWire,
    RefinementHistoryItemWire, WorkspaceBinding, WorkspaceKind, WorkspaceRecord,
    WorktreeInfo,
};
pub use workspace::*;
#[cfg(test)]
use models::supported_levels;
use pa_types::daemon::agent_roster::{classify_session_roster_status, AgentRosterStatus};
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

    /// Upgrade this connection with a direct worker link for the session.
    /// Returns `false` when the daemon does not offer it; supervisor routing stays.
    pub fn upgrade_direct(&self, active_session_id: &str) -> bool {
        self.rt
            .block_on(self.inner.upgrade_direct(active_session_id))
            .unwrap_or(false)
    }

    /// Drop any direct worker link and keep supervisor routing.
    pub fn drop_direct(&self) {
        self.inner.drop_direct();
    }

    #[must_use]
    pub fn direct_session_id(&self) -> Option<String> {
        self.inner.direct_session_id()
    }

    /// Whether the daemon connection is still usable.
    pub fn alive(&self) -> bool {
        !self.dead.load(Ordering::Relaxed)
    }

    /// Close the transport. A blocked frame reader wakes when the pump closes.
    pub fn close(&self) {
        self.dead.store(true, Ordering::Relaxed);
        self.inner.close();
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
    async fn request_async(
        &self,
        ty: &str,
        payload: Value,
        timeout: Duration,
    ) -> std::io::Result<Frame> {
        let command = typed_command(ty, payload)?;
        let millis = u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX);
        let response = self
            .inner
            .request_with_timeout(command, millis)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok(serde_json::to_value(response).unwrap_or(Value::Null))
    }

    pub fn request(
        &self,
        ty: &str,
        payload: Value,
        timeout: Duration,
    ) -> std::io::Result<Frame> {
        self.rt.block_on(self.request_async(ty, payload, timeout))
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

    /// Next frame, backlog first. Blocks until a frame arrives or the pump closes.
    pub fn recv_blocking(&self) -> Option<Frame> {
        if let Some(frame) = self.backlog.lock().expect("backlog").pop_front() {
            return Some(frame);
        }
        let rx = self.frames.lock().expect("frames");
        match rx.recv() {
            Ok(frame) => Some(frame),
            Err(_) => {
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
        wire::daemon_client::DaemonClientEvent::DirectLinkLost { active_session_id } => {
            Some(json!({
                "type": "direct_link_lost",
                "activeSessionId": active_session_id,
            }))
        }
        wire::daemon_client::DaemonClientEvent::SessionListItem {
            session,
            request_id,
        } => Some(json!({
            "type": "session_list_item",
            "session": session,
            "requestId": request_id,
        })),
        wire::daemon_client::DaemonClientEvent::SessionListProgress { loaded, total } => {
            Some(json!({
                "type": "session_list_progress",
                "loaded": loaded,
                "total": total,
            }))
        }
        wire::daemon_client::DaemonClientEvent::DaemonClosing { reason, update } => {
            let payload = update.map(|update| {
                json!({
                    "resume": true,
                    "updateId": update.update_id,
                    "estSeconds": update.est_seconds,
                    "sessions": update.sessions,
                })
            });
            Some(json!({
                "type": "daemon_closing",
                "reason": reason,
                "payload": payload,
            }))
        }
        wire::daemon_client::DaemonClientEvent::SideQuestionEvent {
            active_session_id,
            event,
        } => Some(json!({
            "type": "side_question_event",
            "activeSessionId": active_session_id,
            "event": event,
        })),
        wire::daemon_client::DaemonClientEvent::RosterUpdate {
            changed,
            removed,
            resync,
        } => Some(json!({
            "type": "roster_update",
            "changed": changed,
            "removed": removed,
            "resync": resync,
        })),
        wire::daemon_client::DaemonClientEvent::HeartbeatsChanged => {
            Some(json!({"type": "heartbeats_changed"}))
        }
        wire::daemon_client::DaemonClientEvent::ModelCatalogChanged => {
            Some(json!({"type": "model_catalog_changed"}))
        }
        wire::daemon_client::DaemonClientEvent::WorkspacesChanged => {
            Some(json!({"type": "workspaces_changed"}))
        }
        wire::daemon_client::DaemonClientEvent::HarnessChanged {
            scope,
            workspace_id,
            active_session_id,
        } => Some(json!({
            "type": "harness_changed",
            "scope": scope,
            "workspaceId": workspace_id,
            "activeSessionId": active_session_id,
        })),
        wire::daemon_client::DaemonClientEvent::SessionBinding {
            previous_active_session_id,
            active_session_id,
        } => Some(json!({
            "type": "session_binding",
            "previousActiveSessionId": previous_active_session_id,
            "activeSessionId": active_session_id,
        })),
    }
}


/// The session rows the left sidebar renders.
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
    pub model_name: String,
    pub model_provider: String,
    pub thinking: String,
    /// Where the session is saved. A row with no worker is addressed and
    /// deleted by this path, not by its id. `list` sends it as `sessionFile`
    /// and the saved catalog sends it as `path`.
    pub session_file: String,
    pub messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub is_streaming: bool,
    /// An active turn: streaming, compacting, or a parked message waiting to run.
    /// The daemon's roster classifies a session on this key.
    pub is_session_active: bool,
    pub is_running_tools: bool,
    pub is_bash_running: bool,
    pub is_compacting: bool,
    pub is_quota_parked: bool,
    pub worker_state: String,
    pub has_subagents: bool,
    /// The parent's `sessionId` for a subagent's own session: the link the
    /// family cost rolls up.
    pub parent: String,
    /// The parent's live `activeSessionId`, present on queued children that
    /// have not been rebound to a durable parent session yet.
    pub parent_active: String,
    /// `rlmDepth` from the roster: 0 for a top-level Session, >0 for a spawn.
    pub depth: u64,
    pub last_activity: String,
    pub modified: String,
    pub workspace_binding: Option<WorkspaceBinding>,
}

impl SessionRow {
    /// Whether a worker holds this session. `list` re-sends every saved session
    /// off disk with `lifecycle: "live"` whenever it has a message, so the
    /// lifecycle alone never says a worker exists; the daemon stamps
    /// `workerState` on the rows it routed to a resident.
    #[must_use]
    pub fn is_resident(&self) -> bool {
        !self.worker_state.is_empty()
    }

    /// The daemon's status formula (`classify_session_roster_status` in
    /// `pa-types`): a worker that holds the session and is busy is running, one
    /// that holds it and is quiet is idle, and a session no worker holds is
    /// inactive. An archived lifecycle is inactive whatever the other keys say,
    /// since a session set aside is not one a worker is about to pick up.
    #[must_use]
    pub fn roster_status(&self) -> &'static str {
        match classify_session_roster_status(
            self.is_resident() && self.lifecycle != "archived",
            &self.activity,
            self.is_session_active,
            false,
        ) {
            AgentRosterStatus::Running => "Running",
            AgentRosterStatus::Idle => "Idle",
            AgentRosterStatus::Inactive => "Inactive",
        }
    }

    /// What the agent is doing right now, in the TUI's vocabulary.
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
        if self.is_session_active {
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
        // the resident summary carries `sessionFile`; the saved catalog row
        // carries the same path as `path`, and it is the one a done row needs
        let saved_as = text("sessionFile");
        let session_file = if saved_as.is_empty() { text("path") } else { saved_as };
        // `list_saved_sessions` rows carry `id` and `path`, not the live
        // roster's `activeSessionId`/`sessionId`. Use the saved id for both so
        // done rows have a stable identity instead of collapsing to an empty
        // key in the sidebar.
        let saved_id = text("id");
        let active_id = text("activeSessionId");
        let id = if active_id.is_empty() { saved_id.clone() } else { active_id };
        let live_session_id = text("sessionId");
        let session_id = if live_session_id.is_empty() {
            saved_id
        } else {
            live_session_id
        };
        let named = text("sessionName");
        let name = if !named.is_empty() {
            named
        } else {
            let explicit = text("name");
            if explicit.is_empty() {
                text("firstMessage").chars().take(60).collect::<String>()
            } else {
                explicit
            }
        };
        Self {
            id,
            session_id,
            name,
            cwd: text("cwd"),
            activity: text("activity"),
            lifecycle: text("lifecycle"),
            // the wire `model` is the whole model object, not a string: keep
            // the id, and the provider/name the chip needs to label it
            model: value
                .pointer("/model/id")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| text("model")),
            model_name: value
                .pointer("/model/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            model_provider: value
                .pointer("/model/provider")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            thinking: text("thinkingLevel"),
            session_file,
            messages: number("messageCount"),
            input_tokens: usage_num("inputTokens"),
            output_tokens: usage_num("outputTokens"),
            cost: usage.get("cost").and_then(Value::as_f64).unwrap_or(0.0),
            is_streaming: value.get("isStreaming").and_then(Value::as_bool).unwrap_or(false),
            is_session_active: value
                .get("isSessionActive")
                .and_then(Value::as_bool)
                .unwrap_or(false),
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
            parent: text("parentSessionId"),
            parent_active: text("parentActiveSessionId"),
            depth: value.get("rlmDepth").and_then(Value::as_u64).unwrap_or(0),
            last_activity: text("lastActivityAt"),
            modified: text("modified"),
            workspace_binding: value
                .get("workspaceBinding")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
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

/// Complete initial view returned by a single windowed `attach` round-trip.
#[derive(Debug, Clone, Default)]
pub struct AttachSnapshot {
    pub active_session_id: String,
    pub session_id: String,
    pub messages: Vec<Value>,
    pub first_entry: String,
    pub more: bool,
    pub state: Option<SessionState>,
    pub stats: Option<SessionStats>,
    pub steer: Vec<String>,
    pub follow_up: Vec<String>,
}

fn parse_action_lane(state: &Value, lane: &str) -> Vec<String> {
    state
        .pointer(&format!("/sessionActions/{lane}"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str().map(str::to_string).or_else(|| {
                        item.get("text")
                            .or_else(|| item.get("preview"))
                            .or_else(|| item.get("message"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `attach` with `windowed_snapshot` + `slim_attach`: returns the tail transcript,
/// resolved `activeSessionId`, durable `sessionId`, live model/thinking state,
/// context usage, and queued lanes in one round-trip.
///
/// # Errors
/// Fails on transport errors or a non-success response.
pub fn attach_snapshot(
    client: &Client,
    session: &str,
    client_id: Option<&str>,
) -> std::io::Result<AttachSnapshot> {
    let mut payload = json!({"activeSessionId": session});
    if let Value::Object(map) = &mut payload {
        if let Some(id) = client_id {
            map.insert("clientId".into(), Value::String(id.to_string()));
        }
        map.insert(
            "capabilities".into(),
            json!([
                "attach_snapshot",
                "event_sequence",
                "slim_attach",
                "windowed_snapshot"
            ]),
        );
    }
    let response = client.request("attach", payload, Duration::from_secs(20))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "attach failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    let data = response.get("data").unwrap_or(&Value::Null);
    let snapshot = data.get("snapshot").unwrap_or(&Value::Null);
    let active_session_id = data
        .get("activeSessionId")
        .or_else(|| snapshot.get("activeSessionId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(session)
        .to_string();
    let messages = snapshot
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let history_before = snapshot
        .get("historyBeforeEntryId")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let more = !history_before.is_empty();
    let state_val = snapshot.get("state").filter(|v| v.is_object());
    let session_id = state_val
        .and_then(|s| s.get("sessionId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let state = state_val.map(SessionState::from_value);
    let stats = state_val
        .filter(|s| s.get("contextUsage").is_some_and(Value::is_object))
        .map(|s| {
            let mut st = SessionStats::from_value(s);
            if st.messages == 0 {
                st.messages = s.get("messageCount").and_then(Value::as_u64).unwrap_or(0);
            }
            st
        });
    let steer = state_val
        .map(|s| parse_action_lane(s, "steering"))
        .unwrap_or_default();
    let follow_up = state_val
        .map(|s| parse_action_lane(s, "followUps"))
        .unwrap_or_default();

    Ok(AttachSnapshot {
        active_session_id,
        session_id,
        messages,
        first_entry: history_before,
        more,
        state,
        stats,
        steer,
        follow_up,
    })
}

/// Fire-and-forget `detach` when switching away from a live session so old
/// session frames stop streaming onto this connection.
pub fn detach_session(client: &Client, session: &str) {
    if !session.is_empty() {
        let _ = client.send("detach", json!({"activeSessionId": session}));
    }
}

/// Incremental payload delivered as each concurrent post-attach request settles.
#[derive(Debug)]
pub enum SessionExtra {
    Activity {
        subagents: usize,
        shells: usize,
        heartbeats: usize,
    },
    Stats(SessionStats),
    Models(Vec<ModelEntry>),
    Sessions(Vec<SessionRow>),
    Queue {
        steer: Vec<String>,
        follow_up: Vec<String>,
    },
    Tail(Page),
}

/// Run post-attach metadata requests concurrently over the multiplexed daemon
/// pipe and invoke `emit` immediately as each independent piece completes so
/// slow `list` calls never stall activity, context stats, or models.
pub fn fetch_session_extras_concurrent<F>(
    client: &Client,
    active_session_id: &str,
    session_id: &str,
    need_models: bool,
    need_sessions: bool,
    emit: F,
) where
    F: Fn(SessionExtra) + Sync,
{
    let active_id = active_session_id.to_string();
    let durable_id = session_id.to_string();
    let emit_ref = &emit;

    client.rt.block_on(async {
        let activity_fut = async {
            let (roster_res, shells_res, hb_res) = tokio::join!(
                client.request_async("roster_subscribe", json!({}), Duration::from_secs(3)),
                client.request_async(
                    "list_kernel_bash",
                    json!({"activeSessionId": active_id}),
                    Duration::from_secs(3),
                ),
                client.request_async("heartbeats_list", json!({}), Duration::from_secs(3)),
            );
            let subagents = roster_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.pointer("/data/roster").and_then(Value::as_array).cloned())
                .map(|roster| running_subagents(&roster, &active_id, &durable_id))
                .unwrap_or(0);
            let shells = shells_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.get("data").cloned())
                .map(|data| running_shell_count(&data))
                .unwrap_or(0);
            let heartbeats = hb_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.get("data").cloned())
                .map(|data| heartbeat_count(&data, &active_id, &durable_id))
                .unwrap_or(0);
            emit_ref(SessionExtra::Activity {
                subagents,
                shells,
                heartbeats,
            });
        };

        let stats_fut = async {
            if let Ok(resp) = client
                .request_async(
                    "get_session_stats",
                    json!({"activeSessionId": active_id}),
                    Duration::from_secs(6),
                )
                .await
            {
                if resp.get("success").and_then(Value::as_bool) != Some(false) {
                    if let Some(data) = resp.get("data") {
                        emit_ref(SessionExtra::Stats(SessionStats::from_value(data)));
                    }
                }
            }
        };

        let models_fut = async {
            if need_models {
                if let Ok(resp) = client
                    .request_async(
                        "get_model_catalog",
                        json!({"activeSessionId": active_id}),
                        Duration::from_secs(15),
                    )
                    .await
                {
                    if let Ok(models) = models::parse_model_catalog_response(&resp) {
                        emit_ref(SessionExtra::Models(models));
                    }
                }
            }
        };

        let sessions_fut = async {
            if need_sessions {
                if let Ok(resp) = client
                    .request_async("list", json!({"all": true}), Duration::from_secs(10))
                    .await
                {
                    if resp.get("success").and_then(Value::as_bool) == Some(true) {
                        let rows = resp
                            .pointer("/data/sessions")
                            .and_then(Value::as_array)
                            .map(|sessions| sessions.iter().map(SessionRow::from_value).collect())
                            .unwrap_or_default();
                        emit_ref(SessionExtra::Sessions(rows));
                    }
                }
            }
        };

        tokio::join!(activity_fut, stats_fut, models_fut, sessions_fut);
    });
}

/// Refresh tail transcript, stats, queue, activity, and session list in parallel
/// (used when a turn finishes so context % and activity pills stay current).
pub fn reload_session_parallel<F>(
    client: &Client,
    active_session_id: &str,
    session_id: &str,
    limit: usize,
    emit: F,
) where
    F: Fn(SessionExtra) + Sync,
{
    let active_id = active_session_id.to_string();
    let durable_id = session_id.to_string();
    let emit_ref = &emit;

    client.rt.block_on(async {
        let page_fut = async {
            if let Ok(resp) = client
                .request_async(
                    "get_messages",
                    json!({
                        "activeSessionId": active_id,
                        "page": {"from": "end", "limit": limit},
                    }),
                    Duration::from_secs(10),
                )
                .await
            {
                if resp.get("success").and_then(Value::as_bool) == Some(true) {
                    let data = resp.get("data").unwrap_or(&Value::Null);
                    let page = Page {
                        messages: data
                            .get("messages")
                            .and_then(Value::as_array)
                            .cloned()
                            .unwrap_or_default(),
                        first_entry: data
                            .get("firstEntryId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        last_entry: data
                            .get("lastEntryId")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        more: data.get("more").and_then(Value::as_bool).unwrap_or(false),
                    };
                    emit_ref(SessionExtra::Tail(page));
                }
            }
        };

        let queue_fut = async {
            if let Ok(resp) = client
                .request_async(
                    "get_queue",
                    json!({"activeSessionId": active_id}),
                    Duration::from_secs(6),
                )
                .await
            {
                let texts = |key: &str| -> Vec<String> {
                    resp.pointer(&format!("/data/{key}"))
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
                                .filter(|text| !text.is_empty())
                                .collect()
                        })
                        .unwrap_or_default()
                };
                emit_ref(SessionExtra::Queue {
                    steer: texts("steer"),
                    follow_up: texts("followUp"),
                });
            }
        };

        let stats_fut = async {
            if let Ok(resp) = client
                .request_async(
                    "get_session_stats",
                    json!({"activeSessionId": active_id}),
                    Duration::from_secs(6),
                )
                .await
            {
                if resp.get("success").and_then(Value::as_bool) != Some(false) {
                    if let Some(data) = resp.get("data") {
                        emit_ref(SessionExtra::Stats(SessionStats::from_value(data)));
                    }
                }
            }
        };

        let activity_fut = async {
            let (roster_res, shells_res, hb_res) = tokio::join!(
                client.request_async("roster_subscribe", json!({}), Duration::from_secs(3)),
                client.request_async(
                    "list_kernel_bash",
                    json!({"activeSessionId": active_id}),
                    Duration::from_secs(3),
                ),
                client.request_async("heartbeats_list", json!({}), Duration::from_secs(3)),
            );
            let subagents = roster_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.pointer("/data/roster").and_then(Value::as_array).cloned())
                .map(|roster| running_subagents(&roster, &active_id, &durable_id))
                .unwrap_or(0);
            let shells = shells_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.get("data").cloned())
                .map(|data| running_shell_count(&data))
                .unwrap_or(0);
            let heartbeats = hb_res
                .ok()
                .filter(|r| r.get("success").and_then(Value::as_bool) == Some(true))
                .and_then(|r| r.get("data").cloned())
                .map(|data| heartbeat_count(&data, &active_id, &durable_id))
                .unwrap_or(0);
            emit_ref(SessionExtra::Activity {
                subagents,
                shells,
                heartbeats,
            });
        };

        let sessions_fut = async {
            if let Ok(resp) = client
                .request_async("list", json!({"all": true}), Duration::from_secs(10))
                .await
            {
                if resp.get("success").and_then(Value::as_bool) == Some(true) {
                    let rows = resp
                        .pointer("/data/sessions")
                        .and_then(Value::as_array)
                        .map(|sessions| sessions.iter().map(SessionRow::from_value).collect())
                        .unwrap_or_default();
                    emit_ref(SessionExtra::Sessions(rows));
                }
            }
        };

        tokio::join!(page_fut, queue_fut, stats_fut, activity_fut, sessions_fut);
    });
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
/// One page of the transcript, walking backwards from `before`.
///
/// `before` is a **row id** (`firstEntryId` of the page already held), not a
/// timestamp: the daemon walks its branch from that anchor, so a page costs the
/// page and not the session. `None` means "the newest page", which is `get_tail`.
pub fn get_page(
    client: &Client,
    session: &str,
    before: Option<&str>,
    limit: usize,
) -> std::io::Result<Page> {
    let mut page = json!({"from": "end", "limit": limit});
    if let Some(before) = before {
        page["before"] = json!(before);
    }
    let response = client.request(
        "get_messages",
        json!({
            "activeSessionId": session,
            "page": page,
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
    let text = |key: &str| {
        data.and_then(|data| data.get(key))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Ok(Page {
        messages: data
            .and_then(|data| data.get("messages"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        first_entry: text("firstEntryId"),
        last_entry: text("lastEntryId"),
        more: data
            .and_then(|data| data.get("more"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// One page of a session's transcript.
#[derive(Debug, Clone, Default)]
pub struct Page {
    /// oldest first, as the store returns them
    pub messages: Vec<Value>,
    /// id of the oldest row in the page: the cursor for the page before it
    pub first_entry: String,
    /// id of the newest row in the page
    pub last_entry: String,
    /// whether the store has older rows behind this page
    pub more: bool,
}

/// The newest page: the tail without a cursor.
pub fn get_tail(
    client: &Client,
    session: &str,
    limit: usize,
) -> std::io::Result<(Vec<Value>, String, bool)> {
    let page = get_page(client, session, None, limit)?;
    Ok((page.messages, page.last_entry, page.more))
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
pub fn send_prompt(
    client: &Client,
    session: &str,
    message: &str,
    images: &[Value],
) -> std::io::Result<String> {
    let mut payload = json!({
        "activeSessionId": session,
        "message": message,
        // without this the daemon refuses a prompt sent mid-turn instead
        // of holding it: `queueIfBusy: false` never queues
        "queueIfBusy": true,
        "streamingBehavior": "followUp",
    });
    if !images.is_empty() {
        // `{type:"image", data:<base64>, mimeType}`; the daemon drops entries
        // missing either half rather than failing the turn.
        payload["images"] = Value::Array(images.to_vec());
    }
    client.send("prompt", payload)
}

/// Send a prompt and **wait for the daemon's verdict**.
///
/// The TUI's submit does this, and it is the difference that matters: a
/// fire-and-forget send cannot tell "accepted" from "refused", so a refusal
/// looks exactly like a message that vanished. `Err` carries the daemon's own
/// reason, word for word.
///
/// # Errors
/// Fails when the daemon refuses the prompt or does not answer in time.
pub fn send_prompt_checked(
    client: &Client,
    session: &str,
    message: &str,
    images: &[Value],
) -> Result<(), String> {
    let mut payload = json!({
        "activeSessionId": session,
        "message": message,
        "queueIfBusy": true,
        "streamingBehavior": "followUp",
    });
    if !images.is_empty() {
        payload["images"] = Value::Array(images.to_vec());
    }
    let response = client
        .request("prompt", payload, Duration::from_secs(30))
        .map_err(|error| error.to_string())?;
    if response.get("success").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    Err(response
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("the daemon refused the message")
        .to_string())
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

/// `abort_and_send_queued` → stop the run and keep the queue flowing.
///
/// This is what the TUI's abort does when the daemon advertises the capability
/// (`abort_turn` in `session_ui/apply.rs`): the run stops, the queued messages
/// stay queued, and the pump delivers the next one at the settled boundary. A
/// plain `abort` would strand them until something else nudged the session. A
/// daemon without the capability gets the plain abort instead.
///
/// # Errors
/// Fails only on transport errors.
pub fn stop(client: &Client, session: &str) -> std::io::Result<String> {
    let capable = client
        .hello
        .get("serverCapabilities")
        .and_then(Value::as_array)
        .is_some_and(|capabilities| {
            capabilities
                .iter()
                .any(|entry| entry.as_str() == Some("abort_and_send_queued"))
        });
    if capable {
        client.send("abort_and_send_queued", json!({"activeSessionId": session}))
    } else {
        client.send("abort", json!({"activeSessionId": session}))
    }
}

/// `kill` → stop the session and release its worker. This is the TUI's ctrl+x
/// on an idle session: the worker writes `session_state {status: archived}` on
/// its way out, and the next status poll lists the session as set aside with no
/// worker holding it.
///
/// It never removes the file. Deleting is a second, separate command, which is
/// why the sidebar offers them as two items and never as one.
///
/// # Errors
/// Fails only on transport errors (a closed pipe). The daemon's answer arrives
/// as a `kill` response on the attached connection.
pub fn mark_done(client: &Client, session: &str) -> std::io::Result<String> {
    client.send("kill", json!({"activeSessionId": session}))
}

/// `rename_saved_session` → rename a session by its file.
///
/// This is the command a row with no worker takes. `rename` is not: it resolves
/// the id against a resident session and starts a worker when there is none, so
/// renaming a saved row through it would bring the session back to life.
///
/// # Errors
/// Fails only on transport errors.
pub fn rename_saved_session(
    client: &Client,
    session_path: &str,
    name: &str,
) -> std::io::Result<String> {
    client.send(
        "rename_saved_session",
        json!({"sessionPath": session_path, "name": name}),
    )
}

/// `delete_saved_session` → delete a saved session's file and its artifacts.
///
/// The daemon refuses this for a file some worker currently holds, which is the
/// half the sidebar keeps on its side by never offering Delete on a live row.
///
/// # Errors
/// Fails only on transport errors.
pub fn delete_saved_session(client: &Client, session_path: &str) -> std::io::Result<String> {
    client.send("delete_saved_session", json!({"sessionPath": session_path}))
}

/// `export_html` → the standalone viewer file for the session, written to
/// `output_path`. The answer carries the written path under `path`.
///
/// This is the export the TUI's `/share` uploads; a session with no file on disk
/// (still a draft) cannot be exported and the daemon says so.
///
/// # Errors
/// Fails on a transport error, a refusal, or an answer with no path.
pub fn export_html(
    client: &Client,
    session: &str,
    output_path: &str,
) -> std::io::Result<String> {
    let frame = client.request(
        "export_html",
        json!({"activeSessionId": session, "outputPath": output_path}),
        Duration::from_secs(120),
    )?;
    let failed = frame.get("success").and_then(Value::as_bool) == Some(false);
    if failed {
        let reason = frame
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("the export was refused");
        return Err(std::io::Error::other(reason.to_string()));
    }
    frame
        .pointer("/data/path")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| std::io::Error::other("the export wrote no file"))
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
    images: &[Value],
) -> std::io::Result<String> {
    let mut payload = json!({
        "activeSessionId": session,
        "message": message,
        "streamingBehavior": "steer",
        "queueIfBusy": true,
    });
    if !images.is_empty() {
        payload["images"] = Value::Array(images.to_vec());
    }
    client.send("prompt", payload)
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


/// The session's extra slash commands (prompt templates and skills), as the
/// daemon lists them.
///
/// The builtin table is the same everywhere; these come from the session and
/// only exist there, which is why a menu built from the table alone would look
/// complete and still be missing commands the agent has.
pub fn get_commands(
    client: &Client,
    session: &str,
) -> std::io::Result<Vec<(String, String, String)>> {
    let response = client.request(
        "get_commands",
        json!({ "activeSessionId": session }),
        Duration::from_secs(10),
    )?;
    let entries = response
        .pointer("/data/commands")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("name").and_then(Value::as_str)?.to_string();
            let description = entry
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let is_skill = entry.get("source").and_then(Value::as_str) == Some("skill");
            let hint = entry
                .get("argumentHint")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| is_skill.then(|| "your request".to_string()))
                .unwrap_or_default();
            Some((name, description, hint))
        })
        .collect())
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

/// A session's live configuration: the model that is running and at what effort.
#[derive(Debug, Clone, Default)]
pub struct SessionState {
    pub model_id: String,
    pub provider: String,
    /// `off | minimal | low | medium | high | xhigh | max`
    pub thinking_level: String,
    pub is_streaming: bool,
    pub workspace_binding: Option<WorkspaceBinding>,
}

impl SessionState {
    fn from_value(value: &Value) -> Self {
        let model = value.get("model");
        Self {
            model_id: model
                .and_then(|model| model.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            provider: model
                .and_then(|model| model.get("provider"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            thinking_level: value
                .get("thinkingLevel")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            is_streaming: value
                .get("isStreaming")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            workspace_binding: value
                .get("workspaceBinding")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        }
    }
}

/// `get_state` → the running model and the level it is thinking at.
///
/// # Errors
/// Fails on transport errors or a refused request.
pub fn session_state(client: &Client, session: &str) -> std::io::Result<SessionState> {
    let response = client.request(
        "get_state",
        json!({"activeSessionId": session}),
        Duration::from_secs(10),
    )?;
    Ok(response
        .get("data")
        .map(SessionState::from_value)
        .unwrap_or_default())
}

/// `set_thinking_level` → change how hard the running model thinks.
///
/// # Errors
/// Fails only on transport errors; a clamped or refused level shows in the status.
pub fn set_thinking_level(client: &Client, session: &str, level: &str) -> std::io::Result<String> {
    client.send(
        "set_thinking_level",
        json!({"activeSessionId": session, "level": level}),
    )
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

/// `clear_queue` → drop everything waiting, both lanes.
///
/// # Errors
/// Fails on transport errors (a refusal shows up in the status line).
pub fn clear_queue(client: &Client, session: &str) -> std::io::Result<String> {
    client.send("clear_queue", json!({"activeSessionId": session}))
}

/// One agent stream event → the TUI's loader vocabulary.
///
/// Mirrors `track_stream_activity` in pa-tui: the event *kind*, not the delta,
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


/// `create` → a fresh empty session, returning its active id.
///
/// This is what the TUI's `/new` does (`create_session` in
/// `session_ui/sessions_fork.rs`): a new *draft* session alongside the running
/// one, not a `new_session`, which would swap the engine out from under the
/// turn that is streaming.
///
/// # Errors
/// Fails on transport errors or a refusal from the daemon.
pub fn create_session(client: &Client) -> std::io::Result<String> {
    create_session_with(client, json!({}))
}

/// `create` with the config the new Session should start from: model, thinking
/// level, cwd, or the other create resources the daemon folds before its first
/// build.
///
/// # Errors
/// Fails on transport errors or a refusal from the daemon.
pub fn create_session_with(client: &Client, payload: Value) -> std::io::Result<String> {
    let response = client.request("create", payload, Duration::from_secs(60))?;
    if response.get("success").and_then(Value::as_bool) != Some(true) {
        return Err(std::io::Error::other(format!(
            "create failed: {}",
            response.get("error").and_then(Value::as_str).unwrap_or("?")
        )));
    }
    response
        .pointer("/data/activeSessionId")
        .or_else(|| response.pointer("/data/id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| std::io::Error::other("create reported no session id"))
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
        assert!(parses(json!({"type": "create"})));
        assert!(parses(json!({"type": "heartbeats_list"})));
        assert!(parses(json!({"type": "list_kernel_bash", "activeSessionId": "s"})));
    }

    #[test]
    fn a_renamed_field_is_caught() {
        assert!(!parses(json!({"type": "prompt", "session": "s", "message": "m"})));
    }

    #[test]
    fn a_thinking_model_offers_the_ladder_but_not_the_wired_back_ends() {
        // no map at all: everything but the two levels that need an explicit mapping
        assert_eq!(
            supported_levels(true, None),
            vec!["off", "minimal", "low", "medium", "high"]
        );
        // a map that names xhigh and nulls out minimal
        let map = json!({"xhigh": "max", "minimal": null});
        assert_eq!(
            supported_levels(true, Some(&map)),
            vec!["off", "low", "medium", "high", "xhigh"]
        );
    }

    #[test]
    fn a_model_without_a_thinking_surface_offers_only_off() {
        assert_eq!(supported_levels(false, None), vec!["off"]);
        // an all-null map is not a capability claim
        assert_eq!(
            supported_levels(false, Some(&json!({"low": null}))),
            vec!["off"]
        );
    }

    #[test]
    fn a_wired_level_makes_a_non_reasoning_model_think() {
        assert_eq!(
            supported_levels(false, Some(&json!({"xhigh": "max"}))),
            vec!["off", "minimal", "low", "medium", "high", "xhigh"]
        );
    }

    #[test]
    fn a_level_clamps_onto_the_models_own_ladder() {
        let levels: Vec<String> = ["off", "low", "medium", "high"]
            .iter()
            .map(|level| (*level).to_string())
            .collect();
        // taken as is when the model has it
        assert_eq!(clamp_level(&levels, "medium"), "medium");
        // too strong: nearest lower
        assert_eq!(clamp_level(&levels, "max"), "high");
        // too weak: nearest higher
        assert_eq!(clamp_level(&levels, "minimal"), "low");
        // a model with no thinking surface takes everything down to off
        assert_eq!(clamp_level(&["off".to_string()], "high"), "off");
        // an unknown word still lands somewhere real
        assert_eq!(clamp_level(&levels, ""), "off");
    }
}

#[cfg(test)]
mod image_probe {
    use super::*;
    use serde_json::json;

    /// Probe: does a prompt carrying an image reach the store at all?
    /// A 1x1 PNG, so size cannot be the reason.
    #[test]
    #[ignore = "needs the live daemon"]
    fn tiny_image_prompt_lands() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        let images = vec![json!({"type": "image", "data": png, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "probe with image", &images).expect("send");
        eprintln!("admission: {id}");
        std::thread::sleep(std::time::Duration::from_secs(6));
        let (messages, _cursor, _more) = get_tail(&client, &session, 20).expect("tail");
        let hit = messages.iter().any(|m| {
            m.get("message")
                .and_then(|s| s.get("content"))
                .and_then(Value::as_array)
                .is_some_and(|c| c.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("image")))
        });
        let text = messages.iter().any(|m| {
            m.get("message").and_then(|s| s.get("content")).and_then(Value::as_array).is_some_and(|c| {
                c.iter().any(|b| b.get("text").and_then(Value::as_str).is_some_and(|t| t.contains("probe with image")))
            })
        });
        eprintln!("text landed: {text} | image landed: {hit}");
        assert!(text, "the prompt never became a message");
    }
}

#[cfg(test)]
mod queue_image_probe {
    use super::*;
    use serde_json::json;

    /// Probe: does a prompt carrying an image survive being QUEUED behind a
    /// running turn? The user's window is busy whenever they attach an image,
    /// which is why every image prompt they send disappears.
    #[test]
    #[ignore = "needs the live daemon"]
    fn queued_image_prompt_lands() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let _ = send_prompt(&client, &session, "first", &[]).expect("first");
        std::thread::sleep(std::time::Duration::from_millis(1200));
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        let images = vec![json!({"type": "image", "data": png, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "queued with image", &images).expect("second");
        eprintln!("queued admission: {id}");
        std::thread::sleep(std::time::Duration::from_secs(25));
        let (messages, _cur, _more) = get_tail(&client, &session, 40).expect("tail");
        let mut kinds: Vec<String> = Vec::new();
        for record in &messages {
            let Some(message) = record.get("message") else { continue };
            if message.get("role").and_then(Value::as_str) != Some("user") {
                continue;
            }
            let Some(blocks) = message.get("content").and_then(Value::as_array) else { continue };
            let has_image = blocks.iter().any(|b| b.get("type").and_then(Value::as_str) == Some("image"));
            let text: String = blocks.iter().filter_map(|b| b.get("text").and_then(Value::as_str)).collect();
            kinds.push(format!("{} image={has_image}", text.trim()));
        }
        eprintln!("user rows: {kinds:?}");
    }
}

#[cfg(test)]
mod big_image_probe {
    use super::*;
    use serde_json::json;

    /// Probe: is there a size at which a prompt carrying an image stops landing?
    #[test]
    #[ignore = "needs the live daemon"]
    fn large_image_prompt_lands() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let big = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==".to_string() + &"A".repeat(96_000);
        let images = vec![json!({"type": "image", "data": big, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "big image probe", &images).expect("send");
        eprintln!("admission: {id} ({} base64 chars)", images[0]["data"].as_str().unwrap().len());
        std::thread::sleep(std::time::Duration::from_secs(10));
        eprintln!("session file: {session}");
    }
}

#[cfg(test)]
mod live_queue_probe {
    use super::*;

    #[test]
    #[ignore = "needs the live daemon"]
    fn print_live_queue() {
        let client = Client::connect().expect("connect");
        let session = "4aca1ad66bb5";
        eprintln!("state: {:?}", session_state(&client, session).map(|s| format!("{}/{} streaming={} ", s.provider, s.model_id, s.is_streaming)));
        match get_queue(&client, session) {
            Ok((steering, follow_up)) => eprintln!("queue steering={steering:?} follow_up={follow_up:?}"),
            Err(error) => eprintln!("queue error: {error}"),
        }
    }
}

#[cfg(test)]
mod catalog_probe {
    use super::*;

    #[test]
    #[ignore = "needs the live daemon"]
    fn print_image_capability() {
        let client = Client::connect().expect("connect");
        let catalog = model_catalog(&client, "4aca1ad66bb5").expect("catalog");
        let hit: Vec<String> = catalog
            .iter()
            .filter(|entry| entry.id.contains("DeepSeek-V4.1-Flash-1017"))
            .map(|entry| format!("{}/{} images={}", entry.provider, entry.id, entry.supports_images))
            .collect();
        eprintln!("catalog: {hit:?} (of {} models)", catalog.len());
        let vision = catalog.iter().filter(|e| e.supports_images).count();
        eprintln!("models declaring image input: {vision}");
    }
}

#[cfg(test)]
mod midturn_image_probe {
    use super::*;
    use serde_json::json;

    /// Probe: send an image prompt INTO a running turn and see whether the
    /// stored user message still carries the image.
    #[test]
    #[ignore = "needs the live daemon"]
    fn midturn_image_prompt() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let _ = send_prompt(&client, &session, "count slowly from one to five", &[]).expect("first");
        std::thread::sleep(std::time::Duration::from_millis(250));
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        let images = vec![json!({"type": "image", "data": png, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "midturn with image", &images).expect("second");
        eprintln!("midturn admission: {id} | session file stem: {session}");
        std::thread::sleep(std::time::Duration::from_secs(20));
    }
}

#[cfg(test)]
mod queue_trace_probe {
    use super::*;
    use serde_json::json;

    /// Trace one image prompt sent while a long turn runs: is it queued, and
    /// does it still exist after the turn settles?
    #[test]
    #[ignore = "needs the live daemon"]
    fn trace_queued_image_prompt() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let _ = send_prompt(&client, &session, "Run a bash command that sleeps 20 seconds, then say done.", &[])
            .expect("first");
        std::thread::sleep(std::time::Duration::from_secs(5));
        eprintln!("queue before send: {:?}", get_queue(&client, &session).map(|(s, f)| (s.len(), f.len())));
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        let images = vec![json!({"type": "image", "data": png, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "trace image text", &images).expect("second");
        eprintln!("admission: {id}");
        for step in 0..6 {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let q = get_queue(&client, &session);
            match q {
                Ok((steering, follow_up)) => eprintln!("t+{}s steering={steering:?} follow_up={follow_up:?}", (step + 1) * 5),
                Err(error) => eprintln!("t+{}s queue error {error}", (step + 1) * 5),
            }
        }
        eprintln!("session id: {session}");
    }
}

#[cfg(test)]
mod abort_then_image_probe {
    use super::*;
    use serde_json::json;

    /// After a stop, the daemon suspends queued input until a prompt resumes it.
    /// Does an image prompt sent in that window get swallowed?
    #[test]
    #[ignore = "needs the live daemon"]
    fn image_after_abort() {
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        let _ = send_prompt(&client, &session, "Run a bash command that sleeps 20 seconds, then say done.", &[])
            .expect("first");
        std::thread::sleep(std::time::Duration::from_secs(3));
        let _ = abort(&client, &session);
        eprintln!("aborted");
        std::thread::sleep(std::time::Duration::from_secs(4));
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFAAH/q842iQAAAABJRU5ErkJggg==";
        let images = vec![json!({"type": "image", "data": png, "mimeType": "image/png"})];
        let id = send_prompt(&client, &session, "after abort image text", &images).expect("second");
        eprintln!("admission after abort: {id}");
        std::thread::sleep(std::time::Duration::from_secs(15));
        eprintln!("session id: {session}");
    }
}

#[cfg(test)]
mod image_response_probe {
    use super::*;
    use serde_json::json;

    fn b64(bytes: &[u8]) -> String {
        const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            out.push(T[(n >> 18) as usize & 63] as char);
            out.push(T[(n >> 12) as usize & 63] as char);
            out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
            out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
        }
        out
    }

    /// Probe: what does the daemon actually answer to an image prompt sent
    /// while a turn is running? `Client::send` is fire-and-forget, so the app
    /// never sees this answer — this prints it.
    #[test]
    #[ignore = "needs the live daemon"]
    fn midturn_image_response() {
        let path = std::env::var("PA_PROBE_IMG").expect("PA_PROBE_IMG");
        let bytes = std::fs::read(path).expect("read image");
        let client = Client::connect().expect("connect");
        let session = create_session(&client).expect("create");
        eprintln!("probe session {session} image {} bytes", bytes.len());
        let first = client
            .request(
                "prompt",
                json!({
                    "activeSessionId": session,
                    "message": "Run a bash command that sleeps 20 seconds, then say done.",
                    "queueIfBusy": true,
                    "streamingBehavior": "followUp",
                }),
                std::time::Duration::from_secs(20),
            )
            .expect("first prompt");
        eprintln!("first prompt response: {first}");
        std::thread::sleep(std::time::Duration::from_secs(4));
        let images = vec![json!({"type": "image", "data": b64(&bytes), "mimeType": "image/png"})];
        let payload = json!({
            "activeSessionId": session,
            "message": "probe: midturn picture",
            "queueIfBusy": true,
            "streamingBehavior": "followUp",
            "images": images,
        });
        match client.request("prompt", payload, std::time::Duration::from_secs(30)) {
            Ok(response) => eprintln!("MIDTURN IMAGE RESPONSE: {response}"),
            Err(error) => eprintln!("MIDTURN IMAGE ERROR: {error}"),
        }
        eprintln!(
            "queue after send: {:?}",
            get_queue(&client, &session)
        );
        std::thread::sleep(std::time::Duration::from_secs(25));
        eprintln!(
            "queue after settle: {:?}",
            get_queue(&client, &session)
        );
    }
}

#[cfg(test)]
mod reject_probe {
    use super::*;

    /// Probe: does a refused prompt come back to the fire-and-forget send path,
    /// or is the refusal silent? A bogus session id should be refused.
    #[test]
    #[ignore = "needs the live daemon"]
    fn refused_prompt_verdict() {
        let client = Client::connect().expect("connect");
        let id = send_prompt(&client, "nope-not-a-session", "hello", &[]).expect("send");
        eprintln!("request id: {id}");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(6);
        while std::time::Instant::now() < deadline {
            if let Some(frame) = client.recv(std::time::Duration::from_millis(500)) {
                let ty = frame.get("type").and_then(serde_json::Value::as_str).unwrap_or("?");
                if ty == "response" {
                    eprintln!("VERDICT: {frame}");
                    return;
                }
            }
        }
        eprintln!("NO VERDICT ARRIVED within 6s");
    }
}

#[cfg(test)]
mod checked_send_probe {
    use super::*;

    /// Probe: the checked send reports a refusal as an error and a real send as
    /// success - the two states the window could not tell apart before.
    #[test]
    #[ignore = "needs the live daemon"]
    fn verdict_is_visible() {
        let client = Client::connect().expect("connect");
        eprintln!(
            "refused -> {:?}",
            send_prompt_checked(&client, "nope-not-a-session", "hello", &[])
        );
        let session = create_session(&client).expect("create");
        eprintln!(
            "accepted -> {:?} (session {session})",
            send_prompt_checked(&client, &session, "probe: plain text only", &[])
        );
    }
}

#[cfg(test)]
mod pagination_probe {
    use super::*;

    /// Probe: does a page cursor actually walk backwards, and does the second
    /// page end where the first one begins?
    ///
    /// Read-only: it picks the busiest session and reads two pages of it.
    #[test]
    #[ignore = "needs the live daemon"]
    fn a_page_cursor_walks_backwards() {
        let client = Client::connect().expect("connect");
        let mut sessions = list_sessions(&client).expect("sessions");
        sessions.sort_by_key(|row| std::cmp::Reverse(row.messages));
        let session = &sessions[0].session_id;

        let newest = get_page(&client, session, None, 20).expect("tail page");
        eprintln!(
            "page 1: {} msgs first={:?} last={:?} more={}",
            newest.messages.len(),
            newest.first_entry,
            newest.last_entry,
            newest.more
        );
        assert!(!newest.messages.is_empty(), "no messages to page through");

        let older = get_page(&client, session, Some(&newest.first_entry), 20).expect("older page");
        eprintln!(
            "page 2: {} msgs first={:?} last={:?} more={}",
            older.messages.len(),
            older.first_entry,
            older.last_entry,
            older.more
        );

        let stamp = |message: &Value| {
            message
                .get("timestamp")
                .and_then(Value::as_u64)
                .unwrap_or_default()
        };
        let newest_first = newest.messages.iter().map(stamp).min().unwrap_or(0);
        let older_last = older.messages.iter().map(stamp).max().unwrap_or(u64::MAX);
        assert!(
            older_last <= newest_first,
            "the older page is not older: {older_last} > {newest_first}"
        );
        if older.more {
            assert!(!older.first_entry.is_empty(), "no cursor for the third page");
            assert_ne!(older.first_entry, newest.first_entry, "the cursor did not move");
        }
    }
}


#[cfg(test)]
mod activity_tests {
    use super::*;

    #[test]
    fn heartbeat_count_scopes_by_durable_or_live_session() {
        let data = json!({"heartbeats": [
            {"job": {"id": "a", "sessionId": "durable", "status": "active"}},
            {"job": {"id": "b", "activeSessionId": "live", "status": "paused"}},
            {"job": {"id": "c", "sessionId": "other", "activeSessionId": "other-live"}},
            {"job": {"id": "d", "sessionId": "durable", "status": "paused"}}
        ]});
        assert_eq!(heartbeat_count(&data, "live", "durable"), 3);
        assert_eq!(heartbeat_count(&data, "live", ""), 0);
        assert_eq!(heartbeat_count(&data, "", "durable"), 2);
        assert_eq!(heartbeat_count(&data, "missing", "missing"), 0);
    }

    #[test]
    fn heartbeat_count_never_matches_an_empty_identity() {
        let data = json!({"heartbeats": [
            {"job": {"id": "empty", "sessionId": "", "activeSessionId": "", "status": "active"}}
        ]});
        assert_eq!(heartbeat_count(&data, "", ""), 0);
    }

    /// Read-only smoke against the real daemon. Run explicitly:
    /// cargo test -p pa-client live_activity_counts -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_activity_counts_are_readable() {
        let client = Client::connect().expect("daemon");
        let rows = list_sessions(&client).expect("roster");
        let live: Vec<_> = rows
            .iter()
            .filter(|row| row.is_resident() && !row.id.is_empty())
            .collect();
        assert!(!live.is_empty(), "no live Session to read");
        let roster = list_roster(&client).expect("agent roster");
        assert!(!roster.is_empty(), "the daemon returned an empty roster");
        for row in live {
            let subagents = running_subagents(&roster, &row.id, &row.session_id);
            let heartbeats = scoped_heartbeat_count(&client, &row.id, &row.session_id)
                .expect("heartbeat catalog");
            let shells = running_shells(&client, &row.id).expect("shell registry");
            println!(
                "{}: subagents={subagents} heartbeats={heartbeats} shells={shells}",
                if row.name.is_empty() { &row.id } else { &row.name }
            );
        }
    }

    #[test]
    fn running_subagents_walks_direct_and_nested_roster_children() {
        let roster = vec![
            json!({"agentId": "root", "status": "running", "summary": {
                "activeSessionId": "parent", "sessionId": "uuid-parent",
                "sessionFile": "parent.jsonl"
            }}),
            json!({"agentId": "child-1", "status": "running", "summary": {
                "activeSessionId": "child-1", "sessionId": "uuid-child-1",
                "parentActiveSessionId": "parent", "parentSessionId": "uuid-parent",
                "rlmDepth": 1, "runtimeKind": "subagent"
            }}),
            json!({"agentId": "child-2", "status": "running", "summary": {
                "activeSessionId": "child-2", "sessionId": "uuid-child-2",
                "parentActiveSessionId": "child-1", "parentSessionId": "uuid-child-1",
                "rlmDepth": 2, "runtimeKind": "subagent"
            }}),
            json!({"agentId": "child-3", "status": "idle", "summary": {
                "activeSessionId": "child-3", "sessionId": "uuid-child-3",
                "parentSessionId": "uuid-parent", "rlmDepth": 1,
                "runtimeKind": "subagent"
            }}),
            json!({"agentId": "child-4", "status": "inactive", "summary": {
                "activeSessionId": "child-4", "sessionId": "uuid-child-4",
                "parentSessionId": "uuid-parent", "rlmDepth": 1,
                "runtimeKind": "subagent"
            }}),
            json!({"agentId": "path-child", "status": "running", "summary": {
                "activeSessionId": "path-child", "sessionId": "uuid-path-child",
                "rlmDepth": 1, "runtimeKind": "subagent",
                "parentSessionPath": "parent.jsonl", "sessionFile": "path-child.jsonl"
            }}),
            json!({"agentId": "path-grandchild", "status": "running", "summary": {
                "activeSessionId": "path-grandchild", "sessionId": "uuid-path-grandchild",
                "rlmDepth": 2, "runtimeKind": "subagent",
                "parentSessionPath": "path-child.jsonl", "sessionFile": "grandchild.jsonl"
            }}),
            json!({"agentId": "fork", "status": "running", "summary": {
                "activeSessionId": "fork", "sessionId": "uuid-fork", "rlmDepth": 0,
                "runtimeKind": "top-level", "parentActiveSessionId": "parent",
                "parentSessionPath": "parent.jsonl"
            }})
        ];
        assert_eq!(running_subagents(&roster, "parent", "uuid-parent"), 4);
        assert_eq!(running_subagents(&roster, "child-1", "uuid-child-1"), 1);
        assert_eq!(running_subagents(&roster, "missing", "missing"), 0);
    }

    #[test]
    fn running_shell_count_ignores_finished_and_nameless_rows() {
        let data = json!({"activities": [
            {"id": "one", "status": "running"},
            {"id": "two", "status": "finished"},
            {"id": "three", "status": "running"},
            {"id": "", "status": "running"},
            {"status": "running"}
        ]});
        assert_eq!(running_shell_count(&data), 2);
        assert_eq!(running_shell_count(&data["activities"]), 2);
        assert_eq!(running_shell_count(&json!({})), 0);
    }

    #[test]
    fn roster_update_frames_survive_the_event_pump() {
        let frame = event_frame(wire::daemon_client::DaemonClientEvent::RosterUpdate {
            changed: vec![json!({"agentId": "a1", "status": "idle"})],
            removed: vec!["a2".to_string()],
            resync: true,
        })
        .expect("roster update is a client frame");

        assert_eq!(frame["type"], "roster_update");
        assert_eq!(frame["changed"][0]["agentId"], "a1");
        assert_eq!(frame["removed"][0], "a2");
        assert_eq!(frame["resync"], true);
    }

    #[test]
    fn session_binding_frames_carry_the_new_id() {
        let frame = event_frame(wire::daemon_client::DaemonClientEvent::SessionBinding {
            previous_active_session_id: "old".into(),
            active_session_id: "new".into(),
        })
        .expect("session binding is a client frame");

        assert_eq!(frame["type"], "session_binding");
        assert_eq!(frame["previousActiveSessionId"], "old");
        assert_eq!(frame["activeSessionId"], "new");
    }

    #[test]
    fn model_catalog_changed_frames_survive_the_event_pump() {
        let frame = event_frame(wire::daemon_client::DaemonClientEvent::ModelCatalogChanged)
            .expect("model catalog change is a client frame");
        assert_eq!(frame["type"], "model_catalog_changed");
    }
}

#[cfg(test)]
mod saved_row_tests {
    use super::*;

    #[test]
    fn saved_catalog_rows_keep_their_own_identity() {
        let saved = SessionRow::from_value(&json!({
            "id": "saved-1",
            "path": "/tmp/saved-1.jsonl",
            "name": "named row",
            "firstMessage": "first message",
            "modified": "2026-11-09T12:00:00Z",
            "messageCount": 2
        }));
        assert_eq!(saved.id, "saved-1");
        assert_eq!(saved.session_id, "saved-1");
        assert_eq!(saved.name, "named row");
        assert_eq!(saved.session_file, "/tmp/saved-1.jsonl");
    }

    #[test]
    fn live_rows_still_prefer_the_live_identity() {
        let live = SessionRow::from_value(&json!({
            "activeSessionId": "live-1",
            "sessionId": "uuid-1",
            "sessionName": "live row"
        }));
        assert_eq!(live.id, "live-1");
        assert_eq!(live.session_id, "uuid-1");
    }
}
