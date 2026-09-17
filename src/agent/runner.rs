use std::sync::Arc;

use futures::StreamExt;
use tracing::error;

use crate::llm::{LlmProvider, LlmStreamEvent};
use crate::types::{Message, text_content, AssistantMessage, ContentBlock, StopReason, Usage, ToolCall, ToolResultMessage};
use crate::agent::tool::ToolRegistry;

/// Result of running the agent loop through all turns.
pub struct RunnerResult {
    /// All messages including the input messages plus assistant and tool result messages.
    pub messages: Vec<Message>,
    /// Cumulative usage across all turns.
    pub usage: Usage,
    /// Final stop reason.
    pub stop_reason: StopReason,
}

/// Model-facing agent loop. Handles provider calls, streaming, tool execution,
/// and iteration limits. Pure function of (system_prompt, messages) → (messages, usage, stop_reason).
pub struct AgentRunner {
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
    pub max_turns: u32,
}

impl AgentRunner {
    /// Run the agent loop to completion.
    ///
    /// Returns the accumulated messages, cumulative usage, and final stop reason.
    /// Does NOT manage sessions, channels, or events — that's the AgentLoop's job.
    pub async fn run(
        &self,
        system_prompt: &str,
        mut messages: Vec<Message>,
    ) -> RunnerResult {
        let mut turn = 0;
        let mut cumulative_usage = Usage { input: 0, output: 0, total: 0 };
        let mut last_stop_reason = StopReason::Stop;

        loop {
            turn += 1;
            if turn > self.max_turns {
                tracing::info!("max turns reached ({}), stopping", self.max_turns);
                break;
            }

            // Build LLM request
            let tool_defs = self.tools.definitions();
            let request = crate::llm::LlmRequest {
                model: self.model.clone(),
                system_prompt: system_prompt.to_string(),
                messages: messages.clone(),
                tools: tool_defs,
                max_tokens: Some(4096),
            };

            tracing::debug!("Calling LLM provider with model: {}", self.model);

            // Stream response
            let stream = match self.provider.stream(request).await {
                Ok(s) => s,
                Err(e) => {
                    error!("LLM stream error: {}", e);
                    last_stop_reason = StopReason::Error;
                    break;
                }
            };

            let mut accumulated_text = String::new();
            let mut tool_calls_out: Vec<ToolCall> = Vec::new();
            let mut tool_call_args: Vec<String> = Vec::new();
            let mut usage = Usage { input: 0, output: 0, total: 0 };

            futures::pin_mut!(stream);
            while let Some(event) = stream.next().await {
                match event {
                    LlmStreamEvent::Start => {}
                    LlmStreamEvent::TextDelta(_delta) => {}
                    LlmStreamEvent::ToolCallDelta { index, id, name, args_delta } => {
                        while tool_calls_out.len() <= index {
                            tool_calls_out.push(ToolCall {
                                id: String::new(),
                                name: String::new(),
                                arguments: serde_json::json!({}),
                            });
                            tool_call_args.push(String::new());
                        }
                        if let Some(id) = id { tool_calls_out[index].id = id; }
                        if let Some(name) = name { tool_calls_out[index].name = name; }
                        if !args_delta.is_empty() {
                            tool_call_args[index].push_str(&args_delta);
                        }
                    }
                    LlmStreamEvent::Done { text, tool_calls, stop_reason: sr, usage: u } => {
                        accumulated_text = text;
                        for (i, tc) in tool_calls_out.iter_mut().enumerate() {
                            if i < tool_call_args.len() && !tool_call_args[i].is_empty() {
                                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&tool_call_args[i]) {
                                    tc.arguments = v;
                                }
                            }
                        }
                        if !tool_calls.is_empty() {
                            for tc in tool_calls {
                                if let Some(existing) = tool_calls_out.iter_mut().find(|t| t.name == tc.name && t.id.is_empty()) {
                                    existing.id = tc.id;
                                    existing.name = tc.name;
                                    if !tc.arguments.is_null() && !tc.arguments.as_object().map_or(false, |o| o.is_empty()) {
                                        existing.arguments = tc.arguments;
                                    }
                                } else if !tool_calls_out.iter().any(|t| t.id == tc.id) {
                                    tool_calls_out.push(tc);
                                }
                            }
                        }
                        last_stop_reason = sr;
                        usage = u;
                    }
                    LlmStreamEvent::Error(e) => {
                        error!("LLM stream error: {}", e);
                        last_stop_reason = StopReason::Error;
                        break;
                    }
                }
            }

            cumulative_usage.input += usage.input;
            cumulative_usage.output += usage.output;
            cumulative_usage.total += usage.total;

            // Build assistant message
            let mut content = Vec::new();
            if !accumulated_text.is_empty() {
                content.push(ContentBlock::Text { text: accumulated_text });
            }
            content.extend(tool_calls_out.iter().map(|tc| ContentBlock::ToolCall(tc.clone())));

            let assistant_msg = AssistantMessage {
                content,
                usage,
                stop_reason: last_stop_reason.clone(),
                model: self.model.clone(),
                timestamp: chrono::Utc::now(),
            };

            messages.push(Message::Assistant(assistant_msg.clone()));

            // If no tool calls, we're done
            let has_tool_calls = !tool_calls_out.is_empty();
            if !has_tool_calls || matches!(last_stop_reason, StopReason::Error | StopReason::Aborted) {
                break;
            }

            // Execute tool calls
            for tc in &tool_calls_out {
                tracing::debug!("Executing tool call: {} with args: {}", tc.name, tc.arguments);

                let result = self.tools.execute(&tc.name, tc.arguments.clone()).await;
                tracing::debug!("Tool result: {}", result.output);

                let tool_result_msg = ToolResultMessage {
                    tool_call_id: tc.id.clone(),
                    tool_name: tc.name.clone(),
                    content: vec![text_content(result.output)],
                    is_error: result.is_error,
                    timestamp: chrono::Utc::now(),
                };

                messages.push(Message::ToolResult(tool_result_msg));
            }
        }

