//! Generates the Pi skills that back Alfred's agent-facing CLI commands.
//!
//! Adding a capability means adding a skill: a CLI command plus a `SKILL.md`
//! with Agent Skills frontmatter (`name`, `description`). Pi loads the
//! directory Alfred passes as `--skill` ([`Paths::skills_dir`]); Alfred
//! regenerates the files at startup so an upgrade never leaves a stale skill on
//! disk.
//!
//! Writes are idempotent — a file whose contents already match is left
//! untouched — and a failure is logged with its path and never fatal: the host
//! must still start.

use std::path::Path;

use crate::error::AlfredError;
use crate::paths::Paths;

/// Every Alfred-provided skill as `(directory name, SKILL.md contents)`.
pub const SKILLS: &[(&str, &str)] = &[
    ("todo", TODO_SKILL),
    ("webhook", WEBHOOK_SKILL),
    ("remember", REMEMBER_SKILL),
];

const TODO_SKILL: &str = "\
---
name: todo
description: Track, list, complete, and remove Alfred to-do items.
---

# Todo

Use `alfred todo` to keep the user's task list. Todos live in Alfred's
database; they are separate from memories.

- Add: `alfred todo add --title \"Water the plants\" --priority low`
  - `--priority` is `low`, `medium`, or `high` (default `medium`).
  - `--description` and `--due` are optional.
- List open todos: `alfred todo list`
- Complete one: `alfred todo complete <id>`
- Remove one: `alfred todo remove <id>`

`list` prints the id first; copy it exactly. Completed todos are hidden.
";

const WEBHOOK_SKILL: &str = "\
---
name: webhook
description: POST a JSON body to an allow-listed host with `alfred webhook send`.
---

# Webhook

Use `alfred webhook send` to call an external service. Alfred enforces the
allow-list in `[webhook] allowed_hosts`; a host that is not listed is refused
with `host not allowed: <host>` and no request is sent.

```bash
alfred webhook send https://example.com/hook --json '{\"event\":\"ping\"}'
```

- The URL must be absolute and use http or https.
- `--json` must be valid JSON; it is sent as the request body.
- The command prints the HTTP status and the response body.
- An empty `allowed_hosts` denies every host. Ask the user to add the host to
  the config rather than trying to work around a refusal.
";

const REMEMBER_SKILL: &str = "\
---
name: remember
description: Append a durable fact about the user to Alfred's memories file.
---

# Remember

Use `alfred remember \"<text>\"` to store a durable fact. Alfred appends one
line to `~/.alfred/config/memories.md`; that file is added to the system prompt
on every future run under `## User Memories`.

- One fact per call, phrased so it stands alone.
- Never store secrets, tokens, or passwords.
- Use it for preferences and recurring context, not transient task state —
  use `todo` for that.
";

/// Write every skill under [`Paths::skills_dir`].
///
/// Failures are logged with their path and do not abort startup.
pub fn generate_skills() {
    generate_skills_at(&Paths::skills_dir());
}

/// [`generate_skills`] against an explicit directory.
pub fn generate_skills_at(dir: &Path) {
    for (name, contents) in SKILLS {
        let path = dir.join(name).join("SKILL.md");
        if let Err(error) = write_if_changed(&path, contents) {
            tracing::error!(path = %path.display(), error = %error, "failed to write skill file");
        }
    }
}

/// Write `contents` to `path` only when they differ, creating parent directories.
fn write_if_changed(path: &Path, contents: &str) -> Result<(), AlfredError> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == contents {
            return Ok(());
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            AlfredError::Io(std::io::Error::new(
                error.kind(),
                format!(
                    "failed to create directory {} for {}: {error}",
                    parent.display(),
                    path.display()
                ),
            ))
        })?;
    }
    std::fs::write(path, contents).map_err(|error| {
        AlfredError::Io(std::io::Error::new(
            error.kind(),
            format!("failed to write {}: {error}", path.display()),
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::tempdir;

    /// Parse the `key: value` pairs between the leading `---` fences.
    fn frontmatter(contents: &str) -> HashMap<String, String> {
        let mut fields = HashMap::new();
        let mut lines = contents.lines();
        assert_eq!(
            lines.next(),
            Some("---"),
            "skill must open with frontmatter"
        );
        for line in lines {
            if line.trim() == "---" {
                return fields;
            }
            if let Some((key, value)) = line.split_once(':') {
                fields.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
        panic!("skill frontmatter is not closed");
    }

    #[test]
    fn writes_todo_webhook_and_remember_skills_with_frontmatter() {
        let dir = tempdir().unwrap();
        generate_skills_at(dir.path());

        let names: Vec<&str> = SKILLS.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec!["todo", "webhook", "remember"]);

        for (name, _) in SKILLS {
            let path = dir.path().join(name).join("SKILL.md");
            let contents = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("missing {}: {error}", path.display()));
            let fields = frontmatter(&contents);
            assert_eq!(fields.get("name").map(String::as_str), Some(*name));
            let description = fields.get("description").expect("description present");
            assert!(!description.is_empty(), "description must not be empty");
        }
    }

    #[test]
    fn generation_is_idempotent_and_restores_edits() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("todo").join("SKILL.md");

        generate_skills_at(dir.path());
        let first = std::fs::read_to_string(&path).unwrap();
        generate_skills_at(dir.path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);

        std::fs::write(&path, "hand edited").unwrap();
        generate_skills_at(dir.path());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    }

    #[test]
    fn write_failure_names_the_path() {
        let dir = tempdir().unwrap();
        // A regular file where the skill directory should be, so creating the
        // parent directory fails.
        let blocker = dir.path().join("todo");
        std::fs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("SKILL.md");

        let error = write_if_changed(&path, "contents").expect_err("must fail");
        assert!(
            error.to_string().contains("SKILL.md"),
            "error should name the path, got: {error}"
        );
    }
}
