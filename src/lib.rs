//! Alfred library crate.
//!
//! The binary (`src/main.rs`) is a thin driver on top of this library. Keeping
//! the implementation here makes it usable from integration tests
//! (`tests/*.rs`) and future embedders.

pub mod agent;
pub mod bus;
pub mod config;
pub mod config_watch;
pub mod connectors;
pub mod error;
pub mod llm;
pub mod memory;
pub mod paths;
pub mod prompt;
pub mod scheduler;
pub mod server;
pub mod session;
pub mod store;
pub mod tools;
pub mod tui;
pub mod types;
pub mod workitem;
pub mod workspace;