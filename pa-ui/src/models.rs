//! The model menu: providers, their models, which provider groups are open,
//! the effort ladder, and model capability queries.

use crate::*;
use std::collections::{BTreeMap, HashSet};

pub(crate) struct ModelMenu {
    pub catalog: Rc<RefCell<Vec<pa_client::ModelEntry>>>,
    pub collapsed: Rc<RefCell<HashSet<String>>>,
    pub loaded: Rc< std::cell::Cell<bool>>,
    pub last_session: Rc<RefCell<String>>,
    hovered: Rc<RefCell<Option<(String, String)>>>,
    hover_current: Rc<RefCell<Option<(String, String)>>>,
    hover_close: Rc<Timer>,
}

impl ModelMenu {
    pub fn new(ui: &AppWindow, job_tx: &Sender<Job>) -> Rc<Self> {
        let menu = Rc::new(Self {
            catalog: Rc::new(RefCell::new(Vec::new())),
            collapsed: Rc::new(RefCell::new(HashSet::new())),
            loaded: Rc::new(std::cell::Cell::new(false)),
            last_session: Rc::new(RefCell::new(String::new())),
            hovered: Rc::new(RefCell::new(None)),
            hover_current: Rc::new(RefCell::new(None)),
            hover_close: Rc::new(Timer::default()),
        });
        menu.bind_ui(ui, job_tx);
        menu
    }

    pub fn supports_images(&self, provider: &str, model_id: &str) -> bool {
        supports_images_in(&self.catalog.borrow(), provider, model_id)
    }

    pub fn invalidate(&self) {
        self.loaded.set(false);
    }

    pub fn set_catalog(&self, ui: &AppWindow, entries: Vec<pa_client::ModelEntry>) {
        if !self.loaded.get() {
            self.collapsed
                .borrow_mut()
                .extend(entries.iter().map(|entry| entry.provider.clone()));
        }
        self.loaded.set(true);
        *self.catalog.borrow_mut() = entries;
        refresh_model_groups(ui, &self.catalog.borrow(), &self.collapsed.borrow());
        if crate::env_cfg::cfg().menu && !ui.get_model_menu() {
            self.collapsed.borrow_mut().clear();
            refresh_model_groups(ui, &self.catalog.borrow(), &self.collapsed.borrow());
            ui.set_model_menu(true);
            eprintln!(
                "[menu] auto-open box=({},{},{}) height={}",
                ui.get_box_x(),
                ui.get_box_y(),
                ui.get_box_w(),
                ui.get_model_menu_height()
            );
        }
    }

    pub fn sync_session_state(&self, ui: &AppWindow, state: &pa_client::SessionState) {
        ui.set_current_effort(thinking_level_label(&state.thinking_level).into());
        ui.set_current_level(state.thinking_level.clone().into());
        if !state.model_id.is_empty()
            && (ui.get_model_id() != state.model_id
                || ui.get_model_provider() != state.provider)
        {
            let label = self
                .catalog
                .borrow()
                .iter()
                .find(|entry| entry.id == state.model_id && entry.provider == state.provider)
                .map(|entry| model_label(&entry.name, &entry.id))
                .unwrap_or_else(|| state.model_id.clone());
            ui.set_model_label(label.into());
            ui.set_model_id(state.model_id.clone().into());
            ui.set_model_provider(state.provider.clone().into());
        }
        refresh_model_groups(ui, &self.catalog.borrow(), &self.collapsed.borrow());
    }

    pub fn sync_session_row(&self, ui: &AppWindow, row: &SessionRow) {
        if !row.id.is_empty() {
            *self.last_session.borrow_mut() = row.id.clone();
        }
        if !row.model.is_empty() {
            let label = if row.model_name.is_empty() {
                row.model.clone()
            } else {
                model_label(&row.model_name, &row.model)
            };
            if ui.get_model_label().as_str() != label {
                ui.set_model_label(label.into());
            }
            ui.set_model_id(row.model.clone().into());
            ui.set_model_provider(row.model_provider.clone().into());
        }
    }

