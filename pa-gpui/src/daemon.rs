use pa_client::{Client, ModelEntry, SessionRow, SessionStats};
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Debug)]
pub enum DaemonCmd {
    RefreshSessions,
    SelectSession(String),
    CreateSession,
    SendPrompt { session: String, text: String },
    SteerPrompt { session: String, text: String },
    Abort { session: String },
    MarkDone { session: String },
    DeleteSaved { session_path: String },
    SetModel { session: String, provider: String, model_id: String },
    SetThinking { session: String, level: String },
}

#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected(String),
    Sessions(Vec<SessionRow>),
    Messages { session: String, messages: Vec<Value> },
    Models(Vec<ModelEntry>),
    Stats { session: String, stats: SessionStats },
    LiveFrame(Value),
    Error(String),
}

pub struct DaemonBridge {
    pub tx: Sender<DaemonCmd>,
    pub rx: Receiver<DaemonEvent>,
}

impl DaemonBridge {
    pub fn spawn() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DaemonCmd>();
        let (ev_tx, ev_rx) = mpsc::channel::<DaemonEvent>();

        thread::spawn(move || run_worker(cmd_rx, ev_tx));

        Self { tx: cmd_tx, rx: ev_rx }
    }
}

fn load_session_details(client: &Client, sid: &str, ev_tx: &Sender<DaemonEvent>) {
    match pa_client::attach_messages(client, sid) {
        Ok(messages) => {
            let _ = ev_tx.send(DaemonEvent::Messages {
                session: sid.to_string(),
                messages,
            });
        }
        Err(err) => {
            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
        }
    }
    if let Ok(models) = pa_client::model_catalog(client, sid) {
        let _ = ev_tx.send(DaemonEvent::Models(models));
    }
    if let Ok(stats) = pa_client::session_stats(client, sid) {
        let _ = ev_tx.send(DaemonEvent::Stats {
            session: sid.to_string(),
            stats,
        });
    }
}

fn run_worker(cmd_rx: Receiver<DaemonCmd>, ev_tx: Sender<DaemonEvent>) {
    let client = match Client::connect() {
        Ok(c) => {
            let _ = ev_tx.send(DaemonEvent::Connected);
            c
        }
        Err(err) => {
            let _ = ev_tx.send(DaemonEvent::Disconnected(err.to_string()));
            return;
        }
    };

    if let Ok(sessions) = pa_client::list_sessions(&client) {
        let first_id = sessions.first().map(|s| {
            if !s.id.is_empty() {
                s.id.clone()
            } else {
                s.session_file.clone()
            }
        });
        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
        if let Some(sid) = first_id {
            load_session_details(&client, &sid, &ev_tx);
        }
    }

    loop {
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                DaemonCmd::RefreshSessions => match pa_client::list_sessions(&client) {
                    Ok(sessions) => {
                        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                    }
                    Err(err) => {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                },
                DaemonCmd::SelectSession(sid) => {
                    load_session_details(&client, &sid, &ev_tx);
                }
                DaemonCmd::CreateSession => match pa_client::create_session(&client) {
                    Ok(_) => {
                        if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let new_first = sessions.first().map(|s| {
                                if !s.id.is_empty() {
                                    s.id.clone()
                                } else {
                                    s.session_file.clone()
                                }
                            });
                            let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                            if let Some(sid) = new_first {
                                load_session_details(&client, &sid, &ev_tx);
                            }
                        }
                    }
                    Err(err) => {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                },
                DaemonCmd::SendPrompt { session, text } => {
                    if let Err(err) = pa_client::send_prompt(&client, &session, &text, &[]) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                }
                DaemonCmd::SteerPrompt { session, text } => {
                    if let Err(err) = pa_client::steer_prompt(&client, &session, &text, &[]) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                }
                DaemonCmd::Abort { session } => {
                    if let Err(err) = pa_client::abort(&client, &session) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                }
                DaemonCmd::MarkDone { session } => {
                    if let Err(err) = pa_client::mark_done(&client, &session) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                    }
                }
                DaemonCmd::DeleteSaved { session_path } => {
                    if let Err(err) = pa_client::delete_saved_session(&client, &session_path) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                    }
                }
                DaemonCmd::SetModel {
                    session,
                    provider,
                    model_id,
                } => {
                    if let Err(err) = pa_client::set_model(&client, &session, &provider, &model_id) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                    }
                }
                DaemonCmd::SetThinking { session, level } => {
                    if let Err(err) = pa_client::set_thinking_level(&client, &session, &level) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                    }
                }
            }
        }

        if let Some(frame) = client.recv(Duration::from_millis(24)) {
            let _ = ev_tx.send(DaemonEvent::LiveFrame(frame));
        }

        if !client.alive() {
            let _ = ev_tx.send(DaemonEvent::Disconnected("daemon connection closed".into()));
            break;
        }
    }
}
