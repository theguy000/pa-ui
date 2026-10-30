//! The model menu: providers, their models, and which provider groups are open.

use crate::*;

/// Rebuild the menu: configured providers first, then the rest, alphabetical.
/// A collapsed group carries no models at all, so the panel never instantiates
/// the whole catalog.
pub(crate) fn refresh_model_groups(
    ui: &AppWindow,
    catalog: &[pa_client::ModelEntry],
    collapsed: &std::collections::HashSet<String>,
) {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(bool, String), Vec<&pa_client::ModelEntry>> = BTreeMap::new();
    for entry in catalog {
        groups
            .entry((!entry.configured, entry.provider.clone()))
            .or_default()
            .push(entry);
    }
    let current_id = ui.get_model_id().to_string();
    let current_provider = ui.get_model_provider().to_string();
    let effort = ui.get_current_effort().to_string();
    let mut out: Vec<ModelGroup> = Vec::with_capacity(groups.len());
    let mut height = 12f32;
    for ((_, provider), mut entries) in groups {
        entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        let open = !collapsed.contains(&provider);
        let configured = entries.first().is_some_and(|entry| entry.configured);
        let count = entries.len() as i32;
        height += 26.0;
        let models: Vec<ModelItem> = if open {
            height += 24.0 * entries.len() as f32;
            entries
                .iter()
                .map(|entry| {
                    let current = entry.id == current_id && provider == current_provider;
                    ModelItem {
                        id: entry.id.clone().into(),
                        label: transcript::plain(&model_label(&entry.name, &entry.id)).into(),
                        detail: context_hint(entry.context_window).into(),
                        // only the running model has a level to name
                        effort: if current { effort.clone() } else { String::new() }.into(),
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
            models: ModelRc::from(Rc::new(VecModel::from(models))),
        });
    }
    ui.set_model_menu_height(height.min(340.0));
    ui.set_model_groups(ModelRc::from(Rc::new(VecModel::from(out))));
}
