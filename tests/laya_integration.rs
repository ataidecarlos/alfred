//! Integration tests for the Laya decision layer.
//!
//! Verification target for the "Integrate Laya model for high-confidence
//! action decisions" work item: `cargo test --test laya_integration`.
//!
//! These tests exercise the real request path: high-confidence requests are
//! executed directly without an LLM call, everything else is delegated to the
//! configured provider, and the chosen decision path is logged.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use alfred::agent::tool::ToolRegistry;
use alfred::agent::{run_agent_loop, AgentLoop, AgentLoopContext, AgentRunner};
use alfred::bus::InboundMessage;
use alfred::error::AlfredError;
use alfred::laya::{DecisionPath, LayaModel, DEFAULT_CONFIDENCE_THRESHOLD, LAYA_MODEL_NAME};
use alfred::llm::{LlmProvider, LlmRequest, LlmStream, LlmStreamEvent};
use alfred::session::SessionManager;
use alfred::store::Store;
use alfred::types::{
    extract_text, Content, Message, StopReason, TextContent, Usage, UserMessage,
};

/// Provider that returns a fixed reply and counts how often it was called.
struct CountingProvider {
    reply: String,
    calls: AtomicU32,
}

impl CountingProvider {
    fn new(reply: &str) -> Arc<Self> {
        Arc::new(Self {
            reply: reply.to_string(),
            calls: AtomicU32::new(0),
        })
    }

    fn calls(&self) -> u32 {
        self.calls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl LlmProvider for CountingProvider {
    async fn stream(&self, _request: LlmRequest) -> Result<LlmStream, AlfredError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        let reply = self.reply.clone();
        let reply2 = reply.clone();
        Ok(Box::pin(async_stream::stream! {
            yield LlmStreamEvent::Start;
            yield LlmStreamEvent::TextDelta(reply);
            yield LlmStreamEvent::Done {
                text: reply2,
                tool_calls: vec![],
                stop_reason: StopReason::Stop,
                usage: Usage { input: 1, output: 1, total: 2 },
            };
        }))
    }
}

fn user_message(text: &str) -> Message {
    Message::User(UserMessage {
        content: vec![Content::Text(TextContent { text: text.into() })],
        timestamp: chrono::Utc::now(),
    })
}

fn last_reply(messages: &[Message]) -> String {
    messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant(_) => {
                let text = extract_text(m);
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
            _ => None,
        })
        .unwrap_or_default()
}

fn context(
    provider: Arc<dyn LlmProvider>,
    messages: Vec<Message>,
    laya: LayaModel,
) -> AgentLoopContext {
    let (event_tx, _rx) = broadcast::channel(16);
    AgentLoopContext {
        system_prompt: "system".into(),
        messages,
        provider,
        model: "test-model".into(),
        tools: Arc::new(ToolRegistry::new()),
        event_tx,
        max_turns: 3,
        laya,
    }
}

fn make_agent_loop(provider: Arc<CountingProvider>) -> AgentLoop {
    let runner = AgentRunner {
        provider,
        model: "test-model".into(),
        tools: Arc::new(ToolRegistry::new()),
        max_turns: 3,
    };

    let conn = rusqlite::Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workspaces (
            id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT 'default',
            path TEXT NOT NULL, created_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL DEFAULT 'default',
            user_id TEXT NOT NULL, channel TEXT NOT NULL, title TEXT,
            created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS messages (
            id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
            role TEXT NOT NULL, content TEXT NOT NULL, timestamp INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS conversations (
            id TEXT PRIMARY KEY, user_id TEXT NOT NULL, connector TEXT NOT NULL,
            messages TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
        );",
    )
    .unwrap();

    let store = Arc::new(Store::from_connection(conn));
    let session_manager = Arc::new(SessionManager::new(store));
    let (event_tx, _rx) = broadcast::channel(16);

    AgentLoop::new(runner, session_manager, "system".into(), event_tx)
}

// ── Confidence threshold policy ─────────────────────────────────────

#[test]
fn threshold_boundary_is_exclusive() {
    let model = LayaModel::default();
    assert_eq!(
        model.path_for_confidence(DEFAULT_CONFIDENCE_THRESHOLD),
        DecisionPath::DelegateToLlm
    );
    assert_eq!(model.path_for_confidence(0.79), DecisionPath::DelegateToLlm);
    assert_eq!(model.path_for_confidence(0.81), DecisionPath::ExecuteDirect);
}

#[test]
fn fast_path_request_is_high_confidence() {
    let model = LayaModel::default();
    let decision = model.decide("ping");
    assert_eq!(decision.path, DecisionPath::ExecuteDirect);
    assert_eq!(decision.intent.as_deref(), Some("ping"));
    assert!(decision.confidence > DEFAULT_CONFIDENCE_THRESHOLD);
}

#[test]
fn complex_request_is_low_confidence() {
    let model = LayaModel::default();
    let decision = model.decide("Explain why the sky is blue and compare it with a sunset");
    assert_eq!(decision.path, DecisionPath::DelegateToLlm);
    assert!(decision.confidence <= DEFAULT_CONFIDENCE_THRESHOLD);
}

#[test]
fn empty_request_delegates_to_llm() {
    let model = LayaModel::default();
    let decision = model.decide("   ");
    assert_eq!(decision.path, DecisionPath::DelegateToLlm);
    assert_eq!(decision.confidence, 0.0);
}

#[test]
fn decisions_are_logged_in_order() {
    let model = LayaModel::default();
    model.decide("ping");
    model.decide("Explain quantum entanglement");
    let logged = model.decisions();
    assert_eq!(logged.len(), 2);
    assert_eq!(logged[0].path, DecisionPath::ExecuteDirect);
    assert_eq!(logged[1].path, DecisionPath::DelegateToLlm);
}

// ── Integration with the real request path ──────────────────────────

#[tokio::test]
async fn high_confidence_request_executes_without_llm() {
    let provider = CountingProvider::new("should not be used");
    let mut ctx = context(
        provider.clone(),
        vec![user_message("ping")],
        LayaModel::default(),
    );

    run_agent_loop(&mut ctx).await;

    assert_eq!(last_reply(&ctx.messages), "pong");
    assert_eq!(provider.calls(), 0, "direct path must not call the LLM");

    let logged = ctx.laya.decisions();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].path, DecisionPath::ExecuteDirect);
}

