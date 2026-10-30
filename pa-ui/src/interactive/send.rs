//! Sending, steering and stopping: what the composer does with Enter, Ctrl+Enter and the button.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn steer(ui: &AppWindow, attachment_model: &Rc<VecModel<slint::Image>>, attachments: &Rc<RefCell<Vec<images::Attachment>>>, job_tx: &Sender<Job>, pending_user: &Rc<RefCell<Option<String>>>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let pending_user = pending_user.clone();
    let attachments = attachments.clone();
    let attachment_model = attachment_model.clone();
    ui.on_steer_requested(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        let message = text.trim().to_string();
        if message.is_empty() {
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        ui.set_composer_text("".into());
        ui.set_composer_has_text(false);
        let blocks: Vec<serde_json::Value> = {
            let mut held = attachments.borrow_mut();
            let blocks = held.iter().map(images::Attachment::block).collect();
            held.clear();
            blocks
        };
        while attachment_model.row_count() > 0 {
            attachment_model.remove(attachment_model.row_count() - 1);
        }
        *pending_user.borrow_mut() = None;
        ui.set_status_line("steered".into());
        let _ = jobs.send(Job::Steer { session, message, images: blocks });
    });
}

pub(crate) fn abort(ui: &AppWindow, job_tx: &Sender<Job>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    ui.on_abort_requested(move || {
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        ui.set_status_line("aborting".into());
        let _ = jobs.send(Job::Abort { session });
    });
}

pub(crate) fn send(ui: &AppWindow, attachment_model: &Rc<VecModel<slint::Image>>, attachments: &Rc<RefCell<Vec<images::Attachment>>>, catalog: &Rc<RefCell<Vec<pa_client::ModelEntry>>>, editing: &Rc<RefCell<Option<(String, usize, String)>>>, in_flight: &Rc<std::cell::Cell<bool>>, job_tx: &Sender<Job>, pending_user: &Rc<RefCell<Option<String>>>, sent_thumbs: &Rc<RefCell<Vec<(String, slint::Image)>>>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let pending_user = pending_user.clone();
    let attachments = attachments.clone();
    let attachment_model = attachment_model.clone();
    let catalog_for_send = catalog.clone();
    let in_flight_for_send = in_flight.clone();
    let sent_thumbs = sent_thumbs.clone();
    let editing_to_send = editing.clone();
    ui.on_send_requested(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        if crate::env_cfg::cfg().debug {
            eprintln!("[send] callback fired with {text:?}");
        }
        let message = text.trim().to_string();
        if message.is_empty() && attachments.borrow().is_empty() {
            return;
        }
        let Some(ui) = weak.upgrade() else { return };
        // Sending is the reader saying "show me the answer": follow again,
        // even if they had scrolled up to read something first
        force_scroll_to_end(&ui);
        // Two ways an image send goes wrong, both refused here: a model that
        // cannot see images drops the whole turn, and an image sent into a
        // running turn arrives with the picture missing.
        if !attachments.borrow().is_empty() {
            let model_id = ui.get_model_id().to_string();
            let provider = ui.get_model_provider().to_string();
            let capable = catalog_for_send
                .borrow()
                .iter()
                .find(|entry| entry.id == model_id && entry.provider == provider)
                .is_none_or(|entry| entry.supports_images);
            let in_flight_now = ui.get_busy() || in_flight_for_send.get();
            if let Some(reason) = image_send_blocker(true, capable, in_flight_now) {
                if crate::env_cfg::cfg().debug {
                    eprintln!("[send] held: {reason}");
                }
                ui.set_status_line(reason.into());
                return;
            }
        }
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            ui.set_status_line("no session selected".into());
            return;
        }

        // The word in front of the caret plus whatever was typed after it.
        let command_word = ui.get_composer_command().to_string();
        let message = if command_word.is_empty() {
            message
        } else if message.trim().is_empty() {
            command_word
        } else {
            format!("{command_word} {message}")
        };
        // A command is only sent when the agent runs it. Everything else in
        // the table is a window action: it either happens here or says
        // plainly that it is not wired, but it never leaves as chat words.
        match slash_dispatch(&message) {
            SlashAction::Send => {}
            SlashAction::Model => {
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_command("".into());
                ui.set_model_menu(true);
                ui.set_status_line("pick a model".into());
                return;
            }
            SlashAction::EffortHint => {
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_command("".into());
                ui.set_status_line("the thinking level opens from the model menu".into());
                return;
            }
            SlashAction::NewChat => {
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_command("".into());
                let _ = jobs.send(Job::NewChat);
                return;
            }
            SlashAction::Quit => {
                let _ = slint::quit_event_loop();
                return;
            }
            SlashAction::Rename(name) => {
                if name.is_empty() {
                    ui.set_status_line("give the chat a name: /name <name>".into());
                    return;
                }
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_command("".into());
                let _ = jobs.send(Job::Rename {
                    session,
                    name,
                });
                return;
            }
            SlashAction::NotWired(name) => {
                // the text stays where it is: nothing is lost, and nothing
                // pretends to have happened
                ui.set_status_line(
                    format!("/{name} belongs to the terminal app and isn't wired here yet")
                        .into(),
                );
                return;
            }
        }
        ui.set_composer_text("".into());
        ui.set_composer_has_text(false);
        ui.set_composer_command("".into());
        {
            let held = attachments.borrow();
            if let Some(first) = held.first() {
                if let Ok(thumb) = slint::Image::load_from_path(&first.path) {
                    let mut thumbs = sent_thumbs.borrow_mut();
                    thumbs.push((message.clone(), thumb));
                    while thumbs.len() > 8 {
                        thumbs.remove(0);
                    }
                }
            }
        }
        let blocks: Vec<serde_json::Value> = {
            let mut held = attachments.borrow_mut();
            let blocks = held.iter().map(images::Attachment::block).collect();
            held.clear();
            blocks
        };
        while attachment_model.row_count() > 0 {
            attachment_model.remove(attachment_model.row_count() - 1);
        }
        // An edit of a parked message replaces it where it stands: taking it
        // out and sending it again put it at the back of the queue, which is
        // not what editing means.
        if let Some((lane, slot, expected)) = editing_to_send.borrow_mut().take() {
            ui.set_status_line("parked message updated".into());
            let _ = jobs.send(Job::QueueEdit {
                session,
                lane,
                slot,
                expected,
                text: message,
                images: blocks,
            });
            return;
        }
        // Where the message belongs depends on whether a turn owns the
        // transcript. While one runs, this is a follow-up: it waits in the
        // queue lanes and only becomes a row once it actually runs. Drawing
        // it in the transcript first was the flash - the bubble appeared,
        // the queue arrived, the row was swept away and everything shifted.
        // Nothing is drawn here. Tthe row appears when the daemon echoes the
        // message - the same way the TUI does it - so the transcript never
        // shows a message the daemon has not accepted, and never has to take
        // one back.
        *pending_user.borrow_mut() = Some(message.clone());
        // The turn is already starting; the poll that reports it is up to a
        // second away, and until then the button still offers to send, so a
        // second press queues instead of stopping. Busy starts here.
        if !ui.get_busy() {
            ui.set_busy_label("Waiting".into());
            ui.set_busy(true);
        }
        if let Some(ui) = weak.upgrade() {
            ui.set_status_line("sent".into());
        }
        in_flight_for_send.set(true);
        let _ = jobs.send(Job::Prompt { session, message, images: blocks });
    });
}
