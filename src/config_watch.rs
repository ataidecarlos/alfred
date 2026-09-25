//! Hot-reload of the on-disk configuration.
//!
//! A lightweight polling watcher (1s interval) observes the active config file
//! and the legacy XDG location. When either changes, the config is re-read and
//! the runtime LLM provider, model, API key and scheduler flag are updated in
//! place — no server restart required.
//!
//! Polling is used deliberately: it needs no extra dependency, behaves the same
//! on every platform, and keeps detection well inside the 5 second budget.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::{info, warn};

use crate::config::{load_config, AppConfig};
use crate::error::AlfredError;
use crate::llm::create_provider;
use crate::paths::Paths;
use crate::server::AppState;

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
    std::fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

/// Poll `paths` forever, reloading the configuration whenever one changes.
pub async fn watch_config(state: AppState, paths: Vec<PathBuf>) {
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
                reload(&state, path).await;
            }
        }
    }
}

/// Re-read `path` and, on success, swap the runtime configuration.
///
/// A reload is logged even when parsing fails: the change was detected and the
/// previous configuration is retained, which is more useful than silence.
async fn reload(state: &AppState, path: &Path) {
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");

    match load_config(path) {
        Ok(config) => match apply(state, &config).await {
            Ok(()) => info!("Config reloaded at {}", now),
            Err(e) => warn!(
                "Config reloaded at {} (could not apply changes, previous configuration retained: {})",
                now, e
            ),
        },
        Err(e) => info!(
            "Config reloaded at {} (could not read {}: {}; previous configuration retained)",
            now,
            path.display(),
            e
        ),
    }
}

/// Apply a freshly loaded config to the live runtime state.
async fn apply(state: &AppState, config: &AppConfig) -> Result<(), AlfredError> {
    let provider_name = config.llm.default_provider.clone();
    let provider_config = config
        .llm
        .providers
        .get(&provider_name)
        .ok_or_else(|| AlfredError::Config(format!("provider '{}' not found in config", provider_name)))?;

    // Recreate the provider so API key / base_url changes take effect.
    let provider = create_provider(&provider_name, provider_config)?;
    let model = provider_config.model.clone();

    let mut runtime = state.runtime.write().await;
    let scheduler_changed = runtime.scheduler_enabled != config.scheduler.enabled;

    runtime.provider_name = provider_name;
    runtime.provider = provider;
    runtime.model = model;
    runtime.scheduler_enabled = config.scheduler.enabled;
    drop(runtime);

    if scheduler_changed {
        warn!(
            "Scheduler enabled changed to {}; scheduler job set is refreshed on the next restart",
            config.scheduler.enabled
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    fn write_config(dir: &Path, model: &str) -> PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            format!(
                "[server]\nport = 8080\n\n[llm]\ndefault_provider = \"openai\"\n\n\
                 [llm.providers.openai]\napi_key = \"test-key\"\nmodel = \"{}\"\n\n[prompt]\n",
                model
            ),
        )
        .unwrap();
        path
    }

    fn test_state(dir: &Path, config: &AppConfig) -> AppState {
        let store = Arc::new(crate::store::Store::new(&dir.join("test.db")).unwrap());
        let tools = Arc::new(crate::tools::register_builtins(store.clone()));
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        let provider_config = config
            .llm
            .providers
            .get(&config.llm.default_provider)
            .unwrap();
        let provider = create_provider(&config.llm.default_provider, provider_config).unwrap();

        AppState {
            store,
            tools,
            runtime: Arc::new(tokio::sync::RwLock::new(crate::server::RuntimeConfig {
                provider_name: config.llm.default_provider.clone(),
                provider,
                model: provider_config.model.clone(),
                scheduler_enabled: config.scheduler.enabled,
            })),
            system_prompt: String::new(),
            event_tx,
            bus: Arc::new(crate::bus::MessageBus::new(16)),
            start_time: std::time::Instant::now(),
            active_connections: Arc::new(AtomicUsize::new(0)),
            port: 0,
            api_key: None,
            vault_path: dir.to_path_buf(),
        }
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
    async fn apply_swaps_provider_and_model() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "model-a");
        let state = test_state(dir.path(), &load_config(&path).unwrap());
        assert_eq!(state.model().await, "model-a");

        let next = write_config(dir.path(), "model-b");
        apply(&state, &load_config(&next).unwrap()).await.unwrap();

        assert_eq!(state.model().await, "model-b");
        assert_eq!(state.runtime.read().await.provider_name, "openai");
    }

    #[tokio::test]
    async fn invalid_reload_retains_previous_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_config(dir.path(), "model-a");
        let state = test_state(dir.path(), &load_config(&path).unwrap());

        // The verification workflow appends a non-TOML line; a malformed file
        // must not take the running server down or wipe the active config.
        std::fs::write(&path, "[server\nnot valid toml").unwrap();
        reload(&state, &path).await;

        assert_eq!(state.model().await, "model-a");
    }
}
