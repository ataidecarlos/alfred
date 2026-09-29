//! `pi --mode rpc` invocation builder.
//!
//! Jobs are ephemeral (`--no-session`); channels keep a persistent session in
//! their own session directory. The flags here are the integration contract
//! with Pi's RPC mode; keep them in sync with
//! `pi/packages/coding-agent/docs/rpc.md`.
//!
//! The provider API key is passed through the environment only. It is never
//! placed on the command line, because `ps` would expose it.

use std::path::PathBuf;

use tokio::process::Command;

use crate::config::PiConfig;
use crate::paths::Paths;

/// Pi's private agent config directory.
const ENV_AGENT_DIR: &str = "PI_CODING_AGENT_DIR";
/// Pi's session storage directory.
const ENV_SESSION_DIR: &str = "PI_CODING_AGENT_SESSION_DIR";
/// Keep Pi from checking for a newer version on every job.
const ENV_SKIP_VERSION_CHECK: &str = "PI_SKIP_VERSION_CHECK";
/// Disable Pi's telemetry.
const ENV_TELEMETRY: &str = "PI_TELEMETRY";
/// Keep Pi from making network calls it does not need.
const ENV_OFFLINE: &str = "PI_OFFLINE";

/// A fully resolved `pi --mode rpc` invocation, ready to spawn.
///
/// [`PiInvocation::job`] and [`PiInvocation::channel`] differ in session
/// handling and in which tool allowlist they apply ([`PiConfig::jobs_tools`]
/// versus [`PiConfig::channel_tools`]).
#[derive(Debug, Clone)]
pub struct PiInvocation {
    /// Pi binary path or name (from `[pi].binary`).
    pub binary: String,
    pub provider: String,
    pub model: String,
    pub thinking: String,
    /// Assembled system prompt, passed as a replace (not an append).
    pub system_prompt: String,
    /// Tool allowlist, joined with commas for `--tools`.
    pub tools: Vec<String>,
    /// Directory passed via `--skill`; defaults to
    /// `~/.alfred/config/skills`.
    pub skills_dir: PathBuf,
    /// `PI_CODING_AGENT_DIR`; defaults to `~/.alfred/pi-agent`.
    pub agent_dir: PathBuf,
    /// Session storage directory (`--session-dir` and the session env var).
    pub session_dir: PathBuf,
    /// Channel name for persistent sessions; `None` for a job.
    pub channel: Option<String>,
    /// Parent environment variable that holds the provider API key.
    pub api_key_env: String,
    /// Extra flags appended after the standard invocation.
    pub extra_args: Vec<String>,
}

impl PiInvocation {
    /// Build the invocation for a scheduled job: ephemeral, no session file.
    pub fn job(config: &PiConfig, system_prompt: impl Into<String>) -> Self {
        Self {
            binary: config.binary.clone(),
            provider: config.provider.clone(),
            model: config.model.clone(),
            thinking: config.thinking.clone(),
            system_prompt: system_prompt.into(),
            tools: config.jobs_tools.clone(),
            skills_dir: Paths::skills_dir(),
            agent_dir: Paths::pi_agent_dir(),
            session_dir: PathBuf::from(&config.session_dir),
            channel: None,
            api_key_env: config.api_key_env.clone(),
            extra_args: config.extra_args.clone(),
        }
    }

    /// Build the invocation for a channel: persistent session per channel,
    /// stored under `~/.alfred/pi/<channel>`.
    pub fn channel(config: &PiConfig, system_prompt: impl Into<String>, channel: &str) -> Self {
        Self {
            binary: config.binary.clone(),
            provider: config.provider.clone(),
            model: config.model.clone(),
            thinking: config.thinking.clone(),
            system_prompt: system_prompt.into(),
            tools: config.channel_tools.clone(),
            skills_dir: Paths::skills_dir(),
            agent_dir: Paths::pi_agent_dir(),
            session_dir: Paths::pi_dir().join(channel),
            channel: Some(channel.to_string()),
            api_key_env: config.api_key_env.clone(),
            extra_args: config.extra_args.clone(),
        }
    }

    /// Build the subprocess command line and environment.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.binary);
        command.arg("--mode").arg("rpc");
        if self.channel.is_none() {
            command.arg("--no-session");
        }
        command.arg("--provider").arg(&self.provider);
        command.arg("--model").arg(&self.model);
        command.arg("--thinking").arg(&self.thinking);
        command.arg("--system-prompt").arg(&self.system_prompt);
        command.arg("--no-context-files");
        command.arg("--no-approve");
        command.arg("--tools").arg(self.tools.join(","));
        command.arg("--skill").arg(&self.skills_dir);
        if let Some(channel) = &self.channel {
            command.arg("--session-dir").arg(&self.session_dir);
            command.arg("--name").arg(channel);
        }
        for arg in &self.extra_args {
            command.arg(arg);
        }

        command.env(ENV_AGENT_DIR, &self.agent_dir);
        command.env(ENV_SESSION_DIR, &self.session_dir);
        command.env(ENV_SKIP_VERSION_CHECK, "1");
        command.env(ENV_TELEMETRY, "0");
        command.env(ENV_OFFLINE, "1");
        if let Ok(api_key) = std::env::var(&self.api_key_env) {
            command.env(&self.api_key_env, api_key);
        }
        command
    }
}