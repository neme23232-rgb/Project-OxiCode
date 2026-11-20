use crate::theme::ThemeKind;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiProtocol {
    OpenAICompatible,
    Anthropic,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub protocol: ApiProtocol,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub env_var: Option<&'static str>,
    pub requires_key: bool,
}

pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "anymodel",
        display_name: "AnyModel",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://anymodel.org/v1",
        default_model: "pg/deepseek-v4.1-flash",
        env_var: Some("ANYMODEL_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "opencode",
        display_name: "OpenCode Zen",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://opencode.ai/zen/v1",
        default_model: "big-pickle",
        env_var: Some("OPENCODE_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "openrouter",
        display_name: "OpenRouter",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://openrouter.ai/api/v1",
        default_model: "deepseek/deepseek-chat-v3-0324:free",
        env_var: Some("OPENROUTER_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "nvidia",
        display_name: "NVIDIA NIM",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://integrate.api.nvidia.com/v1",
        default_model: "deepseek-ai/deepseek-r1",
        env_var: Some("NVIDIA_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "google",
        display_name: "Google Gemini",
        protocol: ApiProtocol::Gemini,
        default_base_url: "https://generativelanguage.googleapis.com",
        default_model: "gemini-2.5-flash",
        env_var: Some("GEMINI_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "openai",
        display_name: "OpenAI",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o",
        env_var: Some("OPENAI_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "anthropic",
        display_name: "Anthropic",
        protocol: ApiProtocol::Anthropic,
        default_base_url: "https://api.anthropic.com",
        default_model: "claude-sonnet-4-20250514",
        env_var: Some("ANTHROPIC_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "deepseek",
        display_name: "DeepSeek",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        env_var: Some("DEEPSEEK_API_KEY"),
        requires_key: true,
    },
    ProviderSpec {
        id: "ollama",
        display_name: "Ollama (Local)",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "http://localhost:11434/v1",
        default_model: "qwen2.5-coder",
        env_var: None,
        requires_key: false,
    },
    ProviderSpec {
        id: "custom",
        display_name: "Custom OpenAI-Compatible",
        protocol: ApiProtocol::OpenAICompatible,
        default_base_url: "http://localhost:8000/v1",
        default_model: "default",
        env_var: Some("CUSTOM_API_KEY"),
        requires_key: false,
    },
];

pub fn normalize_base_url(raw: &str) -> String {
    let mut url = raw.trim().trim_end_matches('/').to_string();
    for suffix in ["/chat/completions", "/completions"] {
        if url.to_lowercase().ends_with(suffix) {
            url.truncate(url.len() - suffix.len());
            url = url.trim_end_matches('/').to_string();
        }
    }
    url
}

pub fn normalize_provider_id(raw: &str) -> Option<&'static str> {
    match raw.trim().to_lowercase().as_str() {
        "anymodel" | "am" => Some("anymodel"),
        "opencode" | "zen" | "opencode-zen" => Some("opencode"),
        "openrouter" | "or" => Some("openrouter"),
        "nvidia" | "nim" | "nvidia-nim" => Some("nvidia"),
        "google" | "gemini" => Some("google"),
        "openai" => Some("openai"),
        "anthropic" | "claude" => Some("anthropic"),
        "deepseek" => Some("deepseek"),
        "ollama" | "local" => Some("ollama"),
        "custom" | "openai-compatible" => Some("custom"),
        _ => None,
    }
}

pub fn find_spec(id: &str) -> &'static ProviderSpec {
    let norm = normalize_provider_id(id).unwrap_or("opencode");
    PROVIDERS
        .iter()
        .find(|p| p.id == norm)
        .unwrap_or(&PROVIDERS[0])
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
}

pub const MAX_TOKEN_PRESETS: &[u32] = &[2048, 4096, 8192, 16384, 32768, 65536];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UiStyle {
    #[default]
    #[serde(alias = "fox", alias = "default")]
    Oxicode,
    #[serde(alias = "agy")]
    Antigravity,
    #[serde(alias = "minimal")]
    Logo,
}

impl UiStyle {
    pub fn all() -> &'static [UiStyle] {
        &[UiStyle::Oxicode, UiStyle::Antigravity, UiStyle::Logo]
    }

    pub fn name(&self) -> &'static str {
        match self {
            UiStyle::Oxicode => "oxicode",
            UiStyle::Antigravity => "antigravity",
            UiStyle::Logo => "logo",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            UiStyle::Oxicode => "OxiCode (Animated Fox)",
            UiStyle::Antigravity => "Antigravity (Static Pixel Fox)",
            UiStyle::Logo => "Minimal (OXICODE Logo)",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            UiStyle::Oxicode => "Centered 3D ASCII fox that tracks cursor & typing with boxed prompt",
            UiStyle::Antigravity => {
                "Compact top banner with static pixel fox, divider, and clean > prompt"
            }
            UiStyle::Logo => "Centered OXICODE banner without mascot",
        }
    }

    pub fn parse(s: &str) -> Option<UiStyle> {
        match s.trim().to_ascii_lowercase().as_str() {
            "oxicode" | "fox" | "default" => Some(UiStyle::Oxicode),
            "antigravity" | "agy" => Some(UiStyle::Antigravity),
            "logo" | "minimal" => Some(UiStyle::Logo),
            _ => None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PartialConfig {
    provider: Option<String>,
    max_tokens: Option<u32>,
    auto_approve: Option<bool>,
    default_plan_mode: Option<bool>,
    show_tool_details: Option<bool>,
    show_fox_mascot: Option<bool>,
    mouse_capture: Option<bool>,
    ui_style: Option<UiStyle>,
    theme: Option<ThemeKind>,
    #[serde(default)]
    providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    disabled_plugins: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_provider_name")]
    pub provider: String,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default)]
    pub default_plan_mode: bool,
    #[serde(default)]
    pub show_tool_details: bool,
    #[serde(default = "default_true")]
    pub show_fox_mascot: bool,
    #[serde(default)]
    pub mouse_capture: bool,
    #[serde(default)]
    pub ui_style: UiStyle,
    #[serde(default)]
    pub theme: ThemeKind,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    #[serde(default)]
    pub mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    pub disabled_plugins: Vec<String>,
}

fn default_provider_name() -> String {
    "opencode".to_string()
}

fn default_max_tokens() -> u32 {
    8192
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        for spec in PROVIDERS {
            providers.insert(
                spec.id.to_string(),
                ProviderConfig {
                    api_key: String::new(),
                    base_url: spec.default_base_url.to_string(),
                    model: spec.default_model.to_string(),
                },
            );
        }
        Self {
            provider: default_provider_name(),
            max_tokens: default_max_tokens(),
            auto_approve: false,
            default_plan_mode: false,
            show_tool_details: false,
            show_fox_mascot: true,
            mouse_capture: false,
            ui_style: UiStyle::Oxicode,
            theme: ThemeKind::OpenCodeBeige,
            providers,
            mcp_servers: BTreeMap::new(),
            disabled_plugins: Vec::new(),
        }
    }
}

impl Config {
    pub fn load(
        work_dir: &Path,
        cli_provider: Option<&str>,
        cli_model: Option<&str>,
        cli_base_url: Option<&str>,
    ) -> Result<Self> {
        let mut cfg = Self::default();
        cfg.apply_env();

        let mut had_saved_provider = false;
        if let Some(global_path) = Self::global_config_path()
            && global_path.exists()
        {
            had_saved_provider = true;
            cfg.merge_file(&global_path)?;
        }

        let local_path = work_dir.join(".oxicode.toml");
        if local_path.exists() {
            had_saved_provider = true;
            cfg.merge_file(&local_path)?;
        }

        // Auto-select first provider with a configured key if user has no config file yet
        if !had_saved_provider
            && cli_provider.is_none()
            && let Some(first_with_key) = PROVIDERS.iter().find(|spec| {
                cfg.providers
                    .get(spec.id)
                    .is_some_and(|p| !p.api_key.is_empty())
            })
        {
            cfg.provider = first_with_key.id.to_string();
        }

        if let Some(p) = cli_provider
            && let Some(norm) = normalize_provider_id(p)
        {
            cfg.provider = norm.to_string();
        }
        if let Some(m) = cli_model {
            cfg.active_mut().model = m.to_string();
        }
        if let Some(url) = cli_base_url {
            cfg.active_mut().base_url = normalize_base_url(url);
        }

        Ok(cfg)
    }

    pub fn global_config_path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("oxicode").join("config.toml"))
    }

    pub fn save_global(&self) -> Result<PathBuf> {
        let path = Self::global_config_path()
            .ok_or_else(|| anyhow::anyhow!("Could not locate user config directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self).context("Failed to serialize config")?;
        std::fs::write(&path, body)
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(path)
    }

    fn merge_file(&mut self, path: &Path) -> Result<()> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let parsed: PartialConfig = toml::from_str(&raw)
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))?;

        if let Some(p) = parsed.provider
            && let Some(norm) = normalize_provider_id(&p)
        {
            self.provider = norm.to_string();
        }
        if let Some(v) = parsed.max_tokens {
            self.max_tokens = v;
        }
        if let Some(v) = parsed.auto_approve {
            self.auto_approve = v;
        }
        if let Some(v) = parsed.default_plan_mode {
            self.default_plan_mode = v;
        }
        if let Some(v) = parsed.show_tool_details {
            self.show_tool_details = v;
        }
        if let Some(v) = parsed.show_fox_mascot {
            self.show_fox_mascot = v;
            if parsed.ui_style.is_none() {
                self.ui_style = if v { UiStyle::Oxicode } else { UiStyle::Logo };
            }
        }
        if let Some(style) = parsed.ui_style {
            self.ui_style = style;
            self.show_fox_mascot = !matches!(style, UiStyle::Logo);
        }
        if let Some(mc) = parsed.mouse_capture {
            self.mouse_capture = mc;
        }
        if let Some(v) = parsed.theme {
            self.theme = v;
        }

        for (k, entry) in parsed.providers {
            let key = normalize_provider_id(&k).unwrap_or(&k).to_string();
            let target = self
                .providers
                .entry(key.clone())
                .or_insert_with(|| ProviderConfig {
                    api_key: String::new(),
                    base_url: find_spec(&key).default_base_url.to_string(),
                    model: find_spec(&key).default_model.to_string(),
                });
            if !entry.api_key.is_empty() {
                target.api_key = entry.api_key;
            }
            if !entry.base_url.is_empty() {
                target.base_url = normalize_base_url(&entry.base_url);
            }
            if !entry.model.is_empty() {
                target.model = entry.model;
            }
        }
        for (k, srv) in parsed.mcp_servers {
            self.mcp_servers.insert(k, srv);
        }
        for d in parsed.disabled_plugins {
            if !self.disabled_plugins.contains(&d) {
                self.disabled_plugins.push(d);
            }
        }
        Ok(())
    }

    pub fn context_window_tokens(&self) -> u32 {
        let m = self.active().model.to_lowercase();
        if m.contains("gemini") {
            1_000_000
        } else if m.contains("claude") || m.contains("deepseek") || m.contains("opencode") {
            200_000
        } else {
            128_000
        }
    }

    fn apply_env(&mut self) {
        for spec in PROVIDERS {
            if let Some(env_name) = spec.env_var
                && let Ok(val) = std::env::var(env_name)
                && !val.is_empty()
                && let Some(entry) = self.providers.get_mut(spec.id)
            {
                entry.api_key = val;
            }
        }
        if let Ok(url) = std::env::var("OPENAI_BASE_URL")
            && !url.is_empty()
            && let Some(entry) = self.providers.get_mut("openai")
        {
            entry.base_url = normalize_base_url(&url);
        }
        if let Ok(p) = std::env::var("OXICODE_PROVIDER")
            && let Some(norm) = normalize_provider_id(&p)
        {
            self.provider = norm.to_string();
        }
        if let Ok(m) = std::env::var("OXICODE_MODEL")
            && !m.is_empty()
        {
            self.active_mut().model = m;
        }
    }

    pub fn active_spec(&self) -> &'static ProviderSpec {
        find_spec(&self.provider)
    }

    pub fn active(&self) -> &ProviderConfig {
        self.providers
            .get(&self.provider)
            .unwrap_or_else(|| &self.providers["opencode"])
    }

    pub fn active_mut(&mut self) -> &mut ProviderConfig {
        let id = self.provider.clone();
        let spec = find_spec(&id);
        self.providers
            .entry(id)
            .or_insert_with(|| ProviderConfig {
                api_key: String::new(),
                base_url: spec.default_base_url.to_string(),
                model: spec.default_model.to_string(),
            })
    }

    pub fn set_provider(&mut self, id: &str) -> Option<&'static ProviderSpec> {
        let norm = normalize_provider_id(id)?;
        self.provider = norm.to_string();
        Some(find_spec(norm))
    }

    pub fn has_key(&self, id: &str) -> bool {
        let spec = find_spec(id);
        if !spec.requires_key {
            return true;
        }
        self.providers
            .get(spec.id)
            .is_some_and(|p| !p.api_key.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_chat_completions_suffix() {
        assert_eq!(
            normalize_base_url("https://anymodel.org/v1/chat/completions/"),
            "https://anymodel.org/v1"
        );
        assert_eq!(
            normalize_base_url("https://api.openai.com/v1/"),
            "https://api.openai.com/v1"
        );
    }

    #[test]
    fn provider_aliases_resolve() {
        assert_eq!(normalize_provider_id("zen"), Some("opencode"));
        assert_eq!(normalize_provider_id("or"), Some("openrouter"));
        assert_eq!(normalize_provider_id("nim"), Some("nvidia"));
        assert_eq!(normalize_provider_id("gemini"), Some("google"));
        assert_eq!(normalize_provider_id("claude"), Some("anthropic"));
        assert_eq!(normalize_provider_id("unknown"), None);
    }

    #[test]
    fn default_config_contains_all_providers() {
        let cfg = Config::default();
        for spec in PROVIDERS {
            assert!(cfg.providers.contains_key(spec.id));
        }
        assert_eq!(cfg.theme, ThemeKind::OpenCodeBeige);
    }

    #[test]
    fn partial_merge_preserves_existing_settings() {
        let mut cfg = Config {
            theme: ThemeKind::Nord,
            max_tokens: 16384,
            show_fox_mascot: false,
            ..Default::default()
        };
        let tmp = std::env::temp_dir().join(format!("oxicode_cfg_test_{}.toml", std::process::id()));
        std::fs::write(&tmp, "auto_approve = true\n").unwrap();
        cfg.merge_file(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);

        assert!(cfg.auto_approve);
        assert_eq!(cfg.theme, ThemeKind::Nord);
        assert_eq!(cfg.max_tokens, 16384);
        assert!(!cfg.show_fox_mascot);
    }

    #[test]
    fn ui_style_parses_and_merges_from_toml() {
        assert_eq!(UiStyle::parse("antigravity"), Some(UiStyle::Antigravity));
        assert_eq!(UiStyle::parse("agy"), Some(UiStyle::Antigravity));
        assert_eq!(UiStyle::parse("oxicode"), Some(UiStyle::Oxicode));
        assert_eq!(UiStyle::parse("logo"), Some(UiStyle::Logo));

        let mut cfg = Config::default();
        let tmp = std::env::temp_dir().join(format!("oxicode_style_test_{}.toml", std::process::id()));
        std::fs::write(&tmp, "ui_style = \"antigravity\"\n").unwrap();
        cfg.merge_file(&tmp).unwrap();
        let _ = std::fs::remove_file(&tmp);
        assert_eq!(cfg.ui_style, UiStyle::Antigravity);
        assert!(cfg.show_fox_mascot);
    }
}
