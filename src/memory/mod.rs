//! File-backed memories.
//!
//! A memory is one non-empty, non-heading line in
//! `~/.alfred/config/memories.md`. There is no database index and no
//! frontmatter: the file is the source of truth and can be edited by hand. The
//! SQLite-indexed vault and frontmatter parsing were removed as part of the
//! prune to the Pi-host goal.
//!
//! Every operation has a `*_at(path)` form that acts on an explicit file. The
//! unsuffixed form targets [`Paths::memories_file`]. The `*_at` forms exist so
//! tests and embedders can work in a temporary directory and never touch the
//! real `~/.alfred/`.

use std::path::Path;

use crate::error::AlfredError;
use crate::paths::Paths;

/// Number of leading characters of a memory line that form its slug.
pub const SLUG_LEN: usize = 40;

/// Read the raw memories file.
///
/// A missing file is not an error: it reads as the empty string, so a fresh
/// install simply has no memories.
pub fn read_memories() -> Result<String, AlfredError> {
    read_memories_at(&Paths::memories_file())
}

/// [`read_memories`] against an explicit file.
pub fn read_memories_at(path: &Path) -> Result<String, AlfredError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(e) => Err(io_error("read", path, e)),
    }
}

/// Append one memory line, creating the file (and its directory) if needed.
pub fn append_memory(text: &str) -> Result<(), AlfredError> {
    append_memory_at(&Paths::memories_file(), text)
}

/// [`append_memory`] against an explicit file.
pub fn append_memory_at(path: &Path, text: &str) -> Result<(), AlfredError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(());
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| io_error("create directory", parent, e))?;
    }

    let mut content = read_memories_at(path)?;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(text);
    content.push('\n');

    std::fs::write(path, content).map_err(|e| io_error("write", path, e))
}

/// List memories as `(slug, text)` in file order.
///
/// One entry per non-empty, non-heading line: blank lines and Markdown headings
/// (`# ...`) are skipped. A slug derives from the first [`SLUG_LEN`] characters
/// of the line (see [`slug_for`]). A read failure yields an empty list, because
/// this is a best-effort query; writes are never best-effort.
pub fn list_memories() -> Vec<(String, String)> {
    list_memories_at(&Paths::memories_file())
}

/// [`list_memories`] against an explicit file.
pub fn list_memories_at(path: &Path) -> Vec<(String, String)> {
    let content = read_memories_at(path).unwrap_or_default();
    parse_memories(&content)
}

/// Delete every memory whose slug matches, rewriting the file.
///
/// Deleting a slug that is not present, or on a file that does not exist,
/// succeeds and changes nothing.
pub fn delete_memory(slug: &str) -> Result<(), AlfredError> {
    delete_memory_at(&Paths::memories_file(), slug)
}

/// [`delete_memory`] against an explicit file.
pub fn delete_memory_at(path: &Path, slug: &str) -> Result<(), AlfredError> {
    if !path.exists() {
        return Ok(());
    }

    let content = read_memories_at(path)?;
    let kept: Vec<&str> = content
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.is_empty() || trimmed.starts_with('#') || slug_for(trimmed) != slug
        })
        .collect();

    let mut rewritten = kept.join("\n");
    if !rewritten.is_empty() {
        rewritten.push('\n');
    }
    std::fs::write(path, rewritten).map_err(|e| io_error("write", path, e))
}

/// Parse raw file content into `(slug, text)` entries.
fn parse_memories(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| (slug_for(line), line.to_string()))
        .collect()
}

/// Derive a slug from the first [`SLUG_LEN`] characters of a memory line.
///
/// Runs of non-alphanumeric characters collapse to a single `-`, the result is
/// lowercased, and leading/trailing dashes are trimmed. `"Deploy at 3pm!"`
/// becomes `"deploy-at-3pm"`.
pub fn slug_for(line: &str) -> String {
    let prefix: String = line.trim().chars().take(SLUG_LEN).collect();
    let mut slug = String::new();
    let mut pending_dash = false;
    for ch in prefix.chars() {
        if ch.is_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.extend(ch.to_lowercase());
        } else {
            pending_dash = true;
        }
    }
    slug
}

