mod agent;
mod bus;
mod config;
mod connectors;
mod error;
mod llm;
mod memory;
mod paths;
mod prompt;
mod scheduler;
mod server;
mod session;
mod store;
mod tools;
mod tui;
mod types;
mod workitem;
mod workspace;

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
    /// Dump a test screen to file and exit (for TUI testing)
    #[arg(long)]
    dump: bool,
    /// Dump the command palette to file and exit (for TUI testing)
    #[arg(long)]
    dump_palette: bool,
    /// Filter to apply in --dump-palette (e.g. "/theme")
    #[arg(long)]
    palette_filter: Option<String>,
    /// Move palette selection down N times in --dump-palette
    #[arg(long, default_value_t = 0)]
    palette_select: usize,
    /// Theme to use for dumps: dark or light
    #[arg(long)]
    dump_theme: Option<String>,
    /// Manage work items (for autonomous development)
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Parser)]
enum CliCommand {
    /// Work item management
    #[command(name = "workitem")]
    WorkItem(WorkItemArgs),
    /// Scheduled task management (OS cron / Task Scheduler)
    #[command(name = "scheduler")]
    Scheduler(SchedulerArgs),
}

#[derive(Parser)]
struct SchedulerArgs {
    #[command(subcommand)]
    command: SchedulerCommand,
}

#[derive(Parser)]
enum SchedulerCommand {
    /// List scheduled jobs
    List,
    /// Add a scheduled job
    Add {
        /// Cron schedule, e.g. "*/5 * * * *" or "@daily"
        schedule: String,
        /// Command to run
        command: String,
    },
    /// Remove a scheduled job
    Remove {
        /// Cron schedule of the job to remove
        schedule: String,
        /// Command of the job to remove
        command: String,
    },
}

#[derive(Parser)]
struct WorkItemArgs {
    #[command(subcommand)]
    command: WorkItemCommand,
}

#[derive(Parser)]
enum WorkItemCommand {
    /// List all work items
    List {
        /// Filter by status: pending, in_progress, blocked, completed, failed
        #[arg(short, long)]
        status: Option<String>,
    },
    /// Show the next work item to work on
    Next,
    /// Show details of a specific work item
    Show {
        /// Work item ID
        id: String,
    },
    /// Add a new work item
    Add {
        /// Title
        #[arg(short, long)]
        title: String,
        /// Description
        #[arg(short, long)]
        description: String,
        /// Priority: critical, high, medium, low
        #[arg(short, long, default_value = "medium")]
        priority: String,
        /// Category: infrastructure, feature, bug, experiment, refactor
        #[arg(short, long, default_value = "feature")]
        category: String,
        /// Verification command (shell command to verify completion)
        #[arg(short, long)]
        verification: Option<String>,
        /// Estimated effort: S, M, L, XL
        #[arg(long)]
        effort: Option<String>,
        /// Dependencies (comma-separated work item IDs)
        #[arg(long)]
        depends: Option<String>,
    },
    /// Assign a work item to an agent
    Assign {
        /// Work item ID
        id: String,
        /// Agent ID
        agent: String,
    },
    /// Update work item status
    Update {
        /// Work item ID
        id: String,
        /// New status: pending, in_progress, blocked, completed, failed
        #[arg(short, long)]
        status: Option<String>,
        /// Note to add
        #[arg(short, long)]
        note: Option<String>,
    },
    /// Complete a work item with verification output
    Complete {
        /// Work item ID
        id: String,
        /// Verification output
        #[arg(short, long)]
        verification: String,
    },
    /// Log progress on a work item
    Log {
        /// Work item ID
        id: String,
        /// Progress note
        #[arg(short, long)]
        note: String,
    },
}

