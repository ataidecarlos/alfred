use std::path::Path;
use tracing::info;

use crate::error::AlfredError;

pub fn scaffold_vault(vault_path: &Path) -> Result<(), AlfredError> {
    info!("Scaffolding vault at: {}", vault_path.display());

    // Create the main directories
    let dirs = [
        "people",
        "memory",
        "memory/inbox",
        "todo",
    ];

    for dir in &dirs {
        let path = vault_path.join(dir);
        std::fs::create_dir_all(&path)?;
    }

    // Create README.md if it doesn't exist
    let readme_path = vault_path.join("README.md");
    if !readme_path.exists() {
        std::fs::write(&readme_path, README_TEMPLATE)?;
    }

    // Create memories.md (generic memories)
    let memories_path = vault_path.join("memories.md");
    if !memories_path.exists() {
        std::fs::write(&memories_path, MEMORIES_TEMPLATE)?;
    }

    // Create todo/todo.md (general todos)
    let todo_path = vault_path.join("todo").join("todo.md");
    if !todo_path.exists() {
        std::fs::write(&todo_path, TODO_TEMPLATE)?;
    }

    info!("Vault scaffolding complete");
    Ok(())
}

const README_TEMPLATE: &str = r#"# Alfred Memory Vault

Welcome to your AI-powered memory vault. This is Alfred's home for long-term memory, people profiles, and to-do lists.

## How It Works

Alfred reads and writes to this vault to remember things about you and your work. You can edit any file directly — Alfred will pick up changes on the next interaction.

## Structure

- **people/** — One file per user with memories and instructions
- **memory/** — General memories organized by topic
  - **inbox/** — New items awaiting review
- **memories.md** — Quick reference of important facts
- **todo/** — To-do lists organized by user

## Adding Memories

1. Ask Alfred to remember something
2. Or edit `memories.md` directly
3. Or add a file to `memory/` with YAML frontmatter

## Example Memory File

```yaml
---
title: "Project deadline"
type: fact
status: active
created: 2026-09-17
---

The project deadline is October 15th.
```

## People

Add a file in `people/` for each person Alfred should know about. Use their name as the filename (e.g., `john.md`).

## Tips

- Files are the source of truth — Alfred always refers to them when in doubt
- Use Obsidian to browse and edit your vault
- Tags and frontmatter help Alfred categorize memories
"#;

const MEMORIES_TEMPLATE: &str = r#"# Important Memories

Quick reference of important facts Alfred should remember.

## Work

## Personal

## Technical
"#;

const TODO_TEMPLATE: &str = r#"# To-Do List

Active to-do items. Mark items as complete with `- [x]`.

## High Priority

## Medium Priority

## Low Priority
"#;
