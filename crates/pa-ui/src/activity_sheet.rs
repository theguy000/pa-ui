use crate::app::PaApp;
use crate::daemon::DaemonCmd;
use crate::right_sidebar::RightTabKind;
use crate::theme::PaPalette;
use gpui_kit::component::{h_flex, input::Input, scroll::ScrollableElement as _, v_flex};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{ChipCount, HeartbeatItem, ShellActivityItem, SubagentItem, SubagentMessagePreview};
use std::collections::HashMap;

pub(crate) const SHELL_TAIL_LINES: u32 = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActivityTab {
    Subagents,
    Heartbeats,
    Shells,
}

impl ActivityTab {
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Subagents => "Subagents",
            Self::Heartbeats => "Heartbeats",
            Self::Shells => "Shells",
        }
    }
}

/// Pure state model for the Style B TUI Composer Sheet attached to the top of
/// the composer card, plus cached Session-scoped activity items and shell tails.
#[derive(Debug, Clone, Default)]
pub(crate) struct ActivitySheetModel {
    session_id: String,
    open_tab: Option<ActivityTab>,
    selected_index: usize,
    drill_id: Option<String>,
    hb_editing: bool,
    hb_delivery_mode: String,
    subagents: Vec<SubagentItem>,
    subagent_messages: HashMap<String, Vec<SubagentMessagePreview>>,
    subagent_loading: Option<String>,
    heartbeats: Vec<HeartbeatItem>,
    shells: Vec<ShellActivityItem>,
    shell_tails: HashMap<String, String>,
    tail_loading: Option<String>,
    notice: Option<(bool, String)>,
}

impl ActivitySheetModel {
    pub(crate) fn new() -> Self {
        Self {
            hb_delivery_mode: "steer".to_string(),
            ..Self::default()
        }
    }

    pub(crate) fn open_tab(&self) -> Option<ActivityTab> {
        self.open_tab
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open_tab.is_some()
    }

    pub(crate) fn drill_id(&self) -> Option<&str> {
        self.drill_id.as_deref()
    }

    pub(crate) fn hb_editing(&self) -> bool {
        self.hb_editing
    }

    pub(crate) fn hb_delivery_mode(&self) -> &str {
        if self.hb_delivery_mode.is_empty() {
            "steer"
        } else {
            &self.hb_delivery_mode
        }
    }

    pub(crate) fn subagents(&self) -> &[SubagentItem] {
        &self.subagents
    }

    pub(crate) fn subagent_by_id(&self, id: &str) -> Option<&SubagentItem> {
        self.subagents.iter().find(|s| {
            s.id == id || (!s.child_id.is_empty() && s.child_id == id) || s.active_session_id == id
        })
    }

    pub(crate) fn subagent_messages(&self, subagent_session_id: &str) -> Option<&[SubagentMessagePreview]> {
        self.subagent_messages.get(subagent_session_id).map(Vec::as_slice)
    }

    pub(crate) fn is_subagent_loading(&self, subagent_session_id: &str) -> bool {
        self.subagent_loading.as_deref() == Some(subagent_session_id)
    }

    pub(crate) fn mark_subagent_loading(&mut self, subagent_session_id: &str) {
        self.subagent_loading = Some(subagent_session_id.to_string());
    }

    pub(crate) fn heartbeats(&self) -> &[HeartbeatItem] {
        &self.heartbeats
    }

    pub(crate) fn shells(&self) -> &[ShellActivityItem] {
        &self.shells
    }

    pub(crate) fn shell_by_id(&self, id: &str) -> Option<&ShellActivityItem> {
        self.shells.iter().find(|s| s.id == id)
    }

    pub(crate) fn shell_tail(&self, id: &str) -> Option<&str> {
        self.shell_tails.get(id).map(String::as_str)
    }

    pub(crate) fn is_tail_loading(&self, id: &str) -> bool {
        self.tail_loading.as_deref() == Some(id)
    }

    pub(crate) fn user_heartbeat(&self) -> Option<&HeartbeatItem> {
        self.heartbeats.iter().find(|h| h.is_user_heartbeat())
    }

    pub(crate) fn set_session(&mut self, session_id: &str) {
        if self.session_id == session_id {
            return;
        }
        self.session_id = session_id.to_string();
        self.open_tab = None;
        self.selected_index = 0;
        self.drill_id = None;
        self.hb_editing = false;
        self.hb_delivery_mode = "steer".to_string();
        self.subagents.clear();
        self.subagent_messages.clear();
        self.subagent_loading = None;
        self.heartbeats.clear();
        self.shells.clear();
        self.shell_tails.clear();
        self.tail_loading = None;
        self.notice = None;
    }

    /// Toggle a chip segment. Clicking the currently open segment closes the
    /// sheet; clicking another switches to it and resets drill-in.
    /// Returns `true` if the sheet is now open.
    pub(crate) fn toggle_tab(&mut self, tab: ActivityTab) -> bool {
        self.hb_editing = false;
        self.notice = None;
        if self.open_tab == Some(tab) {
            self.open_tab = None;
            self.drill_id = None;
            self.selected_index = 0;
            return false;
        }
        self.open_tab = Some(tab);
        self.drill_id = None;
        self.selected_index = 0;
        true
    }

    pub(crate) fn close(&mut self) {
        self.open_tab = None;
        self.drill_id = None;
        self.hb_editing = false;
        self.notice = None;
    }

    pub(crate) fn drill_into(&mut self, index: usize, id: String) {
        self.selected_index = index;
        self.drill_id = Some(id);
        self.hb_editing = false;
        self.notice = None;
    }

    pub(crate) fn back(&mut self) {
        self.drill_id = None;
        self.hb_editing = false;
        self.notice = None;
    }

    /// Step back cleanly on Escape: inline editor -> drill-in -> close sheet.
    /// Returns `true` if Escape was consumed.
    pub(crate) fn handle_escape(&mut self) -> bool {
        if self.open_tab.is_none() {
            return false;
        }
        if self.hb_editing {
            self.hb_editing = false;
            return true;
        }
        if self.drill_id.is_some() {
            self.drill_id = None;
            return true;
        }
        self.close();
        true
    }

    pub(crate) fn begin_hb_edit(&mut self, delivery_mode: &str) {
        self.hb_editing = true;
        self.hb_delivery_mode = if delivery_mode == "follow_up" {
            "follow_up".to_string()
        } else {
            "steer".to_string()
        };
        self.notice = None;
    }

    pub(crate) fn cancel_hb_edit(&mut self) {
        self.hb_editing = false;
    }

    pub(crate) fn toggle_hb_delivery(&mut self) {
        self.hb_delivery_mode = if self.hb_delivery_mode() == "steer" {
            "follow_up".to_string()
        } else {
            "steer".to_string()
        };
    }

    pub(crate) fn mark_tail_loading(&mut self, activity_id: &str) {
        self.tail_loading = Some(activity_id.to_string());
    }

    pub(crate) fn set_notice(&mut self, ok: bool, msg: String) {
        self.notice = Some((ok, msg));
    }

