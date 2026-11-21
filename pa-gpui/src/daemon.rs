use async_channel::{Receiver as AsyncReceiver, Sender as AsyncSender};
use pa_client::{Client, ModelEntry, SessionRow, SessionState, SessionStats};
use serde_json::{json, Value};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum DaemonCmd {
    RefreshSessions,
    SelectSession(String),
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
    SetModel { session: String, provider: String, model_id: String },
    SetModelAndThinking {
        session: String,
        provider: String,
        model_id: String,
        level: String,
        switch: bool,
    },
}

#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected(String),
    Sessions(Vec<SessionRow>),
    SessionCreated(String),
    Messages {
        session: String,
        messages: Vec<Value>,
        first_entry: String,
        more: bool,
    },
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
    Notice(String),
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

fn load_session_details(
    client: &Client,
    sid: &str,
    client_id: &str,
    ev_tx: &AsyncSender<DaemonEvent>,
    models_loaded: &mut bool,
) {
    match pa_client::attach_as(client, sid, Some(client_id), None) {
        Ok(fallback) => {
            let (messages, first_entry, more) =
                match pa_client::get_page(client, sid, None, TRANSCRIPT_PAGE_LIMIT) {
                    Ok(page) => (page.messages, page.first_entry, page.more),
                    Err(_) => (fallback, String::new(), false),
                };
            let _ = ev_tx.try_send(DaemonEvent::Messages {
                session: sid.to_string(),
                messages,
                first_entry,
                more,
            });
        }
        Err(err) => {
            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
        }
    }
    if let Ok(sessions) = pa_client::list_sessions(client) {
        let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
    }
    if !*models_loaded {
        if let Ok(models) = pa_client::model_catalog(client, sid) {
            *models_loaded = true;
            let _ = ev_tx.try_send(DaemonEvent::Models(models));
        }
    }
    if let Ok(state) = pa_client::session_state(client, sid) {
        let _ = ev_tx.try_send(DaemonEvent::SessionState {
            session: sid.to_string(),
            state,
        });
    }
    if let Ok(stats) = pa_client::session_stats(client, sid) {
        let _ = ev_tx.try_send(DaemonEvent::Stats {
            session: sid.to_string(),
            stats,
        });
    }
    if let Ok((steer, follow_up)) = pa_client::get_queue(client, sid) {
        let _ = ev_tx.try_send(DaemonEvent::Queue {
            session: sid.to_string(),
            steer,
            follow_up,
        });
    }
    let subagents = pa_client::list_roster(client)
        .map(|roster| pa_client::running_subagents(&roster, sid, ""))
        .unwrap_or(0);
    let shells = pa_client::running_shells(client, sid).unwrap_or(0);
    let heartbeats = pa_client::scoped_heartbeat_count(client, sid, "").unwrap_or(0);
    let _ = ev_tx.try_send(DaemonEvent::Activity {
        session: sid.to_string(),
        subagents,
        shells,
        heartbeats,
    });
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

        let mut models_loaded = false;
        let client_id = format!("pa-gpui-{}", std::process::id());

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
                load_session_details(&client, &sid, &client_id, &ev_tx, &mut models_loaded);
            }
        }

        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    DaemonCmd::RefreshSessions => match pa_client::list_sessions(&client) {
                        Ok(sessions) => {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
                        Err(err) => {
                            let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                        }
                    },
                    DaemonCmd::SelectSession(sid) => {
                        load_session_details(&client, &sid, &client_id, &ev_tx, &mut models_loaded);
                    }
                    DaemonCmd::ReloadTranscript(sid) => {
                        if let Ok(page) =
                            pa_client::get_page(&client, &sid, None, TRANSCRIPT_PAGE_LIMIT)
                        {
                            let _ = ev_tx.try_send(DaemonEvent::TailRefresh {
                                session: sid.clone(),
                                messages: page.messages,
                                first_entry: page.first_entry,
                                more: page.more,
                            });
                        }
                        if let Ok(stats) = pa_client::session_stats(&client, &sid) {
                            let _ = ev_tx.try_send(DaemonEvent::Stats {
                                session: sid.clone(),
                                stats,
                            });
                        }
                        if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &sid) {
                            let _ = ev_tx.try_send(DaemonEvent::Queue {
                                session: sid.clone(),
                                steer,
                                follow_up,
                            });
                        }
                        if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                        }
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
                    } => {
                        let mut config = serde_json::Map::new();
                        if let Some((provider, model_id)) = model {
                            config.insert("provider".into(), Value::String(provider));
                            config.insert("model".into(), Value::String(model_id));
                        }
                        if let Some(level) = level {
                            config.insert("thinking".into(), Value::String(level));
                        }
                        let mut create = json!({});
                        if !config.is_empty() {
                            create["config"] = Value::Object(config);
                        }
                        match pa_client::create_session_with(&client, create) {
                            Ok(new_sid) => {
                                let _ = ev_tx.try_send(DaemonEvent::SessionCreated(new_sid.clone()));
                                if let Err(err) =
                                    pa_client::attach_as(&client, &new_sid, Some(&client_id), None)
                                {
                                    let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                                    continue;
                                }
                                if let Err(err) =
                                    pa_client::send_prompt(&client, &new_sid, &text, &images)
                                {
                                    let _ = ev_tx.try_send(DaemonEvent::Error(err.to_string()));
                                }
                                if let Ok(sessions) = pa_client::list_sessions(&client) {
                                    let _ = ev_tx.try_send(DaemonEvent::Sessions(sessions));
                                }
                                if !models_loaded {
                                    if let Ok(models) = pa_client::model_catalog(&client, &new_sid) {
                                        models_loaded = true;
                                        let _ = ev_tx.try_send(DaemonEvent::Models(models));
                                    }
                                }
                                if let Ok(state) = pa_client::session_state(&client, &new_sid) {
                                    let _ = ev_tx.try_send(DaemonEvent::SessionState {
                                        session: new_sid.clone(),
                                        state,
                                    });
                                }
                                if let Ok(stats) = pa_client::session_stats(&client, &new_sid) {
                                    let _ = ev_tx.try_send(DaemonEvent::Stats {
                                        session: new_sid,
                                        stats,
                                    });
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
                                let _ = ev_tx
                                    .try_send(DaemonEvent::Notice(format!("Exported: {path}")));
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
                }
            }

            if let Some(frame) = client.recv(Duration::from_millis(24)) {
                let _ = ev_tx.try_send(DaemonEvent::LiveFrame(frame));
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
