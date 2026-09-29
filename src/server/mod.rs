pub mod routes;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use axum::{Router, routing::get};
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::Response;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tracing::info;

use crate::config::{JobsConfig, PiConfig, ServerConfig, TelegramConfig};
use crate::agent::event::AgentEvent;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<crate::store::Store>,
    pub event_tx: broadcast::Sender<AgentEvent>,
    pub start_time: Instant,
    pub active_connections: Arc<AtomicUsize>,
    pub port: u16,
    pub api_key: Option<String>,
    /// The loaded `[pi]` section, so validation and spawning use the user's
    /// configuration rather than the defaults.
    pub pi: PiConfig,
    /// The loaded `[jobs]` section.
    pub jobs: JobsConfig,
    /// The loaded `[telegram]` section, when present.
    pub telegram: Option<TelegramConfig>,
    /// The Pi version probed at startup, or `None` when Pi is absent.
    pub pi_version: Option<String>,
}

/// Probe `binary --version` once at startup.
///
/// A missing or failing binary reports `None`, never an error: `/api/info` must
/// stay healthy even when Pi is absent. The binary is the loaded `[pi].binary`,
/// not a default.
pub fn probe_pi_version(binary: &str) -> Option<String> {
    let output = std::process::Command::new(binary)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let version = text.trim();
    if version.is_empty() {
        None
    } else {
        Some(version.to_string())
    }
}

async fn connection_tracker(state: axum::extract::State<AppState>, req: Request, next: Next) -> Response {
    state.active_connections.fetch_add(1, Ordering::Relaxed);
    let response = next.run(req).await;
    state.active_connections.fetch_sub(1, Ordering::Relaxed);
    response
}

/// Build the REST router.
///
/// Exposed separately from [`start_server`] so tests (and future embedders) can
/// drive the exact routes the server binds without opening a fixed port.
pub fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(routes::health))
        .route("/api/info", get(routes::server_info))
        .route("/api/todos", get(routes::list_todos))
        .route("/api/todos", axum::routing::post(routes::add_todo))
        .route("/api/todos/{id}", axum::routing::delete(routes::delete_todo))
        .route("/api/todos/{id}", axum::routing::put(routes::update_todo))
        .route("/api/todos/{id}/complete", axum::routing::post(routes::complete_todo))
        .route("/api/jobs", get(routes::list_jobs))
        .route("/api/jobs", axum::routing::post(routes::create_job))
        .route("/api/jobs/{id}", get(routes::get_job))
        .route("/api/jobs/{id}", axum::routing::put(routes::update_job))
        .route("/api/jobs/{id}", axum::routing::delete(routes::delete_job))
        .route("/api/jobs/{id}/run", axum::routing::post(routes::run_job))
        .route("/api/jobs/{id}/runs", get(routes::list_job_runs))
        .route("/api/memories", get(routes::list_memories))
        .route("/api/memories", axum::routing::post(routes::add_memory))
        .route("/api/memories/{slug}", axum::routing::delete(routes::delete_memory))
        .layer(middleware::from_fn_with_state(state.clone(), connection_tracker))
        .with_state(state)
}

pub async fn start_server(
    config: &ServerConfig,
    state: AppState,
) -> Result<(), crate::error::AlfredError> {
    let app = app(state);

    let addr = format!("{}:{}", config.host, config.port);
    info!("Alfred server listening on {}", addr);
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
