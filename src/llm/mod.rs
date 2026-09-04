pub mod openai;
pub mod anthropic;
pub mod google;

use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use futures::Stream;

use crate::config::ProviderConfig;
use crate::error::AlfredError;
use crate::types::ToolCall;

pub struct LlmRequest {
    pub model: String,
    pub system_prompt: String,
    pub messages: Vec<crate::types::Message>,
    pub tools: Vec<ToolDefinition>,
    pub max_tokens: Option<u32>,
}

pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: schemars::schema::RootSchema,
}

pub enum LlmStreamEvent {
    Start,
    TextDelta(String),
    ToolCallDelta { index: usize, id: Option<String>, name: Option<String>, args_delta: String },
    Done { text: String, tool_calls: Vec<ToolCall>, stop_reason: crate::types::StopReason, usage: crate::types::Usage },
    Error(String),
}

pub type LlmStream = Pin<Box<dyn Stream<Item = LlmStreamEvent> + Send>>;

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(&self, request: LlmRequest) -> Result<LlmStream, AlfredError>;
}

pub fn create_provider(
    provider_name: &str,
    config: &ProviderConfig,
) -> Result<Arc<dyn LlmProvider>, AlfredError> {
    let api_key = config.api_key.as_deref().unwrap_or("");
    match provider_name {
        "openai" => Ok(Arc::new(openai::OpenAiProvider::new(api_key, config.base_url.as_deref().unwrap_or("https://api.openai.com/v1")))),
        "anthropic" => Ok(Arc::new(anthropic::AnthropicProvider::new(api_key))),
        "google" => Ok(Arc::new(google::GoogleProvider::new(api_key))),
        "deepseek" => Ok(Arc::new(openai::OpenAiProvider::new(api_key, config.base_url.as_deref().unwrap_or("https://api.deepseek.com")))),
        _ => Err(AlfredError::Llm(format!("unknown provider: {}", provider_name))),
    }
}
