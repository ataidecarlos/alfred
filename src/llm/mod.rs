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

/// OpenCode Go exposes an OpenAI-compatible chat completions API.
pub const OPENCODE_GO_BASE_URL: &str = "https://opencode.ai/zen/go/v1";

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(&self, request: LlmRequest) -> Result<LlmStream, AlfredError>;
}

pub fn create_provider(
    provider_name: &str,
    config: &ProviderConfig,
) -> Result<Arc<dyn LlmProvider>, AlfredError> {
    let api_key = config.api_key.as_deref().unwrap_or("").trim();
    if api_key.is_empty() {
        return Err(AlfredError::Llm(format!(
            "provider '{}' has no api_key configured; set providers.{}.api_key in {} (supports ${{ENV_VAR}} expansion)",
            provider_name,
            provider_name,
            crate::paths::Paths::config_file().display(),
        )));
    }
    match provider_name {
        "openai" => Ok(Arc::new(openai::OpenAiProvider::new(api_key, config.base_url.as_deref().unwrap_or("https://api.openai.com/v1")))),
        "anthropic" => Ok(Arc::new(anthropic::AnthropicProvider::new(api_key))),
        "google" => Ok(Arc::new(google::GoogleProvider::new(api_key))),
        "deepseek" => Ok(Arc::new(openai::OpenAiProvider::new(api_key, config.base_url.as_deref().unwrap_or("https://api.deepseek.com")))),
        // OpenCode Go is OpenAI-compatible and routed through the Zen gateway.
        "opencode-go" => Ok(Arc::new(openai::OpenAiProvider::new(
            api_key,
            config.base_url.as_deref().unwrap_or(OPENCODE_GO_BASE_URL),
        ))),
        _ => Err(AlfredError::Llm(format!("unknown provider: {}", provider_name))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ProviderConfig;

    fn provider_config(api_key: Option<&str>, base_url: Option<&str>) -> ProviderConfig {
        ProviderConfig {
            api_key: api_key.map(str::to_string),
            model: "space-bunny-free".to_string(),
            base_url: base_url.map(str::to_string),
        }
    }

    #[test]
    fn opencode_go_provider_is_supported() {
        let config = provider_config(Some("test-key"), None);
        assert!(create_provider("opencode-go", &config).is_ok());
    }

    #[test]
    fn opencode_go_provider_accepts_custom_base_url() {
        let config = provider_config(Some("test-key"), Some("https://example.com/v1"));
        assert!(create_provider("opencode-go", &config).is_ok());
    }

    #[test]
    fn provider_without_api_key_is_rejected() {
        let config = provider_config(None, None);
        assert!(create_provider("opencode-go", &config).is_err());
    }

    #[test]
    fn unknown_provider_is_rejected() {
        let config = provider_config(Some("test-key"), None);
        let err = match create_provider("does-not-exist", &config) {
            Ok(_) => panic!("expected unknown provider to be rejected"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("unknown provider"));
    }
}
