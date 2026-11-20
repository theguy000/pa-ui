use pa_client::{Client, SessionRow};
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
    Abort { session: String },
}

#[derive(Debug)]
pub enum DaemonEvent {
    Connected,
    Disconnected(String),
    Sessions(Vec<SessionRow>),
    Messages { session: String, messages: Vec<Value> },
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
        let first_id = sessions
            .first()
            .map(|s| if !s.id.is_empty() { s.id.clone() } else { s.session_file.clone() });
        let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
        if let Some(sid) = first_id {
            if let Ok(msgs) = pa_client::attach_messages(&client, &sid) {
                let _ = ev_tx.send(DaemonEvent::Messages { session: sid, messages: msgs });
            }
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
                    match pa_client::attach_messages(&client, &sid) {
                        Ok(messages) => {
                            let _ = ev_tx.send(DaemonEvent::Messages { session: sid, messages });
                        }
                        Err(err) => {
                            let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                        }
                    }
                }
                DaemonCmd::CreateSession => match pa_client::create_session(&client) {
                    Ok(_) => {
                        if let Ok(sessions) = pa_client::list_sessions(&client) {
                            let _ = ev_tx.send(DaemonEvent::Sessions(sessions));
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
                DaemonCmd::Abort { session } => {
                    if let Err(err) = pa_client::abort(&client, &session) {
                        let _ = ev_tx.send(DaemonEvent::Error(err.to_string()));
                    }
                }
            }
        }

        if let Some(frame) = client.recv(Duration::from_millis(32)) {
            let _ = ev_tx.send(DaemonEvent::LiveFrame(frame));
        }

        if !client.alive() {
            let _ = ev_tx.send(DaemonEvent::Disconnected("daemon connection closed".into()));
            break;
        }
    }
}
