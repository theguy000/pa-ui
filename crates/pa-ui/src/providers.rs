//! Provider and custom endpoint configuration backed by `~/.prime/agent/models.json`
//! and `~/.prime/agent/auth.json`, matching `pa-core`'s schema and lock protocol.

use crate::app::{PaApp, ProvidersEditMode};
use gpui_kit::component::{
    h_flex, scroll::ScrollableElement as _, v_flex, Icon, IconName, StyledExt as _,
};
use gpui_kit::{prelude::FluentBuilder as _, *};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const API_PROTOCOLS: &[(&str, &str)] = &[
    ("openai-completions", "OpenAI Compatible"),
    ("openai-responses", "OpenAI Responses"),
    ("anthropic-messages", "Anthropic Messages"),
    ("google-generative-ai", "Google Gemini"),
];

pub const BUILTIN_PROVIDERS: &[(&str, &str, Option<&str>)] = &[
    ("anthropic", "Anthropic", Some("ANTHROPIC_API_KEY")),
    ("openai", "OpenAI", Some("OPENAI_API_KEY")),
    ("openai-codex", "OpenAI Codex", None),
    ("google", "Google Gemini", Some("GEMINI_API_KEY")),
    ("openrouter", "OpenRouter", Some("OPENROUTER_API_KEY")),
    ("prime-inference", "Prime Inference", Some("PRIME_API_KEY")),
    ("deepseek", "DeepSeek", Some("DEEPSEEK_API_KEY")),
    ("groq", "Groq", Some("GROQ_API_KEY")),
    ("xai", "xAI", Some("XAI_API_KEY")),
    ("mistral", "Mistral", Some("MISTRAL_API_KEY")),
    ("cerebras", "Cerebras", Some("CEREBRAS_API_KEY")),
    ("fireworks", "Fireworks AI", Some("FIREWORKS_API_KEY")),
    ("huggingface", "Hugging Face", Some("HF_TOKEN")),
    ("github-copilot", "GitHub Copilot", Some("GITHUB_TOKEN")),
    ("amazon-bedrock", "Amazon Bedrock", Some("AWS_ACCESS_KEY_ID")),
    ("google-vertex", "Google Vertex AI", Some("GOOGLE_CLOUD_PROJECT")),
    ("azure-openai-responses", "Azure OpenAI", Some("AZURE_OPENAI_API_KEY")),
    ("cloudflare-ai-gateway", "Cloudflare AI Gateway", None),
    ("cloudflare-workers-ai", "Cloudflare Workers AI", Some("CLOUDFLARE_API_TOKEN")),
    ("kimi-coding", "Kimi Coding", Some("KIMI_API_KEY")),
    ("minimax", "MiniMax", Some("MINIMAX_API_KEY")),
    ("minimax-cn", "MiniMax CN", Some("MINIMAX_CN_API_KEY")),
    ("moonshotai", "Moonshot AI", Some("MOONSHOT_API_KEY")),
    ("moonshotai-cn", "Moonshot AI CN", Some("MOONSHOT_API_KEY")),
    ("opencode", "OpenCode", Some("OPENCODE_API_KEY")),
    ("opencode-go", "OpenCode Go", Some("OPENCODE_API_KEY")),
    ("vercel-ai-gateway", "Vercel AI Gateway", Some("AI_GATEWAY_API_KEY")),
    ("xiaomi", "Xiaomi", None),
    ("xiaomi-token-plan-ams", "Xiaomi Token Plan (AMS)", None),
    ("xiaomi-token-plan-cn", "Xiaomi Token Plan (CN)", None),
    ("xiaomi-token-plan-sgp", "Xiaomi Token Plan (SGP)", None),
    ("zai", "Z.AI", Some("ZAI_API_KEY")),
];

pub fn agent_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)?;
    Some(home.join(".prime").join("agent"))
}

pub fn is_builtin_provider(id: &str) -> bool {
    BUILTIN_PROVIDERS.iter().any(|(pid, _, _)| *pid == id)
}

pub fn protocol_label(api: &str) -> &str {
    API_PROTOCOLS
        .iter()
        .find(|(id, _)| *id == api)
        .map(|(_, label)| *label)
        .unwrap_or(api)
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelDefinition {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Preserves `thinkingLevelMap`, `cost`, `headers`, `compat`, etc.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelDefinition>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelsFile {
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthKind {
    ApiKey(String),
    OAuth,
    EnvVar(&'static str),
    None,
}

#[derive(Debug, Clone)]
pub struct BuiltinProviderRow {
    pub id: String,
    pub display_name: String,
    pub env_var: Option<&'static str>,
    pub auth: AuthKind,
    pub base_url_override: Option<String>,
}

impl BuiltinProviderRow {
    pub fn is_configured(&self) -> bool {
        !matches!(self.auth, AuthKind::None)
    }
}

#[derive(Debug, Clone)]
pub struct CustomEndpointRow {
    pub id: String,
    pub config: ProviderConfig,
}

#[derive(Debug, Clone, Default)]
pub struct ProviderSnapshot {
    pub builtins: Vec<BuiltinProviderRow>,
    pub custom: Vec<CustomEndpointRow>,
    pub load_error: Option<String>,
}

/// Strip `//` line comments and trailing commas outside string literals (JSONC).
pub fn strip_json_comments(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut no_comments = String::with_capacity(input.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            no_comments.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            no_comments.push(c);
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        no_comments.push(c);
        i += 1;
    }

    let chars: Vec<char> = no_comments.chars().collect();
    let mut cleaned = String::with_capacity(no_comments.len());
    let mut in_str = false;
    let mut escaped = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            cleaned.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            cleaned.push(c);
            i += 1;
            continue;
        }
        if c == ',' {
            let mut j = i + 1;
            while j < chars.len() && chars[j].is_whitespace() {
                j += 1;
            }
            if j < chars.len() && (chars[j] == '}' || chars[j] == ']') {
                i += 1;
                continue;
            }
        }
        cleaned.push(c);
        i += 1;
    }
    cleaned
}

fn read_models_file(dir: &Path) -> Result<ModelsFile, String> {
    let path = dir.join("models.json");
    if !path.exists() {
        return Ok(ModelsFile::default());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("Failed to read models.json: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(ModelsFile::default());
    }
    let cleaned = strip_json_comments(&raw);
    serde_json::from_str(&cleaned).map_err(|e| format!("Invalid models.json: {e}"))
}

fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, content).map_err(|e| format!("Failed to write temp file: {e}"))?;
    // On Windows, fs::rename replaces an existing target file since Rust 1.75+,
    // but fall back to remove+rename if needed.
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(path);
        fs::rename(&tmp, path).map_err(|e| format!("Failed to replace {}: {e}", path.display()))?;
    }
    Ok(())
}

