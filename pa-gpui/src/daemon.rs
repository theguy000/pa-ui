use async_channel::{Receiver as AsyncReceiver, Sender as AsyncSender};
use pa_client::{
    BranchInfo, Client, HarnessEntryWire, HarnessKindWire, HarnessScopeWire, ModelEntry,
    RefinementHistoryItemWire, SessionExtra, SessionRow, SessionState, SessionStats,
    WorkspaceBinding, WorkspaceRecord, WorktreeInfo,
};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum DaemonCmd {
    RefreshSessions,
    SelectSession {
        session: String,
        request_id: u64,
    },
    DetachCurrent,
    ReloadTranscript(String),
    LoadOlder {
        session: String,
        before: String,
        limit: usize,
    },
    StartChat {
        text: String,
        images: Vec<Value>,
        model: Option<(String, String)>,
        level: Option<String>,
        cwd: Option<String>,
        workspace_id: Option<String>,
        worktree_path: Option<String>,
    },
    SendPrompt {
        session: String,
        text: String,
        images: Vec<Value>,
    },
    SteerPrompt {
        session: String,
        text: String,
        images: Vec<Value>,
    },
    Abort { session: String },
    MarkDone { session: String },
    DeleteSaved { session_path: String },
    RenameSession { session: String, name: String },
    RenameSaved { session_path: String, name: String },
    ClearQueue { session: String },
    MutateQueue {
        session: String,
        lane: String,
        slot: usize,
        expected: String,
        action: String,
    },
    EditQueue {
        session: String,
        lane: String,
        slot: usize,
        expected: String,
        text: String,
        images: Vec<Value>,
    },
    ExportHtml { session: String },
    ExportGist { session: String },
    SetModel { session: String, provider: String, model_id: String },
    SetModelAndThinking {
        session: String,
        provider: String,
        model_id: String,
        level: String,
        switch: bool,
    },
    RefreshModels { session: Option<String> },
    RefreshWorkspaces,
    RegisterWorkspace { path: String, name: Option<String> },
    RelinkWorkspace { workspace_id: String, new_path: String },
    DeleteWorkspace { workspace_id: String, delete_harness: bool },
    LoadWorktrees { workspace_id: String },
    CreateWorktree { workspace_id: String, name: String, base_branch: Option<String> },
    LoadBranches { worktree_path: String },
    SwitchBranch { worktree_path: String, branch: String, active_session_id: Option<String> },
    CreateBranch { worktree_path: String, branch: String, start_point: Option<String>, active_session_id: Option<String> },
    BindSessionWorkspace { session: String, workspace_id: Option<String>, worktree_path: Option<String> },
    SwitchSessionWorktree { session: String, worktree_path: String },
    RefreshSessionWorkspace { session: String },
    HarnessList {
        scope: Option<HarnessScopeWire>,
        workspace_id: Option<String>,
        active_session_id: Option<String>,
        kind: Option<HarnessKindWire>,
        query: Option<String>,
    },
    HarnessUpsert {
        scope: HarnessScopeWire,
        workspace_id: Option<String>,
        active_session_id: Option<String>,
        entry_id: Option<String>,
        kind: HarnessKindWire,
        title: String,
        content: String,
        path: Option<String>,
    },
    HarnessDelete {
        scope: HarnessScopeWire,
        workspace_id: Option<String>,
        active_session_id: Option<String>,
        kind: HarnessKindWire,
        entry_id: String,
    },
    HarnessHistory {
        scope: Option<HarnessScopeWire>,
        workspace_id: Option<String>,
        active_session_id: Option<String>,
    },
    HarnessRollback {
        refinement_id: String,
        workspace_id: Option<String>,
        active_session_id: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Html,
    Gist,
}

#[derive(Debug)]
pub struct SessionLoadedPayload {
    pub requested_session: String,
    pub resolved_session: String,
    pub request_id: u64,
    pub messages: Vec<Value>,
    pub first_entry: String,
    pub more: bool,
    pub state: Option<SessionState>,
    pub stats: Option<SessionStats>,
    pub steer: Vec<String>,
    pub follow_up: Vec<String>,
}

#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected(String),
    Sessions(Vec<SessionRow>),
    SessionCreated(String),
    SessionLoaded(Box<SessionLoadedPayload>),
    TailRefresh {
        session: String,
        messages: Vec<Value>,
        first_entry: String,
        more: bool,
    },
    OlderMessages {
        session: String,
        messages: Vec<Value>,
        first_entry: String,
        more: bool,
    },
    Models(Vec<ModelEntry>),
    SessionState { session: String, state: SessionState },
    Stats { session: String, stats: SessionStats },
    Queue { session: String, steer: Vec<String>, follow_up: Vec<String> },
    Activity { session: String, subagents: usize, shells: usize, heartbeats: usize },
    ExportResult {
        kind: ExportKind,
        primary: String,
        secondary: Option<String>,
    },
    WorkspacesLoaded {
        workspaces: Vec<WorkspaceRecord>,
        worktree_root: String,
    },
    WorkspaceRegistered {
        workspace: WorkspaceRecord,
        binding: WorkspaceBinding,
    },
    WorkspaceRelinked(WorkspaceRecord),
    WorkspaceDeleted(String),
    WorktreesLoaded {
        workspace_id: String,
        worktrees: Vec<WorktreeInfo>,
    },
    WorktreeCreated {
        workspace_id: String,
        worktree: WorktreeInfo,
    },
    BranchesLoaded {
        worktree_path: String,
        branches: Vec<BranchInfo>,
    },
    BranchSwitched {
        worktree_path: String,
        branch: String,
    },
    WorkspaceBindingUpdated {
        session: String,
        binding: Option<WorkspaceBinding>,
    },
    HarnessEntriesLoaded {
        entries: Vec<HarnessEntryWire>,
    },
    HarnessEntrySaved(HarnessEntryWire),
    HarnessEntryDeleted { entry_id: String },
    HarnessHistoryLoaded(Vec<RefinementHistoryItemWire>),
    HarnessRolledBack,
    LiveFrame(Value),
    Error(String),
}

