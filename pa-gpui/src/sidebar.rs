//! Sidebar & window chrome module: custom TitleBar, collapsed icon rail,
//! grouped session list (Active / Idle / Done), inline rename, and right inspector column.

use crate::app::PaApp;
use crate::assets::{PRIME_AGENT_MARK_DARK_PATH, PRIME_AGENT_MARK_LIGHT_PATH};
use crate::daemon::DaemonCmd;
use crate::popups::OpenPopup;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement as _,
    v_flex, Icon, IconName, Sizable as _, StyledExt as _, TitleBar,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::SessionRow;

#[derive(Clone)]
pub(crate) struct RenameTarget {
    pub(crate) key: String,
    pub(crate) id: String,
    pub(crate) file: String,
    pub(crate) is_resident: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SidebarGroup {
    Active,
    Idle,
    Done,
}

pub(crate) fn session_key(row: &SessionRow) -> String {
    if !row.id.is_empty() {
        row.id.clone()
    } else {
        row.session_file.clone()
    }
}

impl PaApp {
    pub(crate) fn start_rename(
        &mut self,
        target: RenameTarget,
        current_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_popup = OpenPopup::None;
        self.renaming = Some(target);
        self.rename_input.update(cx, |input, cx| {
            input.set_value(current_name, window, cx);
        });
        let focus = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub(crate) fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.renaming.take() else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if !name.is_empty() {
            self.roster.rename(&target.key, name.clone());
            if target.is_resident && !target.id.is_empty() {
                let _ = self.bridge.tx.send(DaemonCmd::RenameSession {
                    session: target.id,
                    name,
                });
            } else if !target.file.is_empty() {
                let _ = self.bridge.tx.send(DaemonCmd::RenameSaved {
                    session_path: target.file,
                    name,
                });
            }
        }
        self.rename_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    pub(crate) fn toggle_group(&mut self, group: SidebarGroup, cx: &mut Context<Self>) {
        match group {
            SidebarGroup::Active => self.settings.active_closed = !self.settings.active_closed,
            SidebarGroup::Idle => self.settings.idle_closed = !self.settings.idle_closed,
            SidebarGroup::Done => self.settings.done_closed = !self.settings.done_closed,
        }
        self.settings.save();
        cx.notify();
    }

    pub(crate) fn render_prime_mark(&self) -> impl IntoElement {
        let path = if self.settings.dark_mode {
            PRIME_AGENT_MARK_DARK_PATH
        } else {
            PRIME_AGENT_MARK_LIGHT_PATH
        };
        img(path).size(px(18.)).object_fit(ObjectFit::Contain)
    }

    pub(crate) fn render_slim_input(
        &self,
        state: &Entity<InputState>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement {
        let p = self.palette();
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        div()
            .w_full()
            .px(px(8.))
            .py(px(3.))
            .rounded(px(7.))
            .border_1()
            .border_color(if focused {
                p.composer_edge_focus
            } else {
                p.composer_edge
            })
            .bg(p.composer_surface)
            .child(
                Input::new(state)
                    .xsmall()
                    .appearance(false)
                    .bordered(false)
                    .focus_bordered(false),
            )
    }

    pub(crate) fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let settings_open = self.open_popup == OpenPopup::SettingsMenu;

        TitleBar::new()
            .h(px(32.))
            .bg(p.window_surface)
            .border_b_0()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .pr(px(8.))
                    .child(
                        div()
                            .id("tb-settings")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(settings_open, |d| d.bg(p.sidebar_row_selected))
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(
                                Icon::new(IconName::Settings)
                                    .size(px(14.))
                                    .text_color(p.quiet_ink),
                            )
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_popup = if this.open_popup == OpenPopup::SettingsMenu {
                                    OpenPopup::None
                                } else {
                                    this.refresh_providers_snapshot();
                                    OpenPopup::SettingsMenu
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .id("tb-right-col")
                            .size(px(26.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(self.settings.right_open, |d| d.bg(p.sidebar_row_selected))
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(
                                Icon::new(if self.settings.right_open {
                                    IconName::PanelRightClose
                                } else {
                                    IconName::PanelRightOpen
                                })
                                .size(px(14.))
                                .text_color(p.quiet_ink),
                            )
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_right_column(cx);
                            })),
                    ),
            )
    }