#[tokio::test]
async fn low_confidence_request_delegates_to_llm() {
    let provider = CountingProvider::new("delegated reply");
    let mut ctx = context(
        provider.clone(),
        vec![user_message("Explain why the sky is blue in detail")],
        LayaModel::default(),
    );

    run_agent_loop(&mut ctx).await;

    assert_eq!(provider.calls(), 1, "low-confidence path must call the LLM");
    assert_eq!(last_reply(&ctx.messages), "delegated reply");

    let logged = ctx.laya.decisions();
    assert_eq!(logged.len(), 1);
    assert_eq!(logged[0].path, DecisionPath::DelegateToLlm);
}

#[tokio::test]
async fn strict_threshold_routes_fast_path_to_llm() {
    let provider = CountingProvider::new("delegated reply");
    let mut ctx = context(
        provider.clone(),
        vec![user_message("ping")],
        LayaModel::new(0.99),
    );

    run_agent_loop(&mut ctx).await;

    // "ping" scores 0.95, below the 0.99 threshold, so it delegates.
    assert_eq!(provider.calls(), 1);
    assert_eq!(last_reply(&ctx.messages), "delegated reply");
    assert_eq!(ctx.laya.decisions()[0].path, DecisionPath::DelegateToLlm);
}

#[tokio::test]
async fn direct_reply_is_tagged_with_laya_model() {
    let provider = CountingProvider::new("unused");
    let mut ctx = context(
        provider.clone(),
        vec![user_message("version")],
        LayaModel::default(),
    );

    run_agent_loop(&mut ctx).await;

    let assistant = ctx
        .messages
        .iter()
        .rev()
        .find_map(|m| match m {
            Message::Assistant(a) => Some(a),
            _ => None,
        })
        .expect("expected an assistant message");
    assert_eq!(assistant.model, LAYA_MODEL_NAME);
    assert!(extract_text(&Message::Assistant(assistant.clone()))
        .contains(env!("CARGO_PKG_VERSION")));
}

#[tokio::test]
async fn channel_loop_executes_direct_requests_without_llm() {
    let provider = CountingProvider::new("should not be used");
    let agent = make_agent_loop(provider.clone());

    let out = agent
        .handle(InboundMessage {
            user_id: "u1".into(),
            channel: "api".into(),
            session_key: "u1:api".into(),
            text: "ping".into(),
        })
        .await;

    assert_eq!(out.text, "pong");
    assert_eq!(provider.calls(), 0);
}

#[tokio::test]
async fn channel_loop_delegates_low_confidence_requests() {
    let provider = CountingProvider::new("llm reply");
    let agent = make_agent_loop(provider.clone());

    let out = agent
        .handle(InboundMessage {
            user_id: "u1".into(),
            channel: "api".into(),
            session_key: "u1:api".into(),
            text: "Explain the difference between TCP and UDP".into(),
        })
        .await;

    assert_eq!(provider.calls(), 1);
    assert_eq!(out.text, "llm reply");
}
