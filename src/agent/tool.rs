use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::llm::ToolDefinition;

pub struct ToolOutput {
    pub output: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn success(text: impl Into<String>) -> Self {
        Self { output: text.into(), is_error: false }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self { output: text.into(), is_error: true }
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, args: Value) -> ToolOutput;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: HashMap::new() }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let def = tool.definition();
        self.tools.insert(def.name.clone(), tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn execute(&self, name: &str, args: Value) -> ToolOutput {
        match self.tools.get(name) {
            Some(tool) => tool.execute(args).await,
            None => ToolOutput::error(format!("unknown tool: {}", name)),
        }
    }
}
