//! Per-channel session supervisor and idle-based compaction (issue #14).
//!
//! A channel keeps one long-lived `pi --mode rpc` process so a chat has
//! continuity. Continuity is only useful while the context stays bounded, so
//! this module owns two things:
//!
//! 1. [`Session`] — the process lifecycle: spawn lazily, send a prompt, read
//!    the latest token count, ask Pi to compact, and drop (and therefore kill)
//!    the child. [`Session::send`] and [`Session::start`] stay separate so the
//!    supervisor can drive lifecycle without touching the send path.
//! 2. [`SessionSupervisor`] and [`CompactPolicy`] — the compaction *decision*.
//!
//! # Idle-based, not age-based
//!
//! Personal-assistant sessions are sparse: three messages can sit untouched
//! for hours. Compacting on age would throw away such a conversation. The rule
//! is therefore based on *inactivity*: compact once a session has been idle for
//! longer than `idle_compact_secs`, or once its reported token count exceeds
//! `compact_token_threshold`. Compaction never calls `new_session`; that would
//! discard the continuity the user asked for.
//!
//! # Testability
//!
//! The decision is a pure function of `(last_activity, now, tokens)` and the
//! policy, so it is tested with an injected clock instead of a `sleep`. The I/O
//! half is proved separately: [`CompactionTarget`] abstracts the session the
//! supervisor drives, so a spy records the emitted `compact` call, and the
//! exact RPC command is pinned by [`compact_request`].

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::config::PiConfig;
use crate::error::AlfredError;
use crate::pi::client::{PiClient, PiResponse};
use crate::pi::invocation::PiInvocation;

/// The supervisor's tick period: the issue specifies a 60-second ticker.
pub const COMPACT_TICK: Duration = Duration::from_secs(60);

/// The exact command Pi receives to compact the current conversation.
///
/// Kept as a function (rather than inlined at the call site) so a unit test can
/// pin the wire shape independently of the transport.
pub fn compact_request() -> Value {
    json!({"type": "compact"})
}

/// When a session should be compacted.
///
/// Deliberately a plain value: the decision is a pure function of the policy
/// plus `(last_activity, now, tokens)`, so it can be exercised exhaustively
/// with an injected clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompactPolicy {
    /// Compact after this many seconds without a message.
    pub idle_compact_secs: u64,
    /// Compact once the conversation exceeds this many tokens.
    pub compact_token_threshold: u64,
}

impl CompactPolicy {
    pub fn new(idle_compact_secs: u64, compact_token_threshold: u64) -> Self {
        Self { idle_compact_secs, compact_token_threshold }
    }

    /// Should the session be compacted?
    ///
    /// True when either trigger fires: the session has been idle for strictly
    /// more than `idle_compact_secs`, or its token count strictly exceeds
    /// `compact_token_threshold`. A `now` before `last_activity` (a clock
    /// going backwards) is treated as no idle time.
    pub fn should_compact(&self, last_activity: Instant, now: Instant, tokens: u64) -> bool {
        if tokens > self.compact_token_threshold {
            return true;
        }
        now.saturating_duration_since(last_activity) > Duration::from_secs(self.idle_compact_secs)
    }
}

impl From<&PiConfig> for CompactPolicy {
    fn from(config: &PiConfig) -> Self {
        Self::new(config.idle_compact_secs, config.compact_token_threshold)
    }
}

impl Default for CompactPolicy {
    fn default() -> Self {
        Self::from(&PiConfig::default())
    }
}

/// The session operations the supervisor needs to make and act on its decision.
///
/// Production is [`Session`]; tests inject a spy, so the supervisor's wiring is
/// proved without a subprocess.
#[async_trait]
pub trait CompactionTarget: Send {
    /// Whether a process is running for this session.
    fn is_alive(&self) -> bool;
    /// When this session last saw activity (a message or a compaction).
    fn last_activity(&self) -> Instant;
    /// The last token count observed from `get_session_stats`.
    fn cached_tokens(&self) -> u64;
    /// Record that the process is gone; the next message will restart it.
    fn mark_dead(&mut self);
    /// Reset the idle baseline after a successful compaction, so the next
    /// trigger is measured from `now` rather than compacting every tick.
    fn note_compacted(&mut self, now: Instant);
    /// Whether Pi is currently streaming a response.
    async fn is_streaming(&mut self) -> Result<bool, AlfredError>;
    /// The latest token count from `get_session_stats`.
    async fn token_count(&mut self) -> Result<u64, AlfredError>;
    /// Ask Pi to compact the conversation.
    async fn compact(&mut self) -> Result<(), AlfredError>;
}

