use std::path::PathBuf;

pub struct Paths;

impl Paths {
    pub fn config_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            dirs().join("alfred")
        } else {
            dirs().join("alfred")
        }
    }

    pub fn data_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            dirs().join("alfred")
        } else {
            dirs().join("alfred")
        }
    }

    pub fn cache_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            dirs().join("alfred").join("cache")
        } else {
            dirs().join("alfred")
        }
    }

    pub fn config_file() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn database_file() -> PathBuf {
        Self::data_dir().join("alfred.db")
    }

    pub fn log_file() -> PathBuf {
        Self::cache_dir().join("server.log")
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
}

#[cfg(target_os = "windows")]
fn dirs() -> PathBuf {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("USERPROFILE")
                .expect("Cannot determine home directory");
            PathBuf::from(home).join("AppData").join("Roaming")
        })
}

#[cfg(not(target_os = "windows"))]
fn dirs() -> PathBuf {
    std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME")
                .expect("Cannot determine home directory");
            PathBuf::from(home).join(".config")
        })
}
