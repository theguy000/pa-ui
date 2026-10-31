//! Sending, steering and stopping: what the composer does with Enter, Ctrl+Enter and the button.

use crate::*;

pub(crate) fn steer(ui: &AppWindow, attachment_model: &Rc<VecModel<slint::Image>>, attachments: &Rc<RefCell<Vec<images::Attachment>>>, job_tx: &Sender<Job>, pending_user: &Rc<RefCell<Option<String>>>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let pending_user = pending_user.clone();
    let attachments = attachments.clone();
    let attachment_model = attachment_model.clone();
    ui.on_steer_requested(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        // The gold word in front of the caret is half the message: a command
        // steered into the running turn has to carry it or it steers nothing.
        let message = compose_message(&ui.get_composer_command(), &text);
        if message.is_empty() {
            return;
        }
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        ui.set_composer_text("".into());
        ui.set_composer_has_text(false);
        ui.set_composer_command("".into());
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
        let _ = jobs.send(Job::Abort { session });
    });
}

/// Everything a send needs a handle to. A struct rather than a parameter list
/// nine names long, but still the dependency list: nothing reaches the window
/// that is not named here.
pub(crate) struct Handles<'a> {
    pub attachment_model: &'a Rc<VecModel<slint::Image>>,
    pub attachments: &'a Rc<RefCell<Vec<images::Attachment>>>,
    pub catalog: &'a Rc<RefCell<Vec<pa_client::ModelEntry>>>,
    pub editing: &'a Editing,
    pub in_flight: &'a Rc<std::cell::Cell<bool>>,
    pub job_tx: &'a Sender<Job>,
    pub pending_user: &'a Rc<RefCell<Option<String>>>,
    pub sent_thumbs: &'a SentThumbs,
}

pub(crate) fn send(ui: &AppWindow, handles: &Handles<'_>) {
    let weak = ui.as_weak();
    let jobs = handles.job_tx.clone();
    let pending_user = handles.pending_user.clone();
    let attachments = handles.attachments.clone();
    let attachment_model = handles.attachment_model.clone();
    let catalog_for_send = handles.catalog.clone();
    let in_flight_for_send = handles.in_flight.clone();
    let sent_thumbs = handles.sent_thumbs.clone();
    let editing_to_send = handles.editing.clone();
    ui.on_send_requested(move |text| {
        let Some(ui) = weak.upgrade() else { return };
        if crate::env_cfg::cfg().debug {
            eprintln!("[send] callback fired with {text:?}");
        }
        // The command word in front of the caret and whatever was typed after
        // it are one message, so the guard has to read both: a bare
        // `/compact` leaves the box empty and was dropped here before the word
        // was ever looked at, from Enter and from the Send button alike.
        let message = compose_message(&ui.get_composer_command(), &text);
        if message.is_empty() && attachments.borrow().is_empty() {
            return;
        }
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
                status::trouble(&ui, reason);
                return;
            }
        }
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            status::trouble(&ui, "no session selected");
            return;
        }
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
                status::trouble(&ui, "pick a model");
                return;
            }
            SlashAction::EffortHint => {
                ui.set_composer_text("".into());
                ui.set_composer_has_text(false);
                ui.set_composer_command("".into());
                status::trouble(&ui, "the thinking level opens from the model menu");
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
                    status::trouble(&ui, "give the chat a name: /name <name>");
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
                status::trouble(
                    &ui,
                    format!("/{name} belongs to the terminal app and isn't wired here yet"),
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
                let thumb = images::from_wire(&first.base64);
                if thumb.size().width > 0 {
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
        status::clear(&ui);
        in_flight_for_send.set(true);
        let _ = jobs.send(Job::Prompt { session, message, images: blocks });
    });
}