    pub(crate) fn render_collapsed_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        v_flex()
            .w(px(44.))
            .h_full()
            .bg(p.sidebar_surface)
            .pt(px(4.))
            .gap(px(6.))
            .items_center()
            .child(
                div()
                    .id("rail-open-sidebar")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.rail_mark_hovered = *hovered;
                        cx.notify();
                    }))
                    .child(if self.rail_mark_hovered {
                        Icon::new(IconName::PanelLeftOpen)
                            .size(px(16.))
                            .text_color(p.ink)
                            .into_any_element()
                    } else {
                        self.render_prime_mark().into_any_element()
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.settings.sidebar_open = true;
                        this.rail_mark_hovered = false;
                        this.settings.save();
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("rail-new-session")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(
                        Icon::new(IconName::Plus)
                            .size(px(15.))
                            .text_color(p.quiet_ink),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_new_session(window, cx);
                    })),
            )
            .child(
                div()
                    .id("rail-search")
                    .size(px(32.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(
                        Icon::new(IconName::Search)
                            .size(px(14.))
                            .text_color(p.quiet_ink),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.settings.sidebar_open = true;
                        this.search_open = true;
                        this.settings.save();
                        let focus = this.search_input.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                        cx.notify();
                    })),
            )
    }

    pub(crate) fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.settings.sidebar_open {
            return self.render_collapsed_rail(cx).into_any_element();
        }
        let p = self.palette();
        let query = self.search_input.read(cx).value();
        let grouped = self.roster.grouped_sessions(&query);
        let active_rows = grouped.active;
        let idle_rows = grouped.idle;
        let done_rows = grouped.done;

        v_flex()
            .w(px(244.))
            .h_full()
            .bg(p.sidebar_surface)
            .px(px(10.))
            .pt(px(2.))
            .pb(px(8.))
            .gap(px(6.))
            .child(
                h_flex()
                    .h(px(36.))
                    .w_full()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .pl(px(6.))
                            .gap(px(8.))
                            .items_center()
                            .child(self.render_prime_mark())
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_semibold()
                                    .text_color(p.ink)
                                    .child("Prime Agent"),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .items_center()
                            .child(
                                div()
                                    .id("sb-search-btn")
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .when(self.search_open, |d| d.bg(p.sidebar_row_selected))
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        Icon::new(IconName::Search)
                                            .size(px(14.))
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.search_open = !this.search_open;
                                        if this.search_open {
                                            let focus =
                                                this.search_input.read(cx).focus_handle(cx);
                                            window.focus(&focus, cx);
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("sb-collapse-btn")
                                    .size(px(28.))
                                    .rounded(px(7.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        Icon::new(IconName::PanelLeftClose)
                                            .size(px(14.))
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.settings.sidebar_open = false;
                                        this.rail_mark_hovered = false;
                                        this.settings.save();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .id("sb-new-session")
                    .h(px(34.))
                    .w_full()
                    .rounded(px(9.))
                    .border_1()
                    .border_color(p.sidebar_new_edge)
                    .bg(p.sidebar_new_fill)
                    .hover(|d| d.bg(p.sidebar_new_fill_hover))
                    .items_center()
                    .justify_center()
                    .gap(px(7.))
                    .cursor_pointer()
                    .child(Icon::new(IconName::Plus).size(px(14.)).text_color(p.ink))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(p.ink)
                            .child("New Session"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        this.request_new_session(window, cx);
                    })),
            )
            .when(self.search_open, |col| {
                col.child(self.render_slim_input(&self.search_input, window, cx))
            })
            .when(self.renaming.is_some(), |col| {
                col.child(
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(self.render_slim_input(&self.rename_input, window, cx)),
                        )
                        .child(
                            Button::new("save-rename")
                                .primary()
                                .xsmall()
                                .label("Save")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.commit_rename(window, cx);
                                })),
                        ),
                )
            })
            .child(
                div()
                    .id("sidebar-scroll")
                    .flex_1()
                    .overflow_y_scrollbar()
                    .child(
                        v_flex()
                            .gap(px(8.))
                            .pt(px(4.))
                            .child(self.render_session_group(
                                SidebarGroup::Active,
                                "ACTIVE",
                                &active_rows,
                                self.settings.active_closed,
                                cx,
                            ))
                            .child(self.render_session_group(
                                SidebarGroup::Idle,
                                "IDLE",
                                &idle_rows,
                                self.settings.idle_closed,
                                cx,
                            ))
                            .child(self.render_session_group(
                                SidebarGroup::Done,
                                "DONE",
                                &done_rows,
                                self.settings.done_closed,
                                cx,
                            )),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_session_group(
        &self,
        group_kind: SidebarGroup,
        title: &'static str,
        rows: &[&SessionRow],
        closed: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let empty = rows.is_empty();
        let mut card = v_flex()
            .w_full()
            .rounded(px(10.))
            .border_1()
            .border_color(if empty {
                p.sidebar_group_empty_edge
            } else {
                p.sidebar_group_edge
            })
            .bg(if empty {
                gpui_kit::transparent_black()
            } else {
                p.sidebar_group_surface
            })
            .p(px(4.))
            .gap(px(2.))
            .child(
                h_flex()
                    .id(SharedString::from(format!("grp-hdr-{title}")))
                    .h(px(26.))
                    .w_full()
                    .px(px(6.))
                    .rounded(px(6.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_group(group_kind, cx);
                    }))
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                Icon::new(if closed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(px(11.))
                                .text_color(if empty {
                                    p.sidebar_group_empty_ink
                                } else {
                                    p.sidebar_group_ink
                                }),
                            )
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .font_semibold()
                                    .text_color(if empty {
                                        p.sidebar_group_empty_ink
                                    } else {
                                        p.sidebar_group_ink
                                    })
                                    .child(title),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .font_medium()
                            .text_color(if empty {
                                p.sidebar_group_empty_ink
                            } else {
                                p.sidebar_group_ink
                            })
                            .child(rows.len().to_string()),
                    ),
            );

        if closed || empty {
            return card;
        }

        for (idx, row) in rows.iter().enumerate() {
            let key = session_key(row);
            let selected = self.roster.active_id() == Some(key.as_str());
            let label = {
                let single: String = row
                    .name
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if !single.is_empty() {
                    single
                } else {
                    key.chars().take(12).collect()
                }
            };
            let click_key = key.clone();
            let menu_key = key.clone();
            let menu_id = row.id.clone();
            let menu_file = row.session_file.clone();
            let menu_name = label.clone();
            let is_resident = row.is_resident();
            let dot_color = match group_kind {
                SidebarGroup::Active => Some(p.working),
                SidebarGroup::Idle => Some(p.tag_idle),
                SidebarGroup::Done => None,
            };

            card = card.child(
                h_flex()
                    .id((title, idx))
                    .h(px(28.))
                    .px(px(7.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(p.sidebar_row_selected))
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .when_some(dot_color, |d, c| {
                        d.child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(c))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .text_color(if selected { p.ink } else { p.quiet_ink })
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("sess-dots-{title}-{idx}")))
                            .size(px(22.))
                            .flex_shrink_0()
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(p.sidebar_row_selected))
                            .child(
                                Icon::new(IconName::Ellipsis)
                                    .size(px(12.))
                                    .text_color(p.sidebar_group_empty_ink),
                            )
                            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                let already_open = matches!(
                                    &this.open_popup,
                                    OpenPopup::SessionMenu { key, .. } if key == &menu_key
                                );
                                if already_open {
                                    this.open_popup = OpenPopup::None;
                                } else {
                                    let vp_h = window.viewport_size().height.as_f32();
                                    let top_y = (ev.position().y.as_f32() - 14.0)
                                        .clamp(36.0, (vp_h - 140.0).max(36.0))
                                        .round() as i32;
                                    this.open_popup = OpenPopup::SessionMenu {
                                        key: menu_key.clone(),
                                        id: menu_id.clone(),
                                        file: menu_file.clone(),
                                        name: menu_name.clone(),
                                        is_resident,
                                        anchor_y: top_y,
                                    };
                                }
                                cx.notify();
                            })),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.open_popup = OpenPopup::None;
                        this.hovered_model = None;
                        this.composer_model.set_pending_slash(None);
                        this.roster.select_session_optimistic(&click_key);
                        let _ = this
                            .bridge
                            .tx
                            .send(DaemonCmd::SelectSession(click_key.clone()));
                        this.sync_diff_for_active_session(cx);
                        cx.notify();
                    })),
            );
        }

        card
    }

}
