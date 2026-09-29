use alfred::{cli, config, config_watch, connectors, jobs, paths, pi, scheduler, server, skills, store};

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use clap::Parser;
use tokio::sync::Mutex;
use tracing::{info, error};
use tracing_subscriber::EnvFilter;

use cli::Cli;
use config::load_config;
use connectors::Connector;
use connectors::telegram::{shutdown_sessions, Session, TelegramConnector};
use jobs::delivery::Delivery;
use jobs::dispatch::JobDispatch;
use jobs::runner::{JobRunner, MissingVerdict};
use paths::Paths;
use scheduler::{Scheduler, SchedulerConfig, SystemClock};
use server::{start_server, AppState};
use store::Store;

const DEFAULT_CONFIG_PATH: &str = "config/config.toml";
const EXAMPLE_CONFIG_PATH: &str = "config/config.toml.example";
const EXAMPLE_SYSTEM_PROMPT: &str = "prompts/system.md.example";
const EXAMPLE_USER_PROMPT: &str = "prompts/user.md.example";

fn ensure_directories() {
    let dirs = [
        Paths::config_dir(),
        Paths::prompts_dir(),
        Paths::data_dir(),
        Paths::logs_dir(),
    ];
    for dir in &dirs {
        if !dir.exists() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                eprintln!("Warning: Could not create directory {}: {}", dir.display(), e);
            }
        }
    }
}

