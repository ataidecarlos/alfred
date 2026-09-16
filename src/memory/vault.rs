use std::path::Path;
use tracing::info;

use crate::error::AlfredError;

pub fn scaffold_vault(vault_path: &Path) -> Result<(), AlfredError> {
    info!("Scaffolding vault at: {}", vault_path.display());

    let dirs = [
        "_templates",
        "_attachments",
        "_config",
        "inbox",
        "preferences",
        "facts",
        "decisions",
        "lessons",
        "action-items",
        "daily",
    ];

    for dir in &dirs {
        let path = vault_path.join(dir);
        std::fs::create_dir_all(&path)?;
    }

    let topics_path = vault_path.join("_config/topics.md");
    if !topics_path.exists() {
        std::fs::write(&topics_path, TOPICS_TEMPLATE)?;
    }

    let templates = [
        ("_templates/memory.md", MEMORY_TEMPLATE),
        ("_templates/decision.md", DECISION_TEMPLATE),
        ("_templates/moc.md", MOC_TEMPLATE),
    ];

    for (name, content) in &templates {
        let path = vault_path.join(name);
        if !path.exists() {
            std::fs::write(&path, content)?;
        }
    }

    let index_path = vault_path.join("index.md");
    if !index_path.exists() {
        std::fs::write(&index_path, INDEX_TEMPLATE)?;
    }

    let base_path = vault_path.join("alfred-index.base");
    if !base_path.exists() {
        std::fs::write(&base_path, BASE_TEMPLATE)?;
    }

    let categories = ["preferences", "facts", "decisions", "lessons", "action-items"];
    for category in &categories {
        let index_path = vault_path.join(category).join("_index.md");
        if !index_path.exists() {
            let content = format!("# {}\n\nNo memories yet.", capitalize(category));
            std::fs::write(&index_path, content)?;
        }
    }

    info!("Vault scaffolding complete");
    Ok(())
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

const TOPICS_TEMPLATE: &str = r#"# Topics

Controlled vocabulary for memory categorization.

## Communication
- language
- email
- messaging
- preferences

## Work
- projects
- tasks
- deadlines
- meetings

## Technical
- programming
- tools
- infrastructure
- debugging

## Personal
- preferences
- habits
- goals
- health

## People
- contacts
- relationships
- teams

## Projects
- active
- completed
- ideas
"#;

const MEMORY_TEMPLATE: &str = r#"---
title: ""
type: fact
topics: []
status: active
created: {{date}}
updated: {{date}}
source: conversation
aliases: []
tags:
  - memory
retrieval_count: 0
last_retrieved: null
importance: medium
distilled_from: null
---

# Content

"#;

const DECISION_TEMPLATE: &str = r#"---
title: ""
type: decision
topics: []
status: active
created: {{date}}
updated: {{date}}
source: conversation
aliases: []
tags:
  - memory
  - decision
retrieval_count: 0
last_retrieved: null
importance: high
distilled_from: null
alternatives_rejected: []
rationale: ""
---

# Decision

## Context

## Alternatives Considered

## Chosen Approach

## Rationale
"#;

const MOC_TEMPLATE: &str = r#"---
title: ""
type: moc
---

# {{title}}

```dataview
TABLE file.ctime AS Created, file.mtime AS Updated
FROM ""
WHERE contains(topics, this.file.name)
SORT file.mtime DESC
```
"#;

const INDEX_TEMPLATE: &str = r#"---
title: Alfred Memory Vault
---

# Alfred Memory Vault

Welcome to your AI-powered memory vault.

## Categories

- [[preferences/_index|Preferences]]
- [[facts/_index|Facts]]
- [[decisions/_index|Decisions]]
- [[lessons/_index|Lessons]]
- [[action-items/_index|Action Items]]

## Tools

- Open `alfred-index.base` for table views of all memories
"#;

const BASE_TEMPLATE: &str = r#"---
title: Alfred Index
---

# All Memories

```dataview
TABLE title AS Title, type AS Type, status AS Status, retrieval_count AS "Retrievals"
FROM ""
WHERE type != "moc"
SORT file.mtime DESC
```

# Recent Memories (Last 30 Days)

```dataview
TABLE title AS Title, type AS Type, file.mtime AS Updated
FROM ""
WHERE type != "moc" AND file.mtime >= date(today) - dur(30 days)
SORT file.mtime DESC
```
"#;
