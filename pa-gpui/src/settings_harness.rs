//! Expanded Settings tabs: General Worktree Root, Harness Master-Detail manager
//! (Global | Workspace | Session | History + multi-scope Rollback), and Workspaces
//! management (Missing / Relink / Delete) — Spec #140 Slice 3 (Variant A).

use crate::app::PaApp;
use crate::daemon::DaemonCmd;
use gpui_kit::component::{
    h_flex,
    input::{Input, Textarea},
    scroll::ScrollableElement as _,
    v_flex, Icon, IconName,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{
    HarnessEntryWire, HarnessKindWire, HarnessScopeWire, RefinementHistoryItemWire, WorkspaceKind,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HarnessUiScope {
    Global,
    Workspace,
    Session,
    History,
}

impl HarnessUiScope {
    pub(crate) fn wire_scope(self) -> Option<HarnessScopeWire> {
        match self {
            Self::Global => Some(HarnessScopeWire::Global),
            Self::Workspace => Some(HarnessScopeWire::Workspace),
            Self::Session => Some(HarnessScopeWire::Session),
            Self::History => None,
        }
    }
}

pub(crate) fn harness_kind_label(kind: HarnessKindWire) -> &'static str {
    match kind {
        HarnessKindWire::Memory => "Memory",
        HarnessKindWire::Prompt => "Prompt",
        HarnessKindWire::Skill => "Skill",
        HarnessKindWire::Subagent => "Subagent",
        HarnessKindWire::Factory => "Factory",
    }
}

pub(crate) fn harness_scope_label(scope: HarnessScopeWire) -> &'static str {
    match scope {
        HarnessScopeWire::Global => "Global",
        HarnessScopeWire::Workspace => "Workspace",
        HarnessScopeWire::Session => "Session",
    }
}

pub(crate) fn filter_harness_entries<'a>(
    entries: &'a [HarnessEntryWire],
    kind_filter: Option<HarnessKindWire>,
    query: &str,
) -> Vec<&'a HarnessEntryWire> {
    let q = query.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| {
            if let Some(k) = kind_filter {
                if e.kind != k {
                    return false;
                }
            }
            if q.is_empty() {
                return true;
            }
            e.title.to_lowercase().contains(&q)
                || e.content.to_lowercase().contains(&q)
                || e.path.to_lowercase().contains(&q)
                || e.id.to_lowercase().contains(&q)
        })
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct HarnessUiState {
    pub scope: HarnessUiScope,
    pub selected_workspace_id: Option<String>,
    pub kind_filter: Option<HarnessKindWire>,
    pub entries: Vec<HarnessEntryWire>,
    pub history: Vec<RefinementHistoryItemWire>,
    pub selected_entry_id: Option<String>,
    pub creating_new: bool,
    pub editor_kind: HarnessKindWire,
    pub pending_delete_id: Option<String>,
    pub pending_rollback_id: Option<String>,
    pub notice: Option<(bool, String)>,
}

impl Default for HarnessUiState {
    fn default() -> Self {
        Self {
            scope: HarnessUiScope::Global,
            selected_workspace_id: None,
            kind_filter: None,
            entries: Vec::new(),
            history: Vec::new(),
            selected_entry_id: None,
            creating_new: false,
            editor_kind: HarnessKindWire::Memory,
            pending_delete_id: None,
            pending_rollback_id: None,
            notice: None,
        }
    }
}

impl PaApp {
    /// Resolve which workspace id Settings -> Harness should query when on Workspace or History.
    pub(crate) fn effective_harness_workspace_id(&self) -> Option<String> {
        if let Some(id) = self.harness_ui.selected_workspace_id.as_ref() {
            if self.workspace_ui.workspace_by_id(id).is_some() {
                return Some(id.clone());
            }
        }
        self.current_titlebar_breadcrumb()
            .workspace_id
            .or_else(|| self.workspace_ui.workspaces.first().map(|w| w.id.clone()))
    }

    /// Resolve which active session id Settings -> Harness should query.
    pub(crate) fn effective_harness_session_id(&self) -> Option<String> {
        if self.roster.is_draft_or_empty() {
            return None;
        }
        self.roster.active_id().map(str::to_string)
    }

