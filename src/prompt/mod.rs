use std::sync::Arc;

use crate::error::AlfredError;
use crate::store::Store;

pub struct PromptLayers {
    pub system: String,
    pub user: String,
    pub memories: String,
}

pub fn load_prompt_layers(config: &crate::config::PromptConfig, store: &Arc<Store>) -> Result<PromptLayers, AlfredError> {
    let system = load_file(&config.system_prompt_file).unwrap_or_else(|_| {
        "You are Alfred, an AI assistant for automation, events, and notifications. You are not a coding agent.".into()
    });
    let user = load_file(&config.user_prompt_file).unwrap_or_default();
    let memories = load_memories(store);

    Ok(PromptLayers { system, user, memories })
}

pub fn assemble_system(layers: &PromptLayers) -> String {
    let mut prompt = layers.system.clone();
    if !layers.user.is_empty() {
        prompt.push_str("\n\n## User Context\n\n");
        prompt.push_str(&layers.user);
    }
    if !layers.memories.is_empty() {
        prompt.push_str("\n\n## User Memories\n\n");
        prompt.push_str(&layers.memories);
    }
    prompt
}

fn load_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}

fn load_memories(store: &Arc<Store>) -> String {
    match store.list_memories() {
        Ok(memories) if memories.is_empty() => String::new(),
        Ok(memories) => memories.iter().map(|m| format!("- {}", m.content)).collect::<Vec<_>>().join("\n"),
        Err(_) => String::new(),
    }
}
