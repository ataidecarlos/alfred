//! Alfred library crate.
//!
//! The binary (`src/main.rs`) is a thin driver on top of this library. Keeping
//! the implementation here makes it usable from integration tests
//! (`tests/*.rs`) and future embedders.

pub mod agent;
pub mod cli;
pub mod config;
pub mod config_watch;
pub mod connectors;
pub mod error;
pub mod jobs;
pub mod memory;
pub mod paths;
pub mod pi;
pub mod prompt;
pub mod scheduler;
pub mod server;
pub mod store;
pub mod types;
pub mod workspace;