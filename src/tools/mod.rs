pub mod webhook;
pub mod shell;
pub mod todo;

use std::sync::Arc;

use crate::agent::tool::ToolRegistry;
use crate::store::Store;

pub fn register_builtins(store: Arc<Store>) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    registry.register(Arc::new(webhook::WebhookTool::new()));
    registry.register(Arc::new(shell::ShellTool::new()));
    registry.register(Arc::new(todo::TodoTool::new(store)));
    registry
}