fn auto_generate_config_files() {
    let config_path = Paths::config_file();
    let system_prompt_path = Paths::system_prompt_file();
    let user_prompt_path = Paths::user_prompt_file();

    // Check for legacy config in current directory first
    let legacy_config = Path::new(DEFAULT_CONFIG_PATH);
    if legacy_config.exists() && !config_path.exists() {
        println!("Found legacy config at {}. Migrating to {}...", legacy_config.display(), config_path.display());
        if let Err(e) = std::fs::copy(legacy_config, &config_path) {
            eprintln!("Failed to migrate config: {}", e);
        }
    }

    // Auto-generate config if not exists
    if !config_path.exists() {
        if Path::new(EXAMPLE_CONFIG_PATH).exists() {
            println!("Alfred is starting for the first time.");
            println!("  Creating config directory: {}", Paths::config_dir().display());
            println!("  Please edit {} and fill in your API keys.", config_path.display());
            println!();
            if let Err(e) = std::fs::copy(EXAMPLE_CONFIG_PATH, &config_path) {
                eprintln!("  Failed to generate config: {}", e);
                eprintln!("  Please create {} manually.", config_path.display());
                std::process::exit(1);
            }
        } else {
            eprintln!("ERROR: No config file found and no template available.");
            eprintln!("  Please create {} manually.", config_path.display());
            std::process::exit(1);
        }
    }

    // Auto-generate system prompt if not exists
    if !system_prompt_path.exists() {
        if Path::new(EXAMPLE_SYSTEM_PROMPT).exists() {
            if let Err(e) = std::fs::copy(EXAMPLE_SYSTEM_PROMPT, &system_prompt_path) {
                tracing::warn!("Could not generate system prompt: {}", e);
            }
        }
    }

    // Auto-generate user prompt if not exists
    if !user_prompt_path.exists() {
        if Path::new(EXAMPLE_USER_PROMPT).exists() {
            if let Err(e) = std::fs::copy(EXAMPLE_USER_PROMPT, &user_prompt_path) {
                tracing::warn!("Could not generate user prompt: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Ensure log directory exists before initializing logging
    let log_dir = Paths::logs_dir();
    if !log_dir.exists() {
        let _ = std::fs::create_dir_all(&log_dir);
    }

    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(Paths::log_file())
        .expect("Failed to open log file");

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_writer(std::sync::Mutex::new(log_file))
        .init();

    ensure_directories();
    auto_generate_config_files();
    // Regenerate the Pi skills that document the agent's CLI capabilities. A
    // failure is logged by `skills` and never blocks startup.
    skills::generate_skills();

    let Cli { config, command } = Cli::parse();

    if let Some(command) = command {
        let config_path = resolve_config_path(config.as_deref());
        if let Err(error) = cli::run(command, &config_path).await {
            eprintln!("ERROR: {error}");
            std::process::exit(1);
        }
        return;
    }

    run_server_mode(&config).await;
}

fn resolve_config_path(user_path: Option<&str>) -> String {
    if let Some(path) = user_path {
        return path.to_string();
    }

    let xdg_config = Paths::config_file();
    if xdg_config.exists() {
        return xdg_config.to_string_lossy().to_string();
    }

    let legacy_config = Path::new(DEFAULT_CONFIG_PATH);
    if legacy_config.exists() {
        return DEFAULT_CONFIG_PATH.to_string();
    }

    xdg_config.to_string_lossy().to_string()
}

async fn run_server_mode(config_path: &Option<String>) {
    info!("Alfred starting up...");

    let config_path_str = resolve_config_path(config_path.as_deref());
    let config = match load_config(Path::new(&config_path_str)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            eprintln!("ERROR: Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    // Fail fast: a Pi binary that cannot be launched must stop startup naming
    // the path, rather than surfacing on the first job after the server is up.
    if let Err(error) = pi::invocation::resolve_pi_binary(&config.pi.binary) {
        error!("{error}");
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }

    let state = match initialize_state(&config).await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initialize: {}", e);
            eprintln!("ERROR: Failed to initialize: {}", e);
            std::process::exit(1);
        }
    };

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("Alfred server started");
    println!("  PID: {}", std::process::id());
    println!("  Listening on: {}", addr);
    println!("  Config: {}", Paths::config_file().display());
    println!("  Data: {}", Paths::data_dir().display());
    println!("  Logs: {}", Paths::logs_dir().display());

    info!("Alfred initialized. Starting services...");

    let mut handles = Vec::new();

    // REST surface.
    let server_state = state.clone();
    let server_config = config.server.clone();
    handles.push(tokio::spawn(async move {
        if let Err(e) = start_server(&server_config, server_state).await {
            error!("Server error: {}", e);
        }
    }));

    // Scheduler: run due jobs through Pi and deliver their results. A Telegram
    // failure never stops this loop; a delivery failure is recorded on the run.
    if config.jobs.enabled {
        let runner = Arc::new(JobRunner::new(
            config.pi.clone(),
            MissingVerdict::parse(&config.jobs.missing_verdict),
        ));
        let token = config
            .telegram
            .as_ref()
            .and_then(|telegram| telegram.bot_token.clone());
        let allowed_users: &[u64] = config
            .telegram
            .as_ref()
            .map(|telegram| telegram.allowed_users.as_slice())
            .unwrap_or(&[]);
        let delivery = Arc::new(Delivery::new(token, allowed_users));
        let dispatch = Arc::new(JobDispatch::new(Arc::clone(&state.store), runner, delivery));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&state.store),
            Arc::new(SystemClock),
            dispatch,
            SchedulerConfig::from_jobs(&config.jobs),
        ));
        handles.push(tokio::spawn(scheduler.run()));
        info!(
            max_concurrent = config.jobs.max_concurrent,
            "scheduler started"
        );
    } else {
        info!("[jobs].enabled is false; the scheduler is not started");
    }

    // Telegram connector, with its per-channel session supervisor. A failure to
    // build or start it is logged and leaves the scheduler running.
    let mut sessions: Option<Arc<Mutex<HashMap<String, Session>>>> = None;
    if let Some(ref tg_config) = config.telegram {
        if tg_config.bot_token.is_some() {
            let tg_state = state.clone();
            match TelegramConnector::new(tg_config, &config.pi, &config.prompt, tg_state) {
                Ok(connector) => {
                    sessions = Some(connector.sessions());
                    handles.push(tokio::spawn(async move {
                        if let Err(e) = connector.start().await {
                            error!("Telegram connector error: {}", e);
                        }
                    }));
                    info!("Telegram connector started");
                }
                Err(e) => error!("Failed to create Telegram connector: {}", e),
            }
        }
    }

    // Watch the config file(s) and reject invalid changes without a restart.
    let watch_paths = config_watch::watch_paths(Path::new(&config_path_str));
    {
        let path_list: Vec<String> = watch_paths.iter().map(|p| p.display().to_string()).collect();
        info!("Watching config for changes: {}", path_list.join(", "));
    }
    handles.push(tokio::spawn(async move {
        config_watch::watch_config(watch_paths).await;
    }));

    tokio::signal::ctrl_c().await.expect("failed to listen for ctrl-c");
    info!("Shutting down...");

    for handle in handles {
        handle.abort();
    }

    // Abort and reap every long-lived Pi child (the channel sessions) so none is
    // orphaned. Per-job children are short-lived and reaped by the runner on
    // every path; one still in flight is terminated by its `kill_on_drop`
    // handle when the runtime shuts down.
    if let Some(ref sessions) = sessions {
        shutdown_sessions(sessions).await;
    }
}

async fn initialize_state(config: &config::AppConfig) -> Result<AppState, Box<dyn std::error::Error>> {
    let store = Arc::new(Store::new(Paths::database_file().as_path())?);
    let pi_version = server::probe_pi_version(&config.pi.binary);

    Ok(AppState {
        store,
        start_time: Instant::now(),
        active_connections: Arc::new(AtomicUsize::new(0)),
        port: config.server.port,
        api_key: config.server.api_key.clone(),
        pi: config.pi.clone(),
        jobs: config.jobs.clone(),
        telegram: config.telegram.clone(),
        pi_version,
    })
}
