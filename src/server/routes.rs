use axum::extract::{Path, State, Json};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::extract::FromRequestParts;
use serde::{Deserialize, Serialize};

use crate::agent::{AgentLoopContext, run_agent_loop};
use crate::server::AppState;
use crate::types::Message;

pub struct AuthUser;

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        if let Some(ref required_key) = state.api_key {
            let auth_header = parts.headers.get(AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "));

            match auth_header {
                Some(key) if key == required_key => Ok(AuthUser),
                _ => Err(StatusCode::UNAUTHORIZED),
            }
        } else {
            Ok(AuthUser)
        }
    }
}

pub async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status": "ok"}))
}

#[derive(Serialize)]
pub struct ServerInfo {
    pub pid: u32,
    pub port: u16,
    pub uptime_secs: u64,
    pub active_connections: usize,
}

pub async fn server_info(State(state): State<AppState>) -> Json<ServerInfo> {
    Json(ServerInfo {
        pid: std::process::id(),
        port: state.port,
        uptime_secs: state.start_time.elapsed().as_secs(),
        active_connections: state.active_connections.load(std::sync::atomic::Ordering::Relaxed),
    })
}

#[derive(Deserialize)]
pub struct SendMessageRequest {
    pub user_id: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct SendMessageResponse {
    pub reply: String,
}

pub async fn send_message(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<SendMessageRequest>,
) -> Result<Json<SendMessageResponse>, StatusCode> {
    let mut messages = state.store.load_conversation(&req.user_id, "api")
        .unwrap_or(None)
        .unwrap_or_default();

    use crate::types::{UserMessage, Content};
    let user_msg = Message::User(UserMessage {
        content: vec![Content::Text(crate::types::TextContent { text: req.text })],
        timestamp: chrono::Utc::now(),
    });
    messages.push(user_msg);

    let event_tx = state.event_tx.clone();
    let mut ctx = AgentLoopContext {
        system_prompt: state.system_prompt.clone(),
        messages,
        provider: state.provider.clone(),
        model: state.model.clone(),
        tools: state.tools.clone(),
        event_tx,
        max_turns: 10,
    };

    run_agent_loop(&mut ctx).await;

    let reply = ctx.messages.iter().rev().find_map(|m| {
        if let Message::Assistant(a) = m {
            let text = crate::types::extract_text(m);
            if !text.is_empty() {
                Some(text)
            } else {
                tracing::debug!("Assistant message with no text content: {:?}", a);
                None
            }
        } else {
            None
        }
    }).unwrap_or_else(|| "No response generated.".into());

    let _ = state.store.save_conversation(&req.user_id, "api", &ctx.messages);

    Ok(Json(SendMessageResponse { reply }))
}

#[derive(Serialize)]
pub struct TodoItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: String,
    pub completed: bool,
    pub due_date: Option<String>,
}

pub async fn list_todos(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<TodoItem>>, StatusCode> {
    match state.store.list_todos() {
        Ok(todos) => Ok(Json(todos.into_iter().map(|t| TodoItem {
            id: t.id,
            title: t.title,
            description: t.description,
            priority: t.priority,
            completed: t.completed,
            due_date: t.due_date,
        }).collect())),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
pub struct AddTodoRequest {
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_priority")]
    pub priority: String,
    #[serde(default)]
    pub due: String,
}

fn default_priority() -> String { "medium".into() }

pub async fn add_todo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<AddTodoRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.store.add_todo(&req.title, &req.description, &req.priority, &req.due) {
        Ok(id) => Ok(Json(serde_json::json!({"id": id}))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn delete_todo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match state.store.delete_todo(&id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Serialize)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
}

pub async fn list_memories(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<MemoryItem>>, StatusCode> {
    match state.store.list_memories() {
        Ok(memories) => Ok(Json(memories.into_iter().map(|m| MemoryItem {
            id: m.id,
            content: m.content,
        }).collect())),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

#[derive(Deserialize)]
pub struct AddMemoryRequest {
    pub content: String,
}

pub async fn add_memory(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(req): Json<AddMemoryRequest>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    match state.store.add_memory(&req.content) {
        Ok(id) => Ok(Json(serde_json::json!({"id": id}))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn delete_memory(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match state.store.delete_memory(&id) {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
