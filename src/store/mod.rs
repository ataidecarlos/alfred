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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MemoryRecord {
    pub id: String,
    pub path: String,
    pub title: String,
    pub mem_type: String,
    pub status: String,
    pub topics: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub retrieval_count: i64,
    pub last_retrieved: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DistilledInsight {
    pub id: String,
    pub session_id: String,
    pub complexity: Option<String>,
    pub tools: Option<String>,
    pub files: Option<String>,
    pub outcome: Option<String>,
    pub summary: Option<String>,
    pub basic_lesson: Option<String>,
    pub detailed_lessons: Option<String>,
    pub patterns: Option<String>,
    pub principles: Option<String>,
    pub status: String,
    pub memory_id: Option<String>,
    pub created_at: i64,
}

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    /// Access the raw connection. Used by SessionManager for schema migrations
    /// and direct queries that the Store API doesn't cover yet.
    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().map_err(|e| {
            // Poisoned mutex — this is a programming error, not a runtime one.
            // Unwrap to surface it during development.
            panic!("Store mutex poisoned: {}", e)
        }).unwrap()
    }

    /// Create a Store from an existing connection (for testing).
    pub fn from_connection(conn: Connection) -> Self {
        Self { conn: Mutex::new(conn) }
    }
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
            );
            CREATE TABLE IF NOT EXISTS distillation_backlog (
                session_id TEXT PRIMARY KEY,
                status TEXT NOT NULL DEFAULT 'pending',
                complexity_score TEXT,
                complexity_confidence REAL,
                npu_distilled_at INTEGER,
                llm_distilled_at INTEGER,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS distilled_insights (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                complexity TEXT,
                tools TEXT,
                files TEXT,
                outcome TEXT,
                summary TEXT,
                basic_lesson TEXT,
                detailed_lessons TEXT,
                patterns TEXT,
                principles TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                memory_id TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS memory_index (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                title TEXT NOT NULL,
                type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'active',
                topics TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                retrieval_count INTEGER NOT NULL DEFAULT 0,
                last_retrieved INTEGER
            );
            CREATE TABLE IF NOT EXISTS workspaces (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL DEFAULT 'default',
                path TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL DEFAULT 'default',
                user_id TEXT NOT NULL,
                channel TEXT NOT NULL,
                title TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp INTEGER NOT NULL
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

    /// Mark a todo complete. Returns true if a row was changed, false if the id is unknown.
    pub fn complete_todo(&self, id: &str) -> Result<bool, AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let rows = conn.execute("UPDATE todos SET completed = 1, updated_at = ?1 WHERE id = ?2", params![now, id])?;
        Ok(rows > 0)
    }

    pub fn delete_todo(&self, id: &str) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Update todo fields. Returns true if a row was changed, false if the id is unknown.
    pub fn update_todo(&self, id: &str, title: Option<&str>, description: Option<&str>, priority: Option<&str>, due_date: Option<&str>) -> Result<bool, AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut changed = 0;
        if let Some(t) = title {
            changed += conn.execute("UPDATE todos SET title = ?1, updated_at = ?2 WHERE id = ?3", params![t, now, id])?;
        }
        if let Some(d) = description {
            changed += conn.execute("UPDATE todos SET description = ?1, updated_at = ?2 WHERE id = ?3", params![d, now, id])?;
        }
        if let Some(p) = priority {
            changed += conn.execute("UPDATE todos SET priority = ?1, updated_at = ?2 WHERE id = ?3", params![p, now, id])?;
        }
        if let Some(d) = due_date {
            changed += conn.execute("UPDATE todos SET due_date = ?1, updated_at = ?2 WHERE id = ?3", params![d, now, id])?;
        }
        Ok(changed > 0)
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

    pub fn upsert_memory_index(&self, record: &MemoryRecord) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "INSERT OR REPLACE INTO memory_index (id, path, title, type, status, topics, created_at, updated_at, retrieval_count, last_retrieved) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![record.id, record.path, record.title, record.mem_type, record.status, record.topics, record.created_at, record.updated_at, record.retrieval_count, record.last_retrieved],
        )?;
        Ok(())
    }

    pub fn get_memory_index(&self) -> Result<Vec<MemoryRecord>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, path, title, type, status, topics, created_at, updated_at, retrieval_count, last_retrieved FROM memory_index ORDER BY updated_at DESC")?;
        let records = stmt.query_map([], |row| {
            Ok(MemoryRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                title: row.get(2)?,
                mem_type: row.get(3)?,
                status: row.get(4)?,
                topics: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                retrieval_count: row.get(8)?,
                last_retrieved: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn update_memory_retrieval(&self, id: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "UPDATE memory_index SET retrieval_count = retrieval_count + 1, last_retrieved = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn get_stale_memories(&self, threshold_days: i64) -> Result<Vec<MemoryRecord>, AlfredError> {
        let threshold = Utc::now().timestamp() - (threshold_days * 24 * 60 * 60);
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, path, title, type, status, topics, created_at, updated_at, retrieval_count, last_retrieved FROM memory_index WHERE status = 'active' AND (last_retrieved IS NULL OR last_retrieved < ?1)")?;
        let records = stmt.query_map(params![threshold], |row| {
            Ok(MemoryRecord {
                id: row.get(0)?,
                path: row.get(1)?,
                title: row.get(2)?,
                mem_type: row.get(3)?,
                status: row.get(4)?,
                topics: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                retrieval_count: row.get(8)?,
                last_retrieved: row.get(9)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn add_to_backlog(&self, session_id: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "INSERT OR IGNORE INTO distillation_backlog (session_id, status, created_at) VALUES (?1, 'pending', ?2)",
            params![session_id, now],
        )?;
        Ok(())
    }

    pub fn get_pending_backlog(&self) -> Result<Vec<String>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT session_id FROM distillation_backlog WHERE status = 'pending'")?;
        let ids = stmt.query_map([], |row| row.get(0))?.collect::<Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    pub fn update_backlog_status(&self, session_id: &str, status: &str) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute("UPDATE distillation_backlog SET status = ?1 WHERE session_id = ?2", params![status, session_id])?;
        Ok(())
    }

    pub fn add_distilled_insight(&self, insight: &DistilledInsight) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "INSERT INTO distilled_insights (id, session_id, complexity, tools, files, outcome, summary, basic_lesson, detailed_lessons, patterns, principles, status, memory_id, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![insight.id, insight.session_id, insight.complexity, insight.tools, insight.files, insight.outcome, insight.summary, insight.basic_lesson, insight.detailed_lessons, insight.patterns, insight.principles, insight.status, insight.memory_id, insight.created_at],
        )?;
        Ok(())
    }

    pub fn get_pending_insights(&self) -> Result<Vec<DistilledInsight>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, session_id, complexity, tools, files, outcome, summary, basic_lesson, detailed_lessons, patterns, principles, status, memory_id, created_at FROM distilled_insights WHERE status = 'pending'")?;
        let insights = stmt.query_map([], |row| {
            Ok(DistilledInsight {
                id: row.get(0)?,
                session_id: row.get(1)?,
                complexity: row.get(2)?,
                tools: row.get(3)?,
                files: row.get(4)?,
                outcome: row.get(5)?,
                summary: row.get(6)?,
                basic_lesson: row.get(7)?,
                detailed_lessons: row.get(8)?,
                patterns: row.get(9)?,
                principles: row.get(10)?,
                status: row.get(11)?,
                memory_id: row.get(12)?,
                created_at: row.get(13)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(insights)
    }

    pub fn update_insight_status(&self, id: &str, status: &str, memory_id: Option<&str>) -> Result<(), AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        conn.execute(
            "UPDATE distilled_insights SET status = ?1, memory_id = ?2 WHERE id = ?3",
            params![status, memory_id, id],
        )?;
        Ok(())
    }
}
