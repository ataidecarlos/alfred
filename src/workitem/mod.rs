use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AlfredError;
use crate::store::Store;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub acceptance_criteria: String,
    pub status: String,
    pub priority: String,
    pub category: String,
    pub assigned_agent: Option<String>,
    pub assigned_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub depends_on: Option<String>,
    pub blocks: Option<String>,
    pub progress_log: Option<String>,
    pub verification_command: Option<String>,
    pub estimated_effort: Option<String>,
    pub actual_effort: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkItemHistory {
    pub id: String,
    pub work_item_id: String,
    pub old_status: Option<String>,
    pub new_status: Option<String>,
    pub agent_id: Option<String>,
    pub note: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Deserialize)]
pub struct NewWorkItem {
    pub title: String,
    pub description: String,
    pub acceptance_criteria: Vec<String>,
    pub priority: String,
    pub category: String,
    pub depends_on: Option<Vec<String>>,
    pub verification_command: Option<String>,
    pub estimated_effort: Option<String>,
}

impl Store {
    pub fn workitem_schema(&self) -> Result<(), AlfredError> {
        let conn = self.conn();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS work_items (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                acceptance_criteria TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                priority TEXT NOT NULL DEFAULT 'medium',
                category TEXT NOT NULL,
                assigned_agent TEXT,
                assigned_at INTEGER,
                completed_at INTEGER,
                depends_on TEXT,
                blocks TEXT,
                progress_log TEXT,
                verification_command TEXT,
                estimated_effort TEXT,
                actual_effort TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS work_item_history (
                id TEXT PRIMARY KEY,
                work_item_id TEXT NOT NULL,
                old_status TEXT,
                new_status TEXT,
                agent_id TEXT,
                note TEXT,
                timestamp INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_work_items_status ON work_items(status);
            CREATE INDEX IF NOT EXISTS idx_work_items_priority ON work_items(priority);
            CREATE INDEX IF NOT EXISTS idx_work_item_history_item ON work_item_history(work_item_id);"
        )?;
        Ok(())
    }

    pub fn add_workitem(&self, item: &NewWorkItem) -> Result<String, AlfredError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let criteria = serde_json::to_string(&item.acceptance_criteria)
            .map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let depends_on = item.depends_on.as_ref().map(|d| {
            serde_json::to_string(d).unwrap_or_default()
        });

        let conn = self.conn();
        conn.execute(
            "INSERT INTO work_items (id, title, description, acceptance_criteria, status, priority, category, depends_on, verification_command, estimated_effort, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, 'pending', ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
            params![
                id,
                item.title,
                item.description,
                criteria,
                item.priority,
                item.category,
                depends_on,
                item.verification_command,
                item.estimated_effort,
                now,
            ],
        )?;
        Ok(id)
    }

