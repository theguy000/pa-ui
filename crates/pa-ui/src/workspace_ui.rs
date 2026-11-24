//! Workspace, Worktree & Branch state and Title bar / dropdown UI (Spec #140 Slice 1).
//!
//! Renders the borderless split breadcrumb (`Workspace · Worktree / Branch`, or
//! `Scratch`), the Left Workspace & Worktree dropdown with inline Diff peek and
//! `+ New worktree` / `+ Add workspace`, the Right Branch dropdown with
//! `+ New branch`, and the inline Diff peek banner (`Switch here` / `Return`).

use crate::app::PaApp;
use crate::assets::ICON_GIT_BRANCH_PATH;
use crate::daemon::DaemonCmd;
use crate::popups::OpenPopup;
use gpui_kit::component::{
    h_flex,
    input::Input,
    scroll::ScrollableElement as _,
    v_flex, Icon, IconName,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{BranchInfo, WorkspaceBinding, WorkspaceKind, WorkspaceRecord, WorktreeInfo};
use std::collections::{HashMap, HashSet};

/// Pure presentation model for the Title bar split breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TitlebarBreadcrumb {
    /// Left trigger primary label (`Workspace` or `Workspace · worktree`, or `Scratch`).
    pub left_label: String,
    /// True when no Workspace is bound (Scratch session).
    pub is_scratch: bool,
    /// Optional Right trigger branch label (shown only for Git Workspaces).
    pub branch: Option<String>,
    /// Active worktree path (used for Branch queries / diff target).
    pub worktree_path: Option<String>,
    /// Active workspace id when bound to a registered Workspace.
    pub workspace_id: Option<String>,
}

/// Format the Title bar split breadcrumb from a session binding or draft selection.
pub(crate) fn compute_titlebar_breadcrumb(
    binding: Option<&WorkspaceBinding>,
    draft_ws: Option<&WorkspaceRecord>,
    draft_wt: Option<&WorktreeInfo>,
    draft_wt_path: Option<&str>,
) -> TitlebarBreadcrumb {
    if let Some(b) = binding {
        let is_linked = b.workspace_kind == WorkspaceKind::Git
            && !paths_same(&b.worktree_path, &b.workspace_root)
            && !b.worktree_name.is_empty()
            && b.worktree_name != b.workspace_name;
        let left_label = if is_linked {
            format!("{} · {}", b.workspace_name, b.worktree_name)
        } else {
            b.workspace_name.clone()
        };
        return TitlebarBreadcrumb {
            left_label,
            is_scratch: false,
            branch: (b.workspace_kind == WorkspaceKind::Git).then(|| {
                b.branch
                    .clone()
                    .unwrap_or_else(|| "detached".to_string())
            }),
            worktree_path: Some(b.worktree_path.clone()),
            workspace_id: Some(b.workspace_id.clone()),
        };
    }

    if let Some(ws) = draft_ws {
        let (left_label, branch, wt_path) = if let Some(wt) = draft_wt {
            let label = if !wt.is_main && !wt.name.is_empty() && wt.name != ws.name {
                format!("{} · {}", ws.name, wt.name)
            } else {
                ws.name.clone()
            };
            let br = (ws.kind == WorkspaceKind::Git)
                .then(|| wt.branch.clone().unwrap_or_else(|| "detached".to_string()));
            (label, br, wt.path.clone())
        } else {
            let wt_p = draft_wt_path.unwrap_or(&ws.root_path).to_string();
            let br = (ws.kind == WorkspaceKind::Git).then(|| "HEAD".to_string());
            (ws.name.clone(), br, wt_p)
        };
        return TitlebarBreadcrumb {
            left_label,
            is_scratch: false,
            branch,
            worktree_path: Some(wt_path),
            workspace_id: Some(ws.id.clone()),
        };
    }

    TitlebarBreadcrumb {
        left_label: "Scratch".to_string(),
        is_scratch: true,
        branch: None,
        worktree_path: None,
        workspace_id: None,
    }
}