        RunnerResult {
            messages,
            usage: cumulative_usage,
            stop_reason: last_stop_reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::ToolDefinition;
    use async_trait::async_trait;

    /// A mock provider that returns a fixed text response on the first call,
    /// then stops.
    struct MockProvider {
        response: String,
        call_count: std::sync::atomic::AtomicU32,
    }

    impl MockProvider {
        fn new(response: &str) -> Self {
            Self {
                response: response.to_string(),
                call_count: std::sync::atomic::AtomicU32::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn stream(
            &self,
            _request: crate::llm::LlmRequest,
        ) -> Result<crate::llm::LlmStream, crate::error::AlfredError> {
            use crate::error::AlfredError;
            let count = self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count >= 1 {
                return Err(AlfredError::Llm("mock: no more calls".into()));
            }

            let response = self.response.clone();
            let response2 = response.clone();
            let stream = async_stream::stream! {
                yield LlmStreamEvent::Start;
                yield LlmStreamEvent::TextDelta(response);
                yield LlmStreamEvent::Done {
                    text: response2,
                    tool_calls: vec![],
                    stop_reason: StopReason::Stop,
                    usage: Usage { input: 10, output: 5, total: 15 },
                };
            };
            Ok(Box::pin(stream))
        }
    }

    /// A mock provider that returns a tool call, then a text response.
    struct MockToolProvider {
        call_count: std::sync::atomic::AtomicU32,
    }

    impl MockToolProvider {
        fn new() -> Self {
            Self { call_count: std::sync::atomic::AtomicU32::new(0) }
        }
    }

    #[async_trait]
    impl LlmProvider for MockToolProvider {
        async fn stream(
            &self,
            _request: crate::llm::LlmRequest,
        ) -> Result<crate::llm::LlmStream, crate::error::AlfredError> {
            let count = self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

            // Use Box::pin to unify the async block types
            let stream: crate::llm::LlmStream = if count == 0 {
                Box::pin(async_stream::stream! {
                    yield LlmStreamEvent::Start;
                    yield LlmStreamEvent::Done {
                        text: String::new(),
                        tool_calls: vec![ToolCall {
                            id: "call_1".into(),
                            name: "echo".into(),
                            arguments: serde_json::json!({"text": "hello"}),
                        }],
                        stop_reason: StopReason::ToolUse,
                        usage: Usage { input: 10, output: 5, total: 15 },
                    };
                })
            } else {
                Box::pin(async_stream::stream! {
                    yield LlmStreamEvent::Start;
                    yield LlmStreamEvent::TextDelta("done".into());
                    yield LlmStreamEvent::Done {
                        text: "done".into(),
                        tool_calls: vec![],
                        stop_reason: StopReason::Stop,
                        usage: Usage { input: 10, output: 5, total: 15 },
                    };
                })
            };
            Ok(stream)
        }
    }

    /// A simple echo tool for testing.
    struct EchoTool;

    #[async_trait]
    impl crate::agent::tool::Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".into(),
                description: "Echo back the text".into(),
                parameters: schemars::schema_for!(serde_json::Value),
            }
        }

        async fn execute(&self, args: serde_json::Value) -> crate::agent::tool::ToolOutput {
            let text = args.get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("no text");
            crate::agent::tool::ToolOutput::success(text)
        }
    }

    fn make_runner(provider: Arc<dyn LlmProvider>) -> AgentRunner {
        let mut tools = ToolRegistry::new();
        tools.register(Arc::new(EchoTool));
        AgentRunner {
            provider,
            model: "test".into(),
            tools: Arc::new(tools),
            max_turns: 5,
        }
    }

    #[tokio::test]
    async fn test_runner_single_turn() {
        let provider = Arc::new(MockProvider::new("hello world"));
        let runner = make_runner(provider);

        let result = runner.run("system", vec![]).await;
        assert_eq!(result.messages.len(), 1); // 1 assistant message
        assert_eq!(result.usage.input, 10);
        assert_eq!(result.stop_reason, StopReason::Stop);
    }

    #[tokio::test]
    async fn test_runner_tool_call_loop() {
        let provider = Arc::new(MockToolProvider::new());
        let runner = make_runner(provider);

        let result = runner.run("system", vec![]).await;
        // Expect: assistant (tool call) + tool result + assistant (text)
        assert!(result.messages.len() >= 2, "should have at least assistant + tool_result, got {}", result.messages.len());
        assert_eq!(result.stop_reason, StopReason::Stop);
    }

    #[tokio::test]
    async fn test_runner_max_turns() {
        let provider = Arc::new(MockProvider::new("hello"));
        let mut runner = make_runner(provider);
        runner.max_turns = 1;

        let result = runner.run("system", vec![]).await;
        assert_eq!(result.messages.len(), 1);
    }
}
