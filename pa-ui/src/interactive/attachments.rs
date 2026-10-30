//! Attachments: picking a file, pasting one, removing one, and the thumbnails.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn drain(ui: &AppWindow, attach_rx: &Rc<Receiver<Vec<images::Attachment>>>, attachment_model: &Rc<VecModel<slint::Image>>, attachments: &Rc<RefCell<Vec<images::Attachment>>>) {
    let attach_rx = attach_rx.clone();
    let attachments = attachments.clone();
    let model = attachment_model.clone();
    let drain = Timer::default();
    drain.start(TimerMode::Repeated, std::time::Duration::from_millis(120), move || {
        while let Ok(found) = attach_rx.try_recv() {
            let mut held = attachments.borrow_mut();
            for attachment in found {
                match slint::Image::load_from_path(&attachment.path) {
                    Ok(thumb) => {
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
                        model.push(thumb);
                    }
                    // sendable without a preview; a blank tile is worse than none
                    Err(error) => eprintln!("[attach] no thumbnail: {error}"),
                }
            }
        }
    });
    std::mem::forget(drain);
}

pub(crate) fn pick(ui: &AppWindow, attach_tx: &Sender<Vec<images::Attachment>>, attachment_model: &Rc<VecModel<slint::Image>>, attachments: &Rc<RefCell<Vec<images::Attachment>>>) {
    let tx_pick = attach_tx.clone();
    ui.on_attach_requested(move || {
        let tx = tx_pick.clone();
        std::thread::spawn(move || {
            let _ = tx.send(images::pick_files());
        });
    });
    let tx_paste = attach_tx.clone();
    ui.on_attach_pasted(move || {
        if crate::env_cfg::cfg().debug {
            eprintln!("[attach] paste chord");
        }
        // the answer decides the key: true consumes it, false lets the text
        // input paste whatever text is on the clipboard
        if !images::clipboard_has_image() {
            return false;
        }
        let tx = tx_paste.clone();
        std::thread::spawn(move || {
            let _ = tx.send(images::read_clipboard().into_iter().collect());
        });
        true
    });
    let attachments = attachments.clone();
    let model = attachment_model.clone();
    ui.on_attachment_removed(move |index| {
        let index = usize::try_from(index).unwrap_or(usize::MAX);
        let mut held = attachments.borrow_mut();
        if index < held.len() {
            held.remove(index);
            model.remove(index);
        }
    });
}
