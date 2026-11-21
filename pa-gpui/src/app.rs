use crate::assets::{
    ICON_BASH_CHEVRON_PATH, ICON_BASH_CURSOR_PATH, ICON_BASH_PATH, ICON_HEARTBEAT_PATH,
    ICON_PYTHON_L_PATH, ICON_PYTHON_M_PATH, ICON_PYTHON_PATH, ICON_PYTHON_R_PATH, ICON_SEND_PATH,
    ICON_SHELLS_PATH, ICON_SUBAGENTS_PATH, ICON_THINKING_PATH, PRIME_AGENT_MARK_DARK_PATH,
    PRIME_AGENT_MARK_LIGHT_PATH,
};
use crate::daemon::{DaemonBridge, DaemonCmd, DaemonEvent};
use crate::settings::AppSettings;
use crate::theme::PaPalette;
use crate::transcript::{
    append_text_delta, append_thinking_delta, build_items, compute_tail_trim,
    find_tail_merge_start, finish_tool_call, format_expanded_body, hydrate_markdown, settle_all,
    start_tool_call, CardVariant, ItemKind, TranscriptItem,
};
use base64::Engine as _;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    scroll::ScrollableElement as _,
    text::{TextView, TextViewState},
    v_flex, Icon, IconName, Sizable as _, StyledExt as _, Theme, ThemeMode, TitleBar,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::{
    activity_label, clamp_level, thinking_level_label, ModelEntry, SessionRow, SessionState,
    SessionStats,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone)]
struct Attachment {
    mime: String,
    base64: String,
    image: Arc<Image>,
}

impl Attachment {
    fn from_bytes(bytes: Vec<u8>) -> Option<Self> {
        let (mime, format) = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            ("image/png", ImageFormat::Png)
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            ("image/jpeg", ImageFormat::Jpeg)
        } else if bytes.len() > 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            ("image/webp", ImageFormat::Webp)
        } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
            ("image/gif", ImageFormat::Gif)
        } else {
            return None;
        };
        let base64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        let image = Arc::new(Image::from_bytes(format, bytes));
        Some(Self {
            mime: mime.to_string(),
            base64,
            image,
        })
    }

    fn block(&self) -> Value {
        json!({
            "type": "image",
            "data": self.base64,
            "mimeType": self.mime,
        })
    }
}

