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

use crate::config::ServerConfig;
use crate::agent::event::AgentEvent;
use crate::llm::LlmProvider;
use crate::agent::tool::ToolRegistry;

/// Mutable configuration that can be swapped at runtime by the config watcher.
///
/// Everything here is derived from the on-disk config and is rebuilt when the
/// file changes, without restarting the server.
#[derive(Clone)]
pub struct RuntimeConfig {
    pub provider_name: String,
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub scheduler_enabled: bool,
}

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<crate::store::Store>,
    pub tools: Arc<ToolRegistry>,
    pub runtime: Arc<tokio::sync::RwLock<RuntimeConfig>>,
    pub system_prompt: String,
    pub event_tx: broadcast::Sender<AgentEvent>,
    pub bus: Arc<crate::bus::MessageBus>,
    pub start_time: Instant,
    pub active_connections: Arc<AtomicUsize>,
    pub port: u16,
    pub api_key: Option<String>,
    pub vault_path: std::path::PathBuf,
}

impl AppState {
    /// The currently active LLM provider (may change after a config reload).
    pub async fn provider(&self) -> Arc<dyn LlmProvider> {
        self.runtime.read().await.provider.clone()
    }

    /// The currently active model name (may change after a config reload).
    pub async fn model(&self) -> String {
        self.runtime.read().await.model.clone()
    }
}

async fn connection_tracker(state: axum::extract::State<AppState>, req: Request, next: Next) -> Response {
    state.active_connections.fetch_add(1, Ordering::Relaxed);
    let response = next.run(req).await;
    state.active_connections.fetch_sub(1, Ordering::Relaxed);
    response
}

pub async fn start_server(
    config: &ServerConfig,
    state: AppState,
) -> Result<(), crate::error::AlfredError> {
    let app = Router::new()
        .route("/health", get(routes::health))
        .route("/api/info", get(routes::server_info))
        .route("/api/messages", axum::routing::post(routes::send_message))
        .route("/api/todos", get(routes::list_todos))
        .route("/api/todos", axum::routing::post(routes::add_todo))
        .route("/api/todos/{id}", axum::routing::delete(routes::delete_todo))
        .route("/api/todos/{id}", axum::routing::put(routes::update_todo))
        .route("/api/todos/{id}/complete", axum::routing::post(routes::complete_todo))
        .route("/api/memories", get(routes::list_memories))
        .route("/api/memories", axum::routing::post(routes::add_memory))
        .route("/api/memories/{id}", axum::routing::delete(routes::delete_memory))
        .layer(middleware::from_fn_with_state(state.clone(), connection_tracker))
        .with_state(state);

    let addr = format!("{}:{}", config.host, config.port);
    info!("Alfred server listening on {}", addr);
    let listener = TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