fn ensure_directories() {
    let dirs = [
        Paths::config_dir(),
        Paths::prompts_dir(),
        Paths::data_dir(),
        Paths::logs_dir(),
        Paths::themes_dir(),
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
    tui::theme::ensure_default_themes();

    let cli = Cli::parse();

    if cli.dump_palette {
        run_dump_mode(
            &cli.config,
            cli.dump_theme.as_deref(),
            true,
            cli.palette_filter.as_deref(),
            cli.palette_select,
        )
        .await;
        return;
    }

    if cli.dump || (cli.dump_theme.is_some() && !cli.dump_palette) {
        run_dump_mode(&cli.config, cli.dump_theme.as_deref(), false, None, 0).await;
        return;
    }

    if let Some(command) = cli.command {
        match command {
            CliCommand::WorkItem(workitem_args) => {
                run_workitem_command(workitem_args.command, &cli.config).await
            }
            CliCommand::Scheduler(scheduler_args) => {
                run_scheduler_command(scheduler_args.command)
            }
        }
        return;
    }

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

    let vault_path = std::path::PathBuf::from(&config.memory.vault_path);
    if config.memory.enabled {
        if let Err(e) = memory::vault::scaffold_vault(&vault_path) {
            tracing::warn!("Failed to scaffold vault: {}", e);
        }
    }

    let provider_name = &config.llm.default_provider;
    let provider_config = config.llm.providers.get(provider_name)
        .ok_or_else(|| format!("Provider '{}' not found in config", provider_name))?;
    let provider = create_provider(provider_name, provider_config)?;
    let model = provider_config.model.clone();

    let prompt_layers = load_prompt_layers(&config.prompt, &store)?;
    let system_prompt = prompt::assemble_system(&prompt_layers);

    let tools = Arc::new(register_builtins(store.clone()));
    let (event_tx, _) = tokio::sync::broadcast::channel(256);
    let bus = Arc::new(crate::bus::MessageBus::new(256));

    Ok(AppState {
        store,
        tools,
        provider,
        model,
        system_prompt,
        event_tx,
        bus,
        start_time: Instant::now(),
        active_connections: Arc::new(AtomicUsize::new(0)),
        port: config.server.port,
        api_key: None,
        vault_path,
    })
}

async fn run_dump_mode(
    config_path: &Option<String>,
    theme_name: Option<&str>,
    with_palette: bool,
    palette_filter: Option<&str>,
    palette_select: usize,
) {
    info!("Alfred dump mode...");

    let config_path_str = resolve_config_path(config_path.as_deref());
    let config = match load_config(std::path::Path::new(&config_path_str)) {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to load config: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize full state (store, provider, tools, prompts, vault)
    let state = match initialize_state(&config).await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to initialize: {}", e);
            eprintln!("ERROR: Failed to initialize: {}", e);
            std::process::exit(1);
        }
    };

    let theme_name = theme_name.unwrap_or("dark");
    let theme = tui::theme::Theme::load(theme_name).unwrap_or_else(|_| {
        if theme_name == "light" {
            tui::theme::Theme::default_light()
        } else {
            tui::theme::Theme::default_dark()
        }
    });

    let server_url = format!("http://localhost:{}", config.server.port);
    let mut app = tui::TuiApp::new(server_url);
    app.set_theme(theme);

    // Run a real 2-exchange conversation through the agent loop
    let samples = [
        "Hello there! What is your name?",
        "What is 2+2?",
    ];
    let mut history: Vec<types::Message> = Vec::new();

    for sample in &samples {
        let now = chrono::Utc::now();
        history.push(types::Message::User(types::UserMessage {
            content: vec![types::Content::Text(types::TextContent { text: sample.to_string() })],
            timestamp: now,
        }));
        app.push_message(tui::ChatMessage::User {
            text: sample.to_string(),
            timestamp: now.format("%H:%M").to_string(),
        });

        let mut ctx = agent::AgentLoopContext {
            system_prompt: state.system_prompt.clone(),
            messages: history.clone(),
            provider: state.provider.clone(),
            model: state.model.clone(),
            tools: state.tools.clone(),
            event_tx: state.event_tx.clone(),
            max_turns: 10,
        };
        agent::run_agent_loop(&mut ctx).await;
        history = ctx.messages.clone();

        let reply = history.iter().rev().find_map(|m| {
            let text = types::extract_text(m);
            if !text.is_empty() { Some(text) } else { None }
        }).unwrap_or_else(|| "No response generated.".into());

        app.push_message(tui::ChatMessage::Agent {
            text: reply,
            timestamp: chrono::Utc::now().format("%H:%M").to_string(),
        });
    }

    if with_palette {
        app.open_palette(palette_filter.unwrap_or(""), palette_select);
    }

    // Render through the REAL TUI rendering path (headless backend)
    let content = tui::render_to_text(&mut app, 100, 40);
    let prefix = if with_palette { "screen_dump_palette" } else { "screen_dump" };
    match tui::write_dump(prefix, &content) {
        Ok(path) => println!("Screen dump saved to: {}", path.display()),
        Err(e) => {
            error!("Failed to write dump file: {}", e);
            std::process::exit(1);
        }
    }
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

async fn run_workitem_command(cmd: WorkItemCommand, config_path: &Option<String>) {
    let store = match Store::new(Paths::database_file().as_path()) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to open database: {}", e);
            std::process::exit(1);
        }
    };

    match cmd {
        WorkItemCommand::List { status } => {
            match store.list_workitems(status.as_deref()) {
                Ok(items) => {
                    if items.is_empty() {
                        println!("No work items found.");
                    } else {
                        println!("{:<8} {:<10} {:<10} {:<50}", "ID", "STATUS", "PRIORITY", "TITLE");
                        println!("{}", "-".repeat(88));
                        for item in &items {
                            println!("{:<8} {:<10} {:<10} {:<50}", &item.id[..8], item.status, item.priority, &item.title[..50.min(item.title.len())]);
                        }
                    }
                }
                Err(e) => {
                    error!("Failed to list work items: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Next => {
            match store.get_next_workitem() {
                Ok(Some(item)) => {
                    println!("Next work item: {}", item.id);
                    println!("Title: {}", item.title);
                    println!("Priority: {}", item.priority);
                    println!("Category: {}", item.category);
                    println!("Description: {}", item.description);
                    println!("Effort: {}", item.estimated_effort.unwrap_or_else(|| "Unknown".to_string()));
                    if let Some(verification) = &item.verification_command {
                        println!("Verification: {}", verification);
                    }
                }
                Ok(None) => {
                    println!("No pending work items available.");
                }
                Err(e) => {
                    error!("Failed to get next work item: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Show { id } => {
            match store.get_workitem(&id) {
                Ok(Some(item)) => {
                    println!("Work Item: {}", item.id);
                    println!("Title: {}", item.title);
                    println!("Description: {}", item.description);
                    println!("Status: {}", item.status);
                    println!("Priority: {}", item.priority);
                    println!("Category: {}", item.category);
                    println!("Effort: {}", item.estimated_effort.unwrap_or_else(|| "Unknown".to_string()));
                    println!("Created: {}", chrono::DateTime::from_timestamp(item.created_at, 0).unwrap_or_default());
                    println!("Updated: {}", chrono::DateTime::from_timestamp(item.updated_at, 0).unwrap_or_default());
                    if !item.acceptance_criteria.is_empty() {
                        println!("Acceptance Criteria: {}", item.acceptance_criteria);
                    }
                    if let Some(verification) = &item.verification_command {
                        println!("Verification: {}", verification);
                    }
                    if let Some(agent) = &item.assigned_agent {
                        println!("Assigned Agent: {}", agent);
                    }
                }
                Ok(None) => {
                    println!("Work item not found: {}", id);
                }
                Err(e) => {
                    error!("Failed to get work item: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Add { title, description, priority, category, verification, effort, depends } => {
            let depends_on = depends.map(|d| {
                d.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>()
            });
            let new_item = workitem::NewWorkItem {
                title,
                description,
                acceptance_criteria: vec![],  // Will be set later
                priority,
                category,
                depends_on,
                verification_command: verification,
                estimated_effort: effort,
            };
            match store.add_workitem(&new_item) {
                Ok(id) => {
                    println!("Work item created: {}", id);
                }
                Err(e) => {
                    error!("Failed to create work item: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Assign { id, agent } => {
            match store.assign_workitem(&id, &agent) {
                Ok(()) => {
                    println!("Work item {} assigned to {}", id, agent);
                }
                Err(e) => {
                    error!("Failed to assign work item: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Update { id, status, note } => {
            if let Some(status) = status {
                match store.update_workitem_status(&id, &status, note.as_deref()) {
                    Ok(()) => {
                        println!("Work item {} status updated to {}", id, status);
                    }
                    Err(e) => {
                        error!("Failed to update work item: {}", e);
                        std::process::exit(1);
                    }
                }
            }
        }
        WorkItemCommand::Complete { id, verification } => {
            match store.complete_workitem(&id, &verification) {
                Ok(()) => {
                    println!("Work item {} completed", id);
                }
                Err(e) => {
                    error!("Failed to complete work item: {}", e);
                    std::process::exit(1);
                }
            }
        }
        WorkItemCommand::Log { id, note } => {
            match store.log_workitem_progress(&id, &note) {
                Ok(()) => {
                    println!("Progress logged for work item {}", id);
                }
                Err(e) => {
                    error!("Failed to log progress: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}

fn run_scheduler_command(cmd: SchedulerCommand) {
    match cmd {
        SchedulerCommand::List => match scheduler::control::list_jobs() {
            Ok(jobs) => print!("{}", scheduler::control::format_jobs(&jobs)),
            Err(e) => {
                error!("Failed to list scheduled jobs: {}", e);
                eprintln!("ERROR: {}", e);
                std::process::exit(1);
            }
        },
        SchedulerCommand::Add { schedule, command } => {
            match scheduler::control::add_job(&schedule, &command) {
                Ok(entry) => println!("Scheduled job added: {}", entry.to_line()),
                Err(e) => {
                    error!("Failed to add scheduled job: {}", e);
                    eprintln!("ERROR: {}", e);
                    std::process::exit(1);
                }
            }
        }
        SchedulerCommand::Remove { schedule, command } => {
            match scheduler::control::remove_job(&schedule, &command) {
                Ok(true) => println!("Scheduled job removed: {} {}", schedule, command),
                Ok(false) => println!("No matching scheduled job found."),
                Err(e) => {
                    error!("Failed to remove scheduled job: {}", e);
                    eprintln!("ERROR: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
