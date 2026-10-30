//! The model menu: picking a model, folding a provider, hovering a row to offer an effort.

#[allow(unused_imports)]
use crate::*;

pub(crate) fn provider_toggled(ui: &AppWindow, catalog: &Rc<RefCell<Vec<pa_client::ModelEntry>>>, collapsed: &Rc<RefCell<std::collections::HashSet<String>>>) {
// folding a group is remembered for the rest of the session, so a menu
    // reopened later comes back exactly as it was left
    let weak = ui.as_weak();
    let catalog = catalog.clone();
    let collapsed = collapsed.clone();
    ui.on_provider_toggled(move |provider| {
        let Some(ui) = weak.upgrade() else { return };
        let provider = provider.to_string();
        if crate::env_cfg::cfg().debug {
            eprintln!("[menu] fold {provider}");
        }
        {
            let mut folded = collapsed.borrow_mut();
            if !folded.remove(&provider) {
                folded.insert(provider);
            }
        }
        refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
    });
}

pub(crate) fn open(ui: &AppWindow, catalog_loaded: &Rc<std::cell::Cell<bool>>, job_tx: &Sender<Job>) {
    // opening the menu pulls the catalog once; after that it is cached, and
    // only the group fold state changes
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let catalog_loaded = catalog_loaded.clone();
    ui.on_model_menu_opened(move || {
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        if crate::env_cfg::cfg().debug {
            eprintln!(
                "[menu] opened box=({},{},{}) height={}",
                ui.get_box_x(),
                ui.get_box_y(),
                ui.get_box_w(),
                ui.get_model_menu_height()
            );
        }
        if !catalog_loaded.get() {
            let _ = jobs.send(Job::ModelCatalog {
                session: session.clone(),
            });
        }
        // the level can move under us (the CLI, another window), so the
        // menu opens on a fresh reading rather than the last poll's
        let _ = jobs.send(Job::SessionState { session });
    });
}

pub(crate) fn effort_pick(ui: &AppWindow, hover_current: &Rc<RefCell<Option<(String, String)>>>, catalog: &Rc<RefCell<Vec<pa_client::ModelEntry>>>, hovered: &Rc<RefCell<Option<(String, String)>>>, job_tx: &Sender<Job>) {
    let weak = ui.as_weak();
    let hover_current = hover_current.clone();
    let jobs = job_tx.clone();
    let catalog = catalog.clone();
    let hovered = hovered.clone();
    ui.on_effort_picked(move |index| {
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        let Some((provider, model)) = hovered.borrow().clone() else { return };
        let level = catalog
            .borrow()
            .iter()
            .find(|entry| entry.provider == provider && entry.id == model)
            .and_then(|entry| {
                usize::try_from(index)
                    .ok()
                    .and_then(|index| entry.thinking_levels.get(index).cloned())
            });
        let Some(level) = level else { return };
        let switch = !(model == ui.get_model_id().to_string()
            && provider == ui.get_model_provider().to_string());
        ui.set_effort_open(false);
        *hover_current.borrow_mut() = None;
        ui.set_status_line(
            format!("{model} � {}", thinking_level_label(&level)).into(),
        );
        let _ = jobs.send(Job::SetLevel {
            session,
            provider,
            model,
            level,
            switch,
        });
    });
}

