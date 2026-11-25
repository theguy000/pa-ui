use crate::app::PaApp;
use crate::assets::{ICON_CLOSE_PATH, ICON_SHELLS_PATH, ICON_SUBAGENTS_PATH};
use crate::popups::OpenPopup;
use gpui_kit::component::{h_flex, v_flex, Icon, IconName};
use gpui_kit::{prelude::FluentBuilder as _, *};
use std::collections::HashMap;

/// One dynamic tab hosted in the Right sidebar for a Session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RightTabKind {
    Diff,
    Subagent {
        active_session_id: String,
        child_id: String,
        name: String,
    },
    Shell {
        activity_id: String,
        title: String,
    },
}

impl RightTabKind {
    pub(crate) fn id(&self) -> String {
        match self {
            Self::Diff => "diff".to_string(),
            Self::Subagent {
                active_session_id,
                child_id,
                ..
            } => {
                if !child_id.is_empty() {
                    format!("sub:{child_id}")
                } else {
                    format!("sub:{active_session_id}")
                }
            }
            Self::Shell { activity_id, .. } => format!("sh:{activity_id}"),
        }
    }

    pub(crate) fn title(&self) -> String {
        match self {
            Self::Diff => "Diff".to_string(),
            Self::Subagent { name, .. } => name.clone(),
            Self::Shell { title, .. } => title.clone(),
        }
    }

    pub(crate) fn is_diff(&self) -> bool {
        matches!(self, Self::Diff)
    }
}

#[derive(Debug, Clone, Default)]
struct SessionTabSet {
    tabs: Vec<RightTabKind>,
    active: Option<usize>,
}

impl SessionTabSet {
    fn active_tab(&self) -> Option<&RightTabKind> {
        self.active.and_then(|ix| self.tabs.get(ix))
    }
}

/// All-dynamic, per-Session tab model for the Right sidebar.
///
/// Every tab (including `Diff`) is closeable. Closing the last tab collapses
/// the Right sidebar automatically; toggling it open when a Session has no
/// tabs opens `Diff`.
#[derive(Debug, Clone)]
pub(crate) struct RightSidebarModel {
    is_open: bool,
    current_session: String,
    sessions: HashMap<String, SessionTabSet>,
}

impl RightSidebarModel {
    pub(crate) fn new(initial_open: bool) -> Self {
        let mut model = Self {
            is_open: initial_open,
            current_session: String::new(),
            sessions: HashMap::new(),
        };
        if initial_open {
            model.ensure_default_diff_if_unvisited();
        }
        model
    }

    pub(crate) fn is_open(&self) -> bool {
        self.is_open
    }

    pub(crate) fn set_session(&mut self, session_key: impl Into<String>) {
        let next = session_key.into();
        if self.current_session == next {
            if self.is_open && !self.sessions.contains_key(&next) {
                self.ensure_default_diff_if_unvisited();
            }
            return;
        }
        self.current_session = next;
        if self.is_open {
            if !self.sessions.contains_key(&self.current_session) {
                self.ensure_default_diff_if_unvisited();
            } else if self.tabs().is_empty() {
                self.is_open = false;
            }
        }
    }

    fn ensure_default_diff_if_unvisited(&mut self) {
        self.sessions
            .entry(self.current_session.clone())
            .or_insert_with(|| SessionTabSet {
                tabs: vec![RightTabKind::Diff],
                active: Some(0),
            });
    }

    pub(crate) fn tabs(&self) -> &[RightTabKind] {
        self.sessions
            .get(&self.current_session)
            .map_or(&[], |s| s.tabs.as_slice())
    }

    pub(crate) fn active_index(&self) -> Option<usize> {
        self.sessions
            .get(&self.current_session)
            .and_then(|s| s.active)
    }

    pub(crate) fn active_tab(&self) -> Option<&RightTabKind> {
        self.sessions
            .get(&self.current_session)
            .and_then(SessionTabSet::active_tab)
    }

    pub(crate) fn is_diff_active(&self) -> bool {
        self.is_open && matches!(self.active_tab(), Some(RightTabKind::Diff))
    }

    pub(crate) fn has_diff_tab(&self) -> bool {
        self.tabs().iter().any(RightTabKind::is_diff)
    }

    /// Open (or focus if already open) `tab` in the current Session and
    /// expand the Right sidebar. Returns `true` if `Diff` is now active.
    pub(crate) fn open_tab(&mut self, tab: RightTabKind) -> bool {
        let id = tab.id();
        let entry = self.sessions.entry(self.current_session.clone()).or_default();
        if let Some(pos) = entry.tabs.iter().position(|t| t.id() == id) {
            entry.tabs[pos] = tab;
            entry.active = Some(pos);
        } else {
            entry.tabs.push(tab);
            entry.active = Some(entry.tabs.len() - 1);
        }
        self.is_open = true;
        self.is_diff_active()
    }

