pub mod event;
pub mod tool;

use std::sync::Arc;

use futures::StreamExt;
use tokio::sync::broadcast;
use tracing::{info, error};

use crate::llm::{LlmProvider, LlmStreamEvent};
use crate::types::{Message, text_content, AssistantMessage, ContentBlock, StopReason, Usage, ToolCall, ToolResultMessage};
use crate::agent::event::AgentEvent;
use crate::agent::tool::ToolRegistry;

pub struct AgentLoopContext {
    pub system_prompt: String,
    pub messages: Vec<Message>,
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
    pub tools: Arc<ToolRegistry>,
    pub event_tx: broadcast::Sender<AgentEvent>,
    pub max_turns: u32,
}

pub async fn run_agent_loop(ctx: &mut AgentLoopContext) -> Vec<Message> {
    let mut turn = 0;

    let _ = ctx.event_tx.send(AgentEvent::AgentStart);

    loop {
        turn += 1;
        if turn > ctx.max_turns {
            info!("max turns reached ({}), stopping", ctx.max_turns);
            break;
        }

        let _ = ctx.event_tx.send(AgentEvent::TurnStart { turn });

        // Build LLM request
        let tool_defs = ctx.tools.definitions();
        let request = crate::llm::LlmRequest {
            model: ctx.model.clone(),
            system_prompt: ctx.system_prompt.clone(),
            messages: ctx.messages.clone(),
            tools: tool_defs,
            max_tokens: Some(4096),
        };

        tracing::debug!("Calling LLM provider with model: {}", ctx.model);

        // Stream response
        let stream = match ctx.provider.stream(request).await {
            Ok(s) => s,
            Err(e) => {
                error!("LLM stream error: {}", e);
                let _ = ctx.event_tx.send(AgentEvent::AgentError(e.to_string()));
                break;
            }
        };

    let mut accumulated_text = String::new();
    let mut tool_calls_out: Vec<ToolCall> = Vec::new();
    let mut tool_call_args: Vec<String> = Vec::new();
    let mut stop_reason = StopReason::Stop;
        let mut usage = Usage { input: 0, output: 0, total: 0 };

        futures::pin_mut!(stream);
        while let Some(event) = stream.next().await {
            match event {
                LlmStreamEvent::Start => {
                    let _ = ctx.event_tx.send(AgentEvent::MessageStart);
                }
                LlmStreamEvent::TextDelta(delta) => {
                    let _ = ctx.event_tx.send(AgentEvent::MessageDelta { delta: delta.clone() });
                }
                LlmStreamEvent::ToolCallDelta { index, id, name, args_delta } => {
                    // Accumulate tool calls
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
                    let _ = ctx.event_tx.send(AgentEvent::MessageDelta { delta: format!("[tool_call:{}]", index) });
                }
                LlmStreamEvent::Done { text, tool_calls, stop_reason: sr, usage: u } => {
                    accumulated_text = text;
                    // Parse accumulated tool call args
                    for (i, tc) in tool_calls_out.iter_mut().enumerate() {
                        if i < tool_call_args.len() && !tool_call_args[i].is_empty() {
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&tool_call_args[i]) {
                                tc.arguments = v;
                            }
                        }
                    }
                    // Merge with final tool calls if available
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
                    stop_reason = sr;
                    usage = u;
                }
                LlmStreamEvent::Error(e) => {
                    let _ = ctx.event_tx.send(AgentEvent::AgentError(e));
                    break;
                }
            }
        }

        // Build assistant message
        let mut content = Vec::new();
        if !accumulated_text.is_empty() {
            content.push(ContentBlock::Text { text: accumulated_text });
        }
        content.extend(tool_calls_out.iter().map(|tc| ContentBlock::ToolCall(tc.clone())));

        let assistant_msg = AssistantMessage {
            content,
            usage: usage.clone(),
            stop_reason: stop_reason.clone(),
            model: ctx.model.clone(),
            timestamp: chrono::Utc::now(),
        };

        let _ = ctx.event_tx.send(AgentEvent::MessageEnd);
        ctx.messages.push(Message::Assistant(assistant_msg.clone()));

        // If no tool calls, we're done
        let has_tool_calls = !tool_calls_out.is_empty();
        if !has_tool_calls || matches!(stop_reason, StopReason::Error | StopReason::Aborted) {
            let _ = ctx.event_tx.send(AgentEvent::TurnEnd {
                message: assistant_msg,
                tool_results: Vec::new(),
            });
            break;
        }

        // Execute tool calls
        let mut tool_results = Vec::new();
        for tc in &tool_calls_out {
            tracing::debug!("Executing tool call: {} with args: {}", tc.name, tc.arguments);
            let _ = ctx.event_tx.send(AgentEvent::ToolExecutionStart {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                args: tc.arguments.clone(),
            });

            let result = ctx.tools.execute(&tc.name, tc.arguments.clone()).await;
            tracing::debug!("Tool result: {}", result.output);

            let tool_result_msg = ToolResultMessage {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                content: vec![text_content(result.output)],
                is_error: result.is_error,
                timestamp: chrono::Utc::now(),
            };

            let _ = ctx.event_tx.send(AgentEvent::ToolExecutionEnd {
                tool_call_id: tc.id.clone(),
                result: tool_result_msg.clone(),
            });

            tool_results.push(tool_result_msg.clone());
            ctx.messages.push(Message::ToolResult(tool_result_msg));
        }

        let _ = ctx.event_tx.send(AgentEvent::TurnEnd {
            message: assistant_msg,
            tool_results,
        });
    }

    let _ = ctx.event_tx.send(AgentEvent::AgentEnd {
        messages: ctx.messages.clone(),
    });

    ctx.messages.clone()
}