/// What one supervisor pass did (or deliberately did not do).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickOutcome {
    /// No process is running; nothing to compact.
    NotRunning,
    /// Pi is streaming; compaction is skipped to the next tick.
    SkippedWhileStreaming,
    /// The liveness probe failed; the process was marked dead.
    Died,
    /// The policy says no compaction is due yet.
    NotDue { tokens: u64 },
    /// A `compact` command was sent and accepted.
    Compacted { tokens: u64 },
    /// The `compact` command failed; it will be retried on the next tick.
    CompactFailed,
}

/// Decides whether one session should compact and, if so, drives the target.
pub struct SessionSupervisor<'a, T: CompactionTarget> {
    target: &'a mut T,
    policy: CompactPolicy,
}

impl<'a, T: CompactionTarget> SessionSupervisor<'a, T> {
    pub fn new(target: &'a mut T, policy: CompactPolicy) -> Self {
        Self { target, policy }
    }

    /// Run one supervisor pass for this session at wall-clock `now`.
    pub async fn tick(&mut self, now: Instant) -> TickOutcome {
        if !self.target.is_alive() {
            return TickOutcome::NotRunning;
        }

        // A compact request while Pi is streaming is skipped to the next tick,
        // never interleaved with an in-flight turn.
        match self.target.is_streaming().await {
            Ok(true) => return TickOutcome::SkippedWhileStreaming,
            Ok(false) => {}
            Err(error) => {
                warn!(%error, "channel: liveness probe failed; marking session dead");
                self.target.mark_dead();
                return TickOutcome::Died;
            }
        }

        let tokens = match self.target.token_count().await {
            Ok(tokens) => tokens,
            Err(error) => {
                warn!(%error, "channel: get_session_stats failed; using the cached token count");
                self.target.cached_tokens()
            }
        };

        if !self.policy.should_compact(self.target.last_activity(), now, tokens) {
            return TickOutcome::NotDue { tokens };
        }

        match self.target.compact().await {
            Ok(()) => {
                // Compaction is not user activity, but it resets the trigger:
                // without this the idle rule would fire on every tick.
                self.target.note_compacted(now);
                info!(tokens, "channel: compacted session");
                TickOutcome::Compacted { tokens }
            }
            Err(error) => {
                warn!(%error, "channel: compact failed; will retry on the next tick");
                TickOutcome::CompactFailed
            }
        }
    }
}

/// One supervisor pass over every channel session.
pub async fn tick_all(
    sessions: &Arc<Mutex<HashMap<String, Session>>>,
    policy: CompactPolicy,
) {
    let now = Instant::now();
    let mut guard = sessions.lock().await;
    for (channel, session) in guard.iter_mut() {
        let outcome = SessionSupervisor::new(session, policy).tick(now).await;
        match outcome {
            TickOutcome::Compacted { tokens } => {
                info!(channel = %channel, tokens, "channel: compacted session");
            }
            TickOutcome::CompactFailed => {
                warn!(channel = %channel, "channel: compact failed; retrying next tick");
            }
            TickOutcome::Died => {
                warn!(channel = %channel, "channel: session marked dead; it restarts on the next message");
            }
            TickOutcome::NotRunning
            | TickOutcome::SkippedWhileStreaming
            | TickOutcome::NotDue { .. } => {}
        }
    }
}

/// Run the compaction ticker until the runtime shuts down.
///
/// Every `tick` it takes the sessions lock and gives each channel one
/// supervisor pass. Holding the lock serialises compaction against message
/// handling, which is what keeps a compact from interleaving with a turn.
pub async fn run_compaction_ticker(
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    policy: CompactPolicy,
    tick: Duration,
) {
    let mut interval = tokio::time::interval(tick);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // The first tick fires immediately; a freshly started server has nothing to
    // compact, so consume it and measure the cadence from the interval.
    interval.tick().await;
    loop {
        interval.tick().await;
        tick_all(&sessions, policy).await;
    }
}

/// One channel's long-lived Pi RPC session.
///
/// `start` spawns the subprocess and `send` submits a prompt and streams the
/// reply. Keeping them separate lets the supervisor restart, compact, and shut
/// down sessions without changing the send path.
pub struct Session {
    invocation: PiInvocation,
    client: Option<PiClient>,
    dead: bool,
    last_activity: Instant,
    tokens: u64,
}

impl Session {
    pub fn new(invocation: PiInvocation) -> Self {
        Self { invocation, client: None, dead: false, last_activity: Instant::now(), tokens: 0 }
    }

