//! Composer module: multi-line prompt editor, unified drag-resize geometry,
//! live `/command` color decorations, Tab completion overlay, attachments, and status strip.

use crate::activity_sheet::ActivityTab;
use crate::app::PaApp;
use crate::assets::{
    ICON_HEARTBEAT_PATH, ICON_SEND_PATH, ICON_SHELLS_PATH, ICON_SUBAGENTS_PATH,
};
use crate::daemon::DaemonCmd;
use crate::popups::OpenPopup;
use crate::sidebar::{session_key, RenameTarget};
use crate::slash::{
    complete_slash_token, matching_slash_commands, slash_command_ranges, slash_dispatch,
    slash_token_at, SlashAction,
};
use crate::transcript::{CardVariant, ItemKind, TranscriptItem};
use crate::transcript_view::{ease_in_out_cubic, lerp_hsla};
use base64::Engine as _;
use gpui_kit::component::{
    h_flex,
    input::{TextDecoration, Textarea},
    text::TextViewState,
    v_flex, Icon, IconName, StyledExt as _,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::thinking_level_label;
use serde_json::{json, Value};
use std::sync::Arc;

/// Shared max width for both the transcript column and the composer band so their
/// left/right edges stay locked together on wide displays.
pub const CHAT_COLUMN_MAX_WIDTH: f32 = 860.0;

/// Minimum width of the chat sheet (band padding + composer controls + breathing space)
/// so the composer bottom dock never cramps or collapses its buttons.
pub const CHAT_COLUMN_MIN_WIDTH: f32 = 512.0;

/// Unified minimum height for the composer input in both default auto-grow mode
/// and interactive drag-resize mode so dragging never jumps from the default area.
pub const COMPOSER_MIN_HEIGHT: f32 = 44.0;
pub const COMPOSER_MAX_HEIGHT: f32 = 320.0;

pub const BUSY_STRIP_HEIGHT: f32 = 26.0;
pub const SPINNER_FRAMES: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];
const SLASH_VISIBLE_LIMIT: usize = 6;

#[derive(Clone)]
pub(crate) struct Attachment {
    pub(crate) mime: String,
    pub(crate) base64: String,
    pub(crate) image: Arc<Image>,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl Attachment {
    pub(crate) fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        let (mime, format, bytes) = if bytes
            .starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
        {
            ("image/png", ImageFormat::Png, bytes)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            ("image/jpeg", ImageFormat::Jpeg, bytes)
        } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            ("image/webp", ImageFormat::Webp, bytes)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            ("image/gif", ImageFormat::Gif, bytes)
        } else if bytes.starts_with(b"BM") {
            // Windows clipboard screenshots arrive as CF_DIB/BMP; transcode to PNG for LLM APIs.
            let decoded =
                image::load_from_memory_with_format(&bytes, image::ImageFormat::Bmp).ok()?;
            let mut png_bytes = Vec::new();
            decoded
                .write_to(
                    &mut std::io::Cursor::new(&mut png_bytes),
                    image::ImageFormat::Png,
                )
                .ok()?;
            ("image/png", ImageFormat::Png, png_bytes)
        } else {
            return None;
        };
        let (width, height) = crate::transcript::image_dimensions(&bytes);
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let image = Arc::new(Image::from_bytes(format, bytes));
        Some(Self {
            mime: mime.to_string(),
            base64,
            image,
            width,
            height,
        })
    }

    pub(crate) fn from_clipboard_item(item: &ClipboardItem) -> Vec<Self> {
        let mut out = Vec::new();
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => {
                    if let Some(att) = Self::from_bytes(img.bytes.clone()) {
                        out.push(att);
                    }
                }
                ClipboardEntry::ExternalPaths(paths) => {
                    for path in paths.paths() {
                        if let Ok(bytes) = std::fs::read(path) {
                            if let Some(att) = Self::from_bytes(bytes) {
                                out.push(att);
                            }
                        }
                    }
                }
                ClipboardEntry::String(_) => {}
            }
        }
        out
    }

    pub(crate) fn to_transcript_image(&self) -> crate::transcript::TranscriptImage {
        crate::transcript::TranscriptImage {
            image: self.image.clone(),
            width: self.width,
            height: self.height,
        }
    }

    pub(crate) fn block(&self) -> Value {
        json!({
            "type": "image",
            "data": self.base64,
            "mimeType": self.mime,
        })
    }
}

pub(crate) fn spinner_frame_for_ms(phase_ms: u64) -> &'static str {
    SPINNER_FRAMES[((phase_ms / 80) as usize) % SPINNER_FRAMES.len()]
}

pub(crate) fn busy_strip_geometry(mix: f32, opening: bool) -> (f32, f32) {
    let t = mix.clamp(0.0, 1.0);
    let eased = if opening {
        1.0 - (1.0 - t).powi(3)
    } else {
        t * t
    };
    let opacity = if opening {
        ((t - 0.25) / 0.75).clamp(0.0, 1.0)
    } else {
        (t / 0.85).clamp(0.0, 1.0)
    };
    (BUSY_STRIP_HEIGHT * eased, opacity)
}

