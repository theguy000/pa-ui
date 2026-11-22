use crate::composer::{parse_action_update_lanes, Attachment, CHAT_COLUMN_MAX_WIDTH};
use crate::composer_model::ComposerModel;
use crate::daemon::{DaemonBridge, DaemonCmd, DaemonEvent};
use crate::diff_model::DiffModel;
use crate::popups::OpenPopup;
use crate::providers::{
    delete_custom_endpoint, fetch_models_for_endpoint, load_snapshot, remove_builtin_auth,
    save_builtin_config, upsert_custom_endpoint, AuthKind, BuiltinProviderRow, CustomEndpointRow,
    ModelDefinition, ProviderConfig, ProviderSnapshot,
};
use crate::roster::SessionRoster;
use crate::settings::AppSettings;
use crate::sidebar::RenameTarget;
use crate::theme::PaPalette;
use crate::transcript::{
    append_text_delta, append_thinking_delta, build_items, compute_tail_trim,
    find_tail_merge_start, finish_tool_call, format_expanded_body, hydrate_markdown, settle_all,
    start_tool_call, TranscriptItem,
};
use gpui_kit::component::{
    diff::{DiffEvent, DiffMode, DiffState},
    h_flex,
    input::{IndentInline, InputEvent, InputState, MoveDown, MoveUp, TextareaState},
    scroll::ScrollableElement as _,
    text::TextViewState,
    v_flex, Icon, IconName,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use pa_client::activity_label;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Instant;

const OLDER_PAGE_LIMIT: usize = 80;
const LOAD_OLDER_THRESHOLD: usize = 10;
const DEHYDRATE_RADIUS: usize = 50;
const MAX_TAIL_ITEMS: usize = 220;
const KEEP_TAIL_ITEMS: usize = 140;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsTab {
    General,
    Providers,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ProvidersEditMode {
    List,
    EditBuiltin {
        id: String,
        display_name: String,
        env_var: Option<&'static str>,
        is_oauth: bool,
    },
    EditCustom {
        old_id: Option<String>,
        api: String,
        headers: Vec<(Entity<InputState>, Entity<InputState>)>,
        models: Vec<ModelDefinition>,
        fetched_models: Vec<ModelDefinition>,
        model_editor_open: bool,
        editing_model_idx: Option<usize>,
        model_reasoning: bool,
        model_images: bool,
    },
}

pub struct PaApp {
    pub(crate) bridge: DaemonBridge,
    pub(crate) status: SharedString,
    pub(crate) notice: Option<SharedString>,
    pub(crate) roster: SessionRoster,
    pub(crate) composer_model: ComposerModel,
    pub(crate) diff_model: DiffModel,
    pub(crate) diff_state: Entity<DiffState>,
    pub(crate) renaming: Option<RenameTarget>,
    pub(crate) items: Vec<TranscriptItem>,
    pub(crate) next_item_id: usize,
    pub(crate) oldest_entry_id: Option<String>,
    pub(crate) has_older: bool,
    pub(crate) loading_older: bool,
    pub(crate) pending_trim_tail: bool,
    pub(crate) last_dehydrate_center: usize,
    pub(crate) scrolled_up: bool,
    pub(crate) is_streaming: bool,
    pub(crate) rail_mark_hovered: bool,
    pub(crate) anim_epoch: Instant,
    pub(crate) anim_phase_ms: u64,
    pub(crate) settings: AppSettings,
    pub(crate) settings_tab: SettingsTab,
    pub(crate) providers_snapshot: ProviderSnapshot,
    pub(crate) providers_mode: ProvidersEditMode,
    pub(crate) providers_more_open: bool,
    pub(crate) providers_fetching_models: bool,
    pub(crate) providers_status: Option<(bool, String)>,
    pub(crate) search_open: bool,
    pub(crate) find_open: bool,
    pub(crate) find_index: usize,
    pub(crate) open_popup: OpenPopup,
    pub(crate) collapsed_providers: HashSet<String>,
    pub(crate) hovered_model: Option<(String, String, f32)>,
    pub(crate) model_menu_scroll: ScrollHandle,
    pub(crate) attach_tx: async_channel::Sender<Vec<Attachment>>,
    pub(crate) fetch_models_tx: async_channel::Sender<Result<Vec<ModelDefinition>, String>>,
    pub(crate) list_state: ListState,
    pub(crate) search_input: Entity<InputState>,
    pub(crate) find_input: Entity<InputState>,
    pub(crate) rename_input: Entity<InputState>,
    pub(crate) prov_id_input: Entity<InputState>,
    pub(crate) prov_name_input: Entity<InputState>,
    pub(crate) prov_url_input: Entity<InputState>,
    pub(crate) prov_key_input: Entity<InputState>,
    pub(crate) prov_model_id_input: Entity<InputState>,
    pub(crate) prov_model_name_input: Entity<InputState>,
    pub(crate) prov_model_ctx_input: Entity<InputState>,
    pub(crate) prov_model_max_input: Entity<InputState>,
    pub(crate) composer: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
    _daemon_task: Task<()>,
    _attach_task: Task<()>,
    _fetch_models_task: Task<()>,
}

impl PaApp {
    pub fn new(settings: AppSettings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (bridge, ev_rx) = DaemonBridge::spawn();
        let search_input = cx.new(|cx| InputState::new(window, cx).placeholder("Search sessions…"));
        let find_input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in transcript…"));
        let rename_input = cx.new(|cx| InputState::new(window, cx).placeholder("New session name…"));
        let prov_id_input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. my_gateway"));
        let prov_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("e.g. My Gateway"));
        let prov_url_input = cx.new(|cx| InputState::new(window, cx).placeholder("https://api.example.com/v1"));
        let prov_key_input = cx.new(|cx| InputState::new(window, cx).placeholder("API key or env var (optional for local)"));
        let prov_model_id_input = cx.new(|cx| InputState::new(window, cx).placeholder("Model ID (e.g. gpt-4o)"));
        let prov_model_name_input = cx.new(|cx| InputState::new(window, cx).placeholder("Display name (optional)"));
        let prov_model_ctx_input = cx.new(|cx| InputState::new(window, cx).placeholder("128000"));
        let prov_model_max_input = cx.new(|cx| InputState::new(window, cx).placeholder("16384"));
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
                InputEvent::Change => {
                    this.sync_slash_decorations(cx);
                    this.sync_stop_target(cx);
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => {
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
                            this.composer_model.push_attachments(found);
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

        let (fetch_models_tx, fetch_models_rx) =
            async_channel::unbounded::<Result<Vec<ModelDefinition>, String>>();
        let weak_fetch = cx.entity().downgrade();
        let fetch_models_task = cx.spawn(async move |_this, cx| {
            while let Ok(res) = fetch_models_rx.recv().await {
                let keep_going = weak_fetch
                    .update(cx, |this, cx| {
                        this.providers_fetching_models = false;
                        match res {
                            Ok(fetched) => {
                                if let ProvidersEditMode::EditCustom {
                                    models,
                                    fetched_models,
                                    ..
                                } = &mut this.providers_mode
                                {
                                    let available: Vec<ModelDefinition> = fetched
                                        .into_iter()
                                        .filter(|m| !models.iter().any(|e| e.id == m.id))
                                        .collect();
                                    if available.is_empty() {
                                        fetched_models.clear();
                                        this.providers_status = Some((
                                            false,
                                            "No additional models found on this endpoint."
                                                .to_string(),
                                        ));
                                    } else {
                                        *fetched_models = available;
                                        this.providers_status = None;
                                    }
                                }
                            }
                            Err(err) => {
                                this.providers_status = Some((true, err));
                            }
                        }
                        cx.notify();
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

        let initial_diff_mode = if settings.diff_split {
            DiffMode::Split
        } else {
            DiffMode::Unified
        };
        let diff_state =
            cx.new(|cx| DiffState::new(Vec::new(), cx).with_mode(initial_diff_mode));
        cx.subscribe(&diff_state, |this, _, event: &DiffEvent, cx| match event {
            DiffEvent::FileExpanded(path) => {
                this.on_diff_file_expanded(path.as_ref(), cx);
            }
            DiffEvent::FileCollapsed(path) => {
                this.diff_model.set_file_collapsed(path.as_ref(), true);
                cx.notify();
            }
            _ => {}
        })
        .detach();

        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.on_window_focused(cx);
            }
        })
        .detach();

        Self {
            bridge,
            status: "Connecting…".into(),
            notice: None,
            roster: SessionRoster::new(),
            composer_model: ComposerModel::new(),
            diff_model: DiffModel::new(settings.diff_width),
            diff_state,
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
            rail_mark_hovered: false,
            anim_epoch: Instant::now(),
            anim_phase_ms: 0,
            settings,
            settings_tab: SettingsTab::General,
            providers_snapshot: ProviderSnapshot::default(),
            providers_mode: ProvidersEditMode::List,
            providers_more_open: false,
            providers_fetching_models: false,
            providers_status: None,
            search_open: false,
            find_open: false,
            find_index: 0,
            open_popup: OpenPopup::None,
            collapsed_providers: HashSet::new(),
            hovered_model: None,
            model_menu_scroll: ScrollHandle::new(),
            attach_tx,
            fetch_models_tx,
            list_state,
            search_input,
            find_input,
            rename_input,
            prov_id_input,
            prov_name_input,
            prov_url_input,
            prov_key_input,
            prov_model_id_input,
            prov_model_name_input,
            prov_model_ctx_input,
            prov_model_max_input,
            composer,
            _subscriptions: vec![sub_composer, sub_search, sub_find, sub_rename],
            _daemon_task: daemon_task,
            _attach_task: attach_task,
            _fetch_models_task: fetch_models_task,
        }
    }

    pub(crate) fn palette(&self) -> PaPalette {
        PaPalette::for_mode(self.settings.dark_mode)
    }

    pub(crate) fn sidebar_width(&self) -> f32 {
        if self.settings.sidebar_open {
            244.0
        } else {
            44.0
        }
    }

    pub(crate) fn right_column_width(&self, window: &Window) -> f32 {
        if !self.settings.right_open {
            return 0.0;
        }
        let viewport_w = window.viewport_size().width.as_f32();
        self.diff_model
            .effective_width(viewport_w, self.sidebar_width())
    }

    pub(crate) fn toggle_right_column(&mut self, cx: &mut Context<Self>) {
        self.settings.right_open = !self.settings.right_open;
        if !self.settings.right_open {
            self.diff_model.cancel_drag();
        }
        self.settings.save();
        if self.settings.right_open {
            self.refresh_diff(cx);
        } else {
            cx.notify();
        }
    }

    pub(crate) fn begin_right_column_drag(
        &mut self,
        mouse_x: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let viewport_w = window.viewport_size().width.as_f32();
        let sidebar_w = self.sidebar_width();
        self.diff_model.begin_drag(mouse_x, viewport_w, sidebar_w);
        cx.notify();
    }

    pub(crate) fn finish_right_column_drag(&mut self) -> bool {
        if let Some(width) = self.diff_model.end_drag() {
            self.settings.diff_width = width;
            self.settings.save();
            true
        } else {
            false
        }
    }

    pub(crate) fn reset_right_column_width(&mut self, cx: &mut Context<Self>) {
        self.settings.diff_width = self.diff_model.reset_width();
        self.settings.save();
        cx.notify();
    }

    pub(crate) fn refresh_providers_snapshot(&mut self) {
        self.providers_snapshot = load_snapshot();
    }

    pub(crate) fn refresh_daemon_models(&self) {
        let session = self.roster.active_id().map(str::to_string);
        let _ = self.bridge.tx.send(DaemonCmd::RefreshModels { session });
    }

    pub(crate) fn open_builtin_provider_editor(
        &mut self,
        row: &BuiltinProviderRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.providers_status = None;
        let current_key = match &row.auth {
            AuthKind::ApiKey(k) => k.clone(),
            _ => String::new(),
        };
        let is_oauth = matches!(row.auth, AuthKind::OAuth);
        let current_url = row.base_url_override.clone().unwrap_or_default();
        self.prov_key_input.update(cx, |input, cx| {
            input.set_value(current_key, window, cx);
        });
        self.prov_url_input.update(cx, |input, cx| {
            input.set_value(current_url, window, cx);
        });
        self.providers_mode = ProvidersEditMode::EditBuiltin {
            id: row.id.clone(),
            display_name: row.display_name.clone(),
            env_var: row.env_var,
            is_oauth,
        };
        cx.notify();
    }

    pub(crate) fn open_custom_endpoint_editor(
        &mut self,
        existing: Option<&CustomEndpointRow>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.providers_status = None;
        let (old_id, id_val, name_val, api, url_val, key_val, header_pairs, models) =
            if let Some(row) = existing {
                let pairs: Vec<(String, String)> = row
                    .config
                    .headers
                    .as_ref()
                    .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                (
                    Some(row.id.clone()),
                    row.id.clone(),
                    row.config.name.clone().unwrap_or_default(),
                    row.config
                        .api
                        .clone()
                        .unwrap_or_else(|| "openai-completions".to_string()),
                    row.config.base_url.clone().unwrap_or_default(),
                    row.config.api_key.clone().unwrap_or_default(),
                    pairs,
                    row.config.models.clone().unwrap_or_default(),
                )
            } else {
                (
                    None,
                    String::new(),
                    String::new(),
                    "openai-completions".to_string(),
                    String::new(),
                    String::new(),
                    Vec::new(),
                    Vec::new(),
                )
            };

        let headers = header_pairs
            .into_iter()
            .map(|(k, v)| {
                let ki = cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder("Header name");
                    s.set_value(k, window, cx);
                    s
                });
                let vi = cx.new(|cx| {
                    let mut s = InputState::new(window, cx).placeholder("Header value");
                    s.set_value(v, window, cx);
                    s
                });
                (ki, vi)
            })
            .collect();

        self.prov_id_input
            .update(cx, |input, cx| input.set_value(id_val, window, cx));
        self.prov_name_input
            .update(cx, |input, cx| input.set_value(name_val, window, cx));
        self.prov_url_input
            .update(cx, |input, cx| input.set_value(url_val, window, cx));
        self.prov_key_input
            .update(cx, |input, cx| input.set_value(key_val, window, cx));

        self.providers_mode = ProvidersEditMode::EditCustom {
            old_id,
            api,
            headers,
            models,
            fetched_models: Vec::new(),
            model_editor_open: false,
            editing_model_idx: None,
            model_reasoning: true,
            model_images: true,
        };
        self.clear_model_inputs(window, cx);
        cx.notify();
    }

    pub(crate) fn add_header_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ki = cx.new(|cx| InputState::new(window, cx).placeholder("Header name"));
        let vi = cx.new(|cx| InputState::new(window, cx).placeholder("Header value"));
        if let ProvidersEditMode::EditCustom { headers, .. } = &mut self.providers_mode {
            headers.push((ki, vi));
        }
        cx.notify();
    }

    pub(crate) fn remove_header_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let ProvidersEditMode::EditCustom { headers, .. } = &mut self.providers_mode {
            if idx < headers.len() {
                headers.remove(idx);
            }
        }
        cx.notify();
    }

    pub(crate) fn add_fetched_model(&mut self, id: &str, cx: &mut Context<Self>) {
        if let ProvidersEditMode::EditCustom {
            models,
            fetched_models,
            ..
        } = &mut self.providers_mode
        {
            if let Some(pos) = fetched_models.iter().position(|m| m.id == id) {
                let model = fetched_models.remove(pos);
                if !models.iter().any(|m| m.id == model.id) {
                    models.push(model);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn clear_fetched_models(&mut self, cx: &mut Context<Self>) {
        if let ProvidersEditMode::EditCustom { fetched_models, .. } = &mut self.providers_mode {
            fetched_models.clear();
        }
        cx.notify();
    }

    pub(crate) fn trigger_fetch_models(&mut self, cx: &mut Context<Self>) {
        if self.providers_fetching_models {
            return;
        }
        let ProvidersEditMode::EditCustom {
            api,
            headers: header_inputs,
            ..
        } = &self.providers_mode
        else {
            return;
        };
        let api = api.clone();
        let base_url = self.prov_url_input.read(cx).value().trim().to_string();
        let api_key = self.prov_key_input.read(cx).value().trim().to_string();
        let mut headers = std::collections::BTreeMap::new();
        for (ki, vi) in header_inputs {
            let k = ki.read(cx).value().trim().to_string();
            let v = vi.read(cx).value().trim().to_string();
            if !k.is_empty() {
                headers.insert(k, v);
            }
        }
        self.providers_fetching_models = true;
        self.providers_status = None;
        let tx = self.fetch_models_tx.clone();
        std::thread::spawn(move || {
            let res = fetch_models_for_endpoint(&api, &base_url, &api_key, &headers);
            let _ = tx.try_send(res);
        });
        cx.notify();
    }

    pub(crate) fn open_new_model_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.providers_status = None;
        self.clear_model_inputs(window, cx);
        if let ProvidersEditMode::EditCustom {
            model_editor_open,
            editing_model_idx,
            model_reasoning,
            model_images,
            ..
        } = &mut self.providers_mode
        {
            *model_editor_open = true;
            *editing_model_idx = None;
            *model_reasoning = true;
            *model_images = true;
        }
        cx.notify();
    }

    fn clear_model_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prov_model_id_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prov_model_name_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.prov_model_ctx_input
            .update(cx, |input, cx| input.set_value("128000", window, cx));
        self.prov_model_max_input
            .update(cx, |input, cx| input.set_value("16384", window, cx));
    }

    pub(crate) fn load_model_draft(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let model = match &self.providers_mode {
            ProvidersEditMode::EditCustom { models, .. } => models.get(idx).cloned(),
            _ => None,
        };
        let Some(m) = model else {
            return;
        };
        let reasoning = m.reasoning.unwrap_or(false);
        let images = m
            .input
            .as_ref()
            .is_some_and(|list| list.iter().any(|s| s == "image"));
        self.prov_model_id_input
            .update(cx, |i, cx| i.set_value(m.id.clone(), window, cx));
        self.prov_model_name_input.update(cx, |i, cx| {
            i.set_value(m.name.clone().unwrap_or_default(), window, cx)
        });
        self.prov_model_ctx_input.update(cx, |i, cx| {
            i.set_value(
                m.context_window.unwrap_or(128000).to_string(),
                window,
                cx,
            )
        });
        self.prov_model_max_input.update(cx, |i, cx| {
            i.set_value(m.max_tokens.unwrap_or(16384).to_string(), window, cx)
        });
        if let ProvidersEditMode::EditCustom {
            model_editor_open,
            editing_model_idx,
            model_reasoning,
            model_images,
            ..
        } = &mut self.providers_mode
        {
            *model_editor_open = true;
            *editing_model_idx = Some(idx);
            *model_reasoning = reasoning;
            *model_images = images;
        }
        cx.notify();
    }

    pub(crate) fn commit_model_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mid = self.prov_model_id_input.read(cx).value().trim().to_string();
        if mid.is_empty() {
            self.providers_status = Some((true, "Model ID is required.".to_string()));
            cx.notify();
            return;
        }
        let mname = self.prov_model_name_input.read(cx).value().trim().to_string();
        let ctx: u64 = self
            .prov_model_ctx_input
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(128000)
            .max(1);
        let max_tok: u64 = self
            .prov_model_max_input
            .read(cx)
            .value()
            .trim()
            .parse()
            .unwrap_or(16384)
            .max(1);

        if let ProvidersEditMode::EditCustom {
            models,
            model_editor_open,
            editing_model_idx,
            model_reasoning,
            model_images,
            ..
        } = &mut self.providers_mode
        {
            let mut input_modalities = vec!["text".to_string()];
            if *model_images {
                input_modalities.push("image".to_string());
            }
            let existing_extra = editing_model_idx
                .and_then(|idx| models.get(idx))
                .map(|m| m.extra.clone())
                .unwrap_or_default();
            let def = ModelDefinition {
                id: mid,
                name: if mname.is_empty() { None } else { Some(mname) },
                api: None,
                reasoning: Some(*model_reasoning),
                input: Some(input_modalities),
                context_window: Some(ctx),
                max_tokens: Some(max_tok),
                extra: existing_extra,
            };
            if let Some(idx) = editing_model_idx.take() {
                if idx < models.len() {
                    models[idx] = def;
                } else {
                    models.push(def);
                }
            } else if let Some(pos) = models.iter().position(|m| m.id == def.id) {
                models[pos] = def;
            } else {
                models.push(def);
            }
            *model_editor_open = false;
            *model_reasoning = true;
            *model_images = true;
        }
        self.providers_status = None;
        self.clear_model_inputs(window, cx);
        cx.notify();
    }

    pub(crate) fn remove_model_draft(
        &mut self,
        idx: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let ProvidersEditMode::EditCustom {
            models,
            editing_model_idx,
            ..
        } = &mut self.providers_mode
        {
            if idx < models.len() {
                models.remove(idx);
            }
            if *editing_model_idx == Some(idx) {
                *editing_model_idx = None;
                self.clear_model_inputs(window, cx);
            } else if let Some(cur) = editing_model_idx {
                if *cur > idx {
                    *cur -= 1;
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn save_builtin_provider(&mut self, cx: &mut Context<Self>) {
        let ProvidersEditMode::EditBuiltin {
            id,
            display_name,
            is_oauth,
            ..
        } = &self.providers_mode
        else {
            return;
        };
        let key_val = self.prov_key_input.read(cx).value().to_string();
        let url_val = self.prov_url_input.read(cx).value().to_string();
        let api_key_arg = if *is_oauth && key_val.trim().is_empty() {
            None
        } else {
            Some(key_val.as_str())
        };
        match save_builtin_config(id, api_key_arg, Some(url_val.as_str())) {
            Ok(()) => {
                self.providers_status = Some((false, format!("Saved {display_name}.")));
                self.providers_mode = ProvidersEditMode::List;
                self.refresh_providers_snapshot();
                self.refresh_daemon_models();
            }
            Err(err) => {
                self.providers_status = Some((true, err));
            }
        }
        cx.notify();
    }

    pub(crate) fn remove_builtin_provider_auth(&mut self, id: &str, cx: &mut Context<Self>) {
        match remove_builtin_auth(id) {
            Ok(()) => {
                self.providers_status = None;
                self.providers_mode = ProvidersEditMode::List;
                self.refresh_providers_snapshot();
                self.refresh_daemon_models();
            }
            Err(err) => {
                self.providers_status = Some((true, err));
            }
        }
        cx.notify();
    }

    pub(crate) fn save_custom_endpoint(&mut self, cx: &mut Context<Self>) {
        let ProvidersEditMode::EditCustom {
            old_id,
            api,
            headers: header_inputs,
            models,
            ..
        } = &self.providers_mode
        else {
            return;
        };
        let new_id = self.prov_id_input.read(cx).value().trim().to_string();
        let name = self.prov_name_input.read(cx).value().trim().to_string();
        let url = self.prov_url_input.read(cx).value().trim().to_string();
        let key = self.prov_key_input.read(cx).value().trim().to_string();
        let mut hdr_map = std::collections::BTreeMap::new();
        for (ki, vi) in header_inputs {
            let k = ki.read(cx).value().trim().to_string();
            let v = vi.read(cx).value().trim().to_string();
            if !k.is_empty() {
                hdr_map.insert(k, v);
            }
        }
        let headers = if hdr_map.is_empty() {
            None
        } else {
            Some(hdr_map)
        };

        let existing_extra = old_id
            .as_deref()
            .and_then(|oid| self.providers_snapshot.custom.iter().find(|c| c.id == oid))
            .map(|c| c.config.extra.clone())
            .unwrap_or_default();

        let cfg = ProviderConfig {
            name: if name.is_empty() { None } else { Some(name) },
            base_url: Some(url),
            api_key: if key.is_empty() { None } else { Some(key) },
            api: Some(api.clone()),
            headers,
            auth_header: None,
            models: Some(models.clone()),
            extra: existing_extra,
        };

        match upsert_custom_endpoint(old_id.as_deref(), &new_id, cfg) {
            Ok(()) => {
                self.providers_status = Some((false, format!("Saved Custom Endpoint '{new_id}'.")));
                self.providers_mode = ProvidersEditMode::List;
                self.refresh_providers_snapshot();
                self.refresh_daemon_models();
            }
            Err(err) => {
                self.providers_status = Some((true, err));
            }
        }
        cx.notify();
    }

    pub(crate) fn delete_custom_endpoint_entry(&mut self, id: &str, cx: &mut Context<Self>) {
        match delete_custom_endpoint(id) {
            Ok(()) => {
                self.providers_status = None;
                self.providers_mode = ProvidersEditMode::List;
                self.refresh_providers_snapshot();
                self.refresh_daemon_models();
            }
            Err(err) => {
                self.providers_status = Some((true, err));
            }
        }
        cx.notify();
    }

    pub(crate) fn sync_stop_target(&mut self, cx: &Context<Self>) {
        let composer_empty = self.composer.read(cx).value().trim().is_empty();
        self.composer_model
            .sync_stop_target(self.is_streaming, composer_empty);
    }

    fn advance_stop_anim(&mut self, window: &Window) {
        if self.composer_model.advance_animations(self.is_streaming) {
            window.request_animation_frame();
        }
    }

    pub(crate) fn request_new_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_popup = OpenPopup::None;
        self.hovered_model = None;
        self.roster.begin_draft();
        self.is_streaming = false;
        self.composer_model.clear_busy();
        self.composer_model.clear_queues();
        self.oldest_entry_id = None;
        self.has_older = false;
        self.loading_older = false;
        self.pending_trim_tail = false;
        self.scrolled_up = false;
        self.items.clear();
        self.list_state.reset(0);
        self.list_state.set_follow_mode(FollowMode::Tail);
        self.notice = None;
        self.sync_stop_target(cx);
        self.sync_diff_for_active_session(cx);
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
            self.roster.active_id().map(str::to_string),
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
                self.composer_model.set_pending_slash(None);
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
            }
            DaemonEvent::SessionCreated(new_sid) => {
                self.roster.clear_draft();
                self.roster.set_active_id(Some(new_sid));
                self.notice = None;
                self.sync_stop_target(cx);
                self.sync_diff_for_active_session(cx);
            }
            DaemonEvent::Sessions(rows) => {
                if let Some(auto_sid) = self.roster.apply_sessions(rows) {
                    self.roster.set_active_id(Some(auto_sid));
                }
                if let Some(streaming) = self.roster.active_row_is_streaming() {
                    self.is_streaming = streaming;
                }
                self.sync_stop_target(cx);
                self.sync_diff_for_active_session(cx);
            }
            DaemonEvent::Messages {
                session,
                messages,
                first_entry,
                more,
            } => {
                let switching = self.roster.active_id() != Some(session.as_str());
                if switching {
                    self.composer_model.clear_busy();
                    self.composer_model.cancel_edit_queue();
                }
                self.roster.clear_draft();
                self.roster.set_active_id(Some(session));
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
                self.sync_diff_for_active_session(cx);
            }
            DaemonEvent::TailRefresh {
                session,
                messages,
                first_entry,
                more,
            } => {
                if self.roster.active_id() != Some(session.as_str()) {
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
                if self.roster.active_id() != Some(session.as_str()) {
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
                if self.roster.models().is_empty() {
                    let cur_prov = self.roster.current_model().provider;
                    self.collapsed_providers
                        .extend(models.iter().map(|m| m.provider.clone()));
                    if !cur_prov.is_empty() {
                        self.collapsed_providers.remove(&cur_prov);
                    }
                }
                self.roster.apply_models(models, &self.settings);
            }
            DaemonEvent::SessionState { session, state } => {
                if self.roster.active_id() == Some(session.as_str()) {
                    if !state.provider.is_empty() {
                        self.collapsed_providers.remove(&state.provider);
                    }
                    self.is_streaming = state.is_streaming;
                    self.roster.apply_session_state(&session, state);
                    self.sync_stop_target(cx);
                }
            }
            DaemonEvent::Stats { session, stats } => {
                self.roster.apply_stats(&session, stats);
            }
            DaemonEvent::Queue {
                session,
                steer,
                follow_up,
            } => {
                if self.roster.active_id() == Some(session.as_str()) {
                    self.composer_model.set_queues(steer, follow_up);
                    if self.composer_model.queued_total() == 0
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
                if self.roster.active_id() == Some(session.as_str()) {
                    self.roster.apply_activity(subagents, heartbeats, shells);
                }
            }
            DaemonEvent::Notice(msg) => {
                self.notice = Some(msg.into());
            }
            DaemonEvent::ExportResult {
                kind,
                primary,
                secondary,
            } => {
                if !primary.is_empty() {
                    cx.write_to_clipboard(ClipboardItem::new_string(primary.clone()));
                }
                self.open_popup = OpenPopup::ExportResult {
                    kind,
                    primary,
                    secondary,
                    copied: Some("primary"),
                };
            }
            DaemonEvent::LiveFrame(frame) => {
                self.handle_live_frame(&frame, cx);
            }
            DaemonEvent::Error(err) => {
                self.is_streaming = false;
                self.composer_model.set_pending_slash(None);
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
                    self.composer_model.set_pending_slash(None);
                    settle_all(&mut self.items);
                    let err = frame
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Command rejected");
                    self.notice = Some(err.to_string().into());
                    self.sync_stop_target(cx);
                } else if self.composer_model.take_pending_slash().is_some() {
                    self.is_streaming = false;
                    if matches!(
                        self.notice.as_deref(),
                        Some("Compacting…") | Some("Refining…")
                    ) {
                        self.notice = None;
                    }
                    settle_all(&mut self.items);
                    self.sync_stop_target(cx);
                    if let Some(sid) = self.roster.active_id().map(str::to_string) {
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
        let matches_active = match (frame_sid, self.roster.active_id()) {
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
                if !self.composer_model.has_busy_label() {
                    self.composer_model.set_busy_label("Waiting", false);
                }
                self.roster.mark_active_streaming();
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
                    self.composer_model.set_busy_label(label, false);
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
                self.composer_model.set_busy_label("Running", false);
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
                    self.composer_model.set_busy_label("Waiting", false);
                }
                self.schedule_debounced_diff_refresh(cx);
            }
            "auto_retry_start" => {
                self.is_streaming = true;
                self.composer_model.set_busy_label("Retrying", false);
                self.sync_stop_target(cx);
            }
            "session_action_update" => {
                let (steer, follow_up) = parse_action_update_lanes(event);
                self.composer_model.set_queues(steer, follow_up);
                if self.composer_model.queued_total() == 0
                    && self.open_popup == OpenPopup::QueuePanel
                {
                    self.open_popup = OpenPopup::None;
                }
            }
            "compaction_start" => {
                self.is_streaming = true;
                self.composer_model.set_busy_label("Compacting", true);
                self.notice = Some("Compacting…".into());
                self.sync_stop_target(cx);
            }
            "compaction_end" => {
                self.is_streaming = false;
                self.composer_model.set_pending_slash(None);
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
                if let Some(sid) = self.roster.active_id().map(str::to_string) {
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
                        self.composer_model.set_pending_slash(None);
                        self.notice = None;
                        settle_all(&mut self.items);
                        self.sync_stop_target(cx);
                        if let Some(sid) = self.roster.active_id().map(str::to_string) {
                            let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                        }
                    }
                }
            }
            "agent_end" | "turn_end" => {
                self.is_streaming = false;
                self.composer_model.set_pending_slash(None);
                if matches!(
                    self.notice.as_deref(),
                    Some("Compacting…") | Some("Refining…")
                ) {
                    self.notice = None;
                }
                settle_all(&mut self.items);
                self.sync_stop_target(cx);
                if let Some(sid) = self.roster.active_id().map(str::to_string) {
                    let _ = self.bridge.tx.send(DaemonCmd::ReloadTranscript(sid));
                }
                self.schedule_debounced_diff_refresh(cx);
            }
            "session_updated" => {
                let _ = self.bridge.tx.send(DaemonCmd::RefreshSessions);
            }
            _ => {}
        }
    }

    pub(crate) fn toggle_item(&mut self, id: usize, cx: &mut Context<Self>) {
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

    pub(crate) fn copy_text(&self, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    pub(crate) fn find_matches(&self, cx: &Context<Self>) -> Vec<usize> {
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

    pub(crate) fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
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
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                if this.composer_model.is_dragging() {
                    if ev.pressed_button != Some(MouseButton::Left) {
                        this.composer_model.end_drag();
                        cx.notify();
                        return;
                    }
                    if this.composer_model.update_drag(ev.position.y.as_f32()) {
                        cx.notify();
                    }
                }
                if this.diff_model.is_dragging() {
                    if ev.pressed_button != Some(MouseButton::Left) {
                        if this.finish_right_column_drag() {
                            cx.notify();
                        }
                        return;
                    }
                    let viewport_w = window.viewport_size().width.as_f32();
                    let sidebar_w = this.sidebar_width();
                    if this
                        .diff_model
                        .update_drag(ev.position.x.as_f32(), viewport_w, sidebar_w)
                    {
                        cx.notify();
                    }
                }
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                let idx = this.composer_model.slash_index();
                if this.accept_slash_completion(idx, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                if this.step_slash_selection(1, cx) {
                    cx.stop_propagation();
                }
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                if this.step_slash_selection(-1, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    let changed =
                        this.composer_model.end_drag() | this.finish_right_column_drag();
                    if changed {
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
                if ctrl && ev.keystroke.modifiers.shift && key.eq_ignore_ascii_case("b") {
                    this.toggle_right_column(cx);
                } else if ctrl && key.eq_ignore_ascii_case("b") {
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
                    } else if this.composer_model.take_edit_queue().is_some() {
                        this.set_composer_text("", window, cx);
                        if this.notice.as_deref() == Some("Editing queued message · Esc to cancel") {
                            this.notice = None;
                        }
                        this.sync_stop_target(cx);
                        cx.notify();
                    } else if this.is_streaming {
                        if let Some(sid) = this.roster.active_id().map(str::to_string) {
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
                                                                    .max_w(px(CHAT_COLUMN_MAX_WIDTH))
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
                            .children(self.render_right_column(window, cx)),
                    ),
            )
            .children(self.render_floating_popups(window, cx))
    }
}
