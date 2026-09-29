use std::path::PathBuf;

pub struct Paths;

impl Paths {
    /// Root directory: ~/.alfred (or %USERPROFILE%\.alfred on Windows)
    pub fn home_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME")
                        .expect("Cannot determine home directory");
                    PathBuf::from(home)
                })
                .join(".alfred")
        } else {
            let home = std::env::var("HOME")
                .expect("Cannot determine home directory");
            PathBuf::from(home).join(".alfred")
        }
    }

    /// Config directory: ~/.alfred/config
    pub fn config_dir() -> PathBuf {
        Self::home_dir().join("config")
    }

    /// Data directory: ~/.alfred/data
    pub fn data_dir() -> PathBuf {
        Self::home_dir().join("data")
    }

    /// Logs directory: ~/.alfred/logs
    pub fn logs_dir() -> PathBuf {
        Self::home_dir().join("logs")
    }

    pub fn workspace_dir() -> PathBuf {
        Self::data_dir().join("workspace")
    }

    /// Legacy/XDG config location: ~/.config/alfred/config.toml
    ///
    /// Earlier releases stored the config here. The canonical location is
    /// [`Paths::config_file`], but the hot-reload watcher also observes this
    /// path so existing installs keep working.
    pub fn alt_config_file() -> PathBuf {
        if cfg!(target_os = "windows") {
            std::env::var("APPDATA")
                .map(|appdata| PathBuf::from(appdata).join("alfred").join("config.toml"))
                .unwrap_or_else(|_| Self::config_file())
        } else {
            std::env::var("HOME")
                .map(|home| PathBuf::from(home).join(".config").join("alfred").join("config.toml"))
                .unwrap_or_else(|_| Self::config_file())
        }
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn database_file() -> PathBuf {
        Self::data_dir().join("alfred.db")
    }

    pub fn log_file() -> PathBuf {
        Self::logs_dir().join("alfred.log")
    }

    pub fn prompts_dir() -> PathBuf {
        Self::config_dir().join("prompts")
    }

    pub fn system_prompt_file() -> PathBuf {
        Self::prompts_dir().join("system.md")
    }

    pub fn user_prompt_file() -> PathBuf {
        Self::prompts_dir().join("user.md")
    }

    /// File-backed memories: ~/.alfred/config/memories.md
    pub fn memories_file() -> PathBuf {
        Self::config_dir().join("memories.md")
    }

    /// Pi skills shipped with Alfred: ~/.alfred/skills
    pub fn skills_dir() -> PathBuf {
        Self::home_dir().join("skills")
    }

    /// Alfred's private Pi home: ~/.alfred/pi
    pub fn pi_dir() -> PathBuf {
        Self::home_dir().join("pi")
    }

    /// Pi's private agent config directory: ~/.alfred/pi-agent
    ///
    /// Passed to Pi as `PI_CODING_AGENT_DIR` so it never reads the user's
    /// personal `~/.pi`.
    pub fn pi_agent_dir() -> PathBuf {
        Self::home_dir().join("pi-agent")
    }

    /// Per-job scratch workspace: ~/.alfred/data/jobs/<id>
    pub fn job_workspace_dir(id: &str) -> PathBuf {
        Self::data_dir().join("jobs").join(id)
    }
}
