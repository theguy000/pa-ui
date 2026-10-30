//! A transcript row opening and closing.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn toggle(ui: &AppWindow, row_model: &Rc<VecModel<TRow>>, rows_store: &Rc<RefCell<Vec<Row>>>) {
    let rows = rows_store.clone();
    let model = row_model.clone();
    ui.on_toggle_row(move |index| {
        let index = index as usize;
        let mut rows = rows.borrow_mut();
        let Some(row) = rows.get_mut(index) else { return };
        row.expanded = !row.expanded;
        let converted = to_slint(row);
        drop(rows);
        if index < model.row_count() {
            model.set_row_data(index, converted);
        }
    });
}