fn write_models_file(dir: &Path, models: &ModelsFile) -> Result<(), String> {
    let json = serde_json::to_string_pretty(models)
        .map_err(|e| format!("Failed to serialize models.json: {e}"))?;
    write_atomic(&dir.join("models.json"), &json)
}

struct LockDirGuard {
    lock_path: PathBuf,
}

impl LockDirGuard {
    fn acquire(target: &Path) -> Result<Self, String> {
        let lock_path = PathBuf::from(format!("{}.lock", target.display()));
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create auth dir: {e}"))?;
        }
        for _ in 0..10 {
            match fs::create_dir(&lock_path) {
                Ok(()) => return Ok(Self { lock_path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Ok(meta) = fs::metadata(&lock_path) {
                        if let Ok(modified) = meta.modified() {
                            if SystemTime::now()
                                .duration_since(modified)
                                .unwrap_or_default()
                                > Duration::from_secs(10)
                            {
                                let _ = fs::remove_dir_all(&lock_path);
                                continue;
                            }
                        }
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("Failed to lock auth.json: {e}")),
            }
        }
        Err("auth.json is locked by another process; try again.".to_string())
    }
}

impl Drop for LockDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.lock_path);
    }
}

fn read_auth_map(dir: &Path) -> Result<BTreeMap<String, Value>, String> {
    let path = dir.join("auth.json");
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let raw = fs::read_to_string(&path).map_err(|e| format!("Failed to read auth.json: {e}"))?;
    if raw.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&raw).map_err(|e| format!("Invalid auth.json: {e}"))
}

fn mutate_auth_map(
    dir: &Path,
    f: impl FnOnce(&mut BTreeMap<String, Value>),
) -> Result<(), String> {
    let path = dir.join("auth.json");
    let _guard = LockDirGuard::acquire(&path)?;
    let mut map = read_auth_map(dir)?;
    f(&mut map);
    let json = serde_json::to_string_pretty(&map)
        .map_err(|e| format!("Failed to serialize auth.json: {e}"))?;
    write_atomic(&path, &json)
}

pub fn load_snapshot() -> ProviderSnapshot {
    let Some(dir) = agent_dir() else {
        return ProviderSnapshot {
            load_error: Some("Could not resolve home directory".to_string()),
            ..Default::default()
        };
    };
    load_snapshot_from(&dir)
}

