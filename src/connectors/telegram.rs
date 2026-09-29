//! Telegram connector on top of a Pi session (issue #12).
//!
//! Inbound messages are forwarded to the channel's Pi RPC session as a
//! `prompt`; the streamed assistant text is sent back to the chat. The
//! connector owns the job model's client-facing concerns only — the agent loop
//! itself lives in Pi (see `src/pi/`).
//!
//! # Session lifecycle
//!
//! [`Session`] keeps [`start`](Session::start) and [`send`](Session::send)
//! separate so the per-channel supervisor and idle-based compaction in issue
//! #14 can own process lifecycle without touching the send path. For now the
//! connector starts a process lazily on the first message and restarts it
//! after a failure.
//!
//! # Testability
//!
//! [`send_message`] is a free function so issue #13 (delivery) can reuse it
//! verbatim. Outbound traffic goes through the [`MessageSender`] trait, so
//! tests inject a recorder instead of reaching `api.telegram.org`, and
//! [`TelegramConnector::with_parts`] accepts an explicit invocation and
//! memories path so tests never touch the real `~/.alfred/`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::connectors::Connector;
use crate::error::AlfredError;
use crate::memory::append_memory_at;
use crate::paths::Paths;
use crate::pi::{PiClient, PiInvocation};
use crate::server::AppState;

/// The Pi channel name. One session per channel; issue #14 turns this into a
/// supervisor keyed by the same name.
pub const CHANNEL_NAME: &str = "telegram";

/// Telegram's maximum message length, in characters (not bytes).
const MAX_MESSAGE_LEN: usize = 4096;

/// Outbound transport for replies.
///
/// Production uses [`ApiMessageSender`], which calls [`send_message`]. Tests
/// inject a recorder so no request leaves the process.
#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, chat_id: i64, text: &str) -> Result<(), AlfredError>;
}

/// Sends replies through the Telegram Bot API.
pub struct ApiMessageSender {
    token: String,
}

impl ApiMessageSender {
    pub fn new(token: impl Into<String>) -> Self {
        Self { token: token.into() }
    }
}

#[async_trait]
impl MessageSender for ApiMessageSender {
    async fn send(&self, chat_id: i64, text: &str) -> Result<(), AlfredError> {
        send_message(&self.token, chat_id, text).await
    }
}

/// Send `text` to `chat_id`, splitting it into [`MAX_MESSAGE_LEN`]-character
/// chunks. Reused by issue #13 to deliver job results to the same chat.
pub async fn send_message(token: &str, chat_id: i64, text: &str) -> Result<(), AlfredError> {
    let client = Client::new();
    for chunk in chunk_message(text, MAX_MESSAGE_LEN) {
        send_chunk(&client, token, chat_id, &chunk).await?;
    }
    Ok(())
}

/// Split `text` on character boundaries so no chunk exceeds `max_len`
/// characters. An empty string yields a single empty chunk.
fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![String::new()];
    }
    chars
        .chunks(max_len.max(1))
        .map(|chunk| chunk.iter().collect())
        .collect()
}

async fn send_chunk(
    client: &Client,
    token: &str,
    chat_id: i64,
    text: &str,
) -> Result<(), AlfredError> {
    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let resp = client
        .post(&url)
        .json(&SendMessageRequest { chat_id, text: text.to_string() })
        .send()
        .await
        .map_err(|e| AlfredError::Connector(format!("failed to send: {e}")))?;

    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        error!("Telegram send failed: {}", body);
    }
    Ok(())
}

#[derive(Deserialize)]
struct Update {
    message: Option<TgMessage>,
}

#[derive(Deserialize)]
struct TgMessage {
    from: Option<TgUser>,
    chat: Option<TgChat>,
    text: Option<String>,
}

#[derive(Deserialize)]
struct TgUser {
    id: u64,
}

#[derive(Deserialize)]
struct TgChat {
    id: i64,
}

#[derive(Serialize)]
struct SendMessageRequest {
    chat_id: i64,
    text: String,
}

/// One channel's Pi RPC session.
///
/// `start` spawns the subprocess and `send` submits a prompt and streams the
/// reply. Keeping them separate lets issue #14's supervisor restart, compact,
/// and shut down sessions without changing the send path.
pub struct Session {
    invocation: PiInvocation,
    client: Option<PiClient>,
    dead: bool,
}

impl Session {
    pub fn new(invocation: PiInvocation) -> Self {
        Self { invocation, client: None, dead: false }
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

    /// Mark the process dead and drop it, so the next message restarts it.
    pub fn mark_dead(&mut self) {
        self.dead = true;
        self.client = None;
    }

    /// Send `prompt` and return the assistant text streamed back before the
    /// agent settled.
    pub async fn send(&mut self, prompt: &str) -> Result<String, AlfredError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| AlfredError::Pi("channel session is not started".to_string()))?;

