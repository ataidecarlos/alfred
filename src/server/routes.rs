use axum::extract::{Path, State, Json};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use axum::extract::FromRequestParts;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::AlfredError;
use crate::jobs::dispatch::JobDispatch;
use crate::jobs::{Job, JobKind, JobRun, NewJob, ReportPolicy};
use crate::memory;
use crate::scheduler::{run_to_completion, Dispatch};
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

/// A REST failure: an HTTP status plus a plain-text message body.
///
/// A malformed cron or a below-floor watch interval is a `400` carrying the
/// validation message, an unknown job is a `404`, and every other failure is a
/// `500` carrying the error text.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into() }
    }
}

impl From<AlfredError> for ApiError {
    fn from(error: AlfredError) -> Self {
        match error {
            AlfredError::JobNotFound(id) => {
                ApiError::new(StatusCode::NOT_FOUND, format!("job not found: {id}"))
            }
            AlfredError::JobNameExists => {
                ApiError::new(StatusCode::CONFLICT, "job name already exists")
            }
            AlfredError::JobValidation(message) => ApiError::new(StatusCode::BAD_REQUEST, message),
            AlfredError::WatchIntervalTooShort(secs) => ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("minimum watch interval is {secs}s"),
            ),
            other => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, self.message).into_response()
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
    /// The probed Pi version, or `null` when Pi is not on PATH.
    pub pi_version: Option<String>,
    /// Number of jobs currently enabled.
    pub jobs_enabled: usize,
}

pub async fn server_info(State(state): State<AppState>) -> Result<Json<ServerInfo>, ApiError> {
    let jobs_enabled = state
        .store
        .list_jobs()?
        .into_iter()
        .filter(|job| job.enabled)
        .count();

    Ok(Json(ServerInfo {
        pid: std::process::id(),
        port: state.port,
        uptime_secs: state.start_time.elapsed().as_secs(),
        active_connections: state.active_connections.load(std::sync::atomic::Ordering::Relaxed),
        pi_version: state.pi_version.clone(),
        jobs_enabled,
    }))
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

// ---------------------------------------------------------------- jobs

/// The JSON body accepted when creating or replacing a job.
///
/// Optional fields fall back to the same defaults as [`NewJob::default`], so
/// the minimal body `{"name","kind","prompt",...}` is valid.
#[derive(Deserialize)]
pub struct JobBody {
    pub name: String,
    pub kind: JobKind,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default)]
    pub run_at: Option<i64>,
    pub prompt: String,
    #[serde(default)]
    pub report: Option<ReportPolicy>,
    #[serde(default)]
    pub deliver_to: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default = "default_job_timeout_secs")]
    pub timeout_secs: u64,
}

fn default_job_timeout_secs() -> u64 {
    NewJob::default().timeout_secs
}

impl From<JobBody> for NewJob {
    fn from(body: JobBody) -> Self {
        NewJob {
            name: body.name,
            kind: body.kind,
            schedule: body.schedule,
            run_at: body.run_at,
            prompt: body.prompt,
            report: body.report.unwrap_or(ReportPolicy::OnSignal),
            deliver_to: body.deliver_to,
            model: body.model,
            tools: body.tools,
            timeout_secs: body.timeout_secs,
        }
    }
}

/// The configured watch floor, from the injected `[jobs]` config.
fn min_watch_interval_secs(state: &AppState) -> u64 {
    state.jobs.min_watch_interval_secs
}

pub async fn list_jobs(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<Vec<Job>>, ApiError> {
    Ok(Json(state.store.list_jobs()?))
}

pub async fn create_job(
    State(state): State<AppState>,
    _auth: AuthUser,
    Json(body): Json<JobBody>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let job = state.store.add_job(&body.into(), min_watch_interval_secs(&state))?;
    Ok((StatusCode::CREATED, Json(job)))
}

pub async fn get_job(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(state.store.get_job(&id)?))
}

pub async fn update_job(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<JobBody>,
) -> Result<Json<Job>, ApiError> {
    Ok(Json(
        state.store.update_job(&id, &body.into(), min_watch_interval_secs(&state))?,
    ))
}

pub async fn delete_job(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.store.delete_job(&id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Request a manual run of a job.
///
/// The job is executed synchronously through the same composed
/// [`JobDispatch`] the scheduler installs: the run row is opened, closed, and
/// delivered to exactly as a scheduled run would be. An unknown id is a `404`;
/// a known job always returns `200` carrying the recorded run, whose `status`
/// distinguishes a successful run from a failed one. A run whose `status` is
/// `failed` or `timeout` is a result, not a transport failure.
pub async fn run_job(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<JobRun>, ApiError> {
    let job = state.store.get_job(&id)?;
    let dispatch: Arc<dyn Dispatch> = Arc::new(JobDispatch::from_config(
        Arc::clone(&state.store),
        &state.pi,
        &state.jobs,
        state.telegram.as_ref(),
    ));
    let run = run_to_completion(&state.store, &dispatch, state.jobs.max_runs_per_job, &job).await?;
    Ok(Json(run))
}

pub async fn list_job_runs(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<JobRun>>, ApiError> {
    let limit = state.jobs.max_runs_per_job;
    Ok(Json(state.store.runs_for(&id, limit)?))
}

// ------------------------------------------------------------ memories

#[derive(Serialize)]
pub struct MemoryItem {
    pub slug: String,
    pub text: String,
}

pub async fn list_memories(_auth: AuthUser) -> Json<Vec<MemoryItem>> {
    Json(
        memory::list_memories()
            .into_iter()
            .map(|(slug, text)| MemoryItem { slug, text })
            .collect(),
    )
}

#[derive(Deserialize)]
pub struct AddMemoryRequest {
    #[serde(alias = "content")]
    pub text: String,
}

pub async fn add_memory(
    _auth: AuthUser,
    Json(req): Json<AddMemoryRequest>,
) -> Result<StatusCode, ApiError> {
    memory::append_memory(&req.text)?;
    Ok(StatusCode::CREATED)
}

pub async fn delete_memory(
    _auth: AuthUser,
    Path(slug): Path<String>,
) -> Result<StatusCode, ApiError> {
    memory::delete_memory(&slug)?;
    Ok(StatusCode::NO_CONTENT)
}
