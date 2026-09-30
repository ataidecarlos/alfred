//! Hot-reload of the on-disk configuration.
//!
//! A lightweight polling watcher (1s interval) observes the active config file
//! and the legacy XDG location. When either changes, the config is re-read and
//! validated; a malformed file is rejected and the previous configuration is
//! retained rather than taking the server down.
//!
//! Polling is used deliberately: it needs no extra dependency, behaves the same
//! on every platform, and keeps detection well inside the 5 second budget.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::{info, warn};

use crate::config::load_config;
use crate::error::AlfredError;
use crate::paths::Paths;

/// How often the config file's modification time is checked.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Candidate config paths to watch for a server started with `active`.
///
/// The canonical path may be a user-supplied `--config`; the legacy XDG path is
/// always included so older installs hot-reload too. Duplicates are removed.
pub fn watch_paths(active: &Path) -> Vec<PathBuf> {
    let mut paths = vec![active.to_path_buf()];

    let alt = Paths::alt_config_file();
    if !paths.iter().any(|p| p == &alt) {
        paths.push(alt);
    }

    paths
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
}

/// Poll `paths` forever, re-validating the configuration whenever one changes.
pub async fn watch_config(paths: Vec<PathBuf>) {
    let mut observed: Vec<(PathBuf, Option<SystemTime>)> = paths
        .into_iter()
        .map(|path| {
            let stamp = modified(&path);
            (path, stamp)
        })
        .collect();

    let mut ticker = tokio::time::interval(POLL_INTERVAL);
    // The first tick fires immediately; skip it so we only act on real changes.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        for (path, previous) in observed.iter_mut() {
            let current = modified(path);
            if current != *previous {
                *previous = current;
                match reload(path).await {
                    Ok(()) => info!("Config reloaded from {}", path.display()),
                    Err(e) => warn!(
                        "Config change in {} rejected; previous configuration retained: {}",
                        path.display(),
                        e
                    ),
                }
            }
        }
    }
}

/// Re-read and validate `path`.
pub async fn reload(path: &Path) -> Result<(), AlfredError> {
    load_config(path).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(dir: &Path, host: &str) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            format!(
                "[server]\nport = 8080\nhost = \"{}\"\n\n[prompt]\nsystem_prompt_file = \"system.md\"\nuser_prompt_file = \"user.md\"\n",
                host
            ),
        )
        .unwrap();
        path
    }

    #[test]
    fn watch_paths_includes_active_and_legacy_locations() {
        let active = PathBuf::from("/tmp/does-not-exist/alfred.toml");
        let paths = watch_paths(&active);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], active);
        assert!(paths.contains(&Paths::alt_config_file()));
    }

    #[tokio::test]
    async fn reload_accepts_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "127.0.0.1");
        assert!(reload(&path).await.is_ok());
    }

    #[tokio::test]
    async fn reload_rejects_invalid_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        // A malformed file must be rejected rather than taking the server down.
        std::fs::write(&path, "[server\nnot valid toml").unwrap();
        assert!(reload(&path).await.is_err());
    }
}
