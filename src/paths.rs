use std::path::PathBuf;

/// Environment variable that overrides [`Paths::data_dir`].
///
/// Point it at a throwaway directory to run Alfred against an isolated SQLite
/// database (plus the workspace and per-job scratch directories) without
/// touching the user's `~/.alfred` — and without overriding `USERPROFILE` /
/// `HOME`, which rustup and `cargo` also read.
pub const DATA_DIR_ENV: &str = "ALFRED_DATA_DIR";

pub struct Paths;

impl Paths {
    /// Root directory: ~/.alfred (or %USERPROFILE%\.alfred on Windows)
    pub fn home_dir() -> PathBuf {
        if cfg!(target_os = "windows") {
            std::env::var("USERPROFILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| {
                    let home = std::env::var("HOME").expect("Cannot determine home directory");
                    PathBuf::from(home)
                })
                .join(".alfred")
        } else {
            let home = std::env::var("HOME").expect("Cannot determine home directory");
            PathBuf::from(home).join(".alfred")
        }
    }

    /// Config directory: ~/.alfred/config
    pub fn config_dir() -> PathBuf {
        Self::home_dir().join("config")
    }

    /// The data directory named by [`DATA_DIR_ENV`], when it is set to a
    /// non-empty value.
    ///
    /// This is the override consulted by [`Paths::data_dir`] and, through it,
    /// by [`Paths::database_file`], [`Paths::workspace_dir`] and
    /// [`Paths::job_workspace_dir`].
    pub fn data_dir_override() -> Option<PathBuf> {
        std::env::var(DATA_DIR_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(PathBuf::from)
    }

    /// Data directory: `$ALFRED_DATA_DIR` when set, else `~/.alfred/data`.
    pub fn data_dir() -> PathBuf {
        Self::data_dir_override().unwrap_or_else(|| Self::home_dir().join("data"))
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
                .map(|home| {
                    PathBuf::from(home)
                        .join(".config")
                        .join("alfred")
                        .join("config.toml")
                })
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

    /// Pi skills shipped with Alfred: ~/.alfred/config/skills
    ///
    /// This is the directory passed to Pi as `--skill`, and where
    /// [`crate::skills`] writes the generated `SKILL.md` files.
    pub fn skills_dir() -> PathBuf {
        Self::config_dir().join("skills")
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

#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    /// Set `ALFRED_DATA_DIR` for the duration of `body`, then restore it.
    fn with_data_dir_override<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().expect("env lock");
        let previous = std::env::var(DATA_DIR_ENV).ok();
        match value {
            Some(value) => std::env::set_var(DATA_DIR_ENV, value),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        let result = body();
        match previous {
            Some(previous) => std::env::set_var(DATA_DIR_ENV, previous),
            None => std::env::remove_var(DATA_DIR_ENV),
        }
        result
    }

    #[test]
    fn data_dir_override_redirects_data_and_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = dir.path().to_path_buf();

        let (data_dir, database) =
            with_data_dir_override(Some(dir.path().to_str().unwrap()), || {
                (Paths::data_dir(), Paths::database_file())
            });

        assert_eq!(data_dir, expected);
        assert_eq!(database, expected.join("alfred.db"));
    }

    #[test]
    fn unset_or_blank_override_falls_back_to_the_home_directory() {
        let unset = with_data_dir_override(None, || Paths::data_dir());
        assert_eq!(unset, Paths::home_dir().join("data"));

        let blank = with_data_dir_override(Some("   "), || Paths::data_dir());
        assert_eq!(blank, Paths::home_dir().join("data"));
    }
}