    /// Trigger a daemon fetch for the active Settings -> Harness scope or History.
    pub(crate) fn refresh_harness_view(&mut self, cx: &mut Context<Self>) {
        if self.harness_ui.selected_workspace_id.is_none() {
            self.harness_ui.selected_workspace_id = self.effective_harness_workspace_id();
        }
        let ws_id = self.effective_harness_workspace_id();
        let sid = self.effective_harness_session_id();
        match self.harness_ui.scope {
            HarnessUiScope::History => {
                let _ = self.bridge.tx.send(DaemonCmd::HarnessHistory {
                    scope: None,
                    workspace_id: ws_id,
                    active_session_id: sid,
                });
            }
            other => {
                if other == HarnessUiScope::Workspace && ws_id.is_none() {
                    self.harness_ui.entries.clear();
                    cx.notify();
                    return;
                }
                if other == HarnessUiScope::Session && sid.is_none() {
                    self.harness_ui.entries.clear();
                    cx.notify();
                    return;
                }
                let _ = self.bridge.tx.send(DaemonCmd::HarnessList {
                    scope: other.wire_scope(),
                    workspace_id: ws_id,
                    active_session_id: sid,
                    kind: None,
                    query: None,
                });
            }
        }
    }

    /// Populate the right-side Harness editor inputs from a selected entry.
    pub(crate) fn select_harness_entry(
        &mut self,
        entry: &HarnessEntryWire,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.harness_ui.creating_new = false;
        self.harness_ui.selected_entry_id = Some(entry.id.clone());
        self.harness_ui.editor_kind = entry.kind;
        self.harness_ui.pending_delete_id = None;
        self.harness_ui.notice = None;
        let title = entry.title.clone();
        let path = entry.path.clone();
        let content = entry.content.clone();
        self.harness_title_input
            .update(cx, |s, cx| s.set_value(&title, window, cx));
        self.harness_path_input
            .update(cx, |s, cx| s.set_value(&path, window, cx));
        self.harness_content_input
            .update(cx, |s, cx| s.set_value(&content, window, cx));
        cx.notify();
    }

    /// Enter inline "+ New" entry creation mode in the right-side Harness editor.
    pub(crate) fn begin_new_harness_entry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.harness_ui.creating_new = true;
        self.harness_ui.selected_entry_id = None;
        self.harness_ui.editor_kind = self
            .harness_ui
            .kind_filter
            .unwrap_or(HarnessKindWire::Memory);
        self.harness_ui.pending_delete_id = None;
        self.harness_ui.notice = None;
        let default_path = if self.harness_ui.editor_kind == HarnessKindWire::Prompt {
            "policy"
        } else {
            "general"
        };
        self.harness_title_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.harness_path_input
            .update(cx, |s, cx| s.set_value(default_path, window, cx));
        self.harness_content_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.harness_title_input
            .update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    /// Save the currently edited or created Harness entry via daemon `harness_upsert`.
    pub(crate) fn save_harness_editor(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.harness_ui.scope.wire_scope() else {
            return;
        };
        let title = self.harness_title_input.read(cx).value().trim().to_string();
        let path = self.harness_path_input.read(cx).value().trim().to_string();
        let content = self
            .harness_content_input
            .read(cx)
            .value()
            .trim()
            .to_string();
        if title.is_empty() || content.is_empty() {
            self.harness_ui.notice = Some((false, "Title and content are required".into()));
            cx.notify();
            return;
        }
        let ws_id = (scope == HarnessScopeWire::Workspace)
            .then(|| self.effective_harness_workspace_id())
            .flatten();
        let sid = (scope == HarnessScopeWire::Session)
            .then(|| self.effective_harness_session_id())
            .flatten();
        let entry_id = if self.harness_ui.creating_new {
            None
        } else {
            self.harness_ui.selected_entry_id.clone()
        };
        let _ = self.bridge.tx.send(DaemonCmd::HarnessUpsert {
            scope,
            workspace_id: ws_id,
            active_session_id: sid,
            entry_id,
            kind: self.harness_ui.editor_kind,
            title,
            content,
            path: (!path.is_empty()).then_some(path),
        });
        cx.notify();
    }