pub fn load_snapshot_from(dir: &Path) -> ProviderSnapshot {
    let mut load_error = None;
    let models_file = match read_models_file(dir) {
        Ok(m) => m,
        Err(e) => {
            load_error = Some(e);
            ModelsFile::default()
        }
    };
    let auth_map = match read_auth_map(dir) {
        Ok(m) => m,
        Err(e) => {
            if load_error.is_none() {
                load_error = Some(e);
            }
            BTreeMap::new()
        }
    };

    let mut builtins: Vec<BuiltinProviderRow> = BUILTIN_PROVIDERS
        .iter()
        .map(|(id, name, env_var)| {
            let auth = match auth_map.get(*id) {
                Some(v) => match v.get("type").and_then(Value::as_str) {
                    Some("oauth") => AuthKind::OAuth,
                    Some("api_key") => AuthKind::ApiKey(
                        v.get("key")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    _ => AuthKind::None,
                },
                None => match env_var {
                    Some(var) if std::env::var_os(var).is_some_and(|v| !v.is_empty()) => {
                        AuthKind::EnvVar(var)
                    }
                    _ => AuthKind::None,
                },
            };
            let base_url_override = models_file
                .providers
                .get(*id)
                .and_then(|p| p.base_url.clone());
            BuiltinProviderRow {
                id: (*id).to_string(),
                display_name: (*name).to_string(),
                env_var: *env_var,
                auth,
                base_url_override,
            }
        })
        .collect();

    builtins.sort_by(|a, b| {
        (!a.is_configured())
            .cmp(&!b.is_configured())
            .then_with(|| a.display_name.to_lowercase().cmp(&b.display_name.to_lowercase()))
    });

    let custom: Vec<CustomEndpointRow> = models_file
        .providers
        .into_iter()
        .filter(|(id, _)| !is_builtin_provider(id))
        .map(|(id, config)| CustomEndpointRow { id, config })
        .collect();

    ProviderSnapshot {
        builtins,
        custom,
        load_error,
    }
}

pub fn save_builtin_config(
    provider_id: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<(), String> {
    let dir = agent_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    save_builtin_config_in(&dir, provider_id, api_key, base_url)
}

pub fn save_builtin_config_in(
    dir: &Path,
    provider_id: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<(), String> {
    if let Some(key) = api_key {
        let trimmed = key.trim();
        mutate_auth_map(dir, |map| {
            if trimmed.is_empty() {
                if map.get(provider_id).and_then(|v| v.get("type")).and_then(Value::as_str)
                    == Some("api_key")
                {
                    map.remove(provider_id);
                }
            } else {
                map.insert(
                    provider_id.to_string(),
                    serde_json::json!({ "type": "api_key", "key": trimmed }),
                );
            }
        })?;
    }

    let url_trimmed = base_url.map(str::trim).unwrap_or("");
    let mut models = read_models_file(dir)?;
    if url_trimmed.is_empty() {
        if let Some(entry) = models.providers.get_mut(provider_id) {
            entry.base_url = None;
            if entry.name.is_none()
                && entry.api_key.is_none()
                && entry.api.is_none()
                && entry.headers.is_none()
                && entry.models.as_ref().is_none_or(Vec::is_empty)
                && entry.extra.is_empty()
            {
                models.providers.remove(provider_id);
            }
            write_models_file(dir, &models)?;
        }
    } else {
        let entry = models.providers.entry(provider_id.to_string()).or_default();
        entry.base_url = Some(url_trimmed.to_string());
        write_models_file(dir, &models)?;
    }
    Ok(())
}

pub fn remove_builtin_auth(provider_id: &str) -> Result<(), String> {
    let dir = agent_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    remove_builtin_auth_in(&dir, provider_id)
}

pub fn remove_builtin_auth_in(dir: &Path, provider_id: &str) -> Result<(), String> {
    mutate_auth_map(dir, |map| {
        map.remove(provider_id);
    })
}

pub fn upsert_custom_endpoint(
    old_id: Option<&str>,
    new_id: &str,
    config: ProviderConfig,
) -> Result<(), String> {
    let dir = agent_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    upsert_custom_endpoint_in(&dir, old_id, new_id, config)
}

pub fn upsert_custom_endpoint_in(
    dir: &Path,
    old_id: Option<&str>,
    new_id: &str,
    mut config: ProviderConfig,
) -> Result<(), String> {
    let id = new_id.trim();
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
    {
        return Err("Provider ID must be lowercase letters, digits, '-' or '_'.".to_string());
    }
    if is_builtin_provider(id) {
        return Err(format!("'{id}' is a built-in provider ID."));
    }
    if config.base_url.as_deref().unwrap_or("").trim().is_empty() {
        return Err("Base URL is required.".to_string());
    }
    if config.api.as_deref().unwrap_or("").trim().is_empty() {
        return Err("Protocol is required.".to_string());
    }
    // `pa-core::models::custom::validate_config` requires `apiKey` on custom providers
    // with models; use `"local"` when left blank for keyless local proxies.
    if config.api_key.as_deref().unwrap_or("").trim().is_empty() {
        config.api_key = Some("local".to_string());
    }

    let mut models = read_models_file(dir)?;
    if let Some(prev) = old_id {
        if prev != id {
            models.providers.remove(prev);
        }
    }
    models.providers.insert(id.to_string(), config);
    write_models_file(dir, &models)
}

pub fn delete_custom_endpoint(provider_id: &str) -> Result<(), String> {
    let dir = agent_dir().ok_or_else(|| "Could not resolve home directory".to_string())?;
    delete_custom_endpoint_in(&dir, provider_id)
}

pub fn delete_custom_endpoint_in(dir: &Path, provider_id: &str) -> Result<(), String> {
    let mut models = read_models_file(dir)?;
    models.providers.remove(provider_id);
    write_models_file(dir, &models)
}

fn candidate_model_urls(api: &str, base_url: &str) -> Vec<String> {
    let base = base_url.trim().trim_end_matches('/');
    match api {
        "anthropic-messages" => {
            if base.ends_with("/v1") {
                vec![format!("{base}/models")]
            } else {
                vec![format!("{base}/v1/models"), format!("{base}/models")]
            }
        }
        "google-generative-ai" => {
            if base.contains("/v1") {
                vec![format!("{base}/models")]
            } else {
                vec![format!("{base}/v1beta/models"), format!("{base}/models")]
            }
        }
        _ => {
            if base.ends_with("/v1") {
                vec![format!("{base}/models")]
            } else {
                vec![format!("{base}/models"), format!("{base}/v1/models")]
            }
        }
    }
}

pub fn parse_fetched_models(api: &str, raw: &str) -> Result<Vec<ModelDefinition>, String> {
    let val: Value =
        serde_json::from_str(raw).map_err(|e| format!("Invalid JSON response: {e}"))?;

    if let Some(err_msg) = val
        .get("error")
        .and_then(|e| e.get("message").and_then(Value::as_str).or_else(|| e.as_str()))
    {
        return Err(err_msg.to_string());
    }

    let mut out = Vec::new();
    let list = if api == "google-generative-ai" {
        val.get("models")
            .and_then(Value::as_array)
            .or_else(|| val.get("data").and_then(Value::as_array))
            .or_else(|| val.as_array())
    } else {
        val.get("data")
            .and_then(Value::as_array)
            .or_else(|| val.get("models").and_then(Value::as_array))
            .or_else(|| val.as_array())
    };

    let Some(items) = list else {
        return Err("Response did not contain a models list.".to_string());
    };

    for item in items {
        let raw_id = item
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| item.get("name").and_then(Value::as_str))
            .unwrap_or("")
            .trim();
        let id = raw_id.strip_prefix("models/").unwrap_or(raw_id).to_string();
        if id.is_empty() {
            continue;
        }
        let name = item
            .get("displayName")
            .or_else(|| item.get("display_name"))
            .or_else(|| item.get("name").filter(|_| item.get("id").is_some()))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != id)
            .map(str::to_string);
        let context_window = item
            .get("inputTokenLimit")
            .or_else(|| item.get("context_length"))
            .or_else(|| item.get("contextWindow"))
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        let max_tokens = item
            .get("outputTokenLimit")
            .or_else(|| item.get("max_output_tokens"))
            .or_else(|| item.get("maxTokens"))
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        out.push(ModelDefinition {
            id,
            name,
            api: None,
            reasoning: None,
            input: Some(vec!["text".to_string()]),
            context_window,
            max_tokens,
            extra: BTreeMap::new(),
        });
    }

    Ok(out)
}

/// Fetch available models from any custom endpoint protocol using system `curl`.
pub fn fetch_models_for_endpoint(
    api: &str,
    base_url: &str,
    api_key: &str,
    headers: &BTreeMap<String, String>,
) -> Result<Vec<ModelDefinition>, String> {
    if base_url.trim().is_empty() {
        return Err("Enter a Base URL first.".to_string());
    }
    let has_header = |needle: &str| {
        headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case(needle))
    };

    let mut last_err = "Failed to fetch models.".to_string();
    for url in candidate_model_urls(api, base_url) {
        let mut cmd = std::process::Command::new("curl");
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt as _;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.args(["-sS", "-L", "--max-time", "12", "-w", "\n%{http_code}"]);
        cmd.args(["-H", "Accept: application/json"]);

        let key = api_key.trim();
        if !key.is_empty() {
            match api {
                "anthropic-messages" => {
                    if !has_header("x-api-key") {
                        cmd.args(["-H", &format!("x-api-key: {key}")]);
                    }
                    if !has_header("anthropic-version") {
                        cmd.args(["-H", "anthropic-version: 2023-06-01"]);
                    }
                }
                "google-generative-ai" => {
                    if !has_header("x-goog-api-key") {
                        cmd.args(["-H", &format!("x-goog-api-key: {key}")]);
                    }
                }
                _ => {
                    if !has_header("authorization") {
                        cmd.args(["-H", &format!("Authorization: Bearer {key}")]);
                    }
                }
            }
        } else if api == "anthropic-messages" && !has_header("anthropic-version") {
            cmd.args(["-H", "anthropic-version: 2023-06-01"]);
        }

        for (k, v) in headers {
            if !k.trim().is_empty() {
                cmd.args(["-H", &format!("{}: {}", k.trim(), v.trim())]);
            }
        }
        cmd.arg(&url);

        let output = cmd
            .output()
            .map_err(|e| format!("Failed to run curl: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            last_err = if stderr.is_empty() {
                format!("Request to {url} failed")
            } else {
                stderr
            };
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let (body, status_str) = stdout.rsplit_once('\n').unwrap_or((stdout.as_ref(), "0"));
        let status: u16 = status_str.trim().parse().unwrap_or(0);
        if !(200..300).contains(&status) {
            if let Err(msg) = parse_fetched_models(api, body) {
                last_err = format!("HTTP {status}: {msg}");
            } else {
                last_err = format!("HTTP {status} from {url}");
            }
            continue;
        }
        return parse_fetched_models(api, body);
    }

    Err(last_err)
}

impl PaApp {
    pub(crate) fn render_providers_settings_body(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = self.palette();
        let mut body = v_flex()
            .id("settings-providers-body")
            .flex_1()
            .overflow_y_scrollbar()
            .px(px(28.))
            .py(px(20.))
            .gap(px(20.));

        if let Some(err) = &self.providers_snapshot.load_error {
            body = body.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(p.danger_surface)
                    .text_xs()
                    .text_color(p.danger)
                    .child(err.clone()),
            );
        }

        if let Some((is_err, msg)) = &self.providers_status {
            body = body.child(
                div()
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(px(8.))
                    .bg(if *is_err {
                        p.danger_surface
                    } else {
                        p.sidebar_row_selected
                    })
                    .text_xs()
                    .text_color(if *is_err { p.danger } else { p.ink })
                    .child(msg.clone()),
            );
        }

        let field_label = |text: &'static str| {
            div()
                .text_size(px(11.5))
                .font_medium()
                .text_color(p.quiet_ink)
                .child(text)
        };

        match &self.providers_mode {
            ProvidersEditMode::List => {
                // Cross-check live daemon catalog so every configured provider (whether via
                // auth.json, env var, OAuth, or models.json) lands in Configured and NEVER in
                // the collapsed unconfigured list.
                let mut daemon_configured = std::collections::HashSet::<&str>::new();
                let mut model_counts = std::collections::HashMap::<&str, usize>::new();
                for m in self.roster.models() {
                    *model_counts.entry(m.provider.as_str()).or_insert(0) += 1;
                    if m.configured {
                        daemon_configured.insert(m.provider.as_str());
                    }
                }

                let configured_builtins: Vec<&BuiltinProviderRow> = self
                    .providers_snapshot
                    .builtins
                    .iter()
                    .filter(|b| b.is_configured() || daemon_configured.contains(b.id.as_str()))
                    .collect();

                let unconfigured_builtins: Vec<&BuiltinProviderRow> = self
                    .providers_snapshot
                    .builtins
                    .iter()
                    .filter(|b| !b.is_configured() && !daemon_configured.contains(b.id.as_str()))
                    .collect();

                let total_configured =
                    self.providers_snapshot.custom.len() + configured_builtins.len();

                // 1. Unified Configured section (all active custom endpoints + built-in providers)
                let mut configured_sec = v_flex().gap(px(10.)).child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap(px(2.))
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .font_semibold()
                                        .text_color(p.ink)
                                        .child("Configured"),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(p.quiet_ink)
                                        .child("Active providers and custom endpoints available in your model menu"),
                                ),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(p.quiet_ink)
                                .child(format!("{total_configured}")),
                        ),
                );

                if total_configured == 0 {
                    configured_sec = configured_sec.child(
                        div()
                            .px(px(14.))
                            .py(px(16.))
                            .rounded(px(10.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.window_surface)
                            .text_xs()
                            .text_color(p.quiet_ink)
                            .child("No providers configured yet. Add a Custom Endpoint above or pick a built-in provider below."),
                    );
                } else {
                    let mut list = v_flex()
                        .w_full()
                        .rounded(px(10.))
                        .border_1()
                        .border_color(p.sidebar_group_edge)
                        .bg(p.window_surface)
                        .overflow_hidden();

                    let mut row_i = 0usize;

                    // Custom endpoints
                    for row in &self.providers_snapshot.custom {
                        let title = row
                            .config
                            .name
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .unwrap_or(&row.id)
                            .to_string();
                        let proto = protocol_label(
                            row.config.api.as_deref().unwrap_or("openai-completions"),
                        );
                        let m_count = row
                            .config
                            .models
                            .as_ref()
                            .map(Vec::len)
                            .filter(|n| *n > 0)
                            .or_else(|| model_counts.get(row.id.as_str()).copied())
                            .unwrap_or(0);
                        let count_label = format!(
                            "{m_count} model{}",
                            if m_count == 1 { "" } else { "s" }
                        );
                        let row_edit = row.clone();
                        let idx = row_i;
                        row_i += 1;

                        list = list.child(
                            h_flex()
                                .id(("cfg-row", idx))
                                .w_full()
                                .h(px(42.))
                                .px(px(14.))
                                .when(idx == 0, |d| d.rounded_t(px(9.)))
                                .when(idx + 1 == total_configured, |d| d.rounded_b(px(9.)))
                                .when(idx > 0, |d| {
                                    d.border_t_1().border_color(p.sidebar_group_edge)
                                })
                                .items_center()
                                .justify_between()
                                .gap(px(12.))
                                .cursor_pointer()
                                .hover(|h| h.bg(p.sidebar_row_hover))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_custom_endpoint_editor(Some(&row_edit), window, cx);
                                }))
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(8.))
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .line_height(px(16.))
                                                .font_medium()
                                                .text_color(p.ink)
                                                .truncate()
                                                .child(title),
                                        )
                                        .child(
                                            h_flex()
                                                .flex_shrink_0()
                                                .h(px(18.))
                                                .px(px(6.))
                                                .rounded(px(4.))
                                                .bg(p.sidebar_row_selected)
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .line_height(px(10.))
                                                        .text_color(p.quiet_ink)
                                                        .child(proto.to_string()),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(8.))
                                        .items_center()
                                        .flex_shrink_0()
                                        .child(
                                            div()
                                                .text_xs()
                                                .line_height(px(14.))
                                                .text_color(p.quiet_ink)
                                                .child(count_label),
                                        )
                                        .child(
                                            Icon::new(IconName::ChevronRight)
                                                .size(px(13.))
                                                .text_color(p.quiet_ink),
                                        ),
                                ),
                        );
                    }

                    // Configured built-in providers
                    for row in &configured_builtins {
                        let m_count = model_counts.get(row.id.as_str()).copied().unwrap_or(0);
                        let badge = match &row.auth {
                            AuthKind::ApiKey(_) => "API Key",
                            AuthKind::OAuth => "OAuth",
                            AuthKind::EnvVar(_) => "Env",
                            AuthKind::None => "Active",
                        };
                        let count_label = if m_count > 0 {
                            format!("{m_count} model{}", if m_count == 1 { "" } else { "s" })
                        } else {
                            String::new()
                        };
                        let row_edit = (*row).clone();
                        let idx = row_i;
                        row_i += 1;

                        list = list.child(
                            h_flex()
                                .id(("cfg-row", idx))
                                .w_full()
                                .h(px(42.))
                                .px(px(14.))
                                .when(idx == 0, |d| d.rounded_t(px(9.)))
                                .when(idx + 1 == total_configured, |d| d.rounded_b(px(9.)))
                                .when(idx > 0, |d| {
                                    d.border_t_1().border_color(p.sidebar_group_edge)
                                })
                                .items_center()
                                .justify_between()
                                .gap(px(12.))
                                .cursor_pointer()
                                .hover(|h| h.bg(p.sidebar_row_hover))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_builtin_provider_editor(&row_edit, window, cx);
                                }))
                                .child(
                                    h_flex()
                                        .flex_1()
                                        .min_w_0()
                                        .gap(px(8.))
                                        .items_center()
                                        .child(
                                            div()
                                                .text_size(px(13.))
                                                .line_height(px(16.))
                                                .font_medium()
                                                .text_color(p.ink)
                                                .truncate()
                                                .child(row.display_name.clone()),
                                        )
                                        .child(
                                            h_flex()
                                                .flex_shrink_0()
                                                .h(px(18.))
                                                .px(px(6.))
                                                .rounded(px(4.))
                                                .bg(p.sidebar_row_selected)
                                                .items_center()
                                                .justify_center()
                                                .child(
                                                    div()
                                                        .text_size(px(10.))
                                                        .line_height(px(10.))
                                                        .text_color(p.quiet_ink)
                                                        .child(badge),
                                                ),
                                        ),
                                )
                                .child(
                                    h_flex()
                                        .gap(px(8.))
                                        .items_center()
                                        .flex_shrink_0()
                                        .when(!count_label.is_empty(), |d| {
                                            d.child(
                                                div()
                                                    .text_xs()
                                                    .line_height(px(14.))
                                                    .text_color(p.quiet_ink)
                                                    .child(count_label),
                                            )
                                        })
                                        .child(
                                            Icon::new(IconName::ChevronRight)
                                                .size(px(13.))
                                                .text_color(p.quiet_ink),
                                        ),
                                ),
                        );
                    }

                    configured_sec = configured_sec.child(list);
                }
                body = body.child(configured_sec);

                // 2. Unconfigured Built-in Providers (strictly non-configured, 2-col grid when expanded)
                if !unconfigured_builtins.is_empty() {
                    let expanded = self.providers_more_open;
                    let mut more_sec = v_flex().gap(px(10.)).child(
                        h_flex()
                            .id("providers-more-toggle")
                            .w_full()
                            .py(px(4.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .items_center()
                            .justify_between()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.providers_more_open = !this.providers_more_open;
                                cx.notify();
                            }))
                            .child(
                                h_flex()
                                    .gap(px(6.))
                                    .items_center()
                                    .child(
                                        Icon::new(if expanded {
                                            IconName::ChevronDown
                                        } else {
                                            IconName::ChevronRight
                                        })
                                        .size(px(12.))
                                        .text_color(p.quiet_ink),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(13.))
                                            .font_semibold()
                                            .text_color(p.quiet_ink)
                                            .hover(|h| h.text_color(p.ink))
                                            .child("More Providers"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(p.quiet_ink)
                                    .child(format!("{} available", unconfigured_builtins.len())),
                            ),
                    );

                    if expanded {
                        let mut grid = div().w_full().grid().grid_cols(2).gap(px(8.));
                        for (tile_id, item) in unconfigured_builtins.iter().enumerate() {
                            let row_edit = (*item).clone();
                            grid = grid.child(
                                h_flex()
                                    .id(("bi-avail", tile_id))
                                    .w_full()
                                    .min_w_0()
                                    .h(px(38.))
                                    .px(px(14.))
                                    .rounded(px(8.))
                                    .border_1()
                                    .border_color(p.sidebar_group_edge)
                                    .bg(p.window_surface)
                                    .items_center()
                                    .justify_between()
                                    .cursor_pointer()
                                    .hover(|h| h.bg(p.sidebar_row_hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_builtin_provider_editor(&row_edit, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_medium()
                                            .text_color(p.ink)
                                            .truncate()
                                            .child(item.display_name.clone()),
                                    )
                                    .child(
                                        Icon::new(IconName::Plus)
                                            .size(px(12.))
                                            .text_color(p.quiet_ink),
                                    ),
                            );
                        }
                        more_sec = more_sec.child(grid);
                    }
                    body = body.child(more_sec);
                }
            }
            ProvidersEditMode::EditBuiltin {
                id,
                env_var,
                is_oauth,
                ..
            } => {
                let id_rm = id.clone();
                body = body.child(
                    v_flex()
                        .gap(px(16.))
                        .when(*is_oauth || env_var.is_some(), |d| {
                            let note = if *is_oauth {
                                "Authenticated via OAuth. Saving an API key below switches this provider to API Key auth."
                                    .to_string()
                            } else {
                                format!(
                                    "Detected automatically if {} is set in your environment.",
                                    env_var.unwrap_or("")
                                )
                            };
                            d.child(
                                div()
                                    .px(px(12.))
                                    .py(px(8.))
                                    .rounded(px(8.))
                                    .bg(p.window_surface)
                                    .border_1()
                                    .border_color(p.sidebar_group_edge)
                                    .text_xs()
                                    .text_color(p.quiet_ink)
                                    .child(note),
                            )
                        })
                        .child(
                            v_flex()
                                .gap(px(6.))
                                .child(field_label("API Key"))
                                .child(self.render_slim_input(&self.prov_key_input, window, cx)),
                        )
                        .child(
                            v_flex()
                                .gap(px(6.))
                                .child(field_label("Base URL Override (optional)"))
                                .child(self.render_slim_input(&self.prov_url_input, window, cx)),
                        )
                        .child(
                            h_flex()
                                .pt(px(10.))
                                .border_t_1()
                                .border_color(p.sidebar_group_edge)
                                .items_center()
                                .justify_between()
                                .child(
                                    div()
                                        .id("bi-save-btn")
                                        .px(px(16.))
                                        .py(px(7.))
                                        .rounded(px(8.))
                                        .bg(p.composer_send_fill)
                                        .cursor_pointer()
                                        .hover(|h| h.opacity(0.9))
                                        .text_xs()
                                        .font_semibold()
                                        .text_color(p.composer_send_mark)
                                        .child("Save Changes")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.save_builtin_provider(cx);
                                        })),
                                )
                                .child(
                                    div()
                                        .id("bi-del-auth")
                                        .px(px(10.))
                                        .py(px(6.))
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .hover(|h| h.bg(p.danger_surface))
                                        .text_xs()
                                        .text_color(p.danger)
                                        .child("Remove credentials")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.remove_builtin_provider_auth(&id_rm, cx);
                                        })),
                                ),
                        ),
                );
            }
            ProvidersEditMode::EditCustom {
                old_id,
                api,
                headers,
                models,
                fetched_models,
                model_editor_open,
                editing_model_idx,
                model_reasoning,
                model_images,
            } => {
                if *model_editor_open {
                    let edit_idx = *editing_model_idx;
                    body = body.child(
                        v_flex()
                            .gap(px(14.))
                            .child(
                                v_flex()
                                    .gap(px(5.))
                                    .child(field_label("Model ID"))
                                    .child(self.render_slim_input(
                                        &self.prov_model_id_input,
                                        window,
                                        cx,
                                    )),
                            )
                            .child(
                                v_flex()
                                    .gap(px(5.))
                                    .child(field_label("Display Name"))
                                    .child(self.render_slim_input(
                                        &self.prov_model_name_input,
                                        window,
                                        cx,
                                    )),
                            )
                            .child(
                                h_flex()
                                    .gap(px(10.))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap(px(5.))
                                            .child(field_label("Context Window"))
                                            .child(self.render_slim_input(
                                                &self.prov_model_ctx_input,
                                                window,
                                                cx,
                                            )),
                                    )
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .gap(px(5.))
                                            .child(field_label("Max Output Tokens"))
                                            .child(self.render_slim_input(
                                                &self.prov_model_max_input,
                                                window,
                                                cx,
                                            )),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .gap(px(6.))
                                    .child(field_label("Capabilities"))
                                    .child(
                                        h_flex()
                                            .gap(px(8.))
                                            .child(
                                                div()
                                                    .id("m-toggle-reasoning")
                                                    .px(px(10.))
                                                    .py(px(6.))
                                                    .rounded(px(7.))
                                                    .border_1()
                                                    .border_color(if *model_reasoning {
                                                        p.composer_edge_focus
                                                    } else {
                                                        p.sidebar_group_edge
                                                    })
                                                    .bg(if *model_reasoning {
                                                        p.sidebar_row_selected
                                                    } else {
                                                        p.window_surface
                                                    })
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .font_medium()
                                                    .text_color(if *model_reasoning {
                                                        p.ink
                                                    } else {
                                                        p.quiet_ink
                                                    })
                                                    .child("Reasoning")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        if let ProvidersEditMode::EditCustom {
                                                            model_reasoning,
                                                            ..
                                                        } = &mut this.providers_mode
                                                        {
                                                            *model_reasoning = !*model_reasoning;
                                                            cx.notify();
                                                        }
                                                    })),
                                            )
                                            .child(
                                                div()
                                                    .id("m-toggle-images")
                                                    .px(px(10.))
                                                    .py(px(6.))
                                                    .rounded(px(7.))
                                                    .border_1()
                                                    .border_color(if *model_images {
                                                        p.composer_edge_focus
                                                    } else {
                                                        p.sidebar_group_edge
                                                    })
                                                    .bg(if *model_images {
                                                        p.sidebar_row_selected
                                                    } else {
                                                        p.window_surface
                                                    })
                                                    .cursor_pointer()
                                                    .text_xs()
                                                    .font_medium()
                                                    .text_color(if *model_images {
                                                        p.ink
                                                    } else {
                                                        p.quiet_ink
                                                    })
                                                    .child("Image Input")
                                                    .on_click(cx.listener(|this, _, _, cx| {
                                                        if let ProvidersEditMode::EditCustom {
                                                            model_images,
                                                            ..
                                                        } = &mut this.providers_mode
                                                        {
                                                            *model_images = !*model_images;
                                                            cx.notify();
                                                        }
                                                    })),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .pt(px(10.))
                                    .border_t_1()
                                    .border_color(p.sidebar_group_edge)
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .id("m-commit")
                                            .px(px(16.))
                                            .py(px(7.))
                                            .rounded(px(8.))
                                            .bg(p.composer_send_fill)
                                            .cursor_pointer()
                                            .hover(|h| h.opacity(0.9))
                                            .text_xs()
                                            .font_semibold()
                                            .text_color(p.composer_send_mark)
                                            .child(if edit_idx.is_some() {
                                                "Save Model"
                                            } else {
                                                "Add Model"
                                            })
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.commit_model_draft(window, cx);
                                            })),
                                    )
                                    .when_some(edit_idx, |d, idx_rm| {
                                        d.child(
                                            div()
                                                .id("m-del-editor")
                                                .px(px(10.))
                                                .py(px(6.))
                                                .rounded(px(7.))
                                                .cursor_pointer()
                                                .hover(|h| h.bg(p.danger_surface))
                                                .text_xs()
                                                .text_color(p.danger)
                                                .child("Remove model")
                                                .on_click(cx.listener(move |this, _, window, cx| {
                                                    this.remove_model_draft(idx_rm, window, cx);
                                                    if let ProvidersEditMode::EditCustom {
                                                        model_editor_open,
                                                        ..
                                                    } = &mut this.providers_mode
                                                    {
                                                        *model_editor_open = false;
                                                    }
                                                    cx.notify();
                                                })),
                                        )
                                    }),
                            ),
                    );
                } else {
                    let delete_id = old_id.clone();
                    let mut form = v_flex()
                        .gap(px(14.))
                        .child(
                            h_flex()
                                .gap(px(10.))
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap(px(5.))
                                        .child(field_label("Provider ID"))
                                        .child(self.render_slim_input(
                                            &self.prov_id_input,
                                            window,
                                            cx,
                                        )),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap(px(5.))
                                        .child(field_label("Display Name"))
                                        .child(self.render_slim_input(
                                            &self.prov_name_input,
                                            window,
                                            cx,
                                        )),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap(px(5.))
                                .child(field_label("API Protocol"))
                                .child(
                                    div()
                                        .w_full()
                                        .grid()
                                        .grid_cols(4)
                                        .p(px(3.))
                                        .rounded(px(9.))
                                        .bg(p.window_surface)
                                        .border_1()
                                        .border_color(p.sidebar_group_edge)
                                        .gap(px(3.))
                                        .children(API_PROTOCOLS.iter().enumerate().map(
                                            |(i, (proto_id, label))| {
                                                let selected = api == *proto_id;
                                                let pid = (*proto_id).to_string();
                                                h_flex()
                                                    .id(("proto-pill", i))
                                                    .w_full()
                                                    .min_w_0()
                                                    .h(px(30.))
                                                    .px(px(4.))
                                                    .rounded(px(6.))
                                                    .items_center()
                                                    .justify_center()
                                                    .when(selected, |d| {
                                                        d.bg(p.sidebar_row_selected)
                                                    })
                                                    .when(!selected, |d| {
                                                        d.hover(|h| h.bg(p.sidebar_row_hover))
                                                    })
                                                    .cursor_pointer()
                                                    .child(
                                                        div()
                                                            .text_size(px(11.))
                                                            .line_height(px(11.))
                                                            .font_medium()
                                                            .text_center()
                                                            .whitespace_nowrap()
                                                            .text_color(if selected {
                                                                p.ink
                                                            } else {
                                                                p.quiet_ink
                                                            })
                                                            .child(*label),
                                                    )
                                                    .on_click(cx.listener(move |this, _, _, cx| {
                                                        if let ProvidersEditMode::EditCustom {
                                                            api,
                                                            ..
                                                        } = &mut this.providers_mode
                                                        {
                                                            *api = pid.clone();
                                                            cx.notify();
                                                        }
                                                    }))
                                            },
                                        )),
                                ),
                        )
                        .child(
                            v_flex()
                                .gap(px(5.))
                                .child(field_label("Base URL"))
                                .child(self.render_slim_input(&self.prov_url_input, window, cx)),
                        )
                        .child(
                            v_flex()
                                .gap(px(5.))
                                .child(field_label("API Key (optional for local endpoints)"))
                                .child(self.render_slim_input(&self.prov_key_input, window, cx)),
                        )
                        .child({
                            let mut hdr_sec = v_flex().gap(px(6.)).child(
                                h_flex()
                                    .items_center()
                                    .justify_between()
                                    .child(field_label("Custom Headers (optional)"))
                                    .child(
                                        div()
                                            .id("ce-add-header-btn")
                                            .px(px(9.))
                                            .py(px(4.))
                                            .rounded(px(6.))
                                            .bg(p.sidebar_row_selected)
                                            .cursor_pointer()
                                            .hover(|h| h.bg(p.sidebar_row_hover))
                                            .text_xs()
                                            .font_medium()
                                            .text_color(p.ink)
                                            .child("+ Add Header")
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.add_header_row(window, cx);
                                            })),
                                    ),
                            );
                            for (h_idx, (ki, vi)) in headers.iter().enumerate() {
                                hdr_sec = hdr_sec.child(
                                    h_flex()
                                        .gap(px(6.))
                                        .items_center()
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(self.render_slim_input(ki, window, cx)),
                                        )
                                        .child(
                                            div()
                                                .flex_1()
                                                .child(self.render_slim_input(vi, window, cx)),
                                        )
                                        .child(
                                            div()
                                                .id(("hdr-rm", h_idx))
                                                .size(px(26.))
                                                .rounded(px(6.))
                                                .flex()
                                                .items_center()
                                                .justify_center()
                                                .cursor_pointer()
                                                .hover(|h| h.bg(p.danger_surface))
                                                .text_xs()
                                                .text_color(p.quiet_ink)
                                                .child("×")
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.remove_header_row(h_idx, cx);
                                                })),
                                        ),
                                );
                            }
                            hdr_sec
                        });

                    // Models list (display name only; inputs live in the dedicated Model Editor view)
                    let mut models_sec = v_flex()
                        .gap(px(8.))
                        .pt(px(8.))
                        .border_t_1()
                        .border_color(p.sidebar_group_edge)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .child(field_label("Models"))
                                .child(
                                    h_flex()
                                        .gap(px(6.))
                                        .items_center()
                                        .child(
                                            div()
                                                .id("ce-fetch-models-btn")
                                                .px(px(9.))
                                                .py(px(4.))
                                                .rounded(px(6.))
                                                .bg(p.window_surface)
                                                .border_1()
                                                .border_color(p.sidebar_group_edge)
                                                .cursor_pointer()
                                                .hover(|h| h.bg(p.sidebar_row_hover))
                                                .text_xs()
                                                .font_medium()
                                                .text_color(if self.providers_fetching_models {
                                                    p.quiet_ink
                                                } else {
                                                    p.ink
                                                })
                                                .child(if self.providers_fetching_models {
                                                    "Fetching…"
                                                } else {
                                                    "Fetch Models"
                                                })
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.trigger_fetch_models(cx);
                                                })),
                                        )
                                        .child(
                                            div()
                                                .id("ce-add-model-btn")
                                                .px(px(9.))
                                                .py(px(4.))
                                                .rounded(px(6.))
                                                .bg(p.sidebar_row_selected)
                                                .cursor_pointer()
                                                .hover(|h| h.bg(p.sidebar_row_hover))
                                                .text_xs()
                                                .font_medium()
                                                .text_color(p.ink)
                                                .child("+ Add Model")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.open_new_model_editor(window, cx);
                                                })),
                                        ),
                                ),
                        );

                    if models.is_empty() {
                        models_sec = models_sec.child(
                            div()
                                .px(px(12.))
                                .py(px(10.))
                                .rounded(px(8.))
                                .border_1()
                                .border_color(p.sidebar_group_edge)
                                .bg(p.window_surface)
                                .text_xs()
                                .text_color(p.quiet_ink)
                                .child("No models added yet."),
                        );
                    } else {
                        let m_len = models.len();
                        let mut m_group = v_flex()
                            .w_full()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.window_surface)
                            .overflow_hidden();

                        for (m_idx, m) in models.iter().enumerate() {
                            let display = m
                                .name
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&m.id)
                                .to_string();
                            m_group = m_group.child(
                                h_flex()
                                    .id(("m-row", m_idx))
                                    .w_full()
                                    .h(px(36.))
                                    .px(px(12.))
                                    .when(m_idx == 0, |d| d.rounded_t(px(7.)))
                                    .when(m_idx + 1 == m_len, |d| d.rounded_b(px(7.)))
                                    .when(m_idx > 0, |d| {
                                        d.border_t_1().border_color(p.sidebar_group_edge)
                                    })
                                    .items_center()
                                    .justify_between()
                                    .cursor_pointer()
                                    .hover(|h| h.bg(p.sidebar_row_hover))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.load_model_draft(m_idx, window, cx);
                                    }))
                                    .child(
                                        div()
                                            .text_xs()
                                            .font_medium()
                                            .text_color(p.ink)
                                            .truncate()
                                            .child(display),
                                    )
                                    .child(
                                        Icon::new(IconName::ChevronRight)
                                            .size(px(12.))
                                            .text_color(p.quiet_ink),
                                    ),
                            );
                        }
                        models_sec = models_sec.child(m_group);
                    }

                    // Staging list for fetched models not yet added to saved models
                    if !fetched_models.is_empty() {
                        let f_len = fetched_models.len();
                        let mut f_group = v_flex()
                            .w_full()
                            .rounded(px(8.))
                            .border_1()
                            .border_color(p.sidebar_group_edge)
                            .bg(p.window_surface)
                            .overflow_hidden();

                        for (f_idx, m) in fetched_models.iter().enumerate() {
                            let display = m
                                .name
                                .as_deref()
                                .filter(|s| !s.is_empty())
                                .unwrap_or(&m.id)
                                .to_string();
                            let add_id = m.id.clone();
                            f_group = f_group.child(
                                h_flex()
                                    .id(("fetched-row", f_idx))
                                    .w_full()
                                    .h(px(36.))
                                    .px(px(12.))
                                    .when(f_idx == 0, |d| d.rounded_t(px(7.)))
                                    .when(f_idx + 1 == f_len, |d| d.rounded_b(px(7.)))
                                    .when(f_idx > 0, |d| {
                                        d.border_t_1().border_color(p.sidebar_group_edge)
                                    })
                                    .items_center()
                                    .justify_between()
                                    .gap(px(8.))
                                    .hover(|h| h.bg(p.sidebar_row_hover))
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(p.quiet_ink)
                                            .truncate()
                                            .child(display),
                                    )
                                    .child(
                                        div()
                                            .id(("fetched-add", f_idx))
                                            .px(px(8.))
                                            .py(px(3.))
                                            .rounded(px(5.))
                                            .bg(p.sidebar_row_selected)
                                            .cursor_pointer()
                                            .hover(|h| h.opacity(0.85))
                                            .text_xs()
                                            .font_medium()
                                            .text_color(p.ink)
                                            .child("+ Add")
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.add_fetched_model(&add_id, cx);
                                            })),
                                    ),
                            );
                        }

                        models_sec = models_sec.child(
                            v_flex()
                                .pt(px(6.))
                                .gap(px(6.))
                                .child(
                                    h_flex()
                                        .items_center()
                                        .justify_between()
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .font_medium()
                                                .text_color(p.quiet_ink)
                                                .child(format!(
                                                    "Available from Endpoint ({f_len})"
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("ce-clear-fetched")
                                                .cursor_pointer()
                                                .text_xs()
                                                .text_color(p.quiet_ink)
                                                .hover(|h| h.text_color(p.ink))
                                                .child("Clear")
                                                .on_click(cx.listener(|this, _, _, cx| {
                                                    this.clear_fetched_models(cx);
                                                })),
                                        ),
                                )
                                .child(f_group),
                        );
                    }

                    form = form.child(models_sec).child(
                        h_flex()
                            .pt(px(10.))
                            .border_t_1()
                            .border_color(p.sidebar_group_edge)
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .id("ce-save-btn")
                                    .px(px(16.))
                                    .py(px(7.))
                                    .rounded(px(8.))
                                    .bg(p.composer_send_fill)
                                    .cursor_pointer()
                                    .hover(|h| h.opacity(0.9))
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(p.composer_send_mark)
                                    .child("Save Endpoint")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.save_custom_endpoint(cx);
                                    })),
                            )
                            .when_some(delete_id, |d, id_del| {
                                d.child(
                                    div()
                                        .id("ce-del-btn")
                                        .px(px(10.))
                                        .py(px(6.))
                                        .rounded(px(7.))
                                        .cursor_pointer()
                                        .hover(|h| h.bg(p.danger_surface))
                                        .text_xs()
                                        .text_color(p.danger)
                                        .child("Delete endpoint")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.delete_custom_endpoint_entry(&id_del, cx);
                                        })),
                                )
                            }),
                    );

                    body = body.child(form);
                }
            }
        }

        body.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("pa-ui-providers-{label}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[::core::prelude::v1::test]
    fn jsonc_strip_and_extra_fields_round_trip() {
        let dir = temp_dir("roundtrip");
        let raw = r#"{
            // comment
            "providers": {
                "clove": {
                    "baseUrl": "http://localhost:5201",
                    "apiKey": "1",
                    "api": "anthropic-messages",
                    "models": [
                        {
                            "id": "claude-sonnet-5-5",
                            "name": "Claude Sonnet 5.5",
                            "reasoning": true,
                            "input": ["text", "image"],
                            "contextWindow": 200000,
                            "maxTokens": 32000,
                            "thinkingLevelMap": { "off": "none", "max": "max" },
                        },
                    ],
                },
            }
        }"#;
        fs::write(dir.join("models.json"), raw).unwrap();

        let snap = load_snapshot_from(&dir);
        assert!(snap.load_error.is_none());
        assert_eq!(snap.custom.len(), 1);
        let mut cfg = snap.custom[0].config.clone();
        cfg.name = Some("Clove Local".to_string());
        upsert_custom_endpoint_in(&dir, Some("clove"), "clove", cfg).unwrap();

        let after = load_snapshot_from(&dir);
        let model = &after.custom[0].config.models.as_ref().unwrap()[0];
        assert!(model.extra.contains_key("thinkingLevelMap"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn builtin_and_custom_crud() {
        let dir = temp_dir("crud");
        save_builtin_config_in(&dir, "openai", Some("sk-test-123"), Some("https://proxy.test/v1"))
            .unwrap();
        let snap = load_snapshot_from(&dir);
        let openai = snap.builtins.iter().find(|b| b.id == "openai").unwrap();
        assert_eq!(openai.auth, AuthKind::ApiKey("sk-test-123".to_string()));
        assert_eq!(
            openai.base_url_override.as_deref(),
            Some("https://proxy.test/v1")
        );

        remove_builtin_auth_in(&dir, "openai").unwrap();
        let snap2 = load_snapshot_from(&dir);
        let openai2 = snap2.builtins.iter().find(|b| b.id == "openai").unwrap();
        assert!(!matches!(openai2.auth, AuthKind::ApiKey(_)));

        let mut headers = BTreeMap::new();
        headers.insert("User-Agent".to_string(), "test".to_string());
        upsert_custom_endpoint_in(
            &dir,
            None,
            "my_endpoint",
            ProviderConfig {
                name: Some("My Endpoint".to_string()),
                base_url: Some("https://api.example.com/v1".to_string()),
                api: Some("openai-responses".to_string()),
                api_key: None,
                headers: Some(headers),
                models: Some(vec![ModelDefinition {
                    id: "model-1".to_string(),
                    name: Some("Model 1".to_string()),
                    context_window: Some(128000),
                    max_tokens: Some(8192),
                    reasoning: Some(true),
                    input: Some(vec!["text".to_string()]),
                    ..Default::default()
                }]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(load_snapshot_from(&dir).custom.len(), 1);
        delete_custom_endpoint_in(&dir, "my_endpoint").unwrap();
        assert_eq!(load_snapshot_from(&dir).custom.len(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    #[::core::prelude::v1::test]
    fn parse_fetched_models_across_protocols() {
        let oai = parse_fetched_models(
            "openai-completions",
            r#"{"data":[{"id":"gpt-4o","context_length":128000},{"id":"o3-mini"}]}"#,
        )
        .unwrap();
        assert_eq!(oai.len(), 2);
        assert_eq!(oai[0].id, "gpt-4o");
        assert_eq!(oai[0].context_window, Some(128000));

        let ant = parse_fetched_models(
            "anthropic-messages",
            r#"{"data":[{"id":"claude-sonnet-5-5","display_name":"Claude Sonnet 5.5"}]}"#,
        )
        .unwrap();
        assert_eq!(ant.len(), 1);
        assert_eq!(ant[0].name.as_deref(), Some("Claude Sonnet 5.5"));

        let gem = parse_fetched_models(
            "google-generative-ai",
            r#"{"models":[{"name":"models/gemini-2.5-pro","displayName":"Gemini 2.5 Pro","inputTokenLimit":1048576,"outputTokenLimit":65536}]}"#,
        )
        .unwrap();
        assert_eq!(gem.len(), 1);
        assert_eq!(gem[0].id, "gemini-2.5-pro");
        assert_eq!(gem[0].name.as_deref(), Some("Gemini 2.5 Pro"));
        assert_eq!(gem[0].context_window, Some(1048576));
        assert_eq!(gem[0].max_tokens, Some(65536));
    }
}
