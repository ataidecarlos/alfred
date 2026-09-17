pub mod event;
pub mod runner;
pub mod tool;

// Re-exports
pub use runner::AgentRunner;

use std::sync::Arc;

use tokio::sync::broadcast;
use tracing::info;

use crate::bus::{InboundMessage, OutboundMessage};
use crate::session::{SessionKey, SessionManager};
use crate::agent::event::AgentEvent;
use crate::agent::tool::ToolRegistry;
use crate::llm::LlmProvider;
use crate::types::{Message, UserMessage, Content, TextContent, StopReason, Usage};

/// Channel-facing agent loop. Receives inbound messages, resolves sessions,
/// builds context, calls the runner, saves results, and publishes outbound messages.
pub struct AgentLoop {
    runner: AgentRunner,
    session_manager: Arc<SessionManager>,
    system_prompt: String,
    event_tx: broadcast::Sender<AgentEvent>,
}

impl AgentLoop {
    pub fn new(
        runner: AgentRunner,
        session_manager: Arc<SessionManager>,
        system_prompt: String,
        event_tx: broadcast::Sender<AgentEvent>,
    ) -> Self {
        Self { runner, session_manager, system_prompt, event_tx }
    }

    /// Handle an inbound message: resolve session, run agent, save, return reply.
    pub async fn handle(&self, inbound: InboundMessage) -> OutboundMessage {
        let key = SessionKey::new(&inbound.user_id, &inbound.channel);

        // Resolve or create session
        let mut session = match self.session_manager.resolve_or_create(&key) {
            Ok(s) => s,
            Err(e) => {
                tracing::error!("Failed to resolve session: {}", e);
                return OutboundMessage {
                    session_key: inbound.session_key,
                    channel: inbound.channel,
                    text: format!("Error: {}", e),
                };
            }
        };

        // Append user message
        let user_msg = Message::User(UserMessage {
            content: vec![Content::Text(TextContent { text: inbound.text })],
            timestamp: chrono::Utc::now(),
        });
        session.messages.push(user_msg);

        // Run the agent
        let _ = self.event_tx.send(AgentEvent::AgentStart);
        let result = self.runner.run(&self.system_prompt, session.messages).await;
        let _ = self.event_tx.send(AgentEvent::AgentEnd {
            messages: result.messages.clone(),
        });

        // Update session with new messages
        session.messages = result.messages;

        // Save session
        if let Err(e) = self.session_manager.save(&session) {
            tracing::error!("Failed to save session: {}", e);
        }

        // Extract reply text
        let reply = session.messages.iter().rev().find_map(|m| {
            let text = crate::types::extract_text(m);
            if !text.is_empty() {
                Some(text)
            } else {
                None
            }
        }).unwrap_or_else(|| "No response generated.".into());

        OutboundMessage {
            session_key: inbound.session_key,
            channel: inbound.channel,
            text: reply,
        }
    }
}

/// Legacy context for backward compatibility. New code should use AgentLoop instead.
pub struct AgentLoopContext {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
    pub event_tx: broadcast::Sender<AgentEvent>,
    pub max_turns: u32,
}

/// Legacy function for backward compatibility. New code should use AgentLoop.
pub async fn run_agent_loop(ctx: &mut AgentLoopContext) -> Vec<Message> {
    let runner = AgentRunner {
        provider: ctx.provider.clone(),
        model: ctx.model.clone(),
        tools: ctx.tools.clone(),
        max_turns: ctx.max_turns,
    };

    let _ = ctx.event_tx.send(AgentEvent::AgentStart);
    let result = runner.run(&ctx.system_prompt, ctx.messages.clone()).await;
    let _ = ctx.event_tx.send(AgentEvent::AgentEnd {
        messages: result.messages.clone(),
    });

    ctx.messages = result.messages.clone();
    result.messages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::{LlmRequest, LlmStream, LlmStreamEvent};
    use crate::agent::tool::{ToolRegistry, Tool, ToolOutput};
    use crate::llm::ToolDefinition;
    use crate::error::AlfredError;
    use async_trait::async_trait;

    struct MockProvider;

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn stream(&self, _request: LlmRequest) -> Result<LlmStream, AlfredError> {
            let stream = async_stream::stream! {
                yield LlmStreamEvent::Start;
                yield LlmStreamEvent::TextDelta("test reply".into());
                yield LlmStreamEvent::Done {
                    text: "test reply".into(),
                    tool_calls: vec![],
                    stop_reason: StopReason::Stop,
                    usage: Usage { input: 5, output: 3, total: 8 },
                };
            };
            Ok(Box::pin(stream))
        }
    }

    fn make_loop() -> AgentLoop {
        let provider = Arc::new(MockProvider);
        let runner = AgentRunner {
            provider,
            model: "test".into(),
            tools: Arc::new(ToolRegistry::new()),
            max_turns: 5,
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
            );"
        ).unwrap();

        let store = Arc::new(crate::store::Store::from_connection(conn));
        let session_manager = Arc::new(SessionManager::new(store));
        let (event_tx, _) = broadcast::channel(16);

        AgentLoop::new(runner, session_manager, "system".into(), event_tx)
    }

    #[tokio::test]
    async fn test_handle_returns_reply() {
        let agent = make_loop();
        let inbound = InboundMessage {
            user_id: "u1".into(),
            channel: "api".into(),
            session_key: "u1:api".into(),
            text: "hello".into(),
        };

        let reply = agent.handle(inbound).await;
        assert_eq!(reply.text, "test reply");
        assert_eq!(reply.channel, "api");
    }

    #[tokio::test]
    async fn test_legacy_run_agent_loop() {
        use crate::agent::runner::AgentRunner;

        let provider = Arc::new(MockProvider);
        let mut tools = ToolRegistry::new();
        let (event_tx, _) = broadcast::channel(16);

        let mut ctx = AgentLoopContext {
            system_prompt: "system".into(),
            messages: vec![],
            provider,
            model: "test".into(),
            tools: Arc::new(tools),
            event_tx,
            max_turns: 5,
        };

        let messages = run_agent_loop(&mut ctx).await;
        assert_eq!(messages.len(), 1); // 1 assistant message
    }
}
