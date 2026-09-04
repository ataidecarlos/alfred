use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde_json::Value;

use crate::error::AlfredError;
use crate::llm::{LlmProvider, LlmRequest, LlmStream, LlmStreamEvent, ToolDefinition};
use crate::types::{ContentBlock, StopReason, Usage};

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &str, base_url: &str) -> Self {
        Self { client: Client::new(), api_key: api_key.to_string(), base_url: base_url.to_string() }
    }
}

fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
    tools.iter().map(|t| {
        serde_json::json!({
            "type": "function",
            "function": {
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters
            }
        })
    }).collect()
}

fn convert_messages(system_prompt: &str, messages: &[crate::types::Message]) -> Vec<Value> {
    let mut msgs = Vec::new();
    if !system_prompt.is_empty() {
        msgs.push(serde_json::json!({"role": "system", "content": system_prompt}));
    }
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
                let mut text = String::new();
                let mut tool_calls = Vec::new();
                for block in &a.content {
                    match block {
                        ContentBlock::Text { text: t } => text.push_str(t),
                        ContentBlock::ToolCall(tc) => tool_calls.push(serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {"name": tc.name, "arguments": tc.arguments.to_string()}
                        })),
                        _ => {}
                    }
                }
                let mut obj = serde_json::json!({"role": "assistant", "content": text});
                if !tool_calls.is_empty() {
                    obj["tool_calls"] = Value::Array(tool_calls);
                }
                msgs.push(obj);
            }
            crate::types::Message::ToolResult(tr) => {
                let text: String = tr.content.iter().filter_map(|c| match c {
                    crate::types::Content::Text(t) => Some(t.text.as_str()),
                    _ => None,
                }).collect::<Vec<_>>().join("");
                msgs.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": tr.tool_call_id,
                    "content": text
                }));
            }
        }
    }
    msgs
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn stream(&self, request: LlmRequest) -> Result<LlmStream, AlfredError> {
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": convert_messages(&request.system_prompt, &request.messages),
            "stream": true,
        });
        if let Some(max) = request.max_tokens {
            body["max_tokens"] = serde_json::json!(max);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(convert_tools(&request.tools));
        }

        // DeepSeek enables thinking by default; disable it for simpler responses
        if self.base_url.contains("deepseek") {
            body["thinking"] = serde_json::json!({"type": "disabled"});
        }

        tracing::debug!("Calling OpenAI-compatible API at: {}/chat/completions", self.base_url);
        tracing::debug!("Request body: {}", serde_json::to_string_pretty(&body).unwrap_or_default());

        let resp = self.client.post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        tracing::debug!("Response status: {}", resp.status());

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            tracing::error!("API error {}: {}", status, text);
            return Err(AlfredError::Llm(format!("OpenAI API error {}: {}", status, text)));
        }

        let stream = resp.bytes_stream();
        let event_stream = async_stream::stream! {
            let mut accumulated_text = String::new();
            let mut tool_calls: Vec<ToolAccumulator> = Vec::new();
            let mut buffer = String::new();

            yield LlmStreamEvent::Start;

            futures::pin_mut!(stream);
            while let Some(chunk) = stream.next().await {
                let Ok(bytes) = chunk else { break };
                buffer.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(line_end) = buffer.find('\n') {
                    let line = buffer[..line_end].trim().to_string();
                    buffer = buffer[line_end + 1..].to_string();

                    if line.is_empty() || line == "data: [DONE]" { continue; }
                    let Some(data) = line.strip_prefix("data: ") else { continue; };
                    let Ok(val) = serde_json::from_str::<Value>(data) else { continue; };

                    let Some(choices) = val.get("choices").and_then(|c| c.as_array()) else { continue; };
                    let Some(choice) = choices.first() else { continue; };
                    let Some(delta) = choice.get("delta") else { continue; };

                    if let Some(text) = delta.get("content").and_then(|t| t.as_str()).filter(|s| !s.is_empty()) {
                        accumulated_text.push_str(text);
                        yield LlmStreamEvent::TextDelta(text.to_string());
                    }

                    if let Some(tcs) = delta.get("tool_calls").and_then(|t| t.as_array()) {
                        for tc in tcs {
                            let idx = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            while tool_calls.len() <= idx {
                                tool_calls.push(ToolAccumulator::default());
                            }
                            let acc = &mut tool_calls[idx];
                            if let Some(id) = tc.get("id").and_then(|i| i.as_str()) {
                                acc.id = Some(id.to_string());
                            }
                            if let Some(func) = tc.get("function") {
                                if let Some(name) = func.get("name").and_then(|n| n.as_str()) {
                                    acc.name = Some(name.to_string());
                                }
                                if let Some(args) = func.get("arguments").and_then(|a| a.as_str()) {
                                    acc.args.push_str(args);
                                    yield LlmStreamEvent::ToolCallDelta {
                                        index: idx,
                                        id: acc.id.clone(),
                                        name: acc.name.clone(),
                                        args_delta: args.to_string(),
                                    };
                                }
                            }
                        }
                    }

                    let finish_reason = choice.get("finish_reason").and_then(|f| f.as_str());
                    if let Some(reason) = finish_reason {
                        let stop = match reason {
                            "stop" => StopReason::Stop,
                            "length" => StopReason::Length,
                            "tool_calls" => StopReason::ToolUse,
                            _ => StopReason::Stop,
                        };
                        let usage = val.get("usage").and_then(|u| serde_json::from_value::<Usage>(u.clone()).ok())
                            .unwrap_or(Usage { input: 0, output: 0, total: 0 });
                        let tool_calls_out = tool_calls.into_iter().filter_map(|tc| {
                            let id = tc.id?;
                            let name = tc.name?;
                            let args: Value = serde_json::from_str(&tc.args).unwrap_or(serde_json::json!({}));
                            Some(crate::types::ToolCall { id, name, arguments: args })
                        }).collect();
                        yield LlmStreamEvent::Done {
                            text: accumulated_text,
                            tool_calls: tool_calls_out,
                            stop_reason: stop,
                            usage,
                        };
                        return;
                    }
                }
            }

            yield LlmStreamEvent::Done {
                text: accumulated_text,
                tool_calls: Vec::new(),
                stop_reason: StopReason::Stop,
                usage: Usage { input: 0, output: 0, total: 0 },
            };
        };

        Ok(Box::pin(event_stream))
    }
}

#[derive(Default)]
struct ToolAccumulator {
    id: Option<String>,
    name: Option<String>,
    args: String,
}