    /// Apply Session-scoped subagents only if `session_id` still matches.
    pub(crate) fn apply_subagents(&mut self, session_id: &str, items: Vec<SubagentItem>) -> bool {
        if self.session_id != session_id {
            return false;
        }
        self.subagents = items;
        if self.open_tab == Some(ActivityTab::Subagents) {
            if let Some(ref did) = self.drill_id {
                if !self.subagents.iter().any(|s| &s.id == did) {
                    self.drill_id = None;
                }
            }
            if !self.subagents.is_empty() {
                self.selected_index = self.selected_index.min(self.subagents.len() - 1);
            } else {
                self.selected_index = 0;
            }
        }
        true
    }

    /// Apply read-only subagent messages only if `owner_session_id` still matches.
    pub(crate) fn apply_subagent_messages(
        &mut self,
        owner_session_id: &str,
        subagent_session_id: String,
        messages: Vec<SubagentMessagePreview>,
    ) -> bool {
        if self.session_id != owner_session_id {
            return false;
        }
        if self.subagent_loading.as_deref() == Some(subagent_session_id.as_str()) {
            self.subagent_loading = None;
        }
        self.subagent_messages.insert(subagent_session_id, messages);
        true
    }

    /// Apply Session-scoped heartbeats only if `session_id` still matches the
    /// active Session. Returns `true` if accepted.
    pub(crate) fn apply_heartbeats(&mut self, session_id: &str, items: Vec<HeartbeatItem>) -> bool {
        if self.session_id != session_id {
            return false;
        }
        self.heartbeats = items;
        if self.open_tab == Some(ActivityTab::Heartbeats) {
            if let Some(ref did) = self.drill_id {
                if !self.heartbeats.iter().any(|h| &h.id == did) {
                    self.drill_id = None;
                    self.hb_editing = false;
                }
            }
            if !self.heartbeats.is_empty() {
                self.selected_index = self.selected_index.min(self.heartbeats.len() - 1);
            } else {
                self.selected_index = 0;
            }
        }
        true
    }

    /// Apply Session-scoped shells only if `session_id` still matches.
    pub(crate) fn apply_shells(&mut self, session_id: &str, items: Vec<ShellActivityItem>) -> bool {
        if self.session_id != session_id {
            return false;
        }
        self.shells = items;
        if self.open_tab == Some(ActivityTab::Shells) {
            if let Some(ref did) = self.drill_id {
                if !self.shells.iter().any(|s| &s.id == did) {
                    self.drill_id = None;
                }
            }
            if !self.shells.is_empty() {
                self.selected_index = self.selected_index.min(self.shells.len() - 1);
            } else {
                self.selected_index = 0;
            }
        }
        true
    }

    /// Apply a shell tail response only if `session_id` still matches.
    pub(crate) fn apply_shell_tail(
        &mut self,
        session_id: &str,
        activity_id: String,
        tail: String,
    ) -> bool {
        if self.session_id != session_id {
            return false;
        }
        if self.tail_loading.as_deref() == Some(activity_id.as_str()) {
            self.tail_loading = None;
        }
        self.shell_tails.insert(activity_id, tail);
        true
    }
}

pub(crate) fn format_duration_ms(ms: Option<u64>) -> String {
    let Some(ms) = ms else {
        return "—".to_string();
    };
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        let mins = secs / 60;
        let rem = secs % 60;
        format!("{mins}m {rem}s")
    }
}

fn short_time_label(iso: Option<&str>) -> String {
    let Some(raw) = iso.filter(|s| !s.is_empty()) else {
        return "—".to_string();
    };
    if let Some((_, time_part)) = raw.split_once('T') {
        let trimmed = time_part.trim_end_matches('Z');
        return trimmed.chars().take(5).collect();
    }
    raw.chars().take(10).collect()
}

fn truncate_tab_title(cmd: &str, max_chars: usize) -> String {
    let clean = cmd.trim();
    if clean.is_empty() {
        return "Shell".to_string();
    }
    let count = clean.chars().count();
    if count <= max_chars {
        clean.to_string()
    } else {
        let prefix: String = clean.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{prefix}…")
    }
}

fn status_dot_spec(p: &PaPalette, status: &str, exit_code: Option<i64>) -> (String, Hsla) {
    match status {
        "running" | "active" => (format!("● {status}"), p.tag_idle),
        "paused" => ("◐ paused".to_string(), p.working),
        "cancelled" => ("● cancelled".to_string(), p.danger),
        _ if exit_code.is_some_and(|c| c != 0) => (
            format!("● exit {}", exit_code.unwrap_or(1)),
            p.danger,
        ),
        other => (format!("○ {other}"), p.quiet_ink),
    }
}

impl PaApp {
    pub(crate) fn sync_activity_sheet_session(&mut self) {
        let sid = self.roster.active_id().unwrap_or_default().to_string();
        self.activity_sheet.set_session(&sid);
    }

    pub(crate) fn request_session_subagents(&self) {
        let Some(row) = self.roster.active_session() else {
            return;
        };
        let _ = self.bridge.tx.send(DaemonCmd::LoadSessionSubagents {
            active_session_id: row.id.clone(),
            session_id: row.session_id.clone(),
        });
    }

    pub(crate) fn request_subagent_messages(
        &mut self,
        subagent_active_session_id: String,
        cx: &mut Context<Self>,
    ) {
        let Some(owner) = self.roster.active_id().map(str::to_string) else {
            return;
        };
        if subagent_active_session_id.is_empty() {
            return;
        }
        self.activity_sheet
            .mark_subagent_loading(&subagent_active_session_id);
        let _ = self.bridge.tx.send(DaemonCmd::LoadSubagentMessages {
            owner_session_id: owner,
            subagent_active_session_id,
        });
        cx.notify();
    }

    pub(crate) fn open_subagent_in_right_sidebar(
        &mut self,
        active_session_id: String,
        child_id: String,
        name: String,
        cx: &mut Context<Self>,
    ) {
        self.open_right_tab(
            RightTabKind::Subagent {
                active_session_id: active_session_id.clone(),
                child_id,
                name,
            },
            cx,
        );
        self.request_subagent_messages(active_session_id, cx);
    }

    pub(crate) fn request_session_heartbeats(&self) {
        let Some(row) = self.roster.active_session() else {
            return;
        };
        let _ = self.bridge.tx.send(DaemonCmd::LoadSessionHeartbeats {
            active_session_id: row.id.clone(),
            session_id: row.session_id.clone(),
        });
    }

    pub(crate) fn request_session_shells(&self) {
        let Some(sid) = self.roster.active_id().map(str::to_string) else {
            return;
        };
        let _ = self.bridge.tx.send(DaemonCmd::LoadSessionShells {
            active_session_id: sid,
        });
    }

    pub(crate) fn request_shell_tail(&mut self, activity_id: String, cx: &mut Context<Self>) {
        let Some(sid) = self.roster.active_id().map(str::to_string) else {
            return;
        };
        self.activity_sheet.mark_tail_loading(&activity_id);
        let _ = self.bridge.tx.send(DaemonCmd::TailKernelShell {
            active_session_id: sid,
            activity_id,
            lines: SHELL_TAIL_LINES,
        });
        cx.notify();
    }