    fn bind_ui(&self, ui: &AppWindow, job_tx: &Sender<Job>) {
        // 1. Picked model
        {
            let weak = ui.as_weak();
            let jobs = job_tx.clone();
            let catalog = self.catalog.clone();
            let collapsed = self.collapsed.clone();
            ui.on_model_picked(move |model, provider| {
                let Some(ui) = weak.upgrade() else { return };
                let session = ui.get_selected_id().to_string();
                let draft = ui.get_draft_chat();
                if session.is_empty() && !draft {
                    return;
                }
                if provider.trim().is_empty() {
                    return;
                }
                let changed =
                    ui.get_model_id() != model || ui.get_model_provider() != provider;
                if let Some(entry) = catalog
                    .borrow()
                    .iter()
                    .find(|entry| entry.id == model.as_str() && entry.provider == provider.as_str())
                {
                    ui.set_model_label(model_label(&entry.name, &entry.id).into());
                }
                ui.set_model_id(model.clone());
                ui.set_model_provider(provider.clone());
                refresh_model_groups(&ui, &catalog.borrow(), &collapsed.borrow());
                if draft {
                    ui.set_draft_model_picked(true);
                    if changed {
                        ui.set_draft_level("".into());
                        ui.set_current_effort("".into());
                        ui.set_current_level("".into());
                    }
                    return;
                }
                let _ = jobs.send(Job::SetModel {
                    session: session.clone(),
                    provider: provider.to_string(),
                    model: model.to_string(),
                });
                let _ = jobs.send(Job::SessionState { session });
            });
        }

        // 2. Pointer moved (closes effort ladder after grace period)
        {
            let weak = ui.as_weak();
            let timer = self.hover_close.clone();
            let current = self.hover_current.clone();
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

        // 3. Effort hovered (stops close timer)
        {
            let timer = self.hover_close.clone();
            ui.on_effort_hovered(move || {
                timer.stop();
            });
        }

        // 4. Model hovered (opens effort ladder if >= 2 thinking levels)
        {
            let current = self.hover_current.clone();
            let weak = ui.as_weak();
            let catalog = self.catalog.clone();
            let hovered = self.hovered.clone();
            let timer = self.hover_close.clone();
            ui.on_model_hovered(move |provider, id, row_y| {
                timer.stop();
                let Some(ui) = weak.upgrade() else { return };
                let pair = (provider.to_string(), id.to_string());
                if current.borrow().as_ref() == Some(&pair) {
                    ui.set_effort_y(row_y);
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
                let Some(items) = effort_ladder_for(entry, &ui.get_current_level()) else {
                    ui.set_effort_open(false);
                    *current.borrow_mut() = Some(pair);
                    return;
                };
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

        // 5. Effort picked
        {
            let weak = ui.as_weak();
            let hover_current = self.hover_current.clone();
            let jobs = job_tx.clone();
            let catalog = self.catalog.clone();
            let hovered = self.hovered.clone();
            ui.on_effort_picked(move |index| {
                let Some(ui) = weak.upgrade() else { return };
                let session = ui.get_selected_id().to_string();
                let draft = ui.get_draft_chat();
                if session.is_empty() && !draft {
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
                let switch = !(ui.get_model_id() == model && ui.get_model_provider() == provider);
                ui.set_effort_open(false);
                *hover_current.borrow_mut() = None;
                if draft {
                    if let Some(entry) = catalog
                        .borrow()
                        .iter()
                        .find(|entry| entry.provider == provider && entry.id == model)
                    {
                        ui.set_model_label(model_label(&entry.name, &entry.id).into());
                    }
                    ui.set_model_id(model.into());
                    ui.set_model_provider(provider.into());
                    ui.set_draft_model_picked(true);
                    ui.set_draft_level(level.clone().into());
                    ui.set_current_effort(thinking_level_label(&level).into());
                    ui.set_current_level(level.into());
                    return;
                }
                let _ = jobs.send(Job::SetLevel {
                    session,
                    provider,
                    model,
                    level,
                    switch,
                });
            });
        }

        // 6. Menu opened
        {
            let weak = ui.as_weak();
            let jobs = job_tx.clone();
            let catalog_loaded = self.loaded.clone();
            let last_session = self.last_session.clone();
            ui.on_model_menu_opened(move || {
                let Some(ui) = weak.upgrade() else { return };
                let session = ui.get_selected_id().to_string();
                if !session.is_empty() {
                    *last_session.borrow_mut() = session.clone();
                }
                if session.is_empty() && !ui.get_draft_chat() {
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
                let route = if session.is_empty() {
                    last_session.borrow().clone()
                } else {
                    session.clone()
                };
                if !catalog_loaded.get() && !route.is_empty() {
                    let _ = jobs.send(Job::ModelCatalog { session: route });
                }
                if !session.is_empty() {
                    let _ = jobs.send(Job::SessionState { session });
                }
            });
        }

        // 7. Provider group toggled
        {
            let weak = ui.as_weak();
            let catalog = self.catalog.clone();
            let collapsed = self.collapsed.clone();
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
    }
}

pub(crate) fn supports_images_in(
    catalog: &[pa_client::ModelEntry],
    provider: &str,
    model_id: &str,
) -> bool {
    catalog
        .iter()
        .find(|entry| entry.id == model_id && entry.provider == provider)
        .is_none_or(|entry| entry.supports_images)
}

pub(crate) fn effort_ladder_for(
    entry: &pa_client::ModelEntry,
    current_level: &str,
) -> Option<Vec<EffortItem>> {
    if entry.thinking_levels.len() < 2 {
        return None;
    }
    let landed = clamp_level(&entry.thinking_levels, current_level);
    Some(
        entry
            .thinking_levels
            .iter()
            .map(|candidate| EffortItem {
                level: candidate.clone().into(),
                label: transcript::plain(thinking_level_label(candidate)).into(),
                current: *candidate == landed,
            })
            .collect(),
    )
}

pub(crate) fn build_model_groups(
    catalog: &[pa_client::ModelEntry],
    collapsed: &HashSet<String>,
    current_provider: &str,
    current_id: &str,
    effort: &str,
) -> (Vec<ModelGroup>, f32) {
    let mut groups: BTreeMap<(bool, String), Vec<&pa_client::ModelEntry>> = BTreeMap::new();
    for entry in catalog {
        groups
            .entry((!entry.configured, entry.provider.clone()))
            .or_default()
            .push(entry);
    }
    let mut out: Vec<ModelGroup> = Vec::with_capacity(groups.len());
    let mut height = 12f32;
    let mut top = 0f32;
    for ((_, provider), mut entries) in groups {
        entries.sort_by_key(|a| a.name.to_lowercase());
        let open = !collapsed.contains(&provider);
        let configured = entries.first().is_some_and(|entry| entry.configured);
        let count = entries.len() as i32;
        let group_height = 26.0 + if open {
            24.0 * entries.len() as f32
        } else {
            0.0
        };
        let models: Vec<ModelItem> = if open {
            entries
                .iter()
                .map(|entry| {
                    let current = entry.id == current_id && provider == current_provider;
                    ModelItem {
                        id: entry.id.clone().into(),
                        label: transcript::plain(&model_label(&entry.name, &entry.id)).into(),
                        detail: context_hint(entry.context_window).into(),
                        effort: if current { effort.to_string() } else { String::new() }.into(),
                        current,
                    }
                })
                .collect()
        } else {
            Vec::new()
        };
        out.push(ModelGroup {
            provider: provider.clone().into(),
            label: transcript::plain(&provider_label(&provider)).into(),
            count,
            open,
            configured,
            top,
            models: ModelRc::from(Rc::new(VecModel::from(models))),
        });
        top += group_height;
        height += group_height;
    }
    (out, height.min(340.0))
}

/// Rebuild the menu: configured providers first, then the rest, alphabetical.
/// A collapsed group carries no models at all, so the panel never instantiates
/// the whole catalog.
pub(crate) fn refresh_model_groups(
    ui: &AppWindow,
    catalog: &[pa_client::ModelEntry],
    collapsed: &HashSet<String>,
) {
    let current_id = ui.get_model_id().to_string();
    let current_provider = ui.get_model_provider().to_string();
    let effort = ui.get_current_effort().to_string();
    let (out, height) = build_model_groups(
        catalog,
        collapsed,
        &current_provider,
        &current_id,
        &effort,
    );
    ui.set_model_menu_height(height);
    ui.set_model_groups(ModelRc::from(Rc::new(VecModel::from(out))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    fn entry(provider: &str, id: &str, configured: bool, images: bool, levels: &[&str]) -> pa_client::ModelEntry {
        pa_client::ModelEntry {
            provider: provider.to_string(),
            id: id.to_string(),
            name: format!("{id} ({provider})"),
            configured,
            supports_images: images,
            context_window: 200_000,
            thinking_levels: levels.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn configured_providers_sort_first_and_collapsed_groups_omit_models() {
        let catalog = vec![
            entry("zeta", "z-1", false, true, &[]),
            entry("alpha", "a-1", true, true, &["low", "high"]),
            entry("alpha", "a-2", true, true, &["low", "high"]),
        ];
        let mut collapsed = HashSet::new();
        collapsed.insert("zeta".to_string());
        let (groups, height) = build_model_groups(&catalog, &collapsed, "alpha", "a-1", "High");
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].provider.as_str(), "alpha");
        assert!(groups[0].open);
        assert_eq!(groups[0].models.row_count(), 2);
        assert_eq!(groups[0].top, 0.0);
        assert_eq!(groups[1].provider.as_str(), "zeta");
        assert!(!groups[1].open);
        assert_eq!(groups[1].top, 74.0);
        assert_eq!(groups[1].models.row_count(), 0);
        assert_eq!(height, 112.0);
    }

    #[test]
    fn effort_ladder_requires_at_least_two_levels_and_clamps_current() {
        let blind = entry("alpha", "a-0", true, false, &["medium"]);
        assert!(effort_ladder_for(&blind, "high").is_none());

        let multi = entry("alpha", "a-1", true, true, &["low", "medium", "high"]);
        let ladder = effort_ladder_for(&multi, "high").expect("ladder");
        assert_eq!(ladder.len(), 3);
        assert!(ladder[2].current);
    }

    #[test]
    fn supports_images_checks_catalog_and_defaults_true_when_unknown() {
        let catalog = vec![
            entry("alpha", "blind", true, false, &[]),
            entry("alpha", "vision", true, true, &[]),
        ];
        assert!(!supports_images_in(&catalog, "alpha", "blind"));
        assert!(supports_images_in(&catalog, "alpha", "vision"));
        assert!(supports_images_in(&catalog, "alpha", "unknown-model"));
    }

    #[test]
    fn invalidate_forces_the_next_catalog_load() {
        let menu = ModelMenu {
            catalog: Rc::new(RefCell::new(Vec::new())),
            collapsed: Rc::new(RefCell::new(HashSet::new())),
            loaded: Rc::new(std::cell::Cell::new(true)),
            last_session: Rc::new(RefCell::new(String::new())),
            hovered: Rc::new(RefCell::new(None)),
            hover_current: Rc::new(RefCell::new(None)),
            hover_close: Rc::new(Timer::default()),
        };

        menu.invalidate();
        assert!(!menu.loaded.get());
    }
}