fn pick_image_files() -> Vec<Attachment> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let script = "$ErrorActionPreference='Stop'; Add-Type -AssemblyName System.Windows.Forms; \
             $d=New-Object System.Windows.Forms.OpenFileDialog; \
             $d.Multiselect=$true; \
             $d.Filter='Images|*.png;*.jpg;*.jpeg;*.webp;*.gif|All files|*.*'; \
             if ($d.ShowDialog() -eq [System.Windows.Forms.DialogResult]::OK) { \
                 $d.FileNames | ForEach-Object { Write-Output $_ } }";
        let Ok(output) = std::process::Command::new("powershell")
            .args([
                "-STA",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                script,
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
        else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .filter_map(|line| std::fs::read(Path::new(line)).ok())
            .filter_map(Attachment::from_bytes)
            .collect()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

fn provider_label(provider: &str) -> String {
    provider.replace('_', " ")
}

fn model_label(name: &str, id: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return id.to_string();
    }
    match trimmed.rfind(" (") {
        Some(at) if trimmed.ends_with(')') => trimmed[..at].to_string(),
        _ => trimmed.to_string(),
    }
}

fn context_hint(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{}M", tokens / 1_000_000)
    } else if tokens >= 1_000 {
        format!("{}k", tokens / 1_000)
    } else {
        String::new()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SlashAction {
    SendMessage,
    SendDaemonCommand,
    Model,
    Settings,
    NewChat,
    Rename(String),
    Export,
    Copy,
    Quit,
    NotWired(&'static str),
}

fn is_supported_slash_cmd(cmd: &pa_client::commands::Command) -> bool {
    cmd.session_side
        || matches!(
            cmd.name,
            "model" | "effort" | "settings" | "new" | "name" | "export" | "copy" | "quit"
        )
}

fn slash_dispatch(text: &str) -> SlashAction {
    let trimmed = text.trim();
    let Some(body) = trimmed.strip_prefix('/') else {
        return SlashAction::SendMessage;
    };
    let name = body.split_whitespace().next().unwrap_or("");
    let rest = body[name.len()..].trim().to_string();
    let Some(command) = pa_client::commands::find(name) else {
        return SlashAction::SendMessage;
    };
    if command.session_side {
        return SlashAction::SendDaemonCommand;
    }
    match name {
        "model" | "effort" => SlashAction::Model,
        "settings" => SlashAction::Settings,
        "new" => SlashAction::NewChat,
        "name" => SlashAction::Rename(rest),
        "export" => SlashAction::Export,
        "copy" => SlashAction::Copy,
        "quit" => SlashAction::Quit,
        _ => SlashAction::NotWired(command.name),
    }
}

#[derive(Clone)]
struct RenameTarget {
    key: String,
    id: String,
    file: String,
    is_resident: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SidebarGroup {
    Active,
    Idle,
    Done,
}

#[derive(Clone, PartialEq, Eq)]
enum OpenPopup {
    None,
    ModelMenu,
    SettingsMenu,
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

fn lerp_hsla(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let t = t.clamp(0.0, 1.0);
    let ra: Rgba = a.into();
    let rb: Rgba = b.into();
    Rgba {
        r: ra.r + (rb.r - ra.r) * t,
        g: ra.g + (rb.g - ra.g) * t,
        b: ra.b + (rb.b - ra.b) * t,
        a: ra.a + (rb.a - ra.a) * t,
    }
    .into()
}

fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn python_ripple(phase_ms: u64, offset_ms: u64) -> f32 {
    let pos = ((phase_ms + 1600 - offset_ms) % 1600) as f32 / 1600.0;
    0.5 - 0.5 * (std::f32::consts::TAU * pos).cos()
}

fn bash_cursor_mix(phase_ms: u64) -> f32 {
    let cycle = (phase_ms % 1200) as f32 / 1200.0;
    if cycle <= 0.4 {
        1.0
    } else if cycle <= 0.55 {
        0.5 + 0.5 * (std::f32::consts::PI * (cycle - 0.4) / 0.15).cos()
    } else if cycle <= 0.85 {
        0.0
    } else {
        0.5 - 0.5 * (std::f32::consts::PI * (cycle - 0.85) / 0.15).cos()
    }
}

fn thinking_sweep(phase_ms: u64) -> f32 {
    let pos = (phase_ms % 1450) as f32 / 1450.0;
    0.5 - 0.5 * (std::f32::consts::PI * pos).cos()
}

fn pulse_mix(phase_ms: u64) -> f32 {
    let pos = (phase_ms % 1400) as f32 / 1400.0;
    0.5 - 0.5 * (std::f32::consts::TAU * pos).cos()
}

fn effort_ladder_selected_level(
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

const OLDER_PAGE_LIMIT: usize = 80;
const LOAD_OLDER_THRESHOLD: usize = 10;
const DEHYDRATE_RADIUS: usize = 50;
const MAX_TAIL_ITEMS: usize = 220;
const KEEP_TAIL_ITEMS: usize = 140;
const BUSY_STRIP_HEIGHT: f32 = 26.0;
const SPINNER_FRAMES: [&str; 10] = [
    "\u{280b}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}",
    "\u{2827}", "\u{2807}", "\u{280f}",
];

fn spinner_frame_for_ms(phase_ms: u64) -> &'static str {
    SPINNER_FRAMES[((phase_ms / 80) as usize) % SPINNER_FRAMES.len()]
}

fn busy_strip_geometry(mix: f32, opening: bool) -> (f32, f32) {
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

fn resolve_busy_label<'a>(pending_slash: Option<&str>, incoming: &'a str, force: bool) -> &'a str {
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

fn parse_action_update_lanes(event: &Value) -> (Vec<String>, Vec<String>) {
    let lane = |name: &str| -> Vec<String> {
        event
            .get("actions")
            .and_then(|actions| actions.get(name))
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|item| {
                        item.as_str()
                            .map(str::to_string)
                            .or_else(|| item.get("text").and_then(Value::as_str).map(str::to_string))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    (lane("steering"), lane("followUps"))
}

pub struct PaApp {
    bridge: DaemonBridge,
    status: SharedString,
    notice: Option<SharedString>,
    sessions: Vec<SessionRow>,
    models: Vec<ModelEntry>,
    session_state: Option<SessionState>,
    stats: Option<SessionStats>,
    queued_steer: Vec<String>,
    queued_follow_up: Vec<String>,
    editing_queue: Option<(String, usize, String)>,
    active_session_id: Option<String>,
    renaming: Option<RenameTarget>,
    items: Vec<TranscriptItem>,
    next_item_id: usize,
    oldest_entry_id: Option<String>,
    has_older: bool,
    loading_older: bool,
    pending_trim_tail: bool,
    last_dehydrate_center: usize,
    scrolled_up: bool,
    is_streaming: bool,
    pending_slash_cmd: Option<&'static str>,
    busy_label: String,
    busy_since: Option<Instant>,
    busy_strip_mix: f32,
    draft_chat: bool,
    draft_model_picked: bool,
    rail_mark_hovered: bool,
    // Smooth Send <-> Stop morph state (0.0 = Send arrow, 1.0 = Stop square)
    stop_mix: f32,
    stop_target: f32,
    last_anim_tick: Instant,
    anim_epoch: Instant,
    anim_phase_ms: u64,
    // Interactive composer drag-resize state
    composer_height: Option<f32>,
    dragging_composer: bool,
    drag_start_y: f32,
    drag_start_height: f32,
    handle_hovered: bool,
    settings: AppSettings,
    subagent_count: usize,
    shell_count: usize,
    heartbeat_count: usize,
    search_open: bool,
    find_open: bool,
    find_index: usize,
    open_popup: OpenPopup,
    collapsed_providers: HashSet<String>,
    hovered_model: Option<(String, String, f32)>,
    attachments: Vec<Attachment>,
    attach_tx: async_channel::Sender<Vec<Attachment>>,
    list_state: ListState,
    search_input: Entity<InputState>,
    find_input: Entity<InputState>,
    rename_input: Entity<InputState>,
    composer: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
    _daemon_task: Task<()>,
    _attach_task: Task<()>,
}

impl PaApp {
    pub fn new(settings: AppSettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (bridge, ev_rx) = DaemonBridge::spawn();
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in transcript…"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New session name…"));
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask anything… (/ for commands · Shift+Enter for newline)")
                .soft_wrap(true)
                .auto_grow(1, 8)
                .submit_on_enter(true)
        });

        let sub_composer = cx.subscribe_in(&composer, window, |this, _, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter {
                    secondary,
                    shift: false,
                } => {
                    // Plain Enter sends (or steers if streaming); Ctrl+Enter queues follow-up when streaming.
                    let steer = this.is_streaming && !*secondary;
                    this.send_current_prompt(steer, window, cx);
                }
                InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {
                    this.sync_stop_target(cx);
                    cx.notify();
                }
                _ => {}
            }
        });

        let sub_search = cx.subscribe_in(&search_input, window, |_this, _, ev: &InputEvent, _, cx| {
            if matches!(ev, InputEvent::Change | InputEvent::Focus | InputEvent::Blur) {
                cx.notify();
            }
        });

        let sub_find = cx.subscribe_in(&find_input, window, |this, _, ev: &InputEvent, _, cx| {
            match ev {
                InputEvent::Change => {
                    this.find_index = 0;
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => {
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    this.step_find(1, cx);
                }
            }
        });

        let sub_rename = cx.subscribe_in(&rename_input, window, |this, _, ev: &InputEvent, window, cx| {
            match ev {
                InputEvent::PressEnter {
                    secondary: false,
                    shift: false,
                } => {
                    this.commit_rename(window, cx);
                }
                InputEvent::Focus | InputEvent::Blur => {
                    cx.notify();
                }
                _ => {}
            }
        });

        let daemon_task = cx.spawn(async move |weak_self, cx| {
            while let Ok(first) = ev_rx.recv().await {
                let mut batch = vec![first];
                while let Ok(ev) = ev_rx.try_recv() {
                    batch.push(ev);
                }
                let keep_going = weak_self
                    .update(cx, |this, cx| {
                        for ev in batch {
                            this.handle_daemon_event(ev, cx);
                        }
                        cx.notify();
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        });

        let (attach_tx, attach_rx) = async_channel::unbounded::<Vec<Attachment>>();
        let attach_rx_task = attach_rx.clone();
        let attach_task = cx.spawn(async move |weak_self, cx| {
            while let Ok(found) = attach_rx_task.recv().await {
                let keep_going = weak_self
                    .update(cx, |this, cx| {
                        if !found.is_empty() {
                            this.attachments.extend(found);
                            this.sync_stop_target(cx);
                            cx.notify();
                        }
                    })
                    .is_ok();
                if !keep_going {
                    break;
                }
            }
        });

        // Top alignment keeps short transcripts anchored to the top of the pane while
        // FollowMode::Tail pins overflowing and streaming transcripts to the bottom.
        // 600px overdraw pre-measures items well before they enter the viewport to avoid jumps.
        let list_state = ListState::new(0, ListAlignment::Top, px(600.));
        list_state.set_follow_mode(FollowMode::Tail);
        let weak_scroll = cx.entity().downgrade();
        list_state.set_scroll_handler(move |ev, _window, cx| {
            let _ = weak_scroll.update(cx, |this, cx| {
                this.on_transcript_scroll(ev, cx);
            });
        });

        Self {
            bridge,
            status: "Connecting…".into(),
            notice: None,
            sessions: Vec::new(),
            models: Vec::new(),
            session_state: None,
            stats: None,
            queued_steer: Vec::new(),
            queued_follow_up: Vec::new(),
            editing_queue: None,
            active_session_id: None,
            renaming: None,
            items: Vec::new(),
            next_item_id: 1,
            oldest_entry_id: None,
            has_older: false,
            loading_older: false,
            pending_trim_tail: false,
            last_dehydrate_center: 0,
            scrolled_up: false,
            is_streaming: false,
            pending_slash_cmd: None,
            busy_label: String::new(),
            busy_since: None,
            busy_strip_mix: 0.0,
            draft_chat: false,
            draft_model_picked: false,
            rail_mark_hovered: false,
            stop_mix: 0.0,
            stop_target: 0.0,
            last_anim_tick: Instant::now(),
            anim_epoch: Instant::now(),
            anim_phase_ms: 0,
            composer_height: None,
            dragging_composer: false,
            drag_start_y: 0.0,
            drag_start_height: 54.0,
            handle_hovered: false,
            settings,
            subagent_count: 0,
            shell_count: 0,
            heartbeat_count: 0,
            search_open: false,
            find_open: false,
            find_index: 0,
            open_popup: OpenPopup::None,
            collapsed_providers: HashSet::new(),
            hovered_model: None,
            attachments: Vec::new(),
            attach_tx,
            list_state,
            search_input,
            find_input,
            rename_input,
            composer,
            _subscriptions: vec![sub_composer, sub_search, sub_find, sub_rename],
            _daemon_task: daemon_task,
            _attach_task: attach_task,
        }
    }

    fn palette(&self) -> PaPalette {
        PaPalette::for_mode(self.settings.dark_mode)
    }

    fn busy_strip_wanted(&self) -> bool {
        self.is_streaming || !self.queued_steer.is_empty() || !self.queued_follow_up.is_empty()
    }

    fn set_busy_label(&mut self, label: &str, force: bool) {
        let next = resolve_busy_label(self.pending_slash_cmd, label, force);
        if self.busy_label != next {
            self.busy_label = next.to_string();
            self.busy_since = Some(Instant::now());
        } else if self.busy_since.is_none() {
            self.busy_since = Some(Instant::now());
        }
    }

    fn sync_stop_target(&mut self, cx: &Context<Self>) {
        if self.is_streaming {
            if self.busy_label.is_empty() {
                self.set_busy_label("Working", false);
            } else if self.busy_since.is_none() {
                self.busy_since = Some(Instant::now());
            }
        } else {
            self.busy_label.clear();
            self.busy_since = None;
        }
        let has_content = !self.composer.read(cx).value().trim().is_empty()
            || !self.attachments.is_empty();
        let next = if self.is_streaming && !has_content {
            1.0
        } else {
            0.0
        };
        if next != self.stop_target {
            self.stop_target = next;
            self.last_anim_tick = Instant::now();
        }
    }

    fn advance_stop_anim(&mut self, window: &Window) {
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_anim_tick)
            .as_secs_f32();
        self.last_anim_tick = now;
        let mut animating = false;
        if (self.stop_mix - self.stop_target).abs() > 0.002 {
            let step = (dt / 0.22).min(0.25);
            if self.stop_mix < self.stop_target {
                self.stop_mix = (self.stop_mix + step).min(self.stop_target);
            } else {
                self.stop_mix = (self.stop_mix - step).max(self.stop_target);
            }
            animating = true;
        } else if self.stop_mix != self.stop_target {
            self.stop_mix = self.stop_target;
        }

        let busy_target = if self.busy_strip_wanted() { 1.0 } else { 0.0 };
        if (self.busy_strip_mix - busy_target).abs() > 0.002 {
            let duration = if busy_target > self.busy_strip_mix {
                0.19
            } else {
                0.11
            };
            let step = (dt / duration).min(0.25);
            if self.busy_strip_mix < busy_target {
                self.busy_strip_mix = (self.busy_strip_mix + step).min(busy_target);
            } else {
                self.busy_strip_mix = (self.busy_strip_mix - step).max(busy_target);
            }
            animating = true;
        } else if self.busy_strip_mix != busy_target {
            self.busy_strip_mix = busy_target;
        }

        if animating {
            window.request_animation_frame();
        }
    }

    fn request_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Carry over the current model/thinking so the blank draft chat stays ready to pick or send.
        let (provider, model_id, _, thinking) = self.current_model();
        if !provider.is_empty() && !model_id.is_empty() {
            let st = self.session_state.get_or_insert_with(SessionState::default);
            st.provider = provider;
            st.model_id = model_id;
            st.thinking_level = thinking;
            st.is_streaming = false;
        }
        self.open_popup = OpenPopup::None;
        self.hovered_model = None;
        self.draft_chat = true;
        self.draft_model_picked = false;
        self.active_session_id = None;
        self.is_streaming = false;
        self.pending_slash_cmd = None;
        self.oldest_entry_id = None;
        self.has_older = false;
        self.loading_older = false;
        self.pending_trim_tail = false;
        self.scrolled_up = false;
        self.items.clear();
        self.list_state.reset(0);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.stats = None;
        self.queued_steer.clear();
        self.queued_follow_up.clear();
        self.editing_queue = None;
        self.subagent_count = 0;
        self.shell_count = 0;
        self.heartbeat_count = 0;
        self.notice = None;
        self.sync_stop_target(cx);
        let focus = self.composer.read(cx).focus_handle(cx);
        window.focus(&focus, cx);
        cx.notify();
    }

    fn is_scrolled_up(&self) -> bool {
        if self.items.is_empty() || self.list_state.is_following_tail() {
            return false;
        }
        let max_y = self.list_state.max_offset_for_scrollbar().y;
        if max_y <= px(4.) {
            return false;
        }
        let cur_y = -self.list_state.scroll_px_offset_for_scrollbar().y;
        (max_y - cur_y) > px(80.)
    }

    fn assign_item_ids(&mut self, items: &mut [TranscriptItem]) {
        for item in items.iter_mut() {
            item.id = self.next_item_id;
            self.next_item_id = self.next_item_id.wrapping_add(1);
        }
    }

    fn request_older_history(&mut self) {
        if !self.has_older || self.loading_older {
            return;
        }
        let (Some(sid), Some(before)) = (
            self.active_session_id.clone(),
            self.oldest_entry_id.clone(),
        ) else {
            return;
        };
        self.loading_older = true;
        let _ = self.bridge.tx.send(DaemonCmd::LoadOlder {
            session: sid,
            before,
            limit: OLDER_PAGE_LIMIT,
        });
    }

    fn dehydrate_far_items(&mut self, visible_start: usize, visible_end: usize) {
        let keep_lo = visible_start.saturating_sub(DEHYDRATE_RADIUS);
        let keep_hi = visible_end.saturating_add(DEHYDRATE_RADIUS);
        for (ix, item) in self.items.iter_mut().enumerate() {
            if (ix < keep_lo || ix > keep_hi) && !item.active && item.body_md.is_some() {
                item.body_md = None;
            }
        }
    }

    /// Safely unload deep history once the user returns to the bottom of the transcript.
    /// Called outside `ListState::scroll_handler` so `splice` never re-enters `ListState`.
    fn trim_tail_window(&mut self) {
        self.pending_trim_tail = false;
        if !self.list_state.is_following_tail() || self.loading_older {
            return;
        }
        if let Some((drop_count, new_oldest)) =
            compute_tail_trim(&self.items, MAX_TAIL_ITEMS, KEEP_TAIL_ITEMS)
        {
            self.items.drain(0..drop_count);
            self.oldest_entry_id = Some(new_oldest);
            self.has_older = true;
            self.list_state.splice(0..drop_count, 0);
        }
    }

    fn on_transcript_scroll(&mut self, ev: &ListScrollEvent, cx: &mut Context<Self>) {
        // NOTE: `ListState` holds a mutable borrow while invoking `scroll_handler`, so
        // never call `self.list_state.*` synchronously here.
        let mut notify = false;
        let near_top = ev.visible_range.start <= LOAD_OLDER_THRESHOLD;
        if near_top && self.has_older && !self.loading_older {
            self.request_older_history();
            notify = true;
        }

        let center = (ev.visible_range.start + ev.visible_range.end) / 2;
        if center.abs_diff(self.last_dehydrate_center) >= 12 {
            self.last_dehydrate_center = center;
            self.dehydrate_far_items(ev.visible_range.start, ev.visible_range.end);
        }

        if ev.is_following_tail && self.items.len() > MAX_TAIL_ITEMS && !self.loading_older {
            self.pending_trim_tail = true;
            notify = true;
        }

        let next_scrolled_up = !ev.is_following_tail
            && ev.count > 0
            && ev.visible_range.end < ev.count.saturating_sub(1);
        if next_scrolled_up != self.scrolled_up {
            self.scrolled_up = next_scrolled_up;
            notify = true;
        }

        if notify {
            cx.notify();
        }
    }

    fn sync_list_after_tail_update(&self, old_len: usize) {
        let new_len = self.items.len();
        if new_len > old_len {
            self.list_state.splice(old_len..old_len, new_len - old_len);
        } else if new_len > 0 {
            self.list_state.remeasure_items(new_len - 1..new_len);
        }
    }

    fn handle_daemon_event(&mut self, ev: DaemonEvent, cx: &mut Context<Self>) {
        match ev {
            DaemonEvent::Connected => {
                self.status = "Connected".into();
            }
            DaemonEvent::Disconnected(reason) => {
                self.status = reason.into();
                self.is_streaming = false;
                self.pending_slash_cmd = None;
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
            }
            DaemonEvent::SessionCreated(new_sid) => {
                self.draft_chat = false;
                self.draft_model_picked = false;
                self.active_session_id = Some(new_sid);
                self.notice = None;
                self.sync_stop_target(cx);
            }
            DaemonEvent::Sessions(rows) => {
                self.sessions = rows;
                if self.active_session_id.is_none() && !self.draft_chat {
                    if let Some(first) = self.sessions.first() {
                        self.active_session_id = Some(session_key(first));
                    }
                }
                if let Some(sid) = self.active_session_id.as_deref() {
                    if let Some(row) = self.sessions.iter().find(|s| session_key(s) == sid) {
                        self.is_streaming = row.is_streaming;
                    }
                }
                self.sync_stop_target(cx);
            }
            DaemonEvent::Messages {
                session,
                messages,
                first_entry,
                more,
            } => {
                let switching = self.active_session_id.as_deref() != Some(session.as_str());
                if switching {
                    self.session_state = None;
                    self.busy_label.clear();
                    self.busy_since = None;
                    self.pending_slash_cmd = None;
                    self.editing_queue = None;
                }
                self.draft_chat = false;
                self.draft_model_picked = false;
                self.active_session_id = Some(session);
                self.oldest_entry_id = (!first_entry.is_empty()).then_some(first_entry);
                self.has_older = more;
                self.loading_older = false;
                self.pending_trim_tail = false;
                self.scrolled_up = false;

                let mut next = build_items(&messages);
                self.assign_item_ids(&mut next);
                self.items = next;
                self.list_state
                    .reset_with_uniform_height(self.items.len(), px(44.));
                self.list_state.set_follow_mode(FollowMode::Tail);
            }
            DaemonEvent::TailRefresh {
                session,
                messages,
                first_entry,
                more,
            } => {
                if self.active_session_id.as_deref() != Some(session.as_str()) {
                    return;
                }
                let mut incoming = build_items(&messages);
                if let Some((ex_at, in_at)) =
                    find_tail_merge_start(&self.items, &incoming, &first_entry)
                {
                    let mut tail: Vec<TranscriptItem> = incoming.drain(in_at..).collect();
                    let old_len = self.items.len();
                    if ex_at == old_len && tail.is_empty() {
                        return;
                    }
                    // Fast path: if the suffix has the exact same visual rows (e.g. settled
                    // optimistic/streamed items gaining their daemon `entry_id` and `fingerprint`),
                    // update metadata in place without calling `list_state.splice` — zero flash!
                    let same_visual_rows = old_len.saturating_sub(ex_at) == tail.len()
                        && self.items[ex_at..old_len]
                            .iter()
                            .zip(tail.iter())
                            .all(|(prev, next)| {
                                prev.kind == next.kind
                                    && prev.call_id == next.call_id
                                    && prev.title == next.title
                                    && prev.meta == next.meta
                                    && prev.raw_body == next.raw_body
                                    && prev.failed == next.failed
                            });
                    if same_visual_rows {
                        for (prev, next) in self.items[ex_at..old_len].iter_mut().zip(tail) {
                            prev.entry_id = next.entry_id;
                            prev.fingerprint = next.fingerprint;
                            prev.active = next.active;
                        }
                        return;
                    }
                    // Preserve expanded state & already-hydrated TextViewState entities when
                    // replacing an in-flight item with its settled copy.
                    for (offset, new_item) in tail.iter_mut().enumerate() {
                        if let Some(prev) = self.items.get_mut(ex_at + offset) {
                            if prev.kind == new_item.kind && prev.call_id == new_item.call_id {
                                new_item.expanded = prev.expanded;
                                if prev.raw_body == new_item.raw_body {
                                    new_item.body_md = prev.body_md.take();
                                }
                            }
                        }
                        hydrate_markdown(new_item, cx);
                    }
                    self.assign_item_ids(&mut tail);
                    let added = tail.len();
                    self.items.splice(ex_at..old_len, tail);
                    self.list_state.splice(ex_at..old_len, added);
                    if self.list_state.is_following_tail() && self.items.len() > MAX_TAIL_ITEMS {
                        self.pending_trim_tail = true;
                    }
                } else {
                    // Branch was rewritten (e.g. after `/compact`): cleanly replace transcript.
                    self.oldest_entry_id = (!first_entry.is_empty()).then_some(first_entry);
                    self.has_older = more;
                    self.loading_older = false;
                    self.pending_trim_tail = false;
                    self.assign_item_ids(&mut incoming);
                    self.items = incoming;
                    self.list_state
                        .reset_with_uniform_height(self.items.len(), px(44.));
                    self.list_state.set_follow_mode(FollowMode::Tail);
                }
            }
            DaemonEvent::OlderMessages {
                session,
                messages,
                first_entry,
                more,
            } => {
                if self.active_session_id.as_deref() != Some(session.as_str()) {
                    return;
                }
                self.loading_older = false;
                let prev_cursor = self.oldest_entry_id.clone();
                let advanced = !first_entry.is_empty()
                    && prev_cursor.as_deref() != Some(first_entry.as_str());
                if advanced {
                    self.oldest_entry_id = Some(first_entry);
                }
                self.has_older = more && advanced;

                let mut older = build_items(&messages);
                if older.is_empty() {
                    if self.has_older {
                        self.request_older_history();
                    }
                    return;
                }
                self.assign_item_ids(&mut older);
                let added = older.len();
                older.append(&mut self.items);
                self.items = older;
                // `splice(0..0, added)` shifts `scroll_anchor.item_ix` by `added` and keeps
                // every existing item's measured height — zero viewport jump!
                self.list_state.splice(0..0, added);
            }
            DaemonEvent::Models(models) => {
                if self.models.is_empty() {
                    let (cur_prov, _, _, _) = self.current_model();
                    self.collapsed_providers
                        .extend(models.iter().map(|m| m.provider.clone()));
                    if !cur_prov.is_empty() {
                        self.collapsed_providers.remove(&cur_prov);
                    }
                }
                self.models = models;
            }
            DaemonEvent::SessionState { session, state } => {
                if self.active_session_id.as_deref() == Some(session.as_str()) {
                    if !state.provider.is_empty() {
                        self.collapsed_providers.remove(&state.provider);
                    }
                    self.is_streaming = state.is_streaming;
                    self.session_state = Some(state);
                    self.sync_stop_target(cx);
                }
            }
            DaemonEvent::Stats { session, stats } => {
                if self.active_session_id.as_deref() == Some(session.as_str()) {
                    self.stats = Some(stats);
                }
            }
            DaemonEvent::Queue {
                session,
                steer,
                follow_up,
            } => {
                if self.active_session_id.as_deref() == Some(session.as_str()) {
                    self.queued_steer = steer;
                    self.queued_follow_up = follow_up;
                    self.last_anim_tick = Instant::now();
                    if self.queued_steer.is_empty()
                        && self.queued_follow_up.is_empty()
                        && self.open_popup == OpenPopup::QueuePanel
                    {
                        self.open_popup = OpenPopup::None;
                    }
                }
            }
            DaemonEvent::Activity {
                session,
                subagents,
                shells,
                heartbeats,
            } => {
                if self.active_session_id.as_deref() == Some(session.as_str()) {
                    self.subagent_count = subagents;
                    self.shell_count = shells;
                    self.heartbeat_count = heartbeats;
                }
            }
            DaemonEvent::Notice(msg) => {
                self.notice = Some(msg.into());
            }
            DaemonEvent::LiveFrame(frame) => {
                self.handle_live_frame(&frame, cx);
            }
            DaemonEvent::Error(err) => {
                self.is_streaming = false;
                self.pending_slash_cmd = None;
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
                self.status = format!("Error: {err}").into();
                self.notice = Some(format!("Error: {err}").into());
            }
        }
    }

    fn handle_live_frame(&mut self, frame: &Value, cx: &mut Context<Self>) {
        let top_kind = frame.get("type").and_then(Value::as_str).unwrap_or_default();
        if matches!(
            top_kind,
            "roster_update" | "session_updated" | "session_closed" | "session_binding"
        ) {
            let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            return;
        }
        if top_kind == "response" {
            let cmd = frame.get("command").and_then(Value::as_str).unwrap_or("");
            if matches!(cmd, "prompt" | "steer") {
                if frame.get("success").and_then(Value::as_bool) == Some(false) {
                    self.is_streaming = false;
                    self.pending_slash_cmd = None;
                    settle_all(&mut self.items);
                    let err = frame
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Command rejected");
                    self.notice = Some(err.to_string().into());
                    self.sync_stop_target(cx);
                } else if self.pending_slash_cmd.take().is_some() {
                    self.is_streaming = false;
                    if matches!(
                        self.notice.as_deref(),
                        Some("Compacting…") | Some("Refining…")
                    ) {
                        self.notice = None;
                    }
                    settle_all(&mut self.items);
                    self.sync_stop_target(cx);
                    if let Some(sid) = self.active_session_id.clone() {
                        let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                    }
                }
            }
            return;
        }

        let frame_sid = frame
            .get("activeSessionId")
            .or_else(|| frame.get("sessionId"))
            .and_then(Value::as_str);
        let matches_active = match (frame_sid, self.active_session_id.as_deref()) {
            (Some(fs), Some(cur)) => fs == cur,
            _ => true,
        };
        if !matches_active {
            return;
        }

        let event = frame.get("event").filter(|v| v.is_object()).unwrap_or(frame);
        let kind = event.get("type").and_then(Value::as_str).unwrap_or_default();

        match kind {
            "agent_start" | "turn_start" => {
                self.is_streaming = true;
                if self.busy_label.is_empty() {
                    self.set_busy_label("Waiting", false);
                }
                if let Some(sid) = self.active_session_id.clone() {
                    if let Some(row) = self.sessions.iter_mut().find(|s| session_key(s) == sid) {
                        if row.worker_state.is_empty() {
                            row.worker_state = "ready".into();
                        }
                        row.lifecycle = "live".into();
                        row.activity = "working".into();
                        row.is_session_active = true;
                        row.is_streaming = true;
                    }
                }
                self.sync_stop_target(cx);
                let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            }
            "message_update" => {
                self.is_streaming = true;
                let delta_ev = event
                    .get("assistantMessageEvent")
                    .filter(|v| v.is_object())
                    .unwrap_or(event);
                if let Some(label) = activity_label(delta_ev) {
                    self.set_busy_label(label, false);
                }
                self.sync_stop_target(cx);
                let delta_kind = delta_ev
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if let Some(delta) = delta_ev.get("delta").and_then(Value::as_str) {
                    if delta_kind.starts_with("text_delta") {
                        let old_len = self.items.len();
                        append_text_delta(&mut self.items, delta, cx);
                        self.sync_list_after_tail_update(old_len);
                    } else if delta_kind.starts_with("thinking_delta") {
                        let old_len = self.items.len();
                        append_thinking_delta(&mut self.items, delta, cx);
                        self.sync_list_after_tail_update(old_len);
                    }
                }
            }
            "tool_execution_start" => {
                self.is_streaming = true;
                self.set_busy_label("Running", false);
                self.sync_stop_target(cx);
                let cid = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let name = event
                    .get("toolName")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let args = event.get("args").unwrap_or(&Value::Null);
                let old_len = self.items.len();
                start_tool_call(&mut self.items, cid, name, args);
                self.sync_list_after_tail_update(old_len);
            }
            "tool_execution_end" => {
                let cid = event
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_err = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let details = event
                    .get("result")
                    .and_then(|r| r.get("details"))
                    .or_else(|| event.get("details"));
                let out = event
                    .get("result")
                    .and_then(|r| {
                        r.as_str().map(str::to_string).or_else(|| {
                            r.get("content").and_then(Value::as_array).map(|arr| {
                                arr.iter()
                                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                        })
                    })
                    .unwrap_or_default();
                if let Some(ix) = finish_tool_call(&mut self.items, cid, out, is_err, details, cx) {
                    self.list_state.remeasure_items(ix..ix + 1);
                }
                if self.is_streaming {
                    self.set_busy_label("Waiting", false);
                }
            }
            "auto_retry_start" => {
                self.is_streaming = true;
                self.set_busy_label("Retrying", false);
                self.sync_stop_target(cx);
            }
            "session_action_update" => {
                let (steer, follow_up) = parse_action_update_lanes(event);
                self.queued_steer = steer;
                self.queued_follow_up = follow_up;
                self.last_anim_tick = Instant::now();
                if self.queued_steer.is_empty()
                    && self.queued_follow_up.is_empty()
                    && self.open_popup == OpenPopup::QueuePanel
                {
                    self.open_popup = OpenPopup::None;
                }
            }
            "compaction_start" => {
                self.is_streaming = true;
                self.set_busy_label("Compacting", true);
                self.notice = Some("Compacting…".into());
                self.sync_stop_target(cx);
            }
            "compaction_end" => {
                self.is_streaming = false;
                self.pending_slash_cmd = None;
                settle_all(&mut self.items);
                if event.get("errorSeverity").and_then(Value::as_str) == Some("error") {
                    if let Some(err) = event
                        .get("errorMessage")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        self.notice = Some(err.to_string().into());
                    }
                } else {
                    self.notice = None;
                }
                self.sync_stop_target(cx);
                if let Some(sid) = self.active_session_id.clone() {
                    let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                }
            }
            "message_start" | "message_end" => {
                if let Some(msg) = event.get("message") {
                    if msg.get("display").and_then(Value::as_bool) == Some(false) {
                        return;
                    }
                    let ctype = msg.get("customType").and_then(Value::as_str).unwrap_or("");
                    let role = msg.get("role").and_then(Value::as_str).unwrap_or("");
                    let is_visible_custom = (!ctype.is_empty() && ctype != "session_slash_command")
                        || matches!(role, "compactionSummary" | "branchSummary");
                    if kind == "message_start" && is_visible_custom {
                        let built = build_items(std::slice::from_ref(msg));
                        for mut item in built {
                            let duplicate = self.items.last().is_some_and(|last| {
                                last.kind == item.kind
                                    && last.title == item.title
                                    && last.raw_body == item.raw_body
                            });
                            if !duplicate {
                                let old_len = self.items.len();
                                item.id = self.next_item_id;
                                self.next_item_id = self.next_item_id.wrapping_add(1);
                                self.items.push(item);
                                self.sync_list_after_tail_update(old_len);
                            }
                        }
                    }
                    let is_command_done = matches!(
                        ctype,
                        "session_slash_command_result"
                            | "compaction_outcome"
                            | "refinement_outcome"
                            | "auto_refinement"
                            | "self_refinement"
                            | "model_prompt_error"
                    ) || matches!(role, "compactionSummary" | "branchSummary");
                    if is_command_done {
                        self.is_streaming = false;
                        self.pending_slash_cmd = None;
                        self.notice = None;
                        settle_all(&mut self.items);
                        self.sync_stop_target(cx);
                        if let Some(sid) = self.active_session_id.clone() {
                            let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                        }
                    }
                }
            }
            "agent_end" | "turn_end" => {
                self.is_streaming = false;
                self.pending_slash_cmd = None;
                if matches!(
                    self.notice.as_deref(),
                    Some("Compacting…") | Some("Refining…")
                ) {
                    self.notice = None;
                }
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
                if let Some(sid) = self.active_session_id.clone() {
                    let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                }
            }
            "session_updated" => {
                let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            }
            _ => {}
        }
    }

    fn send_current_prompt(&mut self, steer: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        self.hovered_model = None;
        let text = self.composer.read(cx).value().trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }

        let mut command_notice: Option<&'static str> = None;
        let mut slash_cmd_kind: Option<&'static str> = None;
        if self.attachments.is_empty() {
            match slash_dispatch(&text) {
                SlashAction::SendMessage => {}
                SlashAction::SendDaemonCommand => {
                    let cmd = text
                        .trim()
                        .strip_prefix('/')
                        .and_then(|s| s.split_whitespace().next());
                    command_notice = match cmd {
                        Some("compact") => Some("Compacting…"),
                        Some("refine") => Some("Refining…"),
                        _ => None,
                    };
                    slash_cmd_kind = match cmd {
                        Some("compact") => Some("compact"),
                        Some("refine") => Some("refine"),
                        _ => Some("daemon"),
                    };
                }
                SlashAction::Model => {
                    self.open_popup = OpenPopup::ModelMenu;
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                    return;
                }
                SlashAction::Settings => {
                    self.open_popup = OpenPopup::SettingsMenu;
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                    return;
                }
                SlashAction::NewChat => {
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    self.request_new_session(window, cx);
                    return;
                }
                SlashAction::Rename(new_name) => {
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    if let Some(row) = self.active_session().cloned() {
                        let target = RenameTarget {
                            key: session_key(&row),
                            id: row.id.clone(),
                            file: row.session_file.clone(),
                            is_resident: row.is_resident(),
                        };
                        if new_name.is_empty() {
                            self.start_rename(target, row.name, window, cx);
                        } else {
                            if let Some(r) = self.sessions.iter_mut().find(|s| session_key(s) == target.key) {
                                r.name = new_name.clone();
                            }
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
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    if let Some(sid) = self.active_session_id.clone() {
                        let _ = self.bridge.tx.send(DaemonCmd::ExportHtml { session: sid });
                    }
                    cx.notify();
                    return;
                }
                SlashAction::Copy => {
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    if let Some(last) = self
                        .items
                        .iter()
                        .rev()
                        .find(|it| it.kind == ItemKind::Assistant && !it.raw_body.is_empty())
                    {
                        self.copy_text(last.raw_body.clone(), cx);
                        self.notice = Some("Copied last reply".into());
                    }
                    cx.notify();
                    return;
                }
                SlashAction::Quit => {
                    cx.quit();
                    return;
                }
                SlashAction::NotWired(name) => {
                    self.composer.update(cx, |input, cx| input.set_value("", window, cx));
                    self.notice = Some(format!("/{name} is not available in the desktop app").into());
                    cx.notify();
                    return;
                }
            }
        }

        let held = std::mem::take(&mut self.attachments);
        let preview_images: Vec<Arc<Image>> = held.iter().map(|a| a.image.clone()).collect();
        let wire_images: Vec<Value> = held.iter().map(Attachment::block).collect();

        // In-place queue edit: replace the message at its original slot without reordering.
        if let Some((lane, slot, expected)) = self.editing_queue.take() {
            if let Some(sid) = self.active_session_id.clone() {
                let _ = self.bridge.tx.send(DaemonCmd::EditQueue {
                    session: sid,
                    lane,
                    slot,
                    expected,
                    text,
                    images: wire_images,
                });
                self.composer.update(cx, |input, cx| {
                    input.set_value("", window, cx);
                });
                if self.notice.as_deref() == Some("Editing queued message · Esc to cancel") {
                    self.notice = None;
                }
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

        if let Some(note) = command_notice {
            self.notice = Some(note.into());
        }
        // Only take over the status strip word when starting a fresh turn; queuing a message
        // mid-turn must not overwrite the running turn's label or reset its elapsed timer.
        if !self.is_streaming {
            self.pending_slash_cmd = slash_cmd_kind;
            self.set_busy_label("Waiting", false);
        }

        if self.draft_chat || self.active_session_id.is_none() {
            let (provider, model_id, _, thinking) = self.current_model();
            let model = (!provider.is_empty() && !model_id.is_empty())
                .then_some((provider, model_id));
            let level = model
                .as_ref()
                .and_then(|_| (!thinking.is_empty()).then_some(thinking));
            self.draft_chat = false;
            self.draft_model_picked = false;
            self.is_streaming = true;
            let _ = self.bridge.tx.send(DaemonCmd::StartChat {
                text,
                images: wire_images,
                model,
                level,
            });
            self.composer.update(cx, |input, cx| {
                input.set_value("", window, cx);
            });
            self.sync_stop_target(cx);
            cx.notify();
            return;
        }

        let Some(sid) = self.active_session_id.clone() else {
            return;
        };

        if let Some(row) = self.sessions.iter_mut().find(|s| session_key(s) == sid) {
            if row.worker_state.is_empty() {
                row.worker_state = "ready".into();
            }
            row.lifecycle = "live".into();
            row.activity = "working".into();
            row.is_session_active = true;
            row.is_streaming = true;
        }
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
        self.composer.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        self.sync_stop_target(cx);
        cx.notify();
    }

    fn start_rename(
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

    fn commit_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(target) = self.renaming.take() else {
            return;
        };
        let name = self.rename_input.read(cx).value().trim().to_string();
        if !name.is_empty() {
            if let Some(row) = self
                .sessions
                .iter_mut()
                .find(|s| session_key(s) == target.key)
            {
                row.name = name.clone();
            }
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

    fn toggle_group(&mut self, group: SidebarGroup, cx: &mut Context<Self>) {
        match group {
            SidebarGroup::Active => self.settings.active_closed = !self.settings.active_closed,
            SidebarGroup::Idle => self.settings.idle_closed = !self.settings.idle_closed,
            SidebarGroup::Done => self.settings.done_closed = !self.settings.done_closed,
        }
        self.settings.save();
        cx.notify();
    }

    fn toggle_item(&mut self, id: usize, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        if let Some((ix, item)) = self
            .items
            .iter_mut()
            .enumerate()
            .find(|(_, i)| i.id == id)
        {
            item.expanded = !item.expanded;
            if item.expanded && item.body_md.is_none() && !item.raw_body.is_empty() {
                let formatted = format_expanded_body(item);
                item.body_md = Some(cx.new(|cx| TextViewState::markdown(&formatted, cx)));
            }
            self.list_state.remeasure_items(ix..ix + 1);
            cx.notify();
        }
    }

    fn copy_text(&self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn active_session(&self) -> Option<&SessionRow> {
        let sid = self.active_session_id.as_deref()?;
        self.sessions.iter().find(|s| session_key(s) == sid)
    }

    fn current_model(&self) -> (String, String, String, String) {
        let row = self.active_session();
        let state = self.session_state.as_ref();

        let provider = state
            .map(|s| s.provider.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.model_provider.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let model_id = state
            .map(|s| s.model_id.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.model.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let thinking = state
            .map(|s| s.thinking_level.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| row.map(|s| s.thinking.as_str()).filter(|s| !s.is_empty()))
            .unwrap_or_default()
            .to_string();

        let label = self
            .models
            .iter()
            .find(|m| m.id == model_id && (provider.is_empty() || m.provider == provider))
            .map(|m| model_label(&m.name, &m.id))
            .or_else(|| {
                row.filter(|s| !s.model_name.is_empty())
                    .map(|s| model_label(&s.model_name, &s.model))
            })
            .or_else(|| (!model_id.is_empty()).then(|| model_label("", &model_id)))
            .unwrap_or_else(|| "Model".to_string());

        (provider, model_id, label, thinking)
    }

    fn find_matches(&self, cx: &Context<Self>) -> Vec<usize> {
        let q = self.find_input.read(cx).value().trim().to_ascii_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.items
            .iter()
            .filter(|it| {
                it.raw_body.to_ascii_lowercase().contains(&q)
                    || it.title.to_ascii_lowercase().contains(&q)
                    || it.output
                        .as_deref()
                        .is_some_and(|o| o.to_ascii_lowercase().contains(&q))
            })
            .map(|it| it.id)
            .collect()
    }

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        let matches = self.find_matches(cx);
        if matches.is_empty() {
            self.find_index = 0;
            return;
        }
        let len = matches.len() as isize;
        self.find_index = ((self.find_index as isize + delta).rem_euclid(len)) as usize;
        if let Some(ix) = self
            .items
            .iter()
            .position(|it| it.id == matches[self.find_index])
        {
            self.list_state.scroll_to_reveal_item(ix);
        }
        cx.notify();
    }

    fn render_prime_mark(&self) -> impl IntoElement {
        let path = if self.settings.dark_mode {
            PRIME_AGENT_MARK_DARK_PATH
        } else {
            PRIME_AGENT_MARK_LIGHT_PATH
        };
        img(path).size(px(18.)).object_fit(ObjectFit::Contain)
    }

    fn render_slim_input(
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

    fn render_titlebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                            .child(Icon::new(IconName::Settings).size(px(14.)).text_color(p.quiet_ink))
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.open_popup = if this.open_popup == OpenPopup::SettingsMenu {
                                    OpenPopup::None
                                } else {
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
                            .on_click(cx.listener(|this, _, _, cx| {
                                cx.stop_propagation();
                                this.settings.right_open = !this.settings.right_open;
                                this.settings.save();
                                cx.notify();
                            })),
                    ),
            )
    }

    fn render_collapsed_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child(Icon::new(IconName::Plus).size(px(15.)).text_color(p.quiet_ink))
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
                    .child(Icon::new(IconName::Search).size(px(14.)).text_color(p.quiet_ink))
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

    fn render_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        if !self.settings.sidebar_open {
            return self.render_collapsed_rail(cx).into_any_element();
        }
        let p = self.palette();
        let query = self.search_input.read(cx).value().trim().to_ascii_lowercase();
        let matches_query = |s: &&SessionRow| -> bool {
            if query.is_empty() {
                return true;
            }
            s.name.to_ascii_lowercase().contains(&query)
                || s.id.to_ascii_lowercase().contains(&query)
                || s.cwd.to_ascii_lowercase().contains(&query)
        };

        let is_top_level = |s: &&SessionRow| -> bool {
            s.depth == 0 && s.parent.is_empty() && s.parent_active.is_empty()
        };

        let active_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| s.roster_status() == "Running")
            .filter(matches_query)
            .collect();
        let idle_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| s.roster_status() == "Idle")
            .filter(matches_query)
            .collect();
        let done_rows: Vec<&SessionRow> = self
            .sessions
            .iter()
            .filter(is_top_level)
            .filter(|s| !matches!(s.roster_status(), "Running" | "Idle"))
            .filter(matches_query)
            .take(60)
            .collect();

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
                                    .child(Icon::new(IconName::Search).size(px(14.)).text_color(p.quiet_ink))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        cx.stop_propagation();
                                        this.search_open = !this.search_open;
                                        if this.search_open {
                                            let focus = this.search_input.read(cx).focus_handle(cx);
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

    fn render_session_group(
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
            let selected = self.active_session_id.as_deref() == Some(key.as_str());
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
                        this.draft_chat = false;
                        this.draft_model_picked = false;
                        this.pending_slash_cmd = None;
                        if let Some(row) = this
                            .sessions
                            .iter_mut()
                            .find(|s| session_key(s) == click_key)
                        {
                            if !row.is_resident() || row.lifecycle == "archived" {
                                row.worker_state = "ready".into();
                                row.lifecycle = "live".into();
                                row.activity = "idle".into();
                                row.is_session_active = false;
                            }
                        }
                        let _ = this
                            .bridge
                            .tx
                            .send(DaemonCmd::SelectSession(click_key.clone()));
                        cx.notify();
                    })),
            );
        }

        card
    }

    fn render_find_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.find_open {
            return None;
        }
        let p = self.palette();
        let matches = self.find_matches(cx);
        let count_label = if matches.is_empty() {
            "0 matches".to_string()
        } else {
            format!("{}/{}", (self.find_index % matches.len()) + 1, matches.len())
        };
        Some(
            h_flex()
                .w_full()
                .px_5()
                .py_1p5()
                .border_b_1()
                .border_color(p.transcript_cell_edge)
                .bg(p.transcript_cell_fill)
                .gap_2()
                .items_center()
                .child(
                    div()
                        .w(px(260.))
                        .child(self.render_slim_input(&self.find_input, window, cx)),
                )
                .child(div().text_xs().text_color(p.quiet_ink).child(count_label))
                .child(
                    Button::new("find-prev")
                        .ghost()
                        .xsmall()
                        .label("Prev")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .label("Next")
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                )
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.find_open = false;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_activity_chip(&self) -> impl IntoElement {
        let p = self.palette();
        let metric = |icon_path: &'static str, count: usize| {
            let ink = if count > 0 { p.working } else { p.quiet_ink };
            h_flex()
                .h_full()
                .items_center()
                .gap(px(5.))
                .child(svg().path(icon_path).size(px(13.5)).text_color(ink))
                .child(
                    div()
                        .text_size(px(11.5))
                        .line_height(px(14.))
                        .text_color(ink)
                        .child(count.to_string()),
                )
        };

        h_flex()
            .h(px(26.))
            .px(px(10.))
            .gap(px(12.))
            .rounded(px(7.))
            .border_1()
            .border_color(p.composer_edge)
            .bg(p.sidebar_group_surface)
            .items_center()
            .child(metric(ICON_SUBAGENTS_PATH, self.subagent_count))
            .child(metric(ICON_HEARTBEAT_PATH, self.heartbeat_count))
            .child(metric(ICON_SHELLS_PATH, self.shell_count))
    }

    fn render_send_stop_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let mix = ease_in_out_cubic(self.stop_mix);
        let bg_color = lerp_hsla(p.composer_send_fill, p.danger, mix);
        let is_stop = self.stop_target > 0.5;

        // Exact pixel-centered 26x26 circle:
        // - Arrow is 12x12 -> (26 - 12)/2 = 7px exact integer offset
        // - Stop square is 8x8 -> (26 - 8)/2 = 9px exact integer offset
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
                    if let Some(sid) = this.active_session_id.clone() {
                        let _ = this.bridge.tx.send(DaemonCmd::Abort { session: sid });
                    }
                } else {
                    let steer = this.is_streaming;
                    this.send_current_prompt(steer, window, cx);
                }
            }))
    }

    fn render_busy_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let opening = self.busy_strip_wanted();
        let (strip_h, strip_opacity) = busy_strip_geometry(self.busy_strip_mix, opening);
        if strip_h <= 0.5 {
            return None;
        }
        let p = self.palette();
        let queued_total = self.queued_steer.len() + self.queued_follow_up.len();
        let queue_open = self.open_popup == OpenPopup::QueuePanel;
        let show_working = self.is_streaming || !self.busy_label.is_empty();
        let label = if self.busy_label.is_empty() {
            "Working"
        } else {
            self.busy_label.as_str()
        };
        let elapsed_secs = self
            .busy_since
            .map(|since| since.elapsed().as_secs())
            .unwrap_or(0);
        let spinner = spinner_frame_for_ms(self.anim_phase_ms);

        Some(
            div()
                .id("busy-status-strip")
                .relative()
                .w_full()
                .h(px(strip_h))
                .opacity(strip_opacity)
                // Tucked rounded-top shell extending 12px behind the composer box's top corners
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

    fn render_composer_band(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette();
        let composer_focused = self.composer.read(cx).focus_handle(cx).is_focused(window);
        let (_, _, model_chip_label, model_chip_thinking) = self.current_model();
        let effort_badge = (!model_chip_thinking.is_empty() && model_chip_thinking != "off")
            .then(|| thinking_level_label(&model_chip_thinking).to_string());

        let ctx_pct = self
            .stats
            .as_ref()
            .filter(|st| st.context_window > 0)
            .map(|st| format!("{:.0}%", st.percent))
            .unwrap_or_else(|| "0%".to_string());

        let handle_color = if self.dragging_composer {
            p.working
        } else if self.handle_hovered {
            p.quiet_ink
        } else {
            p.composer_handle
        };

        v_flex()
            .w_full()
            .px(px(18.))
            .pb(px(12.))
            .pt(px(2.))
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
                    // Interactive resize grab handle at top center
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
                                this.handle_hovered = *hovered;
                                cx.notify();
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if ev.click_count >= 2 {
                                        this.composer_height = None;
                                        this.dragging_composer = false;
                                    } else {
                                        this.dragging_composer = true;
                                        this.drag_start_y = ev.position.y.as_f32();
                                        this.drag_start_height =
                                            this.composer_height.unwrap_or(56.0);
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
                    // Multi-line composer textarea (supports manual drag height OR auto-grow)
                    .when(!self.attachments.is_empty(), |card| {
                        card.child(
                            h_flex()
                                .w_full()
                                .px(px(12.))
                                .pb(px(6.))
                                .gap(px(8.))
                                .flex_wrap()
                                .children(self.attachments.iter().enumerate().map(|(idx, att)| {
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
                                            if idx < this.attachments.len() {
                                                this.attachments.remove(idx);
                                                this.sync_stop_target(cx);
                                                cx.notify();
                                            }
                                        }))
                                })),
                        )
                    })
                    .child(
                        div()
                            .w_full()
                            .px(px(10.))
                            .pb(px(6.))
                            .child(match self.composer_height {
                                Some(h) => Textarea::new(&self.composer)
                                    .h(px(h))
                                    .appearance(false)
                                    .bordered(false),
                                None => Textarea::new(&self.composer)
                                    .appearance(false)
                                    .bordered(false),
                            }),
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
                                    .rounded(px(7.))
                                    .bg(p.composer_attach_fill)
                                    .hover(|d| d.bg(p.sidebar_row_selected))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .child(Icon::new(IconName::Plus).size(px(13.)).text_color(p.quiet_ink))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        cx.stop_propagation();
                                        let tx = this.attach_tx.clone();
                                        std::thread::spawn(move || {
                                            let picked = pick_image_files();
                                            let _ = tx.try_send(picked);
                                        });
                                    })),
                            )
                            .child(self.render_activity_chip())
                            .when_some(self.notice.clone(), |row, note| {
                                row.child(
                                    div()
                                        .text_size(px(11.))
                                        .text_color(p.quiet_ink)
                                        .overflow_hidden()
                                        .whitespace_nowrap()
                                        .text_ellipsis()
                                        .child(note),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                div()
                                    .id("ctx-ring-btn")
                                    .h(px(26.))
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
                                            .text_size(px(11.5))
                                            .line_height(px(14.))
                                            .text_color(p.quiet_ink)
                                            .child(model_chip_label),
                                    )
                                    .when_some(effort_badge, |chip, eff| {
                                        chip.child(
                                            div()
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
    }

    fn render_context_card_content(&self) -> impl IntoElement {
        let p = self.palette();
        let (used_str, pct_str, fill, msgs, tools, inp, outp, cached, cost) = match &self.stats {
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
                "—".into(),
                "0%".into(),
                0.0,
                "0".into(),
                "0".into(),
                "0".into(),
                "0".into(),
                "0".into(),
                "$0.0000".into(),
            ),
        };

        let ledger = |label: &'static str, value: String| {
            h_flex()
                .w_full()
                .justify_between()
                .child(div().text_size(px(11.)).text_color(p.quiet_ink).child(label))
                .child(div().text_size(px(11.)).text_color(p.ink).child(value))
        };

        v_flex()
            .w_full()
            .gap(px(6.))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .child(div().text_size(px(10.5)).text_color(p.quiet_ink).child("context"))
                    .child(div().text_size(px(12.)).font_semibold().text_color(p.ink).child(pct_str)),
            )
            .child(
                div()
                    .w_full()
                    .h(px(3.))
                    .rounded(px(2.))
                    .bg(p.panel_edge)
                    .child(
                        div()
                            .h_full()
                            .w(relative(fill))
                            .rounded(px(2.))
                            .bg(p.working),
                    ),
            )
            .child(div().text_size(px(10.5)).text_color(p.quiet_ink).child(used_str))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("messages", msgs))
            .child(ledger("tool calls", tools))
            .child(ledger("input", inp))
            .child(ledger("output", outp))
            .child(ledger("cache read", cached))
            .child(div().h(px(1.)).w_full().bg(p.panel_edge))
            .child(ledger("session cost", cost))
    }

    fn render_slash_overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let val = self.composer.read(cx).value();
        let trimmed = val.trim_start();
        if !trimmed.starts_with('/') || trimmed.contains(' ') || trimmed.contains('\n') {
            return None;
        }
        let prefix = &trimmed[1..];
        let matches: Vec<_> = pa_client::commands::filter(prefix)
            .into_iter()
            .filter(|cmd| is_supported_slash_cmd(cmd))
            .collect();
        if matches.is_empty() {
            return None;
        }
        let p = self.palette();
        let left_offset = if self.settings.sidebar_open {
            px(262.)
        } else {
            px(62.)
        };
        let mut list = v_flex()
            .absolute()
            .bottom(px(112.))
            .left(left_offset)
            .w(px(420.))
            .p_1p5()
            .rounded(px(10.))
            .border_1()
            .border_color(p.panel_edge)
            .bg(p.panel_surface)
            .shadow_lg()
            .gap_0p5();

        for (i, cmd) in matches.into_iter().take(6).enumerate() {
            let cmd_name = cmd.name;
            let cmd_desc = cmd.description;
            list = list.child(
                h_flex()
                    .id(("slash-opt", i))
                    .px_2()
                    .py_1()
                    .gap_3()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|d| d.bg(p.sidebar_row_hover))
                    .child(
                        div()
                            .text_xs()
                            .font_semibold()
                            .text_color(p.ink)
                            .child(format!("/{cmd_name}")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(p.quiet_ink)
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(cmd_desc),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.composer.update(cx, |input, cx| {
                            input.set_value(format!("/{cmd_name} "), window, cx);
                        });
                        cx.notify();
                    })),
            );
        }

        Some(list.into_any_element())
    }

    fn render_right_column(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.settings.right_open {
            return None;
        }
        let p = self.palette();
        let active_name = self
            .active_session()
            .map(|s| {
                if !s.name.trim().is_empty() {
                    s.name.clone()
                } else {
                    session_key(s)
                }
            })
            .unwrap_or_else(|| "No session".to_string());

        Some(
            v_flex()
                .w(px(300.))
                .h_full()
                .bg(p.window_surface)
                .px_4()
                .py_3()
                .gap_3()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(p.quiet_ink)
                        .child("SESSION"),
                )
                .child(div().text_sm().font_semibold().text_color(p.ink).child(active_name))
                .child(
                    div()
                        .p_3()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .bg(p.sidebar_group_surface)
                        .child(self.render_context_card_content()),
                )
                .when(self.active_session_id.is_some(), |col| {
                    col.child(
                        h_flex()
                            .id("right-export")
                            .h(px(30.))
                            .px_3()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.sidebar_new_fill)
                            .hover(|d| d.bg(p.sidebar_new_fill_hover))
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .text_xs()
                            .text_color(p.ink)
                            .child("Export HTML")
                            .on_click(cx.listener(|this, _, _, _| {
                                if let Some(sid) = this.active_session_id.clone() {
                                    let _ = this
                                        .bridge
                                        .tx
                                        .send(DaemonCmd::ExportHtml { session: sid });
                                }
                            })),
                    )
                })
                .child(div().flex_1())
                .child(div().text_xs().text_color(p.quiet_ink).child(self.status.clone()))
                .into_any_element(),
        )
    }

    fn render_floating_popups(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let p = self.palette();
        let right_offset = if self.settings.right_open {
            px(322.)
        } else {
            px(24.)
        };

        match &self.open_popup {
            OpenPopup::None => self.render_slash_overlay(cx),
            OpenPopup::QueuePanel => {
                let queued_total = self.queued_steer.len() + self.queued_follow_up.len();
                if queued_total == 0 {
                    return None;
                }
                let strip_h = busy_strip_geometry(self.busy_strip_mix, self.busy_strip_wanted()).0;
                let attach_h = if self.attachments.is_empty() { 0.0 } else { 52.0 };
                let panel_bottom =
                    12.0 + 12.0 + self.composer_height.unwrap_or(54.0) + attach_h + 40.0 + strip_h + 6.0;
                let panel_right = if self.settings.right_open {
                    px(318.)
                } else {
                    px(18.)
                };

                let mut col = v_flex().gap(px(2.));
                let header_label = if queued_total == 1 {
                    "1 message waiting".to_string()
                } else {
                    format!("{queued_total} messages waiting")
                };

                if !self.queued_steer.is_empty() {
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
                    for (idx, txt) in self.queued_steer.iter().enumerate() {
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
                                            this.editing_queue = Some((
                                                "steering".into(),
                                                idx,
                                                exp_edit.clone(),
                                            ));
                                            this.composer.update(cx, |input, cx| {
                                                input.set_value(exp_edit.clone(), window, cx);
                                            });
                                            this.notice =
                                                Some("Editing queued message · Esc to cancel".into());
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
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
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

                if !self.queued_follow_up.is_empty() {
                    let fu_header = if self.queued_steer.is_empty() {
                        format!("{header_label} · runs next")
                    } else {
                        "runs next".to_string()
                    };
                    col = col.child(
                        div()
                            .px(px(8.))
                            .pt(if self.queued_steer.is_empty() {
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
                    for (idx, txt) in self.queued_follow_up.iter().enumerate() {
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
                                            this.editing_queue = Some((
                                                "followUp".into(),
                                                idx,
                                                exp_edit.clone(),
                                            ));
                                            this.composer.update(cx, |input, cx| {
                                                input.set_value(exp_edit.clone(), window, cx);
                                            });
                                            this.notice =
                                                Some("Editing queued message · Esc to cancel".into());
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
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
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
                                            if let Some(sid) = this.active_session_id.clone() {
                                                let _ = this.bridge.tx.send(DaemonCmd::MutateQueue {
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
                                    this.editing_queue = None;
                                    if let Some(sid) = this.active_session_id.clone() {
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
                let (cur_provider, cur_model_id, _, raw_thinking) = self.current_model();
                let cur_thinking = if raw_thinking.is_empty() {
                    "medium".to_string()
                } else {
                    raw_thinking
                };

                let mut groups: BTreeMap<(bool, String), Vec<&ModelEntry>> = BTreeMap::new();
                for entry in &self.models {
                    groups
                        .entry((!entry.configured, entry.provider.clone()))
                        .or_default()
                        .push(entry);
                }

                let mut model_list = v_flex().gap(px(2.));
                let mut running_top: f32 = 6.0;
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
                                        .when(selected, |d| d.bg(p.sidebar_row_selected))
                                        .hover(|d| d.bg(p.sidebar_row_hover))
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
                                                .text_color(if selected {
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
                                                .gap(px(6.))
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
                                                            .text_color(p.ink),
                                                    )
                                                }),
                                        )
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            if configured {
                                                let saved_level = this
                                                    .settings
                                                    .model_thinking_for(&prov, &mid)
                                                    .map(|lvl| {
                                                        if levels_c.is_empty() {
                                                            lvl.to_string()
                                                        } else {
                                                            clamp_level(&levels_c, lvl)
                                                        }
                                                    });
                                                let st = this
                                                    .session_state
                                                    .get_or_insert_with(SessionState::default);
                                                st.provider = prov.clone();
                                                st.model_id = mid.clone();
                                                if let Some(level) = &saved_level {
                                                    st.thinking_level = level.clone();
                                                } else if !levels_c.is_empty() {
                                                    st.thinking_level = clamp_level(
                                                        &levels_c,
                                                        &st.thinking_level,
                                                    );
                                                }
                                                if this.draft_chat
                                                    || this.active_session_id.is_none()
                                                {
                                                    this.draft_model_picked = true;
                                                } else if let Some(sid) =
                                                    this.active_session_id.clone()
                                                {
                                                    if let Some(level) = saved_level {
                                                        let _ = this.bridge.tx.send(
                                                            DaemonCmd::SetModelAndThinking {
                                                                session: sid,
                                                                provider: prov.clone(),
                                                                model_id: mid.clone(),
                                                                level,
                                                                switch: true,
                                                            },
                                                        );
                                                    } else {
                                                        let _ = this.bridge.tx.send(
                                                            DaemonCmd::SetModel {
                                                                session: sid,
                                                                provider: prov.clone(),
                                                                model_id: mid.clone(),
                                                            },
                                                        );
                                                    }
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

                let menu_height = (running_top + 6.0).clamp(64.0, 340.0);
                let ladder_elem = self.hovered_model.as_ref().and_then(
                    |(h_prov, h_id, row_top)| {
                        let entry = self
                            .models
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
                        let ladder_h = 10.0 + (entry.thinking_levels.len() as f32) * 26.0;
                        let from_bottom =
                            (menu_height - row_top - ladder_h + 18.0).clamp(0.0, (menu_height - ladder_h).max(0.0));

                        let mut ladder_col = v_flex()
                            .id("floating-effort-ladder")
                            .absolute()
                            .bottom(px(112.0 + from_bottom))
                            .right(right_offset + px(306.))
                            .w(px(136.))
                            .p(px(5.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.panel_edge)
                            .bg(p.panel_surface)
                            .shadow_lg()
                            .on_click(|_, _, cx| cx.stop_propagation());

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
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(
                                        div()
                                            .text_size(px(11.5))
                                            .text_color(if is_checked { p.ink } else { p.quiet_ink })
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
                                        this.settings
                                            .set_model_thinking(&prov_c, &mid_c, &level_c);
                                        let st = this
                                            .session_state
                                            .get_or_insert_with(SessionState::default);
                                        st.provider = prov_c.clone();
                                        st.model_id = mid_c.clone();
                                        st.thinking_level = level_c.clone();
                                        if this.draft_chat || this.active_session_id.is_none() {
                                            this.draft_model_picked = true;
                                        } else if let Some(sid) = this.active_session_id.clone() {
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
                    },
                );

                Some(
                    div()
                        .child(
                            v_flex()
                                .id("floating-model-menu")
                                .absolute()
                                .bottom(px(112.))
                                .right(right_offset)
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
                                        .id("model-popup-scroll")
                                        .max_h(px(328.))
                                        .overflow_y_scrollbar()
                                        .child(model_list),
                                ),
                        )
                        .children(ladder_elem)
                        .into_any_element(),
                )
            }
            OpenPopup::ContextCard => Some(
                div()
                    .id("floating-ctx-card")
                    .absolute()
                    .bottom(px(112.))
                    .right(right_offset)
                    .w(px(236.))
                    .p_3()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.panel_surface)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(self.render_context_card_content())
                    .into_any_element(),
            ),
            OpenPopup::SettingsMenu => Some(
                div()
                    .id("floating-settings-menu")
                    .absolute()
                    .top(px(36.))
                    .right(px(16.))
                    .w(px(210.))
                    .p_2()
                    .rounded(px(10.))
                    .border_1()
                    .border_color(p.panel_edge)
                    .bg(p.panel_surface)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        v_flex()
                            .gap_1()
                            .child(
                                div()
                                    .px_2()
                                    .py_0p5()
                                    .text_size(px(10.5))
                                    .font_semibold()
                                    .text_color(p.quiet_ink)
                                    .child("APPEARANCE"),
                            )
                            .child(
                                h_flex()
                                    .id("set-theme-toggle")
                                    .px_2()
                                    .py_1p5()
                                    .rounded(px(6.))
                                    .justify_between()
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(div().text_xs().text_color(p.ink).child("Theme"))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.quiet_ink)
                                            .child(if self.settings.dark_mode { "Dark" } else { "Light" }),
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.settings.dark_mode = !this.settings.dark_mode;
                                        this.settings.save();
                                        Theme::change(
                                            if this.settings.dark_mode {
                                                ThemeMode::Dark
                                            } else {
                                                ThemeMode::Light
                                            },
                                            Some(window),
                                            cx,
                                        );
                                        cx.notify();
                                    })),
                            )
                            .child(
                                h_flex()
                                    .id("set-export-html")
                                    .px_2()
                                    .py_1p5()
                                    .rounded(px(6.))
                                    .items_center()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(p.sidebar_row_hover))
                                    .child(div().text_xs().text_color(p.ink).child("Export session HTML"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(sid) = this.active_session_id.clone() {
                                            let _ = this
                                                .bridge
                                                .tx
                                                .send(DaemonCmd::ExportHtml { session: sid });
                                        }
                                        this.open_popup = OpenPopup::None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .into_any_element(),
            ),
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
                let file_ren = file.clone();
                let file_del = file.clone();
                let name_c = name.clone();
                let resident = *is_resident;
                let top_y = *anchor_y as f32;
                let can_rename = (resident && !id_ren.is_empty()) || !file_ren.is_empty();
                Some(
                    div()
                        .id("floating-session-menu")
                        .absolute()
                        .top(px(top_y))
                        .left(px(248.))
                        .w(px(190.))
                        .p_1p5()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.panel_edge)
                        .bg(p.panel_surface)
                        .shadow_lg()
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(
                            v_flex()
                                .gap_0p5()
                                .when(can_rename, |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-rename")
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.sidebar_row_hover))
                                            .text_xs()
                                            .text_color(p.ink)
                                            .child("Rename")
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
                                .when(resident, |col| {
                                    col.child(
                                        h_flex()
                                            .id("sm-stop")
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.sidebar_row_hover))
                                            .text_xs()
                                            .text_color(p.ink)
                                            .child("Stop")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(row) = this
                                                    .sessions
                                                    .iter_mut()
                                                    .find(|s| s.id == id_c || session_key(s) == id_c)
                                                {
                                                    row.worker_state.clear();
                                                    row.lifecycle = "archived".into();
                                                    row.activity = "idle".into();
                                                    row.is_streaming = false;
                                                    row.is_session_active = false;
                                                }
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
                                            .px_2()
                                            .py_1()
                                            .rounded(px(6.))
                                            .cursor_pointer()
                                            .hover(|d| d.bg(p.danger_surface))
                                            .text_xs()
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

    fn render_card_icon(&self, item: &TranscriptItem, color: Hsla) -> AnyElement {
        let p = self.palette();
        let phase = self.anim_phase_ms;
        let working_dim = p.working.opacity(0.28);

        match (item.kind, item.variant) {
            (ItemKind::Thinking, _) if item.active => {
                let win_x = -7.0 + 21.0 * thinking_sweep(phase);
                div()
                    .relative()
                    .size(px(14.))
                    .overflow_hidden()
                    .child(
                        svg()
                            .path(ICON_THINKING_PATH)
                            .size(px(14.))
                            .text_color(working_dim),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .left(px(win_x))
                            .w(px(7.))
                            .h(px(14.))
                            .overflow_hidden()
                            .child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left(px(-win_x))
                                    .size(px(14.))
                                    .child(
                                        svg()
                                            .path(ICON_THINKING_PATH)
                                            .size(px(14.))
                                            .text_color(p.working),
                                    ),
                            ),
                    )
                    .into_any_element()
            }
            (ItemKind::Thinking, _) => svg()
                .path(ICON_THINKING_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, CardVariant::Bash) | (_, CardVariant::BashDone) if item.active => {
                let cursor_color = lerp_hsla(p.working, working_dim, bash_cursor_mix(phase));
                div()
                    .relative()
                    .size(px(14.))
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_BASH_CHEVRON_PATH)
                                .size(px(14.))
                                .text_color(p.working),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_BASH_CURSOR_PATH)
                                .size(px(14.))
                                .text_color(cursor_color),
                        ),
                    )
                    .into_any_element()
            }
            (ItemKind::ToolCall, CardVariant::Bash) | (_, CardVariant::BashDone) => svg()
                .path(ICON_BASH_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (ItemKind::ToolCall, _) if item.active => {
                let c_l = lerp_hsla(p.working, working_dim, python_ripple(phase, 0));
                let c_m = lerp_hsla(p.working, working_dim, python_ripple(phase, 320));
                let c_r = lerp_hsla(p.working, working_dim, python_ripple(phase, 640));
                div()
                    .relative()
                    .size(px(14.))
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_L_PATH)
                                .size(px(14.))
                                .text_color(c_l),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_M_PATH)
                                .size(px(14.))
                                .text_color(c_m),
                        ),
                    )
                    .child(
                        div().absolute().inset_0().child(
                            svg()
                                .path(ICON_PYTHON_R_PATH)
                                .size(px(14.))
                                .text_color(c_r),
                        ),
                    )
                    .into_any_element()
            }
            (ItemKind::ToolCall, _) => svg()
                .path(ICON_PYTHON_PATH)
                .size(px(14.))
                .text_color(color)
                .into_any_element(),
            (_, CardVariant::AgentMessage) => {
                let tint = if item.active {
                    lerp_hsla(p.working, working_dim, pulse_mix(phase))
                } else {
                    color
                };
                Icon::new(IconName::Bot)
                    .size(px(14.))
                    .text_color(tint)
                    .into_any_element()
            }
            (_, CardVariant::ProviderRetry) => {
                let mut icon = Icon::new(IconName::Loader).size(px(14.)).text_color(color);
                if item.active {
                    let angle = ((phase % 1000) as f32 / 1000.0) * std::f32::consts::TAU;
                    icon = icon.rotate(radians(angle));
                }
                icon.into_any_element()
            }
            _ => {
                let tint = if item.active {
                    lerp_hsla(p.working, working_dim, pulse_mix(phase))
                } else {
                    color
                };
                Icon::new(IconName::Inbox)
                    .size(px(14.))
                    .text_color(tint)
                    .into_any_element()
            }
        }
    }

    fn render_markdown(
        &self,
        md: &Entity<TextViewState>,
        item_id: usize,
        stream_fade: bool,
    ) -> TextView {
        let p = self.palette();
        TextView::new(md)
            .selectable(true)
            .stream_fade(stream_fade)
            .code_block_actions(move |block, _window, _cx| {
                use std::hash::{Hash as _, Hasher as _};
                let code = block.code();
                let lang = block.lang().filter(|l| !l.is_empty());
                let mut h = std::collections::hash_map::DefaultHasher::new();
                item_id.hash(&mut h);
                code.hash(&mut h);
                let btn_id = h.finish() as usize;
                h_flex()
                    .items_center()
                    .gap(px(6.))
                    .when_some(lang, |row, l| {
                        row.child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.sidebar_group_empty_ink)
                                .child(l),
                        )
                    })
                    .child(
                        div()
                            .id(("code-copy-btn", btn_id))
                            .size(px(22.))
                            .rounded(px(5.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|d| d.bg(p.sidebar_row_hover))
                            .child(
                                Icon::new(IconName::Copy)
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                            )
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                            }),
                    )
            })
    }

    fn render_item(&self, item: &TranscriptItem, cx: &mut Context<Self>) -> AnyElement {
        let p = self.palette();
        match item.kind {
            ItemKind::User => v_flex()
                .w_full()
                .rounded(px(10.))
                .bg(p.transcript_user_fill)
                .px_4()
                .py_3()
                .gap_2()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| self.render_markdown(md, item.id, false)),
                )
                .when(!item.images.is_empty(), |col| {
                    col.child(
                        h_flex().gap_2().flex_wrap().children(
                            item.images.iter().cloned().map(|image| {
                                img(image)
                                    .max_w(px(360.))
                                    .max_h(px(220.))
                                    .rounded(px(6.))
                            }),
                        ),
                    )
                })
                .into_any_element(),
            ItemKind::Assistant => div()
                .w_full()
                .py_1()
                .children(
                    item.body_md
                        .as_ref()
                        .map(|md| self.render_markdown(md, item.id, item.active)),
                )
                .into_any_element(),
            ItemKind::Error => div()
                .w_full()
                .rounded(px(8.))
                .border_1()
                .border_color(p.danger)
                .bg(p.danger_surface)
                .px_3()
                .py_2()
                .text_sm()
                .text_color(p.ink)
                .child(item.raw_body.clone())
                .into_any_element(),
            ItemKind::Thinking | ItemKind::ToolCall | ItemKind::Summary | ItemKind::Custom => {
                let id = item.id;
                let has_body = !item.raw_body.trim().is_empty();
                let icon_color = if item.active {
                    p.working
                } else if item.failed {
                    p.danger
                } else {
                    p.quiet_ink
                };

                v_flex()
                    .w_full()
                    .rounded(px(8.))
                    .border_1()
                    .border_color(if item.active {
                        p.working
                    } else {
                        p.transcript_cell_edge
                    })
                    .bg(p.transcript_cell_fill)
                    .child(
                        h_flex()
                            .id(("card-hdr", id))
                            .h(px(30.))
                            .px_3()
                            .gap_2()
                            .items_center()
                            .when(has_body, |row| {
                                row.cursor_pointer().on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.toggle_item(id, cx);
                                }))
                            })
                            .child(
                                div()
                                    .size(px(14.))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(self.render_card_icon(item, icon_color)),
                            )
                            .when(!item.verb.is_empty(), |row| {
                                row.child(
                                    div()
                                        .text_xs()
                                        .font_medium()
                                        .text_color(if item.active { p.working } else { p.quiet_ink })
                                        .child(item.verb.clone()),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .text_color(if item.failed { p.danger } else { p.ink })
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_ellipsis()
                                    .child(item.title.clone()),
                            )
                            .when(!item.meta.is_empty(), |row| {
                                let badge = if item.failed {
                                    div()
                                        .px(px(6.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(p.danger_surface)
                                        .text_size(px(11.))
                                        .line_height(px(14.))
                                        .text_color(p.danger)
                                        .child(item.meta.clone())
                                } else if item.active {
                                    div()
                                        .px(px(6.))
                                        .py(px(1.))
                                        .rounded(px(4.))
                                        .bg(p.working.opacity(0.14))
                                        .text_size(px(11.))
                                        .line_height(px(14.))
                                        .text_color(p.working)
                                        .child(item.meta.clone())
                                } else {
                                    div()
                                        .text_size(px(11.))
                                        .line_height(px(14.))
                                        .text_color(p.quiet_ink)
                                        .child(item.meta.clone())
                                };
                                row.child(badge)
                            })
                            .when(has_body, |row| {
                                row.child(
                                    Icon::new(if item.expanded {
                                        IconName::ChevronDown
                                    } else {
                                        IconName::ChevronRight
                                    })
                                    .size(px(12.))
                                    .text_color(p.quiet_ink),
                                )
                            }),
                    )
                    .when(item.expanded && has_body, |card| {
                        card.child(
                            h_flex()
                                .w_full()
                                .border_t_1()
                                .border_color(p.transcript_cell_edge)
                                .items_stretch()
                                .child(
                                    div()
                                        .id(("card-rail", id))
                                        .group("card-rail")
                                        .w(px(16.))
                                        .flex_shrink_0()
                                        .py_2()
                                        .flex()
                                        .justify_center()
                                        .cursor_pointer()
                                        .hover(|d| d.bg(p.sidebar_row_hover))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.toggle_item(id, cx);
                                        }))
                                        .child(
                                            div()
                                                .w(px(2.))
                                                .h_full()
                                                .rounded(px(1.))
                                                .bg(p.transcript_cell_edge)
                                                .group_hover("card-rail", |s| s.bg(p.working)),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .pl_1()
                                        .pr_3()
                                        .py_2()
                                        .children(
                                            item.body_md
                                                .as_ref()
                                                .map(|md| self.render_markdown(md, id, false)),
                                        ),
                                ),
                        )
                    })
                    .into_any_element()
            }
        }
    }
}

