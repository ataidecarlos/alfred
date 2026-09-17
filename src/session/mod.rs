use std::sync::Arc;

use chrono::Utc;
use rusqlite::params;
use uuid::Uuid;

use crate::error::AlfredError;
use crate::store::Store;
use crate::types::Message;

/// A session key derived from (user_id, channel).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionKey {
    pub user_id: String,
    pub channel: String,
}

impl SessionKey {
    pub fn new(user_id: impl Into<String>, channel: impl Into<String>) -> Self {
        Self { user_id: user_id.into(), channel: channel.into() }
    }

    /// String representation used as a lookup key.
    pub fn as_str(&self) -> String {
        format!("{}:{}", self.user_id, self.channel)
    }
}

/// A conversation session.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub workspace_id: String,
    pub user_id: String,
    pub channel: String,
    pub title: Option<String>,
    pub messages: Vec<Message>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Manages session lifecycle: resolve, create, save, load, list.
pub struct SessionManager {
    store: Arc<Store>,
    default_workspace_id: String,
}

impl SessionManager {
    pub fn new(store: Arc<Store>) -> Self {
        Self {
            store,
            default_workspace_id: "default".into(),
        }
    }

    /// Resolve an existing session or create a new one.
    ///
    /// On first call for a given key, checks for a legacy conversation in the
    /// `conversations` table and migrates it if found.
    pub fn resolve_or_create(&self, key: &SessionKey) -> Result<Session, AlfredError> {
        let conn = self.store.conn();

        // Check for existing session
        let existing: Option<(String, String, String, String, Option<String>, i64, i64)> = conn.query_row(
            "SELECT id, workspace_id, user_id, channel, title, created_at, updated_at
             FROM sessions WHERE user_id = ?1 AND channel = ?2",
            params![key.user_id, key.channel],
            |row| Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
            )),
        ).ok();

        if let Some((id, ws_id, user_id, channel, title, created_at, updated_at)) = existing {
            // Load messages for this session
            let mut stmt = conn.prepare(
                "SELECT content FROM messages WHERE session_id = ?1 ORDER BY timestamp"
            ).map_err(|e| AlfredError::Store(e))?;

            let messages: Vec<Message> = stmt.query_map(params![id], |row| {
                let json: String = row.get(0)?;
                Ok(json)
            }).map_err(|e| AlfredError::Store(e))?
            .filter_map(|r| r.ok())
            .filter_map(|json| serde_json::from_str(&json).ok())
            .collect();

            return Ok(Session {
                id, workspace_id: ws_id, user_id, channel, title,
                messages, created_at, updated_at,
            });
        }

        // No session found — check for legacy conversation and migrate
        let legacy_id = format!("{}:{}", key.user_id, key.channel);
        let legacy_messages: Vec<Message> = conn.query_row(
            "SELECT messages FROM conversations WHERE id = ?1",
            params![legacy_id],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

        // Delete the legacy row after migration
        if !legacy_messages.is_empty() {
            let _ = conn.execute(
                "DELETE FROM conversations WHERE id = ?1",
                params![legacy_id],
            );
        }

        // Create new session
        let now = Utc::now().timestamp();
        let session_id = Uuid::new_v4().to_string();

        conn.execute(
            "INSERT INTO sessions (id, workspace_id, user_id, channel, title, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6)",
            params![session_id, self.default_workspace_id, key.user_id, key.channel, now, now],
        ).map_err(|e| AlfredError::Store(e))?;

        Ok(Session {
            id: session_id,
            workspace_id: self.default_workspace_id.clone(),
            user_id: key.user_id.clone(),
            channel: key.channel.clone(),
            title: None,
            messages: legacy_messages,
            created_at: now,
            updated_at: now,
        })
    }

    /// Save a session's messages to the database.
    pub fn save(&self, session: &Session) -> Result<(), AlfredError> {
        let conn = self.store.conn();
        let now = Utc::now().timestamp();

        // Update session metadata
        conn.execute(
            "UPDATE sessions SET updated_at = ?1, title = ?2 WHERE id = ?3",
            params![now, session.title, session.id],
        ).map_err(|e| AlfredError::Store(e))?;

        // Delete existing messages for this session
        conn.execute(
            "DELETE FROM messages WHERE session_id = ?1",
            params![session.id],
        ).map_err(|e| AlfredError::Store(e))?;

        // Insert all messages
        for msg in &session.messages {
            let msg_id = Uuid::new_v4().to_string();
            let role = match msg {
                Message::User(_) => "user",
                Message::Assistant(_) => "assistant",
                Message::ToolResult(_) => "toolResult",
            };
            let content = serde_json::to_string(msg)
                .map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
            let ts = msg.timestamp().timestamp();

            conn.execute(
                "INSERT INTO messages (id, session_id, role, content, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![msg_id, session.id, role, content, ts],
            ).map_err(|e| AlfredError::Store(e))?;
        }

        Ok(())
    }

    /// List all sessions for a workspace (without loading messages).
    pub fn list(&self, workspace_id: &str) -> Result<Vec<Session>, AlfredError> {
        let conn = self.store.conn();
        let mut stmt = conn.prepare(
            "SELECT id, workspace_id, user_id, channel, title, created_at, updated_at
             FROM sessions WHERE workspace_id = ?1 ORDER BY updated_at DESC"
        ).map_err(|e| AlfredError::Store(e))?;

        let sessions: Vec<Session> = stmt.query_map(params![workspace_id], |row| {
            Ok(Session {
                id: row.get(0)?,
                workspace_id: row.get(1)?,
                user_id: row.get(2)?,
                channel: row.get(3)?,
                title: row.get(4)?,
                messages: Vec::new(),
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        }).map_err(|e| AlfredError::Store(e))?
        .filter_map(|r| r.ok())
        .collect();

        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_store() -> Arc<Store> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT 'default',
                path TEXT NOT NULL, created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL DEFAULT 'default',
                user_id TEXT NOT NULL, channel TEXT NOT NULL, title TEXT,
                created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY, session_id TEXT NOT NULL,
                role TEXT NOT NULL, content TEXT NOT NULL, timestamp INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY, user_id TEXT NOT NULL, connector TEXT NOT NULL,
                messages TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
            );"
        ).unwrap();
        Arc::new(Store::from_connection(conn))
    }

    #[test]
    fn test_session_key_as_str() {
        let key = SessionKey::new("user1", "api");
        assert_eq!(key.as_str(), "user1:api");
    }

    #[test]
    fn test_resolve_creates_new() {
        let store = test_store();
        let mgr = SessionManager::new(store);
        let key = SessionKey::new("u1", "api");

        let session = mgr.resolve_or_create(&key).unwrap();
        assert_eq!(session.user_id, "u1");
        assert_eq!(session.channel, "api");
        assert!(session.messages.is_empty());
        assert!(!session.id.is_empty());
    }

    #[test]
    fn test_resolve_returns_existing() {
        let store = test_store();
        let mgr = SessionManager::new(store);
        let key = SessionKey::new("u1", "api");

        let s1 = mgr.resolve_or_create(&key).unwrap();
        let s2 = mgr.resolve_or_create(&key).unwrap();
        assert_eq!(s1.id, s2.id);
    }

    #[test]
    fn test_save_and_load() {
        let store = test_store();
        let mgr = SessionManager::new(store);
        let key = SessionKey::new("u1", "api");

        let mut session = mgr.resolve_or_create(&key).unwrap();
        session.messages.push(Message::User(crate::types::UserMessage {
            content: vec![crate::types::Content::Text(crate::types::TextContent { text: "hello".into() })],
            timestamp: Utc::now(),
        }));
        mgr.save(&session).unwrap();

        let loaded = mgr.resolve_or_create(&key).unwrap();
        assert_eq!(loaded.messages.len(), 1);
    }

    #[test]
    fn test_list_sessions() {
        let store = test_store();
        let mgr = SessionManager::new(store);
        let key1 = SessionKey::new("u1", "api");
        let key2 = SessionKey::new("u2", "telegram");

        mgr.resolve_or_create(&key1).unwrap();
        mgr.resolve_or_create(&key2).unwrap();

        let sessions = mgr.list("default").unwrap();
        assert_eq!(sessions.len(), 2);
    }

    #[test]
    fn test_migrate_legacy() {
        let store = test_store();
        // Insert a legacy conversation
        {
            let conn = store.conn();
            let legacy = vec![
                Message::User(crate::types::UserMessage {
                    content: vec![crate::types::Content::Text(crate::types::TextContent { text: "old msg".into() })],
                    timestamp: Utc::now(),
                }),
            ];
            let json = serde_json::to_string(&legacy).unwrap();
            conn.execute(
                "INSERT INTO conversations (id, user_id, connector, messages, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params!["u1:telegram", "u1", "telegram", json, 0i64, 0i64],
            ).unwrap();
        }

        let mgr = SessionManager::new(store);
        let key = SessionKey::new("u1", "telegram");

        let session = mgr.resolve_or_create(&key).unwrap();
        assert_eq!(session.messages.len(), 1);

        // Legacy row should be deleted
        {
            let conn = mgr.store.conn();
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM conversations", [], |row| row.get(0)
            ).unwrap();
            assert_eq!(count, 0);
        }
    }
}
