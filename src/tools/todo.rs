use std::sync::Arc;

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::tool::{Tool, ToolOutput};
use crate::llm::ToolDefinition;
use crate::store::Store;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TodoArgs {
    /// Action: list, add, update, delete, complete
    pub action: String,
    /// Todo ID (for update/delete/complete)
    pub id: Option<String>,
    /// Title (for add/update)
    pub title: Option<String>,
    /// Description (for add/update)
    pub description: Option<String>,
    /// Priority: low, medium, high (for add/update)
    pub priority: Option<String>,
    /// Due date in ISO 8601 format (for add/update)
    pub due: Option<String>,
}

pub struct TodoTool {
    store: Arc<Store>,
}

impl TodoTool {
    pub fn new(store: Arc<Store>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn definition(&self) -> ToolDefinition {
        let schema = schemars::schema_for!(TodoArgs);
        ToolDefinition {
            name: "todo".into(),
            description: "Manage to-do items. Actions: list, add, update, delete, complete.".into(),
            parameters: schema,
        }
    }

    async fn execute(&self, args: serde_json::Value) -> ToolOutput {
        let args: TodoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolOutput::error(format!("invalid arguments: {}", e)),
        };

        match args.action.as_str() {
            "list" => {
                match self.store.list_todos() {
                    Ok(todos) => {
                        if todos.is_empty() {
                            return ToolOutput::success("No todos found.");
                        }
                        let formatted: Vec<String> = todos.iter().map(|t| {
                            let status = if t.completed { "x" } else { " " };
                            let priority = &t.priority;
                            let due = t.due_date.as_deref().unwrap_or("no due date");
                            format!("[{}] {} ({}) - due: {}", status, t.title, priority, due)
                        }).collect();
                        ToolOutput::success(formatted.join("\n"))
                    }
                    Err(e) => ToolOutput::error(format!("failed to list todos: {}", e)),
                }
            }
            "add" => {
                let title = match &args.title {
                    Some(t) => t.clone(),
                    None => return ToolOutput::error(String::from("title is required for add")),
                };
                let priority = args.priority.unwrap_or_else(|| "medium".into());
                let description = args.description.unwrap_or_default();
                let due_date = args.due.unwrap_or_default();

                match self.store.add_todo(&title, &description, &priority, &due_date) {
                    Ok(id) => ToolOutput::success(format!("Todo created with ID: {}", id)),
                    Err(e) => ToolOutput::error(format!("failed to add todo: {}", e)),
                }
            }
            "complete" => {
                let id = match &args.id {
                    Some(i) => i.clone(),
                    None => return ToolOutput::error(String::from("id is required for complete")),
                };
                match self.store.complete_todo(&id) {
                    Ok(()) => ToolOutput::success(format!("Todo {} marked as complete.", id)),
                    Err(e) => ToolOutput::error(format!("failed to complete todo: {}", e)),
                }
            }
            "delete" => {
                let id = match &args.id {
                    Some(i) => i.clone(),
                    None => return ToolOutput::error(String::from("id is required for delete")),
                };
                match self.store.delete_todo(&id) {
                    Ok(()) => ToolOutput::success(format!("Todo {} deleted.", id)),
                    Err(e) => ToolOutput::error(format!("failed to delete todo: {}", e)),
                }
            }
            "update" => {
                let id = match &args.id {
                    Some(i) => i.clone(),
                    None => return ToolOutput::error(String::from("id is required for update")),
                };
                match self.store.update_todo(&id, args.title.as_deref(), args.description.as_deref(), args.priority.as_deref(), args.due.as_deref()) {
                    Ok(()) => ToolOutput::success(format!("Todo {} updated.", id)),
                    Err(e) => ToolOutput::error(format!("failed to update todo: {}", e)),
                }
            }
            _ => ToolOutput::error(format!("unknown action: {}", args.action)),
        }
    }
}