    pub(crate) fn activate(&mut self, index: usize) -> bool {
        let Some(entry) = self.sessions.get_mut(&self.current_session) else {
            return false;
        };
        if index >= entry.tabs.len() {
            return false;
        }
        entry.active = Some(index);
        self.is_diff_active()
    }

    /// Close tab at `index`. Returns `true` if the Right sidebar collapsed.
    pub(crate) fn close(&mut self, index: usize) -> bool {
        let Some(entry) = self.sessions.get_mut(&self.current_session) else {
            return false;
        };
        if index >= entry.tabs.len() {
            return false;
        }
        entry.tabs.remove(index);
        if entry.tabs.is_empty() {
            entry.active = None;
            self.is_open = false;
            return true;
        }
        let next_active = match entry.active {
            Some(cur) if cur == index => index.min(entry.tabs.len() - 1),
            Some(cur) if cur > index => cur - 1,
            Some(cur) => cur.min(entry.tabs.len() - 1),
            None => 0,
        };
        entry.active = Some(next_active);
        false
    }

    /// Close a tab in the current Session by its stable `id()`.
    /// Returns `true` if a tab was removed.
    pub(crate) fn close_by_id(&mut self, tab_id: &str) -> bool {
        let Some(pos) = self.tabs().iter().position(|t| t.id() == tab_id) else {
            return false;
        };
        let _ = self.close(pos);
        true
    }

    /// Toggle the Right sidebar open or closed. Opening when the current
    /// Session has zero tabs opens `Diff`. Returns the new `is_open` state.
    pub(crate) fn toggle_open(&mut self) -> bool {
        if self.is_open {
            self.is_open = false;
            return false;
        }
        self.is_open = true;
        if self.tabs().is_empty() {
            let entry = self.sessions.entry(self.current_session.clone()).or_default();
            entry.tabs.push(RightTabKind::Diff);
            entry.active = Some(0);
        } else if self.active_index().is_none()
            && let Some(entry) = self.sessions.get_mut(&self.current_session)
        {
            entry.active = Some(0);
        }
        true
    }
}

impl PaApp {
    pub(crate) fn sync_right_sidebar_session(&mut self) {
        let key = self
            .roster
            .active_id()
            .unwrap_or_default()
            .to_string();
        self.activity_sheet.set_session(&key);
        let was_open = self.right_sidebar.is_open();
        self.right_sidebar.set_session(key);
        if self.right_sidebar.is_open() != was_open {
            self.settings.right_open = self.right_sidebar.is_open();
            self.settings.save();
        }
    }

    pub(crate) fn open_right_tab(&mut self, tab: RightTabKind, cx: &mut Context<Self>) {
        let diff_active = self.right_sidebar.open_tab(tab);
        if self.settings.right_open != self.right_sidebar.is_open() {
            self.settings.right_open = self.right_sidebar.is_open();
            self.settings.save();
        }
        if diff_active {
            self.refresh_diff(cx);
        } else {
            if matches!(self.open_popup, OpenPopup::DiffBaseBranchMenu) {
                self.open_popup = OpenPopup::None;
            }
            cx.notify();
        }
    }

