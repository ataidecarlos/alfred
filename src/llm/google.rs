use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::error::AlfredError;
use crate::llm::{LlmProvider, LlmRequest, LlmStream, LlmStreamEvent, ToolDefinition};
use crate::types::{ContentBlock, StopReason, Usage};

pub struct GoogleProvider {
    client: Client,
    api_key: String,
}

impl GoogleProvider {
    pub fn new(api_key: &str) -> Self {
        Self { client: Client::new(), api_key: api_key.to_string() }
    }
}

fn convert_tools(tools: &[ToolDefinition]) -> Value {
    let decls: Vec<Value> = tools.iter().map(|t| {
        serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters
        })
    }).collect();
    serde_json::json!([{"functionDeclarations": decls}])
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
                msgs.push(serde_json::json!({"role": "user", "parts": [{"text": text}]}));
            }
            crate::types::Message::Assistant(a) => {
                let mut parts = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text { text } => {
                            parts.push(serde_json::json!({"text": text}));
                        }
                        ContentBlock::ToolCall(tc) => {
                            parts.push(serde_json::json!({
                                "functionCall": {"name": tc.name, "args": tc.arguments}
                            }));
                        }
                        _ => {}
                    }
                }
                msgs.push(serde_json::json!({"role": "model", "parts": parts}));
            }
            crate::types::Message::ToolResult(tr) => {
                let text: String = tr.content.iter().filter_map(|c| match c {
                    crate::types::Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                }).collect::<Vec<_>>().join("");
                msgs.push(serde_json::json!({
                    "role": "function",
                    "parts": [{"functionResponse": {"name": tr.tool_name, "response": {"result": text}}}]
                }));
            }
        }
    }
    msgs
}

#[async_trait]
impl LlmProvider for GoogleProvider {
    async fn stream(&self, request: LlmRequest) -> Result<LlmStream, AlfredError> {
        let mut body = serde_json::json!({
            "contents": convert_messages(&request.messages),
            "generationConfig": {
                "maxOutputTokens": request.max_tokens.unwrap_or(4096),
            },
        });

        if !request.system_prompt.is_empty() {
            body["systemInstruction"] = serde_json::json!({"parts": [{"text": request.system_prompt}]});
        }
        if !request.tools.is_empty() {
            body["tools"] = convert_tools(&request.tools);
        }

        let model = &request.model;
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            model, self.api_key
        );

        let resp = self.client.post(&url)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AlfredError::Llm(format!("Google API error {}: {}", status, text)));
        }

        let stream = resp.bytes_stream();
        let event_stream = async_stream::stream! {
            let mut accumulated_text = String::new();
            let mut tool_calls = Vec::new();
            let mut buffer = String::new();

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

                    let Some(candidates) = val.get("candidates").and_then(|c| c.as_array()) else { continue; };
                    let Some(candidate) = candidates.first() else { continue; };
                    let Some(content) = candidate.get("content") else { continue; };
                    let Some(parts) = content.get("parts").and_then(|p| p.as_array()) else { continue; };

                    for part in parts {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            accumulated_text.push_str(text);
                            yield LlmStreamEvent::TextDelta(text.to_string());
                        }
                        if let Some(fc) = part.get("functionCall") {
                            let name = fc.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                            let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
                            let id = format!("call_{}", uuid::Uuid::new_v4());
                            let idx = tool_calls.len();
                            tool_calls.push(crate::types::ToolCall {
                                id: id.clone(),
                                name: name.clone(),
                                arguments: args.clone(),
                            });
                            yield LlmStreamEvent::ToolCallDelta {
                                index: idx,
                                id: Some(id),
                                name: Some(name),
                                args_delta: args.to_string(),
                            };
                        }
                    }

                    let finish = candidate.get("finishReason").and_then(|f| f.as_str());
                    if let Some(reason) = finish {
                        let stop_reason = match reason {
                            "STOP" => StopReason::Stop,
                            "MAX_TOKENS" => StopReason::Length,
                            "OTHER" => StopReason::Stop,
                            _ => StopReason::Stop,
                        };
                        let usage_metadata = val.get("usageMetadata");
                        let input = usage_metadata.and_then(|u| u.get("promptTokenCount")).and_then(|t| t.as_u64()).unwrap_or(0) as u32;
                        let output = usage_metadata.and_then(|u| u.get("candidatesTokenCount")).and_then(|t| t.as_u64()).unwrap_or(0) as u32;
                        yield LlmStreamEvent::Done {
                            text: accumulated_text,
                            tool_calls,
                            stop_reason,
                            usage: Usage { input, output, total: input + output },
                        };
                        return;
                    }
                }
            }

            yield LlmStreamEvent::Done {
                text: accumulated_text,
                tool_calls,
                stop_reason: StopReason::Stop,
                usage: Usage { input: 0, output: 0, total: 0 },
            };
        };

        Ok(Box::pin(event_stream))
    }
}
