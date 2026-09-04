use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, error, warn};

use crate::agent::{AgentLoopContext, run_agent_loop};
use crate::connectors::Connector;
use crate::error::AlfredError;
use crate::server::AppState;
use crate::types::{Message, UserMessage, Content};

pub struct TelegramConnector {
    client: Client,
    bot_token: String,
    allowed_users: Vec<u64>,
    state: AppState,
}

#[derive(Deserialize)]
struct Update {
    update_id: i64,
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

impl TelegramConnector {
    pub fn new(config: &crate::config::TelegramConfig, state: AppState) -> Result<Self, AlfredError> {
        let token = config.bot_token.as_deref()
            .ok_or_else(|| AlfredError::Connector("telegram bot_token is required".into()))?;
        Ok(Self {
            client: Client::new(),
            bot_token: token.to_string(),
            allowed_users: config.allowed_users.clone(),
            state,
        })
    }

    async fn send_reply(&self, chat_id: i64, text: &str) -> Result<(), AlfredError> {
        let max_len = 4096;
        let chars: Vec<char> = text.chars().collect();
        let total = chars.len();

        if total <= max_len {
            self.send_single(chat_id, text).await?;
        } else {
            let mut start = 0;
            while start < total {
                let end = std::cmp::min(start + max_len, total);
                let chunk: String = chars[start..end].iter().collect();
                self.send_single(chat_id, &chunk).await?;
                start = end;
            }
        }
        Ok(())
    }

    async fn send_single(&self, chat_id: i64, text: &str) -> Result<(), AlfredError> {
        let url = format!("https://api.telegram.org/bot{}/sendMessage", self.bot_token);
        let resp = self.client.post(&url)
            .json(&SendMessageRequest { chat_id, text: text.to_string() })
            .send()
            .await
            .map_err(|e| AlfredError::Connector(format!("failed to send: {}", e)))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            error!("Telegram send failed: {}", body);
        }
        Ok(())
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

        // Check allowed users
        if !self.allowed_users.is_empty() && !self.allowed_users.contains(&from.id) {
            warn!("Ignoring message from unauthorized user {}", from.id);
            return Ok(());
        }

        info!("Telegram message from user {}: {}", from.id, text);

        // Handle commands
        if text.starts_with("/clear") {
            self.state.store.save_conversation(&from.id.to_string(), "telegram", &[])?;
            self.send_reply(chat.id, "Conversation cleared.").await?;
            return Ok(());
        }

        if text.starts_with("/todos") {
            match self.state.store.list_todos() {
                Ok(todos) if todos.is_empty() => {
                    self.send_reply(chat.id, "No todos found.").await?;
                }
                Ok(todos) => {
                    let formatted: Vec<String> = todos.iter().map(|t| {
                        let status = if t.completed { "x" } else { " " };
                        format!("[{}] {} ({})", status, t.title, t.priority)
                    }).collect();
                    self.send_reply(chat.id, &formatted.join("\n")).await?;
                }
                Err(e) => {
                    self.send_reply(chat.id, &format!("Error: {}", e)).await?;
                }
            }
            return Ok(());
        }

        if text.starts_with("/memories") {
            match self.state.store.list_memories() {
                Ok(memories) if memories.is_empty() => {
                    self.send_reply(chat.id, "No memories stored.").await?;
                }
                Ok(memories) => {
                    let formatted: Vec<String> = memories.iter().map(|m| format!("- {}", m.content)).collect();
                    self.send_reply(chat.id, &formatted.join("\n")).await?;
                }
                Err(e) => {
                    self.send_reply(chat.id, &format!("Error: {}", e)).await?;
                }
            }
            return Ok(());
        }

        // Regular message - run agent loop
        let user_id = from.id.to_string();
        let mut messages = self.state.store.load_conversation(&user_id, "telegram")
            .unwrap_or(None)
            .unwrap_or_default();

        messages.push(Message::User(UserMessage {
            content: vec![Content::Text(crate::types::TextContent { text: text.clone() })],
            timestamp: chrono::Utc::now(),
        }));

        let event_tx = self.state.event_tx.clone();
        let mut ctx = AgentLoopContext {
            system_prompt: self.state.system_prompt.clone(),
            messages,
            provider: self.state.provider.clone(),
            model: self.state.model.clone(),
            tools: self.state.tools.clone(),
            event_tx,
            max_turns: 5,
        };

        run_agent_loop(&mut ctx).await;

        // Get reply
        let reply = ctx.messages.iter().rev().find_map(|m| {
            if matches!(m, Message::Assistant(_)) {
                Some(crate::types::extract_text(m))
            } else {
                None
            }
        }).unwrap_or_else(|| "I could not generate a response.".into());

        self.send_reply(chat.id, &reply).await?;
        self.state.store.save_conversation(&user_id, "telegram", &ctx.messages)?;

        Ok(())
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
                if let Ok(update) = serde_json::from_value::<Update>(update_val.clone()) {
                    offset = update.update_id + 1;
                    if let Err(e) = self.handle_update(update).await {
                        error!("Error handling Telegram update: {}", e);
                    }
                }
            }
        }
    }
}
