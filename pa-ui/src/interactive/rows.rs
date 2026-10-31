//! A transcript row opening and closing.

use crate::*;

pub(crate) fn toggle(ui: &AppWindow, sheet: &Rc<sheet::Sheet>) {
    let sheet = sheet.clone();
    ui.on_toggle_row(move |index| sheet.toggle(index as usize));
}
