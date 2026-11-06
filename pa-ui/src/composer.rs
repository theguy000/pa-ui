//! The composer: the draft, command word, slash menu, image attachments,
//! custom canvas painter, and parked-message queue lanes.

use crate::*;

/// Why an image send cannot go out right now, if it cannot.
pub(crate) fn image_send_blocker(
    has_images: bool,
    model_capable: bool,
    turn_in_flight: bool,
) -> Option<&'static str> {
    if !has_images {
        return None;
    }
    if !model_capable {
        return Some(
            "this model cannot take images - switch to a vision model with /model, then send",
        );
    }
    let _ = turn_in_flight;
    None
}

/// The words a send carries: the command word that sits in front of the caret
/// (`/compact`, taken from the menu) and whatever the reader typed after it.
pub(crate) fn compose_message(command_word: &str, typed: &str) -> String {
    let typed = typed.trim();
    if command_word.is_empty() {
        typed.to_string()
    } else if typed.is_empty() {
        command_word.to_string()
    } else {
        format!("{command_word} {typed}")
    }
}

/// Project the steering and follow-up lanes onto the window's queue items.
pub(crate) fn apply_queue(
    ui: &AppWindow,
    steering: &[String],
    follow_up: &[String],
    sent_thumbs: &SentThumbs,
) {
    if crate::env_cfg::cfg().debug {
        eprintln!(
            "[queue] projection steering={} follow_up={steering:?}->{follow_up:?}",
            steering.len()
        );
    }
    let total = steering.len() + follow_up.len();
    ui.set_queued_count(total as i32);

    let lane_items = |texts: &[String]| -> Vec<QueueItem> {
        texts
            .iter()
            .enumerate()
            .map(|(slot, text)| {
                let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
                let cut: String = flat.chars().take(120).collect();
                let thumb = sent_thumbs
                    .borrow()
                    .iter()
                    .rev()
                    .find(|(sent, _)| sent.trim() == text.trim())
                    .map(|(_, thumb)| thumb.clone());
                QueueItem {
                    text: cut.into(),
                    raw: text.clone().into(),
                    slot: slot as i32,
                    thumb: thumb.unwrap_or_default(),
                }
            })
            .collect()
    };
    ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::from(lane_items(steering)))));
    ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::from(lane_items(follow_up)))));
}

pub(crate) struct Composer {
    pub in_flight: Rc<std::cell::Cell<bool>>,
    pub pending_user: Rc<RefCell<Option<String>>>,
    command_word: CommandWord,
    pub editing: Editing,
    pub extra_commands: SlashCommands,
    pub sent_thumbs: SentThumbs,
    slash_token: Rc<RefCell<(usize, usize)>>,
    attachments: Rc<RefCell<Vec<images::Attachment>>>,
    attachment_model: Rc<VecModel<slint::Image>>,
    repaint_canvas: Repaint,
}

impl Composer {
    pub fn new(
        ui: &AppWindow,
        job_tx: &Sender<Job>,
        models: &Rc<ModelMenu>,
        command_word: CommandWord,
    ) -> Rc<Self> {
        let in_flight = Rc::new(std::cell::Cell::new(false));
        let pending_user = Rc::new(RefCell::new(None));
        let editing: Editing = Rc::new(RefCell::new(None));
        let extra_commands: SlashCommands = Rc::new(RefCell::new(Vec::new()));
        let slash_token = Rc::new(RefCell::new((0usize, 0usize)));
        let sent_thumbs: SentThumbs = Rc::new(RefCell::new(Vec::new()));
        let attachments: Rc<RefCell<Vec<images::Attachment>>> = Rc::new(RefCell::new(Vec::new()));
        let attachment_model = Rc::new(VecModel::<slint::Image>::from(Vec::new()));
        ui.set_attachment_images(ModelRc::from(attachment_model.clone()));

        let canvas_cell: Rc<RefCell<Option<composer_canvas::Canvas>>> = Rc::new(RefCell::new(None));
        let canvas_key: Rc<RefCell<(String, usize, i32)>> =
            Rc::new(RefCell::new((String::new(), usize::MAX, i32::MIN)));
        let repaint_canvas = painter(&canvas_cell, &canvas_key, &extra_commands, true);
        let repaint_for_accept = painter(&canvas_cell, &canvas_key, &extra_commands, false);

        let composer = Rc::new(Self {
            in_flight,
            pending_user,
            command_word,
            editing,
            extra_commands,
            sent_thumbs,
            slash_token,
            attachments,
            attachment_model,
            repaint_canvas,
        });
        composer.bind_ui(ui, job_tx, models, &repaint_for_accept);
        composer
    }

