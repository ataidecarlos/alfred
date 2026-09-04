use thiserror::Error;

#[derive(Debug, Error)]
pub enum AlfredError {
    #[error("config error: {0}")]
    Config(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("store error: {0}")]
    Store(#[from] rusqlite::Error),

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