    pub(crate) fn toggle_activity_tab(&mut self, tab: ActivityTab, cx: &mut Context<Self>) {
        if self.roster.is_draft_or_empty() {
            return;
        }
        self.sync_activity_sheet_session();
        let opened = self.activity_sheet.toggle_tab(tab);
        if opened {
            match tab {
                ActivityTab::Heartbeats => self.request_session_heartbeats(),
                ActivityTab::Shells => self.request_session_shells(),
                ActivityTab::Subagents => self.request_session_subagents(),
            }
        }
        cx.notify();
    }

    pub(crate) fn open_hb_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (sched, prompt, deliv) = if let Some(hb) = self.activity_sheet.user_heartbeat() {
            (
                hb.schedule.clone(),
                hb.prompt.clone(),
                hb.delivery_mode.clone(),
            )
        } else {
            ("every 5m".to_string(), String::new(), "steer".to_string())
        };
        self.activity_sheet.begin_hb_edit(&deliv);
        self.hb_schedule_input
            .update(cx, |s, cx| s.set_value(sched, window, cx));
        self.hb_prompt_input
            .update(cx, |s, cx| s.set_value(prompt, window, cx));
        self.hb_prompt_input
            .update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub(crate) fn submit_hb_editor(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.roster.active_session() else {
            return;
        };
        let active_session_id = row.id.clone();
        let session_id = row.session_id.clone();
        let schedule = self.hb_schedule_input.read(cx).value().trim().to_string();
        let prompt = self.hb_prompt_input.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            self.activity_sheet
                .set_notice(false, "Instruction cannot be empty".into());
            cx.notify();
            return;
        }
        let schedule = if schedule.is_empty() {
            "every 5m".to_string()
        } else {
            schedule
        };
        let delivery_mode = self.activity_sheet.hb_delivery_mode().to_string();
        self.activity_sheet.cancel_hb_edit();
        let _ = self.bridge.tx.send(DaemonCmd::SetHeartbeat {
            active_session_id,
            session_id,
            schedule,
            prompt,
            delivery_mode,
        });
        cx.notify();
    }

    pub(crate) fn open_shell_in_right_sidebar(
        &mut self,
        activity_id: String,
        command: String,
        cx: &mut Context<Self>,
    ) {
        let title = truncate_tab_title(&command, 22);
        self.open_right_tab(
            RightTabKind::Shell {
                activity_id: activity_id.clone(),
                title,
            },
            cx,
        );
        self.request_shell_tail(activity_id, cx);
    }

    /// Style B: borderless TUI Composer Sheet seamlessly mounted inside the top
    /// of the composer card.
    pub(crate) fn render_activity_sheet(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let tab = self.activity_sheet.open_tab()?;
        let p = self.palette();
        let (sub_c, hb_c, sh_c) = self.roster.activity_counts();
        let metric: ChipCount = match tab {
            ActivityTab::Subagents => sub_c,
            ActivityTab::Heartbeats => hb_c,
            ActivityTab::Shells => sh_c,
        };
        let (disp_num, is_live) = metric.display_value();
        let drill = self.activity_sheet.drill_id();

        let header = h_flex()
            .w_full()
            .h(px(28.))
            .px(px(12.))
            .gap(px(8.))
            .items_center()
            .child(if drill.is_some() {
                h_flex()
                    .id("act-sheet-back")
                    .gap(px(6.))
                    .items_center()
                    .cursor_pointer()
                    .rounded(px(4.))
                    .px(px(4.))
                    .hover(move |s| s.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.activity_sheet.back();
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(p.quiet_ink)
                            .child("← back"),
                    )
                    .into_any_element()
            } else {
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.ink)
                            .child(tab.title()),
                    )
                    .child(
                        div()
                            .text_size(px(11.5))
                            .text_color(if is_live { p.working } else { p.quiet_ink })
                            .child(disp_num.to_string()),
                    )
                    .into_any_element()
            })
            .child(div().flex_1())
            .when_some(self.activity_sheet.notice.as_ref(), |row, (ok, msg)| {
                row.child(
                    div()
                        .text_size(px(11.))
                        .text_color(if *ok { p.tag_idle } else { p.danger })
                        .child(msg.clone()),
                )
            })
            .child(
                div()
                    .id("act-sheet-close")
                    .px(px(5.))
                    .py(px(2.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(p.quiet_ink)
                    .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.activity_sheet.close();
                        cx.notify();
                    }))
                    .child("esc"),
            );

        let body: AnyElement = match tab {
            ActivityTab::Heartbeats => self.render_heartbeats_sheet_body(cx),
            ActivityTab::Shells => self.render_shells_sheet_body(cx),
            ActivityTab::Subagents => self.render_subagents_sheet_body(cx),
        };

        Some(
            v_flex()
                .id("activity-tui-sheet")
                .w_full()
                .max_h(px(268.))
                .bg(p.transcript_cell_fill)
                .border_b_1()
                .border_color(p.sidebar_group_edge)
                .child(header)
                .child(body)
                .into_any_element(),
        )
    }

    fn render_tui_action_row(
        &self,
        id: &'static str,
        label: &'static str,
        description: &'static str,
        danger: bool,
        on_click: impl Fn(&mut Self, &ClickEvent, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        h_flex()
            .id(id)
            .w_full()
            .px(px(12.))
            .py(px(4.))
            .gap(px(8.))
            .rounded(px(4.))
            .items_center()
            .cursor_pointer()
            .hover(move |s| s.bg(p.sidebar_row_selected))
            .on_click(cx.listener(on_click))
            .child(div().w(px(10.)).text_size(px(12.)).text_color(p.working).child("›"))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if danger { p.danger } else { p.ink })
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.5))
                    .text_color(p.quiet_ink)
                    .child(description),
            )
            .into_any_element()
    }

    fn render_hb_inline_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        let delivery = self.activity_sheet.hb_delivery_mode();
        let deliv_label = if delivery == "follow_up" {
            "follow-up"
        } else {
            "steer"
        };
        h_flex()
            .id("act-hb-editor")
            .w_full()
            .px(px(12.))
            .py(px(7.))
            .gap(px(6.))
            .items_center()
            .border_t_1()
            .border_color(p.sidebar_group_edge)
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(div().w(px(92.)).child(Input::new(&self.hb_schedule_input).cleanable(false)))
            .child(
                div()
                    .id("act-hb-delivery-toggle")
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.sidebar_group_edge)
                    .bg(p.sidebar_group_surface)
                    .flex()
                    .items_center()
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(p.ink)
                    .hover(move |s| s.bg(p.sidebar_row_hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.activity_sheet.toggle_hb_delivery();
                        cx.notify();
                    }))
                    .child(deliv_label),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.hb_prompt_input).cleanable(false)),
            )
            .child(
                div()
                    .id("act-hb-save")
                    .px(px(7.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.ink)
                    .hover(move |s| s.bg(p.sidebar_row_selected))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.submit_hb_editor(cx);
                    }))
                    .child("save"),
            )
            .child(
                div()
                    .id("act-hb-cancel")
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(4.))
                    .cursor_pointer()
                    .text_size(px(11.5))
                    .text_color(p.quiet_ink)
                    .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        this.activity_sheet.cancel_hb_edit();
                        cx.notify();
                    }))
                    .child("cancel"),
            )
            .into_any_element()
    }

    fn render_heartbeats_sheet_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();

        // Drill-in mode
        if let Some(drill_id) = self.activity_sheet.drill_id() {
            if let Some(hb) = self
                .activity_sheet
                .heartbeats()
                .iter()
                .find(|h| h.id == drill_id)
            {
                let is_active = hb.is_active();
                let job_id_toggle = hb.id.clone();
                let job_id_stop = hb.id.clone();
                let next_run = if is_active {
                    short_time_label(hb.next_run_at.as_deref())
                } else {
                    "paused".to_string()
                };
                let created_by = if hb.is_user_heartbeat() {
                    "user"
                } else {
                    "agent"
                };

                let pair = |k: &'static str, v: String, ink: Hsla| {
                    h_flex()
                        .w_full()
                        .px(px(12.))
                        .gap(px(10.))
                        .child(
                            div()
                                .w(px(72.))
                                .flex_shrink_0()
                                .text_size(px(11.5))
                                .text_color(p.sidebar_group_empty_ink)
                                .child(k),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(ink)
                                .child(v),
                        )
                };

                let mut box_el = v_flex()
                    .id("act-hb-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .pb(px(6.))
                    .gap(px(2.))
                    .child(pair("created", created_by.to_string(), p.quiet_ink))
                    .child(pair("delivery", hb.delivery_label().to_string(), p.quiet_ink))
                    .child(pair("schedule", hb.schedule.clone(), p.quiet_ink))
                    .child(pair(
                        "next run",
                        format!("{next_run} · {} runs", hb.run_count),
                        p.quiet_ink,
                    ))
                    .child(pair("prompt", hb.prompt.clone(), p.ink));

                if let Some(err) = hb.last_error.as_deref() {
                    box_el = box_el.child(pair("last error", err.to_string(), p.danger));
                }

                let toggle_action = if is_active { "pause" } else { "resume" };
                let toggle_label = if is_active { "Pause" } else { "Resume" };
                let toggle_desc = if is_active {
                    "Pause scheduled firing without deleting"
                } else {
                    "Resume scheduled firing"
                };

                box_el = box_el
                    .child(div().h(px(4.)))
                    .child(self.render_tui_action_row(
                        "act-hb-toggle",
                        toggle_label,
                        toggle_desc,
                        false,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(row) = this.roster.active_session() {
                                let _ = this.bridge.tx.send(DaemonCmd::ManageHeartbeat {
                                    active_session_id: row.id.clone(),
                                    session_id: row.session_id.clone(),
                                    job_id: job_id_toggle.clone(),
                                    action: toggle_action.to_string(),
                                });
                            }
                        },
                        cx,
                    ));

                if hb.is_user_heartbeat() {
                    box_el = box_el.child(self.render_tui_action_row(
                        "act-hb-edit-act",
                        "Edit",
                        "Update schedule, delivery mode, or instruction",
                        false,
                        |this, _, window, cx| {
                            cx.stop_propagation();
                            this.open_hb_editor(window, cx);
                        },
                        cx,
                    ));
                }

                box_el = box_el.child(self.render_tui_action_row(
                    "act-hb-stop",
                    "Stop",
                    "Cancel and remove this heartbeat",
                    true,
                    move |this, _, _, cx| {
                        cx.stop_propagation();
                        if let Some(row) = this.roster.active_session() {
                            let _ = this.bridge.tx.send(DaemonCmd::ManageHeartbeat {
                                active_session_id: row.id.clone(),
                                session_id: row.session_id.clone(),
                                job_id: job_id_stop.clone(),
                                action: "stop".to_string(),
                            });
                            this.activity_sheet.back();
                            cx.notify();
                        }
                    },
                    cx,
                ));

                if self.activity_sheet.hb_editing() {
                    box_el = box_el.child(self.render_hb_inline_editor(cx));
                }

                return box_el.into_any_element();
            }
        }

        // List mode
        let items = self.activity_sheet.heartbeats();
        let has_user_hb = self.activity_sheet.user_heartbeat().is_some();
        let sel_ix = self.activity_sheet.selected_index;

        let mut list = v_flex()
            .id("act-hb-list-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(6.))
            .pb(px(4.))
            .gap(px(1.));

        if items.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No heartbeats for this session."),
            );
        } else {
            list = list.child(
                h_flex()
                    .w_full()
                    .px(px(6.))
                    .py(px(3.))
                    .gap(px(10.))
                    .child(div().w(px(10.)))
                    .child(div().w(px(86.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Interval"))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Instruction"))
                    .child(div().w(px(58.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Next run"))
                    .child(div().w(px(74.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Status")),
            );
            for (idx, hb) in items.iter().enumerate() {
                let selected = idx == sel_ix;
                let hb_id = hb.id.clone();
                let next_lbl = if hb.is_active() {
                    short_time_label(hb.next_run_at.as_deref())
                } else {
                    "paused".to_string()
                };
                let (st_text, st_color) = status_dot_spec(&p, &hb.status, None);
                let instr = hb
                    .label
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(&hb.prompt)
                    .to_string();

                list = list.child(
                    h_flex()
                        .id(("act-hb-row", idx))
                        .w_full()
                        .h(px(26.))
                        .px(px(6.))
                        .gap(px(10.))
                        .rounded(px(4.))
                        .items_center()
                        .cursor_pointer()
                        .bg(if selected {
                            p.sidebar_row_selected
                        } else {
                            Hsla::transparent_black()
                        })
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.activity_sheet.drill_into(idx, hb_id.clone());
                            cx.notify();
                        }))
                        .child(
                            div()
                                .w(px(10.))
                                .text_size(px(12.))
                                .text_color(if selected {
                                    p.working
                                } else {
                                    Hsla::transparent_black()
                                })
                                .child("›"),
                        )
                        .child(
                            div()
                                .w(px(86.))
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(if selected {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(p.ink)
                                .child(hb.schedule.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(p.quiet_ink)
                                .child(instr),
                        )
                        .child(
                            div()
                                .w(px(58.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.quiet_ink)
                                .child(next_lbl),
                        )
                        .child(
                            div()
                                .w(px(74.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(st_color)
                                .child(st_text),
                        ),
                );
            }
        }

        let mut container = v_flex().flex_1().min_h_0().w_full().child(list);
        if self.activity_sheet.hb_editing() {
            container = container.child(self.render_hb_inline_editor(cx));
        } else {
            container = container.child(
                h_flex()
                    .w_full()
                    .h(px(26.))
                    .px(px(12.))
                    .items_center()
                    .border_t_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.sidebar_group_empty_ink)
                            .child("click row to inspect · esc close"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("act-hb-set-btn")
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_hb_editor(window, cx);
                            }))
                            .child(if has_user_hb {
                                "edit heartbeat"
                            } else {
                                "+ set heartbeat"
                            }),
                    ),
            );
        }
        container.into_any_element()
    }

    fn render_shells_sheet_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();

        // Drill-in mode
        if let Some(drill_id) = self.activity_sheet.drill_id() {
            if let Some(sh) = self.activity_sheet.shell_by_id(drill_id) {
                let sh_id_open = sh.id.clone();
                let sh_cmd_open = sh.command.clone();
                let sh_id_ref = sh.id.clone();
                let sh_id_kill = sh.id.clone();
                let is_running = sh.is_running();
                let (st_text, st_color) = status_dot_spec(&p, &sh.status, sh.exit_code);
                let pid_str = sh
                    .pid
                    .map_or_else(|| "—".to_string(), |v| v.to_string());
                let started_str = short_time_label(sh.started_at.as_deref());
                let dur_str = format_duration_ms(sh.duration_ms);
                let tail_text = if self.activity_sheet.is_tail_loading(&sh.id)
                    && self.activity_sheet.shell_tail(&sh.id).is_none()
                {
                    "Loading output…".to_string()
                } else {
                    self.activity_sheet
                        .shell_tail(&sh.id)
                        .filter(|s| !s.is_empty())
                        .unwrap_or("(no output yet)")
                        .to_string()
                };

                let mut box_el = v_flex()
                    .id("act-sh-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .pb(px(6.))
                    .gap(px(4.))
                    .child(
                        div()
                            .px(px(12.))
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(p.ink)
                            .child(sh.command.clone()),
                    )
                    .child(
                        h_flex()
                            .px(px(12.))
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.quiet_ink)
                                    .child(format!(
                                        "pid {pid_str} · started {started_str} · duration {dur_str}"
                                    )),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(st_color)
                                    .child(st_text),
                            ),
                    )
                    .child(
                        div()
                            .id("act-sh-tail-preview")
                            .mx(px(12.))
                            .pl(px(10.))
                            .py(px(5.))
                            .max_h(px(92.))
                            .overflow_y_scrollbar()
                            .border_l_2()
                            .border_color(p.composer_edge)
                            .text_size(px(11.5))
                            .text_color(p.ink)
                            .child(tail_text),
                    )
                    .child(self.render_tui_action_row(
                        "act-sh-open-rs",
                        "Open in Right sidebar",
                        "Keep full tail output open side by side",
                        false,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_shell_in_right_sidebar(
                                sh_id_open.clone(),
                                sh_cmd_open.clone(),
                                cx,
                            );
                        },
                        cx,
                    ))
                    .child(self.render_tui_action_row(
                        "act-sh-refresh",
                        "Refresh tail",
                        "Fetch latest output lines",
                        false,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.request_shell_tail(sh_id_ref.clone(), cx);
                        },
                        cx,
                    ));

                if is_running {
                    box_el = box_el.child(self.render_tui_action_row(
                        "act-sh-kill",
                        "Kill",
                        "Terminate running shell (SIGTERM → SIGKILL)",
                        true,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(sid) = this.roster.active_id().map(str::to_string) {
                                let _ = this.bridge.tx.send(DaemonCmd::KillKernelShell {
                                    active_session_id: sid,
                                    activity_id: sh_id_kill.clone(),
                                });
                            }
                        },
                        cx,
                    ));
                }

                return box_el.into_any_element();
            }
        }

        // List mode
        let items = self.activity_sheet.shells();
        let sel_ix = self.activity_sheet.selected_index;
        let mut list = v_flex()
            .id("act-sh-list-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(6.))
            .pb(px(4.))
            .gap(px(1.));

        if items.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No shells in this session."),
            );
        } else {
            list = list.child(
                h_flex()
                    .w_full()
                    .px(px(6.))
                    .py(px(3.))
                    .gap(px(10.))
                    .child(div().w(px(10.)))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Command"))
                    .child(div().w(px(58.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Duration"))
                    .child(div().w(px(52.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("PID"))
                    .child(div().w(px(76.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Status")),
            );
            for (idx, sh) in items.iter().enumerate() {
                let selected = idx == sel_ix;
                let sh_id = sh.id.clone();
                let dur = format_duration_ms(sh.duration_ms);
                let pid = sh
                    .pid
                    .map_or_else(|| "—".to_string(), |v| v.to_string());
                let (st_text, st_color) = status_dot_spec(&p, &sh.status, sh.exit_code);

                list = list.child(
                    h_flex()
                        .id(("act-sh-row", idx))
                        .w_full()
                        .h(px(26.))
                        .px(px(6.))
                        .gap(px(10.))
                        .rounded(px(4.))
                        .items_center()
                        .cursor_pointer()
                        .bg(if selected {
                            p.sidebar_row_selected
                        } else {
                            Hsla::transparent_black()
                        })
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.activity_sheet.drill_into(idx, sh_id.clone());
                            this.request_shell_tail(sh_id.clone(), cx);
                        }))
                        .child(
                            div()
                                .w(px(10.))
                                .text_size(px(12.))
                                .text_color(if selected {
                                    p.working
                                } else {
                                    Hsla::transparent_black()
                                })
                                .child("›"),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(if selected {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(p.ink)
                                .child(sh.command.clone()),
                        )
                        .child(
                            div()
                                .w(px(58.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.quiet_ink)
                                .child(dur),
                        )
                        .child(
                            div()
                                .w(px(52.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.quiet_ink)
                                .child(pid),
                        )
                        .child(
                            div()
                                .w(px(76.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(st_color)
                                .child(st_text),
                        ),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(list)
            .child(
                h_flex()
                    .w_full()
                    .h(px(26.))
                    .px(px(12.))
                    .items_center()
                    .border_t_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.sidebar_group_empty_ink)
                            .child("click row to inspect tail · esc close"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("act-sh-refresh-list")
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.request_session_shells();
                                cx.notify();
                            }))
                            .child("refresh"),
                    ),
            )
            .into_any_element()
    }


    fn render_subagents_sheet_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();

        // Drill-in mode
        if let Some(drill_id) = self.activity_sheet.drill_id() {
            if let Some(sub) = self.activity_sheet.subagent_by_id(drill_id) {
                let is_running = sub.is_running();
                let can_manage = sub.can_manage();
                let sub_active = sub.active_session_id.clone();
                let sub_child = sub.child_id.clone();
                let sub_name = sub.name.clone();
                let parent_active = sub.parent_active_session_id.clone();
                let (st_text, st_color) = status_dot_spec(&p, &sub.status, None);
                let dur_str = format_duration_ms(sub.duration_ms);
                let model_str = if sub.model.is_empty() {
                    "inherited".to_string()
                } else {
                    sub.model.clone()
                };

                let preview_text = if self.activity_sheet.is_subagent_loading(&sub.active_session_id)
                    && self
                        .activity_sheet
                        .subagent_messages(&sub.active_session_id)
                        .is_none()
                {
                    "Loading transcript…".to_string()
                } else if let Some(msgs) =
                    self.activity_sheet.subagent_messages(&sub.active_session_id)
                {
                    if msgs.is_empty() {
                        sub.answer_preview
                            .clone()
                            .unwrap_or_else(|| sub.activity.clone())
                    } else {
                        msgs.iter()
                            .rev()
                            .take(4)
                            .collect::<Vec<_>>()
                            .into_iter()
                            .rev()
                            .map(|m| format!("[{}] {}", m.role, m.text))
                            .collect::<Vec<_>>()
                            .join("\n")
                    }
                } else {
                    sub.answer_preview
                        .clone()
                        .unwrap_or_else(|| sub.activity.clone())
                };

                let open_active = sub_active.clone();
                let open_child = sub_child.clone();
                let open_name = sub_name.clone();
                let stop_parent = parent_active.clone();
                let stop_child = sub_child.clone();
                let del_parent = parent_active;
                let del_child = sub_child.clone();
                let del_tab_id = RightTabKind::Subagent {
                    active_session_id: sub_active,
                    child_id: sub_child,
                    name: sub_name.clone(),
                }
                .id();

                let mut box_el = v_flex()
                    .id("act-sub-detail-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .pb(px(6.))
                    .gap(px(4.))
                    .child(
                        h_flex()
                            .px(px(12.))
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.ink)
                                    .child(sub_name),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.quiet_ink)
                                    .child(format!("{model_str} · {dur_str}")),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(st_color)
                                    .child(st_text),
                            ),
                    )
                    .child(
                        div()
                            .id("act-sub-preview")
                            .mx(px(12.))
                            .pl(px(10.))
                            .py(px(5.))
                            .max_h(px(92.))
                            .overflow_y_scrollbar()
                            .border_l_2()
                            .border_color(p.composer_edge)
                            .text_size(px(11.5))
                            .text_color(p.ink)
                            .child(preview_text),
                    )
                    .child(self.render_tui_action_row(
                        "act-sub-open-rs",
                        "Open in Right sidebar",
                        "Watch read-only transcript alongside this session",
                        false,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_subagent_in_right_sidebar(
                                open_active.clone(),
                                open_child.clone(),
                                open_name.clone(),
                                cx,
                            );
                        },
                        cx,
                    ));

                if can_manage && is_running {
                    box_el = box_el.child(self.render_tui_action_row(
                        "act-sub-stop",
                        "Stop",
                        "Abort subagent and record \"Cancelled by user\"",
                        true,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(row) = this.roster.active_session() {
                                let _ = this.bridge.tx.send(DaemonCmd::CancelSubagent {
                                    owner_session_id: row.id.clone(),
                                    owner_durable_id: row.session_id.clone(),
                                    parent_active_session_id: stop_parent.clone(),
                                    child_id: stop_child.clone(),
                                });
                            }
                        },
                        cx,
                    ));
                } else if can_manage {
                    box_el = box_el.child(self.render_tui_action_row(
                        "act-sub-delete",
                        "Delete",
                        "Remove inactive subagent from this session",
                        true,
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(row) = this.roster.active_session() {
                                let _ = this.bridge.tx.send(DaemonCmd::DeleteSubagent {
                                    owner_session_id: row.id.clone(),
                                    owner_durable_id: row.session_id.clone(),
                                    parent_active_session_id: del_parent.clone(),
                                    child_id: del_child.clone(),
                                    tab_id: del_tab_id.clone(),
                                });
                                this.activity_sheet.back();
                                cx.notify();
                            }
                        },
                        cx,
                    ));
                }

                return box_el.into_any_element();
            }
        }

        // List mode
        let items = self.activity_sheet.subagents();
        let sel_ix = self.activity_sheet.selected_index;
        let mut list = v_flex()
            .id("act-sub-list-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .px(px(6.))
            .pb(px(4.))
            .gap(px(1.));

        if items.is_empty() {
            list = list.child(
                div()
                    .px(px(8.))
                    .py(px(10.))
                    .text_size(px(12.))
                    .text_color(p.quiet_ink)
                    .child("No subagents in this session."),
            );
        } else {
            list = list.child(
                h_flex()
                    .w_full()
                    .px(px(6.))
                    .py(px(3.))
                    .gap(px(10.))
                    .child(div().w(px(10.)))
                    .child(div().w(px(150.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Subagent"))
                    .child(div().flex_1().min_w_0().text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Activity"))
                    .child(div().w(px(56.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Duration"))
                    .child(div().w(px(78.)).text_size(px(11.)).text_color(p.sidebar_group_empty_ink).child("Status")),
            );
            for (idx, sub) in items.iter().enumerate() {
                let selected = idx == sel_ix;
                let sub_id = sub.id.clone();
                let sub_sess = sub.active_session_id.clone();
                let dur = format_duration_ms(sub.duration_ms);
                let (st_text, st_color) = status_dot_spec(&p, &sub.status, None);
                let display_name = if sub.depth > 0 {
                    let indent = "  ".repeat(sub.depth.saturating_sub(1).min(3));
                    format!("{indent}↳ {}", sub.name)
                } else {
                    sub.name.clone()
                };

                list = list.child(
                    h_flex()
                        .id(("act-sub-row", idx))
                        .w_full()
                        .h(px(26.))
                        .px(px(6.))
                        .gap(px(10.))
                        .rounded(px(4.))
                        .items_center()
                        .cursor_pointer()
                        .bg(if selected {
                            p.sidebar_row_selected
                        } else {
                            Hsla::transparent_black()
                        })
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.activity_sheet.drill_into(idx, sub_id.clone());
                            this.request_subagent_messages(sub_sess.clone(), cx);
                        }))
                        .child(
                            div()
                                .w(px(10.))
                                .text_size(px(12.))
                                .text_color(if selected {
                                    p.working
                                } else {
                                    Hsla::transparent_black()
                                })
                                .child("›"),
                        )
                        .child(
                            div()
                                .w(px(150.))
                                .truncate()
                                .text_size(px(12.))
                                .font_weight(if selected {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .text_color(p.ink)
                                .child(display_name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(p.quiet_ink)
                                .child(sub.activity.clone()),
                        )
                        .child(
                            div()
                                .w(px(56.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(p.quiet_ink)
                                .child(dur),
                        )
                        .child(
                            div()
                                .w(px(78.))
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(st_color)
                                .child(st_text),
                        ),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(list)
            .child(
                h_flex()
                    .w_full()
                    .h(px(26.))
                    .px(px(12.))
                    .items_center()
                    .border_t_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(p.sidebar_group_empty_ink)
                            .child("click row to inspect · esc close"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("act-sub-refresh-list")
                            .px(px(6.))
                            .py(px(2.))
                            .rounded(px(4.))
                            .cursor_pointer()
                            .text_size(px(11.))
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.request_session_subagents();
                                cx.notify();
                            }))
                            .child("refresh"),
                    ),
            )
            .into_any_element()
    }

    /// Read-only live Subagent transcript tab rendered inside the Right sidebar.
    pub(crate) fn render_right_subagent_tab(
        &self,
        subagent_active_session_id: &str,
        child_id: &str,
        fallback_name: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let lookup_key = if !child_id.is_empty() {
            child_id
        } else {
            subagent_active_session_id
        };
        let sub = self.activity_sheet.subagent_by_id(lookup_key);
        let name = sub
            .map(|s| s.name.clone())
            .unwrap_or_else(|| fallback_name.to_string());
        let is_running = sub.is_some_and(SubagentItem::is_running);
        let can_manage = sub.is_some_and(SubagentItem::can_manage);
        let (st_text, st_color) = sub.map_or_else(
            || ("○ subagent".to_string(), p.quiet_ink),
            |s| status_dot_spec(&p, &s.status, None),
        );
        let meta_line = sub.map(|s| {
            let model = if s.model.is_empty() {
                "inherited"
            } else {
                &s.model
            };
            format!("{model} · {}", format_duration_ms(s.duration_ms))
        });

        let sess_for_refresh = subagent_active_session_id.to_string();
        let stop_parent = sub
            .map(|s| s.parent_active_session_id.clone())
            .unwrap_or_default();
        let stop_child = sub
            .map(|s| s.child_id.clone())
            .unwrap_or_else(|| child_id.to_string());
        let del_parent = stop_parent.clone();
        let del_child = stop_child.clone();
        let del_tab_id = RightTabKind::Subagent {
            active_session_id: subagent_active_session_id.to_string(),
            child_id: child_id.to_string(),
            name: name.clone(),
        }
        .id();

        let msgs = self
            .activity_sheet
            .subagent_messages(subagent_active_session_id);
        let loading = self
            .activity_sheet
            .is_subagent_loading(subagent_active_session_id);

        let mut transcript_col = v_flex()
            .id("rs-sub-transcript-scroll")
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scrollbar()
            .p(px(12.))
            .gap(px(10.))
            .bg(p.transcript_surface);

        match msgs {
            Some(list) if !list.is_empty() => {
                for (ix, m) in list.iter().enumerate() {
                    transcript_col = transcript_col.child(
                        v_flex()
                            .id(("rs-sub-msg", ix))
                            .w_full()
                            .pl(px(10.))
                            .py(px(5.))
                            .gap(px(3.))
                            .border_l_2()
                            .border_color(if m.role == "task" {
                                p.working
                            } else {
                                p.composer_edge
                            })
                            .child(
                                div()
                                    .text_size(px(10.5))
                                    .text_color(p.sidebar_group_empty_ink)
                                    .child(m.role.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .line_height(px(18.))
                                    .text_color(p.ink)
                                    .child(m.text.clone()),
                            ),
                    );
                }
            }
            _ => {
                let placeholder = if loading {
                    "Loading transcript…".to_string()
                } else {
                    sub.and_then(|s| s.answer_preview.clone())
                        .unwrap_or_else(|| "No messages yet.".to_string())
                };
                transcript_col = transcript_col.child(
                    div()
                        .text_size(px(12.))
                        .text_color(p.quiet_ink)
                        .child(placeholder),
                );
            }
        }

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .border_b_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.ink)
                                    .child(name),
                            )
                            .when_some(meta_line, |col, meta| {
                                col.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.quiet_ink)
                                        .child(meta),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.5))
                            .text_color(st_color)
                            .child(st_text),
                    )
                    .child(
                        div()
                            .id("rs-sub-refresh")
                            .px(px(7.))
                            .py(px(3.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.request_session_subagents();
                                this.request_subagent_messages(sess_for_refresh.clone(), cx);
                            }))
                            .child("Refresh"),
                    )
                    .when(can_manage && is_running, |row| {
                        let sp = stop_parent.clone();
                        let sc = stop_child.clone();
                        row.child(
                            div()
                                .id("rs-sub-stop")
                                .px(px(7.))
                                .py(px(3.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .text_color(p.danger)
                                .hover(move |s| s.bg(p.danger_surface))
                                .on_click(cx.listener(move |this, _, _, _| {
                                    if let Some(row) = this.roster.active_session() {
                                        let _ = this.bridge.tx.send(DaemonCmd::CancelSubagent {
                                            owner_session_id: row.id.clone(),
                                            owner_durable_id: row.session_id.clone(),
                                            parent_active_session_id: sp.clone(),
                                            child_id: sc.clone(),
                                        });
                                    }
                                }))
                                .child("Stop"),
                        )
                    })
                    .when(can_manage && !is_running, |row| {
                        let dp = del_parent.clone();
                        let dc = del_child.clone();
                        let dt = del_tab_id.clone();
                        row.child(
                            div()
                                .id("rs-sub-delete")
                                .px(px(7.))
                                .py(px(3.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .text_color(p.danger)
                                .hover(move |s| s.bg(p.danger_surface))
                                .on_click(cx.listener(move |this, _, _, _| {
                                    if let Some(row) = this.roster.active_session() {
                                        let _ = this.bridge.tx.send(DaemonCmd::DeleteSubagent {
                                            owner_session_id: row.id.clone(),
                                            owner_durable_id: row.session_id.clone(),
                                            parent_active_session_id: dp.clone(),
                                            child_id: dc.clone(),
                                            tab_id: dt.clone(),
                                        });
                                    }
                                }))
                                .child("Delete"),
                        )
                    }),
            )
            .child(transcript_col)
            .child(
                div()
                    .w_full()
                    .px(px(12.))
                    .py(px(6.))
                    .border_t_1()
                    .border_color(p.sidebar_group_edge)
                    .text_size(px(11.))
                    .text_color(p.sidebar_group_empty_ink)
                    .child("read-only · subagents are controlled by the parent agent"),
            )
            .into_any_element()
    }

    /// Full scrollable Shell tail tab rendered inside the Right sidebar.
    pub(crate) fn render_right_shell_tab(
        &self,
        activity_id: &str,
        fallback_title: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let sh = self.activity_sheet.shell_by_id(activity_id);
        let cmd = sh
            .map(|s| s.command.clone())
            .unwrap_or_else(|| fallback_title.to_string());
        let is_running = sh.is_some_and(ShellActivityItem::is_running);
        let (st_text, st_color) = sh.map_or_else(
            || ("○ shell".to_string(), p.quiet_ink),
            |s| status_dot_spec(&p, &s.status, s.exit_code),
        );
        let meta_line = sh.map(|s| {
            let pid = s.pid.map_or_else(|| "—".to_string(), |v| v.to_string());
            let dur = format_duration_ms(s.duration_ms);
            format!("pid {pid} · duration {dur}")
        });
        let tail_text = if self.activity_sheet.is_tail_loading(activity_id)
            && self.activity_sheet.shell_tail(activity_id).is_none()
        {
            "Loading output…".to_string()
        } else {
            self.activity_sheet
                .shell_tail(activity_id)
                .filter(|s| !s.is_empty())
                .unwrap_or("(no output yet)")
                .to_string()
        };

        let id_ref = activity_id.to_string();
        let id_kill = activity_id.to_string();

        v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .w_full()
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(8.))
                    .items_center()
                    .border_b_1()
                    .border_color(p.sidebar_group_edge)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(px(2.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(p.ink)
                                    .child(cmd),
                            )
                            .when_some(meta_line, |col, meta| {
                                col.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.quiet_ink)
                                        .child(meta),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.5))
                            .text_color(st_color)
                            .child(st_text),
                    )
                    .child(
                        div()
                            .id("rs-sh-refresh")
                            .px(px(7.))
                            .py(px(3.))
                            .rounded(px(5.))
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(p.quiet_ink)
                            .hover(move |s| s.bg(p.sidebar_row_hover).text_color(p.ink))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.request_session_shells();
                                this.request_shell_tail(id_ref.clone(), cx);
                            }))
                            .child("Refresh"),
                    )
                    .when(is_running, |row| {
                        row.child(
                            div()
                                .id("rs-sh-kill")
                                .px(px(7.))
                                .py(px(3.))
                                .rounded(px(5.))
                                .cursor_pointer()
                                .text_size(px(11.5))
                                .text_color(p.danger)
                                .hover(move |s| s.bg(p.danger_surface))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    if let Some(sid) = this.roster.active_id().map(str::to_string) {
                                        let _ = this.bridge.tx.send(DaemonCmd::KillKernelShell {
                                            active_session_id: sid,
                                            activity_id: id_kill.clone(),
                                        });
                                    }
                                }))
                                .child("Kill"),
                        )
                    }),
            )
            .child(
                div()
                    .id("rs-sh-tail-scroll")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scrollbar()
                    .p(px(12.))
                    .bg(p.transcript_surface)
                    .text_size(px(11.5))
                    .line_height(px(18.))
                    .text_color(p.ink)
                    .child(tail_text),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_hb(id: &str, status: &str) -> HeartbeatItem {
        HeartbeatItem {
            id: id.to_string(),
            active_session_id: "s1".to_string(),
            session_id: "u1".to_string(),
            status: status.to_string(),
            source: "heartbeat".to_string(),
            delivery_mode: "steer".to_string(),
            schedule: "every 5m".to_string(),
            label: None,
            prompt: "check status".to_string(),
            next_run_at: None,
            run_count: 0,
            last_error: None,
        }
    }

    fn sample_shell(id: &str, status: &str) -> ShellActivityItem {
        ShellActivityItem {
            id: id.to_string(),
            command: "cargo test".to_string(),
            pid: Some(42),
            started_at: None,
            status: status.to_string(),
            exit_code: if status == "running" { None } else { Some(0) },
            duration_ms: Some(1200),
        }
    }

    #[::core::prelude::v1::test]
    fn toggle_tab_opens_switches_and_closes_on_same_segment() {
        let mut m = ActivitySheetModel::new();
        m.set_session("s1");
        assert!(!m.is_open());

        assert!(m.toggle_tab(ActivityTab::Heartbeats));
        assert_eq!(m.open_tab(), Some(ActivityTab::Heartbeats));

        m.drill_into(0, "hb-1".into());
        assert_eq!(m.drill_id(), Some("hb-1"));

        // Switching to Shells resets drill-in.
        assert!(m.toggle_tab(ActivityTab::Shells));
        assert_eq!(m.open_tab(), Some(ActivityTab::Shells));
        assert_eq!(m.drill_id(), None);

        // Clicking active segment closes sheet.
        assert!(!m.toggle_tab(ActivityTab::Shells));
        assert!(!m.is_open());
    }

    #[::core::prelude::v1::test]
    fn escape_steps_back_from_editor_then_drill_then_closes_sheet() {
        let mut m = ActivitySheetModel::new();
        m.set_session("s1");
        m.toggle_tab(ActivityTab::Heartbeats);
        m.drill_into(0, "hb-1".into());
        m.begin_hb_edit("follow_up");
        assert_eq!(m.hb_delivery_mode(), "follow_up");

        assert!(m.handle_escape());
        assert!(!m.hb_editing());
        assert_eq!(m.drill_id(), Some("hb-1"));

        assert!(m.handle_escape());
        assert_eq!(m.drill_id(), None);
        assert!(m.is_open());

        assert!(m.handle_escape());
        assert!(!m.is_open());
        assert!(!m.handle_escape());
    }

    #[::core::prelude::v1::test]
    fn stale_session_replies_are_rejected_after_session_switch() {
        let mut m = ActivitySheetModel::new();
        m.set_session("s1");
        assert!(m.apply_heartbeats("s1", vec![sample_hb("hb-1", "active")]));
        assert!(m.apply_shells("s1", vec![sample_shell("sh-1", "running")]));
        assert!(m.apply_shell_tail("s1", "sh-1".into(), "ok".into()));

        m.set_session("s2");
        assert!(m.heartbeats().is_empty());
        assert!(m.shells().is_empty());

        // Stale s1 replies must not write into s2.
        assert!(!m.apply_heartbeats("s1", vec![sample_hb("hb-1", "active")]));
        assert!(!m.apply_shells("s1", vec![sample_shell("sh-1", "running")]));
        assert!(!m.apply_shell_tail("s1", "sh-1".into(), "stale".into()));
        assert!(m.heartbeats().is_empty());
        assert!(m.shells().is_empty());
        assert_eq!(m.shell_tail("sh-1"), None);
    }

    #[::core::prelude::v1::test]
    fn hb_editor_toggles_delivery_mode_and_cancels_cleanly() {
        let mut m = ActivitySheetModel::new();
        m.set_session("s1");
        m.begin_hb_edit("steer");
        assert!(m.hb_editing());
        assert_eq!(m.hb_delivery_mode(), "steer");
        m.toggle_hb_delivery();
        assert_eq!(m.hb_delivery_mode(), "follow_up");
        m.toggle_hb_delivery();
        assert_eq!(m.hb_delivery_mode(), "steer");
        m.cancel_hb_edit();
        assert!(!m.hb_editing());
    }

    fn sample_sub(id: &str, status: &str) -> SubagentItem {
        SubagentItem {
            id: id.to_string(),
            child_id: id.to_string(),
            active_session_id: format!("sess-{id}"),
            parent_active_session_id: "s1".to_string(),
            name: format!("worker-{id}"),
            status: status.to_string(),
            depth: 0,
            model: "claude-sonnet-4".to_string(),
            activity: "Working".to_string(),
            duration_ms: Some(2500),
            answer_preview: None,
        }
    }

    #[::core::prelude::v1::test]
    fn subagent_drill_in_and_stale_session_rejection_behave_cleanly() {
        let mut m = ActivitySheetModel::new();
        m.set_session("s1");
        m.toggle_tab(ActivityTab::Subagents);
        assert!(m.apply_subagents("s1", vec![sample_sub("c1", "running")]));
        m.drill_into(0, "c1".into());
        assert_eq!(m.drill_id(), Some("c1"));
        assert!(m.apply_subagent_messages(
            "s1",
            "sess-c1".into(),
            vec![SubagentMessagePreview {
                role: "task".into(),
                text: "Audit".into(),
            }],
        ));
        assert_eq!(m.subagent_messages("sess-c1").map(<[_]>::len), Some(1));

        // Removing the drilled subagent drops drill-in cleanly.
        assert!(m.apply_subagents("s1", Vec::new()));
        assert_eq!(m.drill_id(), None);

        // Switching to s2 rejects stale s1 subagent payloads.
        m.set_session("s2");
        assert!(!m.apply_subagents("s1", vec![sample_sub("c1", "running")]));
        assert!(!m.apply_subagent_messages("s1", "sess-c1".into(), Vec::new()));
        assert!(m.subagents().is_empty());
    }
}
