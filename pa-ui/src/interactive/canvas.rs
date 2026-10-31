//! The composer painted by hand, on the keystroke rather than on the tick.
//!
//! One painter, used twice: on every keystroke, and again after a command is
//! taken so the gold word is replaced in the same frame. The second call used to
//! be a copy of this one; it differs only in not repeating the debug line.

use crate::*;

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
        let canvas = slot.get_or_insert_with(|| composer_canvas::Canvas::new(13.0, 18.0));
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