pub(crate) fn paths_same(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/")
            .to_lowercase()
    };
    norm(a) == norm(b)
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WorkspaceUiState {
    pub workspaces: Vec<WorkspaceRecord>,
    pub worktree_root: String,
    pub draft_workspace_id: Option<String>,
    pub draft_worktree_path: Option<String>,
    pub worktrees_by_ws: HashMap<String, Vec<WorktreeInfo>>,
    pub expanded_workspaces: HashSet<String>,
    pub branches_for_wt: Option<(String, Vec<BranchInfo>)>,
    /// Active inline Diff peek: `(worktree_path, display_label)`.
    pub diff_peek: Option<(String, String)>,
    /// Workspace id currently showing inline `+ New worktree` input in the Left dropdown.
    pub creating_worktree_ws: Option<String>,
    /// True when the Right dropdown is showing the inline `+ New branch` input.
    pub creating_branch: bool,
    /// Pending confirmation workspace id in Settings -> Workspaces Delete action.
    pub pending_delete_ws: Option<String>,
    /// Transient feedback banner in Settings -> Workspaces or General Worktree Root.
    pub notice: Option<(bool, String)>,
}

impl WorkspaceUiState {
    pub(crate) fn new(worktree_root: String) -> Self {
        Self {
            worktree_root,
            ..Self::default()
        }
    }

    pub(crate) fn workspace_by_id(&self, id: &str) -> Option<&WorkspaceRecord> {
        self.workspaces.iter().find(|w| w.id == id)
    }

    pub(crate) fn worktree_for(&self, ws_id: &str, path: &str) -> Option<&WorktreeInfo> {
        self.worktrees_by_ws
            .get(ws_id)
            .and_then(|list| list.iter().find(|wt| paths_same(&wt.path, path)))
    }
}

impl PaApp {
    /// Resolve the active session's `WorkspaceBinding` (or draft selection when in draft).
    pub(crate) fn current_workspace_binding(&self) -> Option<&WorkspaceBinding> {
        if self.roster.is_draft_or_empty() {
            return None;
        }
        self.roster
            .active_session()
            .and_then(|row| row.workspace_binding.as_ref())
    }

    /// Resolve the Title bar split breadcrumb for the active session or draft canvas.
    pub(crate) fn current_titlebar_breadcrumb(&self) -> TitlebarBreadcrumb {
        if !self.roster.is_draft_or_empty() {
            let binding = self.current_workspace_binding();
            if let Some(b) = binding {
                return compute_titlebar_breadcrumb(Some(b), None, None, None);
            }
            return TitlebarBreadcrumb {
                left_label: "Scratch".to_string(),
                is_scratch: true,
                branch: None,
                worktree_path: None,
                workspace_id: None,
            };
        }

        let draft_ws = self
            .workspace_ui
            .draft_workspace_id
            .as_deref()
            .and_then(|id| self.workspace_ui.workspace_by_id(id));
        let draft_wt = draft_ws.and_then(|ws| {
            let wt_path = self
                .workspace_ui
                .draft_worktree_path
                .as_deref()
                .unwrap_or(&ws.root_path);
            self.workspace_ui.worktree_for(&ws.id, wt_path)
        });
        compute_titlebar_breadcrumb(
            None,
            draft_ws,
            draft_wt,
            self.workspace_ui.draft_worktree_path.as_deref(),
        )
    }

    /// Select a Workspace + Worktree (or Scratch when `workspace_id` is `None`) for the
    /// active draft or live session.
    pub(crate) fn apply_workspace_selection(
        &mut self,
        workspace_id: Option<String>,
        worktree_path: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.workspace_ui.diff_peek = None;
        self.workspace_ui.creating_worktree_ws = None;
        self.open_popup = OpenPopup::None;

        if self.roster.is_draft_or_empty() {
            self.workspace_ui.draft_workspace_id = workspace_id.clone();
            self.workspace_ui.draft_worktree_path = worktree_path.clone();
            let effective_cwd = worktree_path.clone().or_else(|| {
                workspace_id
                    .as_deref()
                    .and_then(|id| self.workspace_ui.workspace_by_id(id))
                    .map(|w| w.root_path.clone())
            });
            self.roster.set_draft_cwd(effective_cwd);
            if let Some(ref ws_id) = workspace_id {
                let _ = self.bridge.tx.send(DaemonCmd::LoadWorktrees {
                    workspace_id: ws_id.clone(),
                });
            }
            self.sync_diff_for_active_session(cx);
            cx.notify();
            return;
        }

        if let Some(sid) = self.roster.active_id().map(str::to_string) {
            let current_ws = self
                .current_workspace_binding()
                .map(|b| b.workspace_id.clone());
            if workspace_id.is_some() && workspace_id == current_ws {
                if let Some(wt) = worktree_path {
                    let _ = self.bridge.tx.send(DaemonCmd::SwitchSessionWorktree {
                        session: sid,
                        worktree_path: wt,
                    });
                    cx.notify();
                    return;
                }
            }
            let _ = self.bridge.tx.send(DaemonCmd::BindSessionWorkspace {
                session: sid,
                workspace_id,
                worktree_path,
            });
        }
        cx.notify();
    }

