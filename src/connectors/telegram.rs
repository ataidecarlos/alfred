//! Telegram connector on top of a Pi session (issue #12).
//!
//! Inbound messages are forwarded to the channel's Pi RPC session as a
//! `prompt`; the streamed assistant text is sent back to the chat. The
//! connector owns the job model's client-facing concerns only — the agent loop
//! itself lives in Pi (see `src/pi/`).
//!
//! # Session lifecycle
//!
//! [`Session`] (re-exported from [`crate::pi::session`]) keeps
//! [`start`](Session::start) and [`send`](Session::send) separate. The connector
//! starts a process lazily on the first message and restarts it after a
//! failure; the idle-based supervisor built on the same session lives in
//! [`crate::pi::session`] and is ticked by [`run_compaction_ticker`] from
//! [`TelegramConnector::start`].
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
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::connectors::Connector;
use crate::error::AlfredError;
use crate::memory::append_memory_at;
use crate::paths::Paths;
use crate::pi::session::{run_compaction_ticker, CompactPolicy, COMPACT_TICK};
use crate::pi::PiInvocation;
use crate::server::AppState;

/// Re-export the channel session so connectors and tests name it here.
pub use crate::pi::session::Session;

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
        Self {
            token: token.into(),
        }
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
        .json(&SendMessageRequest {
            chat_id,
            text: text.to_string(),
        })
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

pub struct TelegramConnector {
    client: Client,
    bot_token: String,
    allowed_users: Vec<u64>,
    state: AppState,
    invocation: PiInvocation,
    sender: Arc<dyn MessageSender>,
    memories_file: PathBuf,
    sessions: Arc<Mutex<HashMap<String, Session>>>,
    policy: CompactPolicy,
}

/// Abort and reap every live channel session.
///
/// Each running session is sent Pi's `abort` command, then its child is killed
/// and waited on, so no `pi --mode rpc` process outlives the server. A failure
/// is logged, never propagated: shutdown must not be blocked by a wedged child.
pub async fn shutdown_sessions(sessions: &Arc<Mutex<HashMap<String, Session>>>) {
    let mut guard = sessions.lock().await;
    for (channel, session) in guard.iter_mut() {
        info!(channel = %channel, "channel shutdown: aborting Pi session");
        session.shutdown().await;
    }
    guard.clear();
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
        )
        .with_compaction_policy(CompactPolicy::from(pi)))
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
            sessions: Arc::new(Mutex::new(HashMap::new())),
            policy: CompactPolicy::default(),
        }
    }

    /// Replace the compaction policy (tests set an explicit idle window).
    pub fn with_compaction_policy(mut self, policy: CompactPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Replace the outbound transport (tests inject a recorder).
    pub fn with_sender(mut self, sender: Arc<dyn MessageSender>) -> Self {
        self.sender = sender;
        self
    }

    /// A shared handle to the live channel sessions.
    ///
    /// Startup holds one so shutdown can abort and reap every long-lived Pi
    /// child even after the connector itself has been moved into its run task.
    pub fn sessions(&self) -> Arc<Mutex<HashMap<String, Session>>> {
        Arc::clone(&self.sessions)
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
                self.report(chat_id, &format!("Pi session error: {error}"))
                    .await;
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
                self.report(chat_id, &format!("Pi session error: {error}"))
                    .await;
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

    /// Start the per-channel compaction ticker.
    ///
    /// Every [`COMPACT_TICK`] one supervisor pass runs against each session and
    /// compacts it when it has been idle past `idle_compact_secs` or its context
    /// exceeds `compact_token_threshold`. The ticker holds a clone of the shared
    /// session map; the sessions (and therefore their child processes, which
    /// `PiClient` marks `kill_on_drop`) are terminated when the runtime shuts
    /// down.
    fn spawn_compaction_ticker(&self) {
        let sessions = Arc::clone(&self.sessions);
        let policy = self.policy;
        tokio::spawn(run_compaction_ticker(sessions, policy, COMPACT_TICK));
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
                    self.sender
                        .send(chat.id, &format!("Remembered: {memory}"))
                        .await?;
                }
                Err(error) => {
                    error!(path = %self.memories_file.display(), %error, "telegram: /remember failed");
                    self.sender
                        .send(chat.id, &format!("Error: {error}"))
                        .await?;
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
        self.spawn_compaction_ticker();
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
            let updates = body
                .get("result")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();

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
}
