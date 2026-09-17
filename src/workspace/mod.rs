use std::path::PathBuf;

/// A workspace represents an isolated environment for an agent instance.
///
/// Each workspace has its own memory, sessions, and skills directories.
/// The default workspace lives at `~/.local/share/alfred/workspace/`.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}

impl Workspace {
    /// Create a workspace from an existing path.
    pub fn new(id: impl Into<String>, name: impl Into<String>, path: PathBuf) -> Self {
        Self { id: id.into(), name: name.into(), path }
    }

    /// Memory directory for this workspace.
    pub fn memory_dir(&self) -> PathBuf {
        self.path.join("memory")
    }

    /// Skills directory for this workspace.
    pub fn skills_dir(&self) -> PathBuf {
        self.path.join("skills")
    }

    /// Ensure all workspace directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.memory_dir())?;
        std::fs::create_dir_all(self.skills_dir())?;
        Ok(())
    }
}

/// Manages workspace lifecycle: resolve, create, list.
pub struct WorkspaceManager {
    base_dir: PathBuf,
}

impl WorkspaceManager {
    /// Create a workspace manager rooted at the given base directory.
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Create a workspace manager using the default XDG path.
    pub fn default_path() -> PathBuf {
        crate::paths::Paths::workspace_dir()
    }

    /// Resolve a workspace by ID, or return the default workspace.
    pub fn resolve(&self, id: &str) -> Workspace {
        let path = self.base_dir.join(id);
        if path.exists() {
            Workspace::new(id, id, path)
        } else {
            self.default_workspace()
        }
    }

    /// Return the default workspace ("default").
    pub fn default_workspace(&self) -> Workspace {
        let path = self.base_dir.join("default");
        Workspace::new("default", "default", path)
    }

    /// Create a new workspace with the given name.
    pub fn create(&self, name: &str) -> Result<Workspace, std::io::Error> {
        let id = sanitize_id(name);
        let path = self.base_dir.join(&id);
        std::fs::create_dir_all(&path)?;
        let ws = Workspace::new(&id, name, path);
        ws.ensure_dirs()?;
        Ok(ws)
    }

    /// List all workspaces in the base directory.
    pub fn list(&self) -> Vec<Workspace> {
        let mut workspaces = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.base_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    workspaces.push(Workspace::new(&name, &name, entry.path()));
                }
            }
        }
        workspaces
    }
}

/// Sanitize a name into a valid directory ID.
fn sanitize_id(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect::<String>()
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workspace_path_helpers() {
        let ws = Workspace::new("w1", "test", PathBuf::from("/tmp/test-workspace"));
        assert_eq!(ws.memory_dir(), PathBuf::from("/tmp/test-workspace/memory"));
        assert_eq!(ws.skills_dir(), PathBuf::from("/tmp/test-workspace/skills"));
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(sanitize_id("My Workspace"), "my-workspace");
        assert_eq!(sanitize_id("hello/world"), "hello-world");
        assert_eq!(sanitize_id("already-ok"), "already-ok");
    }

    #[test]
    fn test_workspace_manager_default() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorkspaceManager::new(tmp.path().to_path_buf());

        let ws = mgr.default_workspace();
        assert_eq!(ws.id, "default");
        assert_eq!(ws.path, tmp.path().join("default"));
    }

    #[test]
    fn test_workspace_manager_create_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorkspaceManager::new(tmp.path().to_path_buf());

        let ws = mgr.create("My Project").unwrap();
        assert_eq!(ws.id, "my-project");
        assert!(ws.path.exists());
        assert!(ws.memory_dir().exists());

        let workspaces = mgr.list();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].id, "my-project");
    }

    #[test]
    fn test_workspace_manager_resolve_existing() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorkspaceManager::new(tmp.path().to_path_buf());

        mgr.create("project-a").unwrap();
        let ws = mgr.resolve("project-a");
        assert_eq!(ws.id, "project-a");
    }

    #[test]
    fn test_workspace_manager_resolve_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let mgr = WorkspaceManager::new(tmp.path().to_path_buf());

        let ws = mgr.resolve("nonexistent");
        assert_eq!(ws.id, "default");
    }
}
