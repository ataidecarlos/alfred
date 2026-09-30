//! System prompt assembly.
//!
//! [`assemble_system`] produces the exact string Alfred passes to Pi as
//! `--system-prompt`. That flag *replaces* Pi's coding-harness persona; Alfred
//! never appends to it, so Pi does not behave like a coding agent.
//!
//! The assembled prompt is the system prompt, then the `## User Context`
//! section (the user prompt), then the `## User Memories` section
//! ([`crate::memory::read_memories`]). Empty sections are omitted.

use std::path::Path;

use crate::error::AlfredError;
use crate::memory;
use crate::paths::Paths;

/// Used when no system prompt file exists yet.
const DEFAULT_SYSTEM_PROMPT: &str =
    "You are Alfred, a personal assistant for automation, events, and notifications. You are not a coding agent.";

/// The three layers that make up the assembled system prompt.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PromptLayers {
    /// Base persona. Replaces Pi's coding persona.
    pub system: String,
    /// Per-user instructions, rendered under `## User Context`.
    pub user: String,
    /// File-backed memories, rendered under `## User Memories`.
    pub memories: String,
}

/// Load the prompt layers from `config`, reading memories from
/// [`Paths::memories_file`].
pub fn load_prompt_layers(
    config: &crate::config::PromptConfig,
) -> Result<PromptLayers, AlfredError> {
    load_prompt_layers_at(config, &Paths::memories_file())
}

/// [`load_prompt_layers`] with an explicit memories file.
///
/// A missing system prompt file falls back to the built-in persona; a missing
/// user prompt or memories file reads as empty.
pub fn load_prompt_layers_at(
    config: &crate::config::PromptConfig,
    memories_path: &Path,
) -> Result<PromptLayers, AlfredError> {
    let system =
        load_file(&config.system_prompt_file).unwrap_or_else(|_| DEFAULT_SYSTEM_PROMPT.to_string());
    let user = load_file(&config.user_prompt_file).unwrap_or_default();
    let memories = memory::read_memories_at(memories_path)?;

    Ok(PromptLayers {
        system,
        user,
        memories,
    })
}

/// Assemble the system prompt passed to Pi as `--system-prompt`.
///
/// Sections appear in this order: system prompt, `## User Context`,
/// `## User Memories`. Empty sections are omitted.
pub fn assemble_system(layers: &PromptLayers) -> String {
    let mut sections: Vec<String> = Vec::new();

    let system = layers.system.trim();
    if !system.is_empty() {
        sections.push(system.to_string());
    }

    let user = layers.user.trim();
    if !user.is_empty() {
        sections.push(format!("## User Context\n\n{user}"));
    }

    let memories = layers.memories.trim();
    if !memories.is_empty() {
        sections.push(format!("## User Memories\n\n{memories}"));
    }

    sections.join("\n\n")
}

fn load_file(path: &str) -> Result<String, std::io::Error> {
    std::fs::read_to_string(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{PiConfig, PromptConfig};
    use crate::pi::PiInvocation;
    use tempfile::tempdir;

    fn layers(system: &str, user: &str, memories: &str) -> PromptLayers {
        PromptLayers {
            system: system.to_string(),
            user: user.to_string(),
            memories: memories.to_string(),
        }
    }

    #[test]
    fn assemble_system_emits_sections_in_documented_order() {
        let assembled = assemble_system(&layers(
            "You are Alfred.",
            "Reply in Portuguese.",
            "Likes tea",
        ));
        assert_eq!(
            assembled,
            "You are Alfred.\n\n## User Context\n\nReply in Portuguese.\n\n## User Memories\n\nLikes tea"
        );
    }

    #[test]
    fn assemble_system_omits_empty_sections() {
        assert_eq!(
            assemble_system(&layers("system only", "", "")),
            "system only"
        );
        assert_eq!(
            assemble_system(&layers("", "user only", "")),
            "## User Context\n\nuser only"
        );
        assert_eq!(
            assemble_system(&layers("", "", "memory only")),
            "## User Memories\n\nmemory only"
        );
        assert_eq!(assemble_system(&PromptLayers::default()), "");
    }

    #[test]
    fn load_prompt_layers_reads_system_user_and_memories() {
        let dir = tempdir().unwrap();
        let system_path = dir.path().join("system.md");
        let user_path = dir.path().join("user.md");
        let memories_path = dir.path().join("memories.md");
        std::fs::write(&system_path, "You are Alfred.").unwrap();
        std::fs::write(&user_path, "Reply in Portuguese.").unwrap();
        std::fs::write(&memories_path, "Likes tea\n").unwrap();

        let config = PromptConfig {
            system_prompt_file: system_path.to_string_lossy().into_owned(),
            user_prompt_file: user_path.to_string_lossy().into_owned(),
        };

        let loaded = load_prompt_layers_at(&config, &memories_path).unwrap();
        assert_eq!(
            loaded,
            layers("You are Alfred.", "Reply in Portuguese.", "Likes tea\n")
        );
    }

    #[test]
    fn load_prompt_layers_falls_back_when_files_are_missing() {
        let dir = tempdir().unwrap();
        let config = PromptConfig {
            system_prompt_file: dir
                .path()
                .join("absent-system.md")
                .to_string_lossy()
                .into_owned(),
            user_prompt_file: dir
                .path()
                .join("absent-user.md")
                .to_string_lossy()
                .into_owned(),
        };

        let loaded =
            load_prompt_layers_at(&config, &dir.path().join("absent-memories.md")).unwrap();
        assert_eq!(loaded.system, DEFAULT_SYSTEM_PROMPT);
        assert!(loaded.user.is_empty());
        assert!(loaded.memories.is_empty());

        // A missing memories file must not add a `## User Memories` section.
        assert!(!assemble_system(&loaded).contains("## User Memories"));
    }

    #[test]
    fn assembled_prompt_replaces_pi_coding_prompt() {
        let assembled = assemble_system(&layers("You are Alfred.", "user context", "a memory"));
        let invocation = PiInvocation::job(&PiConfig::default(), assembled.clone());

        let args: Vec<String> = invocation
            .command()
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let index = args
            .iter()
            .position(|arg| arg == "--system-prompt")
            .expect("--system-prompt must be present");
        assert_eq!(args[index + 1], assembled);
        assert!(
            !args.iter().any(|arg| arg == "--append-system-prompt"),
            "the prompt is a replace, never an append: {args:?}"
        );
    }
}