/// Build an [`AlfredError::Io`] that names the operation and the file path.
fn io_error(operation: &str, path: &Path, source: std::io::Error) -> AlfredError {
    AlfredError::Io(std::io::Error::new(
        source.kind(),
        format!("failed to {operation} {}: {source}", path.display()),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_list_delete_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config").join("memories.md");

        assert!(
            list_memories_at(&path).is_empty(),
            "missing file lists as empty"
        );

        append_memory_at(&path, "First memory").unwrap();
        append_memory_at(&path, "Second memory").unwrap();
        assert!(path.exists(), "first append creates the file");

        let listed = list_memories_at(&path);
        assert_eq!(
            listed,
            vec![
                ("first-memory".to_string(), "First memory".to_string()),
                ("second-memory".to_string(), "Second memory".to_string()),
            ]
        );

        delete_memory_at(&path, "first-memory").unwrap();
        assert_eq!(
            list_memories_at(&path),
            vec![("second-memory".to_string(), "Second memory".to_string())]
        );

        // Deleting an unknown slug is a no-op, not an error.
        delete_memory_at(&path, "not-there").unwrap();
        assert_eq!(list_memories_at(&path).len(), 1);
    }

    #[test]
    fn missing_file_reads_as_empty_and_is_created_on_first_write() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config").join("memories.md");
        assert!(!path.exists());

        assert_eq!(read_memories_at(&path).unwrap(), "");
        assert!(list_memories_at(&path).is_empty());

        append_memory_at(&path, "Remember the milk").unwrap();
        assert!(path.exists(), "append creates a missing memories.md");
        assert_eq!(read_memories_at(&path).unwrap(), "Remember the milk\n");
        assert_eq!(
            list_memories_at(&path),
            vec![(
                "remember-the-milk".to_string(),
                "Remember the milk".to_string()
            )]
        );
    }

    #[test]
    fn list_skips_blank_lines_and_headings() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("memories.md");
        std::fs::write(
            &path,
            "# Important Memories\n\n## Work\nDeploy on Friday\n\n## Personal\nCalls mom on Sunday\n",
        )
        .unwrap();

        assert_eq!(
            list_memories_at(&path),
            vec![
                (
                    "deploy-on-friday".to_string(),
                    "Deploy on Friday".to_string()
                ),
                (
                    "calls-mom-on-sunday".to_string(),
                    "Calls mom on Sunday".to_string()
                ),
            ]
        );
    }

    #[test]
    fn slug_derives_from_the_first_40_characters() {
        // Two lines that agree on their first 40 characters share a slug.
        let prefix = "0123456789012345678901234567890123456789";
        let a = format!("{prefix} alpha");
        let b = format!("{prefix} beta");
        assert_eq!(slug_for(&a), slug_for(&b));
        assert_eq!(slug_for(&a).chars().count(), SLUG_LEN);

        assert_eq!(slug_for("Deploy at 3pm!"), "deploy-at-3pm");
        assert_eq!(slug_for("  multiple   spaces  "), "multiple-spaces");
    }

    #[test]
    fn delete_keeps_headings_and_other_lines() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("memories.md");
        std::fs::write(
            &path,
            "# Memories\nkeep this\nDelete me\n# Another heading\n",
        )
        .unwrap();

        delete_memory_at(&path, "delete-me").unwrap();
        let content = read_memories_at(&path).unwrap();
        assert!(content.contains("keep this"));
        assert!(content.contains("# Memories"));
        assert!(content.contains("# Another heading"));
        assert!(!content.contains("Delete me"));
    }

    #[test]
    fn write_failure_returns_io_error_naming_the_path() {
        let dir = tempdir().unwrap();
        // A regular file where the parent directory should be, so creating the
        // parent directory fails.
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, "not a directory").unwrap();
        let path = blocker.join("memories.md");

        let error = append_memory_at(&path, "anything").unwrap_err();
        match error {
            AlfredError::Io(source) => {
                let message = source.to_string();
                assert!(
                    message.contains("blocker"),
                    "error should name the failing path, got: {message}"
                );
            }
            other => panic!("expected AlfredError::Io, got {other:?}"),
        }
    }
}
