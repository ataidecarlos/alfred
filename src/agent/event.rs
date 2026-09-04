use crate::types::{AssistantMessage, Message, ToolResultMessage};

#[derive(Debug, Clone)]
pub enum AgentEvent {
    AgentStart,
    AgentEnd { messages: Vec<Message> },
    AgentError(String),
    TurnStart { turn: u32 },
    TurnEnd { message: AssistantMessage, tool_results: Vec<ToolResultMessage> },
    MessageStart,
    MessageDelta { delta: String },
    MessageEnd,
    ToolExecutionStart { tool_call_id: String, tool_name: String, args: serde_json::Value },
    ToolExecutionEnd { tool_call_id: String, result: ToolResultMessage },
}