impl Render for PaApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.pending_trim_tail {
            self.trim_tail_window();
        }
        self.advance_stop_anim(window);
        if self.is_streaming || self.items.iter().any(|it| it.active) {
            self.anim_phase_ms = self.anim_epoch.elapsed().as_millis() as u64;
            window.request_animation_frame();
        }
        let p = self.palette();
        let scrolled_up = self.scrolled_up || self.is_scrolled_up();

        div()
            .id("pa-root")
            .relative()
            .size_full()
            .bg(p.window_surface)
            .text_color(p.ink)
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| {
                if this.dragging_composer {
                    if ev.pressed_button != Some(MouseButton::Left) {
                        this.dragging_composer = false;
                        cx.notify();
                        return;
                    }
                    let dy = this.drag_start_y - ev.position.y.as_f32();
                    let next = (this.drag_start_height + dy).clamp(44.0, 320.0);
                    this.composer_height = Some(next);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.dragging_composer {
                        this.dragging_composer = false;
                        cx.notify();
                    }
                }),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if this.open_popup != OpenPopup::None {
                    this.open_popup = OpenPopup::None;
                    this.hovered_model = None;
                    cx.notify();
                }
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let key = ev.keystroke.key.as_str();
                let ctrl = ev.keystroke.modifiers.control || ev.keystroke.modifiers.platform;
                if ctrl && key == "b" {
                    this.settings.sidebar_open = !this.settings.sidebar_open;
                    this.rail_mark_hovered = false;
                    this.settings.save();
                    cx.notify();
                } else if ctrl && key == "n" {
                    this.request_new_session(window, cx);
                } else if ctrl && key == "f" {
                    this.find_open = !this.find_open;
                    if this.find_open {
                        let focus = this.find_input.read(cx).focus_handle(cx);
                        window.focus(&focus, cx);
                    }
                    cx.notify();
                } else if key == "escape" {
                    if this.open_popup != OpenPopup::None {
                        this.open_popup = OpenPopup::None;
                        this.hovered_model = None;
                        cx.notify();
                    } else if this.find_open {
                        this.find_open = false;
                        cx.notify();
                    } else if this.renaming.is_some() {
                        this.renaming = None;
                        cx.notify();
                    } else if this.editing_queue.take().is_some() {
                        this.composer.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                        });
                        if this.notice.as_deref() == Some("Editing queued message · Esc to cancel") {
                            this.notice = None;
                        }
                        this.sync_stop_target(cx);
                        cx.notify();
                    } else if this.is_streaming {
                        if let Some(sid) = this.active_session_id.clone() {
                            let _ = this.bridge.tx.send(DaemonCmd::Abort { session: sid });
                        }
                    }
                }
            }))
            .child(
                v_flex()
                    .size_full()
                    .child(self.render_titlebar(cx))
                    .child(
                        h_flex()
                            .flex_1()
                            .w_full()
                            .min_h_0()
                            .child(self.render_sidebar(window, cx))
                            .child(
                                v_flex()
                                    .relative()
                                    .flex_1()
                                    .h_full()
                                    .min_w_0()
                                    .bg(p.transcript_surface)
                                    .rounded_tl(px(10.))
                                    .when(self.settings.right_open, |s| s.rounded_tr(px(10.)))
                                    .overflow_hidden()
                                    .children(self.render_find_bar(window, cx))
                                    .child(
                                        div()
                                            .relative()
                                            .flex_1()
                                            .w_full()
                                            .min_h_0()
                                            .child(
                                                list(
                                                    self.list_state.clone(),
                                                    cx.processor(|this, ix, _window, cx| {
                                                        let len = this.items.len();
                                                        if let Some(item) = this.items.get_mut(ix) {
                                                            hydrate_markdown(item, cx);
                                                        }
                                                        let Some(item) = this.items.get(ix) else {
                                                            return div().into_any_element();
                                                        };
                                                        div()
                                                            .w_full()
                                                            .px_6()
                                                            .when(ix == 0, |d| d.pt_4())
                                                            .pb(if ix + 1 == len {
                                                                px(16.)
                                                            } else {
                                                                px(12.)
                                                            })
                                                            .child(
                                                                div()
                                                                    .w_full()
                                                                    .max_w(px(860.))
                                                                    .mx_auto()
                                                                    .child(this.render_item(item, cx)),
                                                            )
                                                            .into_any_element()
                                                    }),
                                                )
                                                .size_full(),
                                            )
                                            .vertical_scrollbar(&self.list_state)
                                            .when(self.loading_older, |wrap| {
                                                wrap.child(
                                                    h_flex()
                                                        .absolute()
                                                        .left_0()
                                                        .right_0()
                                                        .top(px(8.))
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .px(px(10.))
                                                                .py(px(3.))
                                                                .rounded_full()
                                                                .border_1()
                                                                .border_color(p.panel_edge)
                                                                .bg(p.panel_surface)
                                                                .text_size(px(11.))
                                                                .text_color(p.quiet_ink)
                                                                .child("Loading older messages…"),
                                                        ),
                                                )
                                            })
                                            .when(scrolled_up, |wrap| {
                                                wrap.child(
                                                    h_flex()
                                                        .absolute()
                                                        .left_0()
                                                        .right_0()
                                                        .bottom(px(12.))
                                                        .justify_center()
                                                        .child(
                                                            div()
                                                                .id("jump-to-bottom-btn")
                                                                .size(px(30.))
                                                                .rounded_full()
                                                                .border_1()
                                                                .border_color(p.panel_edge)
                                                                .bg(p.panel_surface)
                                                                .shadow_md()
                                                                .flex()
                                                                .items_center()
                                                                .justify_center()
                                                                .cursor_pointer()
                                                                .hover(|d| d.bg(p.sidebar_new_fill_hover))
                                                                .child(
                                                                    Icon::new(IconName::ChevronDown)
                                                                        .size(px(14.))
                                                                        .text_color(p.ink),
                                                                )
                                                                .on_click(cx.listener(|this, _, _, cx| {
                                                                    cx.stop_propagation();
                                                                    this.scrolled_up = false;
                                                                    this.list_state.set_follow_mode(
                                                                        FollowMode::Tail,
                                                                    );
                                                                    if this.items.len() > MAX_TAIL_ITEMS {
                                                                        this.pending_trim_tail = true;
                                                                    }
                                                                    cx.notify();
                                                                })),
                                                        ),
                                                )
                                            }),
                                    )
                                    .child(self.render_composer_band(window, cx)),
                            )
                            .children(self.render_right_column(cx)),
                    ),
            )
            .children(self.render_floating_popups(cx))
    }
}

