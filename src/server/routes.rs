use axum::extract::{Path, State, Json};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::extract::FromRequestParts;
use serde::{Deserialize, Serialize};

use crate::server::AppState;

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

#[derive(Deserialize)]
pub struct UpdateTodoRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
}

pub async fn update_todo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateTodoRequest>,
) -> Result<StatusCode, StatusCode> {
    match state.store.update_todo(
        &id,
        req.title.as_deref(),
        req.description.as_deref(),
        req.priority.as_deref(),
        req.due.as_deref(),
    ) {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

pub async fn complete_todo(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, StatusCode> {
    match state.store.complete_todo(&id) {
        Ok(true) => Ok(StatusCode::NO_CONTENT),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
