use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AlfredError;

#[derive(Debug, Serialize, Deserialize)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub description: String,
    pub priority: String,
    pub completed: bool,
    pub due_date: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub content: String,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn new(path: &Path) -> Result<Self, AlfredError> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS todos (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT DEFAULT '',
                priority TEXT NOT NULL DEFAULT 'medium',
                completed INTEGER NOT NULL DEFAULT 0,
                due_date TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                user_id TEXT NOT NULL,
                connector TEXT NOT NULL,
                messages TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS scheduled_tasks (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                cron_expr TEXT NOT NULL,
                prompt TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_run INTEGER,
                created_at INTEGER NOT NULL
            );"
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn add_todo(&self, title: &str, description: &str, priority: &str, due_date: &str) -> Result<String, AlfredError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "INSERT INTO todos (id, title, description, priority, due_date, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, title, description, priority, if due_date.is_empty() { None } else { Some(due_date) }, now, now],
        )?;
        Ok(id)
    }

    pub fn list_todos(&self) -> Result<Vec<Todo>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, title, description, priority, completed, due_date FROM todos WHERE completed = 0 ORDER BY CASE priority WHEN 'high' THEN 0 WHEN 'medium' THEN 1 WHEN 'low' THEN 2 END, created_at")?;
        let todos = stmt.query_map([], |row| {
            Ok(Todo {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                priority: row.get(3)?,
                completed: row.get::<_, i32>(4)? != 0,
                due_date: row.get(5)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(todos)
    }

    pub fn complete_todo(&self, id: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("UPDATE todos SET completed = 1, updated_at = ?1 WHERE id = ?2", params![now, id])?;
        Ok(())
    }

    pub fn delete_todo(&self, id: &str) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn update_todo(&self, id: &str, title: Option<&str>, description: Option<&str>, priority: Option<&str>, due_date: Option<&str>) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        if let Some(t) = title {
            conn.execute("UPDATE todos SET title = ?1, updated_at = ?2 WHERE id = ?3", params![t, now, id])?;
        }
        if let Some(d) = description {
            conn.execute("UPDATE todos SET description = ?1, updated_at = ?2 WHERE id = ?3", params![d, now, id])?;
        }
        if let Some(p) = priority {
            conn.execute("UPDATE todos SET priority = ?1, updated_at = ?2 WHERE id = ?3", params![p, now, id])?;
        }
        if let Some(d) = due_date {
            conn.execute("UPDATE todos SET due_date = ?1, updated_at = ?2 WHERE id = ?3", params![d, now, id])?;
        }
        Ok(())
    }

    pub fn add_memory(&self, content: &str) -> Result<String, AlfredError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("INSERT INTO memories (id, content, created_at) VALUES (?1, ?2, ?3)", params![id, content, now])?;
        Ok(id)
    }

    pub fn list_memories(&self) -> Result<Vec<Memory>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, content FROM memories ORDER BY created_at")?;
        let memories = stmt.query_map([], |row| {
            Ok(Memory { id: row.get(0)?, content: row.get(1)? })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(memories)
    }

    pub fn delete_memory(&self, id: &str) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("DELETE FROM memories WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn save_conversation(&self, user_id: &str, connector: &str, messages: &[crate::types::Message]) -> Result<(), AlfredError> {
        let id = format!("{}:{}", user_id, connector);
        let json = serde_json::to_string(messages).map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "INSERT OR REPLACE INTO conversations (id, user_id, connector, messages, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, user_id, connector, json, now, now],
        )?;
        Ok(())
    }

    pub fn load_conversation(&self, user_id: &str, connector: &str) -> Result<Option<Vec<crate::types::Message>>, AlfredError> {
        let id = format!("{}:{}", user_id, connector);
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT messages FROM conversations WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], |row| row.get::<_, String>(0))?;
        match rows.next() {
            Some(Ok(json)) => {
                let messages: Vec<crate::types::Message> = serde_json::from_str(&json)
                    .map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
                Ok(Some(messages))
            }
            _ => Ok(None),
        }
    }
}
