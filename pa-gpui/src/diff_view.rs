//! Read-only Git diff right column: header controls, collapsible jump list,
//! background loader orchestration, lazy single-file expansion, and `gpui-kit` `Diff` view.

use crate::app::PaApp;
use crate::diff_git::{
    load_diff_for_scope, load_single_file_diff, main_project_root_for, probe_repo_context,
    DiffFileSummary, DiffLoadOutcome,
};
use crate::diff_model::DiffViewStatus;
use crate::popups::OpenPopup;
use crate::settings::DiffScope;
use crate::sidebar::session_key;
use gpui_kit::component::{
    diff::{
        Diff, DiffChangeIndicator, DiffFileStatus, DiffHoverHighlight, DiffHunkSeparator, DiffMode,
    },
    h_flex,
    scroll::ScrollableElement as _,
    v_flex, Icon, IconName,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

const DIFF_REFRESH_DEBOUNCE_MS: u64 = 350;

fn split_dir_and_name(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((dir, name)) => (dir, name),
        None => ("", path),
    }
}

impl PaApp {
    pub(crate) fn diff_mode_from_settings(&self) -> DiffMode {
        if self.settings.diff_split {
            DiffMode::Split
        } else {
            DiffMode::Unified
        }
    }

    pub(crate) fn active_diff_target(&self) -> (Option<String>, Option<String>) {
        if let Some((peek_path, _)) = self.workspace_ui.diff_peek.as_ref() {
            return (Some(format!("__peek__:{peek_path}")), Some(peek_path.clone()));
        }
        if self.roster.is_draft_or_empty() {
            if let Some(wt) = self.workspace_ui.draft_worktree_path.as_ref() {
                return (Some("__draft__".to_string()), Some(wt.clone()));
            }
            if let Some(ws) = self
                .workspace_ui
                .draft_workspace_id
                .as_deref()
                .and_then(|id| self.workspace_ui.workspace_by_id(id))
            {
                return (Some("__draft__".to_string()), Some(ws.root_path.clone()));
            }
            let cwd = self
                .roster
                .draft_cwd()
                .or_else(|| self.roster.pending_start_cwd())
                .map(str::to_string);
            return (cwd.as_ref().map(|_| "__draft__".to_string()), cwd);
        }
        let Some(row) = self.roster.active_session() else {
            let sid = self.roster.active_id().map(str::to_string);
            let cwd = self.roster.pending_start_cwd().map(str::to_string);
            return (sid, cwd);
        };
        let key = session_key(row);
        let bound_wt = row
            .workspace_binding
            .as_ref()
            .map(|b| b.worktree_path.as_str())
            .filter(|s| !s.trim().is_empty());
        let cwd = bound_wt.unwrap_or_else(|| row.cwd.trim());
        let cwd_opt = (!cwd.is_empty()).then(|| cwd.to_string());
        let effective = self
            .diff_model
            .effective_cwd(Some(key.as_str()), cwd_opt.as_deref());
        (Some(key), effective)
    }