    /// Delete the selected Harness entry via daemon `harness_delete`.
    pub(crate) fn confirm_delete_harness_entry(&mut self, cx: &mut Context<Self>) {
        let Some(scope) = self.harness_ui.scope.wire_scope() else {
            return;
        };
        let Some(entry_id) = self.harness_ui.selected_entry_id.clone() else {
            return;
        };
        let ws_id = (scope == HarnessScopeWire::Workspace)
            .then(|| self.effective_harness_workspace_id())
            .flatten();
        let sid = (scope == HarnessScopeWire::Session)
            .then(|| self.effective_harness_session_id())
            .flatten();
        self.harness_ui.pending_delete_id = None;
        let _ = self.bridge.tx.send(DaemonCmd::HarnessDelete {
            scope,
            workspace_id: ws_id,
            active_session_id: sid,
            kind: self.harness_ui.editor_kind,
            entry_id,
        });
        cx.notify();
    }

    /// Worktree Root section rendered inside Settings -> General.
    pub(crate) fn render_settings_general_worktree_root(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let root_display = if self.workspace_ui.worktree_root.is_empty() {
            crate::settings::default_worktree_root()
        } else {
            self.workspace_ui.worktree_root.clone()
        };

        v_flex()
            .w_full()
            .gap(px(8.))
            .pt(px(14.))
            .border_t_1()
            .border_color(p.sidebar_group_edge)
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child("Worktree root"),
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
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.sidebar_group_surface)
                            .flex()
                            .items_center()
                            .child(
                                div()
                                    .w_full()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(p.ink)
                                    .child(root_display),
                            ),
                    )
                    .child(
                        div()
                            .id("settings-wt-root-browse")
                            .h(px(32.))
                            .px(px(12.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.sidebar_group_surface)
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.pick_worktree_root(cx);
                            }))
                            .child("Browse"),
                    )
                    .child(
                        div()
                            .id("settings-wt-root-reset")
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(|this, _, _, cx| {
                                match crate::settings::save_agent_worktree_root(None) {
                                    Ok(effective) => {
                                        this.workspace_ui.worktree_root = effective;
                                        this.workspace_ui.notice =
                                            Some((true, "Reset to default worktree root".into()));
                                        let _ = this.bridge.tx.send(DaemonCmd::RefreshWorkspaces);
                                    }
                                    Err(err) => {
                                        this.workspace_ui.notice = Some((false, err.to_string()));
                                    }
                                }
                                cx.notify();
                            }))
                            .child("Reset"),
                    ),
            )
            .when_some(self.workspace_ui.notice.as_ref(), |col, (ok, msg)| {
                col.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(if *ok { p.quiet_ink } else { p.danger })
                        .child(msg.clone()),
                )
            })
            .into_any_element()
    }

    /// Settings -> Harness tab (Variant A Master-Detail Split + multi-scope History Rollback).
    pub(crate) fn render_settings_harness_tab(
        &self,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let active_scope = self.harness_ui.scope;

        // Borderless scope switcher matching Live | Done style (no border, zero text shift)
        let scope_tab = |id: &'static str, label: &'static str, target: HarnessUiScope| {
            let active = active_scope == target;
            div()
                .id(id)
                .h(px(26.))
                .px(px(10.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .bg(if active {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .text_color(if active { p.ink } else { p.quiet_ink })
                .hover(move |s| {
                    if active {
                        s
                    } else {
                        s.bg(p.sidebar_row_hover).text_color(p.ink)
                    }
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.harness_ui.scope = target;
                    this.harness_ui.selected_entry_id = None;
                    this.harness_ui.creating_new = false;
                    this.harness_ui.pending_delete_id = None;
                    this.harness_ui.pending_rollback_id = None;
                    this.harness_ui.notice = None;
                    this.refresh_harness_view(cx);
                    cx.notify();
                }))
                .child(label)
        };

        let top_bar = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .child(
                h_flex()
                    .gap(px(2.))
                    .items_center()
                    .child(scope_tab("h-scope-global", "Global", HarnessUiScope::Global))
                    .child(scope_tab("h-scope-ws", "Workspace", HarnessUiScope::Workspace))
                    .child(scope_tab("h-scope-sess", "Session", HarnessUiScope::Session))
                    .child(scope_tab("h-scope-hist", "History", HarnessUiScope::History)),
            )
            .when(active_scope != HarnessUiScope::History, |row| {
                let can_create = match active_scope {
                    HarnessUiScope::Global => true,
                    HarnessUiScope::Workspace => self.effective_harness_workspace_id().is_some(),
                    HarnessUiScope::Session => self.effective_harness_session_id().is_some(),
                    HarnessUiScope::History => false,
                };
                row.when(can_create, |r| {
                    r.child(
                        h_flex()
                            .id("harness-new-btn")
                            .h(px(26.))
                            .px(px(10.))
                            .gap(px(5.))
                            .rounded(px(6.))
                            .bg(p.sidebar_row_selected)
                            .items_center()
                            .cursor_pointer()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin_new_harness_entry(window, cx);
                            }))
                            .child(Icon::new(IconName::Plus).size(px(12.)).text_color(p.ink))
                            .child("New"),
                    )
                })
            });

        // Workspace selector strip when Workspace scope is active
        let ws_strip: Option<AnyElement> = if active_scope == HarnessUiScope::Workspace {
            let selected_ws = self.effective_harness_workspace_id();
            let mut strip = h_flex().w_full().gap(px(6.)).flex_wrap().items_center();
            if self.workspace_ui.workspaces.is_empty() {
                strip = strip.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.quiet_ink)
                        .child("No registered workspaces"),
                );
            } else {
                for (i, ws) in self.workspace_ui.workspaces.iter().enumerate() {
                    let is_sel = selected_ws.as_deref() == Some(ws.id.as_str());
                    let ws_id_click = ws.id.clone();
                    strip = strip.child(
                        div()
                            .id(("h-ws-chip", i))
                            .h(px(23.))
                            .px(px(8.))
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .font_weight(FontWeight::MEDIUM)
                            .bg(if is_sel {
                                p.sidebar_row_selected
                            } else {
                                p.sidebar_group_surface
                            })
                            .text_color(if is_sel { p.ink } else { p.quiet_ink })
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.harness_ui.selected_workspace_id = Some(ws_id_click.clone());
                                this.harness_ui.selected_entry_id = None;
                                this.harness_ui.creating_new = false;
                                this.refresh_harness_view(cx);
                                cx.notify();
                            }))
                            .child(ws.name.clone()),
                    );
                }
            }
            Some(strip.into_any_element())
        } else {
            None
        };

        // Main body: History view vs Master-Detail Split
        let body: AnyElement = if active_scope == HarnessUiScope::History {
            self.render_harness_history_body(cx)
        } else {
            self.render_harness_master_detail(cx)
        };

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap(px(10.))
            .child(top_bar)
            .children(ws_strip)
            .child(body)
            .into_any_element()
    }

    fn render_harness_master_detail(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let query = self.harness_search_input.read(cx).value().to_string();
        let filtered = filter_harness_entries(&self.harness_ui.entries, self.harness_ui.kind_filter, &query);

        let kind_pill = |id: &'static str, label: &'static str, kind: Option<HarnessKindWire>| {
            let active = self.harness_ui.kind_filter == kind;
            div()
                .id(id)
                .h(px(22.))
                .px(px(8.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .cursor_pointer()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .bg(if active {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .text_color(if active { p.ink } else { p.quiet_ink })
                .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.harness_ui.kind_filter = kind;
                    cx.notify();
                }))
                .child(label)
        };

        let filter_row = h_flex()
            .w_full()
            .gap(px(8.))
            .items_center()
            .child(
                div()
                    .w(px(210.))
                    .child(Input::new(&self.harness_search_input).cleanable(true)),
            )
            .child(
                h_flex()
                    .gap(px(2.))
                    .items_center()
                    .child(kind_pill("hk-all", "All", None))
                    .child(kind_pill("hk-mem", "Memory", Some(HarnessKindWire::Memory)))
                    .child(kind_pill("hk-prm", "Prompt", Some(HarnessKindWire::Prompt)))
                    .child(kind_pill("hk-skl", "Skill", Some(HarnessKindWire::Skill)))
                    .child(kind_pill("hk-sub", "Subagent", Some(HarnessKindWire::Subagent))),
            );

        // Left Master list (~276px)
        let mut master = v_flex()
            .id("harness-master-list")
            .w(px(276.))
            .h_full()
            .pr(px(8.))
            .border_r_1()
            .border_color(p.sidebar_group_edge)
            .overflow_y_scrollbar()
            .gap(px(3.));

        if self.harness_ui.scope == HarnessUiScope::Session
            && self.effective_harness_session_id().is_none()
        {
            master = master.child(
                div()
                    .p(px(10.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("Open a session to view session entries"),
            );
        } else if filtered.is_empty() {
            master = master.child(
                div()
                    .p(px(10.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No entries"),
            );
        } else {
            for (idx, entry) in filtered.iter().enumerate() {
                let is_sel = !self.harness_ui.creating_new
                    && self.harness_ui.selected_entry_id.as_deref() == Some(entry.id.as_str());
                let entry_clone = (*entry).clone();
                master = master.child(
                    v_flex()
                        .id(("harness-row", idx))
                        .w_full()
                        .px(px(9.))
                        .py(px(7.))
                        .gap(px(2.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .bg(if is_sel {
                            p.sidebar_row_selected
                        } else {
                            Hsla::transparent_black()
                        })
                        .hover(move |s| {
                            if is_sel {
                                s
                            } else {
                                s.bg(p.sidebar_row_hover)
                            }
                        })
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_harness_entry(&entry_clone, window, cx);
                        }))
                        .child(
                            div()
                                .w_full()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.ink)
                                .child(entry.title.clone()),
                        )
                        .child(
                            h_flex()
                                .w_full()
                                .gap(px(6.))
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.quiet_ink)
                                        .child(harness_kind_label(entry.kind)),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .text_color(p.quiet_ink)
                                        .opacity(0.6)
                                        .child("·"),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .text_size(px(10.5))
                                        .text_color(p.quiet_ink)
                                        .child(entry.path.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.))
                                        .text_color(p.quiet_ink)
                                        .child(format!("v{}", entry.version)),
                                ),
                        ),
                );
            }
        }

        // Right Detail editor
        let has_editor = self.harness_ui.creating_new || self.harness_ui.selected_entry_id.is_some();
        let detail: AnyElement = if !has_editor {
            v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.quiet_ink)
                        .child("Select an entry or create a new one"),
                )
                .into_any_element()
        } else {
            let is_new = self.harness_ui.creating_new;
            let confirming_del = self.harness_ui.pending_delete_id.is_some()
                && self.harness_ui.pending_delete_id == self.harness_ui.selected_entry_id;

            let kind_btn = |id: &'static str, k: HarnessKindWire| {
                let active = self.harness_ui.editor_kind == k;
                div()
                    .id(id)
                    .h(px(23.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .bg(if active {
                        p.sidebar_row_selected
                    } else {
                        p.sidebar_group_surface
                    })
                    .text_color(if active { p.ink } else { p.quiet_ink })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.harness_ui.editor_kind = k;
                        cx.notify();
                    }))
                    .child(harness_kind_label(k))
            };

            v_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .pl(px(12.))
                .gap(px(9.))
                .when(is_new, |col| {
                    col.child(
                        h_flex()
                            .gap(px(4.))
                            .items_center()
                            .child(kind_btn("hek-mem", HarnessKindWire::Memory))
                            .child(kind_btn("hek-prm", HarnessKindWire::Prompt))
                            .child(kind_btn("hek-skl", HarnessKindWire::Skill))
                            .child(kind_btn("hek-sub", HarnessKindWire::Subagent)),
                    )
                })
                .child(
                    h_flex()
                        .w_full()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(Input::new(&self.harness_title_input).cleanable(false)),
                        )
                        .child(
                            div()
                                .w(px(130.))
                                .child(Input::new(&self.harness_path_input).cleanable(false)),
                        ),
                )
                .child(
                    div()
                        .w_full()
                        .flex_1()
                        .min_h(px(160.))
                        .child(Textarea::new(&self.harness_content_input).h_full()),
                )
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_between()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(
                                    self.harness_ui
                                        .notice
                                        .as_ref()
                                        .map_or(p.quiet_ink, |(ok, _)| {
                                            if *ok { p.quiet_ink } else { p.danger }
                                        }),
                                )
                                .child(
                                    self.harness_ui
                                        .notice
                                        .as_ref()
                                        .map(|(_, m)| m.clone())
                                        .unwrap_or_default(),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .items_center()
                                .when(!is_new, |row| {
                                    if confirming_del {
                                        row.child(
                                            div()
                                                .id("harness-del-confirm")
                                                .h(px(28.))
                                                .px(px(10.))
                                                .rounded(px(6.))
                                                .bg(p.danger_surface)
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.danger)
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.confirm_delete_harness_entry(cx);
                                                }))
                                                .child("Confirm delete"),
                                        )
                                        .child(
                                            div()
                                                .id("harness-del-cancel")
                                                .h(px(28.))
                                                .px(px(8.))
                                                .rounded(px(6.))
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(12.))
                                                .text_color(p.quiet_ink)
                                                .hover(move |s| s.bg(p.sidebar_row_hover))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.harness_ui.pending_delete_id = None;
                                                    cx.notify();
                                                }))
                                                .child("Cancel"),
                                        )
                                    } else {
                                        row.child(
                                            div()
                                                .id("harness-del-btn")
                                                .h(px(28.))
                                                .px(px(10.))
                                                .rounded(px(6.))
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(12.))
                                                .text_color(p.danger)
                                                .hover(move |s| s.bg(p.danger_surface))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.harness_ui.pending_delete_id =
                                                        this.harness_ui.selected_entry_id.clone();
                                                    cx.notify();
                                                }))
                                                .child("Delete"),
                                        )
                                    }
                                })
                                .child(
                                    div()
                                        .id("harness-save-btn")
                                        .h(px(28.))
                                        .px(px(14.))
                                        .rounded(px(6.))
                                        .bg(p.sidebar_row_selected)
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .text_size(px(12.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.ink)
                                        .hover(move |s| s.bg(p.sidebar_row_hover))
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.save_harness_editor(cx);
                                        }))
                                        .child("Save"),
                                ),
                        ),
                )
                .into_any_element()
        };

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap(px(10.))
            .child(filter_row)
            .child(
                h_flex()
                    .w_full()
                    .flex_1()
                    .min_h_0()
                    .child(master)
                    .child(detail),
            )
            .into_any_element()
    }

    fn render_harness_history_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let mut list = v_flex()
            .id("harness-history-scroll")
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .gap(px(8.));

        if self.harness_ui.history.is_empty() {
            list = list.child(
                div()
                    .p(px(12.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No refinement history yet"),
            );
        } else {
            for (idx, item) in self.harness_ui.history.iter().enumerate() {
                let ref_id = item.id.clone();
                let confirming = self.harness_ui.pending_rollback_id.as_deref()
                    == Some(item.id.as_str());
                let scope_badges = item
                    .scopes
                    .iter()
                    .map(|s| harness_scope_label(*s))
                    .collect::<Vec<_>>()
                    .join(" · ");
                let changes_preview = if item.changes.is_empty() {
                    item.outcome.clone()
                } else {
                    item.changes.join(" · ")
                };

                list = list.child(
                    v_flex()
                        .id(("h-hist-row", idx))
                        .w_full()
                        .p(px(11.))
                        .gap(px(5.))
                        .rounded(px(9.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .bg(p.sidebar_group_surface)
                        .child(
                            h_flex()
                                .w_full()
                                .items_center()
                                .justify_between()
                                .gap(px(8.))
                                .child(
                                    h_flex()
                                        .gap(px(7.))
                                        .items_center()
                                        .child(
                                            div()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(p.sidebar_row_selected)
                                                .text_size(px(10.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.ink)
                                                .child(scope_badges),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(12.))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.ink)
                                                .child(item.trigger.clone()),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.))
                                                .text_color(p.quiet_ink)
                                                .child(item.created_at.clone()),
                                        ),
                                )
                                .when(item.can_rollback, |row| {
                                    let ref_id_click = ref_id.clone();
                                    let ref_id_confirm = ref_id.clone();
                                    if confirming {
                                        row.child(
                                            h_flex()
                                                .gap(px(6.))
                                                .child(
                                                    div()
                                                        .id(("h-rb-go", idx))
                                                        .h(px(24.))
                                                        .px(px(9.))
                                                        .rounded(px(5.))
                                                        .bg(p.danger_surface)
                                                        .flex()
                                                        .items_center()
                                                        .cursor_pointer()
                                                        .text_size(px(11.5))
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(p.danger)
                                                        .on_click(cx.listener(move |this, _, _, cx| {
                                                            this.harness_ui.pending_rollback_id = None;
                                                            let ws_id = this.effective_harness_workspace_id();
                                                            let sid = this.effective_harness_session_id();
                                                            let _ = this.bridge.tx.send(
                                                                DaemonCmd::HarnessRollback {
                                                                    refinement_id: ref_id_confirm.clone(),
                                                                    workspace_id: ws_id,
                                                                    active_session_id: sid,
                                                                },
                                                            );
                                                            cx.notify();
                                                        }))
                                                        .child("Confirm rollback"),
                                                )
                                                .child(
                                                    div()
                                                        .id(("h-rb-cancel", idx))
                                                        .h(px(24.))
                                                        .px(px(8.))
                                                        .rounded(px(5.))
                                                        .flex()
                                                        .items_center()
                                                        .cursor_pointer()
                                                        .text_size(px(11.5))
                                                        .text_color(p.quiet_ink)
                                                        .hover(move |s| s.bg(p.sidebar_row_hover))
                                                        .on_click(cx.listener(|this, _, _, cx| {
                                                            this.harness_ui.pending_rollback_id = None;
                                                            cx.notify();
                                                        }))
                                                        .child("Cancel"),
                                                ),
                                        )
                                    } else {
                                        row.child(
                                            div()
                                                .id(("h-rb-btn", idx))
                                                .h(px(24.))
                                                .px(px(9.))
                                                .rounded(px(5.))
                                                .bg(p.sidebar_row_selected)
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.ink)
                                                .hover(move |s| s.bg(p.sidebar_row_hover))
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.harness_ui.pending_rollback_id =
                                                        Some(ref_id_click.clone());
                                                    cx.notify();
                                                }))
                                                .child("Rollback"),
                                        )
                                    }
                                }),
                        )
                        .when(!changes_preview.is_empty(), |card| {
                            card.child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(p.quiet_ink)
                                    .child(changes_preview),
                            )
                        }),
                );
            }
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap(px(8.))
            .when_some(self.harness_ui.notice.as_ref(), |col, (ok, msg)| {
                col.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(if *ok { p.quiet_ink } else { p.danger })
                        .child(msg.clone()),
                )
            })
            .child(list)
            .into_any_element()
    }

    /// Settings -> Workspaces tab: list registered Workspaces with Missing/Relink/Delete.
    pub(crate) fn render_settings_workspaces_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let header = h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .child("Registered workspaces"),
            )
            .child(
                h_flex()
                    .id("settings-ws-add")
                    .h(px(28.))
                    .px(px(10.))
                    .gap(px(5.))
                    .rounded(px(6.))
                    .bg(p.sidebar_row_selected)
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink)
                    .hover(move |s| s.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.pick_and_register_workspace(false, cx);
                    }))
                    .child(Icon::new(IconName::Plus).size(px(12.)).text_color(p.ink))
                    .child("Add workspace"),
            );

        let mut rows = v_flex()
            .id("settings-ws-scroll")
            .w_full()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .gap(px(8.));

        if self.workspace_ui.workspaces.is_empty() {
            rows = rows.child(
                div()
                    .p(px(12.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No workspaces yet"),
            );
        } else {
            for (idx, ws) in self.workspace_ui.workspaces.iter().enumerate() {
                let ws_id = ws.id.clone();
                let ws_id_relink = ws_id.clone();
                let ws_id_del = ws_id.clone();
                let ws_id_confirm = ws_id.clone();
                let confirming = self.workspace_ui.pending_delete_ws.as_deref()
                    == Some(ws.id.as_str());
                let kind_tag = match ws.kind {
                    WorkspaceKind::Git => "Git",
                    WorkspaceKind::Plain => "Folder",
                };

                rows = rows.child(
                    h_flex()
                        .id(("settings-ws-row", idx))
                        .w_full()
                        .p(px(11.))
                        .gap(px(10.))
                        .rounded(px(9.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .bg(p.sidebar_group_surface)
                        .items_center()
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap(px(2.))
                                .child(
                                    h_flex()
                                        .gap(px(6.))
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(12.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.ink)
                                                .child(ws.name.clone()),
                                        )
                                        .child(
                                            div()
                                                .px(px(5.))
                                                .py(px(1.))
                                                .rounded(px(4.))
                                                .bg(p.sidebar_row_selected)
                                                .text_size(px(10.))
                                                .text_color(p.quiet_ink)
                                                .child(kind_tag),
                                        )
                                        .when(ws.missing, |r| {
                                            r.child(
                                                div()
                                                    .px(px(5.))
                                                    .py(px(1.))
                                                    .rounded(px(4.))
                                                    .bg(p.danger_surface)
                                                    .text_size(px(10.))
                                                    .text_color(p.danger)
                                                    .child("Missing"),
                                            )
                                        }),
                                )
                                .child(
                                    div()
                                        .w_full()
                                        .truncate()
                                        .text_size(px(11.))
                                        .text_color(p.quiet_ink)
                                        .child(ws.root_path.clone()),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap(px(6.))
                                .items_center()
                                .child(
                                    div()
                                        .id(("settings-ws-relink", idx))
                                        .h(px(26.))
                                        .px(px(9.))
                                        .rounded(px(6.))
                                        .bg(p.sidebar_row_selected)
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.ink)
                                        .hover(move |s| s.bg(p.sidebar_row_hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.pick_and_relink_workspace(ws_id_relink.clone(), cx);
                                        }))
                                        .child("Relink"),
                                )
                                .child(if confirming {
                                    h_flex()
                                        .gap(px(4.))
                                        .child(
                                            div()
                                                .id(("settings-ws-del-go", idx))
                                                .h(px(26.))
                                                .px(px(9.))
                                                .rounded(px(6.))
                                                .bg(p.danger_surface)
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(11.5))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(p.danger)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.workspace_ui.pending_delete_ws = None;
                                                    let _ = this.bridge.tx.send(
                                                        DaemonCmd::DeleteWorkspace {
                                                            workspace_id: ws_id_confirm.clone(),
                                                            delete_harness: true,
                                                        },
                                                    );
                                                    cx.notify();
                                                }))
                                                .child("Confirm"),
                                        )
                                        .child(
                                            div()
                                                .id(("settings-ws-del-cancel", idx))
                                                .h(px(26.))
                                                .px(px(7.))
                                                .rounded(px(6.))
                                                .flex()
                                                .items_center()
                                                .cursor_pointer()
                                                .text_size(px(11.5))
                                                .text_color(p.quiet_ink)
                                                .hover(move |s| s.bg(p.sidebar_row_hover))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.workspace_ui.pending_delete_ws = None;
                                                    cx.notify();
                                                }))
                                                .child("Cancel"),
                                        )
                                        .into_any_element()
                                } else {
                                    div()
                                        .id(("settings-ws-del", idx))
                                        .h(px(26.))
                                        .px(px(9.))
                                        .rounded(px(6.))
                                        .flex()
                                        .items_center()
                                        .cursor_pointer()
                                        .text_size(px(11.5))
                                        .text_color(p.danger)
                                        .hover(move |s| s.bg(p.danger_surface))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.workspace_ui.pending_delete_ws =
                                                Some(ws_id_del.clone());
                                            cx.notify();
                                        }))
                                        .child("Delete")
                                        .into_any_element()
                                }),
                        ),
                );
            }
        }

        v_flex()
            .w_full()
            .flex_1()
            .min_h_0()
            .gap(px(10.))
            .child(header)
            .when_some(self.workspace_ui.notice.as_ref(), |col, (ok, msg)| {
                col.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(if *ok { p.quiet_ink } else { p.danger })
                        .child(msg.clone()),
                )
            })
            .child(rows)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entry(id: &str, kind: HarnessKindWire, title: &str, path: &str) -> HarnessEntryWire {
        HarnessEntryWire {
            id: id.into(),
            kind,
            title: title.into(),
            content: format!("content for {title}"),
            path: path.into(),
            scope: HarnessScopeWire::Workspace,
            workspace_id: Some("ws-1".into()),
            session_id: None,
            reference: Default::default(),
            arguments: Default::default(),
            metadata: Default::default(),
            source: "agent".into(),
            created_at: "2026-11-23T00:00:00Z".into(),
            updated_at: "2026-11-23T00:00:00Z".into(),
            version: 1,
        }
    }

    #[::core::prelude::v1::test]
    fn filter_harness_entries_matches_kind_and_query() {
        let entries = vec![
            sample_entry("m1", HarnessKindWire::Memory, "Build Rule", "build"),
            sample_entry("p1", HarnessKindWire::Prompt, "Short Replies", "policy"),
            sample_entry("s1", HarnessKindWire::Skill, "Run Checks", "general"),
        ];
        assert_eq!(filter_harness_entries(&entries, None, "").len(), 3);
        assert_eq!(
            filter_harness_entries(&entries, Some(HarnessKindWire::Prompt), "").len(),
            1
        );
        let q = filter_harness_entries(&entries, None, "build");
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].id, "m1");
    }
}