    pub fn repaint(&self, ui: &AppWindow) {
        (self.repaint_canvas)(ui);
    }

    pub fn project_queue(&self, ui: &AppWindow, steering: &[String], follow_up: &[String]) {
        apply_queue(ui, steering, follow_up, &self.sent_thumbs);
    }

    pub fn set_extra_commands(&self, rows: Vec<(String, String, String)>) {
        *self.extra_commands.borrow_mut() = rows;
    }

    pub fn reset_for_new_chat(&self, ui: &AppWindow, id: &str) {
        self.in_flight.set(false);
        if let Ok(opening) = std::env::var("PA_SLASH") {
            ui.set_composer_text(opening.as_str().into());
            ui.set_composer_has_text(!opening.is_empty());
            let opening_text = ui.get_composer_text().to_string();
            if let Some(name) = opening_text.trim().strip_prefix('/') {
                let rows = slash_rows(name, &self.extra_commands.borrow());
                if !rows.is_empty() {
                    ui.set_slash_items(ModelRc::from(Rc::new(VecModel::from(rows))));
                    ui.set_slash_open(true);
                }
            }
        }
        ui.set_composer_text("".into());
        ui.set_composer_has_text(false);
        ui.set_queued_count(0);
        ui.set_queued_steering(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
        ui.set_queued_followup(ModelRc::from(Rc::new(VecModel::<QueueItem>::from(Vec::new()))));
        ui.set_busy(false);
        ui.set_selected_id(id.into());
        if crate::env_cfg::cfg().debug {
            eprintln!("[new] selected {id}");
        }
    }

    pub fn on_prompt_rejected(&self, ui: &AppWindow, sheet: &sheet::Sheet, text: &str) {
        self.in_flight.set(false);
        if ui.get_composer_text().is_empty() {
            ui.set_composer_text(text.into());
            ui.set_composer_has_text(!ui.get_composer_text().is_empty());
        }
        self.pending_user.borrow_mut().take();
        sheet.take_back_prompt(text);
    }

    pub fn on_prompt_result(&self, sheet: &sheet::Sheet, error: Option<String>) {
        if error.is_some() {
            self.in_flight.set(false);
            let text = self.pending_user.borrow_mut().take().unwrap_or_default();
            sheet.take_back_prompt(&text);
        }
    }

    fn bind_ui(
        &self,
        ui: &AppWindow,
        job_tx: &Sender<Job>,
        models: &Rc<ModelMenu>,
        repaint_for_accept: &Repaint,
    ) {
        // 1. Send
        {
            let weak = ui.as_weak();
            let jobs = job_tx.clone();
            let pending_user = self.pending_user.clone();
            let command_word = self.command_word.clone();
            let attachments = self.attachments.clone();
            let attachment_model = self.attachment_model.clone();
            let models = models.clone();
            let in_flight_for_send = self.in_flight.clone();
            let sent_thumbs = self.sent_thumbs.clone();
            let editing_to_send = self.editing.clone();
            ui.on_send_requested(move |text| {
                let Some(ui) = weak.upgrade() else { return };
                if crate::env_cfg::cfg().debug {
                    eprintln!("[send] callback fired with {text:?}");
                }
                let message = compose_message(&ui.get_composer_command(), &text);
                if message.is_empty() && attachments.borrow().is_empty() {
                    return;
                }
                force_scroll_to_end(&ui);
                if !attachments.borrow().is_empty() {
                    let model_id = ui.get_model_id().to_string();
                    let provider = ui.get_model_provider().to_string();
                    let capable = models.supports_images(&provider, &model_id);
                    let in_flight_now = ui.get_busy() || in_flight_for_send.get();
                    if let Some(reason) = image_send_blocker(true, capable, in_flight_now) {
                        if crate::env_cfg::cfg().debug {
                            eprintln!("[send] held: {reason}");
                        }
                        return;
                    }
                }
                let session = ui.get_selected_id().to_string();
                if session.is_empty() {
                    return;
                }
                match slash_dispatch(&message) {
                    SlashAction::Send => {}
                    SlashAction::Model => {
                        ui.set_composer_text("".into());
                        ui.set_composer_has_text(false);
                        ui.set_composer_command("".into());
                        ui.set_model_menu(true);
                        return;
                    }
                    SlashAction::EffortHint => {
                        ui.set_composer_text("".into());
                        ui.set_composer_has_text(false);
                        ui.set_composer_command("".into());
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
                            return;
                        }
                        ui.set_composer_text("".into());
                        ui.set_composer_has_text(false);
                        ui.set_composer_command("".into());
                        let _ = jobs.send(Job::Rename { session, name });
                        return;
                    }
                    SlashAction::NotWired(_) => {
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
                *pending_user.borrow_mut() = Some(message.clone());
                // A send while a turn runs is a parked message: it has no
                // status word yet. The word starts when the message does.
                if !ui.get_busy() {
                    command_word.clear();
                    if let Some(label) = command_word.hold(&message) {
                        ui.set_busy_label(label.into());
                        ui.set_busy(true);
                    } else {
                        ui.set_busy_label("Waiting".into());
                        ui.set_busy(true);
                    }
                }
                in_flight_for_send.set(true);
                let _ = jobs.send(Job::Prompt {
                    session,
                    message,
                    images: blocks,
                });
            });
        }

        // 2. Steer
        {
            let weak = ui.as_weak();
            let jobs = job_tx.clone();
            let pending_user = self.pending_user.clone();
            let attachments = self.attachments.clone();
            let attachment_model = self.attachment_model.clone();
            ui.on_steer_requested(move |text| {
                let Some(ui) = weak.upgrade() else { return };
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
                let _ = jobs.send(Job::Steer {
                    session,
                    message,
                    images: blocks,
                });
            });
        }

        // 3. Abort
        {
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

        // 4. Attachments pick, paste, remove, and drain
        {
            let (attach_tx, attach_rx) = std::sync::mpsc::channel::<Vec<images::Attachment>>();
            let attach_rx = Rc::new(attach_rx);
            let tx_pick = attach_tx.clone();
            ui.on_attach_requested(move || {
                let tx = tx_pick.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(images::pick_files());
                });
            });
            let tx_paste = attach_tx;
            ui.on_attach_pasted(move || {
                if crate::env_cfg::cfg().debug {
                    eprintln!("[attach] paste chord");
                }
                if !images::clipboard_has_image() {
                    return false;
                }
                let tx = tx_paste.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(images::read_clipboard().into_iter().collect());
                });
                true
            });
            let attachments_rem = self.attachments.clone();
            let model_rem = self.attachment_model.clone();
            ui.on_attachment_removed(move |index| {
                let index = usize::try_from(index).unwrap_or(usize::MAX);
                let mut held = attachments_rem.borrow_mut();
                if index < held.len() {
                    held.remove(index);
                    model_rem.remove(index);
                }
            });
            let attachments_drain = self.attachments.clone();
            let model_drain = self.attachment_model.clone();
            let drain = Timer::default();
            drain.start(TimerMode::Repeated, Duration::from_millis(120), move || {
                while let Ok(found) = attach_rx.try_recv() {
                    let mut held = attachments_drain.borrow_mut();
                    for attachment in found {
                        let thumb = images::from_wire(&attachment.base64);
                        if thumb.size().width > 0 && thumb.size().height > 0 {
                            if crate::env_cfg::cfg().debug {
                                eprintln!(
                                    "[attach] {} ({} base64 chars) {}x{}",
                                    attachment.mime,
                                    attachment.base64.len(),
                                    thumb.size().width,
                                    thumb.size().height
                                );
                            }
                            held.push(attachment);
                            model_drain.push(thumb);
                        } else {
                            eprintln!("[attach] no thumbnail: decode failed");
                        }
                    }
                }
            });
            std::mem::forget(drain);
        }