pub struct DaemonBridge {
    pub tx: Sender<DaemonCmd>,
}

impl DaemonBridge {
    pub fn spawn() -> (Self, AsyncReceiver<DaemonEvent>) {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DaemonCmd>();
        let (ev_tx, ev_rx) = async_channel::unbounded::<DaemonEvent>();

        thread::spawn(move || run_worker_loop(cmd_rx, ev_tx));

        (Self { tx: cmd_tx }, ev_rx)
    }
}

fn request_checked(
    client: &Client,
    ty: &str,
    payload: Value,
    timeout: Duration,
) -> Result<(), String> {
    match client.request(ty, payload, timeout) {
        Ok(resp) if resp.get("success").and_then(Value::as_bool) == Some(false) => Err(resp
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("request rejected")
            .to_string()),
        Ok(_) => Ok(()),
        Err(err) => Err(err.to_string()),
    }
}

const TRANSCRIPT_PAGE_LIMIT: usize = 80;

fn drain_and_coalesce(cmd_rx: &Receiver<DaemonCmd>, queue: &mut VecDeque<DaemonCmd>) {
    while let Ok(cmd) = cmd_rx.try_recv() {
        match cmd {
            DaemonCmd::SelectSession { .. } => {
                // Keep only the latest session selection and drop superseded refreshes.
                queue.retain(|c| !matches!(c, DaemonCmd::SelectSession { .. } | DaemonCmd::DetachCurrent));
                queue.push_back(cmd);
            }
            DaemonCmd::DetachCurrent => {
                queue.retain(|c| !matches!(c, DaemonCmd::SelectSession { .. } | DaemonCmd::DetachCurrent));
                queue.push_back(cmd);
            }
            DaemonCmd::RefreshSessions => {
                if !queue.iter().any(|c| matches!(c, DaemonCmd::RefreshSessions)) {
                    queue.push_back(cmd);
                }
            }
            other => queue.push_back(other),
        }
    }
}

struct WorkerSessionState<'a> {
    client: &'a Client,
    client_id: &'a str,
    ev_tx: &'a AsyncSender<DaemonEvent>,
    cmd_rx: &'a Receiver<DaemonCmd>,
    models_loaded: bool,
    current_attached: Option<(String, String)>,
    pending_cmds: VecDeque<DaemonCmd>,
}