    /// Peek a worktree in the right Diff column without mutating the session's bound worktree.
    pub(crate) fn start_diff_peek(
        &mut self,
        worktree_path: String,
        label: String,
        cx: &mut Context<Self>,
    ) {
        self.workspace_ui.diff_peek = Some((worktree_path, label));
        self.open_popup = OpenPopup::None;
        if !self.settings.right_open {
            self.settings.right_open = true;
            self.settings.save();
        }
        self.sync_diff_for_active_session(cx);
        cx.notify();
    }

    /// Stop peeking and return the Diff column to the session's bound worktree.
    pub(crate) fn clear_diff_peek(&mut self, cx: &mut Context<Self>) {
        if self.workspace_ui.diff_peek.take().is_some() {
            self.sync_diff_for_active_session(cx);
            cx.notify();
        }
    }

    /// Switch the active session/draft to the currently peeked worktree.
    pub(crate) fn switch_to_peeked_worktree(&mut self, cx: &mut Context<Self>) {
        let Some((wt_path, _)) = self.workspace_ui.diff_peek.take() else {
            return;
        };
        let ws_id = self
            .current_titlebar_breadcrumb()
            .workspace_id
            .or_else(|| {
                self.workspace_ui
                    .workspaces
                    .iter()
                    .find(|w| {
                        paths_same(&w.root_path, &wt_path)
                            || self
                                .workspace_ui
                                .worktrees_by_ws
                                .get(&w.id)
                                .is_some_and(|list| {
                                    list.iter().any(|wt| paths_same(&wt.path, &wt_path))
                                })
                    })
                    .map(|w| w.id.clone())
            });
        self.apply_workspace_selection(ws_id, Some(wt_path), cx);
    }

