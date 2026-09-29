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
            CREATE TABLE IF NOT EXISTS jobs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                kind TEXT NOT NULL CHECK (kind IN ('once','recurring','watch')),
                schedule TEXT,
                run_at INTEGER,
                prompt TEXT NOT NULL,
                report TEXT NOT NULL DEFAULT 'on_signal' CHECK (report IN ('always','on_signal')),
                deliver_to TEXT,
                model TEXT,
                tools TEXT,
                timeout_secs INTEGER NOT NULL DEFAULT 900,
                enabled INTEGER NOT NULL DEFAULT 1,
                last_run INTEGER,
                last_status TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS job_runs (
                id TEXT PRIMARY KEY,
                job_id TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                finished_at INTEGER,
                status TEXT NOT NULL,
                verdict TEXT,
                output TEXT,
                error TEXT,
                tokens_input INTEGER,
                tokens_output INTEGER,
                cost_usd REAL,
                delivered INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_jobs_enabled ON jobs(enabled);
            CREATE INDEX IF NOT EXISTS idx_job_runs_job ON job_runs(job_id);
            DROP TABLE IF EXISTS scheduled_tasks;
            DROP TABLE IF EXISTS distillation_backlog;
            DROP TABLE IF EXISTS distilled_insights;
            DROP TABLE IF EXISTS memory_index;
            DROP TABLE IF EXISTS work_items;
            DROP TABLE IF EXISTS work_item_history;
            DROP TABLE IF EXISTS conversations;
            DROP TABLE IF EXISTS sessions;
            DROP TABLE IF EXISTS messages;
            DROP TABLE IF EXISTS memories;"
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
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(store: &Store, name: &str) -> bool {
        let conn = store.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            params![name],
            |row| row.get::<_, i64>(0),
        ).map(|count| count > 0).unwrap()
    }

    #[test]
    fn schema_creates_jobs_and_drops_old_tables() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("schema.db")).unwrap();

        assert!(table_exists(&store, "todos"));
        assert!(table_exists(&store, "jobs"));
        assert!(table_exists(&store, "job_runs"));

        for dropped in [
            "scheduled_tasks",
            "distillation_backlog",
            "distilled_insights",
            "memory_index",
            "work_items",
            "work_item_history",
            "conversations",
            "sessions",
            "messages",
            "memories",
        ] {
            assert!(!table_exists(&store, dropped), "table {dropped} should be dropped");
        }
    }

    #[test]
    fn schema_indexes_exist() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("indexes.db")).unwrap();
        let conn = store.conn.lock().unwrap();
        for index in ["idx_jobs_enabled", "idx_job_runs_job"] {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                params![index],
                |row| row.get(0),
            ).unwrap();
            assert_eq!(count, 1, "index {index} should exist");
        }
    }
}