impl WorkerSessionState<'_> {
    fn load_session(&mut self, sid: &str, request_id: u64, refresh_sessions: bool) {
        if let Some((prev_active, _)) = self.current_attached.as_ref()
            && prev_active != sid
        {
            pa_client::detach_session(self.client, prev_active);
        }

        let snap = match pa_client::attach_snapshot(self.client, sid, Some(self.client_id)) {
            Ok(snap) => snap,
            Err(err) => {
                let _ = self.ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                return;
            }
        };

        let resolved_sid = snap.active_session_id.clone();
        let durable_sid = snap.session_id.clone();
        self.current_attached = Some((resolved_sid.clone(), durable_sid.clone()));

        let _ = self.ev_tx.try_send(DaemonEvent::SessionLoaded(Box::new(
            SessionLoadedPayload {
                requested_session: sid.to_string(),
                resolved_session: resolved_sid.clone(),
                request_id,
                messages: snap.messages,
                first_entry: snap.first_entry,
                more: snap.more,
                state: snap.state,
                stats: snap.stats,
                steer: snap.steer,
                follow_up: snap.follow_up,
            },
        )));

        drain_and_coalesce(self.cmd_rx, &mut self.pending_cmds);
        if self.pending_cmds.iter().any(|c| {
            matches!(
                c,
                DaemonCmd::SelectSession { .. } | DaemonCmd::DetachCurrent
            )
        }) {
            return;
        }

        let need_models = !self.models_loaded;
        let got_models = AtomicBool::new(false);
        let need_sessions = refresh_sessions || resolved_sid != sid;
        let sid_for_emit = resolved_sid.clone();
        let ev_tx = self.ev_tx;
        pa_client::fetch_session_extras_concurrent(
            self.client,
            &resolved_sid,
            &durable_sid,
            need_models,
            need_sessions,
            |extra| match extra {
                SessionExtra::Activity {
                    subagents,
                    shells,
                    heartbeats,
                } => {
                    let _ = ev_tx.try_send(DaemonEvent::Activity {
                        session: sid_for_emit.clone(),
                        subagents,
                        shells,
                        heartbeats,
                    });
                }
                SessionExtra::Stats(stats) => {
                    let _ = ev_tx.try_send(DaemonEvent::Stats {
                        session: sid_for_emit.clone(),
                        stats,
                    });
                }
                SessionExtra::Models(models) => {
                    got_models.store(true, Ordering::Relaxed);
                    let _ = ev_tx.try_send(DaemonEvent::Models(models));
                }
                SessionExtra::Sessions(sessions) => {
                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                }
                SessionExtra::Queue { steer, follow_up } => {
                    let _ = ev_tx.try_send(DaemonEvent::Queue {
                        session: sid_for_emit.clone(),
                        steer,
                        follow_up,
                    });
                }
                SessionExtra::Tail(_) => {}
            },
        );
        if got_models.load(Ordering::Relaxed) {
            self.models_loaded = true;
        }
    }
}