pub(crate) fn resolve_busy_label<'a>(
    pending_slash: Option<&str>,
    incoming: &'a str,
    force: bool,
) -> &'a str {
    if !force {
        if let Some(cmd) = pending_slash {
            return match cmd {
                "compact" => "Compacting",
                "refine" => "Refining",
                _ => incoming,
            };
        }
    }
    incoming
}

pub(crate) fn parse_action_update_lanes(event: &Value) -> (Vec<String>, Vec<String>) {
    let lane = |name: &str| -> Vec<String> {
        event
            .get("actions")
            .and_then(|actions| actions.get(name))
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|item| {
                        item.as_str().map(str::to_string).or_else(|| {
                            item.get("text")
                                .and_then(Value::as_str)
                                .map(str::to_string)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    (lane("steering"), lane("followUps"))
}

impl PaApp {
    /// Current effective height of the composer textarea, unified across default
    /// auto-grow and manual drag-resize modes.
    pub(crate) fn effective_composer_height(&self, cx: &App) -> f32 {
        let measured = self.composer.read(cx).input_bounds().size.height.as_f32();
        self.composer_model.effective_height(measured)
    }

    /// Recompute `/command` color decorations on the composer textarea and clamp
    /// the active slash popup selection index.
    pub(crate) fn sync_slash_decorations(&mut self, cx: &mut Context<Self>) {
        let p = self.palette();
        let (text, caret) = {
            let input = self.composer.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let style = HighlightStyle {
            color: Some(p.working),
            font_weight: Some(FontWeight::MEDIUM),
            ..Default::default()
        };
        let decorations: Vec<TextDecoration> = slash_command_ranges(&text)
            .into_iter()
            .map(|range| TextDecoration::new(range, style))
            .collect();
        self.composer.update(cx, |input, cx| {
            input.set_decorations(decorations, cx);
        });

        let match_count = slash_token_at(&text, caret)
            .map(|(_, prefix)| matching_slash_commands(prefix).len().min(SLASH_VISIBLE_LIMIT))
            .unwrap_or(0);
        self.composer_model.clamp_slash_index(match_count);
    }

    /// Active slash command matches for the `/prefix` token under the composer caret.
    pub(crate) fn active_slash_matches(
        &self,
        cx: &App,
    ) -> Option<Vec<&'static pa_client::commands::Command>> {
        if self.open_popup != OpenPopup::None {
            return None;
        }
        let input = self.composer.read(cx);
        let text = input.value();
        let caret = input.cursor();
        let (_, prefix) = slash_token_at(&text, caret)?;
        let matches: Vec<_> = matching_slash_commands(prefix)
            .into_iter()
            .take(SLASH_VISIBLE_LIMIT)
            .collect();
        if matches.is_empty() {
            None
        } else {
            Some(matches)
        }
    }

    /// Step the highlighted row in the slash command popup when open.
    pub(crate) fn step_slash_selection(&mut self, delta: isize, cx: &mut Context<Self>) -> bool {
        let Some(matches) = self.active_slash_matches(cx) else {
            return false;
        };
        if self.composer_model.step_slash_index(delta, matches.len()) {
            cx.notify();
            true
        } else {
            false
        }
    }

    /// Complete the selected slash command into the composer at the caret (triggered by Tab or click).
    pub(crate) fn accept_slash_completion(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(matches) = self.active_slash_matches(cx) else {
            return false;
        };
        let pick = index.min(matches.len().saturating_sub(1));
        let cmd_name = matches[pick].name;
        let (text, caret) = {
            let input = self.composer.read(cx);
            (input.value().to_string(), input.cursor())
        };
        let Some((completed, new_caret)) = complete_slash_token(&text, caret, cmd_name) else {
            return false;
        };
        self.composer.update(cx, |input, cx| {
            input.set_value(completed, window, cx);
            input.set_selected_range(new_caret..new_caret, cx);
        });
        self.composer_model.reset_slash_index();
        self.sync_slash_decorations(cx);
        self.sync_stop_target(cx);
        cx.notify();
        true
    }

    pub(crate) fn set_composer_text(
        &mut self,
        value: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |input, cx| {
            input.set_value(value.into(), window, cx);
        });
        self.composer_model.reset_slash_index();
        self.sync_slash_decorations(cx);
    }

    pub(crate) fn send_current_prompt(
        &mut self,
        steer: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_popup = OpenPopup::None;
        self.hovered_model = None;
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() && self.composer_model.attachments().is_empty() {
            return;
        }

        let mut slash_cmd_kind: Option<&'static str> = None;
        if self.composer_model.attachments().is_empty() {
            match slash_dispatch(&text) {
                SlashAction::SendMessage => {}
                SlashAction::SendDaemonCommand => {
                    let cmd = text
                        .trim()
                        .strip_prefix('/')
                        .and_then(|s| s.split_whitespace().next());
                    slash_cmd_kind = match cmd {
                        Some("compact") => Some("compact"),
                        Some("refine") => Some("refine"),
                        _ => Some("daemon"),
                    };
                }
                SlashAction::Model => {
                    self.open_popup = OpenPopup::ModelMenu;
                    self.set_composer_text("", window, cx);
                    cx.notify();
                    return;
                }
                SlashAction::Settings => {
                    self.refresh_providers_snapshot();
                    self.open_popup = OpenPopup::SettingsMenu;
                    self.set_composer_text("", window, cx);
                    cx.notify();
                    return;
                }
                SlashAction::NewChat => {
                    self.set_composer_text("", window, cx);
                    self.request_new_session(window, cx);
                    return;
                }
                SlashAction::Rename(new_name) => {
                    self.set_composer_text("", window, cx);
                    if let Some(row) = self.roster.active_session().cloned() {
                        let target = RenameTarget {
                            key: session_key(&row),
                            id: row.id.clone(),
                            file: row.session_file.clone(),
                            is_resident: row.is_resident(),
                        };
                        if new_name.is_empty() {
                            self.start_rename(target, row.name, window, cx);
                        } else {
                            self.roster.rename(&target.key, new_name.clone());
                            if target.is_resident && !target.id.is_empty() {
                                let _ = self.bridge.tx.send(DaemonCmd::RenameSession {
                                    session: target.id,
                                    name: new_name,
                                });
                            } else if !target.file.is_empty() {
                                let _ = self.bridge.tx.send(DaemonCmd::RenameSaved {
                                    session_path: target.file,
                                    name: new_name,
                                });
                            }
                            cx.notify();
                        }
                    }
                    return;
                }
                SlashAction::Export => {
                    self.set_composer_text("", window, cx);
                    if let Some(sid) = self.roster.active_id().map(str::to_string) {
                        let _ = self.bridge.tx.send(DaemonCmd::ExportHtml { session: sid });
                    }
                    cx.notify();
                    return;
                }
                SlashAction::Copy => {
                    self.set_composer_text("", window, cx);
                    if let Some(last) = self
                        .items
                        .iter()
                        .rev()
                        .find(|it| it.kind == ItemKind::Assistant && !it.raw_body.is_empty())
                    {
                        self.copy_text(last.raw_body.clone(), cx);
                    }
                    cx.notify();
                    return;
                }
                SlashAction::Quit => {
                    cx.quit();
                    return;
                }
                SlashAction::NotWired(name) => {
                    self.set_composer_text("", window, cx);
                    self.dev_errors.push(
                        "slash",
                        format!("/{name} is not available in the desktop app"),
                    );
                    cx.notify();
                    return;
                }
            }
        }

        let held = self.composer_model.take_attachments();
        let preview_images: Vec<crate::transcript::TranscriptImage> = held.iter().map(Attachment::to_transcript_image).collect();
        let wire_images: Vec<Value> = held.iter().map(Attachment::block).collect();

        if let Some((lane, slot, expected)) = self.composer_model.take_edit_queue() {
            if let Some(sid) = self.roster.active_id().map(str::to_string) {
                let _ = self.bridge.tx.send(DaemonCmd::EditQueue {
                    session: sid,
                    lane,
                    slot,
                    expected,
                    text,
                    images: wire_images,
                });
                self.set_composer_text("", window, cx);
                self.sync_stop_target(cx);
                cx.notify();
                return;
            }
        }

        if !self.is_streaming {
            let old_len = self.items.len();
            let next_id = self.next_item_id;
            self.next_item_id = self.next_item_id.wrapping_add(1);
            let md = if text.is_empty() {
                None
            } else {
                Some(cx.new(|cx| TextViewState::markdown(&text, cx)))
            };
            let mut optimistic = TranscriptItem {
                id: next_id,
                entry_id: None,
                call_id: None,
                kind: ItemKind::User,
                variant: CardVariant::None,
                verb: SharedString::default(),
                title: "You".into(),
                meta: SharedString::default(),
                body_md: md,
                raw_body: text.clone(),
                output: None,
                images: preview_images,
                expanded: true,
                active: false,
                failed: false,
                fingerprint: 0,
            };
            optimistic.fingerprint = optimistic.compute_fingerprint();
            self.items.push(optimistic);
            self.list_state.splice(old_len..old_len, 1);
            self.list_state.set_follow_mode(FollowMode::Tail);
        }


        if !self.is_streaming {
            self.composer_model.set_pending_slash(slash_cmd_kind);
            self.composer_model.set_busy_label("Waiting", false);
        }

        if self.roster.is_draft_or_empty() {
            let (model, level) = self.roster.consume_draft_model_for_start();
            let cwd = self.roster.take_draft_cwd();
            if let Some(ref dir) = cwd {
                let project = self
                    .diff_model
                    .worktrees()
                    .iter()
                    .find(|wt| wt.is_main)
                    .map(|wt| wt.path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dir.clone());
                self.settings.record_recent_project(&project, dir);
                self.settings.save();
            }
            self.session_switch_gen = self.session_switch_gen.wrapping_add(1);
            self.is_streaming = true;
            let workspace_id = self.workspace_ui.draft_workspace_id.clone();
            let worktree_path = self.workspace_ui.draft_worktree_path.clone();
            let _ = self.bridge.tx.send(DaemonCmd::StartChat {
                text,
                images: wire_images,
                model,
                level,
                cwd,
                workspace_id,
                worktree_path,
            });
            self.set_composer_text("", window, cx);
            self.sync_stop_target(cx);
            cx.notify();
            return;
        }

        let Some(sid) = self.roster.active_id().map(str::to_string) else {
            return;
        };

        self.roster.mark_active_streaming();
        self.is_streaming = true;

        if steer {
            let _ = self.bridge.tx.send(DaemonCmd::SteerPrompt {
                session: sid,
                text,
                images: wire_images,
            });
        } else {
            let _ = self.bridge.tx.send(DaemonCmd::SendPrompt {
                session: sid,
                text,
                images: wire_images,
            });
        }
        self.set_composer_text("", window, cx);
        self.sync_stop_target(cx);
        cx.notify();
    }

    pub(crate) fn render_activity_chip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let open_tab = self.activity_sheet.open_tab();
        let disabled = self.roster.is_draft_or_empty();
        let (subagent_count, heartbeat_count, shell_count) = self.roster.activity_counts();

        let segment = |id: &'static str,
                       tab: ActivityTab,
                       icon_path: &'static str,
                       count: pa_client::ChipCount,
                       cx: &mut Context<Self>| {
            let (val, live) = count.display_value();
            let is_open = open_tab == Some(tab);
            let ink = if live {
                p.working
            } else if is_open {
                p.ink
            } else {
                p.quiet_ink
            };
            h_flex()
                .id(id)
                .h(px(20.))
                .px(px(7.))
                .flex_shrink_0()
                .rounded(px(5.))
                .items_center()
                .gap(px(5.))
                .bg(if is_open {
                    p.sidebar_row_selected
                } else {
                    Hsla::transparent_black()
                })
                .when(!disabled, |el| {
                    el.cursor_pointer()
                        .hover(move |s| s.bg(p.sidebar_row_hover))
                        .on_mouse_down(MouseButton::Left, |_, window, cx| {
                            window.prevent_default();
                            cx.stop_propagation();
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.toggle_activity_tab(tab, cx);
                        }))
                })
                .child(
                    svg()
                        .path(icon_path)
                        .size(px(13.))
                        .flex_shrink_0()
                        .text_color(ink),
                )
                .child(
                    div()
                        .pb(px(1.5))
                        .text_size(px(11.5))
                        .line_height(px(11.5))
                        .whitespace_nowrap()
                        .text_color(ink)
                        .child(val.to_string()),
                )
        };

        h_flex()
            .h(px(26.))
            .flex_shrink_0()
            .px(px(3.))
            .gap(px(2.))
            .rounded(px(7.))
            .border_1()
            .border_color(p.composer_edge)
            .bg(p.sidebar_group_surface)
            .items_center()
            .child(segment("act-chip-sub", ActivityTab::Subagents, ICON_SUBAGENTS_PATH, subagent_count, cx))
            .child(segment("act-chip-hb", ActivityTab::Heartbeats, ICON_HEARTBEAT_PATH, heartbeat_count, cx))
            .child(segment("act-chip-sh", ActivityTab::Shells, ICON_SHELLS_PATH, shell_count, cx))
    }

    pub(crate) fn render_send_stop_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let mix = ease_in_out_cubic(self.composer_model.stop_mix());
        let bg_color = lerp_hsla(p.composer_send_fill, p.danger, mix);
        let is_stop = self.composer_model.is_stop_target();

        let arrow_size = 12.0 * (1.0 - 0.35 * mix);
        let arrow_opacity = (1.0 - mix).clamp(0.0, 1.0);
        let square_size = 4.0 + 4.0 * mix;
        let square_radius = 3.0 - 1.5 * mix;
        let square_opacity = mix.clamp(0.0, 1.0);

        div()
            .id("send-stop-circle")
            .relative()
            .size(px(26.))
            .flex_shrink_0()
            .rounded_full()
            .bg(bg_color)
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .when(arrow_opacity > 0.01, |btn| {
                btn.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(arrow_opacity)
                        .child(
                            svg()
                                .path(ICON_SEND_PATH)
                                .size(px(arrow_size))
                                .text_color(p.composer_send_mark),
                        ),
                )
            })
            .when(square_opacity > 0.01, |btn| {
                btn.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(square_opacity)
                        .child(
                            div()
                                .size(px(square_size))
                                .rounded(px(square_radius))
                                .bg(rgb(0xffffff)),
                        ),
                )
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                cx.stop_propagation();
                if is_stop {
                    if let Some(sid) = this.roster.active_id().map(str::to_string) {
                        let _ = this.bridge.tx.send(DaemonCmd::Abort { session: sid });
                    }
                } else {
                    let steer = this.is_streaming;
                    this.send_current_prompt(steer, window, cx);
                }
            }))
    }

    pub(crate) fn render_busy_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (strip_h, strip_opacity, label, elapsed_secs) =
            self.composer_model.busy_strip_metrics(self.is_streaming);
        if strip_h <= 0.5 {
            return None;
        }
        let p = self.palette();
        let queued_total = self.composer_model.queued_total();
        let queue_open = self.open_popup == OpenPopup::QueuePanel;
        let show_working = self.is_streaming || self.composer_model.has_busy_label();
        let spinner = spinner_frame_for_ms(self.anim_phase_ms);

        Some(
            div()
                .id("busy-status-strip")
                .relative()
                .w_full()
                .h(px(strip_h))
                .opacity(strip_opacity)
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(px(strip_h + 12.0))
                        .rounded_t(px(12.))
                        .border_t_1()
                        .border_l_1()
                        .border_r_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface),
                )
                .child(
                    h_flex()
                        .relative()
                        .w_full()
                        .h(px(strip_h))
                        .px(px(14.))
                        .items_center()
                        .justify_between()
                        .overflow_hidden()
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(5.))
                                .min_w_0()
                                .when(show_working && strip_h > 1.0, |left| {
                                    left.child(
                                        div()
                                            .w(px(14.))
                                            .text_size(px(16.))
                                            .line_height(px(strip_h))
                                            .font_family("Cascadia Code")
                                            .text_color(p.working)
                                            .child(spinner),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .line_height(px(14.))
                                            .font_family("Cascadia Code")
                                            .text_color(p.working)
                                            .whitespace_nowrap()
                                            .child(label.to_string()),
                                    )
                                    .child(
                                        div()
                                            .ml(px(4.))
                                            .text_size(px(11.))
                                            .line_height(px(14.))
                                            .font_family("Cascadia Code")
                                            .text_color(p.sidebar_group_empty_ink)
                                            .whitespace_nowrap()
                                            .child(format!("{elapsed_secs}s")),
                                    )
                                }),
                        )
                        .when(queued_total > 0 && strip_h > 1.0, |row| {
                            row.child(
                                h_flex()
                                    .id("queue-chip-btn")
                                    .h(px(18.))
                                    .px(px(8.))
                                    .gap(px(4.))
                                    .rounded(px(6.))
                                    .items_center()
                                    .cursor_pointer()
                                    .when(queue_open, |d| d.bg(p.sidebar_new_fill_hover))
                                    .hover(|d| d.bg(p.sidebar_new_fill_hover))
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .line_height(px(12.))
                                            .text_color(if queue_open { p.ink } else { p.quiet_ink })
                                            .child(format!("{queued_total}")),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.5))
                                            .line_height(px(12.))
                                            .text_color(if queue_open {
                                                p.quiet_ink
                                            } else {
                                                p.sidebar_group_empty_ink
                                            })
                                            .child("queued"),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.open_popup =
                                            if this.open_popup == OpenPopup::QueuePanel {
                                                OpenPopup::None
                                            } else {
                                                OpenPopup::QueuePanel
                                            };
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    pub(crate) fn render_composer_band(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = self.palette();
        let composer_focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let cur_model = self.roster.current_model();
        let model_chip_label = cur_model.label;
        let effort_badge = (!cur_model.thinking.is_empty() && cur_model.thinking != "off")
            .then(|| thinking_level_label(&cur_model.thinking).to_string());

        let ctx_pct = self
            .roster
            .stats()
            .filter(|st| st.context_window > 0)
            .map(|st| format!("{:.0}%", st.percent))
            .unwrap_or_else(|| "0%".to_string());

        let handle_color = if self.composer_model.is_dragging() {
            p.working
        } else if self.composer_model.is_handle_hovered() {
            p.quiet_ink
        } else {
            p.composer_handle
        };

        div()
            .w_full()
            .px_6()
            .pb(px(12.))
            .pt(px(2.))
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(CHAT_COLUMN_MAX_WIDTH))
                    .mx_auto()
                    .children(self.render_busy_strip(cx))
                    .child(
                        v_flex()
                            .relative()
                            .w_full()
                            .rounded(px(12.))
                            .border_1()
                            .border_color(if composer_focused {
                                p.composer_edge_focus
                            } else {
                                p.composer_edge
                            })
                            .bg(p.composer_surface)
                            .overflow_hidden()
                    .children(self.render_activity_sheet(cx))
                    // Interactive resize grab handle at top center (unified with COMPOSER_MIN_HEIGHT)
                    .child(
                        div()
                            .id("composer-resize-handle")
                            .w_full()
                            .h(px(12.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_ns_resize()
                            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                                this.composer_model.set_handle_hovered(*hovered);
                                cx.notify();
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if ev.click_count >= 2 {
                                        this.composer_model.reset_height();
                                    } else {
                                        let measured = this
                                            .composer
                                            .read(cx)
                                            .input_bounds()
                                            .size
                                            .height
                                            .as_f32();
                                        this.composer_model
                                            .begin_drag(ev.position.y.as_f32(), measured);
                                    }
                                    cx.notify();
                                }),
                            )
                            .child(
                                div()
                                    .w(px(36.))
                                    .h(px(3.))
                                    .rounded(px(1.5))
                                    .bg(handle_color),
                            ),
                    )
                    .when(!self.composer_model.attachments().is_empty(), |card| {
                        card.child(
                            h_flex()
                                .w_full()
                                .px(px(12.))
                                .pb(px(6.))
                                .gap(px(8.))
                                .flex_wrap()
                                .children(self.composer_model.attachments().iter().enumerate().map(
                                    |(idx, att)| {
                                        div()
                                            .id(("att-thumb", idx))
                                            .relative()
                                            .size(px(44.))
                                            .rounded(px(8.))
                                            .border_1()
                                            .border_color(p.composer_edge)
                                            .bg(p.sidebar_group_surface)
                                            .overflow_hidden()
                                            .cursor_pointer()
                                            .hover(|d| d.border_color(p.danger))
                                            .child(
                                                img(att.image.clone())
                                                    .size_full()
                                                    .object_fit(ObjectFit::Cover),
                                            )
                                            .child(
                                                div()
                                                    .absolute()
                                                    .top(px(2.))
                                                    .right(px(2.))
                                                    .size(px(14.))
                                                    .rounded_full()
                                                    .bg(p.panel_surface)
                                                    .flex()
                                                    .items_center()
                                                    .justify_center()
                                                    .child(
                                                        Icon::new(IconName::Close)
                                                            .size(px(9.))
                                                            .text_color(p.ink),
                                                    ),
                                            )
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                if this.composer_model.remove_attachment(idx) {
                                                    this.sync_stop_target(cx);
                                                    cx.notify();
                                                }
                                            }))
                                    },
                                )),
                        )
                    })
                    .child(
                        div()
                            .w_full()
                            .px(px(10.))
                            .pb(px(6.))
                            .child(
                                div()
                                    .w_full()
                                    .min_h(px(COMPOSER_MIN_HEIGHT))
                                    .child({
                                        let view = cx.entity();
                                        let ta = Textarea::new(&self.composer)
                                            .appearance(false)
                                            .bordered(false)
                                            .on_paste(move |item, _, cx| {
                                                let found = Attachment::from_clipboard_item(item);
                                                if found.is_empty() {
                                                    return false;
                                                }
                                                view.update(cx, |this, cx| {
                                                    this.composer_model.push_attachments(found);
                                                    this.sync_stop_target(cx);
                                                    cx.notify();
                                                });
                                                true
                                            });
                                        match self.composer_model.explicit_height() {
                                            Some(h) => ta.h(px(h.max(COMPOSER_MIN_HEIGHT))),
                                            None => ta,
                                        }
                                    }),
                            ),
                    )
                    // Pixel-aligned bottom dock: all controls share centered 26px row alignment & 7px chip radius
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(36.))
                            .px(px(12.))
                            .pb(px(8.))
                            .gap(px(8.))
                            .items_center()
                            .child(
                                div()
                                    .id("attach-btn")
                                    .size(px(26.))
                                    .flex_shrink_0()
                                    .rounded(px(7.))
                                    .bg(p.composer_attach_fill)
                                    .hover(|d| d.bg(p.sidebar_row_selected))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(px(13.))
                                            .flex_shrink_0()
                                            .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        let rx = cx.prompt_for_paths(PathPromptOptions {
                                            files: true,
                                            directories: false,
                                            multiple: true,
                                            prompt: Some("Attach images".into()),
                                        });
                                        let tx = this.attach_tx.clone();
                                        cx.spawn(async move |_, cx| {
                                            let Ok(Ok(Some(paths))) = rx.await else {
                                                return;
                                            };
                                            let picked = cx
                                                .background_spawn(async move {
                                                    paths
                                                        .into_iter()
                                                        .filter_map(|p| std::fs::read(p).ok())
                                                        .filter_map(Attachment::from_bytes)
                                                        .collect::<Vec<_>>()
                                                })
                                                .await;
                                            if !picked.is_empty() {
                                                let _ = tx.try_send(picked);
                                            }
                                        })
                                        .detach();
                                    })),
                            )
                            .child(self.render_activity_chip(cx))
                            .when(self.composer_model.is_editing_queue(), |row| {
                                row.child(
                                    h_flex()
                                        .flex_shrink_0()
                                        .gap(px(4.))
                                        .px(px(6.))
                                        .h(px(20.))
                                        .rounded(px(5.))
                                        .bg(p.sidebar_row_selected)
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(10.5))
                                                .whitespace_nowrap()
                                                .text_color(p.quiet_ink)
                                                .child("Editing queued · Esc"),
                                        ),
                                )
                            })
                            .child(div().flex_1().min_w(px(12.)))
                            .child(
                                div()
                                    .id("ctx-ring-btn")
                                    .h(px(26.))
                                    .flex_shrink_0()
                                    .px(px(9.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(if self.open_popup == OpenPopup::ContextCard {
                                        p.working
                                    } else {
                                        p.composer_edge
                                    })
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .text_size(px(11.5))
                                    .line_height(px(14.))
                                    .whitespace_nowrap()
                                    .text_color(p.quiet_ink)
                                    .child(ctx_pct)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.open_popup =
                                            if this.open_popup == OpenPopup::ContextCard {
                                                OpenPopup::None
                                            } else {
                                                OpenPopup::ContextCard
                                            };
                                        cx.notify();
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("model-chip-btn")
                                    .h(px(26.))
                                    .min_w_0()
                                    .overflow_hidden()
                                    .px(px(10.))
                                    .gap(px(6.))
                                    .rounded(px(7.))
                                    .border_1()
                                    .border_color(if self.open_popup == OpenPopup::ModelMenu {
                                        p.working
                                    } else {
                                        p.composer_edge
                                    })
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .whitespace_nowrap()
                                            .text_ellipsis()
                                            .text_size(px(11.5))
                                            .line_height(px(14.))
                                            .text_color(p.quiet_ink)
                                            .child(model_chip_label),
                                    )
                                    .when_some(effort_badge, |chip, eff| {
                                        chip.child(
                                            div()
                                                .flex_shrink_0()
                                                .whitespace_nowrap()
                                                .text_size(px(11.))
                                                .line_height(px(14.))
                                                .text_color(p.working)
                                                .child(eff),
                                        )
                                    })
                                    .child(
                                        Icon::new(if self.open_popup == OpenPopup::ModelMenu {
                                            IconName::ChevronUp
                                        } else {
                                            IconName::ChevronDown
                                        })
                                        .size(px(11.5))
                                        .flex_shrink_0()
                                        .text_color(p.quiet_ink),
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.hovered_model = None;
                                        this.open_popup =
                                            if this.open_popup == OpenPopup::ModelMenu {
                                                OpenPopup::None
                                            } else {
                                                OpenPopup::ModelMenu
                                            };
                                        cx.notify();
                                    })),
                            )
                            .child(self.render_send_stop_button(cx)),
                    ),
            )
            )
    }

    pub(crate) fn render_context_card_content(&self) -> impl IntoElement {
        let p = self.palette();
        let (used_str, pct_str, fill, msgs, tools, inp, outp, cached, cost) = match self.roster.stats() {
            Some(st) => {
                let fill = if st.context_window > 0 {
                    (st.context_tokens as f32 / st.context_window as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (
                    format!("{} / {}", st.context_tokens, st.context_window),
                    format!("{:.0}%", st.percent),
                    fill,
                    st.messages.to_string(),
                    st.tool_calls.to_string(),
                    st.input_tokens.to_string(),
                    st.output_tokens.to_string(),
                    st.cache_read.to_string(),
                    format!("${:.4}", st.cost),
                )
            }
            None => (
                "0 / 0".to_string(),
                "0%".to_string(),
                0.0,
                "0".to_string(),
                "0".to_string(),
                "0".to_string(),
                "0".to_string(),
                "0".to_string(),
                "$0.0000".to_string(),
            ),
        };

        let ledger = |label: &'static str, val: String| {
            h_flex()
                .justify_between()
                .text_xs()
                .child(div().text_color(p.quiet_ink).child(label))
                .child(div().text_color(p.ink).child(val))
        };

        v_flex()
            .gap_1p5()
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(div().text_xs().text_color(p.quiet_ink).child("context"))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(p.ink)
                            .child(pct_str),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(4.))
                    .rounded(px(2.))
                    .bg(p.sidebar_row_hover)
                    .child(
                        div()
                            .w(relative(fill))
                            .h_full()
                            .rounded(px(2.))
                            .bg(p.working),
                    ),
            )
            .child(div().text_xs().text_color(p.quiet_ink).child(used_str))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("messages", msgs))
            .child(ledger("tools", tools))
            .child(ledger("input", inp))
            .child(ledger("output", outp))
            .child(ledger("cache read", cached))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("session cost", cost))
    }

    /// Compute `(left_edge_px, right_inset_px)` of the centered composer column in window coordinates.
    pub(crate) fn composer_column_insets(&self, window: &Window) -> (f32, f32) {
        let viewport_w = window.viewport_size().width.as_f32();
        let sidebar_w = self.sidebar_width();
        let right_w = self.right_column_width(window);
        let center_w = (viewport_w - sidebar_w - right_w).max(0.0);
        let col_w = (center_w - 48.0).clamp(0.0, CHAT_COLUMN_MAX_WIDTH);
        let gutter = ((center_w - col_w) * 0.5).max(24.0);
        (sidebar_w + gutter, right_w + gutter)
    }

    pub(crate) fn render_slash_overlay(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let matches = self.active_slash_matches(cx)?;
        let active_idx = self
            .composer_model
            .slash_index()
            .min(matches.len().saturating_sub(1));
        let p = self.palette();
        let (composer_left, _) = self.composer_column_insets(window);
        let (strip_h, _, _, _) = self.composer_model.busy_strip_metrics(self.is_streaming);
        let attach_h = if self.composer_model.attachments().is_empty() {
            0.0
        } else {
            52.0
        };
        let popup_bottom =
            12.0 + 12.0 + self.effective_composer_height(cx) + attach_h + 42.0 + strip_h + 6.0;

        let mut list = v_flex()
            .id("floating-slash-menu")
            .absolute()
            .bottom(px(popup_bottom))
            .left(px(composer_left))
            .w(px(440.))
            .p_1p5()
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_lg()
            .gap_0p5()
            .on_click(|_, _, cx| cx.stop_propagation());

        for (i, cmd) in matches.into_iter().enumerate() {
            let selected = i == active_idx;
            let cmd_name = cmd.name;
            let cmd_hint = cmd.hint;
            let cmd_desc = cmd.description;
            list = list.child(
                h_flex()
                    .id(("slash-opt", i))
                    .h(px(28.))
                    .px_2p5()
                    .gap_2p5()
                    .items_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(p.sidebar_row_selected))
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(p.working)
                            .child(format!("/{cmd_name}")),
                    )
                    .when(!cmd_hint.is_empty(), |row| {
                        row.child(
                            div()
                                .text_size(px(11.))
                                .text_color(p.sidebar_group_empty_ink)
                                .child(cmd_hint),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(if selected { p.ink } else { p.quiet_ink })
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(cmd_desc),
                    )
                    .when(selected, |row| {
                        row.child(
                            div()
                                .h(px(17.))
                                .px(px(5.5))
                                .pb(px(0.5))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(p.composer_edge)
                                .flex()
                                .items_center()
                                .justify_center()
                                .text_size(px(10.))
                                .line_height(px(11.))
                                .text_color(p.quiet_ink)
                                .child("Tab"),
                        )
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.accept_slash_completion(i, window, cx);
                    })),
            );
        }

        Some(list.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn busy_strip_helpers_resolve_labels_spinner_and_geometry() {
        assert_eq!(
            resolve_busy_label(Some("compact"), "Thinking", false),
            "Compacting"
        );
        assert_eq!(
            resolve_busy_label(Some("refine"), "Writing", false),
            "Refining"
        );
        assert_eq!(
            resolve_busy_label(Some("refine"), "Compacting", true),
            "Compacting"
        );
        assert_eq!(resolve_busy_label(None, "Thinking", false), "Thinking");

        assert_eq!(spinner_frame_for_ms(0), SPINNER_FRAMES[0]);
        assert_eq!(spinner_frame_for_ms(80), SPINNER_FRAMES[1]);
        assert_eq!(spinner_frame_for_ms(800), SPINNER_FRAMES[0]);

        let (h0, o0) = busy_strip_geometry(0.0, true);
        let (h_mid, o_mid) = busy_strip_geometry(0.5, true);
        let (h1, o1) = busy_strip_geometry(1.0, true);
        assert_eq!(h0, 0.0);
        assert_eq!(o0, 0.0);
        assert!(h_mid > BUSY_STRIP_HEIGHT * 0.5 && h_mid < BUSY_STRIP_HEIGHT);
        assert!(o_mid > 0.0 && o_mid < 1.0);
        assert_eq!(h1, BUSY_STRIP_HEIGHT);
        assert_eq!(o1, 1.0);

        let ev = json!({
            "type": "session_action_update",
            "actions": {
                "steering": ["fix tests"],
                "followUps": ["then commit", {"text": "and push"}]
            }
        });
        let (st, fu) = parse_action_update_lanes(&ev);
        assert_eq!(st, vec!["fix tests"]);
        assert_eq!(fu, vec!["then commit", "and push"]);
    }

    #[::core::prelude::v1::test]
    fn clipboard_paste_transcodes_windows_bmp_to_png_with_dimensions() {
        let img = image::RgbaImage::from_pixel(40, 20, image::Rgba([12, 34, 56, 255]));
        let mut bmp = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut std::io::Cursor::new(&mut bmp), image::ImageFormat::Bmp)
            .expect("encode bmp");

        let item = ClipboardItem {
            entries: vec![
                ClipboardEntry::Image(Image::from_bytes(ImageFormat::Bmp, bmp)),
                ClipboardEntry::String(ClipboardString::new("ignored caption".into())),
            ],
        };
        let found = Attachment::from_clipboard_item(&item);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].mime, "image/png");
        assert_eq!((found[0].width, found[0].height), (40.0, 20.0));
        assert!(found[0].image.bytes.starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
