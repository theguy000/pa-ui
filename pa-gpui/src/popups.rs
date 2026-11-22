//! Floating popups module: queue panel, model & effort ladder menu,
//! context card popover, settings menu, session actions menu, and model label helpers.

use crate::app::{PaApp, ProvidersEditMode, SettingsTab};
use crate::daemon::{DaemonCmd, ExportKind};
use crate::roster::ModelPickEffect;
use crate::sidebar::RenameTarget;
use gpui_kit::component::{
    h_flex, scroll::ScrollableElement as _, v_flex, Icon, IconName, StyledExt as _, Theme,
    ThemeMode,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{clamp_level, thinking_level_label, ModelEntry};
use std::collections::BTreeMap;

#[derive(Clone, PartialEq, Eq)]
pub(crate) enum OpenPopup {
    None,
    ModelMenu,
    SettingsMenu,
    ExportResult {
        kind: ExportKind,
        primary: String,
        secondary: Option<String>,
        copied: Option<&'static str>,
    },
    ContextCard,
    QueuePanel,
    SessionMenu {
        key: String,
        id: String,
        file: String,
        name: String,
        is_resident: bool,
        anchor_y: i32,
    },
}

pub(crate) fn provider_label(provider: &str) -> String {
    provider.replace('_', " ")
}

pub(crate) fn model_label(name: &str, id: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return id.to_string();
    }
    match trimmed.rfind(" (") {
        Some(at) if trimmed.ends_with(')') => trimmed[..at].to_string(),
        _ => trimmed.to_string(),
    }
}

pub(crate) fn context_hint(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        String::new()
    }
}

pub(crate) fn effort_ladder_selected_level(
    hovered_is_current: bool,
    levels: &[String],
    cur_thinking: &str,
    saved_level: Option<&str>,
) -> Option<String> {
    if levels.is_empty() {
        return None;
    }
    if hovered_is_current {
        Some(clamp_level(levels, cur_thinking))
    } else {
        saved_level.map(|level| clamp_level(levels, level))
    }
}

pub(crate) fn effort_ladder_from_bottom(
    menu_height: f32,
    content_row_top: f32,
    scroll_y: f32,
    ladder_h: f32,
) -> f32 {
    // 7px top inset (1px border + 6px padding) + 12px half-row height (24px row).
    let visible_row_center = 7.0 + content_row_top + scroll_y + 12.0;
    let row_center_from_bottom = menu_height - visible_row_center;
    (row_center_from_bottom - ladder_h * 0.5).clamp(0.0, (menu_height - ladder_h).max(0.0))
}

impl PaApp {
    pub(crate) fn render_floating_popups(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let p = self.palette();
        let (_, composer_right_inset) = self.composer_column_insets(window);
        // Anchor model/context popups directly above the composer's bottom toolbar chips
        // (12px band pb + 8px dock pb + 26px chip height + 8px breathing room = 54px).
        let dock_popup_bottom = 54.0_f32;
        // Offset left of the 26px send button + 8px gap + 12px inner card padding = 46px.
        let menu_right = composer_right_inset + 46.0;

        match &self.open_popup {
            OpenPopup::None => self.render_slash_overlay(window, cx),
            OpenPopup::QueuePanel => {
                let queued_total = self.composer_model.queued_total();
                if queued_total == 0 {
                    return None;
                }
                let (strip_h, _, _, _) = self.composer_model.busy_strip_metrics(self.is_streaming);
                let attach_h = if self.composer_model.attachments().is_empty() {
                    0.0
                } else {
                    52.0
                };
                let panel_bottom = 12.0
                    + 12.0
                    + self.effective_composer_height(cx)
                    + attach_h
                    + 42.0
                    + strip_h
                    + 6.0;
                let panel_right = px(composer_right_inset);

                let mut col = v_flex().gap(px(2.));
                let header_label = if queued_total == 1 {
                    "1 message waiting".to_string()
                } else {
                    format!("{queued_total} messages waiting")
                };

                let queued_steer = self.composer_model.queued_steer();
                let queued_follow_up = self.composer_model.queued_follow_up();

                if !queued_steer.is_empty() {
                    col = col.child(
                        div()
                            .px(px(8.))
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .text_size(px(10.))
                            .text_color(p.sidebar_group_empty_ink)
                            .child(header_label.clone()),
                    );
                    for (idx, txt) in queued_steer.iter().enumerate() {
                        let exp_edit = txt.clone();
                        let exp_del = txt.clone();
                        col = col.child(
                            h_flex()
                                .id(("q-steer", idx))
                                .min_h(px(32.))
                                .px(px(8.))
                                .py(px(5.))
                                .rounded(px(6.))
                                .hover(|d| d.bg(p.sidebar_row_hover))
                                .items_center()
                                .gap(px(6.))
                                .child(
                                    div()
                                        .id(("q-steer-edit", idx))
                                        .flex_1()
                                        .min_w_0()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .line_height(px(15.))
                                        .text_color(p.ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(txt.clone())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.composer_model.begin_edit_queue(
                                                "steering".into(),
                                                idx,
                                                exp_edit.clone(),
                                            );
                                            this.set_composer_text(exp_edit.clone(), window, cx);
                                            this.notice = Some(
                                                "Editing queued message Â· Esc to cancel".into(),
                                            );
                                            this.open_popup = OpenPopup::None;
                                            let focus = this.composer.read(cx).focus_handle(cx);
                                            window.focus(&focus, cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id(("q-steer-del", idx))
                                        .size(px(22.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.danger_surface))
                                        .child(
                                            Icon::new(IconName::Close)
                                                .size(px(11.))
                                                .text_color(p.quiet_ink),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) =
                                                this.roster.active_id().map(str::to_string)
                                            {
                                                let _ =
                                                    this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                        session: sid,
                                                        lane: "steering".into(),
                                                        slot: idx,
                                                        expected: exp_del.clone(),
                                                        action: "delete".into(),
                                                    });
                                            }
                                        })),
                                ),
                        );
                    }
                }