    /// A session is alive once its process is running and has not failed.
    pub fn is_alive(&self) -> bool {
        self.client.is_some() && !self.dead
    }

    /// Spawn the Pi subprocess for this channel.
    pub async fn start(&mut self) -> Result<(), AlfredError> {
        let client =
            PiClient::spawn(&self.invocation.binary, self.invocation.command()).await?;
        self.client = Some(client);
        self.dead = false;
        Ok(())
    }

    /// Mark the process dead and drop it, so the next message restarts it. The
    /// dropped [`PiClient`] kills its child through `kill_on_drop`.
    pub fn mark_dead(&mut self) {
        self.dead = true;
        self.client = None;
    }

    /// Send `prompt` and return the assistant text streamed back before the
    /// agent settled. Recorded as activity so idle-based compaction restarts.
    pub async fn send(&mut self, prompt: &str) -> Result<String, AlfredError> {
        let response = self
            .request(json!({"type": "prompt", "message": prompt}))
            .await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "prompt rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }

        let mut reply = String::new();
        loop {
            let message = self.client_mut()?.next_message().await?;
            match message.get("type").and_then(Value::as_str) {
                Some("message_update") => {
                    if let Some(delta) = delta_text(&message) {
                        reply.push_str(delta);
                    }
                }
                Some("agent_settled") => break,
                _ => {}
            }
        }

        // A reply can arrive without a text delta (for example, only a final
        // message); fall back to Pi's last assistant text in that case.
        if reply.trim().is_empty() {
            let fallback = self
                .request(json!({"type": "get_last_assistant_text"}))
                .await?;
            if fallback.success {
                if let Some(text) = fallback
                    .data
                    .as_ref()
                    .and_then(|data| data.get("text"))
                    .and_then(Value::as_str)
                {
                    reply = text.to_string();
                }
            }
        }

        self.last_activity = Instant::now();
        Ok(reply)
    }

    /// Clear the conversation by sending `new_session`. This is a user action,
    /// never the compaction path.
    pub async fn new_session(&mut self) -> Result<(), AlfredError> {
        let response = self.request(json!({"type": "new_session"})).await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "new_session rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        self.last_activity = Instant::now();
        Ok(())
    }

    /// Ask Pi to compact the conversation. Continuity is preserved: the process
    /// and its session file are untouched.
    pub async fn compact(&mut self) -> Result<(), AlfredError> {
        let response = self.request(compact_request()).await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "compact rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        Ok(())
    }