        // 5. Slash command callbacks
        {
            let weak = ui.as_weak();
            ui.on_composer_command_cleared(move || {
                let Some(ui) = weak.upgrade() else { return };
                if crate::env_cfg::cfg().debug {
                    eprintln!("[composer] command word removed");
                }
                ui.set_composer_command("".into());
            });
        }
        {
            let weak = ui.as_weak();
            let extra_commands = self.extra_commands.clone();
            let token = self.slash_token.clone();
            let paint = self.repaint_canvas.clone();
            ui.on_slash_typed(move |text, caret| {
                let Some(ui) = weak.upgrade() else { return };
                paint(&ui);
                let (start, end) =
                    slash_token_at(&text, usize::try_from(caret.max(0)).unwrap_or(0));
                *token.borrow_mut() = (start, end);
                let body = text.get(start..end).unwrap_or_default();
                let Some(prefix) = body
                    .strip_prefix('/')
                    .filter(|name| !name.contains(char::is_whitespace) && !name.contains('/'))
                else {
                    ui.set_slash_open(false);
                    return;
                };
                let rows = slash_rows(prefix, &extra_commands.borrow());
                if rows.is_empty() {
                    ui.set_slash_open(false);
                    return;
                }
                ui.set_slash_items(ModelRc::from(Rc::new(VecModel::from(rows))));
                ui.set_slash_index(0);
                ui.set_slash_open(true);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_slash_move(move |delta| {
                let Some(ui) = weak.upgrade() else { return };
                let count = ui.get_slash_items().row_count() as i32;
                if !ui.get_slash_open() || count == 0 {
                    return;
                }
                let next = (ui.get_slash_index() + delta).rem_euclid(count);
                ui.set_slash_index(next);
            });
        }
        {
            let weak = ui.as_weak();
            ui.on_slash_dismiss(move || {
                if let Some(ui) = weak.upgrade() {
                    ui.set_slash_open(false);
                }
            });
        }
        {
            let weak = ui.as_weak();
            let repaint_for_accept = repaint_for_accept.clone();
            let token = self.slash_token.clone();
            ui.on_slash_accept(move || {
                let Some(ui) = weak.upgrade() else { return };
                let index = ui.get_slash_index().max(0) as usize;
                let rows = ui.get_slash_items();
                let Some(row) = rows.row_data(index) else {
                    ui.set_slash_open(false);
                    return;
                };
                ui.set_slash_open(false);
                let name = row.name.to_string();
                if name == "model" {
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_model_menu(true);
                    return;
                }
                let text = ui.get_composer_text().to_string();
                let (start, end) = *token.borrow();
                let (start, end) = (start.min(text.len()), end.min(text.len()));
                if !ui.get_canvas_mode() && start == 0 && end == text.len() {
                    ui.set_composer_command(format!("/{name}").as_str().into());
                    ui.set_composer_text("".into());
                    ui.set_composer_has_text(false);
                    ui.set_composer_caret_request(0);
                    return;
                }
                let mut next = String::with_capacity(text.len() + name.len() + 2);
                next.push_str(&text[..start]);
                next.push('/');
                next.push_str(&name);
                next.push(' ');
                next.push_str(&text[end..]);
                ui.set_composer_text(next.as_str().into());
                ui.set_composer_has_text(true);
                ui.set_composer_caret_request(i32::try_from(start + name.len() + 2).unwrap_or(0));
                repaint_for_accept(&ui);
            });
        }

        // 6. Queue lane actions & clear
        {
            let weak = ui.as_weak();
            let editing = self.editing.clone();
            let jobs = job_tx.clone();
            ui.on_queue_act(move |lane, slot, action| {
                if crate::env_cfg::cfg().debug {
                    eprintln!("[queue] act {action} {lane}[{slot}]");
                }
                let Some(ui) = weak.upgrade() else { return };
                let session = ui.get_selected_id().to_string();
                if session.is_empty() {
                    return;
                }
                let text = if lane == "steering" {
                    ui.get_queued_steering()
                        .row_data(slot.max(0) as usize)
                        .map(|row| row.raw.to_string())
                } else {
                    ui.get_queued_followup()
                        .row_data(slot.max(0) as usize)
                        .map(|row| row.raw.to_string())
                };
                let Some(text) = text else { return };
                if action == "edit" {
                    *editing.borrow_mut() =
                        Some((lane.to_string(), slot.max(0) as usize, text.clone()));
                    let _ = jobs.send(Job::Queue { session });
                    ui.set_composer_text(text.as_str().into());
                    ui.set_composer_has_text(!text.is_empty());
                    return;
                }
                let _ = jobs.send(Job::QueueAct {
                    session,
                    lane: lane.to_string(),
                    slot: slot.max(0) as usize,
                    text,
                    action: action.to_string(),
                });
            });
        }
        {
            let weak = ui.as_weak();
            let jobs = job_tx.clone();
            ui.on_queue_clear(move || {
                let Some(ui) = weak.upgrade() else { return };
                let session = ui.get_selected_id().to_string();
                if session.is_empty() {
                    return;
                }
                let _ = jobs.send(Job::QueueAct {
                    session,
                    lane: String::new(),
                    slot: 0,
                    text: String::new(),
                    action: "clear".into(),
                });
            });
        }
    }
}

pub(crate) fn painter(
    canvas_cell: &Rc<RefCell<Option<composer_canvas::Canvas>>>,
    canvas_key: &Rc<RefCell<(String, usize, i32)>>,
    extra_commands: &SlashCommands,
    trace: bool,
) -> Rc<dyn Fn(&AppWindow)> {
    let canvas_cell = canvas_cell.clone();
    let canvas_key = canvas_key.clone();
    let extra_commands = extra_commands.clone();
    Rc::new(move |ui: &AppWindow| {
        if !ui.get_canvas_mode() {
            return;
        }
        let text_now = ui.get_composer_text().to_string();
        let caret_now = usize::try_from(ui.get_composer_caret().max(0)).unwrap_or(0);
        let anchor_now = usize::try_from(ui.get_composer_anchor().max(0)).unwrap_or(0);
        let width_now = ui.get_composer_width();
        let key = (
            text_now.clone(),
            caret_now * 100_000 + anchor_now,
            (width_now * 4.0) as i32,
        );
        if *canvas_key.borrow() == key {
            return;
        }
        *canvas_key.borrow_mut() = key;
        let known = extra_commands.borrow().clone();
        let mut slot = canvas_cell.borrow_mut();
        let canvas = slot.get_or_insert_with(|| {
            composer_canvas::Canvas::new(15.0, 20.0, &crate::theme::current())
        });
        let (image, caret) = canvas.render(
            &text_now,
            caret_now,
            (anchor_now, caret_now),
            width_now,
            |name| is_command_word(&known, name),
        );
        if trace && crate::env_cfg::cfg().debug {
            eprintln!(
                "[canvas] {} chars, w={width_now:.0} h={:.0} caret=({:.1},{:.1})",
                text_now.chars().count(),
                caret.content_height,
                caret.x,
                caret.y
            );
        }
        ui.set_composer_canvas(image);
        ui.set_composer_canvas_height(caret.content_height);
        ui.set_composer_caret_x(caret.x);
        ui.set_composer_caret_y(caret.y);
        ui.set_composer_caret_height(caret.height);
    })
}

#[cfg(test)]
mod compose_message_tests {
    use super::compose_message;

