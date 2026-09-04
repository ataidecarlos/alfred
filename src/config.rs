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

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self { enabled: true }
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
