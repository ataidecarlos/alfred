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

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn database_file() -> PathBuf {
        Self::data_dir().join("alfred.db")
    }

    pub fn log_file() -> PathBuf {
        Self::logs_dir().join("server.log")
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

    pub fn themes_dir() -> PathBuf {
        Self::config_dir().join("themes")
    }
}
