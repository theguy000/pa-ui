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
    RenameSaved { session_path: String, name: String },
    ClearQueue { session: String },
    ExportHtml { session: String },
    SetModel { session: String, provider: String, model_id: String },
    SetThinking { session: String, level: String },
}

#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected(String),
    Sessions(Vec<SessionRow>),
    SessionCreated(String),
    Messages { session: String, messages: Vec<Value> },
    Models(Vec<ModelEntry>),
    Stats { session: String, stats: SessionStats },
    Queue { session: String, steer: Vec<String>, follow_up: Vec<String> },
    Activity { session: String, subagents: usize, shells: usize, heartbeats: usize },
    Notice(String),
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

        thread::spawn(move || run_worker_loop(cmd_rx, ev_tx));

        Self { tx: cmd_tx, rx: ev_rx }
    }
}

fn load_session_details(
    client: &Client,
    sid: &str,
    ev_tx: &Sender<DaemonEvent>,
    models_loaded: &mut bool,
) {
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
    if !*models_loaded {
        if let Ok(models) = pa_client::model_catalog(client, sid) {
            *models_loaded = true;
            let _ = ev_tx.send(DaemonEvent::Models(models));
        }
    }
    if let Ok(stats) = pa_client::session_stats(client, sid) {
        let _ = ev_tx.send(DaemonEvent::Stats {
            session: sid.to_string(),
            stats,
        });
    }
    if let Ok((steer, follow_up)) = pa_client::get_queue(client, sid) {
        let _ = ev_tx.send(DaemonEvent::Queue {
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
    let _ = ev_tx.send(DaemonEvent::Activity {
        session: sid.to_string(),
        subagents,
        shells,
        heartbeats,
    });
}

fn run_worker_loop(cmd_rx: Receiver<DaemonCmd>, ev_tx: Sender<DaemonEvent>) {
    loop {
        let client = match Client::connect() {
            Ok(c) => {
                let _ = ev_tx.send(DaemonEvent::Connected);
                c
            }
            Err(err) => {
                let _ = ev_tx.send(DaemonEvent::Disconnected(format!(
                    "Reconnecting… ({err})"
                )));
                while cmd_rx.try_recv().is_ok() {}
                thread::sleep(Duration::from_millis(1500));
                continue;
            }
        };

        let mut models_loaded = false;

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
                load_session_details(&client, &sid, &ev_tx, &mut models_loaded);
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
                        load_session_details(&client, &sid, &ev_tx, &mut models_loaded);
                    }
                    DaemonCmd::CreateSession => match pa_client::create_session(&client) {
                        Ok(new_sid) => {
                            let _ = ev_tx.send(DaemonEvent::SessionCreated(new_sid.clone()));
                            let _ = ev_tx.send(DaemonEvent::Messages {
                                session: new_sid.clone(),
                                messages: Vec::new(),
                            });
                            let _ = ev_tx.send(DaemonEvent::Queue {
                                session: new_sid.clone(),
                                steer: Vec::new(),
                                follow_up: Vec::new(),
                            });
                            let _ = ev_tx.send(DaemonEvent::Activity {
                                session: new_sid.clone(),
                                subagents: 0,
                                shells: 0,
                                heartbeats: 0,
                            });
                            if let Ok(sessions) = pa_client::list_sessions(&client) {
                                let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                            }
                            if !models_loaded {
                                if let Ok(models) = pa_client::model_catalog(&client, &new_sid) {
                                    models_loaded = true;
                                    let _ = ev_tx.send(DaemonEvent::Models(models));
                                }
                            }
                            if let Ok(stats) = pa_client::session_stats(&client, &new_sid) {
                                let _ = ev_tx.send(DaemonEvent::Stats {
                                    session: new_sid,
                                    stats,
                                });
                            }
                        }
                        Err(err) => {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        }
                    },
                    DaemonCmd::SendPrompt { session, text } => {
                        if let Err(err) = pa_client::send_prompt(&client, &session, &text, &[]) {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.send(DaemonEvent::Queue {
                                session,
                                steer,
                                follow_up,
                            });
                        }
                    }
                    DaemonCmd::SteerPrompt { session, text } => {
                        if let Err(err) = pa_client::steer_prompt(&client, &session, &text, &[]) {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok((steer, follow_up)) = pa_client::get_queue(&client, &session) {
                            let _ = ev_tx.send(DaemonEvent::Queue {
                                session,
                                steer,
                                follow_up,
                            });
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
                    DaemonCmd::RenameSaved { session_path, name } => {
                        if let Err(err) =
                            pa_client::rename_saved_session(&client, &session_path, &name)
                        {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::ClearQueue { session } => {
                        if let Err(err) = pa_client::clear_queue(&client, &session) {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        } else {
                            let _ = ev_tx.send(DaemonEvent::Queue {
                                session,
                                steer: Vec::new(),
                                follow_up: Vec::new(),
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
                                let _ = ev_tx.send(DaemonEvent::Notice(format!("Exported: {path}")));
                            }
                            Err(err) => {
                                let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                            }
                        }
                    }
                    DaemonCmd::SetModel {
                        session,
                        provider,
                        model_id,
                    } => {
                        if let Err(err) =
                            pa_client::set_model(&client, &session, &provider, &model_id)
                        {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        } else if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
                        }
                    }
                    DaemonCmd::SetThinking { session, level } => {
                        if let Err(err) =
                            pa_client::set_thinking_level(&client, &session, &level)
                        {
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
                let _ = ev_tx.send(DaemonEvent::Disconnected(
                    "Daemon disconnected — reconnecting…".into(),
                ));
                thread::sleep(Duration::from_millis(1200));
                break;
            }
        }
    }
}