    /// Open native folder picker to register a new Workspace (optionally binding the active session).
    pub(crate) fn pick_and_register_workspace(&mut self, bind_after: bool, cx: &mut Context<Self>) {
        if bind_after {
            self.open_popup = OpenPopup::None;
            cx.notify();
        }
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select Workspace folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                let dir_str = dir.to_string_lossy().to_string();
                let _ = this.update(cx, |this, cx| {
                    this.pending_bind_registered_workspace = bind_after;
                    let _ = this.bridge.tx.send(DaemonCmd::RegisterWorkspace {
                        path: dir_str,
                        name: None,
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Open native folder picker to relink a missing Workspace.
    pub(crate) fn pick_and_relink_workspace(&mut self, workspace_id: String, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select relocated Workspace folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                let new_path = dir.to_string_lossy().to_string();
                let _ = this.update(cx, |this, cx| {
                    let _ = this.bridge.tx.send(DaemonCmd::RelinkWorkspace {
                        workspace_id,
                        new_path,
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Open native folder picker to change the Worktree Root in `~/.prime/agent/settings.json`.
    pub(crate) fn pick_worktree_root(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Select Worktree Root folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                let dir_str = dir.to_string_lossy().to_string();
                let _ = this.update(cx, |this, cx| {
                    match crate::settings::save_agent_worktree_root(Some(&dir_str)) {
                        Ok(effective) => {
                            this.workspace_ui.worktree_root = effective;
                            this.workspace_ui.notice = Some((true, "Worktree root updated".into()));
                            let _ = this.bridge.tx.send(DaemonCmd::RefreshWorkspaces);
                        }
                        Err(err) => {
                            this.workspace_ui.notice = Some((false, err.to_string()));
                        }
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Submit inline `+ New worktree` form inside the Left dropdown.
    pub(crate) fn submit_new_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ws_id) = self.workspace_ui.creating_worktree_ws.clone() else {
            return;
        };
        let name = self.ws_new_name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            return;
        }
        self.ws_new_name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.workspace_ui.creating_worktree_ws = None;
        self.open_popup = OpenPopup::None;
        let _ = self.bridge.tx.send(DaemonCmd::CreateWorktree {
            workspace_id: ws_id,
            name,
            base_branch: None,
        });
        cx.notify();
    }

    /// Submit inline `+ New branch` form inside the Right dropdown.
    pub(crate) fn submit_new_branch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.workspace_ui.creating_branch {
            return;
        }
        let branch = self.branch_new_name_input.read(cx).value().trim().to_string();
        if branch.is_empty() {
            return;
        }
        let Some(wt_path) = self.current_titlebar_breadcrumb().worktree_path else {
            return;
        };
        let active_sid = (!self.roster.is_draft_or_empty())
            .then(|| self.roster.active_id().map(str::to_string))
            .flatten();
        self.branch_new_name_input
            .update(cx, |s, cx| s.set_value("", window, cx));
        self.workspace_ui.creating_branch = false;
        self.open_popup = OpenPopup::None;
        let _ = self.bridge.tx.send(DaemonCmd::CreateBranch {
            worktree_path: wt_path,
            branch,
            start_point: None,
            active_session_id: active_sid,
        });
        cx.notify();
    }

    /// Variant A v2 borderless split breadcrumb in the Title bar:
    /// `[Workspace · Worktree v]` `/` `[Branch v]` (or `[Scratch v]`).
    pub(crate) fn render_titlebar_workspace_context(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let bc = self.current_titlebar_breadcrumb();
        let left_open = self.open_popup == OpenPopup::FolderMenu;
        let right_open = self.open_popup == OpenPopup::BranchMenu;
        let active_ws_id = bc.workspace_id.clone();
        let active_wt_path = bc.worktree_path.clone();

        let left_trigger = h_flex()
            .id("tb-workspace-btn")
            .h(px(22.))
            .px(px(6.))
            .gap(px(5.))
            .max_w(px(240.))
            .rounded(px(5.))
            .bg(if left_open {
                p.sidebar_row_selected
            } else {
                Hsla::transparent_black()
            })
            .items_center()
            .cursor_pointer()
            .hover(move |s| s.bg(p.sidebar_row_hover))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                if this.open_popup == OpenPopup::FolderMenu {
                    this.open_popup = OpenPopup::None;
                    this.workspace_ui.creating_worktree_ws = None;
                } else {
                    this.open_popup = OpenPopup::FolderMenu;
                    this.workspace_ui.creating_worktree_ws = None;
                    let _ = this.bridge.tx.send(DaemonCmd::RefreshWorkspaces);
                    if let Some(ref ws_id) = active_ws_id {
                        this.workspace_ui.expanded_workspaces.insert(ws_id.clone());
                        let _ = this.bridge.tx.send(DaemonCmd::LoadWorktrees {
                            workspace_id: ws_id.clone(),
                        });
                    }
                }
                cx.notify();
            }))
            .child(
                Icon::new(if bc.is_scratch {
                    IconName::Folder
                } else {
                    IconName::FolderOpen
                })
                .size(px(12.))
                .text_color(p.quiet_ink),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if bc.is_scratch { p.quiet_ink } else { p.ink })
                    .child(bc.left_label),
            )
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(px(10.5))
                    .text_color(p.quiet_ink),
            );

        let right_trigger: Option<AnyElement> = bc.branch.map(|branch_name| {
            let wt_for_click = active_wt_path.clone();
            h_flex()
                .id("tb-branch-btn")
                .h(px(22.))
                .px(px(6.))
                .gap(px(5.))
                .max_w(px(200.))
                .rounded(px(5.))
                .bg(if right_open {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .items_center()
                .cursor_pointer()
                .hover(move |s| s.bg(p.sidebar_row_hover))
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    if this.open_popup == OpenPopup::BranchMenu {
                        this.open_popup = OpenPopup::None;
                        this.workspace_ui.creating_branch = false;
                    } else {
                        this.open_popup = OpenPopup::BranchMenu;
                        this.workspace_ui.creating_branch = false;
                        if let Some(ref wt) = wt_for_click {
                            let _ = this.bridge.tx.send(DaemonCmd::LoadBranches {
                                worktree_path: wt.clone(),
                            });
                        }
                    }
                    cx.notify();
                }))
                .child(
                    svg()
                        .path(ICON_GIT_BRANCH_PATH)
                        .size(px(11.5))
                        .text_color(p.quiet_ink),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .child(branch_name),
                )
                .child(
                    Icon::new(IconName::ChevronDown)
                        .size(px(10.5))
                        .text_color(p.quiet_ink),
                )
                .into_any_element()
        });

        h_flex()
            .id("tb-workspace-breadcrumb")
            .items_center()
            .gap(px(2.))
            .child(left_trigger)
            .when_some(right_trigger, |row, br_el| {
                row.child(
                    div()
                        .px(px(2.))
                        .text_size(px(12.))
                        .text_color(p.quiet_ink)
                        .opacity(0.55)
                        .child("/"),
                )
                .child(br_el)
            })
            .into_any_element()
    }

    /// Left dropdown: `Scratch`, registered Workspaces with collapsible Worktrees,
    /// inline `Diff` peek button, inline `+ New worktree`, and `+ Add workspace`.
    pub(crate) fn render_workspace_menu_popup(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let bc = self.current_titlebar_breadcrumb();
        let left_offset = if self.settings.sidebar_open { 246. } else { 46. };

        let mut list_body = v_flex()
            .id("ws-menu-scroll")
            .w_full()
            .max_h(px(360.))
            .overflow_y_scrollbar()
            .p(px(5.))
            .gap(px(2.));

        // 1. Scratch row
        let scratch_active = bc.is_scratch;
        list_body = list_body.child(
            h_flex()
                .id("ws-opt-scratch")
                .w_full()
                .h(px(28.))
                .px(px(8.))
                .gap(px(8.))
                .rounded(px(6.))
                .items_center()
                .cursor_pointer()
                .bg(if scratch_active {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .hover(move |s| s.bg(p.sidebar_row_hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    cx.stop_propagation();
                    this.apply_workspace_selection(None, None, cx);
                }))
                .child(
                    Icon::new(IconName::Folder)
                        .size(px(12.5))
                        .text_color(p.quiet_ink),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .child("Scratch"),
                )
                .when(scratch_active, |r| {
                    r.child(Icon::new(IconName::Check).size(px(12.)).text_color(p.working))
                }),
        );

        if !self.workspace_ui.workspaces.is_empty() {
            list_body = list_body.child(
                div()
                    .w_full()
                    .h(px(1.))
                    .my(px(4.))
                    .bg(p.sidebar_group_edge),
            );
        }

        // 2. Registered Workspaces
        for (idx, ws) in self.workspace_ui.workspaces.iter().enumerate() {
            let ws_id = ws.id.clone();
            let ws_root = ws.root_path.clone();
            let is_git = ws.kind == WorkspaceKind::Git;
            let expanded = self.workspace_ui.expanded_workspaces.contains(&ws.id);
            let is_active_ws = bc.workspace_id.as_deref() == Some(ws.id.as_str());
            let is_active_main = is_active_ws
                && bc
                    .worktree_path
                    .as_deref()
                    .is_none_or(|wt| paths_same(wt, &ws.root_path));
            let ws_missing = ws.missing;

            let ws_id_exp = ws_id.clone();
            let ws_id_sel = ws_id.clone();
            let ws_root_sel = ws_root.clone();

            list_body = list_body.child(
                h_flex()
                    .id(("ws-row", idx))
                    .w_full()
                    .h(px(28.))
                    .px(px(6.))
                    .gap(px(5.))
                    .rounded(px(6.))
                    .items_center()
                    .cursor_pointer()
                    .bg(if is_active_main {
                        p.sidebar_row_selected
                    } else {
                        Hsla::transparent_black()
                    })
                    .hover(move |s| s.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !ws_missing {
                            this.apply_workspace_selection(
                                Some(ws_id_sel.clone()),
                                Some(ws_root_sel.clone()),
                                cx,
                            );
                        }
                    }))
                    .child(if is_git {
                        div()
                            .id(("ws-exp", idx))
                            .size(px(18.))
                            .rounded(px(4.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(move |s| s.bg(p.sidebar_row_selected))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                if this.workspace_ui.expanded_workspaces.contains(&ws_id_exp) {
                                    this.workspace_ui.expanded_workspaces.remove(&ws_id_exp);
                                } else {
                                    this.workspace_ui
                                        .expanded_workspaces
                                        .insert(ws_id_exp.clone());
                                    let _ = this.bridge.tx.send(DaemonCmd::LoadWorktrees {
                                        workspace_id: ws_id_exp.clone(),
                                    });
                                }
                                cx.notify();
                            }))
                            .child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(11.))
                                .text_color(p.quiet_ink),
                            )
                            .into_any_element()
                    } else {
                        div().w(px(18.)).into_any_element()
                    })
                    .child(
                        Icon::new(IconName::FolderOpen)
                            .size(px(12.5))
                            .text_color(p.quiet_ink),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if ws_missing { p.quiet_ink } else { p.ink })
                            .child(ws.name.clone()),
                    )
                    .when(ws_missing, |r| {
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
                    })
                    .when(is_active_main && !ws_missing, |r| {
                        r.child(Icon::new(IconName::Check).size(px(12.)).text_color(p.working))
                    }),
            );

            // Expanded Git worktrees
            if is_git && expanded && !ws_missing {
                let worktrees = self
                    .workspace_ui
                    .worktrees_by_ws
                    .get(&ws.id)
                    .cloned()
                    .unwrap_or_default();

                for (wt_idx, wt) in worktrees.iter().enumerate() {
                    let wt_path = wt.path.clone();
                    let wt_label = if wt.is_main {
                        format!("{} (main)", wt.name)
                    } else {
                        wt.name.clone()
                    };
                    let wt_branch = wt.branch.clone().unwrap_or_default();
                    let is_active_wt = is_active_ws
                        && bc
                            .worktree_path
                            .as_deref()
                            .is_some_and(|cur| paths_same(cur, &wt.path));
                    let is_peeking_wt = self
                        .workspace_ui
                        .diff_peek
                        .as_ref()
                        .is_some_and(|(pp, _)| paths_same(pp, &wt.path));

                    let ws_id_wt = ws.id.clone();
                    let wt_path_sel = wt_path.clone();
                    let wt_path_peek = wt_path.clone();
                    let peek_title = format!("{} · {}", ws.name, wt.name);

                    list_body = list_body.child(
                        h_flex()
                            .id(("ws-wt-row", idx * 100 + wt_idx))
                            .w_full()
                            .h(px(26.))
                            .pl(px(26.))
                            .pr(px(6.))
                            .gap(px(6.))
                            .rounded(px(5.))
                            .items_center()
                            .cursor_pointer()
                            .bg(if is_active_wt {
                                p.sidebar_row_selected
                            } else {
                                Hsla::transparent_black()
                            })
                            .hover(move |s| s.bg(p.sidebar_row_hover))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.apply_workspace_selection(
                                    Some(ws_id_wt.clone()),
                                    Some(wt_path_sel.clone()),
                                    cx,
                                );
                            }))
                            .child(
                                svg()
                                    .path(ICON_GIT_BRANCH_PATH)
                                    .size(px(11.))
                                    .text_color(p.quiet_ink),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(p.ink)
                                    .child(wt_label),
                            )
                            .when(!wt_branch.is_empty(), |r| {
                                r.child(
                                    div()
                                        .max_w(px(88.))
                                        .truncate()
                                        .text_size(px(10.5))
                                        .text_color(p.quiet_ink)
                                        .child(wt_branch),
                                )
                            })
                            .when(!is_active_wt, |r| {
                                r.child(
                                    div()
                                        .id(("ws-wt-diff", idx * 100 + wt_idx))
                                        .h(px(19.))
                                        .px(px(6.))
                                        .rounded(px(4.))
                                        .flex()
                                        .items_center()
                                        .text_size(px(10.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .bg(if is_peeking_wt {
                                            p.sidebar_row_selected
                                        } else {
                                            Hsla::transparent_black()
                                        })
                                        .text_color(if is_peeking_wt {
                                            p.ink
                                        } else {
                                            p.quiet_ink
                                        })
                                        .hover(move |s| {
                                            s.bg(p.sidebar_row_selected).text_color(p.ink)
                                        })
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.start_diff_peek(
                                                wt_path_peek.clone(),
                                                peek_title.clone(),
                                                cx,
                                            );
                                        }))
                                        .child("Diff"),
                                )
                            })
                            .when(is_active_wt, |r| {
                                r.child(
                                    Icon::new(IconName::Check)
                                        .size(px(11.5))
                                        .text_color(p.working),
                                )
                            }),
                    );
                }

                // Inline "+ New worktree" row or input
                let creating_here = self.workspace_ui.creating_worktree_ws.as_deref()
                    == Some(ws.id.as_str());
                if creating_here {
                    list_body = list_body.child(
                        h_flex()
                            .id(("ws-new-wt-form", idx))
                            .w_full()
                            .pl(px(26.))
                            .pr(px(6.))
                            .py(px(3.))
                            .gap(px(5.))
                            .items_center()
                            .on_click(|_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&self.ws_new_name_input).cleanable(false)),
                            )
                            .child(
                                div()
                                    .id(("ws-new-wt-go", idx))
                                    .h(px(24.))
                                    .px(px(8.))
                                    .rounded(px(5.))
                                    .bg(p.sidebar_row_selected)
                                    .flex()
                                    .items_center()
                                    .cursor_pointer()
                                    .text_size(px(11.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.ink)
                                    .hover(move |s| s.bg(p.sidebar_row_hover))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.submit_new_worktree(window, cx);
                                    }))
                                    .child("Create"),
                            ),
                    );
                } else {
                    let ws_id_new = ws.id.clone();
                    list_body = list_body.child(
                        h_flex()
                            .id(("ws-new-wt-btn", idx))
                            .w_full()
                            .h(px(24.))
                            .pl(px(26.))
                            .pr(px(8.))
                            .gap(px(6.))
                            .rounded(px(5.))
                            .items_center()
                            .cursor_pointer()
                            .hover(move |s| s.bg(p.sidebar_row_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.workspace_ui.creating_worktree_ws = Some(ws_id_new.clone());
                                this.ws_new_name_input
                                    .update(cx, |s, cx| s.set_value("", window, cx));
                                this.ws_new_name_input
                                    .update(cx, |s, cx| s.focus(window, cx));
                                cx.notify();
                            }))
                            .child(
                                Icon::new(IconName::Plus)
                                    .size(px(11.))
                                    .text_color(p.quiet_ink),
                            )
                            .child(
                                div()
                                    .text_size(px(11.5))
                                    .text_color(p.quiet_ink)
                                    .child("New worktree"),
                            ),
                    );
                }
            }
        }

        // Footer: + Add workspace
        let footer = h_flex()
            .id("ws-add-btn")
            .w_full()
            .h(px(32.))
            .px(px(10.))
            .gap(px(7.))
            .border_t_1()
            .border_color(p.sidebar_group_edge)
            .items_center()
            .cursor_pointer()
            .hover(move |s| s.bg(p.sidebar_row_hover))
            .on_click(cx.listener(|this, _, _, cx| {
                cx.stop_propagation();
                this.pick_and_register_workspace(true, cx);
            }))
            .child(Icon::new(IconName::Plus).size(px(12.)).text_color(p.quiet_ink))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink)
                    .child("Add workspace"),
            );

        div()
            .id("floating-workspace-menu")
            .absolute()
            .top(px(34.))
            .left(px(left_offset))
            .w(px(310.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(v_flex().w_full().child(list_body).child(footer))
            .into_any_element()
    }

    /// Right dropdown: local Git branches for active worktree + inline `+ New branch`.
    pub(crate) fn render_branch_menu_popup(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let bc = self.current_titlebar_breadcrumb();
        let left_offset = if self.settings.sidebar_open { 356. } else { 156. };
        let wt_path = bc.worktree_path.clone().unwrap_or_default();
        let active_sid = (!self.roster.is_draft_or_empty())
            .then(|| self.roster.active_id().map(str::to_string))
            .flatten();

        let branches: Vec<BranchInfo> = self
            .workspace_ui
            .branches_for_wt
            .as_ref()
            .filter(|(p_wt, _)| paths_same(p_wt, &wt_path))
            .map(|(_, list)| list.clone())
            .unwrap_or_default();

        let mut list_body = v_flex()
            .id("branch-menu-scroll")
            .w_full()
            .max_h(px(300.))
            .overflow_y_scrollbar()
            .p(px(5.))
            .gap(px(2.));

        if branches.is_empty() {
            list_body = list_body.child(
                div()
                    .px(px(8.))
                    .py(px(8.))
                    .text_size(px(11.5))
                    .text_color(p.quiet_ink)
                    .child("Loading branches"),
            );
        } else {
            for (idx, br) in branches.iter().enumerate() {
                let is_current = br.is_current
                    || bc.branch.as_deref() == Some(br.name.as_str());
                let br_name = br.name.clone();
                let wt_click = wt_path.clone();
                let sid_click = active_sid.clone();

                list_body = list_body.child(
                    h_flex()
                        .id(("branch-opt", idx))
                        .w_full()
                        .h(px(27.))
                        .px(px(8.))
                        .gap(px(7.))
                        .rounded(px(6.))
                        .items_center()
                        .cursor_pointer()
                        .bg(if is_current {
                            p.sidebar_row_selected
                        } else {
                            Hsla::transparent_black()
                        })
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_popup = OpenPopup::None;
                            if !is_current {
                                let _ = this.bridge.tx.send(DaemonCmd::SwitchBranch {
                                    worktree_path: wt_click.clone(),
                                    branch: br_name.clone(),
                                    active_session_id: sid_click.clone(),
                                });
                            }
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path(ICON_GIT_BRANCH_PATH)
                                .size(px(11.5))
                                .text_color(p.quiet_ink),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.ink)
                                .child(br.name.clone()),
                        )
                        .when(is_current, |r| {
                            r.child(
                                Icon::new(IconName::Check)
                                    .size(px(12.))
                                    .text_color(p.working),
                            )
                        }),
                );
            }
        }

        let footer: AnyElement = if self.workspace_ui.creating_branch {
            h_flex()
                .id("branch-new-form")
                .w_full()
                .p(px(6.))
                .gap(px(5.))
                .border_t_1()
                .border_color(p.sidebar_group_edge)
                .items_center()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.branch_new_name_input).cleanable(false)),
                )
                .child(
                    div()
                        .id("branch-new-go")
                        .h(px(24.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .bg(p.sidebar_row_selected)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(|this, _, window, cx| {
                            cx.stop_propagation();
                            this.submit_new_branch(window, cx);
                        }))
                        .child("Create"),
                )
                .into_any_element()
        } else {
            h_flex()
                .id("branch-new-btn")
                .w_full()
                .h(px(32.))
                .px(px(10.))
                .gap(px(7.))
                .border_t_1()
                .border_color(p.sidebar_group_edge)
                .items_center()
                .cursor_pointer()
                .hover(move |s| s.bg(p.sidebar_row_hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    cx.stop_propagation();
                    this.workspace_ui.creating_branch = true;
                    this.branch_new_name_input
                        .update(cx, |s, cx| s.set_value("", window, cx));
                    this.branch_new_name_input
                        .update(cx, |s, cx| s.focus(window, cx));
                    cx.notify();
                }))
                .child(Icon::new(IconName::Plus).size(px(12.)).text_color(p.quiet_ink))
                .child(
                    div()
                        .text_size(px(12.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .child("New branch"),
                )
                .into_any_element()
        };

        div()
            .id("floating-branch-menu")
            .absolute()
            .top(px(34.))
            .left(px(left_offset))
            .w(px(250.))
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_xl()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(v_flex().w_full().child(list_body).child(footer))
            .into_any_element()
    }

    /// Slim neutral banner inside the right Diff column when peeking another worktree.
    pub(crate) fn render_diff_peek_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (_, peek_label) = self.workspace_ui.diff_peek.as_ref()?;
        let p = self.palette();
        Some(
            h_flex()
                .id("diff-peek-banner")
                .w_full()
                .px(px(10.))
                .py(px(6.))
                .gap(px(8.))
                .border_b_1()
                .border_color(p.sidebar_group_edge)
                .bg(p.sidebar_group_surface)
                .items_center()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.5))
                        .text_color(p.quiet_ink)
                        .child(format!("Peeking {peek_label}")),
                )
                .child(
                    div()
                        .id("diff-peek-switch")
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .bg(p.sidebar_row_selected)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.ink)
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.switch_to_peeked_worktree(cx);
                        }))
                        .child("Switch here"),
                )
                .child(
                    div()
                        .id("diff-peek-return")
                        .h(px(22.))
                        .px(px(8.))
                        .rounded(px(5.))
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .text_size(px(11.))
                        .text_color(p.quiet_ink)
                        .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.clear_diff_peek(cx);
                        }))
                        .child("Return"),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn breadcrumb_formats_scratch_main_linked_and_plain_folder() {
        let scratch = compute_titlebar_breadcrumb(None, None, None, None);
        assert!(scratch.is_scratch);
        assert_eq!(scratch.left_label, "Scratch");
        assert!(scratch.branch.is_none());

        let main_git = WorkspaceBinding {
            workspace_id: "ws-1".into(),
            workspace_name: "prime-agent".into(),
            workspace_kind: WorkspaceKind::Git,
            workspace_root: "C:/repos/prime-agent".into(),
            worktree_path: "C:/repos/prime-agent".into(),
            worktree_name: "prime-agent".into(),
            branch: Some("main".into()),
        };
        let bc_main = compute_titlebar_breadcrumb(Some(&main_git), None, None, None);
        assert!(!bc_main.is_scratch);
        assert_eq!(bc_main.left_label, "prime-agent");
        assert_eq!(bc_main.branch.as_deref(), Some("main"));

        let linked_git = WorkspaceBinding {
            worktree_path: "C:/wt/prime-agent/feat-x".into(),
            worktree_name: "feat-x".into(),
            branch: Some("feat/x".into()),
            ..main_git.clone()
        };
        let bc_linked = compute_titlebar_breadcrumb(Some(&linked_git), None, None, None);
        assert_eq!(bc_linked.left_label, "prime-agent · feat-x");
        assert_eq!(bc_linked.branch.as_deref(), Some("feat/x"));

        let plain = WorkspaceBinding {
            workspace_id: "ws-2".into(),
            workspace_name: "notes".into(),
            workspace_kind: WorkspaceKind::Plain,
            workspace_root: "C:/docs/notes".into(),
            worktree_path: "C:/docs/notes".into(),
            worktree_name: "notes".into(),
            branch: None,
        };
        let bc_plain = compute_titlebar_breadcrumb(Some(&plain), None, None, None);
        assert_eq!(bc_plain.left_label, "notes");
        assert!(bc_plain.branch.is_none());
    }
}
