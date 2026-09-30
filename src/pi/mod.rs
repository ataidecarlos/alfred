//! Pi subprocess boundary.
//!
//! Alfred delegates the agent loop to Pi (https://github.com/earendil-works/pi),
//! run as a subprocess in RPC mode. [`invocation`] builds the exact command line
//! and environment; [`client`] speaks the JSONL protocol documented in
//! `pi/packages/coding-agent/docs/rpc.md`.
//!
//! Framing is strict: records are separated by LF only, a trailing CR is
//! stripped, and U+2028/U+2029 are ordinary characters inside JSON strings.

pub mod client;
pub mod invocation;
pub mod session;

pub use client::{JsonlReader, PiClient, PiResponse};
pub use invocation::PiInvocation;
pub use session::{
    compact_request, run_compaction_ticker, CompactPolicy, CompactionTarget, Session,
    SessionSupervisor, TickOutcome, COMPACT_TICK,
};