pub(crate) fn hover(
    ui: &AppWindow,
    catalog: &Rc<RefCell<Vec<pa_client::ModelEntry>>>,
    hover_close: &Rc<Timer>,
    hover_current: &Rc<RefCell<Option<(String, String)>>>,
    hovered: &Rc<RefCell<Option<(String, String)>>>,
) {
    // A row's ladder hangs off the row. The hover repeats on every pointer
    // move, so the pair is remembered and only a change rebuilds the list.
    let current = hover_current.clone();
    let weak = ui.as_weak();
    let catalog = catalog.clone();
    let hovered = hovered.clone();
    let timer = hover_close.clone();
    ui.on_model_hovered(move |provider, id, row_y| {
        timer.stop();
        let Some(ui) = weak.upgrade() else { return };
        let pair = (provider.to_string(), id.to_string());
        if current.borrow().as_ref() == Some(&pair) {
            return;
        }
        let catalog = catalog.borrow();
        let Some(entry) = catalog
            .iter()
            .find(|entry| entry.provider == pair.0 && entry.id == pair.1)
        else {
            if crate::env_cfg::cfg().debug {
                eprintln!("[menu] hover {}/{} not in catalog", pair.0, pair.1);
            }
            return;
        };
        if crate::env_cfg::cfg().debug {
            eprintln!(
                "[menu] hover {}/{} levels={:?}",
                pair.0, pair.1, entry.thinking_levels
            );
        }
        // nothing to choose on a model with no thinking surface
        if entry.thinking_levels.len() < 2 {
            ui.set_effort_open(false);
            *current.borrow_mut() = Some(pair);
            return;
        }
        // Switching keeps the level and clamps it to the new model's own
        // ladder, so the rung that would apply is the clamped one � the
        // submenu shows where this model would land, not a guess.
        let landed = clamp_level(&entry.thinking_levels, &ui.get_current_level());
        let items: Vec<EffortItem> = entry
            .thinking_levels
            .iter()
            .map(|candidate| EffortItem {
                level: candidate.clone().into(),
                label: transcript::plain(thinking_level_label(candidate)).into(),
                current: *candidate == landed,
            })
            .collect();
        ui.set_effort_items(ModelRc::from(Rc::new(VecModel::from(items))));
        ui.set_effort_y(row_y);
        if !ui.get_effort_open() && crate::env_cfg::cfg().debug {
            eprintln!("[menu] ladder open at y={row_y}");
        }
        ui.set_effort_open(true);
        *hovered.borrow_mut() = Some(pair.clone());
        *current.borrow_mut() = Some(pair);
    });
}

pub(crate) fn effort_hover(ui: &AppWindow, hover_close: &Rc<Timer>) {
    let timer = hover_close.clone();
    ui.on_effort_hovered(move || {
        timer.stop();
    });
}

pub(crate) fn pointer(ui: &AppWindow, hover_close: &Rc<Timer>, hover_current: &Rc<RefCell<Option<(String, String)>>>) {
    let weak = ui.as_weak();
    let timer = hover_close.clone();
    let current = hover_current.clone();
    ui.on_pointer_moved(move || {
        let Some(ui) = weak.upgrade() else { return };
        if !ui.get_effort_open() {
            return;
        }
        let weak = ui.as_weak();
        let current = current.clone();
        timer.start(TimerMode::SingleShot, Duration::from_millis(120), move || {
            if let Some(ui) = weak.upgrade() {
                if crate::env_cfg::cfg().debug {
                    eprintln!("[menu] ladder closed");
                }
                *current.borrow_mut() = None;
                ui.set_effort_open(false);
            }
        });
    });
}

pub(crate) fn picked(ui: &AppWindow, catalog: &Rc<RefCell<Vec<pa_client::ModelEntry>>>, collapsed: &Rc<RefCell<std::collections::HashSet<String>>>, job_tx: &Sender<Job>) {
    let weak = ui.as_weak();
    let jobs = job_tx.clone();
    let catalog = catalog.clone();
    let collapsed2 = collapsed.clone();
    let collapsed = collapsed2.clone();
    ui.on_model_picked(move |model, provider| {
        let Some(ui) = weak.upgrade() else { return };
        let session = ui.get_selected_id().to_string();
        if session.is_empty() {
            return;
        }
        if provider.trim().is_empty() {
            // no provider known for this entry: say so instead of guessing
            ui.set_status_line(format!("{model}: provider unknown, not switching").into());
            return;
        }
        // the chip follows the click; the daemon's own state lands on the
        // next list refresh
        if let Some(entry) = catalog.borrow().iter().find(|entry| {
            entry.id == model.as_str() && entry.provider == provider.as_str()
        }) {
            ui.set_model_label(model_label(&entry.name, &entry.id).into());
        }
        ui.set_model_id(model.clone());
        ui.set_model_provider(provider.clone());
        refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
        ui.set_status_line(format!("switching to {model}").into());
        let _ = jobs.send(Job::SetModel {
            session: session.clone(),
            provider: provider.to_string(),
            model: model.to_string(),
        });
        // the switch's own level lands on the next poll, not from here
        let _ = jobs.send(Job::SessionState { session });
    });
}