        let response = client
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
            let message = client.next_message().await?;
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
            let fallback = client
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

        Ok(reply)
    }

    /// Clear the conversation by sending `new_session`.
    pub async fn new_session(&mut self) -> Result<(), AlfredError> {
        let client = self
            .client
            .as_mut()
            .ok_or_else(|| AlfredError::Pi("channel session is not started".to_string()))?;
        let response = client.request(json!({"type": "new_session"})).await?;
        if !response.success {
            return Err(AlfredError::Pi(format!(
                "new_session rejected: {}",
                response.error.unwrap_or_else(|| "unknown error".to_string())
            )));
        }
        Ok(())
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

pub struct TelegramConnector {
    client: Client,
    bot_token: String,
    allowed_users: Vec<u64>,
    state: AppState,
    invocation: PiInvocation,
    sender: Arc<dyn MessageSender>,
    memories_file: PathBuf,
    sessions: Mutex<HashMap<String, Session>>,
}

impl TelegramConnector {
    /// Build the connector from configuration. The system prompt is assembled
    /// once and reused for the channel's session.
    pub fn new(
        config: &crate::config::TelegramConfig,
        pi: &crate::config::PiConfig,
        prompt: &crate::config::PromptConfig,
        state: AppState,
    ) -> Result<Self, AlfredError> {
        let token = config
            .bot_token
            .as_deref()
            .ok_or_else(|| AlfredError::Connector("telegram bot_token is required".into()))?;
        let layers = crate::prompt::load_prompt_layers(prompt)?;
        let system_prompt = crate::prompt::assemble_system(&layers);
        let invocation = PiInvocation::channel(pi, system_prompt, CHANNEL_NAME);
        Ok(Self::with_parts(
            token.to_string(),
            config.allowed_users.clone(),
            state,
            invocation,
            Paths::memories_file(),
        ))
    }

    /// Assemble the connector from explicit parts. Used by [`new`] and by
    /// tests/embedders that supply their own Pi invocation and paths.
    pub fn with_parts(
        bot_token: String,
        allowed_users: Vec<u64>,
        state: AppState,
        invocation: PiInvocation,
        memories_file: PathBuf,
    ) -> Self {
        let sender: Arc<dyn MessageSender> = Arc::new(ApiMessageSender::new(bot_token.clone()));
        Self {
            client: Client::new(),
            bot_token,
            allowed_users,
            state,
            invocation,
            sender,
            memories_file,
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Replace the outbound transport (tests inject a recorder).
    pub fn with_sender(mut self, sender: Arc<dyn MessageSender>) -> Self {
        self.sender = sender;
        self
    }

    /// Handle one raw Telegram update.
    ///
    /// Public so a test or embedder can drive a single update without the
    /// `getUpdates` long-poll loop.
    pub async fn process_update(&self, update: Value) -> Result<(), AlfredError> {
        let update: Update = serde_json::from_value(update)
            .map_err(|e| AlfredError::Connector(format!("invalid telegram update: {e}")))?;
        self.handle_update(update).await
    }

    /// Ensure the channel's session has a running process.
    async fn ensure_started(&self, session: &mut Session) -> Result<(), AlfredError> {
        if !session.is_alive() {
            session.start().await?;
        }
        Ok(())
    }

    /// Forward `prompt` to the channel session and send the reply back.
    async fn prompt_channel(&self, chat_id: i64, prompt: &str) -> Result<(), AlfredError> {
        let result = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(CHANNEL_NAME.to_string())
                .or_insert_with(|| Session::new(self.invocation.clone()));
            let result = match self.ensure_started(session).await {
                Ok(()) => session.send(prompt).await,
                Err(error) => Err(error),
            };
            if result.is_err() {
                session.mark_dead();
            }
            result
        };

        match result {
            Ok(reply) => self.sender.send(chat_id, &reply).await,
            Err(error) => {
                self.report(chat_id, &format!("Pi session error: {error}")).await;
                Err(error)
            }
        }
    }

    /// Clear the channel session with `new_session`.
    async fn clear_channel(&self, chat_id: i64) -> Result<(), AlfredError> {
        let result = {
            let mut sessions = self.sessions.lock().await;
            let session = sessions
                .entry(CHANNEL_NAME.to_string())
                .or_insert_with(|| Session::new(self.invocation.clone()));
            let result = match self.ensure_started(session).await {
                Ok(()) => session.new_session().await,
                Err(error) => Err(error),
            };
            if result.is_err() {
                session.mark_dead();
            }
            result
        };

        match result {
            Ok(()) => self.sender.send(chat_id, "Session cleared.").await,
            Err(error) => {
                self.report(chat_id, &format!("Pi session error: {error}")).await;
                Err(error)
            }
        }
    }

    /// Best-effort notice to the chat. A failed notice is logged, not fatal.
    async fn report(&self, chat_id: i64, text: &str) {
        if let Err(error) = self.sender.send(chat_id, text).await {
            error!(chat_id, %error, "telegram: failed to report to chat");
        }
    }

    async fn handle_update(&self, update: Update) -> Result<(), AlfredError> {
        let msg = match update.message {
            Some(m) => m,
            None => return Ok(()),
        };

        let text = match msg.text {
            Some(t) => t,
            None => return Ok(()),
        };

        let from = match msg.from {
            Some(u) => u,
            None => return Ok(()),
        };

        let chat = match msg.chat {
            Some(c) => c,
            None => return Ok(()),
        };

        // Check allowed users before touching Pi: an unauthorized sender is
        // dropped and warned, and must not start a session.
        if !self.allowed_users.is_empty() && !self.allowed_users.contains(&from.id) {
            warn!("Ignoring message from unauthorized user {}", from.id);
            return Ok(());
        }

        info!("Telegram message from user {}: {}", from.id, text);

        if text.starts_with("/todos") {
            match self.state.store.list_todos() {
                Ok(todos) if todos.is_empty() => {
                    self.sender.send(chat.id, "No todos found.").await?;
                }
                Ok(todos) => {
                    let formatted: Vec<String> = todos
                        .iter()
                        .map(|t| {
                            let status = if t.completed { "x" } else { " " };
                            format!("[{}] {} ({})", status, t.title, t.priority)
                        })
                        .collect();
                    self.sender.send(chat.id, &formatted.join("\n")).await?;
                }
                Err(e) => {
                    self.sender.send(chat.id, &format!("Error: {e}")).await?;
                }
            }
            return Ok(());
        }

        if let Some(rest) = text.strip_prefix("/remember") {
            let memory = rest.trim();
            if memory.is_empty() {
                self.sender.send(chat.id, "Usage: /remember <text>").await?;
                return Ok(());
            }
            match append_memory_at(&self.memories_file, memory) {
                Ok(()) => {
                    self.sender.send(chat.id, &format!("Remembered: {memory}")).await?;
                }
                Err(error) => {
                    error!(path = %self.memories_file.display(), %error, "telegram: /remember failed");
                    self.sender.send(chat.id, &format!("Error: {error}")).await?;
                }
            }
            return Ok(());
        }

        if text.trim() == "/clear" {
            return self.clear_channel(chat.id).await;
        }

        // Anything else is a conversational turn on the channel's Pi session.
        self.prompt_channel(chat.id, &text).await
    }
}

#[async_trait]
impl Connector for TelegramConnector {
    async fn start(&self) -> Result<(), AlfredError> {
        info!("Starting Telegram connector...");
        let mut offset: i64 = 0;

        loop {
            let url = format!(
                "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout=30",
                self.bot_token, offset
            );

            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    error!("Telegram poll error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    continue;
                }
            };

            if !resp.status().is_success() {
                let body = resp.text().await.unwrap_or_default();
                error!("Telegram API error: {}", body);
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                continue;
            }

            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            let updates = body.get("result").and_then(|r| r.as_array()).cloned().unwrap_or_default();

            for update_val in updates {
                if let Some(id) = update_val.get("update_id").and_then(Value::as_i64) {
                    offset = id + 1;
                }
                if let Err(e) = self.process_update(update_val).await {
                    error!("Error handling Telegram update: {}", e);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_message_never_exceeds_the_limit_on_char_boundaries() {
        // Exactly the limit: one chunk.
        let exact: String = "a".repeat(MAX_MESSAGE_LEN);
        assert_eq!(chunk_message(&exact, MAX_MESSAGE_LEN).len(), 1);

        // One over the limit: two chunks, the second holding one character.
        let over = "a".repeat(MAX_MESSAGE_LEN + 1);
        let chunks = chunk_message(&over, MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].chars().count(), MAX_MESSAGE_LEN);
        assert_eq!(chunks[1], "a");

        // Multi-byte characters count by character, not byte, so a chunk is
        // never split inside a UTF-8 sequence.
        let emoji = "😀".repeat(MAX_MESSAGE_LEN + 5);
        let chunks = chunk_message(&emoji, MAX_MESSAGE_LEN);
        assert_eq!(chunks.len(), 2);
        assert!(chunks.iter().all(|c| c.chars().count() <= MAX_MESSAGE_LEN));
        assert!(chunks.iter().all(|c| c.chars().all(|ch| ch == '😀')));

        // An empty reply is still one (empty) chunk.
        assert_eq!(chunk_message("", MAX_MESSAGE_LEN), vec![String::new()]);
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
}