    #[test]
    fn a_command_alone_is_the_whole_message() {
        assert_eq!(compose_message("/compact", ""), "/compact");
        assert_eq!(compose_message("/new", "   "), "/new");
    }

    #[test]
    fn a_command_keeps_the_words_typed_after_it() {
        assert_eq!(
            compose_message("/refine", "tighten the notes"),
            "/refine tighten the notes"
        );
        assert_eq!(
            compose_message("/name", "  plan the week  "),
            "/name plan the week"
        );
    }

    #[test]
    fn prose_goes_out_as_typed() {
        assert_eq!(compose_message("", "  hello there  "), "hello there");
        assert_eq!(compose_message("", ""), "");
    }
}

#[cfg(test)]
mod image_send_tests {
    use super::image_send_blocker;

    #[test]
    fn text_only_is_never_blocked() {
        assert_eq!(image_send_blocker(false, false, true), None);
    }

    #[test]
    fn images_with_a_blind_model_are_refused() {
        assert!(image_send_blocker(true, false, false).is_some());
        assert!(image_send_blocker(true, false, true).is_some());
    }

    #[test]
    fn images_mid_turn_queue_rather_than_refuse() {
        assert_eq!(image_send_blocker(true, true, true), None);
    }

    #[test]
    fn images_to_a_vision_model_between_turns_go_out() {
        assert_eq!(image_send_blocker(true, true, false), None);
    }
}