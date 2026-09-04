use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::error::AlfredError;
use crate::llm::{LlmProvider, LlmRequest, LlmStream, LlmStreamEvent, ToolDefinition};
use crate::types::{ContentBlock, StopReason, Usage};

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: &str) -> Self {
        Self { client: Client::new(), api_key: api_key.to_string() }
    }
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools.iter().map(|t| {
        serde_json::json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.parameters
        })
    }).collect()
}

fn convert_messages(messages: &[crate::types::Message]) -> Vec<Value> {
    let mut msgs = Vec::new();
    for m in messages {
        match m {
            crate::types::Message::User(u) => {
                let text: String = u.content.iter().filter_map(|c| match c {
                    crate::types::Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                }).collect::<Vec<_>>().join("");
                msgs.push(serde_json::json!({"role": "user", "content": text}));
            }
            crate::types::Message::Assistant(a) => {
                let mut content = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text { text } => {
                            content.push(serde_json::json!({"type": "text", "text": text}));
                        }
                        ContentBlock::ToolCall(tc) => {
                            content.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments
                            }));
                        }
                        _ => {}
                    }
                }
                msgs.push(serde_json::json!({"role": "assistant", "content": content}));
            }
            crate::types::Message::ToolResult(tr) => {
                let text: String = tr.content.iter().filter_map(|c| match c {
                    crate::types::Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                }).collect::<Vec<_>>().join("");
                msgs.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tr.tool_call_id,
                        "content": text
                    }]
                }));
            }
        }
    }
    msgs
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn stream(&self, request: LlmRequest) -> Result<LlmStream, AlfredError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "max_tokens": request.max_tokens.unwrap_or(4096),
            "stream": true,
            "system": request.system_prompt,
            "messages": convert_messages(&request.messages),
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(convert_tools(&request.tools));
        }

        let resp = self.client.post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AlfredError::Llm(format!("Anthropic API error {}: {}", status, text)));
        }

        let stream = resp.bytes_stream();
        let event_stream = async_stream::stream! {
            let mut accumulated_text = String::new();
            let mut tool_calls = Vec::new();
            let mut buffer = String::new();
            let mut current_tool: Option<ToolBuilder> = None;
            let mut input_tokens = 0u32;
            let mut output_tokens = 0u32;

            yield LlmStreamEvent::Start;

            futures::pin_mut!(stream);
            while let Some(chunk) = stream.next().await {
                let Ok(bytes) = chunk else { break };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() { continue; }
                    let Some(data) = line.strip_prefix("data: ") else { continue; };
                    let Ok(val) = serde_json::from_str::<Value>(data) else { continue; };

                    let Some(event_type) = val.get("type").and_then(|t| t.as_str()) else { continue; };

                    match event_type {
                        "content_block_start" => {
                            if let Some(block) = val.get("content_block") {
                                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                                    let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string();
                                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                                    current_tool = Some(ToolBuilder { id, name, input_json: String::new() });
                                }
                            }
                        }
                        "content_block_delta" => {
                            if let Some(delta) = val.get("delta") {
                                let delta_type = delta.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                match delta_type {
                                    "text_delta" => {
                                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                            accumulated_text.push_str(text);
                                            yield LlmStreamEvent::TextDelta(text.to_string());
                                        }
                                    }
                                    "input_json_delta" => {
                                        if let Some(json) = delta.get("partial_json").and_then(|j| j.as_str()) {
                                            if let Some(ref mut tc) = current_tool {
                                                tc.input_json.push_str(json);
                                                yield LlmStreamEvent::ToolCallDelta {
                                                    index: tool_calls.len(),
                                                    id: Some(tc.id.clone()),
                                                    name: Some(tc.name.clone()),
                                                    args_delta: json.to_string(),
                                                };
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "content_block_stop" => {
                            if let Some(tc) = current_tool.take() {
                                let args: Value = serde_json::from_str(&tc.input_json).unwrap_or(serde_json::json!({}));
                                tool_calls.push(crate::types::ToolCall {
                                    id: tc.id,
                                    name: tc.name,
                                    arguments: args,
                                });
                            }
                        }
                        "message_delta" => {
                            if let Some(usage) = val.get("usage") {
                                if let Some(tokens) = usage.get("output_tokens").and_then(|t| t.as_u64()) {
                                    output_tokens = tokens as u32;
                                }
                            }
                            let stop = val.get("delta").and_then(|d| d.get("stop_reason")).and_then(|r| r.as_str());
                            if let Some(reason) = stop {
                                let stop_reason = match reason {
                                    "end_turn" => StopReason::Stop,
                                    "max_tokens" => StopReason::Length,
                                    "tool_use" => StopReason::ToolUse,
                                    _ => StopReason::Stop,
                                };
                                yield LlmStreamEvent::Done {
                                    text: accumulated_text,
                                    tool_calls,
                                    stop_reason,
                                    usage: Usage { input: input_tokens, output: output_tokens, total: input_tokens + output_tokens },
                                };
                                return;
                            }
                        }
                        "message_start" => {
                            if let Some(message) = val.get("message") {
                                if let Some(usage) = message.get("usage") {
                                    if let Some(tokens) = usage.get("input_tokens").and_then(|t| t.as_u64()) {
                                        input_tokens = tokens as u32;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            yield LlmStreamEvent::Done {
                text: accumulated_text,
                tool_calls,
                stop_reason: StopReason::Stop,
                usage: Usage { input: input_tokens, output: output_tokens, total: input_tokens + output_tokens },
            };
        };

        Ok(Box::pin(event_stream))
    }
}

struct ToolBuilder {
    id: String,
    name: String,
    input_json: String,
}
