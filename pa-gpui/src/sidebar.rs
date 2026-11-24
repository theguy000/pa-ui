//! Sidebar & window chrome module: custom TitleBar, collapsed icon rail,
//! grouped session list (Active / Idle / Done), inline rename, and title bar.

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

const WORKSPACE_DONE_PREVIEW: usize = 5;

#[allow(dead_code)]
pub(crate) fn display_folder_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    if let Some(home_path) = home {
        let home_str = home_path.to_string_lossy();
        let same_home = {
            #[cfg(windows)]
            {
                trimmed.trim_end_matches(['\\', '/']).eq_ignore_ascii_case(home_str.trim_end_matches(['\\', '/']))
            }
            #[cfg(not(windows))]
            {
                trimmed.trim_end_matches('/') == home_str.trim_end_matches('/')
            }
        };
        if same_home {
            return "~".to_string();
        }
    }
    std::path::Path::new(trimmed)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(trimmed)
        .to_string()
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

    pub(crate) fn toggle_live_radar(&mut self, cx: &mut Context<Self>) {
        self.settings.live_radar_closed = !self.settings.live_radar_closed;
        self.settings.save();
        cx.notify();
    }

    pub(crate) fn toggle_workspace_group(&mut self, key: &str, cx: &mut Context<Self>) {
        self.settings.toggle_workspace_collapsed(key);
        if self.settings.is_workspace_collapsed(key) {
            self.expanded_older_workspaces.remove(key);
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
        let left_offset = if self.settings.sidebar_open { 246. } else { 46. };
        let dev_open = matches!(self.open_popup, OpenPopup::DevErrors { .. });
        let dev_enabled = self.dev_errors.is_enabled();
        let dev_count = self.dev_errors.len();

        TitleBar::new()
            .h(px(32.))
            .bg(p.window_surface)
            .border_b_0()
            .child(
                h_flex()
                    .w_full()
                    .h_full()
                    .pl(px(left_offset))
                    .pr(px(8.))
                    .gap(px(4.))
                    .items_center()
                    .child(self.render_titlebar_workspace_context(cx))
                    .child(div().flex_1().h_full())
                    .when(dev_enabled, |bar| {
                        bar.child(
                            h_flex()
                                .id("tb-dev-info")
                                .h(px(26.))
                                .px(px(if dev_count > 0 { 7. } else { 6. }))
                                .gap(px(4.))
                                .rounded(px(6.))
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .when(dev_open, |d| d.bg(p.sidebar_row_selected))
                                .hover(|d| d.bg(p.sidebar_row_hover))
                                .child(
                                    Icon::new(IconName::Info)
                                        .size(px(14.))
                                        .text_color(if dev_count > 0 {
                                            p.danger
                                        } else {
                                            p.quiet_ink
                                        }),
                                )
                                .when(dev_count > 0, |d| {
                                    d.child(
                                        div()
                                            .text_size(px(10.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(p.danger)
                                            .child(format!("{dev_count}")),
                                    )
                                })
                                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                    window.prevent_default();
                                    cx.stop_propagation();
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.open_popup =
                                        if matches!(this.open_popup, OpenPopup::DevErrors { .. }) {
                                            OpenPopup::None
                                        } else {
                                            OpenPopup::DevErrors { copied: None }
                                        };
                                    cx.notify();
                                })),
                        )
                    })
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
                                    let _ = this.bridge.tx.send(DaemonCmd::RefreshWorkspaces);
                                    if this.settings_tab == crate::app::SettingsTab::Harness {
                                        this.refresh_harness_view(cx);
                                    }
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
            .flex_shrink_0()
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
        let has_query = !query.trim().is_empty();
        let view = self
            .roster
            .workspace_sidebar_view(&self.workspace_ui.workspaces, &query);

        v_flex()
            .w(px(244.))
            .flex_shrink_0()
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
                                        this.expanded_older_workspaces.clear();
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
                    .child({
                        let mut col = v_flex().w_full().gap(px(4.)).pt(px(2.));

                        // Variant B: Pinned Live radar (Running + Idle with Workspace tag)
                        if !view.live.is_empty() {
                            col = col.child(self.render_live_radar(&view.live, has_query, cx));
                            col = col.child(
                                div()
                                    .w_full()
                                    .h(px(1.))
                                    .my(px(2.))
                                    .bg(p.sidebar_group_edge),
                            );
                        }

                        if view.groups.is_empty() {
                            col = col.child(
                                div()
                                    .w_full()
                                    .py(px(24.))
                                    .flex()
                                    .justify_center()
                                    .text_size(px(12.))
                                    .text_color(p.sidebar_group_empty_ink)
                                    .child(if has_query {
                                        "No matching sessions"
                                    } else {
                                        "No sessions yet"
                                    }),
                            );
                        } else {
                            for grp in &view.groups {
                                col = col.child(self.render_workspace_group(grp, has_query, cx));
                            }
                        }

                        col
                    }),
            )
            .into_any_element()
    }

    fn render_live_radar(
        &self,
        live_rows: &[(&SessionRow, String)],
        force_open: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let closed = self.settings.live_radar_closed && !force_open;
        let mut section = v_flex()
            .w_full()
            .gap(px(2.))
            .child(
                h_flex()
                    .id("sb-live-radar-hdr")
                    .h(px(24.))
                    .w_full()
                    .px(px(6.))
                    .rounded(px(6.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_live_radar(cx);
                    }))
                    .child(
                        h_flex()
                            .gap(px(5.))
                            .items_center()
                            .child(
                                Icon::new(if closed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(px(10.))
                                .text_color(p.quiet_ink),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(p.sidebar_group_ink)
                                    .child("Live"),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.quiet_ink)
                            .child(live_rows.len().to_string()),
                    ),
            );

        if closed {
            return section;
        }

        for (idx, (row, ws_name)) in live_rows.iter().enumerate() {
            section = section.child(self.render_live_row(idx, row, ws_name, cx));
        }
        section
    }

    fn render_live_row(
        &self,
        idx: usize,
        row: &SessionRow,
        ws_name: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let key = session_key(row);
        let selected = self
            .roster
            .active_id()
            .is_some_and(|sid| crate::roster::row_matches_selector(row, sid));
        let menu_open = matches!(
            &self.open_popup,
            OpenPopup::SessionMenu { key: open_k, .. } if open_k == &key
        );
        let label = format_session_label(row, &key);
        let click_key = key.clone();
        let menu_key = key.clone();
        let menu_id = row.id.clone();
        let menu_file = row.session_file.clone();
        let menu_name = label.clone();
        let is_resident = row.is_resident();
        let dot_color = if row.roster_status() == "Running" {
            p.working
        } else {
            p.tag_idle
        };
        let row_group: SharedString = format!("sb-live-grp-{idx}").into();

        v_flex()
            .id(SharedString::from(format!("sess-live-{idx}")))
            .group(row_group.clone())
            .w_full()
            .px(px(8.))
            .py(px(6.))
            .gap(px(2.))
            .rounded(px(7.))
            .cursor_pointer()
            .when(selected, |d| d.bg(p.sidebar_row_selected))
            .hover(|d| d.bg(p.sidebar_row_hover))
            .child(
                h_flex()
                    .w_full()
                    .h(px(20.))
                    .gap(px(7.))
                    .items_center()
                    .child(div().size(px(6.)).flex_shrink_0().rounded_full().bg(dot_color))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(12.5))
                            .font_weight(if selected {
                                FontWeight::MEDIUM
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(p.ink)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(label),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!("sess-dots-live-{idx}")))
                            .h(px(20.))
                            .flex_shrink_0()
                            .overflow_hidden()
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .when(menu_open, |d| d.w(px(22.)).opacity(1.0))
                            .when(!menu_open, |d| {
                                d.w(px(0.)).opacity(0.0).group_hover(row_group, |s| {
                                    s.w(px(22.)).opacity(1.0)
                                })
                            })
                            .hover(|d| d.bg(p.sidebar_row_selected))
                            .child(
                                Icon::new(IconName::Ellipsis)
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                            )
                            .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.toggle_session_menu(
                                    menu_key.clone(),
                                    menu_id.clone(),
                                    menu_file.clone(),
                                    menu_name.clone(),
                                    is_resident,
                                    ev,
                                    window,
                                    cx,
                                );
                            })),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .pl(px(13.))
                    .text_size(px(11.))
                    .text_color(p.quiet_ink)
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_ellipsis()
                    .child(ws_name.to_string()),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.select_session(click_key.clone(), cx);
            }))
    }

    fn render_workspace_group(
        &self,
        grp: &crate::roster::WorkspaceSidebarGroup<'_>,
        force_open: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let closed = !force_open && self.settings.is_workspace_collapsed(&grp.key);
        let toggle_key = grp.key.clone();
        let add_ws_id = grp.workspace_id.clone();
        let add_root = grp.root_path.clone();
        let older_key = grp.key.clone();
        let expanded_older = force_open || self.expanded_older_workspaces.contains(&grp.key);

        let visible_limit = if expanded_older {
            grp.rows.len()
        } else {
            WORKSPACE_DONE_PREVIEW
        };
        let hidden_older = grp.rows.len().saturating_sub(visible_limit);
        let hdr_group: SharedString = format!("ws-hdr-grp-{}", grp.key).into();
        let folder_icon = if closed {
            IconName::Folder
        } else {
            IconName::FolderOpen
        };

        let mut card = v_flex()
            .w_full()
            .mt(px(4.))
            .gap(px(2.))
            .child(
                h_flex()
                    .id(SharedString::from(format!("ws-hdr-{}", grp.key)))
                    .group(hdr_group.clone())
                    .h(px(28.))
                    .w_full()
                    .px(px(6.))
                    .rounded(px(6.))
                    .items_center()
                    .justify_between()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.toggle_workspace_group(&toggle_key, cx);
                    }))
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                Icon::new(if closed {
                                    IconName::ChevronRight
                                } else {
                                    IconName::ChevronDown
                                })
                                .size(px(10.))
                                .text_color(p.quiet_ink),
                            )
                            .child(
                                Icon::new(folder_icon)
                                    .size(px(13.))
                                    .text_color(p.quiet_ink),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.ink)
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(grp.name.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap(px(4.))
                            .items_center()
                            .flex_shrink_0()
                            .when(!grp.rows.is_empty(), |d| {
                                d.child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.sidebar_group_empty_ink)
                                        .child(grp.rows.len().to_string()),
                                )
                            })
                            .child(
                                div()
                                    .id(SharedString::from(format!("ws-add-{}", grp.key)))
                                    .size(px(20.))
                                    .rounded(px(5.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .opacity(0.0)
                                    .group_hover(hdr_group, |s| s.opacity(1.0))
                                    .hover(|d| d.bg(p.sidebar_row_selected))
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(px(12.))
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.request_new_session_in_workspace(
                                            add_ws_id.clone(),
                                            add_root.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    ),
            );

        if closed {
            return card;
        }

        if grp.rows.is_empty() {
            if grp.live_count == 0 {
                card = card.child(
                    div()
                        .pl(px(26.))
                        .py(px(3.))
                        .text_size(px(11.5))
                        .text_color(p.quiet_ink)
                        .child("No sessions yet"),
                );
            }
            return card;
        }

        let scope_prefix = format!("ws-{}", grp.key);
        for (idx, row) in grp.rows.iter().take(visible_limit).enumerate() {
            card = card.child(self.render_done_row(&scope_prefix, idx, row, cx));
        }

        if hidden_older > 0 {
            card = card.child(
                div()
                    .id(SharedString::from(format!("ws-older-{}", grp.key)))
                    .pl(px(26.))
                    .h(px(24.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .text_size(px(11.))
                    .text_color(p.quiet_ink)
                    .child(format!("Show {hidden_older} older"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.expanded_older_workspaces.insert(older_key.clone());
                        cx.notify();
                    })),
            );
        }

        card
    }

    fn render_done_row(
        &self,
        scope: &str,
        idx: usize,
        row: &SessionRow,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let key = session_key(row);
        let selected = self
            .roster
            .active_id()
            .is_some_and(|sid| crate::roster::row_matches_selector(row, sid));
        let menu_open = matches!(
            &self.open_popup,
            OpenPopup::SessionMenu { key: open_k, .. } if open_k == &key
        );
        let label = format_session_label(row, &key);
        let click_key = key.clone();
        let menu_key = key.clone();
        let menu_id = row.id.clone();
        let menu_file = row.session_file.clone();
        let menu_name = label.clone();
        let is_resident = row.is_resident();
        let row_group: SharedString = format!("sb-done-grp-{scope}-{idx}").into();

        h_flex()
            .id(SharedString::from(format!("sess-row-{scope}-{idx}")))
            .group(row_group.clone())
            .h(px(28.))
            .pl(px(26.))
            .pr(px(6.))
            .gap(px(4.))
            .items_center()
            .rounded(px(6.))
            .cursor_pointer()
            .when(selected, |d| d.bg(p.sidebar_row_selected))
            .hover(|d| d.bg(p.sidebar_row_hover))
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
                    .id(SharedString::from(format!("sess-dots-{scope}-{idx}")))
                    .h(px(22.))
                    .flex_shrink_0()
                    .overflow_hidden()
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(menu_open, |d| d.w(px(22.)).opacity(1.0))
                    .when(!menu_open, |d| {
                        d.w(px(0.)).opacity(0.0).group_hover(row_group, |s| {
                            s.w(px(22.)).opacity(1.0)
                        })
                    })
                    .hover(|d| d.bg(p.sidebar_row_selected))
                    .child(
                        Icon::new(IconName::Ellipsis)
                            .size(px(12.))
                            .text_color(p.quiet_ink),
                    )
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.toggle_session_menu(
                            menu_key.clone(),
                            menu_id.clone(),
                            menu_file.clone(),
                            menu_name.clone(),
                            is_resident,
                            ev,
                            window,
                            cx,
                        );
                    })),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.select_session(click_key.clone(), cx);
            }))
    }

    #[allow(clippy::too_many_arguments)]
    fn toggle_session_menu(
        &mut self,
        menu_key: String,
        menu_id: String,
        menu_file: String,
        menu_name: String,
        is_resident: bool,
        ev: &ClickEvent,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let already_open = matches!(
            &self.open_popup,
            OpenPopup::SessionMenu { key, .. } if key == &menu_key
        );
        if already_open {
            self.open_popup = OpenPopup::None;
        } else {
            let vp_h = window.viewport_size().height.as_f32();
            let top_y = (ev.position().y.as_f32() - 14.0)
                .clamp(36.0, (vp_h - 140.0).max(36.0))
                .round() as i32;
            self.open_popup = OpenPopup::SessionMenu {
                key: menu_key,
                id: menu_id,
                file: menu_file,
                name: menu_name,
                is_resident,
                anchor_y: top_y,
            };
        }
        cx.notify();
    }
}

fn format_session_label(row: &SessionRow, key: &str) -> String {
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
}
