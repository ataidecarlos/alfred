use std::path::Path;

use serde::Deserialize;

use crate::error::AlfredError;
use crate::paths::Paths;

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    pub prompt: PromptConfig,
    #[serde(default)]
    pub pi: PiConfig,
    #[serde(default)]
    pub jobs: JobsConfig,
    #[serde(default)]
    pub webhook: WebhookConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_db_path")]
    pub db_path: String,
    /// Bearer token required by the REST surface when set.
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
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

/// How Alfred invokes the Pi subprocess.
#[derive(Debug, Deserialize, Clone)]
pub struct PiConfig {
    #[serde(default = "default_pi_binary")]
    pub binary: String,
    #[serde(default = "default_pi_api_key_env")]
    pub api_key_env: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_pi_thinking")]
    pub thinking: String,
    #[serde(default)]
    pub jobs_tools: Vec<String>,
    #[serde(default)]
    pub channel_tools: Vec<String>,
    #[serde(default = "default_pi_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_idle_compact_secs")]
    pub idle_compact_secs: u64,
    #[serde(default = "default_compact_token_threshold")]
    pub compact_token_threshold: u64,
    #[serde(default = "default_pi_session_dir")]
    pub session_dir: String,
    #[serde(default)]
    pub extra_args: Vec<String>,
}

/// Scheduling policy for the job model.
#[derive(Debug, Deserialize, Clone)]
pub struct JobsConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
    #[serde(default = "default_min_watch_interval_secs")]
    pub min_watch_interval_secs: u64,
    #[serde(default = "default_max_runs_per_job")]
    pub max_runs_per_job: usize,
    #[serde(default = "default_missing_verdict")]
    pub missing_verdict: String,
}

/// Policy for the agent's outbound `alfred webhook send` capability.
#[derive(Debug, Deserialize)]
pub struct WebhookConfig {
    /// Hosts the agent may POST to. An empty list denies every host — the
    /// default, because the agent runs unattended on a 24x7 host.
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
}

fn default_port() -> u16 { 8080 }
fn default_host() -> String { "127.0.0.1".into() }
fn default_db_path() -> String {
    Paths::database_file().to_string_lossy().to_string()
}
fn default_system_prompt_path() -> String {
    Paths::system_prompt_file().to_string_lossy().to_string()
}
fn default_user_prompt_path() -> String {
    Paths::user_prompt_file().to_string_lossy().to_string()
}
fn default_true() -> bool { true }

fn default_pi_binary() -> String { "pi".into() }
fn default_pi_api_key_env() -> String { "PI_API_KEY".into() }
fn default_pi_thinking() -> String { "off".into() }
fn default_pi_timeout_secs() -> u64 { 900 }
fn default_idle_compact_secs() -> u64 { 43_200 }
fn default_compact_token_threshold() -> u64 { 60_000 }
fn default_pi_session_dir() -> String {
    Paths::pi_dir().join("sessions").to_string_lossy().to_string()
}

fn default_max_concurrent() -> usize { 2 }
fn default_min_watch_interval_secs() -> u64 { 900 }
fn default_max_runs_per_job() -> usize { 100 }
fn default_missing_verdict() -> String { "notify".into() }

impl Default for PiConfig {
    fn default() -> Self {
        Self {
            binary: default_pi_binary(),
            api_key_env: default_pi_api_key_env(),
            provider: String::new(),
            model: String::new(),
            thinking: default_pi_thinking(),
            jobs_tools: Vec::new(),
            channel_tools: Vec::new(),
            timeout_secs: default_pi_timeout_secs(),
            idle_compact_secs: default_idle_compact_secs(),
            compact_token_threshold: default_compact_token_threshold(),
            session_dir: default_pi_session_dir(),
            extra_args: Vec::new(),
        }
    }
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_concurrent: default_max_concurrent(),
            min_watch_interval_secs: default_min_watch_interval_secs(),
            max_runs_per_job: default_max_runs_per_job(),
            missing_verdict: default_missing_verdict(),
        }
    }
}

impl Default for WebhookConfig {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The user requirement is a 12-hour idle default and a 60k token threshold.
    /// #4 left these unspecified and a worker chose 300 / 100_000, which would have
    /// compacted every conversation after five minutes of quiet.
    #[test]
    fn compaction_defaults_match_the_specified_policy() {
        let pi = PiConfig::default();
        assert_eq!(pi.idle_compact_secs, 43_200, "idle default must be 12 hours");
        assert_eq!(pi.compact_token_threshold, 60_000);
    }

    #[test]
    fn example_config_loads() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/config.toml.example");
        let config = load_config(&path).expect("bundled example config should parse");
        assert_eq!(config.server.host, "127.0.0.1");
        assert_eq!(config.server.api_key, None);
        assert_eq!(config.pi.binary, "pi");
        assert!(config.jobs.enabled);
        // An unconfigured webhook allow-list denies every host.
        assert!(config.webhook.allowed_hosts.is_empty());
    }

    #[test]
    fn webhook_allowed_hosts_are_parsed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[server]\nport = 9\n\n[prompt]\nsystem_prompt_file = \"s.md\"\nuser_prompt_file = \"u.md\"\n\n\
             [webhook]\nallowed_hosts = [\"example.com\", \"hooks.example.org\"]\n",
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(
            config.webhook.allowed_hosts,
            vec!["example.com".to_string(), "hooks.example.org".to_string()]
        );
    }

    #[test]
    fn missing_webhook_section_defaults_to_deny_all() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[server]\nport = 9\n\n[prompt]\nsystem_prompt_file = \"s.md\"\nuser_prompt_file = \"u.md\"\n",
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert!(config.webhook.allowed_hosts.is_empty());
    }

    #[test]
    fn missing_pi_and_jobs_sections_use_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "[server]\nport = 9\n\n[prompt]\nsystem_prompt_file = \"s.md\"\nuser_prompt_file = \"u.md\"\n",
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.pi.binary, "pi");
        assert_eq!(config.jobs.min_watch_interval_secs, 900);
        assert_eq!(config.jobs.missing_verdict, "notify");
    }
}
