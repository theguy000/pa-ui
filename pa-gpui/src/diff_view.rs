//! Read-only Git diff right column: header controls, collapsible jump list,
//! background loader orchestration, lazy single-file expansion, and `gpui-kit` `Diff` view.

use crate::app::PaApp;
use crate::diff_git::{
    load_diff_for_scope, load_single_file_diff, DiffFileSummary, DiffLoadOutcome,
};
use crate::diff_model::DiffViewStatus;
use crate::popups::OpenPopup;
use crate::settings::DiffScope;
use crate::sidebar::session_key;
use gpui_kit::component::{
    diff::{Diff, DiffMode},
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
        if self.roster.is_draft() {
            return (None, None);
        }
        let Some(row) = self.roster.active_session() else {
            return (None, None);
        };
        let key = session_key(row);
        let cwd = row.cwd.trim();
        let cwd_opt = (!cwd.is_empty()).then(|| cwd.to_string());
        let effective = self
            .diff_model
            .effective_cwd(Some(key.as_str()), cwd_opt.as_deref());
        (Some(key), effective)
    }

    pub(crate) fn sync_diff_for_active_session(&mut self, cx: &mut Context<Self>) {
        if !self.settings.right_open {
            return;
        }
        let (key, cwd) = self.active_diff_target();
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
            OpenPopup::DiffBaseBranchMenu | OpenPopup::DiffWorktreeMenu
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

    pub(crate) fn select_diff_worktree(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let (key, _) = self.active_diff_target();
        self.diff_model.set_selected_worktree(key.as_deref(), path);
        self.open_popup = OpenPopup::None;
        self.refresh_diff(cx);
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
                        .children(self.render_diff_jump_list(cx))
                        .child(self.render_diff_body(cx)),
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
        let scope = self.settings.diff_scope;
        let is_branch = scope == DiffScope::Branch;
        let base_branch = self.diff_model.base_branch().map(str::to_string);
        let on_base = self.diff_model.on_base_branch();
        let has_multi_wt = self.diff_model.has_multiple_worktrees();
        let active_wt_label = self
            .diff_model
            .active_worktree()
            .map(|wt| wt.branch.as_deref().unwrap_or(&wt.label).to_string());
        let branch_menu_open = self.open_popup == OpenPopup::DiffBaseBranchMenu;
        let wt_menu_open = self.open_popup == OpenPopup::DiffWorktreeMenu;

        let pill = |id: &'static str, label: &'static str, active: bool| {
            div()
                .id(id)
                .h(px(22.))
                .px(px(7.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .text_size(px(11.))
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
                    .h(px(34.))
                    .px(px(10.))
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
                                    .p(px(2.))
                                    .gap(px(1.))
                                    .rounded(px(6.))
                                    .bg(p.sidebar_group_surface)
                                    .border_1()
                                    .border_color(p.sidebar_group_edge)
                                    .child(
                                        pill("diff-scope-uncommitted", "Uncommitted", !is_branch)
                                            .h(px(20.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.set_diff_scope(DiffScope::Uncommitted, cx);
                                            })),
                                    )
                                    .child(
                                        pill("diff-scope-branch", "Branch", is_branch)
                                            .h(px(20.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.set_diff_scope(DiffScope::Branch, cx);
                                            })),
                                    ),
                            )
                            .when_some(
                                is_branch.then_some(base_branch.clone()).flatten(),
                                |row, base| {
                                    let base_pill_label = base.clone();
                                    let on_base_text = format!("On {base}");
                                    row.child(
                                        h_flex()
                                            .id("diff-base-branch-btn")
                                            .h(px(22.))
                                            .px(px(7.))
                                            .gap(px(3.))
                                            .min_w_0()
                                            .rounded(px(5.))
                                            .border_1()
                                            .border_color(if branch_menu_open {
                                                p.working
                                            } else {
                                                p.sidebar_group_edge
                                            })
                                            .bg(if branch_menu_open {
                                                p.sidebar_row_selected
                                            } else {
                                                Hsla::transparent_black()
                                            })
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(p.sidebar_row_hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.open_popup =
                                                    if this.open_popup
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
                                                .text_size(px(11.))
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
                            .gap(px(4.))
                            .items_center()
                            .when_some(
                                has_multi_wt.then_some(active_wt_label).flatten(),
                                |row, wt_label| {
                                    row.child(
                                        h_flex()
                                            .id("diff-worktree-btn")
                                            .h(px(22.))
                                            .px(px(7.))
                                            .gap(px(3.))
                                            .max_w(px(120.))
                                            .rounded(px(5.))
                                            .border_1()
                                            .border_color(if wt_menu_open {
                                                p.working
                                            } else {
                                                p.sidebar_group_edge
                                            })
                                            .bg(if wt_menu_open {
                                                p.sidebar_row_selected
                                            } else {
                                                Hsla::transparent_black()
                                            })
                                            .items_center()
                                            .cursor_pointer()
                                            .hover(move |s| s.bg(p.sidebar_row_hover))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.open_popup =
                                                    if this.open_popup
                                                        == OpenPopup::DiffWorktreeMenu
                                                    {
                                                        OpenPopup::None
                                                    } else {
                                                        OpenPopup::DiffWorktreeMenu
                                                    };
                                                cx.notify();
                                            }))
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .truncate()
                                                    .text_size(px(11.))
                                                    .text_color(p.ink)
                                                    .child(wt_label),
                                            )
                                            .child(
                                                Icon::new(IconName::ChevronDown)
                                                    .size(px(11.))
                                                    .text_color(p.quiet_ink),
                                            ),
                                    )
                                },
                            )
                            .child(
                                div()
                                    .id("diff-refresh-btn")
                                    .size(px(22.))
                                    .rounded(px(5.))
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
            .child(
                h_flex()
                    .w_full()
                    .h(px(30.))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .justify_between()
                    .border_t_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(px(8.))
                            .items_center()
                            .when(has_files, |row| {
                                row.child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.ink)
                                        .child(files_label.clone()),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(5.))
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(
                                            div()
                                                .text_color(p.tag_idle)
                                                .child(format!("+{additions}")),
                                        )
                                        .child(
                                            div()
                                                .text_color(p.danger)
                                                .child(format!("-{deletions}")),
                                        ),
                                )
                            })
                            .when(!has_files, |row| {
                                row.child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(p.quiet_ink)
                                        .child(if is_branch {
                                            "Branch changes"
                                        } else {
                                            "Changes"
                                        }),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap(px(2.))
                            .items_center()
                            .when(has_files, |row| {
                                row.child(
                                    pill(
                                        "diff-expand-toggle",
                                        if all_expanded { "Collapse" } else { "Expand" },
                                        false,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_diff_expand_all(cx);
                                    })),
                                )
                            })
                            .child(
                                pill(
                                    "diff-mode-toggle",
                                    if is_split { "Split" } else { "Unified" },
                                    is_split,
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_diff_split(cx);
                                })),
                            )
                            .child(
                                pill("diff-wrap-toggle", "Wrap", is_wrap).on_click(cx.listener(
                                    |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_diff_wrap(cx);
                                    },
                                )),
                            ),
                    ),
            )
    }

    fn render_diff_jump_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let summaries = self.diff_model.summaries();
        if summaries.is_empty() {
            return None;
        }
        let p = self.palette();
        let collapsed = self.settings.diff_jump_collapsed;

        Some(
            v_flex()
                .w_full()
                .flex_shrink_0()
                .border_b_1()
                .border_color(p.sidebar_group_edge)
                .bg(p.sidebar_group_surface)
                .child(
                    h_flex()
                        .id("diff-jump-toggle")
                        .w_full()
                        .h(px(26.))
                        .px(px(8.))
                        .gap(px(5.))
                        .items_center()
                        .cursor_pointer()
                        .text_color(p.quiet_ink)
                        .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_diff_jump_collapsed(cx);
                        }))
                        .child(
                            Icon::new(if collapsed {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(12.)),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Files"),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.sidebar_group_empty_ink)
                                .child(format!("{}", summaries.len())),
                        ),
                )
                .when(!collapsed, |col| {
                    let mut rows = v_flex()
                        .id("diff-jump-rows")
                        .w_full()
                        .max_h(px(180.))
                        .overflow_y_scrollbar()
                        .px(px(4.))
                        .pb(px(4.))
                        .gap(px(1.));
                    for (ix, summary) in summaries.iter().enumerate() {
                        rows = rows.child(self.render_jump_row(ix, summary, cx));
                    }
                    col.child(rows)
                })
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
            format!("{dir}/")
        };
        let path_click = summary.path.clone();
        let additions = summary.additions;
        let deletions = summary.deletions;
        let binary = summary.binary;
        let untracked = summary.untracked;
        let truncated = summary.truncated;
        let deferred = summary.deferred;
        let loading = self.diff_model.is_file_loading(&summary.path);

        h_flex()
            .id(("diff-jump-row", ix))
            .w_full()
            .h(px(24.))
            .px(px(6.))
            .gap(px(6.))
            .rounded(px(4.))
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
                    .gap(px(4.))
                    .items_center()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.5))
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
                    .gap(px(4.))
                    .items_center()
                    .text_size(px(11.))
                    .when(loading, |row| {
                        row.child(div().text_color(p.working).child("…"))
                    })
                    .when(deferred && !loading, |row| {
                        row.child(div().text_color(p.quiet_ink).child("lazy"))
                    })
                    .when(untracked, |row| {
                        row.child(div().text_color(p.working).child("U"))
                    })
                    .when(truncated, |row| {
                        row.child(div().text_color(p.quiet_ink).child("256K"))
                    })
                    .when(binary, |row| {
                        row.child(div().text_color(p.quiet_ink).child("bin"))
                    })
                    .when(!binary, |row| {
                        row.when(additions > 0, |r| {
                            r.child(div().text_color(p.tag_idle).child(format!("+{additions}")))
                        })
                        .when(deletions > 0, |r| {
                            r.child(div().text_color(p.danger).child(format!("-{deletions}")))
                        })
                    }),
            )
    }

    fn render_diff_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        match self.diff_model.status() {
            DiffViewStatus::Ready => {
                // Deferred files use a zero-hunk placeholder so `gpui-kit` omits default `+ / -`;
                // supply accurate numstat counts and lazy/loading status via `header_suffix`.
                let file_meta: HashMap<String, (usize, usize, bool, bool, bool)> = self
                    .diff_model
                    .summaries()
                    .iter()
                    .filter(|s| s.deferred || s.truncated)
                    .map(|s| {
                        (
                            s.path.clone(),
                            (
                                s.additions,
                                s.deletions,
                                s.deferred,
                                s.truncated,
                                self.diff_model.is_file_loading(&s.path),
                            ),
                        )
                    })
                    .collect();
                let quiet_ink = p.quiet_ink;
                let tag_idle = p.tag_idle;
                let danger = p.danger;
                let working = p.working;

                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(
                        Diff::new(&self.diff_state)
                            .soft_wrap(self.settings.diff_wrap)
                            .header_suffix(move |file, _, _| {
                                let Some(&(add, del, deferred, truncated, loading)) =
                                    file_meta.get(file.path().as_ref())
                                else {
                                    return div().into_any_element();
                                };
                                h_flex()
                                    .gap(px(6.))
                                    .items_center()
                                    .text_size(px(11.))
                                    .when(deferred && add > 0, |r| {
                                        r.child(div().text_color(tag_idle).child(format!("+{add}")))
                                    })
                                    .when(deferred && del > 0, |r| {
                                        r.child(div().text_color(danger).child(format!("-{del}")))
                                    })
                                    .when(truncated, |r| {
                                        r.child(div().text_color(quiet_ink).child("256K preview"))
                                    })
                                    .when(deferred, |r| {
                                        r.child(
                                            div()
                                                .text_color(if loading {
                                                    working
                                                } else {
                                                    quiet_ink
                                                })
                                                .child(if loading { "Loading…" } else { "Lazy" }),
                                        )
                                    })
                                    .into_any_element()
                            })
                            .size_full(),
                    )
                    .into_any_element()
            }
            DiffViewStatus::NotAGitRepo => v_flex()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .px(px(20.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.quiet_ink)
                        .child("Not a git repository"),
                )
                .into_any_element(),
            DiffViewStatus::Empty => {
                let empty_msg = match (
                    self.settings.diff_scope,
                    self.diff_model.base_branch(),
                    self.diff_model.on_base_branch(),
                ) {
                    (DiffScope::Branch, Some(base), true) => {
                        format!("On {base} · no uncommitted changes")
                    }
                    (DiffScope::Branch, Some(base), false) => {
                        format!("No changes against {base}")
                    }
                    _ => "No uncommitted changes".to_string(),
                };
                v_flex()
                    .flex_1()
                    .w_full()
                    .items_center()
                    .justify_center()
                    .px(px(20.))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(p.quiet_ink)
                            .child(empty_msg),
                    )
                    .into_any_element()
            }
            DiffViewStatus::Idle | DiffViewStatus::Loading => v_flex()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .px(px(20.))
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.quiet_ink)
                        .child("Loading changes..."),
                )
                .into_any_element(),
            DiffViewStatus::Error(msg) => v_flex()
                .flex_1()
                .w_full()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .px(px(20.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.danger)
                        .child(msg.clone()),
                )
                .child(
                    div()
                        .id("diff-retry-btn")
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .text_size(px(11.5))
                        .text_color(p.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .child("Retry")
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.refresh_diff(cx);
                        })),
                )
                .into_any_element(),
        }
    }
}
