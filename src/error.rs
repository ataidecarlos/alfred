use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlfredError {
    #[error("config error: {0}")]
    Config(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("pi error: {0}")]
    Pi(String),

    #[error("failed to spawn Pi binary '{binary}': {message}")]
    PiSpawn { binary: String, message: String },

    #[error("Pi process '{binary}' exited while running (status: {status}); stderr: {stderr}")]
    PiProcessExited { binary: String, status: String, stderr: String },

    #[error("store error: {0}")]
    Store(#[from] rusqlite::Error),

    #[error("job name already exists")]
    JobNameExists,

    #[error("job not found: {0}")]
    JobNotFound(String),

    #[error("minimum watch interval is {0}s")]
    WatchIntervalTooShort(u64),

    #[error("{0}")]
    JobValidation(String),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("tool '{name}' failed: {message}")]
    Tool { name: String, message: String },

    #[error("connector error: {0}")]
    Connector(String),

    #[error("scheduler error: {0}")]
    Scheduler(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
