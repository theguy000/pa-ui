//! The command menu: the token under the caret, what it offers, and what accepting one does.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn accept(ui: &AppWindow, repaint_canvas_for_accept: &Rc<dyn Fn(&AppWindow)>, slash_token: &Rc<RefCell<(usize, usize)>>) {
    let weak = ui.as_weak();
    let repaint_canvas_for_accept = repaint_canvas_for_accept.clone();
    let token = slash_token.clone();
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
        // The commands this window can carry out itself open their own
        // pickers instead of leaving a word in the box to press Enter on.
        if name == "model" {
            ui.set_composer_text("".into());
            ui.set_composer_has_text(false);
            ui.set_model_menu(true);
            ui.set_status_line("pick a model".into());
            return;
        }
        let text = ui.get_composer_text().to_string();
        let (start, end) = *token.borrow();
        let (start, end) = (start.min(text.len()), end.min(text.len()));
        // In canvas mode the command is always part of the text: the painting
        // colours it wherever it sits, so there is no outside slot to keep.
        if !ui.get_canvas_mode() && start == 0 && end == text.len() {
            // The whole box is the command: it becomes the gold word in
            // front of the caret, and the box is left empty for arguments.
            ui.set_composer_command(format!("/{name}").as_str().into());
            ui.set_composer_text("".into());
            ui.set_composer_has_text(false);
            ui.set_composer_caret_request(0);
            return;
        }
        // Started between words: replace that token where it stands and leave
        // the rest of the sentence exactly as it was.
        let mut next = String::with_capacity(text.len() + name.len() + 2);
        next.push_str(&text[..start]);
        next.push('/');
        next.push_str(&name);
        next.push(' ');
        next.push_str(&text[end..]);
        ui.set_composer_text(next.as_str().into());
        ui.set_composer_has_text(true);
        ui.set_composer_caret_request(i32::try_from(start + name.len() + 2).unwrap_or(0));
        repaint_canvas_for_accept(&ui);
    });
}

pub(crate) fn dismiss(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_slash_dismiss(move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_slash_open(false);
        }
    });
}

pub(crate) fn step(ui: &AppWindow) {
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

pub(crate) fn typed(ui: &AppWindow, extra_commands: &Rc<RefCell<Vec<(String, String, String)>>>, repaint_canvas: &Rc<dyn Fn(&AppWindow)>, slash_token: &Rc<RefCell<(usize, usize)>>) {
    let weak = ui.as_weak();
    let extra_commands = extra_commands.clone();
    let token = slash_token.clone();
    let paint = repaint_canvas.clone();
    ui.on_slash_typed(move |text, caret| {
        let Some(ui) = weak.upgrade() else { return };
        // the words paint in the same frame as the keystroke
        paint(&ui);
        let (start, end) = slash_token_at(&text, usize::try_from(caret.max(0)).unwrap_or(0));
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

pub(crate) fn cleared(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_composer_command_cleared(move || {
        let Some(ui) = weak.upgrade() else { return };
        if crate::env_cfg::cfg().debug {
            eprintln!("[composer] command word removed");
        }
        ui.set_composer_command("".into());
    });
}