fn session_key(row: &SessionRow) -> String {
    if !row.id.is_empty() {
        row.id.clone()
    } else {
        row.session_file.clone()
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
    fn slash_dispatch_routes_window_and_daemon_commands_without_leaking() {
        assert_eq!(slash_dispatch("/compact"), SlashAction::SendDaemonCommand);
        assert_eq!(slash_dispatch("/refine"), SlashAction::SendDaemonCommand);
        assert_eq!(slash_dispatch("/model"), SlashAction::Model);
        assert_eq!(slash_dispatch("/name sprint 2"), SlashAction::Rename("sprint 2".into()));
        assert_eq!(slash_dispatch("/system-prompt"), SlashAction::NotWired("system-prompt"));
        assert_eq!(slash_dispatch("hello"), SlashAction::SendMessage);
    }

    #[::core::prelude::v1::test]
    fn row_icon_animation_curves_stay_bounded_and_vary() {
        for ms in [0u64, 200, 400, 600, 800, 1000, 1200, 1450] {
            let r = python_ripple(ms, 320);
            let b = bash_cursor_mix(ms);
            let t = thinking_sweep(ms);
            let p = pulse_mix(ms);
            assert!((0.0..=1.0).contains(&r));
            assert!((0.0..=1.0).contains(&b));
            assert!((0.0..=1.0).contains(&t));
            assert!((0.0..=1.0).contains(&p));
        }
        assert!(bash_cursor_mix(100) > 0.99);
        assert!(bash_cursor_mix(800) < 0.01);
        assert!((python_ripple(0, 0) - python_ripple(0, 320)).abs() > 0.1);
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
        // Saved preference not supported by the model clamps cleanly.
        assert_eq!(
            effort_ladder_selected_level(false, &levels, "max", Some("medium")),
            Some("high".into())
        );
    }

    #[::core::prelude::v1::test]
    fn busy_strip_helpers_resolve_labels_spinner_and_geometry() {
        assert_eq!(resolve_busy_label(Some("compact"), "Thinking", false), "Compacting");
        assert_eq!(resolve_busy_label(Some("refine"), "Writing", false), "Refining");
        assert_eq!(resolve_busy_label(Some("refine"), "Compacting", true), "Compacting");
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
}