    pub(crate) fn sync_repo_context(&mut self, cx: &mut Context<Self>) {
        let (_, cwd) = self.active_diff_target();
        let Some((generation, cwd_path)) = self.diff_model.begin_context_probe(cwd.as_deref())
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let repo_ctx = cx
                .background_spawn(async move { probe_repo_context(&cwd_path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.diff_model.apply_context_probe(generation, repo_ctx) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[allow(dead_code)]
    pub(crate) fn refresh_recent_project_worktrees(&mut self, cx: &mut Context<Self>) {
        let folders = self.settings.recent_folders.clone();
        if folders.is_empty() {
            return;
        }
        let generation = self.diff_model.begin_recent_worktrees_probe();
        cx.spawn(async move |this, cx| {
            let map = cx
                .background_spawn(async move {
                    let mut out = HashMap::new();
                    for dir in folders {
                        if let Some(ctx) = probe_repo_context(std::path::Path::new(&dir)) {
                            out.insert(dir, ctx.worktrees);
                        }
                    }
                    out
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.diff_model.apply_recent_worktrees(generation, map) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[allow(dead_code)]
    pub(crate) fn clear_draft_folder(&mut self, cx: &mut Context<Self>) {
        self.diff_model.bump_draft_folder_generation();
        self.open_popup = OpenPopup::None;
        self.roster.set_draft_cwd(None);
        self.settings.last_folder = None;
        self.settings.save();
        self.sync_diff_for_active_session(cx);
        cx.notify();
    }

    pub(crate) fn select_draft_folder(
        &mut self,
        folder: String,
        project_root: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let trimmed = folder.trim().to_string();
        if trimmed.is_empty() {
            return;
        }
        let folder_gen = self.diff_model.bump_draft_folder_generation();
        self.open_popup = OpenPopup::None;
        self.roster.set_draft_cwd(Some(trimmed.clone()));
        if let Some(project) = project_root {
            self.settings.record_recent_project(&project, &trimmed);
            self.settings.save();
        } else {
            let active_for_bg = trimmed.clone();
            self.settings.last_folder = Some(trimmed.clone());
            self.settings.save();
            cx.spawn(async move |this, cx| {
                let project = cx
                    .background_spawn(async move {
                        main_project_root_for(std::path::Path::new(&active_for_bg))
                            .to_string_lossy()
                            .into_owned()
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if this.diff_model.is_current_draft_folder_generation(folder_gen) {
                        this.settings.record_recent_project(&project, &trimmed);
                        this.settings.save();
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        self.sync_diff_for_active_session(cx);
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn pick_draft_folder(&mut self, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        cx.notify();
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose session folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(dir) = paths.into_iter().next()
            {
                let dir_str = dir.to_string_lossy().to_string();
                let _ = this.update(cx, |this, cx| {
                    if this.roster.is_draft_or_empty() {
                        this.select_draft_folder(dir_str, None, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub(crate) fn sync_diff_for_active_session(&mut self, cx: &mut Context<Self>) {
        let (key, cwd) = self.active_diff_target();
        if !self.settings.right_open {
            self.sync_repo_context(cx);
            return;
        }
        let scope = self.settings.diff_scope;
        let requested_base = self
            .diff_model
            .requested_base_for_session(key.as_deref())
            .map(str::to_string);
        if self.diff_model.matches_target(
            key.as_deref(),
            cwd.as_deref(),
            scope,
            requested_base.as_deref(),
        ) {
            return;
        }
        if matches!(
            self.open_popup,
            OpenPopup::DiffBaseBranchMenu
        ) {
            self.open_popup = OpenPopup::None;
        }
        self.refresh_diff(cx);
    }

    pub(crate) fn schedule_debounced_diff_refresh(&mut self, cx: &mut Context<Self>) {
        let Some(seq) = self
            .diff_model
            .begin_debounced_refresh(self.settings.right_open)
        else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(DIFF_REFRESH_DEBOUNCE_MS))
                .await;
            let _ = this.update(cx, |app, cx| {
                if app
                    .diff_model
                    .should_fire_debounced_refresh(seq, app.settings.right_open)
                {
                    app.refresh_diff(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn on_window_focused(&mut self, cx: &mut Context<Self>) {
        if !self.roster.is_draft_or_empty() {
            if let Some(sid) = self.roster.active_id().map(str::to_string) {
                let _ = self
                    .bridge
                    .tx
                    .send(crate::daemon::DaemonCmd::RefreshSessionWorkspace { session: sid });
            }
        }
        if self.settings.right_open {
            self.refresh_diff(cx);
        }
    }

    pub(crate) fn refresh_diff(&mut self, cx: &mut Context<Self>) {
        if !self.settings.right_open {
            return;
        }
        let (key, cwd) = self.active_diff_target();
        let Some(cwd_str) = cwd.clone() else {
            self.diff_model.mark_not_a_git_repo(key, None);
            self.diff_state.update(cx, |state, cx| {
                state.set_files(Vec::new(), cx);
            });
            cx.notify();
            return;
        };

        let scope = self.settings.diff_scope;
        let requested_base = self
            .diff_model
            .requested_base_for_session(key.as_deref())
            .map(str::to_string);
        let (generation, clear_existing) =
            self.diff_model
                .begin_load(key, cwd, scope, requested_base.clone());
        if clear_existing {
            self.diff_state.update(cx, |state, cx| {
                state.set_files(Vec::new(), cx);
            });
        }
        cx.notify();

        let cwd_path = PathBuf::from(cwd_str);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_spawn(async move {
                    load_diff_for_scope(&cwd_path, scope, requested_base.as_deref())
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                let Some(expanded_paths) = app.diff_model.apply_load_outcome(generation, &outcome)
                else {
                    return;
                };
                match outcome {
                    DiffLoadOutcome::Loaded(loaded) => {
                        app.diff_state.update(cx, |state, cx| {
                            state.set_files(loaded.files, cx);
                            state.set_all_files_collapsed(true, cx);
                            if !expanded_paths.is_empty() {
                                state.set_files_collapsed(
                                    expanded_paths.iter().cloned().map(SharedString::from),
                                    false,
                                    cx,
                                );
                            }
                        });
                        for path in expanded_paths {
                            app.maybe_load_deferred_file(&path, false, cx);
                        }
                    }
                    DiffLoadOutcome::NotAGitRepo | DiffLoadOutcome::Error(_) => {
                        app.diff_state.update(cx, |state, cx| {
                            state.set_files(Vec::new(), cx);
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn on_diff_file_expanded(&mut self, path: &str, cx: &mut Context<Self>) {
        self.diff_model.set_file_collapsed(path, false);
        self.maybe_load_deferred_file(path, true, cx);
        cx.notify();
    }

    pub(crate) fn toggle_diff_file_collapsed(&mut self, path: &str, cx: &mut Context<Self>) {
        let next_collapsed = !self.diff_state.read(cx).is_file_collapsed(path);
        self.diff_state.update(cx, |state, cx| {
            state.set_file_collapsed(path, next_collapsed, cx);
        });
        if next_collapsed {
            self.diff_model.set_file_collapsed(path, true);
            cx.notify();
        } else {
            self.on_diff_file_expanded(path, cx);
        }
    }

    pub(crate) fn maybe_load_deferred_file(
        &mut self,
        path: &str,
        scroll_after: bool,
        cx: &mut Context<Self>,
    ) {
        let Some((generation, repo_root, base_ref, old_path, untracked)) =
            self.diff_model.begin_deferred_file_load(path)
        else {
            return;
        };
        let owned_path = path.to_string();
        cx.notify();
        cx.spawn(async move |this, cx| {
            let bg_path = owned_path.clone();
            let loaded_file = cx
                .background_spawn(async move {
                    load_single_file_diff(
                        &repo_root,
                        &base_ref,
                        &bg_path,
                        old_path.as_deref(),
                        untracked,
                    )
                })
                .await;
            let _ = this.update(cx, |app, cx| {
                let Some(ix) = app.diff_model.finish_deferred_file_load(
                    generation,
                    &owned_path,
                    loaded_file.is_some(),
                ) else {
                    cx.notify();
                    return;
                };
                if let Some(df) = loaded_file {
                    let expanded_paths = app.diff_model.expanded_paths_for_current_files();
                    let target_path = owned_path.clone();
                    app.diff_state.update(cx, |state, cx| {
                        let mut files = state.files().to_vec();
                        if ix < files.len() {
                            files[ix] = df;
                            state.set_files(files, cx);
                            state.set_all_files_collapsed(true, cx);
                            if !expanded_paths.is_empty() {
                                state.set_files_collapsed(
                                    expanded_paths.into_iter().map(SharedString::from),
                                    false,
                                    cx,
                                );
                            }
                            if scroll_after {
                                state.scroll_to_file(&target_path, cx);
                            }
                        }
                    });
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn set_diff_scope(&mut self, scope: DiffScope, cx: &mut Context<Self>) {
        if self.settings.diff_scope == scope {
            return;
        }
        self.settings.diff_scope = scope;
        self.settings.save();
        if scope == DiffScope::Uncommitted && self.open_popup == OpenPopup::DiffBaseBranchMenu {
            self.open_popup = OpenPopup::None;
        }
        self.refresh_diff(cx);
    }

    pub(crate) fn select_diff_base_branch(&mut self, branch: String, cx: &mut Context<Self>) {
        let (key, _) = self.active_diff_target();
        self.diff_model.set_selected_base(key.as_deref(), branch);
        self.open_popup = OpenPopup::None;
        self.refresh_diff(cx);
    }

    #[allow(dead_code)]
    pub(crate) fn select_diff_worktree(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        if self.roster.is_draft_or_empty() {
            let dir_str = path.to_string_lossy().to_string();
            let main_root = self
                .diff_model
                .worktrees()
                .iter()
                .find(|wt| wt.is_main)
                .map(|wt| wt.path.to_string_lossy().into_owned());
            self.select_draft_folder(dir_str, main_root, cx);
            return;
        }
        let (key, _) = self.active_diff_target();
        self.diff_model.set_selected_worktree(key.as_deref(), path);
        self.sync_diff_for_active_session(cx);
        cx.notify();
    }

    pub(crate) fn toggle_diff_split(&mut self, cx: &mut Context<Self>) {
        self.settings.diff_split = !self.settings.diff_split;
        self.settings.save();
        let mode = self.diff_mode_from_settings();
        self.diff_state.update(cx, |state, cx| {
            state.set_mode(mode, cx);
        });
        cx.notify();
    }

    pub(crate) fn toggle_diff_wrap(&mut self, cx: &mut Context<Self>) {
        self.settings.diff_wrap = !self.settings.diff_wrap;
        self.settings.save();
        cx.notify();
    }

    pub(crate) fn toggle_diff_jump_collapsed(&mut self, cx: &mut Context<Self>) {
        self.settings.diff_jump_collapsed = !self.settings.diff_jump_collapsed;
        self.settings.save();
        cx.notify();
    }

    pub(crate) fn toggle_diff_expand_all(&mut self, cx: &mut Context<Self>) {
        let collapse = self.diff_model.all_expanded();
        self.diff_model.set_all_collapsed(collapse);
        self.diff_state.update(cx, |state, cx| {
            state.set_all_files_collapsed(collapse, cx);
        });
        if !collapse {
            for path in self.diff_model.deferred_paths() {
                self.maybe_load_deferred_file(&path, false, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn jump_to_diff_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.diff_model.expand_for_jump(path);
        let owned = path.to_string();
        self.diff_state.update(cx, |state, cx| {
            state.set_file_collapsed(&owned, false, cx);
            state.scroll_to_file(&owned, cx);
        });
        self.maybe_load_deferred_file(path, true, cx);
        cx.notify();
    }

    pub(crate) fn render_right_column(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.settings.right_open {
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
                        .child(self.render_diff_header(cx))
                        .children(self.render_diff_peek_banner(cx))
                        .children(self.render_diff_jump_list(cx))
                        .child(self.render_diff_body(window, cx)),
                )
                .into_any_element(),
        )
    }

    fn render_diff_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let summaries = self.diff_model.summaries();
        let file_count = summaries.len();
        let additions = self.diff_model.total_additions();
        let deletions = self.diff_model.total_deletions();
        let all_expanded = self.diff_model.all_expanded();
        let has_files = file_count > 0;
        let is_split = self.settings.diff_split;
        let is_wrap = self.settings.diff_wrap;
        let jump_collapsed = self.settings.diff_jump_collapsed;
        let scope = self.settings.diff_scope;
        let is_branch = scope == DiffScope::Branch;
        let base_branch = self.diff_model.base_branch().map(str::to_string);
        let on_base = self.diff_model.on_base_branch();
        let branch_menu_open = self.open_popup == OpenPopup::DiffBaseBranchMenu;
        let in_git_repo = !matches!(self.diff_model.status(), DiffViewStatus::NotAGitRepo);

        let seg_btn = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(8.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .line_height(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .text_color(if active { p.ink } else { p.quiet_ink })
                .bg(if active {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .hover(move |s| {
                    if active {
                        s
                    } else {
                        s.bg(p.sidebar_row_hover).text_color(p.ink)
                    }
                })
                .child(label)
        };

        let toolbar_btn = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(7.))
                .rounded(px(5.))
                .border_1()
                .border_color(if active {
                    p.composer_edge
                } else {
                    Hsla::transparent_black()
                })
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(11.))
                .line_height(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .text_color(if active { p.ink } else { p.quiet_ink })
                .bg(if active {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                .child(label)
        };

        let files_label = if file_count == 1 {
            "1 file".to_string()
        } else {
            format!("{file_count} files")
        };

        v_flex()
            .w_full()
            .flex_shrink_0()
            .border_b_1()
            .border_color(p.sidebar_group_edge)
            .child(
                h_flex()
                    .w_full()
                    .h(px(38.))
                    .px(px(12.))
                    .gap(px(6.))
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                h_flex()
                                    .flex_shrink_0()
                                    .h(px(28.))
                                    .p(px(2.))
                                    .gap(px(1.))
                                    .items_center()
                                    .rounded(px(7.))
                                    .bg(p.sidebar_group_surface)
                                    .border_1()
                                    .border_color(p.sidebar_group_edge)
                                    .child(
                                        seg_btn("diff-scope-uncommitted", "Uncommitted", !is_branch)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.set_diff_scope(DiffScope::Uncommitted, cx);
                                            })),
                                    )
                                    .child(
                                        seg_btn("diff-scope-branch", "Branch", is_branch)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.set_diff_scope(DiffScope::Branch, cx);
                                            })),
                                    ),
                            )
                            .when_some(
                                (in_git_repo && is_branch)
                                    .then_some(base_branch.clone())
                                    .flatten(),
                                |row, base| {
                                    let base_pill_label = format!("vs {base}");
                                    let on_base_text = format!("On {base}");
                                    row.child(
                                        h_flex()
                                            .id("diff-base-branch-btn")
                                            .h(px(28.))
                                            .px(px(8.))
                                            .gap(px(4.))
                                            .min_w_0()
                                            .rounded(px(6.))
                                            .border_1()
                                            .border_color(if branch_menu_open {
                                                p.working
                                            } else {
                                                p.sidebar_group_edge
                                            })
                                            .bg(if branch_menu_open {
                                                p.sidebar_row_selected
                                            } else {
                                                p.sidebar_group_surface
                                            })
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(p.sidebar_row_hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.open_popup = if this.open_popup
                                                    == OpenPopup::DiffBaseBranchMenu
                                                {
                                                    OpenPopup::None
                                                } else {
                                                    OpenPopup::DiffBaseBranchMenu
                                                };
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(11.))
                                                    .line_height(px(11.))
                                                    .text_color(p.ink)
                                                    .child(base_pill_label),
                                            )
                                            .child(
                                                Icon::new(IconName::ChevronDown)
                                                    .size(px(11.))
                                                    .text_color(p.quiet_ink),
                                            ),
                                    )
                                    .when(on_base, |r| {
                                        r.child(
                                            div()
                                                .min_w_0()
                                                .truncate()
                                                .px(px(6.))
                                                .py(px(2.))
                                                .rounded(px(4.))
                                                .bg(p.sidebar_group_surface)
                                                .text_size(px(10.5))
                                                .text_color(p.quiet_ink)
                                                .child(on_base_text),
                                        )
                                    })
                                },
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(px(5.))
                            .items_center()
                            .child(
                                div()
                                    .id("diff-refresh-btn")
                                    .size(px(24.))
                                    .rounded(px(6.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(if self.diff_model.is_loading() {
                                        p.working
                                    } else {
                                        p.quiet_ink
                                    })
                                    .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                                    .child(Icon::new(IconName::RefreshCw).size(px(12.5)))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.refresh_diff(cx);
                                    })),
                            ),
                    ),
            )
            .when(has_files, |col| {
                col.child(
                    h_flex()
                        .w_full()
                        .h(px(32.))
                        .px(px(12.))
                        .pb(px(4.))
                        .gap(px(8.))
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .id("diff-jump-toggle")
                                .h(px(24.))
                                .px(px(6.))
                                .ml(px(-6.))
                                .gap(px(5.))
                                .rounded(px(5.))
                                .items_center()
                                .cursor_pointer()
                                .hover(move |s| s.bg(p.sidebar_row_hover))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_diff_jump_collapsed(cx);
                                }))
                                .child(
                                    Icon::new(if jump_collapsed {
                                        IconName::ChevronRight
                                    } else {
                                        IconName::ChevronDown
                                    })
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                                )
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.ink)
                                        .child(files_label.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(4.))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .when(additions > 0, |s| {
                                            s.child(
                                                div()
                                                    .text_color(p.tag_idle)
                                                    .child(format!("+{additions}")),
                                            )
                                        })
                                        .when(deletions > 0, |s| {
                                            s.child(
                                                div()
                                                    .text_color(p.danger)
                                                    .child(format!("-{deletions}")),
                                            )
                                        }),
                                ),
                        )
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .gap(px(4.))
                                .items_center()
                                .child(
                                    toolbar_btn(
                                        "diff-expand-toggle",
                                        if all_expanded { "Collapse" } else { "Expand" },
                                        false,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_diff_expand_all(cx);
                                    })),
                                )
                                .child(
                                    h_flex()
                                        .p(px(1.5))
                                        .gap(px(1.))
                                        .rounded(px(6.))
                                        .bg(p.sidebar_group_surface)
                                        .border_1()
                                        .border_color(p.sidebar_group_edge)
                                        .child(
                                            seg_btn("diff-mode-unified", "Unified", !is_split)
                                                .h(px(19.))
                                                .px(px(6.))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    if this.settings.diff_split {
                                                        this.toggle_diff_split(cx);
                                                    }
                                                })),
                                        )
                                        .child(
                                            seg_btn("diff-mode-split", "Split", is_split)
                                                .h(px(19.))
                                                .px(px(6.))
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    cx.stop_propagation();
                                                    if !this.settings.diff_split {
                                                        this.toggle_diff_split(cx);
                                                    }
                                                })),
                                        ),
                                )
                                .child(
                                    toolbar_btn("diff-wrap-toggle", "Wrap", is_wrap).on_click(
                                        cx.listener(|this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_diff_wrap(cx);
                                        }),
                                    ),
                                ),
                        ),
                )
            })
    }

    fn render_diff_jump_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let summaries = self.diff_model.summaries();
        if summaries.is_empty() || self.settings.diff_jump_collapsed {
            return None;
        }
        let p = self.palette();

        let mut rows = v_flex()
            .id("diff-jump-rows")
            .w_full()
            .max_h(px(168.))
            .overflow_y_scrollbar()
            .px(px(8.))
            .py(px(6.))
            .gap(px(2.));
        for (ix, summary) in summaries.iter().enumerate() {
            rows = rows.child(self.render_jump_row(ix, summary, cx));
        }

        Some(
            v_flex()
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(p.sidebar_group_edge)
                .bg(p.sidebar_group_surface)
                .child(rows)
                .into_any_element(),
        )
    }

    fn render_jump_row(
        &self,
        ix: usize,
        summary: &DiffFileSummary,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let (dir, name) = split_dir_and_name(&summary.path);
        let dir_label = if dir.is_empty() {
            String::new()
        } else {
            dir.to_string()
        };
        let path_click = summary.path.clone();
        let additions = summary.additions;
        let deletions = summary.deletions;
        let binary = summary.binary;
        let untracked = summary.untracked;
        let truncated = summary.truncated;
        let deferred = summary.deferred;
        let loading = self.diff_model.is_file_loading(&summary.path);
        let dot_color = if untracked || summary.added || (additions > 0 && deletions == 0 && summary.old_path.is_none()) {
            p.tag_idle
        } else if additions == 0 && deletions > 0 {
            p.danger
        } else if summary.old_path.is_some() {
            p.quiet_ink
        } else {
            p.working
        };

        h_flex()
            .id(("diff-jump-row", ix))
            .w_full()
            .h(px(26.))
            .px(px(8.))
            .gap(px(7.))
            .rounded(px(6.))
            .items_center()
            .justify_between()
            .cursor_pointer()
            .hover(move |s| s.bg(p.sidebar_row_hover))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.jump_to_diff_file(&path_click, cx);
            }))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(dot_color),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .child(name.to_string()),
                    )
                    .when(!dir_label.is_empty(), |row| {
                        row.child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.))
                                .text_color(p.quiet_ink)
                                .child(dir_label),
                        )
                    }),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .gap(px(5.))
                    .items_center()
                    .text_size(px(11.))
                    .when(loading, |row| {
                        row.child(div().text_color(p.working).child("…"))
                    })
                    .when(deferred && !loading, |row| {
                        row.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(p.sidebar_row_hover)
                                .text_size(px(10.))
                                .text_color(p.quiet_ink)
                                .child("lazy"),
                        )
                    })
                    .when(truncated, |row| {
                        row.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(p.sidebar_row_hover)
                                .text_size(px(10.))
                                .text_color(p.quiet_ink)
                                .child("256K"),
                        )
                    })
                    .when(binary, |row| {
                        row.child(
                            div()
                                .px(px(5.))
                                .py(px(1.))
                                .rounded(px(4.))
                                .bg(p.sidebar_row_hover)
                                .text_size(px(10.))
                                .text_color(p.quiet_ink)
                                .child("bin"),
                        )
                    })
                    .when(!binary, |row| {
                        row.when(additions > 0, |r| {
                            r.child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.tag_idle)
                                    .child(format!("+{additions}")),
                            )
                        })
                        .when(deletions > 0, |r| {
                            r.child(
                                div()
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.danger)
                                    .child(format!("-{deletions}")),
                            )
                        })
                    }),
            )
    }

    fn render_diff_empty_state(
        &self,
        icon: IconName,
        icon_color: Hsla,
        title: String,
        subtitle: Option<String>,
        action: Option<AnyElement>,
    ) -> AnyElement {
        let p = self.palette();
        v_flex()
            .flex_1()
            .w_full()
            .items_center()
            .justify_center()
            .gap(px(10.))
            .px(px(28.))
            .child(Icon::new(icon).size(px(20.)).text_color(icon_color))
            .child(
                v_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .child(title),
                    )
                    .when_some(subtitle, |col, hint| {
                        col.child(
                            div()
                                .max_w(px(240.))
                                .text_center()
                                .text_size(px(11.5))
                                .text_color(p.quiet_ink)
                                .child(hint),
                        )
                    }),
            )
            .children(action)
            .into_any_element()
    }

    fn render_diff_body(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let header_visible_w = (self.right_column_width(window) - 52.).max(180.);
        match self.diff_model.status() {
            DiffViewStatus::Ready => {
                #[derive(Clone, Copy)]
                struct HeaderMeta {
                    additions: usize,
                    deletions: usize,
                    added: bool,
                    untracked: bool,
                    deferred: bool,
                    truncated: bool,
                    binary: bool,
                    loading: bool,
                }

                let file_meta: HashMap<String, HeaderMeta> = self
                    .diff_model
                    .summaries()
                    .iter()
                    .map(|s| {
                        (
                            s.path.clone(),
                            HeaderMeta {
                                additions: s.additions,
                                deletions: s.deletions,
                                added: s.added,
                                untracked: s.untracked,
                                deferred: s.deferred,
                                truncated: s.truncated,
                                binary: s.binary,
                                loading: self.diff_model.is_file_loading(&s.path),
                            },
                        )
                    })
                    .collect();

                let app_entity = cx.entity().clone();
                let ink = p.ink;
                let quiet_ink = p.quiet_ink;
                let tag_idle = p.tag_idle;
                let danger = p.danger;
                let working = p.working;
                let row_hover = p.sidebar_row_hover;
                let row_selected = p.sidebar_row_selected;

                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        Diff::new(&self.diff_state)
                            .border_0()
                            .bg(p.window_surface)
                            .soft_wrap(self.settings.diff_wrap)
                            .hover_highlight(DiffHoverHighlight::Line)
                            .change_indicator(DiffChangeIndicator::Bars)
                            .hunk_separator(DiffHunkSeparator::LineInfo)
                            .header(move |file, _, _| {
                                let path = file.path().clone();
                                let meta = file_meta.get(path.as_ref()).copied();
                                let (dir, name) = split_dir_and_name(path.as_ref());
                                let name_owned = name.to_string();
                                let subtitle = match (file.original_path(), file.modified_path()) {
                                    (Some(old), Some(new)) if old != new => {
                                        format!("from {old}")
                                    }
                                    _ => dir.to_string(),
                                };

                                let (badge, badge_ink, badge_bg) = if meta.is_some_and(|m| m.untracked) {
                                    ("U", tag_idle, tag_idle.opacity(0.14))
                                } else if meta.is_some_and(|m| m.added) {
                                    ("A", tag_idle, tag_idle.opacity(0.14))
                                } else {
                                    match file.status() {
                                        DiffFileStatus::Added => {
                                            ("A", tag_idle, tag_idle.opacity(0.14))
                                        }
                                        DiffFileStatus::Deleted => {
                                            ("D", danger, danger.opacity(0.14))
                                        }
                                        DiffFileStatus::Renamed | DiffFileStatus::Copied => {
                                            ("R", quiet_ink, row_selected)
                                        }
                                        DiffFileStatus::Conflicted => {
                                            ("!", working, working.opacity(0.16))
                                        }
                                        _ => ("M", working, working.opacity(0.14)),
                                    }
                                };

                                let add = meta.map_or(file.additions(), |m| m.additions);
                                let del = meta.map_or(file.deletions(), |m| m.deletions);
                                let binary = meta.map_or(file.is_binary(), |m| m.binary);
                                let deferred = meta.is_some_and(|m| m.deferred);
                                let truncated = meta.is_some_and(|m| m.truncated);
                                let loading = meta.is_some_and(|m| m.loading);
                                let app_click = app_entity.clone();
                                let path_click = path.clone();

                                h_flex()
                                    .w(px(header_visible_w))
                                    .min_w_0()
                                    .px(px(4.))
                                    .py(px(3.))
                                    .gap(px(7.))
                                    .rounded(px(5.))
                                    .items_center()
                                    .justify_between()
                                    .cursor_pointer()
                                    .hover(move |s| s.bg(row_hover))
                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                        cx.stop_propagation();
                                        app_click.update(cx, |this, cx| {
                                            this.toggle_diff_file_collapsed(path_click.as_ref(), cx);
                                        });
                                    })
                                    .child(
                                        h_flex()
                                            .flex_1()
                                            .min_w_0()
                                            .gap(px(6.))
                                            .items_center()
                                            .child(
                                                div()
                                                    .size(px(16.))
                                                    .rounded(px(4.))
                                                    .flex_shrink_0()
                                                    .bg(badge_bg)
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .text_size(px(9.5))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(badge_ink)
                                                    .child(badge),
                                            )
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_size(px(12.))
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(ink)
                                                    .child(name_owned),
                                            )
                                            .when(!subtitle.is_empty(), |row| {
                                                row.child(
                                                    div()
                                                        .min_w_0()
                                                        .truncate()
                                                        .text_size(px(11.))
                                                        .text_color(quiet_ink)
                                                        .child(subtitle),
                                                )
                                            }),
                                    )
                                    .child(
                                        h_flex()
                                            .flex_shrink_0()
                                            .gap(px(5.))
                                            .items_center()
                                            .text_size(px(11.))
                                            .when(truncated, |r| {
                                                r.child(
                                                    div()
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .rounded(px(4.))
                                                        .bg(row_hover)
                                                        .text_size(px(10.))
                                                        .text_color(quiet_ink)
                                                        .child("256K preview"),
                                                )
                                            })
                                            .when(deferred, |r| {
                                                r.child(
                                                    div()
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .rounded(px(4.))
                                                        .bg(row_hover)
                                                        .text_size(px(10.))
                                                        .text_color(if loading {
                                                            working
                                                        } else {
                                                            quiet_ink
                                                        })
                                                        .child(if loading {
                                                            "Loading…"
                                                        } else {
                                                            "Lazy"
                                                        }),
                                                )
                                            })
                                            .when(binary, |r| {
                                                r.child(
                                                    div()
                                                        .px(px(5.))
                                                        .py(px(1.))
                                                        .rounded(px(4.))
                                                        .bg(row_hover)
                                                        .text_size(px(10.))
                                                        .text_color(quiet_ink)
                                                        .child("Binary"),
                                                )
                                            })
                                            .when(!binary && add > 0, |r| {
                                                r.child(
                                                    div()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(tag_idle)
                                                        .child(format!("+{add}")),
                                                )
                                            })
                                            .when(!binary && del > 0, |r| {
                                                r.child(
                                                    div()
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(danger)
                                                        .child(format!("-{del}")),
                                                )
                                            }),
                                    )
                            })
                            .size_full(),
                    )
                    .into_any_element()
            }
            DiffViewStatus::NotAGitRepo => {
                let (title, hint) =
                    if self.roster.is_draft_or_empty() && self.roster.draft_cwd().is_none() {
                        (
                            "Choose a folder".to_string(),
                            "Pick a git folder in the title bar above to preview diffs".to_string(),
                        )
                    } else {
                        (
                            "Not a git repository".to_string(),
                            "Start a new session in a git folder to inspect diffs".to_string(),
                        )
                    };
                self.render_diff_empty_state(
                    IconName::Folder,
                    p.quiet_ink,
                    title,
                    Some(hint),
                    None,
                )
            }
            DiffViewStatus::Empty => {
                let (title, hint) = match (
                    self.settings.diff_scope,
                    self.diff_model.base_branch(),
                    self.diff_model.on_base_branch(),
                ) {
                    (DiffScope::Branch, Some(base), true) => (
                        format!("On {base}"),
                        "No uncommitted changes in this working tree".to_string(),
                    ),
                    (DiffScope::Branch, Some(base), false) => (
                        format!("No changes against {base}"),
                        "Your branch matches the base commit".to_string(),
                    ),
                    _ => (
                        "Working tree clean".to_string(),
                        "Edits made in this session will appear here".to_string(),
                    ),
                };
                self.render_diff_empty_state(
                    IconName::CircleCheck,
                    p.tag_idle,
                    title,
                    Some(hint),
                    None,
                )
            }
            DiffViewStatus::Idle | DiffViewStatus::Loading => self.render_diff_empty_state(
                IconName::RefreshCw,
                p.working,
                "Loading changes…".to_string(),
                None,
                None,
            ),
            DiffViewStatus::Error(msg) => {
                let retry = div()
                    .id("diff-retry-btn")
                    .mt(px(2.))
                    .px(px(12.))
                    .py(px(5.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.sidebar_group_edge)
                    .bg(p.sidebar_group_surface)
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.ink)
                    .cursor_pointer()
                    .hover(move |s| s.bg(p.sidebar_row_hover))
                    .child("Retry")
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.refresh_diff(cx);
                    }))
                    .into_any_element();
                self.render_diff_empty_state(
                    IconName::TriangleAlert,
                    p.danger,
                    "Could not load diff".to_string(),
                    Some(msg.clone()),
                    Some(retry),
                )
            }
        }
    }
}
