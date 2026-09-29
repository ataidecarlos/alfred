use crate::error::AlfredError;

pub struct PromptLayers {
    pub system: String,
    pub user: String,
}

pub fn load_prompt_layers(config: &crate::config::PromptConfig) -> Result<PromptLayers, AlfredError> {
    let system = load_file(&config.system_prompt_file).unwrap_or_else(|_| {
        "You are Alfred, a personal assistant for automation, events, and notifications. You are not a coding agent.".into()
    });
    let user = load_file(&config.user_prompt_file).unwrap_or_default();

    Ok(PromptLayers { system, user })
}

pub fn assemble_system(layers: &PromptLayers) -> String {
    let mut prompt = layers.system.clone();
    if !layers.user.is_empty() {
        prompt.push_str("\n\n## User Context\n\n");
        prompt.push_str(&layers.user);
    }
    prompt
}

fn load_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}