fn run_worker_loop(cmd_rx: Receiver<DaemonCmd>, ev_tx: AsyncSender<DaemonEvent>) {
    loop {
        let client = match Client::connect() {
            Ok(c) => {
                let _ = ev_tx.try_send(DaemonEvent::Connected);
                c
            }
            Err(err) => {
                let _ = ev_tx.try_send(DaemonEvent::Disconnected(format!(
                    "Reconnecting… ({err})"
                )));
                while cmd_rx.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(1500));
                continue;
            }
        };

        let client_id = format!("pa-gpui-{}", std::process::id());
        let mut ws = WorkerSessionState {
            client: &client,
            client_id: &client_id,
            ev_tx: &ev_tx,
            cmd_rx: &cmd_rx,
            models_loaded: false,
            current_attached: None,
            pending_cmds: VecDeque::new(),
        };

        if let Ok((workspaces, worktree_root)) = pa_client::list_workspaces(&client) {
            let _ = ev_tx.try_send(DaemonEvent::WorkspacesLoaded {
                workspaces,
                worktree_root,
            });
        }

        if let Ok(sessions) = pa_client::list_sessions(&client) {
            let first_id = sessions.first().map(|s| {
                if !s.id.is_empty() {
                    s.id.clone()
                } else {
                    s.session_file.clone()
                }
            });
            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
            if let Some(sid) = first_id {
                ws.load_session(&sid, 0, false);
            }
        }

        loop {
            drain_and_coalesce(&cmd_rx, &mut ws.pending_cmds);
            while let Some(cmd) = ws.pending_cmds.pop_front() {
                match cmd {
                    DaemonCmd::RefreshSessions => match pa_client::list_sessions(&client) {
                        Ok(sessions) => {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                        Err(err) => {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        }
                    },
                    DaemonCmd::SelectSession { session, request_id } => {
                        ws.load_session(&session, request_id, false);
                    }
                    DaemonCmd::DetachCurrent => {
                        if let Some((prev_active, _)) = ws.current_attached.take() {
                            pa_client::detach_session(&client, &prev_active);
                        }
                    }
                    DaemonCmd::ReloadTranscript(sid) => {
                        let durable_sid = ws.current_attached
                            .as_ref()
                            .filter(|(active, _)| active == &sid)
                            .map(|(_, durable)| durable.clone())
                            .unwrap_or_default();
                        let sid_emit = sid.clone();
                        pa_client::reload_session_parallel(
                            &client,
                            &sid,
                            &durable_sid,
                            TRANSCRIPT_PAGE_LIMIT,
                            |extra| match extra {
                                SessionExtra::Tail(page) => {
                                    let _ = ev_tx.try_send(DaemonEvent::TailRefresh {
                                        session: sid_emit.clone(),
                                        messages: page.messages,
                                        first_entry: page.first_entry,
                                        more: page.more,
                                    });
                                }
                                SessionExtra::Stats(stats) => {
                                    let _ = ev_tx.try_send(DaemonEvent::Stats {
                                        session: sid_emit.clone(),
                                        stats,
                                    });
                                }
                                SessionExtra::Queue { steer, follow_up } => {
                                    let _ = ev_tx.try_send(DaemonEvent::Queue {
                                        session: sid_emit.clone(),
                                        steer,
                                        follow_up,
                                    });
                                }
                                SessionExtra::Activity {
                                    subagents,
                                    shells,
                                    heartbeats,
                                } => {
                                    let _ = ev_tx.try_send(DaemonEvent::Activity {
                                        session: sid_emit.clone(),
                                        subagents,
                                        shells,
                                        heartbeats,
                                    });
                                }
                                SessionExtra::Sessions(sessions) => {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                                SessionExtra::Models(_) => {}
                            },
                        );
                    }
                    DaemonCmd::LoadOlder {
                        session,
                        before,
                        limit,
                    } => match pa_client::get_page(&client, &session, Some(&before), limit) {
                        Ok(page) => {
                            let _ = ev_tx.try_send(DaemonEvent::OlderMessages {
                                session,
                                messages: page.messages,
                                first_entry: page.first_entry,
                                more: page.more,
                            });
                        }
                        Err(_) => {
                            let _ = ev_tx.try_send(DaemonEvent::OlderMessages {
                                session,
                                messages: Vec::new(),
                                first_entry: String::new(),
                                more: false,
                            });
                        }
                    },
                    DaemonCmd::StartChat {
                        text,
                        images,
                        model,
                        level,
                        cwd,
                        workspace_id,
                        worktree_path,
                    } => {
                        let mut config = serde_json::Map::new();
                        if let Some((provider, model_id)) = model {
                            config.insert("provider".into(), Value::String(provider));
                            config.insert("model".into(), Value::String(model_id));
                        }
                        if let Some(level) = level {
                            config.insert("thinking".into(), Value::String(level));
                        }
                        if let Some(cwd) = cwd {
                            config.insert("cwd".into(), Value::String(cwd));
                        }
                        let mut create = json!({});
                        if !config.is_empty() {
                            create["config"] = Value::Object(config);
                        }
                        if let Some(ws_id) = workspace_id {
                            create["workspaceId"] = Value::String(ws_id);
                        }
                        if let Some(wt_path) = worktree_path {
                            create["worktreePath"] = Value::String(wt_path);
                        }
                        if let Some((prev_active, _)) = ws.current_attached.take() {
                            pa_client::detach_session(&client, &prev_active);
                        }
                        match pa_client::create_session_with(&client, create) {
                            Ok(new_sid) => {
                                let _ = ev_tx.try_send(DaemonEvent::SessionCreated(new_sid.clone()));
                                let snap = match pa_client::attach_snapshot(
                                    &client,
                                    &new_sid,
                                    Some(&client_id),
                                ) {
                                    Ok(s) => s,
                                    Err(err) => {
                                        let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                                        continue;
                                    }
                                };
                                ws.current_attached =
                                    Some((new_sid.clone(), snap.session_id.clone()));
                                if let Some(state) = snap.state {
                                    let _ = ev_tx.try_send(DaemonEvent::SessionState {
                                        session: new_sid.clone(),
                                        state,
                                    });
                                }
                                if let Some(stats) = snap.stats {
                                    let _ = ev_tx.try_send(DaemonEvent::Stats {
                                        session: new_sid.clone(),
                                        stats,
                                    });
                                }
                                if let Err(err) =
                                    pa_client::send_prompt(&client, &new_sid, &text, &images)
                                {
                                    let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                                }
                                let got_models = AtomicBool::new(false);
                                let sid_emit = new_sid.clone();
                                pa_client::fetch_session_extras_concurrent(
                                    &client,
                                    &new_sid,
                                    &snap.session_id,
                                    !ws.models_loaded,
                                    true,
                                    |extra| match extra {
                                        SessionExtra::Activity {
                                            subagents,
                                            shells,
                                            heartbeats,
                                        } => {
                                            let _ = ev_tx.try_send(DaemonEvent::Activity {
                                                session: sid_emit.clone(),
                                                subagents,
                                                shells,
                                                heartbeats,
                                            });
                                        }
                                        SessionExtra::Stats(stats) => {
                                            let _ = ev_tx.try_send(DaemonEvent::Stats {
                                                session: sid_emit.clone(),
                                                stats,
                                            });
                                        }
                                        SessionExtra::Models(models) => {
                                            got_models.store(true, Ordering::Relaxed);
                                            let _ = ev_tx.try_send(DaemonEvent::Models(models));
                                        }
                                        SessionExtra::Sessions(sessions) => {
                                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                        }
                                        SessionExtra::Queue { .. } | SessionExtra::Tail(_) => {}
                                    },
                                );
                                if got_models.load(Ordering::Relaxed) {
                                    ws.models_loaded = true;
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::SendPrompt {
                        session,
                        text,
                        images,
                    } => {
                        let outcome =
                            pa_client::send_prompt_checked(&client, &session, &text, &images);
                        if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session: session.clone(),
                                steer,
                                follow_up,
                            });
                        }
                        if let Err(err) = outcome {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::SteerPrompt {
                        session,
                        text,
                        images,
                    } => {
                        let mut payload = json!({
                            "activeSessionId": session,
                            "message": text,
                            "streamingBehavior": "steer",
                            "queueIfBusy": true,
                        });
                        if !images.is_empty() {
                            payload["images"] = Value::Array(images);
                        }
                        let outcome =
                            request_checked(&client, "prompt", payload, Duration::from_secs(30));
                        if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session: session.clone(),
                                steer,
                                follow_up,
                            });
                        }
                        if let Err(err) = outcome {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::Abort { session } => {
                        if let Err(err) = pa_client::abort(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::MarkDone { session } => {
                        if let Err(err) = pa_client::mark_done(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        } else {
                            thread::sleep(Duration::from_millis(120));
                            if let Ok(sessions) = pa_client::list_sessions(&client) {
                                let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                            }
                        }
                    }
                    DaemonCmd::DeleteSaved { session_path } => {
                        if let Err(err) = pa_client::delete_saved_session(&client, &session_path) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::RenameSession { session, name } => {
                        match request_checked(
                            &client,
                            "rename",
                            json!({"activeSessionId": session, "name": name}),
                            Duration::from_secs(10),
                        ) {
                            Ok(()) => {
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err));
                            }
                        }
                    }
                    DaemonCmd::RenameSaved { session_path, name } => {
                        match request_checked(
                            &client,
                            "rename_saved_session",
                            json!({"sessionPath": session_path, "name": name}),
                            Duration::from_secs(10),
                        ) {
                            Ok(()) => {
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err));
                            }
                        }
                    }
                    DaemonCmd::ClearQueue { session } => {
                        if let Err(err) = pa_client::clear_queue(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        } else {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session,
                                steer: Vec::new(),
                                follow_up: Vec::new(),
                            });
                        }
                    }
                    DaemonCmd::MutateQueue {
                        session,
                        lane,
                        slot,
                        expected,
                        action,
                    } => {
                        let mutation = if action == "promote" {
                            json!({"type": "replace", "lane": "steering", "text": expected})
                        } else {
                            json!({"type": "delete"})
                        };
                        if let Err(err) = pa_client::mutate_queue(
                            &client, &session, &lane, slot, &expected, mutation,
                        ) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        }
                        if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session,
                                steer,
                                follow_up,
                            });
                        }
                    }
                    DaemonCmd::EditQueue {
                        session,
                        lane,
                        slot,
                        expected,
                        text,
                        images,
                    } => {
                        let mut mutation = json!({"type": "replace", "text": text});
                        if !images.is_empty() {
                            mutation["images"] = Value::Array(images.clone());
                        }
                        let outcome = pa_client::mutate_queue(
                            &client, &session, &lane, slot, &expected, mutation,
                        );
                        if !matches!(outcome.as_deref(), Ok("applied")) {
                            // The slot was drained or moved while editing — send as a new prompt
                            // so the user's edited message is never lost.
                            if let Err(err) =
                                pa_client::send_prompt_checked(&client, &session, &text, &images)
                            {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err));
                            }
                        }
                        if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session,
                                steer,
                                follow_up,
                            });
                        }
                    }
                    DaemonCmd::ExportHtml { session } => {
                        let out = std::env::temp_dir().join(format!(
                            "prime-agent-{}.html",
                            session.chars().take(8).collect::<String>()
                        ));
                        let out_str = out.to_string_lossy().to_string();
                        match pa_client::export_html(&client, &session, &out_str) {
                            Ok(path) => {
                                let _ = ev_tx.try_send(DaemonEvent::ExportResult {
                                    kind: ExportKind::Html,
                                    primary: path,
                                    secondary: None,
                                });
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::ExportGist { session } => {
                        let out = std::env::temp_dir().join("session.html");
                        let out_str = out.to_string_lossy().to_string();
                        match pa_client::export_html(&client, &session, &out_str) {
                            Ok(path) => {
                                match std::process::Command::new("gh")
                                    .args(["gist", "create", &path, "--desc", "Prime Agent Session"])
                                    .output()
                                {
                                    Ok(out) if out.status.success() => {
                                        let gist_url = String::from_utf8_lossy(&out.stdout)
                                            .lines()
                                            .rev()
                                            .find(|l| !l.trim().is_empty())
                                            .unwrap_or("")
                                            .trim()
                                            .to_string();
                                        let gist_id = gist_url
                                            .rsplit('/')
                                            .next()
                                            .filter(|s| !s.is_empty());
                                        let (primary, secondary) = match gist_id {
                                            Some(id) if !gist_url.is_empty() => (
                                                format!("https://pi.dev/session/#{id}"),
                                                Some(gist_url),
                                            ),
                                            _ => (gist_url, None),
                                        };
                                        let _ = ev_tx.try_send(DaemonEvent::ExportResult {
                                            kind: ExportKind::Gist,
                                            primary,
                                            secondary,
                                        });
                                    }
                                    Ok(out) => {
                                        let stderr = String::from_utf8_lossy(&out.stderr)
                                            .trim()
                                            .to_string();
                                        let _ = ev_tx.try_send(DaemonEvent::Error(if stderr.is_empty() {
                                            "gh gist create failed".to_string()
                                        } else {
                                            stderr
                                        }));
                                    }
                                    Err(err) => {
                                        let _ = ev_tx.try_send(DaemonEvent::Error(format!(
                                            "Failed to run gh CLI: {err}"
                                        )));
                                    }
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::SetModel {
                        session,
                        provider,
                        model_id,
                    } => {
                        if let Err(err) = request_checked(
                            &client,
                            "set_model",
                            json!({
                                "activeSessionId": session,
                                "provider": provider,
                                "modelId": model_id,
                            }),
                            Duration::from_secs(15),
                        ) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err));
                        }
                        if let Ok(state) = pa_client::session_state(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::SessionState {
                                session: session.clone(),
                                state,
                            });
                        }
                        if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::SetModelAndThinking {
                        session,
                        provider,
                        model_id,
                        level,
                        switch,
                    } => {
                        if switch {
                            if let Err(err) = request_checked(
                                &client,
                                "set_model",
                                json!({
                                    "activeSessionId": session,
                                    "provider": provider,
                                    "modelId": model_id,
                                }),
                                Duration::from_secs(15),
                            ) {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err));
                                if let Ok(state) = pa_client::session_state(&client, &session) {
                                    let _ = ev_tx.try_send(DaemonEvent::SessionState {
                                        session,
                                        state,
                                    });
                                }
                                continue;
                            }
                        }
                        if let Err(err) = request_checked(
                            &client,
                            "set_thinking_level",
                            json!({"activeSessionId": session, "level": level}),
                            Duration::from_secs(15),
                        ) {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err));
                        }
                        if let Ok(state) = pa_client::session_state(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::SessionState {
                                session: session.clone(),
                                state,
                            });
                        }
                        if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::RefreshModels { session } => {
                        let sid = session.as_deref().unwrap_or("");
                        if let Ok(models) = pa_client::model_catalog(&client, sid) {
                            let _ = ev_tx.try_send(DaemonEvent::Models(models));
                        }
                    }
                    DaemonCmd::RefreshWorkspaces => {
                        match pa_client::list_workspaces(&client) {
                            Ok((workspaces, worktree_root)) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspacesLoaded {
                                    workspaces,
                                    worktree_root,
                                });
                            }
                            Err(err) if !err.to_string().contains("Unknown daemon command") => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                            Err(_) => {}
                        }
                    }
                    DaemonCmd::RegisterWorkspace { path, name } => {
                        match pa_client::register_workspace(&client, &path, name.as_deref()) {
                            Ok((workspace, binding)) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspaceRegistered {
                                    workspace,
                                    binding,
                                });
                                if let Ok((workspaces, worktree_root)) = pa_client::list_workspaces(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::WorkspacesLoaded {
                                        workspaces,
                                        worktree_root,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::RelinkWorkspace { workspace_id, new_path } => {
                        match pa_client::relink_workspace(&client, &workspace_id, &new_path) {
                            Ok(workspace) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspaceRelinked(workspace));
                                if let Ok((workspaces, worktree_root)) = pa_client::list_workspaces(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::WorkspacesLoaded {
                                        workspaces,
                                        worktree_root,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::DeleteWorkspace { workspace_id, delete_harness } => {
                        match pa_client::delete_workspace(&client, &workspace_id, delete_harness) {
                            Ok(_) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspaceDeleted(workspace_id));
                                if let Ok((workspaces, worktree_root)) = pa_client::list_workspaces(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::WorkspacesLoaded {
                                        workspaces,
                                        worktree_root,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::LoadWorktrees { workspace_id } => {
                        match pa_client::list_worktrees(&client, &workspace_id) {
                            Ok(worktrees) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorktreesLoaded {
                                    workspace_id,
                                    worktrees,
                                });
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::CreateWorktree { workspace_id, name, base_branch } => {
                        match pa_client::create_worktree(
                            &client,
                            &workspace_id,
                            &name,
                            base_branch.as_deref(),
                        ) {
                            Ok(worktree) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorktreeCreated {
                                    workspace_id: workspace_id.clone(),
                                    worktree,
                                });
                                if let Ok(worktrees) = pa_client::list_worktrees(&client, &workspace_id) {
                                    let _ = ev_tx.try_send(DaemonEvent::WorktreesLoaded {
                                        workspace_id,
                                        worktrees,
                                    });
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::LoadBranches { worktree_path } => {
                        match pa_client::list_branches(&client, &worktree_path) {
                            Ok(branches) => {
                                let _ = ev_tx.try_send(DaemonEvent::BranchesLoaded {
                                    worktree_path,
                                    branches,
                                });
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::SwitchBranch { worktree_path, branch, active_session_id } => {
                        match pa_client::switch_branch(
                            &client,
                            &worktree_path,
                            &branch,
                            active_session_id.as_deref(),
                        ) {
                            Ok(current) => {
                                let _ = ev_tx.try_send(DaemonEvent::BranchSwitched {
                                    worktree_path: worktree_path.clone(),
                                    branch: current,
                                });
                                if let Ok(branches) = pa_client::list_branches(&client, &worktree_path) {
                                    let _ = ev_tx.try_send(DaemonEvent::BranchesLoaded {
                                        worktree_path,
                                        branches,
                                    });
                                }
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::CreateBranch { worktree_path, branch, start_point, active_session_id } => {
                        match pa_client::create_branch(
                            &client,
                            &worktree_path,
                            &branch,
                            start_point.as_deref(),
                            active_session_id.as_deref(),
                        ) {
                            Ok(current) => {
                                let _ = ev_tx.try_send(DaemonEvent::BranchSwitched {
                                    worktree_path: worktree_path.clone(),
                                    branch: current,
                                });
                                if let Ok(branches) = pa_client::list_branches(&client, &worktree_path) {
                                    let _ = ev_tx.try_send(DaemonEvent::BranchesLoaded {
                                        worktree_path,
                                        branches,
                                    });
                                }
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::BindSessionWorkspace { session, workspace_id, worktree_path } => {
                        match pa_client::bind_session_workspace(
                            &client,
                            &session,
                            workspace_id.as_deref(),
                            worktree_path.as_deref(),
                        ) {
                            Ok(binding) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspaceBindingUpdated {
                                    session,
                                    binding,
                                });
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::SwitchSessionWorktree { session, worktree_path } => {
                        match pa_client::switch_session_worktree(&client, &session, &worktree_path) {
                            Ok(binding) => {
                                let _ = ev_tx.try_send(DaemonEvent::WorkspaceBindingUpdated {
                                    session,
                                    binding,
                                });
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::RefreshSessionWorkspace { session } => {
                        if let Ok(binding) = pa_client::refresh_session_workspace(&client, &session) {
                            let _ = ev_tx.try_send(DaemonEvent::WorkspaceBindingUpdated {
                                session,
                                binding,
                            });
                        }
                    }
                    DaemonCmd::HarnessList {
                        scope,
                        workspace_id,
                        active_session_id,
                        kind,
                        query,
                    } => {
                        match pa_client::harness_list(
                            &client,
                            scope,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                            kind,
                            query.as_deref(),
                        ) {
                            Ok(entries) => {
                                let _ = ev_tx.try_send(DaemonEvent::HarnessEntriesLoaded { entries });
                            }
                            Err(err) if !err.to_string().contains("Unknown daemon command") => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                            Err(_) => {}
                        }
                    }
                    DaemonCmd::HarnessUpsert {
                        scope,
                        workspace_id,
                        active_session_id,
                        entry_id,
                        kind,
                        title,
                        content,
                        path,
                    } => {
                        match pa_client::harness_upsert(
                            &client,
                            scope,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                            entry_id.as_deref(),
                            kind,
                            &title,
                            &content,
                            path.as_deref(),
                        ) {
                            Ok(entry) => {
                                let _ = ev_tx.try_send(DaemonEvent::HarnessEntrySaved(entry));
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::HarnessDelete {
                        scope,
                        workspace_id,
                        active_session_id,
                        kind,
                        entry_id,
                    } => {
                        match pa_client::harness_delete(
                            &client,
                            scope,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                            kind,
                            &entry_id,
                        ) {
                            Ok(_) => {
                                let _ = ev_tx.try_send(DaemonEvent::HarnessEntryDeleted { entry_id });
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::HarnessHistory {
                        scope,
                        workspace_id,
                        active_session_id,
                    } => {
                        match pa_client::harness_history(
                            &client,
                            scope,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                        ) {
                            Ok(items) => {
                                let _ = ev_tx.try_send(DaemonEvent::HarnessHistoryLoaded(items));
                            }
                            Err(err) if !err.to_string().contains("Unknown daemon command") => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                            Err(_) => {}
                        }
                    }
                    DaemonCmd::HarnessRollback {
                        refinement_id,
                        workspace_id,
                        active_session_id,
                    } => {
                        match pa_client::harness_rollback(
                            &client,
                            &refinement_id,
                            workspace_id.as_deref(),
                            active_session_id.as_deref(),
                        ) {
                            Ok(_) => {
                                let _ = ev_tx.try_send(DaemonEvent::HarnessRolledBack);
                            }
                            Err(err) => {
                                let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                }
                drain_and_coalesce(&cmd_rx, &mut ws.pending_cmds);
            }

            if let Some(frame) = client.recv(Duration::from_millis(24)) {
                let _ = ev_tx.try_send(DaemonEvent::LiveFrame(frame));
                while let Some(more) = client.recv(Duration::ZERO) {
                    let _ = ev_tx.try_send(DaemonEvent::LiveFrame(more));
                }
            }

            if !client.alive() {
                let _ = ev_tx.try_send(DaemonEvent::Disconnected(
                    "Daemon disconnected — reconnecting…".into(),
                ));
                thread::sleep(Duration::from_millis(1200));
                break;
            }
        }
    }
}