    /// Whether Pi is currently streaming a response (`get_state`).
    pub async fn is_streaming(&mut self) -> Result<bool, AlfredError> {
        let response = self.request(json!({"type": "get_state"})).await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "get_state rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        Ok(response
            .data
            .as_ref()
            .and_then(|data| data.get("isStreaming"))
            .and_then(Value::as_bool)
            .unwrap_or(false))
    }

    /// Read the latest total token count from `get_session_stats` and cache it.
    pub async fn token_count(&mut self) -> Result<u64, AlfredError> {
        let response = self.request(json!({"type": "get_session_stats"})).await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "get_session_stats rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        let tokens = response
            .data
            .as_ref()
            .and_then(|data| data.get("tokens"))
            .and_then(|tokens| tokens.get("total"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        self.tokens = tokens;
        Ok(tokens)
    }

    /// Send one command and await its response.
    async fn request(&mut self, command: Value) -> Result<PiResponse, AlfredError> {
        self.client_mut()?.request(command).await
    }

    fn client_mut(&mut self) -> Result<&mut PiClient, AlfredError> {
        self.client
            .as_mut()
            .ok_or_else(|| AlfredError::Pi("channel session is not started".to_string()))
    }
}

#[async_trait]
impl CompactionTarget for Session {
    fn is_alive(&self) -> bool {
        Session::is_alive(self)
    }

    fn last_activity(&self) -> Instant {
        self.last_activity
    }

    fn cached_tokens(&self) -> u64 {
        self.tokens
    }

    fn mark_dead(&mut self) {
        Session::mark_dead(self);
    }

    fn note_compacted(&mut self, now: Instant) {
        self.last_activity = now;
        self.tokens = 0;
    }

    async fn is_streaming(&mut self) -> Result<bool, AlfredError> {
        Session::is_streaming(self).await
    }

    async fn token_count(&mut self) -> Result<u64, AlfredError> {
        Session::token_count(self).await
    }

    async fn compact(&mut self) -> Result<(), AlfredError> {
        Session::compact(self).await
    }
}

/// The text delta carried by a `message_update` event, if any.
fn delta_text(message: &Value) -> Option<&str> {
    let event = message.get("assistantMessageEvent")?;
    if event.get("type").and_then(Value::as_str) != Some("text_delta") {
        return None;
    }
    event.get("delta").and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ------------------------------------------------------------- policy

    #[test]
    fn policy_compacts_only_after_strictly_more_than_the_idle_window() {
        let policy = CompactPolicy::new(60, 60_000);
        let t0 = Instant::now();

        // At exactly the window (and a fresh session) the answer is no.
        assert!(!policy.should_compact(t0, t0, 15));
        assert!(!policy.should_compact(t0, t0 + Duration::from_secs(60), 15));
        // One second past the window it compacts.
        assert!(policy.should_compact(t0, t0 + Duration::from_secs(61), 15));
        // The issue's acceptance scenario: 90 s idle compacts.
        assert!(policy.should_compact(t0, t0 + Duration::from_secs(90), 15));
    }

    #[test]
    fn policy_does_not_compact_a_sparse_three_message_session() {
        // The default personal-assistant policy is "half a day", which is the
        // whole point of idle-based over age-based: a session untouched for a
        // minute must survive.
        let policy = CompactPolicy::new(43_200, 60_000);
        let t0 = Instant::now();
        assert!(!policy.should_compact(t0, t0 + Duration::from_secs(60), 3));
        assert!(!policy.should_compact(t0, t0 + Duration::from_secs(3600), 3));
        assert!(policy.should_compact(t0, t0 + Duration::from_secs(43_201), 3));
    }

    #[test]
    fn policy_compacts_on_token_pressure_while_fresh() {
        let policy = CompactPolicy::new(43_200, 60_000);
        let t0 = Instant::now();
        // Fresh session, oversized context: compact.
        assert!(policy.should_compact(t0, t0, 60_001));
        // Exactly at the threshold does not "exceed" it.
        assert!(!policy.should_compact(t0, t0, 60_000));
        assert!(!policy.should_compact(t0, t0 + Duration::from_secs(30), 0));
    }

    #[test]
    fn policy_treats_a_backwards_clock_as_zero_idle() {
        let policy = CompactPolicy::new(60, 60_000);
        let t0 = Instant::now();
        // No panic and no spurious compaction.
        assert!(!policy.should_compact(t0, t0 - Duration::from_secs(5), 0));
    }

    #[test]
    fn delta_text_extracts_only_text_deltas() {
        let delta = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "text_delta", "delta": "hi" },
        });
        assert_eq!(delta_text(&delta), Some("hi"));

        let other = json!({
            "type": "message_update",
            "assistantMessageEvent": { "type": "tool_call", "delta": "hi" },
        });
        assert_eq!(delta_text(&other), None);
        assert_eq!(delta_text(&json!({"type": "agent_settled"})), None);
    }

    #[test]
    fn compact_request_is_the_rpc_compact_command() {
        assert_eq!(compact_request(), json!({"type": "compact"}));
        // It must never be `new_session`: that would discard continuity.
        assert_ne!(compact_request().get("type"), Some(&json!("new_session")));
    }

    // ------------------------------------------------------------- supervisor

    /// A spy `CompactionTarget` that records what the supervisor did.
    struct SpyTarget {
        alive: bool,
        streaming: bool,
        streaming_fails: bool,
        tokens: u64,
        stats_fails: bool,
        compact_fails: bool,
        last_activity: Instant,
        compact_calls: usize,
        marked_dead: usize,
        noted: usize,
    }

    impl SpyTarget {
        fn new(last_activity: Instant) -> Self {
            Self {
                alive: true,
                streaming: false,
                streaming_fails: false,
                tokens: 0,
                stats_fails: false,
                compact_fails: false,
                last_activity,
                compact_calls: 0,
                marked_dead: 0,
                noted: 0,
            }
        }
    }

    #[async_trait]
    impl CompactionTarget for SpyTarget {
        fn is_alive(&self) -> bool {
            self.alive
        }

        fn last_activity(&self) -> Instant {
            self.last_activity
        }

        fn cached_tokens(&self) -> u64 {
            self.tokens
        }

        fn mark_dead(&mut self) {
            self.alive = false;
            self.marked_dead += 1;
        }

        fn note_compacted(&mut self, now: Instant) {
            self.last_activity = now;
            self.tokens = 0;
            self.noted += 1;
        }

        async fn is_streaming(&mut self) -> Result<bool, AlfredError> {
            if self.streaming_fails {
                return Err(AlfredError::Pi("liveness probe failed".into()));
            }
            Ok(self.streaming)
        }

        async fn token_count(&mut self) -> Result<u64, AlfredError> {
            if self.stats_fails {
                return Err(AlfredError::Pi("stats failed".into()));
            }
            Ok(self.tokens)
        }

        async fn compact(&mut self) -> Result<(), AlfredError> {
            self.compact_calls += 1;
            if self.compact_fails {
                return Err(AlfredError::Pi("compact failed".into()));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn supervisor_emits_compact_when_a_session_is_idle() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.tokens = 15;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(60, 60_000));

        let outcome = supervisor.tick(t0 + Duration::from_secs(90)).await;

        assert_eq!(outcome, TickOutcome::Compacted { tokens: 15 });
        assert_eq!(spy.compact_calls, 1, "the compact command must be emitted");
        assert_eq!(spy.noted, 1, "the idle baseline must reset after compaction");
    }

    #[tokio::test]
    async fn supervisor_emits_compact_on_token_pressure() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.tokens = 60_001;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(43_200, 60_000));

        let outcome = supervisor.tick(t0).await;

        assert_eq!(outcome, TickOutcome::Compacted { tokens: 60_001 });
        assert_eq!(spy.compact_calls, 1);
    }

    #[tokio::test]
    async fn supervisor_does_not_compact_a_fresh_sparse_session() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.tokens = 3;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(43_200, 60_000));

        let outcome = supervisor.tick(t0 + Duration::from_secs(60)).await;

        assert_eq!(outcome, TickOutcome::NotDue { tokens: 3 });
        assert_eq!(spy.compact_calls, 0);
    }

    #[tokio::test]
    async fn supervisor_skips_compaction_while_streaming() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.streaming = true;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(60, 60_000));

        let outcome = supervisor.tick(t0 + Duration::from_secs(90)).await;

        assert_eq!(outcome, TickOutcome::SkippedWhileStreaming);
        assert_eq!(spy.compact_calls, 0, "a streaming session must not be compacted");
    }

    #[tokio::test]
    async fn supervisor_retries_a_failed_compact() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.tokens = 15;
        spy.compact_fails = true;
        let policy = CompactPolicy::new(60, 60_000);

        {
            let mut supervisor = SessionSupervisor::new(&mut spy, policy);
            let outcome = supervisor.tick(t0 + Duration::from_secs(90)).await;
            assert_eq!(outcome, TickOutcome::CompactFailed);
            assert_eq!(spy.compact_calls, 1);
            assert_eq!(spy.noted, 0, "a failed compact must not reset the idle baseline");
        }

        // Next tick: the failure has cleared and the still-idle session retries.
        spy.compact_fails = false;
        let mut supervisor = SessionSupervisor::new(&mut spy, policy);
        let outcome = supervisor.tick(t0 + Duration::from_secs(120)).await;
        assert_eq!(outcome, TickOutcome::Compacted { tokens: 15 });
        assert_eq!(spy.compact_calls, 2, "the failed compact must be retried");
    }

    #[tokio::test]
    async fn supervisor_marks_a_dead_process_and_leaves_restart_to_the_caller() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.streaming_fails = true;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(60, 60_000));

        let outcome = supervisor.tick(t0 + Duration::from_secs(90)).await;

        assert_eq!(outcome, TickOutcome::Died);
        assert_eq!(spy.marked_dead, 1);
        assert_eq!(spy.compact_calls, 0);
    }

    #[tokio::test]
    async fn supervisor_ignores_a_session_that_is_not_running() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.alive = false;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(60, 60_000));

        let outcome = supervisor.tick(t0 + Duration::from_secs(90)).await;

        assert_eq!(outcome, TickOutcome::NotRunning);
        assert_eq!(spy.compact_calls, 0);
    }

    #[tokio::test]
    async fn supervisor_falls_back_to_cached_tokens_when_stats_fail() {
        let t0 = Instant::now();
        let mut spy = SpyTarget::new(t0);
        spy.tokens = 60_001;
        spy.stats_fails = true;
        let mut supervisor = SessionSupervisor::new(&mut spy, CompactPolicy::new(43_200, 60_000));

        // Stats are unavailable, but the cached count still trips the token rule.
        let outcome = supervisor.tick(t0).await;
        assert_eq!(outcome, TickOutcome::Compacted { tokens: 60_001 });
        assert_eq!(spy.compact_calls, 1);
    }
}