                if !queued_follow_up.is_empty() {
                    let fu_header = if queued_steer.is_empty() {
                        format!("{header_label} Â· runs next")
                    } else {
                        "runs next".to_string()
                    };
                    col = col.child(
                        div()
                            .px(px(8.))
                            .pt(if queued_steer.is_empty() {
                                px(0.)
                            } else {
                                px(4.)
                            })
                            .h(px(20.))
                            .flex()
                            .items_center()
                            .text_size(px(10.))
                            .text_color(p.sidebar_group_empty_ink)
                            .child(fu_header),
                    );
                    for (idx, txt) in queued_follow_up.iter().enumerate() {
                        let exp_edit = txt.clone();
                        let exp_p = txt.clone();
                        let exp_d = txt.clone();
                        col = col.child(
                            h_flex()
                                .id(("q-fu", idx))
                                .min_h(px(32.))
                                .px(px(8.))
                                .py(px(5.))
                                .rounded(px(6.))
                                .hover(|d| d.bg(p.sidebar_row_hover))
                                .items_center()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .id(("q-fu-edit", idx))
                                        .flex_1()
                                        .min_w_0()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .line_height(px(15.))
                                        .text_color(p.ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(txt.clone())
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.composer_model.begin_edit_queue(
                                                "followUp".into(),
                                                idx,
                                                exp_edit.clone(),
                                            );
                                            this.set_composer_text(exp_edit.clone(), window, cx);
                                            this.notice = Some(
                                                "Editing queued message Â· Esc to cancel".into(),
                                            );
                                            this.open_popup = OpenPopup::None;
                                            let focus = this.composer.read(cx).focus_handle(cx);
                                            window.focus(&focus, cx);
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    div()
                                        .id(("q-fu-prom", idx))
                                        .size(px(22.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.sidebar_new_fill_hover))
                                        .child(
                                            Icon::new(IconName::ArrowUp)
                                                .size(px(11.))
                                                .text_color(p.working),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) =
                                                this.roster.active_id().map(str::to_string)
                                            {
                                                let _ =
                                                    this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                        session: sid,
                                                        lane: "followUp".into(),
                                                        slot: idx,
                                                        expected: exp_p.clone(),
                                                        action: "promote".into(),
                                                    });
                                            }
                                        })),
                                )
                                .child(
                                    div()
                                        .id(("q-fu-del", idx))
                                        .size(px(22.))
                                        .rounded(px(5.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.danger_surface))
                                        .child(
                                            Icon::new(IconName::Close)
                                                .size(px(11.))
                                                .text_color(p.quiet_ink),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) =
                                                this.roster.active_id().map(str::to_string)
                                            {
                                                let _ =
                                                    this.bridge.tx.send(DaemonCmd::MutateQueue {
                                                        session: sid,
                                                        lane: "followUp".into(),
                                                        slot: idx,
                                                        expected: exp_d.clone(),
                                                        action: "delete".into(),
                                                    });
                                            }
                                        })),
                                ),
                        );
                    }
                }

                Some(
                    v_flex()
                        .id("floating-queue-panel")
                        .absolute()
                        .bottom(px(panel_bottom))
                        .right(panel_right)
                        .w(px(320.))
                        .p(px(6.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .gap(px(4.))
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .id("queue-scroll")
                                .max_h(px(240.))
                                .overflow_y_scrollbar()
                                .child(col),
                        )
                        .child(div().h(px(1.)).mx(px(6.)).bg(p.panel_edge))
                        .child(
                            h_flex()
                                .id("queue-clear-all")
                                .h(px(26.))
                                .px(px(8.))
                                .rounded(px(6.))
                                .items_center()
                                .justify_between()
                                .cursor_pointer()
                                .text_size(px(11.))
                                .text_color(p.quiet_ink)
                                .hover(|d| d.bg(p.danger_surface).text_color(p.danger))
                                .child("Clear all")
                                .child(
                                    Icon::new(IconName::Delete)
                                        .size(px(11.5))
                                        .text_color(p.sidebar_group_empty_ink),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.composer_model.cancel_edit_queue();
                                    if let Some(sid) =
                                        this.roster.active_id().map(str::to_string)
                                    {
                                        let _ = this
                                            .bridge
                                            .tx
                                            .send(DaemonCmd::ClearQueue { session: sid });
                                    }
                                    this.open_popup = OpenPopup::None;
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                )
            }
            OpenPopup::ModelMenu => {
                let cur = self.roster.current_model();
                let cur_provider = cur.provider;
                let cur_model_id = cur.model_id;
                let cur_thinking = if cur.thinking.is_empty() {
                    "medium".to_string()
                } else {
                    cur.thinking
                };

                let mut groups: BTreeMap<(bool, String), Vec<&ModelEntry>> = BTreeMap::new();
                for entry in self.roster.models() {
                    groups
                        .entry((!entry.configured, entry.provider.clone()))
                        .or_default()
                        .push(entry);
                }

                let mut model_list = v_flex().gap(px(2.));
                let mut running_top: f32 = 0.0;
                if groups.is_empty() {
                    model_list = model_list.child(
                        div()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(p.quiet_ink)
                            .child("No models loaded"),
                    );
                } else {
                    for (g_idx, ((unconfigured, provider), mut entries)) in
                        groups.into_iter().enumerate()
                    {
                        entries.sort_by_key(|a| a.name.to_lowercase());
                        let open = !self.collapsed_providers.contains(&provider);
                        let configured = !unconfigured;
                        let count_label = if configured {
                            entries.len().to_string()
                        } else {
                            "not signed in".to_string()
                        };
                        let prov_toggle = provider.clone();

                        model_list = model_list.child(
                            h_flex()
                                .id(("m-grp", g_idx))
                                .h(px(26.))
                                .px(px(6.))
                                .rounded(px(5.))
                                .items_center()
                                .justify_between()
                                .cursor_pointer()
                                .hover(|d| d.bg(p.sidebar_row_hover))
                                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                    if *hovered && this.hovered_model.is_some() {
                                        this.hovered_model = None;
                                        cx.notify();
                                    }
                                }))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if !this.collapsed_providers.remove(&prov_toggle) {
                                        this.collapsed_providers.insert(prov_toggle.clone());
                                    }
                                    this.hovered_model = None;
                                    cx.notify();
                                }))
                                .child(
                                    h_flex()
                                        .gap(px(6.))
                                        .items_center()
                                        .child(
                                            Icon::new(if open {
                                                IconName::ChevronDown
                                            } else {
                                                IconName::ChevronRight
                                            })
                                            .size(px(11.))
                                            .text_color(p.quiet_ink),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .font_medium()
                                                .text_color(if configured {
                                                    p.ink
                                                } else {
                                                    p.sidebar_group_empty_ink
                                                })
                                                .child(provider_label(&provider)),
                                        ),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(p.sidebar_group_empty_ink)
                                        .child(count_label),
                                ),
                        );
                        running_top += 28.0;

                        if open {
                            for (m_idx, m) in entries.into_iter().enumerate() {
                                let selected = m.id == cur_model_id
                                    && (cur_provider.is_empty() || m.provider == cur_provider);
                                let is_hovered_row = self
                                    .hovered_model
                                    .as_ref()
                                    .is_some_and(|(hp, hi, _)| hp == &m.provider && hi == &m.id);
                                let has_ladder = configured && m.thinking_levels.len() >= 2;
                                let prov = m.provider.clone();
                                let mid = m.id.clone();
                                let levels_c = m.thinking_levels.clone();
                                let prov_h = m.provider.clone();
                                let mid_h = m.id.clone();
                                let row_top = running_top;
                                let label = model_label(&m.name, &m.id);
                                let right_detail = if selected && !cur_thinking.is_empty() {
                                    thinking_level_label(&cur_thinking).to_string()
                                } else {
                                    context_hint(m.context_window)
                                };
                                let right_color = if selected && !cur_thinking.is_empty() {
                                    p.working
                                } else {
                                    p.sidebar_group_empty_ink
                                };

                                model_list = model_list.child(
                                    h_flex()
                                        .id((SharedString::from(format!("m-row-{g_idx}")), m_idx))
                                        .h(px(24.))
                                        .pl(px(22.))
                                        .pr(px(8.))
                                        .rounded(px(5.))
                                        .justify_between()
                                        .items_center()
                                        .cursor_pointer()
                                        .when(selected && is_hovered_row, |d| {
                                            d.bg(p.working.opacity(0.20))
                                        })
                                        .when(selected && !is_hovered_row, |d| {
                                            d.bg(p.working.opacity(0.14))
                                        })
                                        .when(!selected && is_hovered_row, |d| {
                                            d.bg(p.sidebar_row_hover)
                                        })
                                        .hover(move |d| {
                                            d.bg(if selected {
                                                p.working.opacity(0.20)
                                            } else {
                                                p.sidebar_row_hover
                                            })
                                        })
                                        .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                            if *hovered {
                                                let next =
                                                    (prov_h.clone(), mid_h.clone(), row_top);
                                                if this.hovered_model.as_ref() != Some(&next) {
                                                    this.hovered_model = Some(next);
                                                    cx.notify();
                                                }
                                            }
                                        }))
                                        .child(
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .text_size(px(11.5))
                                                .when(selected, |d| d.font_medium())
                                                .text_color(if is_hovered_row || selected {
                                                    p.ink
                                                } else if configured {
                                                    p.quiet_ink
                                                } else {
                                                    p.sidebar_group_empty_ink
                                                })
                                                .overflow_hidden()
                                                .whitespace_nowrap()
                                                .text_ellipsis()
                                                .child(label),
                                        )
                                        .child(
                                            h_flex()
                                                .gap(px(5.))
                                                .items_center()
                                                .child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .text_color(right_color)
                                                        .child(right_detail),
                                                )
                                                .when(selected, |r| {
                                                    r.child(
                                                        Icon::new(IconName::Check)
                                                            .size(px(11.))
                                                            .text_color(p.working),
                                                    )
                                                })
                                                .when(has_ladder && !selected, |r| {
                                                    r.child(
                                                        Icon::new(IconName::ChevronLeft)
                                                            .size(px(10.))
                                                            .text_color(if is_hovered_row {
                                                                p.working
                                                            } else {
                                                                p.sidebar_group_empty_ink
                                                            }),
                                                    )
                                                }),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if configured {
                                                match this.roster.pick_model(
                                                    &prov,
                                                    &mid,
                                                    &levels_c,
                                                    &this.settings,
                                                ) {
                                                    ModelPickEffect::SetModelAndThinking {
                                                        session,
                                                        provider,
                                                        model_id,
                                                        level,
                                                    } => {
                                                        let _ = this.bridge.tx.send(
                                                            DaemonCmd::SetModelAndThinking {
                                                                session,
                                                                provider,
                                                                model_id,
                                                                level,
                                                                switch: true,
                                                            },
                                                        );
                                                    }
                                                    ModelPickEffect::SetModel {
                                                        session,
                                                        provider,
                                                        model_id,
                                                    } => {
                                                        let _ = this.bridge.tx.send(
                                                            DaemonCmd::SetModel {
                                                                session,
                                                                provider,
                                                                model_id,
                                                            },
                                                        );
                                                    }
                                                    ModelPickEffect::DraftOnly
                                                    | ModelPickEffect::None => {}
                                                }
                                                this.open_popup = OpenPopup::None;
                                                this.hovered_model = None;
                                                cx.notify();
                                            }
                                        })),
                                );
                                running_top += 26.0;
                            }
                        }
                    }
                }

                let menu_height = (running_top.min(328.0) + 14.0).max(64.0);
                let scroll_y = f32::from(self.model_menu_scroll.offset().y);
                let ladder_elem =
                    self.hovered_model
                        .as_ref()
                        .and_then(|(h_prov, h_id, row_top)| {
                            let entry = self
                                .roster
                                .models()
                                .iter()
                                .find(|m| &m.provider == h_prov && &m.id == h_id)?;
                            if !entry.configured || entry.thinking_levels.len() < 2 {
                                return None;
                            }
                            let is_hovered_current = *h_id == cur_model_id
                                && (cur_provider.is_empty() || *h_prov == cur_provider);
                            let saved_level = self.settings.model_thinking_for(h_prov, h_id);
                            let checked_level = effort_ladder_selected_level(
                                is_hovered_current,
                                &entry.thinking_levels,
                                &cur_thinking,
                                saved_level,
                            );
                            let ladder_h = 30.0 + (entry.thinking_levels.len() as f32) * 26.0;
                            let from_bottom = effort_ladder_from_bottom(
                                menu_height,
                                *row_top,
                                scroll_y,
                                ladder_h,
                            );
                            let ladder_title = model_label(&entry.name, &entry.id);

                            let mut ladder_col = v_flex()
                                .id("floating-effort-ladder")
                                .absolute()
                                .bottom(px(dock_popup_bottom + from_bottom))
                                .right(px(menu_right + 302.))
                                .w(px(154.))
                                .p(px(5.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(p.panel_edge)
                                .bg(p.panel_surface)
                                .shadow_lg()
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .child(
                                    div()
                                        .px(px(8.))
                                        .py(px(3.))
                                        .text_size(px(10.))
                                        .font_medium()
                                        .text_color(p.sidebar_group_empty_ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(ladder_title),
                                );

                            for (idx, candidate) in entry.thinking_levels.iter().enumerate() {
                                let is_checked =
                                    checked_level.as_deref() == Some(candidate.as_str());
                                let level_c = candidate.clone();
                                let prov_c = h_prov.clone();
                                let mid_c = h_id.clone();
                                let switch = !(cur_model_id == mid_c
                                    && (cur_provider.is_empty() || cur_provider == prov_c));
                                ladder_col = ladder_col.child(
                                    h_flex()
                                        .id(("eff-rung", idx))
                                        .h(px(26.))
                                        .px(px(10.))
                                        .rounded(px(5.))
                                        .justify_between()
                                        .items_center()
                                        .cursor_pointer()
                                        .when(is_checked && is_hovered_current, |d| {
                                            d.bg(p.working.opacity(0.14))
                                        })
                                        .when(is_checked && !is_hovered_current, |d| {
                                            d.bg(p.sidebar_row_selected)
                                        })
                                        .hover(move |d| {
                                            d.bg(if is_checked && is_hovered_current {
                                                p.working.opacity(0.20)
                                            } else {
                                                p.sidebar_row_hover
                                            })
                                        })
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(if is_checked {
                                                    p.ink
                                                } else {
                                                    p.quiet_ink
                                                })
                                                .child(thinking_level_label(candidate).to_string()),
                                        )
                                        .when(is_checked, |r| {
                                            r.child(
                                                Icon::new(IconName::Check)
                                                    .size(px(11.))
                                                    .text_color(if is_hovered_current {
                                                        p.working
                                                    } else {
                                                        p.quiet_ink
                                                    }),
                                            )
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(sid) = this.roster.pick_effort(
                                                &prov_c,
                                                &mid_c,
                                                &level_c,
                                                &mut this.settings,
                                            ) {
                                                let _ = this.bridge.tx.send(
                                                    DaemonCmd::SetModelAndThinking {
                                                        session: sid,
                                                        provider: prov_c.clone(),
                                                        model_id: mid_c.clone(),
                                                        level: level_c.clone(),
                                                        switch,
                                                    },
                                                );
                                            }
                                            this.open_popup = OpenPopup::None;
                                            this.hovered_model = None;
                                            cx.notify();
                                        })),
                                );
                            }
                            Some(ladder_col)
                        });

                Some(
                    div()
                        .child(
                            v_flex()
                                .id("floating-model-menu")
                                .absolute()
                                .bottom(px(dock_popup_bottom))
                                .right(px(menu_right))
                                .w(px(300.))
                                .p(px(6.))
                                .rounded(px(10.))
                                .border_1()
                                .border_color(p.panel_edge)
                                .bg(p.panel_surface)
                                .shadow_lg()
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .child(
                                    div()
                                        .relative()
                                        .child(
                                            div()
                                                .id("model-popup-scroll")
                                                .max_h(px(328.))
                                                .overflow_y_scroll()
                                                .track_scroll(&self.model_menu_scroll)
                                                .child(model_list),
                                        )
                                        .vertical_scrollbar(&self.model_menu_scroll),
                                ),
                        )
                        .children(ladder_elem)
                        .into_any_element(),
                )
            }
            OpenPopup::ContextCard => Some(
                self.dock_popup_surface("floating-ctx-card", dock_popup_bottom, menu_right, 236.)
                    .p_3()
                    .child(self.render_context_card_content())
                    .into_any_element(),
            ),
            OpenPopup::SettingsMenu => Some(self.render_settings_popup(window, cx)),
            OpenPopup::ExportResult {
                kind,
                primary,
                secondary,
                copied,
            } => Some(self.render_export_result_popup(
                *kind,
                primary.clone(),
                secondary.clone(),
                *copied,
                cx,
            )),
            OpenPopup::SessionMenu {
                key,
                id,
                file,
                name,
                is_resident,
                anchor_y,
            } => {
                let key_ren = key.clone();
                let id_ren = id.clone();
                let id_c = id.clone();
                let id_html = id.clone();
                let id_gist = id.clone();
                let file_ren = file.clone();
                let file_del = file.clone();
                let name_c = name.clone();
                let resident = *is_resident;
                let top_y = *anchor_y as f32;
                let can_rename = (resident && !id_ren.is_empty()) || !file_ren.is_empty();
                let can_export = !id.is_empty();
                Some(
                    div()
                        .id("floating-session-menu")
                        .absolute()
                        .top(px(top_y))
                        .left(px(248.))
                        .w(px(210.))
                        .p(px(6.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .when(can_rename, |col| {
                                    col.child(
                                        self.dock_popup_row("sm-rename", "Rename", "")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.start_rename(
                                                    RenameTarget {
                                                        key: key_ren.clone(),
                                                        id: id_ren.clone(),
                                                        file: file_ren.clone(),
                                                        is_resident: resident,
                                                    },
                                                    name_c.clone(),
                                                    window,
                                                    cx,
                                                );
                                            })),
                                    )
                                })
                                .when(can_export, |col| {
                                    col.child(
                                        div()
                                            .my(px(2.))
                                            .h(px(1.))
                                            .w_full()
                                            .bg(p.sidebar_group_edge),
                                    )
                                    .child(
                                        self.dock_popup_row("sm-export-html", "Export HTML", ".html")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                let _ = this.bridge.tx.send(DaemonCmd::ExportHtml {
                                                    session: id_html.clone(),
                                                });
                                                this.open_popup = OpenPopup::None;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        self.dock_popup_row(
                                            "sm-export-gist",
                                            "Share secret Gist",
                                            "gh gist",
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            let _ = this.bridge.tx.send(DaemonCmd::ExportGist {
                                                session: id_gist.clone(),
                                            });
                                            this.open_popup = OpenPopup::None;
                                            cx.notify();
                                        })),
                                    )
                                })
                                .when(resident, |col| {
                                    col.child(
                                        div()
                                            .my(px(2.))
                                            .h(px(1.))
                                            .w_full()
                                            .bg(p.sidebar_group_edge),
                                    )
                                    .child(
                                        self.dock_popup_row("sm-stop", "Stop", "")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.roster.mark_stopped(&id_c);
                                                let _ = this.bridge.tx.send(DaemonCmd::MarkDone {
                                                    session: id_c.clone(),
                                                });
                                                this.open_popup = OpenPopup::None;
                                                cx.notify();
                                            })),
                                    )
                                })
                                .when(!resident && !file_del.is_empty(), |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-delete")
                                            .h(px(26.))
                                            .px(px(8.))
                                            .rounded(px(5.))
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.danger_surface))
                                            .text_size(px(11.5))
                                            .text_color(p.danger)
                                            .child("Delete session")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let _ =
                                                    this.bridge.tx.send(DaemonCmd::DeleteSaved {
                                                        session_path: file_del.clone(),
                                                    });
                                                this.open_popup = OpenPopup::None;
                                                cx.notify();
                                            })),
                                    )
                                }),
                        )
                        .into_any_element(),
                )
            }
        }
    }

    fn dock_popup_surface(
        &self,
        id: &'static str,
        bottom: f32,
        right: f32,
        width: f32,
    ) -> Stateful<Div> {
        let p = self.palette();
        div()
            .id(id)
            .absolute()
            .bottom(px(bottom))
            .right(px(right))
            .w(px(width))
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_lg()
            .on_click(|_, _, cx| cx.stop_propagation())
    }

    fn dock_popup_row(
        &self,
        id: &'static str,
        label: &'static str,
        detail: &'static str,
    ) -> Stateful<Div> {
        let p = self.palette();
        h_flex()
            .id(id)
            .h(px(26.))
            .px(px(8.))
            .rounded(px(5.))
            .justify_between()
            .items_center()
            .cursor_pointer()
            .hover(move |d| d.bg(p.sidebar_row_hover))
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.ink)
                    .child(label),
            )
            .child(
                div()
                    .text_size(px(10.))
                    .text_color(p.sidebar_group_empty_ink)
                    .child(detail),
            )
    }

    fn modal_close_button(&self, id: &'static str, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        div()
            .id(id)
            .size(px(28.))
            .rounded(px(8.))
            .bg(p.sidebar_row_hover)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .text_color(p.quiet_ink)
            .hover(move |d| d.bg(p.sidebar_row_selected).text_color(p.ink))
            .child(
                svg()
                    .path(crate::assets::ICON_CLOSE_PATH)
                    .size(px(12.))
                    .text_color(p.ink),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_popup = OpenPopup::None;
                cx.notify();
            }))
    }

    fn render_settings_popup(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let dark = self.settings.dark_mode;
        let backdrop: Hsla = if dark {
            rgba(0x00000088).into()
        } else {
            rgba(0x00000038).into()
        };

        let theme_card = |id: &'static str,
                          label: &'static str,
                          is_dark_card: bool,
                          selected: bool| {
            let preview_bg: Hsla = if is_dark_card {
                rgb(0x09090b).into()
            } else {
                rgb(0xf4f4f5).into()
            };
            let preview_canvas: Hsla = if is_dark_card {
                rgb(0x111114).into()
            } else {
                rgb(0xffffff).into()
            };
            let preview_chip: Hsla = if is_dark_card {
                rgb(0x232329).into()
            } else {
                rgb(0xe4e4e7).into()
            };
            let preview_edge: Hsla = if is_dark_card {
                rgb(0x27272e).into()
            } else {
                rgb(0xd4d4d8).into()
            };

            v_flex()
                .id(id)
                .flex_1()
                .p(px(10.))
                .gap(px(10.))
                .rounded(px(12.))
                .border_1()
                .cursor_pointer()
                .when(selected, |d| {
                    d.border_color(p.ink).bg(p.sidebar_row_selected)
                })
                .when(!selected, |d| {
                    d.border_color(p.sidebar_group_edge)
                        .bg(p.sidebar_group_surface)
                        .hover(|h| h.bg(p.sidebar_row_hover).border_color(p.panel_edge))
                })
                .child(
                    h_flex()
                        .w_full()
                        .h(px(74.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(preview_edge)
                        .bg(preview_bg)
                        .overflow_hidden()
                        .child(
                            v_flex()
                                .w(px(36.))
                                .h_full()
                                .p(px(6.))
                                .gap(px(4.))
                                .rounded_tl(px(7.))
                                .rounded_bl(px(7.))
                                .border_r_1()
                                .border_color(preview_edge)
                                .child(div().w_full().h(px(6.)).rounded(px(2.)).bg(preview_chip))
                                .child(div().w(px(18.)).h(px(4.)).rounded(px(2.)).bg(preview_chip))
                                .child(div().w(px(22.)).h(px(4.)).rounded(px(2.)).bg(preview_chip)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .h_full()
                                .rounded_tr(px(7.))
                                .rounded_br(px(7.))
                                .bg(preview_canvas)
                                .p(px(7.))
                                .justify_between()
                                .child(
                                    v_flex()
                                        .gap(px(4.))
                                        .child(
                                            div()
                                                .w(px(64.))
                                                .h(px(6.))
                                                .rounded(px(3.))
                                                .bg(preview_chip),
                                        )
                                        .child(
                                            div()
                                                .w(px(44.))
                                                .h(px(5.))
                                                .rounded(px(2.5))
                                                .bg(preview_chip)
                                                .opacity(0.65),
                                        ),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .h(px(16.))
                                        .rounded(px(5.))
                                        .border_1()
                                        .border_color(preview_edge)
                                        .bg(preview_bg),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px(px(2.))
                        .items_center()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .font_medium()
                                .text_color(if selected { p.ink } else { p.quiet_ink })
                                .child(label),
                        )
                        .child(
                            div()
                                .size(px(16.))
                                .rounded_full()
                                .border_1()
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(selected, |d| {
                                    d.border_color(p.composer_send_fill)
                                        .bg(p.composer_send_fill)
                                        .child(
                                            Icon::new(IconName::Check)
                                                .size(px(10.))
                                                .text_color(p.composer_send_mark),
                                        )
                                })
                                .when(!selected, |d| d.border_color(p.panel_edge)),
                        ),
                )
        };

        div()
            .id("floating-settings-backdrop")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .flex()
            .items_center()
            .justify_center()
            .p_8()
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_popup = OpenPopup::None;
                cx.notify();
            }))
            .child(
                h_flex()
                    .id("floating-settings-menu")
                    .w(px(740.))
                    .max_w_full()
                    .h(px(480.))
                    .max_h_full()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.transcript_surface)
                    .shadow_2xl()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .w(px(208.))
                            .h_full()
                            .rounded_tl(px(15.))
                            .rounded_bl(px(15.))
                            .bg(p.window_surface)
                            .border_r_1()
                            .border_color(p.sidebar_group_edge)
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(52.))
                                    .px(px(18.))
                                    .items_center()
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_semibold()
                                            .text_color(p.quiet_ink)
                                            .child("Settings"),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .flex_1()
                                    .px(px(10.))
                                    .gap(px(2.))
                                    .child({
                                        let active = self.settings_tab == SettingsTab::General;
                                        h_flex()
                                            .id("settings-nav-general")
                                            .h(px(34.))
                                            .px(px(10.))
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .when(active, |d| d.bg(p.sidebar_row_selected))
                                            .when(!active, |d| d.hover(|h| h.bg(p.sidebar_row_hover)))
                                            .items_center()
                                            .gap(px(9.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings_tab = SettingsTab::General;
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .size(px(16.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        svg()
                                                            .path(crate::assets::ICON_SWATCH_BOOK_PATH)
                                                            .size(px(14.))
                                                            .text_color(if active { p.ink } else { p.quiet_ink }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .line_height(px(16.))
                                                    .font_medium()
                                                    .text_color(if active { p.ink } else { p.quiet_ink })
                                                    .child("General"),
                                            )
                                    })
                                    .child({
                                        let active = self.settings_tab == SettingsTab::Providers;
                                        h_flex()
                                            .id("settings-nav-providers")
                                            .h(px(34.))
                                            .px(px(10.))
                                            .rounded(px(8.))
                                            .cursor_pointer()
                                            .when(active, |d| d.bg(p.sidebar_row_selected))
                                            .when(!active, |d| d.hover(|h| h.bg(p.sidebar_row_hover)))
                                            .items_center()
                                            .gap(px(9.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.settings_tab = SettingsTab::Providers;
                                                this.providers_mode = ProvidersEditMode::List;
                                                this.providers_status = None;
                                                this.refresh_providers_snapshot();
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .size(px(16.))
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        svg()
                                                            .path(crate::assets::ICON_PROVIDERS_PATH)
                                                            .size(px(14.))
                                                            .text_color(if active { p.ink } else { p.quiet_ink }),
                                                    ),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(13.))
                                                    .line_height(px(16.))
                                                    .font_medium()
                                                    .text_color(if active { p.ink } else { p.quiet_ink })
                                                    .child("Providers"),
                                            )
                                    }),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .rounded_tr(px(15.))
                            .rounded_br(px(15.))
                            .bg(p.transcript_surface)
                            .overflow_hidden()
                            .child(
                                h_flex()
                                    .w_full()
                                    .h(px(52.))
                                    .pl(if self.settings_tab == SettingsTab::Providers
                                        && !matches!(self.providers_mode, ProvidersEditMode::List)
                                    {
                                        px(18.)
                                    } else {
                                        px(28.)
                                    })
                                    .pr(px(14.))
                                    .border_b_1()
                                    .border_color(p.sidebar_group_edge)
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .items_center()
                                            .when(
                                                self.settings_tab == SettingsTab::Providers
                                                    && !matches!(
                                                        self.providers_mode,
                                                        ProvidersEditMode::List
                                                    ),
                                                |d| {
                                                    d.child(
                                                        div()
                                                            .id("settings-prov-back-btn")
                                                            .size(px(28.))
                                                            .rounded(px(7.))
                                                            .flex()
                                                            .items_center()
                                                            .justify_center()
                                                            .cursor_pointer()
                                                            .hover(|h| h.bg(p.sidebar_row_hover))
                                                            .on_click(cx.listener(|this, _, _, cx| {
                                                                if let ProvidersEditMode::EditCustom {
                                                                    model_editor_open,
                                                                    editing_model_idx,
                                                                    ..
                                                                } = &mut this.providers_mode
                                                                {
                                                                    if *model_editor_open {
                                                                        *model_editor_open = false;
                                                                        *editing_model_idx = None;
                                                                        this.providers_status = None;
                                                                        cx.notify();
                                                                        return;
                                                                    }
                                                                }
                                                                this.providers_mode =
                                                                    ProvidersEditMode::List;
                                                                this.providers_status = None;
                                                                cx.notify();
                                                            }))
                                                            .child(
                                                                Icon::new(IconName::ChevronLeft)
                                                                    .size(px(15.))
                                                                    .text_color(p.ink),
                                                            ),
                                                    )
                                                },
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(15.))
                                                    .font_semibold()
                                                    .text_color(p.ink)
                                                    .child(match self.settings_tab {
                                                        SettingsTab::General => {
                                                            "General".to_string()
                                                        }
                                                        SettingsTab::Providers => {
                                                            match &self.providers_mode {
                                                                ProvidersEditMode::List => {
                                                                    "Providers".to_string()
                                                                }
                                                                ProvidersEditMode::EditBuiltin {
                                                                    display_name,
                                                                    ..
                                                                } => display_name.clone(),
                                                                ProvidersEditMode::EditCustom {
                                                                    old_id,
                                                                    model_editor_open,
                                                                    editing_model_idx,
                                                                    ..
                                                                } => {
                                                                    if *model_editor_open {
                                                                        if editing_model_idx
                                                                            .is_some()
                                                                        {
                                                                            "Edit Model".to_string()
                                                                        } else {
                                                                            "Add Model".to_string()
                                                                        }
                                                                    } else if old_id.is_some() {
                                                                        "Edit Custom Endpoint"
                                                                            .to_string()
                                                                    } else {
                                                                        "New Custom Endpoint"
                                                                            .to_string()
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }),
                                            ),
                                    )
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .items_center()
                                            .when(
                                                self.settings_tab == SettingsTab::Providers
                                                    && matches!(
                                                        self.providers_mode,
                                                        ProvidersEditMode::List
                                                    ),
                                                |d| {
                                                    d.child(
                                                        div()
                                                            .id("providers-add-custom-btn")
                                                            .px(px(11.))
                                                            .py(px(5.))
                                                            .rounded(px(7.))
                                                            .bg(p.composer_send_fill)
                                                            .cursor_pointer()
                                                            .hover(|h| h.opacity(0.9))
                                                            .text_xs()
                                                            .font_medium()
                                                            .text_color(p.composer_send_mark)
                                                            .child("+ Custom Endpoint")
                                                            .on_click(cx.listener(
                                                                |this, _, window, cx| {
                                                                    this.open_custom_endpoint_editor(
                                                                        None, window, cx,
                                                                    );
                                                                },
                                                            )),
                                                    )
                                                },
                                            )
                                            .child(self.modal_close_button("settings-close-btn", cx)),
                                    ),
                            )
                            .child(match self.settings_tab {
                                SettingsTab::General => v_flex()
                                    .id("settings-general-body")
                                    .flex_1()
                                    .overflow_y_scrollbar()
                                    .px(px(28.))
                                    .py(px(22.))
                                    .gap(px(20.))
                                    .child(
                                        v_flex()
                                            .gap(px(12.))
                                            .child(
                                                v_flex()
                                                    .gap(px(3.))
                                                    .child(
                                                        div()
                                                            .text_size(px(13.))
                                                            .font_semibold()
                                                            .text_color(p.ink)
                                                            .child("Appearance"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(p.quiet_ink)
                                                            .child(
                                                                "Choose how Prime Agent looks on this device",
                                                            ),
                                                    ),
                                            )
                                            .child(
                                                h_flex()
                                                    .w_full()
                                                    .max_w(px(360.))
                                                    .gap(px(14.))
                                                    .child(
                                                        theme_card("set-theme-dark", "Dark", true, dark)
                                                            .on_click(cx.listener(|this, _, window, cx| {
                                                                if !this.settings.dark_mode {
                                                                    this.settings.dark_mode = true;
                                                                    this.settings.save();
                                                                    Theme::change(
                                                                        ThemeMode::Dark,
                                                                        Some(window),
                                                                        cx,
                                                                    );
                                                                    this.sync_slash_decorations(cx);
                                                                    cx.notify();
                                                                }
                                                            })),
                                                    )
                                                    .child(
                                                        theme_card("set-theme-light", "Light", false, !dark)
                                                            .on_click(cx.listener(|this, _, window, cx| {
                                                                if this.settings.dark_mode {
                                                                    this.settings.dark_mode = false;
                                                                    this.settings.save();
                                                                    Theme::change(
                                                                        ThemeMode::Light,
                                                                        Some(window),
                                                                        cx,
                                                                    );
                                                                    this.sync_slash_decorations(cx);
                                                                    cx.notify();
                                                                }
                                                            })),
                                                    ),
                                            ),
                                    )
                                    .into_any_element(),
                                SettingsTab::Providers => {
                                    self.render_providers_settings_body(window, cx)
                                }
                            }),
                    ),
            )
            .into_any_element()
    }


    fn render_export_result_popup(
        &self,
        kind: ExportKind,
        primary: String,
        secondary: Option<String>,
        copied: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let backdrop: Hsla = if self.settings.dark_mode {
            rgba(0x00000088).into()
        } else {
            rgba(0x00000038).into()
        };

        let title = match kind {
            ExportKind::Html => "Exported HTML",
            ExportKind::Gist => "Secret Gist Created",
        };
        let primary_label = match kind {
            ExportKind::Html => "File path",
            ExportKind::Gist => "Share URL",
        };

        let render_value_row = |label: &'static str,
                                value: String,
                                tag: &'static str,
                                is_url: bool,
                                cx: &mut Context<Self>| {
            let is_copied = copied == Some(tag);
            let copy_val = value.clone();
            let open_val = value.clone();

            v_flex()
                .w_full()
                .gap(px(6.))
                .child(
                    div()
                        .text_xs()
                        .font_medium()
                        .text_color(p.quiet_ink)
                        .child(label),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(8.))
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h(px(34.))
                                .px(px(11.))
                                .rounded(px(9.))
                                .border_1()
                                .border_color(p.sidebar_group_edge)
                                .bg(p.window_surface)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .w_full()
                                        .text_xs()
                                        .text_color(p.ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(value),
                                ),
                        )
                        .child(
                            h_flex()
                                .id(SharedString::from(format!("export-copy-{tag}")))
                                .h(px(34.))
                                .px(px(12.))
                                .rounded(px(9.))
                                .border_1()
                                .border_color(if is_copied {
                                    p.tag_idle
                                } else {
                                    p.sidebar_group_edge
                                })
                                .bg(p.sidebar_new_fill)
                                .hover(|d| {
                                    d.bg(p.sidebar_new_fill_hover)
                                        .border_color(p.panel_edge)
                                })
                                .items_center()
                                .gap(px(6.))
                                .cursor_pointer()
                                .text_xs()
                                .font_medium()
                                .text_color(if is_copied { p.tag_idle } else { p.ink })
                                .child(
                                    Icon::new(if is_copied {
                                        IconName::Check
                                    } else {
                                        IconName::Copy
                                    })
                                    .size(px(12.5))
                                    .text_color(if is_copied { p.tag_idle } else { p.ink }),
                                )
                                .child(if is_copied { "Copied" } else { "Copy" })
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copy_val.clone(),
                                    ));
                                    if let OpenPopup::ExportResult { copied, .. } =
                                        &mut this.open_popup
                                    {
                                        *copied = Some(tag);
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            h_flex()
                                .id(SharedString::from(format!("export-open-{tag}")))
                                .h(px(34.))
                                .px(px(12.))
                                .rounded(px(9.))
                                .border_1()
                                .border_color(p.sidebar_group_edge)
                                .bg(p.sidebar_new_fill)
                                .hover(|d| {
                                    d.bg(p.sidebar_new_fill_hover)
                                        .border_color(p.panel_edge)
                                })
                                .items_center()
                                .cursor_pointer()
                                .text_xs()
                                .font_medium()
                                .text_color(p.ink)
                                .child("Open")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    let target = if is_url {
                                        open_val.clone()
                                    } else {
                                        format!("file:///{}", open_val.replace('\\', "/"))
                                    };
                                    cx.open_url(&target);
                                })),
                        ),
                )
        };

        let primary_row = render_value_row(
            primary_label,
            primary,
            "primary",
            kind == ExportKind::Gist,
            cx,
        );
        let secondary_row = secondary.map(|url| {
            render_value_row("Gist URL", url, "secondary", true, cx)
        });

        div()
            .id("floating-export-result-backdrop")
            .absolute()
            .inset_0()
            .bg(backdrop)
            .flex()
            .items_center()
            .justify_center()
            .p_6()
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_popup = OpenPopup::None;
                cx.notify();
            }))
            .child(
                v_flex()
                    .id("floating-export-result-modal")
                    .w(px(520.))
                    .max_w_full()
                    .rounded(px(16.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.transcript_surface)
                    .shadow_2xl()
                    .overflow_hidden()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(52.))
                            .pl(px(22.))
                            .pr(px(14.))
                            .border_b_1()
                            .border_color(p.sidebar_group_edge)
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(14.5))
                                    .font_semibold()
                                    .text_color(p.ink)
                                    .child(title),
                            )
                            .child(self.modal_close_button("export-result-close-btn", cx)),
                    )
                    .child(
                        v_flex()
                            .w_full()
                            .px(px(22.))
                            .py(px(20.))
                            .gap(px(16.))
                            .child(primary_row)
                            .children(secondary_row),
                    ),
            )
            .into_any_element()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn model_and_provider_labels_strip_redundant_suffixes() {
        assert_eq!(provider_label("river_ai"), "river ai");
        assert_eq!(
            model_label("Claude 3.7 Sonnet (anthropic)", "claude-3-7-sonnet"),
            "Claude 3.7 Sonnet"
        );
        assert_eq!(model_label("", "gpt-5"), "gpt-5");
        assert_eq!(context_hint(200_000), "200k");
        assert_eq!(context_hint(1_000_000), "1M");
    }

    #[::core::prelude::v1::test]
    fn effort_ladder_uses_live_level_on_current_and_saved_preference_on_other_models() {
        let levels = vec!["off".into(), "low".into(), "high".into(), "max".into()];
        assert_eq!(
            effort_ladder_selected_level(true, &levels, "max", Some("low")),
            Some("max".into())
        );
        assert_eq!(
            effort_ladder_selected_level(false, &levels, "max", None),
            None
        );
        assert_eq!(
            effort_ladder_selected_level(false, &levels, "max", Some("low")),
            Some("low".into())
        );
        assert_eq!(
            effort_ladder_selected_level(false, &levels, "max", Some("medium")),
            Some("high".into())
        );
    }

    #[::core::prelude::v1::test]
    fn effort_ladder_tracks_hovered_row_across_scroll_offsets() {
        let menu_h = 342.0;
        let ladder_h = 134.0;
        // Middle row unscrolled: dead-centered vertically on the row.
        let mid = effort_ladder_from_bottom(menu_h, 158.0, 0.0, ladder_h);
        assert!((mid - 98.0).abs() < 0.01);
        // Same visible position after scrolling down 300px.
        let scrolled_mid = effort_ladder_from_bottom(menu_h, 458.0, -300.0, ladder_h);
        assert!((scrolled_mid - mid).abs() < 0.01);
        // Top row clamps to menu top; bottom row clamps to menu bottom.
        assert_eq!(effort_ladder_from_bottom(menu_h, 28.0, 0.0, ladder_h), menu_h - ladder_h);
        assert_eq!(effort_ladder_from_bottom(menu_h, 300.0, 0.0, ladder_h), 0.0);
    }
}