    pub fn get_workitem(&self, id: &str) -> Result<Option<WorkItem>, AlfredError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(WorkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                acceptance_criteria: row.get(3)?,
                status: row.get(4)?,
                priority: row.get(5)?,
                category: row.get(6)?,
                assigned_agent: row.get(7)?,
                assigned_at: row.get(8)?,
                completed_at: row.get(9)?,
                depends_on: row.get(10)?,
                blocks: row.get(11)?,
                progress_log: row.get(12)?,
                verification_command: row.get(13)?,
                estimated_effort: row.get(14)?,
                actual_effort: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            _ => Ok(None),
        }
    }

    pub fn list_workitems(&self, status_filter: Option<&str>) -> Result<Vec<WorkItem>, AlfredError> {
        let conn = self.conn();
        let sql = match status_filter {
            Some(_) => "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE status = ?1 ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at",
            None => "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at",
        };
        let mut stmt = conn.prepare(sql)?;
        let row_mapper = |row: &rusqlite::Row| {
            Ok(WorkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                acceptance_criteria: row.get(3)?,
                status: row.get(4)?,
                priority: row.get(5)?,
                category: row.get(6)?,
                assigned_agent: row.get(7)?,
                assigned_at: row.get(8)?,
                completed_at: row.get(9)?,
                depends_on: row.get(10)?,
                blocks: row.get(11)?,
                progress_log: row.get(12)?,
                verification_command: row.get(13)?,
                estimated_effort: row.get(14)?,
                actual_effort: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            })
        };
        let items = match status_filter {
            Some(status) => stmt.query_map(params![status], row_mapper)?.collect::<Result<Vec<_>, _>>()?,
            None => stmt.query_map([], row_mapper)?.collect::<Result<Vec<_>, _>>()?,
        };
        Ok(items)
    }

    pub fn get_next_workitem(&self) -> Result<Option<WorkItem>, AlfredError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE status = 'pending' AND (depends_on IS NULL OR depends_on = '[]') ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at LIMIT 1"
        )?;
        let mut rows = stmt.query_map([], |row| {
            Ok(WorkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                acceptance_criteria: row.get(3)?,
                status: row.get(4)?,
                priority: row.get(5)?,
                category: row.get(6)?,
                assigned_agent: row.get(7)?,
                assigned_at: row.get(8)?,
                completed_at: row.get(9)?,
                depends_on: row.get(10)?,
                blocks: row.get(11)?,
                progress_log: row.get(12)?,
                verification_command: row.get(13)?,
                estimated_effort: row.get(14)?,
                actual_effort: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })?;
        match rows.next() {
            Some(Ok(item)) => Ok(Some(item)),
            _ => Ok(None),
        }
    }

    pub fn assign_workitem(&self, id: &str, agent_id: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn();
        conn.execute(
            "UPDATE work_items SET status = 'in_progress', assigned_agent = ?1, assigned_at = ?2, updated_at = ?2 WHERE id = ?3",
            params![agent_id, now, id],
        )?;
        Self::log_workitem_history(&conn, id, None, Some("in_progress"), Some(agent_id), Some("Work started"))?;
        Ok(())
    }

    pub fn update_workitem_status(&self, id: &str, status: &str, note: Option<&str>) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn();

        let old_status: Option<String> = conn.query_row(
            "SELECT status FROM work_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).ok();

        let completed_at = if status == "completed" { Some(now) } else { None };

        conn.execute(
            "UPDATE work_items SET status = ?1, completed_at = ?2, updated_at = ?3 WHERE id = ?4",
            params![status, completed_at, now, id],
        )?;
        Self::log_workitem_history(&conn, id, old_status.as_deref(), Some(status), None, note)?;
        Ok(())
    }

    pub fn complete_workitem(&self, id: &str, verification_output: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn();

        let old_status: Option<String> = conn.query_row(
            "SELECT status FROM work_items WHERE id = ?1",
            params![id],
            |row| row.get(0),
        ).ok();

        conn.execute(
            "UPDATE work_items SET status = 'completed', completed_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Self::log_workitem_history(&conn, id, old_status.as_deref(), Some("completed"), None, Some(&format!("Verification: {}", verification_output)))?;
        Ok(())
    }

    pub fn log_workitem_progress(&self, id: &str, note: &str) -> Result<(), AlfredError> {
        let now = Utc::now().timestamp();
        let conn = self.conn();
        let entry = serde_json::json!({"timestamp": now, "note": note}).to_string();
        conn.execute(
            "UPDATE work_items SET progress_log = CASE WHEN progress_log IS NULL THEN ?1 ELSE progress_log || ',' || ?1 END, updated_at = ?2 WHERE id = ?3",
            params![entry, now, id],
        )?;
        Ok(())
    }

    fn log_workitem_history(conn: &rusqlite::Connection, work_item_id: &str, old_status: Option<&str>, new_status: Option<&str>, agent_id: Option<&str>, note: Option<&str>) -> Result<(), AlfredError> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        conn.execute(
            "INSERT INTO work_item_history (id, work_item_id, old_status, new_status, agent_id, note, timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![id, work_item_id, old_status, new_status, agent_id, note, now],
        )?;
        Ok(())
    }

    pub fn get_unblocked_items(&self) -> Result<Vec<WorkItem>, AlfredError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE status = 'pending' AND (depends_on IS NULL OR depends_on = '[]') ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at"
        )?;
        let items = stmt.query_map([], |row| {
            Ok(WorkItem {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                acceptance_criteria: row.get(3)?,
                status: row.get(4)?,
                priority: row.get(5)?,
                category: row.get(6)?,
                assigned_agent: row.get(7)?,
                assigned_at: row.get(8)?,
                completed_at: row.get(9)?,
                depends_on: row.get(10)?,
                blocks: row.get(11)?,
                progress_log: row.get(12)?,
                verification_command: row.get(13)?,
                estimated_effort: row.get(14)?,
                actual_effort: row.get(15)?,
                created_at: row.get(16)?,
                updated_at: row.get(17)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(items)
    }

    pub fn get_workitem_history(&self, work_item_id: &str) -> Result<Vec<WorkItemHistory>, AlfredError> {
        let conn = self.conn();
        let mut stmt = conn.prepare(
            "SELECT id, work_item_id, old_status, new_status, agent_id, note, timestamp FROM work_item_history WHERE work_item_id = ?1 ORDER BY timestamp"
        )?;
        let history = stmt.query_map(params![work_item_id], |row| {
            Ok(WorkItemHistory {
                id: row.get(0)?,
                work_item_id: row.get(1)?,
                old_status: row.get(2)?,
                new_status: row.get(3)?,
                agent_id: row.get(4)?,
                note: row.get(5)?,
                timestamp: row.get(6)?,
            })
        })?.collect::<Result<Vec<_>, _>>()?;
        Ok(history)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn test_store() -> Store {
        let store = Store::from_connection(Connection::open_in_memory().unwrap());
        store.workitem_schema().unwrap();
        store
    }

    fn new_item() -> NewWorkItem {
        NewWorkItem {
            title: "Test".to_string(),
            description: "Test item".to_string(),
            acceptance_criteria: vec![],
            priority: "medium".to_string(),
            category: "feature".to_string(),
            depends_on: None,
            verification_command: None,
            estimated_effort: None,
        }
    }

    #[test]
    fn assign_workitem_records_history() {
        let store = test_store();
        let id = store.add_workitem(&new_item()).unwrap();

        store.assign_workitem(&id, "agent").unwrap();

        let item = store.get_workitem(&id).unwrap().unwrap();
        assert_eq!(item.status, "in_progress");
        assert_eq!(item.assigned_agent.as_deref(), Some("agent"));

        let history = store.get_workitem_history(&id).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].new_status.as_deref(), Some("in_progress"));
        assert_eq!(history[0].agent_id.as_deref(), Some("agent"));
    }

    #[test]
    fn update_and_complete_record_history() {
        let store = test_store();
        let id = store.add_workitem(&new_item()).unwrap();
        store.assign_workitem(&id, "agent").unwrap();

        store.update_workitem_status(&id, "failed", Some("boom")).unwrap();
        assert_eq!(store.get_workitem(&id).unwrap().unwrap().status, "failed");

        store.complete_workitem(&id, "ok").unwrap();
        let item = store.get_workitem(&id).unwrap().unwrap();
        assert_eq!(item.status, "completed");
        assert!(item.completed_at.is_some());

        let history = store.get_workitem_history(&id).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[2].new_status.as_deref(), Some("completed"));
        assert!(history[2].note.as_deref().unwrap().contains("Verification: ok"));
    }
}