    pub(crate) fn activate_right_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let diff_active = self.right_sidebar.activate(index);
        if diff_active {
            self.refresh_diff(cx);
        } else {
            if matches!(self.open_popup, OpenPopup::DiffBaseBranchMenu) {
                self.open_popup = OpenPopup::None;
            }
            cx.notify();
        }
    }

    pub(crate) fn close_right_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        let collapsed = self.right_sidebar.close(index);
        if collapsed {
            self.settings.right_open = false;
            self.diff_model.cancel_pending_on_close();
            if matches!(self.open_popup, OpenPopup::DiffBaseBranchMenu) {
                self.open_popup = OpenPopup::None;
            }
            self.settings.save();
            cx.notify();
        } else if self.right_sidebar.is_diff_active() {
            self.refresh_diff(cx);
        } else {
            if matches!(self.open_popup, OpenPopup::DiffBaseBranchMenu) {
                self.open_popup = OpenPopup::None;
            }
            cx.notify();
        }
    }

    fn render_right_sidebar_tabbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let tabs = self.right_sidebar.tabs();
        let active_ix = self.right_sidebar.active_index();
        let has_diff = self.right_sidebar.has_diff_tab();

        h_flex()
            .id("right-sidebar-tabbar")
            .w_full()
            .h(px(32.))
            .flex_shrink_0()
            .px(px(6.))
            .gap(px(3.))
            .items_center()
            .border_b_1()
            .border_color(p.sidebar_group_edge)
            .bg(p.window_surface)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .gap(px(3.))
                    .items_center()
                    .overflow_hidden()
                    .children(tabs.iter().enumerate().map(|(ix, tab)| {
                        let is_active = active_ix == Some(ix);
                        let label = tab.title();
                        let tab_icon: AnyElement = match tab {
                            RightTabKind::Diff => Icon::new(IconName::File)
                                .size(px(11.5))
                                .text_color(if is_active { p.ink } else { p.quiet_ink })
                                .into_any_element(),
                            RightTabKind::Subagent { .. } => svg()
                                .path(ICON_SUBAGENTS_PATH)
                                .size(px(11.5))
                                .text_color(if is_active { p.ink } else { p.quiet_ink })
                                .into_any_element(),
                            RightTabKind::Shell { .. } => svg()
                                .path(ICON_SHELLS_PATH)
                                .size(px(11.5))
                                .text_color(if is_active { p.ink } else { p.quiet_ink })
                                .into_any_element(),
                        };

                        h_flex()
                            .id(("right-tab", ix))
                            .h(px(24.))
                            .pl(px(8.))
                            .pr(px(4.))
                            .gap(px(5.))
                            .max_w(px(176.))
                            .rounded(px(6.))
                            .items_center()
                            .cursor_pointer()
                            .bg(if is_active {
                                p.sidebar_row_selected
                            } else {
                                Hsla::transparent_black()
                            })
                            .hover(move |s| s.bg(p.sidebar_row_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.activate_right_tab(ix, cx);
                            }))
                            .child(tab_icon)
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .line_height(px(11.5))
                                    .font_weight(if is_active {
                                        FontWeight::MEDIUM
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(if is_active { p.ink } else { p.quiet_ink })
                                    .child(label),
                            )
                            .child(
                                div()
                                    .id(("right-tab-close", ix))
                                    .size(px(16.))
                                    .rounded(px(4.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .flex_shrink_0()
                                    .hover(move |s| s.bg(p.sidebar_row_selected))
                                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                        window.prevent_default();
                                        cx.stop_propagation();
                                    })
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.close_right_tab(ix, cx);
                                    }))
                                    .child(
                                        svg()
                                            .path(ICON_CLOSE_PATH)
                                            .size(px(9.5))
                                            .text_color(p.quiet_ink),
                                    ),
                            )
                    })),
            )
            .when(!has_diff, |bar| {
                bar.child(
                    h_flex()
                        .id("right-tab-add-diff")
                        .h(px(22.))
                        .px(px(7.))
                        .gap(px(4.))
                        .flex_shrink_0()
                        .rounded(px(5.))
                        .items_center()
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_right_tab(RightTabKind::Diff, cx);
                        }))
                        .child(Icon::new(IconName::Plus).size(px(10.5)).text_color(p.quiet_ink))
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(px(11.))
                                .text_color(p.quiet_ink)
                                .child("Diff"),
                        ),
                )
            })
    }

    pub(crate) fn render_right_column(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.right_sidebar.is_open() {
            return None;
        }
        let p = self.palette();
        let width = self.right_column_width(window);
        if width <= 0.0 {
            return None;
        }
        let handle_active = self.diff_model.is_dragging() || self.diff_model.is_handle_hovered();
        let edge_color = if handle_active {
            p.composer_edge_focus
        } else {
            p.sidebar_group_edge
        };

        let active_body: AnyElement = match self.right_sidebar.active_tab() {
            Some(RightTabKind::Diff) | None => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_hidden()
                .child(self.render_diff_header(cx))
                .children(self.render_diff_peek_banner(cx))
                .children(self.render_diff_jump_list(cx))
                .child(self.render_diff_body(window, cx))
                .into_any_element(),
            Some(RightTabKind::Subagent {
                active_session_id,
                child_id,
                name,
            }) => self.render_right_subagent_tab(active_session_id, child_id, name, cx),
            Some(RightTabKind::Shell { activity_id, title }) => {
                self.render_right_shell_tab(activity_id, title, cx)
            }
        };

        Some(
            h_flex()
                .id("right-diff-column")
                .relative()
                .w(px(width))
                .h_full()
                .flex_shrink_0()
                .bg(p.window_surface)
                .child(
                    div()
                        .id("diff-resize-handle")
                        .w(px(5.))
                        .h_full()
                        .flex_shrink_0()
                        .cursor_col_resize()
                        .flex()
                        .justify_start()
                        .child(div().w(px(1.)).h_full().bg(edge_color))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if this.diff_model.is_handle_hovered() != *hovered {
                                this.diff_model.set_handle_hovered(*hovered);
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                window.prevent_default();
                                if ev.click_count >= 2 {
                                    this.reset_right_column_width(cx);
                                    return;
                                }
                                this.begin_right_column_drag(ev.position.x.as_f32(), window, cx);
                            }),
                        ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .overflow_hidden()
                        .child(self.render_right_sidebar_tabbar(cx))
                        .child(active_body),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(id: &str, name: &str) -> RightTabKind {
        RightTabKind::Subagent {
            active_session_id: format!("sess-{id}"),
            child_id: id.to_string(),
            name: name.to_string(),
        }
    }

    fn shell(id: &str, title: &str) -> RightTabKind {
        RightTabKind::Shell {
            activity_id: id.to_string(),
            title: title.to_string(),
        }
    }

    #[::core::prelude::v1::test]
    fn toggle_open_from_empty_opens_diff_and_recloses() {
        let mut m = RightSidebarModel::new(false);
        assert!(!m.is_open());
        assert!(m.tabs().is_empty());

        assert!(m.toggle_open());
        assert!(m.is_open());
        assert!(m.is_diff_active());
        assert_eq!(m.tabs(), &[RightTabKind::Diff]);

        assert!(!m.toggle_open());
        assert!(!m.is_open());
        assert!(!m.is_diff_active());
    }

    #[::core::prelude::v1::test]
    fn open_tab_dedupes_focuses_and_auto_expands() {
        let mut m = RightSidebarModel::new(false);
        m.set_session("s1");

        assert!(!m.open_tab(sub("c1", "audit")));
        assert!(m.is_open());
        assert_eq!(m.tabs().len(), 1);
        assert_eq!(m.active_index(), Some(0));

        assert!(!m.open_tab(shell("sh1", "cargo test")));
        assert_eq!(m.tabs().len(), 2);
        assert_eq!(m.active_index(), Some(1));

        // Re-opening existing subagent updates title and focuses index 0 without duplicating.
        assert!(!m.open_tab(sub("c1", "audit-renamed")));
        assert_eq!(m.tabs().len(), 2);
        assert_eq!(m.active_index(), Some(0));
        assert_eq!(m.active_tab().map(RightTabKind::title).as_deref(), Some("audit-renamed"));
    }

    #[::core::prelude::v1::test]
    fn close_tab_selects_neighbor_and_auto_collapses_on_last_tab() {
        let mut m = RightSidebarModel::new(true);
        m.set_session("s1");
        m.open_tab(sub("c1", "audit"));
        m.open_tab(shell("sh1", "cargo test"));
        assert_eq!(m.tabs().len(), 3);
        assert_eq!(m.active_index(), Some(2));

        // Close active last tab -> neighbor (index 1) becomes active.
        assert!(!m.close(2));
        assert!(m.is_open());
        assert_eq!(m.active_index(), Some(1));

        // Close middle tab -> Diff (index 0) becomes active.
        assert!(!m.close(1));
        assert!(m.is_diff_active());

        // Close Diff (last tab) -> auto-collapses.
        assert!(m.close(0));
        assert!(!m.is_open());
        assert!(m.tabs().is_empty());

        // Re-toggling open creates Diff again.
        assert!(m.toggle_open());
        assert!(m.is_diff_active());
    }

    #[::core::prelude::v1::test]
    fn closing_diff_exposes_has_diff_false_and_reopen_diff_restores_it() {
        let mut m = RightSidebarModel::new(true);
        m.set_session("s1");
        m.open_tab(shell("sh1", "git status"));
        assert!(m.has_diff_tab());

        assert!(!m.close(0));
        assert!(m.is_open());
        assert!(!m.has_diff_tab());
        assert_eq!(m.active_index(), Some(0));

        assert!(m.open_tab(RightTabKind::Diff));
        assert!(m.has_diff_tab());
        assert!(m.is_diff_active());
    }

    #[::core::prelude::v1::test]
    fn switching_sessions_swaps_tabs_without_leaking() {
        let mut m = RightSidebarModel::new(true);
        m.set_session("s1");
        m.open_tab(sub("c1", "worker-s1"));
        assert_eq!(m.tabs().len(), 2);

        // Switching to unvisited Session s2 while open seeds Diff only (no s1 subagent leak).
        m.set_session("s2");
        assert!(m.is_open());
        assert_eq!(m.tabs(), &[RightTabKind::Diff]);
        assert!(m.is_diff_active());

        // Switching back to s1 restores s1's 2 tabs and active subagent tab.
        m.set_session("s1");
        assert_eq!(m.tabs().len(), 2);
        assert_eq!(m.active_index(), Some(1));
        assert_eq!(m.active_tab().map(RightTabKind::title).as_deref(), Some("worker-s1"));
    }

    #[::core::prelude::v1::test]
    fn close_by_id_removes_matching_tab_and_collapses_if_last() {
        let mut m = RightSidebarModel::new(false);
        m.set_session("s1");
        m.open_tab(sub("c1", "worker"));
        assert!(!m.close_by_id("sub:missing"));
        assert!(m.close_by_id("sub:c1"));
        assert!(!m.is_open());
        assert!(m.tabs().is_empty());
    }
}
