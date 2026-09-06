mod agent;
mod config;
mod connectors;
mod error;
mod llm;
mod paths;
mod prompt;
mod scheduler;
mod server;
mod store;
mod tools;
mod tui;
mod types;

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

use clap::Parser;
use tracing::{info, error};
use tracing_subscriber::EnvFilter;

use config::load_config;
use connectors::Connector;
use connectors::telegram::TelegramConnector;
use llm::create_provider;
use paths::Paths;
use prompt::load_prompt_layers;
use scheduler::Scheduler;
use server::{AppState, start_server};
use store::Store;
use tools::register_builtins;

const DEFAULT_CONFIG_PATH: &str = "config/config.toml";
const EXAMPLE_CONFIG_PATH: &str = "config/config.toml.example";
const EXAMPLE_SYSTEM_PROMPT: &str = "prompts/system.md.example";
const EXAMPLE_USER_PROMPT: &str = "prompts/user.md.example";

#[derive(Parser)]
#[command(name = "alfred", about = "24x7 AI agent server", version = env!("CARGO_PKG_VERSION"))]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<String>,
    /// Launch the chat TUI (connects to running server)
    #[arg(long)]
    tui: bool,
}

fn ensure_directories() {
    let dirs = [
        Paths::config_dir(),
        Paths::prompts_dir(),
        Paths::data_dir(),
        Paths::cache_dir(),
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
    let legacy_config = std::path::Path::new(DEFAULT_CONFIG_PATH);
    if legacy_config.exists() && !config_path.exists() {
        println!("Found legacy config at {}. Migrating to {}...", legacy_config.display(), config_path.display());
        if let Err(e) = std::fs::copy(legacy_config, &config_path) {
            eprintln!("Failed to migrate config: {}", e);
        }
    }

    // Auto-generate config if not exists
    if !config_path.exists() {
        if std::path::Path::new(EXAMPLE_CONFIG_PATH).exists() {
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
        if std::path::Path::new(EXAMPLE_SYSTEM_PROMPT).exists() {
            if let Err(e) = std::fs::copy(EXAMPLE_SYSTEM_PROMPT, &system_prompt_path) {
                tracing::warn!("Could not generate system prompt: {}", e);
            }
        }
    }

    // Auto-generate user prompt if not exists
    if !user_prompt_path.exists() {
        if std::path::Path::new(EXAMPLE_USER_PROMPT).exists() {
            if let Err(e) = std::fs::copy(EXAMPLE_USER_PROMPT, &user_prompt_path) {
                tracing::warn!("Could not generate user prompt: {}", e);
            }
        }
    }
}

#[tokio::main]
async fn main() {
    // Ensure log directory exists before initializing logging
    let log_dir = Paths::cache_dir();
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

    let cli = Cli::parse();

    if cli.tui {
        run_tui_mode(&cli.config).await;
    } else {
        run_server_mode(&cli.config).await;
    }
}

fn resolve_config_path(user_path: Option<&str>) -> String {
    if let Some(path) = user_path {
        return path.to_string();
    }

    let xdg_config = Paths::config_file();
    if xdg_config.exists() {
        return xdg_config.to_string_lossy().to_string();
    }

    let legacy_config = std::path::Path::new(DEFAULT_CONFIG_PATH);
    if legacy_config.exists() {
        return DEFAULT_CONFIG_PATH.to_string();
    }

    xdg_config.to_string_lossy().to_string()
}

async fn run_server_mode(config_path: &Option<String>) {
    info!("Alfred starting up...");

    let config_path_str = resolve_config_path(config_path.as_deref());
    let config = match load_config(std::path::Path::new(&config_path_str)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    let server_url = format!("http://localhost:{}", config.server.port);
    if tui::check_server(&server_url).await {
        println!("Alfred server is already running.");
        if let Some(info) = tui::get_server_info(&server_url).await {
            println!("  PID: {}", info.pid);
            println!("  Port: {}", info.port);
            println!("  Uptime: {} seconds", info.uptime_secs);
            println!("  Active connections: {}", info.active_connections);
        } else {
            println!("  Could not retrieve server info.");
        }
        return;
    }

    let state = match initialize_state(&config).await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initialize: {}", e);
            std::process::exit(1);
        }
    };

    let addr = format!("{}:{}", config.server.host, config.server.port);
    println!("Alfred server started");
    println!("  PID: {}", std::process::id());
    println!("  Listening on: {}", addr);
    println!("  Config: {}", Paths::config_file().display());
    println!("  Data: {}", Paths::data_dir().display());
    println!("  Logs: {}", Paths::cache_dir().display());

    info!("Alfred initialized. Starting services...");

    let mut handles = Vec::new();

    let server_state = state.clone();
    let server_config = config.server.clone();
    handles.push(tokio::spawn(async move {
        if let Err(e) = start_server(&server_config, server_state).await {
            error!("Server error: {}", e);
        }
    }));

    if let Some(ref tg_config) = config.telegram {
        if tg_config.bot_token.is_some() {
            let tg_state = state.clone();
            match TelegramConnector::new(tg_config, tg_state) {
                Ok(connector) => {
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

    if config.scheduler.enabled {
        let sched = Scheduler::new(state.clone());
        handles.push(tokio::spawn(async move {
            if let Err(e) = sched.start().await {
                error!("Scheduler error: {}", e);
            }
        }));
    }

    tokio::signal::ctrl_c().await.expect("failed to listen for ctrl-c");
    info!("Shutting down...");

    for handle in handles {
        handle.abort();
    }
}

async fn initialize_state(config: &config::AppConfig) -> Result<AppState, Box<dyn std::error::Error>> {
    let store = Arc::new(Store::new(Paths::database_file().as_path())?);

    let provider_name = &config.llm.default_provider;
    let provider_config = config.llm.providers.get(provider_name)
        .ok_or_else(|| format!("Provider '{}' not found in config", provider_name))?;
    let provider = create_provider(provider_name, provider_config)?;
    let model = provider_config.model.clone();

    let prompt_layers = load_prompt_layers(&config.prompt, &store)?;
    let system_prompt = prompt::assemble_system(&prompt_layers);

    let tools = Arc::new(register_builtins(store.clone()));
    let (event_tx, _) = tokio::sync::broadcast::channel(256);

    Ok(AppState {
        store,
        tools,
        provider,
        model,
        system_prompt,
        event_tx,
        start_time: Instant::now(),
        active_connections: Arc::new(AtomicUsize::new(0)),
        port: config.server.port,
        api_key: None,
    })
}

async fn run_tui_mode(config_path: &Option<String>) {
    info!("Alfred TUI starting...");

    let config_path_str = resolve_config_path(config_path.as_deref());
    let config = match load_config(std::path::Path::new(&config_path_str)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    let server_url = format!("http://localhost:{}", config.server.port);

    if !tui::check_server(&server_url).await {
        println!("Server not running. Starting Alfred server...");

        let exe = std::env::current_exe().expect("Failed to get current executable");
        match std::process::Command::new(exe)
            .arg("--config")
            .arg(&config_path_str)
            .spawn()
        {
            Ok(_child) => {
                println!("Server process started. Waiting for it to initialize...");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                if !tui::check_server(&server_url).await {
                    error!("Failed to start server");
                    std::process::exit(1);
                }
                println!("Server is ready.");
            }
            Err(e) => {
                error!("Failed to start server process: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        println!("Connecting to running server at {}", server_url);
    }

    if let Err(e) = tui::run(server_url).await {
        error!("TUI error: {}", e);
        std::process::exit(1);
    }
}
