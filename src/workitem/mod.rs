use std::path::PathBuf;
use std::process::Command;

use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::AlfredError;
use crate::store::Store;

/// Result of executing a work item's verification command.
#[derive(Debug, Clone, PartialEq)]
pub struct VerificationOutcome {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

/// Result of attempting to complete a work item.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedCompletion {
    /// True when the item was marked completed, false when verification failed.
    pub completed: bool,
    /// Human-readable verification output or failure reason.
    pub output: String,
}

/// Run a verification command through the platform shell.
///
/// The directory containing the currently running executable is prepended to
/// `PATH` so verification commands which invoke `alfred` resolve to the same
/// build that is performing the verification.
pub fn run_verification_command(command: &str) -> VerificationOutcome {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    };
    #[cfg(not(target_os = "windows"))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    };

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let mut paths: Vec<PathBuf> =
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
            paths.insert(0, dir.to_path_buf());
            if let Ok(joined) = std::env::join_paths(paths) {
                cmd.env("PATH", joined);
            }
        }
    }

    match cmd.output() {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            let mut combined = String::new();
            combined.push_str(stdout.trim_end());
            if !stderr.trim().is_empty() {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(stderr.trim_end());
            }
            VerificationOutcome {
                success: out.status.success(),
                exit_code: out.status.code(),
                output: combined.trim().to_string(),
            }
        }
        Err(e) => VerificationOutcome {
            success: false,
            exit_code: None,
            output: format!("failed to execute verification command: {}", e),
        },
    }
}

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
            "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE status = 'pending' ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at"
        )?;
        let mut items = stmt.query_map([], |row| {
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
        // Collect all pending items first, then filter by satisfied dependencies.
        // Cannot hold the connection guard while calling self.conn() again (non-reentrant mutex).
        let all_pending: Vec<WorkItem> = items.by_ref().collect::<Result<Vec<_>, _>>()?;
        drop(items);
        drop(stmt);
        drop(conn);
        for item in all_pending {
            if self.dependencies_satisfied(&item)? {
                return Ok(Some(item));
            }
        }
        Ok(None)
    }

    /// Check if all dependencies for a work item are completed.
    fn dependencies_satisfied(&self, item: &WorkItem) -> Result<bool, AlfredError> {
        let deps: Vec<String> = item.depends_on.as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        if deps.is_empty() {
            return Ok(true);
        }
        let conn = self.conn();
        for dep_id in &deps {
            let status: Option<String> = conn.query_row(
                "SELECT status FROM work_items WHERE id = ?1",
                params![dep_id],
                |row| row.get(0),
            ).ok();
            if status.as_deref() != Some("completed") {
                return Ok(false);
            }
        }
        Ok(true)
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

    /// Complete a work item, running its verification command when one is set.
    ///
    /// - verification command exits 0 -> status `completed`
    /// - verification command exits != 0 -> status `failed`, output recorded
    /// - no verification command -> status `completed`, recording `manual_output`
    pub fn complete_workitem_verified(
        &self,
        id: &str,
        manual_output: Option<&str>,
    ) -> Result<VerifiedCompletion, AlfredError> {
        let item = match self.get_workitem(id)? {
            Some(item) => item,
            None => return Err(AlfredError::Store(rusqlite::Error::QueryReturnedNoRows)),
        };

        match item.verification_command.as_deref().map(str::trim) {
            Some(command) if !command.is_empty() => {
                let outcome = run_verification_command(command);
                if outcome.success {
                    self.complete_workitem(id, &outcome.output)?;
                    Ok(VerifiedCompletion {
                        completed: true,
                        output: outcome.output,
                    })
                } else {
                    let code = outcome
                        .exit_code
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    let reason = if outcome.output.is_empty() {
                        format!("Verification failed (exit {})", code)
                    } else {
                        format!("Verification failed (exit {}): {}", code, outcome.output)
                    };
                    self.update_workitem_status(id, "failed", Some(&reason))?;
                    Ok(VerifiedCompletion {
                        completed: false,
                        output: reason,
                    })
                }
            }
            _ => {
                let output = manual_output
                    .unwrap_or("No verification command")
                    .to_string();
                self.complete_workitem(id, &output)?;
                Ok(VerifiedCompletion {
                    completed: true,
                    output,
                })
            }
        }
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
            "SELECT id, title, description, acceptance_criteria, status, priority, category, assigned_agent, assigned_at, completed_at, depends_on, blocks, progress_log, verification_command, estimated_effort, actual_effort, created_at, updated_at FROM work_items WHERE status = 'pending' ORDER BY CASE priority WHEN 'critical' THEN 0 WHEN 'high' THEN 1 WHEN 'medium' THEN 2 WHEN 'low' THEN 3 END, created_at"
        )?;
        let mut items = stmt.query_map([], |row| {
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
        let all_pending: Vec<WorkItem> = items.by_ref().collect::<Result<Vec<_>, _>>()?;
        drop(items);
        drop(stmt);
        drop(conn);
        let mut result = Vec::new();
        for item in all_pending {
            if self.dependencies_satisfied(&item)? {
                result.push(item);
            }
        }
        Ok(result)
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

    fn item_with_verification(command: &str) -> NewWorkItem {
        NewWorkItem {
            verification_command: Some(command.to_string()),
            ..new_item()
        }
    }

    #[test]
    fn run_verification_command_captures_output_and_success() {
        let outcome = run_verification_command("echo hello");
        assert!(outcome.success);
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.output, "hello");

        let failure = run_verification_command("exit 3");
        assert!(!failure.success);
        assert_eq!(failure.exit_code, Some(3));
    }

    #[test]
    fn verification_success_marks_item_completed() {
        let store = test_store();
        let id = store.add_workitem(&item_with_verification("echo success")).unwrap();

        let result = store.complete_workitem_verified(&id, None).unwrap();

        assert!(result.completed);
        assert!(result.output.contains("success"));
        let item = store.get_workitem(&id).unwrap().unwrap();
        assert_eq!(item.status, "completed");
        assert!(item.completed_at.is_some());
    }

    #[test]
    fn verification_failure_marks_item_failed_and_logs_output() {
        let store = test_store();
        let id = store.add_workitem(&item_with_verification("echo boom && exit 2")).unwrap();

        let result = store.complete_workitem_verified(&id, None).unwrap();

        assert!(!result.completed);
        assert!(result.output.contains("Verification failed"));
        assert!(result.output.contains("boom"));
        let item = store.get_workitem(&id).unwrap().unwrap();
        assert_eq!(item.status, "failed");
        assert!(item.completed_at.is_none());

        let history = store.get_workitem_history(&id).unwrap();
        let last = history.last().unwrap();
        assert_eq!(last.new_status.as_deref(), Some("failed"));
        assert!(last.note.as_deref().unwrap().contains("boom"));
    }

    #[test]
    fn completion_without_verification_command_uses_manual_output() {
        let store = test_store();
        let id = store.add_workitem(&new_item()).unwrap();

        let result = store
            .complete_workitem_verified(&id, Some("manual check ok"))
            .unwrap();

        assert!(result.completed);
        assert_eq!(result.output, "manual check ok");
        assert_eq!(store.get_workitem(&id).unwrap().unwrap().status, "completed");
    }

    #[test]
    fn complete_verified_unknown_id_errors() {
        let store = test_store();
        assert!(store.complete_workitem_verified("missing", None).is_err());
    }

    #[test]
    fn dependency_resolution_skips_blocked_items() {
        let store = test_store();
        // Item A with no dependencies
        let a = store.add_workitem(&new_item()).unwrap();
        // Item B depends on A
        let mut b = new_item();
        b.depends_on = Some(vec![a.clone()]);
        let b_id = store.add_workitem(&b).unwrap();
        // Item C depends on B
        let mut c = new_item();
        c.depends_on = Some(vec![b_id.clone()]);
        let c_id = store.add_workitem(&c).unwrap();

        // First next should be A
        let next = store.get_next_workitem().unwrap().unwrap();
        assert_eq!(next.id, a);

        // Complete A
        store.complete_workitem(&a, "done").unwrap();

        // Now next should be B
        let next = store.get_next_workitem().unwrap().unwrap();
        assert_eq!(next.id, b_id);

        // Complete B
        store.complete_workitem(&b_id, "done").unwrap();

        // Now next should be C
        let next = store.get_next_workitem().unwrap().unwrap();
        assert_eq!(next.id, c_id);

        // Complete C
        store.complete_workitem(&c_id, "done").unwrap();

        // No more pending
        assert!(store.get_next_workitem().unwrap().is_none());
    }

    #[test]
    fn unblocked_items_excludes_dependent_pending() {
        let store = test_store();
        let a = store.add_workitem(&new_item()).unwrap();
        let mut b = new_item();
        b.depends_on = Some(vec![a.clone()]);
        store.add_workitem(&b).unwrap();

        let unblocked = store.get_unblocked_items().unwrap();
        assert_eq!(unblocked.len(), 1);
        assert_eq!(unblocked[0].id, a);
    }
}
