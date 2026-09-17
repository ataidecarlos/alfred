use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

use crate::error::AlfredError;
use crate::paths::Paths;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    pub prompt: PromptConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_db_path")]
    pub db_path: String,
}

#[derive(Debug, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_provider")]
    pub default_provider: String,
    pub providers: HashMap<String, ProviderConfig>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub model: String,
    pub base_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramConfig {
    pub bot_token: Option<String>,
    #[serde(default)]
    pub allowed_users: Vec<u64>,
}

#[derive(Debug, Deserialize)]
pub struct PromptConfig {
    #[serde(default = "default_system_prompt_path")]
    pub system_prompt_file: String,
    #[serde(default = "default_user_prompt_path")]
    pub user_prompt_file: String,
}

#[derive(Debug, Deserialize)]
pub struct SchedulerConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MemoryConfig {
    #[serde(default = "default_memory_enabled")]
    pub enabled: bool,
    #[serde(default = "default_vault_path")]
    pub vault_path: String,
    #[serde(default = "default_memory_mode")]
    pub mode: String,
    #[serde(default = "default_cli_check_interval")]
    pub cli_check_interval_secs: u64,
    #[serde(default = "default_retrieval_threshold")]
    pub retrieval_review_threshold_days: i64,
    #[serde(default = "default_distillation_interval")]
    pub distillation_interval_secs: u64,
    #[serde(default = "default_cloud_provider")]
    pub cloud_provider: String,
    #[serde(default = "default_cloud_model")]
    pub cloud_model: String,
    #[serde(default = "default_cloud_limit")]
    pub cloud_monthly_limit: f64,
    #[serde(default)]
    pub local_preprocessing: bool,
    #[serde(default = "default_local_backend")]
    pub local_backend: String,
    #[serde(default = "default_local_model")]
    pub local_model: String,
    #[serde(default)]
    pub full_local: bool,
    #[serde(default = "default_local_llm_provider")]
    pub local_llm_provider: String,
    #[serde(default = "default_local_llm_url")]
    pub local_llm_base_url: String,
    #[serde(default = "default_local_llm_model")]
    pub local_llm_model: String,
}

fn default_port() -> u16 { 8080 }
fn default_host() -> String { "0.0.0.0".into() }
fn default_db_path() -> String {
    Paths::database_file().to_string_lossy().to_string()
}
fn default_provider() -> String { "openai".into() }
fn default_system_prompt_path() -> String {
    Paths::system_prompt_file().to_string_lossy().to_string()
}
fn default_user_prompt_path() -> String {
    Paths::user_prompt_file().to_string_lossy().to_string()
}
fn default_true() -> bool { true }

fn default_memory_enabled() -> bool { true }
fn default_vault_path() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("USERPROFILE")
            .map(|home| format!("{}\\alfred", home))
            .unwrap_or_else(|_| "C:\\Users\\alfred".into())
    } else {
        std::env::var("HOME")
            .map(|home| format!("{}/alfred", home))
            .unwrap_or_else(|_| "/home/user/alfred".into())
    }
}
fn default_memory_mode() -> String { "auto".into() }
fn default_cli_check_interval() -> u64 { 300 }
fn default_retrieval_threshold() -> i64 { 90 }
fn default_distillation_interval() -> u64 { 7200 }
fn default_cloud_provider() -> String { "openai".into() }
fn default_cloud_model() -> String { "gpt-4o-mini".into() }
fn default_cloud_limit() -> f64 { 5.0 }
fn default_local_backend() -> String { "auto".into() }
fn default_local_model() -> String { "phi-4-mini-instruct".into() }
fn default_local_llm_provider() -> String { "ollama".into() }
fn default_local_llm_url() -> String { "http://localhost:11434/v1".into() }
fn default_local_llm_model() -> String { "qwen3:8b".into() }

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { enabled: true }
    }
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            vault_path: default_vault_path(),
            mode: "auto".into(),
            cli_check_interval_secs: 300,
            retrieval_review_threshold_days: 90,
            distillation_interval_secs: 7200,
            cloud_provider: "openai".into(),
            cloud_model: "gpt-4o-mini".into(),
            cloud_monthly_limit: 5.0,
            local_preprocessing: false,
            local_backend: "auto".into(),
            local_model: "phi-4-mini-instruct".into(),
            full_local: false,
            local_llm_provider: "ollama".into(),
            local_llm_base_url: "http://localhost:11434/v1".into(),
            local_llm_model: "qwen3:8b".into(),
        }
    }
}

fn expand_env_vars(s: &str) -> String {
    let mut result = s.to_string();
    while let Some(start) = result.find("${") {
        if let Some(end) = result[start + 2..].find('}') {
            let var_name = &result[start + 2..start + 2 + end];
            let value = std::env::var(var_name).unwrap_or_default();
            result.replace_range(start..start + 2 + end + 1, &value);
        } else {
            break;
        }
    }
    result
}

pub fn load_config(path: &Path) -> Result<AppConfig, AlfredError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| AlfredError::Config(format!("failed to read {}: {}", path.display(), e)))?;
    let expanded = expand_env_vars(&content);
    let config: AppConfig = toml::from_str(&expanded)
        .map_err(|e| AlfredError::Config(format!("failed to parse config: {}", e)))?;

    Ok(config)
}
