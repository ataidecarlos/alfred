use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex, MutexGuard};

use chrono::Utc;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::connectors::telegram::MessageSender;
use crate::error::AlfredError;
use crate::jobs::{self, Job, JobKind, JobRun, NewJob, ReportPolicy, RunEnd};

/// The `jobs` columns, in the order [`job_from_row`] reads them.
const JOB_COLUMNS: &str = "id, name, kind, schedule, run_at, prompt, report, deliver_to, model, \
     tools, timeout_secs, enabled, last_run, last_status, created_at, updated_at";

/// The `job_runs` columns, in the order [`job_run_from_row`] reads them.
const RUN_COLUMNS: &str = "id, job_id, started_at, finished_at, status, verdict, output, error, \
     tokens_input, tokens_output, cost_usd, delivered";

/// Translate a UNIQUE or CHECK violation on a job write into a named error.
/// Other rusqlite failures pass through as [`AlfredError::Store`].
fn map_job_write(error: rusqlite::Error) -> AlfredError {
    let text = error.to_string();
    if text.contains("UNIQUE constraint failed: jobs.name") {
        AlfredError::JobNameExists
    } else if text.contains("CHECK constraint failed") {
        AlfredError::JobValidation("job row violates a CHECK constraint".to_string())
    } else {
        AlfredError::Store(error)
    }
}

fn conversion_error(column: usize, error: AlfredError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(error))
}

fn job_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Job> {
    let kind: String = row.get(2)?;
    let report: String = row.get(6)?;
    Ok(Job {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: JobKind::from_str(&kind).map_err(|error| conversion_error(2, error))?,
        schedule: row.get(3)?,
        run_at: row.get(4)?,
        prompt: row.get(5)?,
        report: ReportPolicy::from_str(&report).map_err(|error| conversion_error(6, error))?,
        deliver_to: row.get(7)?,
        model: row.get(8)?,
        tools: jobs::decode_tools(row.get(9)?),
        timeout_secs: row.get::<_, i64>(10)? as u64,
        enabled: row.get::<_, i64>(11)? != 0,
        last_run: row.get(12)?,
        last_status: row.get(13)?,
        created_at: row.get(14)?,
        updated_at: row.get(15)?,
    })
}

fn job_run_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JobRun> {
    Ok(JobRun {
        id: row.get(0)?,
        job_id: row.get(1)?,
        started_at: row.get(2)?,
        finished_at: row.get(3)?,
        status: row.get(4)?,
        verdict: row.get(5)?,
        output: row.get(6)?,
        error: row.get(7)?,
        tokens_input: row.get(8)?,
        tokens_output: row.get(9)?,
        cost_usd: row.get(10)?,
        delivered: row.get::<_, i64>(11)? != 0,
    })
}

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
    /// Process-wide override for the Telegram transport used by delivery.
    ///
    /// The REST manual-run handler builds its dispatcher from
    /// [`crate::server::AppState`], which has no place to inject a transport.
    /// The store already reaches every delivery path, so it carries the
    /// override; production leaves it [`None`] and delivery uses the real Bot
    /// API sender from `[telegram].bot_token`. Tests set a recorder here so no
    /// request leaves the process.
    telegram_sender: Option<Arc<dyn MessageSender>>,
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
        Ok(Self {
            conn: Mutex::new(conn),
            telegram_sender: None,
        })
    }

    /// Use `sender` for every delivery this store's dispatchers perform.
    ///
    /// The store holds the override because the manual-run handlers build
    /// their delivery from state that has no transport slot; this is the single
    /// seam tests use to keep delivery off the network. Production never calls
    /// it.
    pub fn with_telegram_sender_override(self, sender: Option<Arc<dyn MessageSender>>) -> Self {
        Self {
            telegram_sender: sender,
            ..self
        }
    }

    /// The injected Telegram transport, when one was set.
    pub fn telegram_sender_override(&self) -> Option<Arc<dyn MessageSender>> {
        self.telegram_sender.clone()
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

    /// Look up a todo by id, whether or not it is complete.
    pub fn get_todo(&self, id: &str) -> Result<Option<Todo>, AlfredError> {
        let conn = self.conn.lock().map_err(|e| AlfredError::Store(rusqlite::Error::InvalidParameterName(e.to_string())))?;
        let mut stmt = conn.prepare("SELECT id, title, description, priority, completed, due_date FROM todos WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(Todo {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                priority: row.get(3)?,
                completed: row.get::<_, i32>(4)? != 0,
                due_date: row.get(5)?,
            })
        })?;
        match rows.next() {
            Some(todo) => Ok(Some(todo?)),
            None => Ok(None),
        }
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

    // ------------------------------------------------------------------ jobs

    /// Lock the connection, mapping a poisoned mutex to a typed store error.
    fn db(&self) -> Result<MutexGuard<'_, Connection>, AlfredError> {
        self.conn.lock().map_err(|error| {
            AlfredError::Store(rusqlite::Error::InvalidParameterName(error.to_string()))
        })
    }

    /// Validate and insert a job, returning the stored row.
    pub fn add_job(&self, new: &NewJob, min_watch_interval_secs: u64) -> Result<Job, AlfredError> {
        new.validate(min_watch_interval_secs)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        {
            let conn = self.db()?;
            conn.execute(
                "INSERT INTO jobs (id, name, kind, schedule, run_at, prompt, report, deliver_to, model, tools, timeout_secs, enabled, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, ?12)",
                params![
                    id,
                    new.name,
                    new.kind.as_str(),
                    new.schedule,
                    new.run_at,
                    new.prompt,
                    new.report.as_str(),
                    new.deliver_to,
                    new.model,
                    jobs::encode_tools(&new.tools),
                    new.timeout_secs as i64,
                    now,
                ],
            )
            .map_err(map_job_write)?;
        }
        self.get_job(&id)
    }

    /// All jobs, enabled and disabled, oldest first.
    pub fn list_jobs(&self) -> Result<Vec<Job>, AlfredError> {
        let conn = self.db()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs ORDER BY created_at, id"
        ))?;
        let jobs = stmt
            .query_map([], job_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(jobs)
    }

    /// Look a job up by id or by name.
    pub fn get_job(&self, id_or_name: &str) -> Result<Job, AlfredError> {
        let conn = self.db()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs WHERE id = ?1 OR name = ?1 \
             ORDER BY (id = ?1) DESC LIMIT 1"
        ))?;
        let mut rows = stmt.query_map(params![id_or_name], job_from_row)?;
        match rows.next() {
            Some(job) => Ok(job?),
            None => Err(AlfredError::JobNotFound(id_or_name.to_string())),
        }
    }

    /// Validate and fully replace a job's mutable fields.
    pub fn update_job(
        &self,
        id_or_name: &str,
        new: &NewJob,
        min_watch_interval_secs: u64,
    ) -> Result<Job, AlfredError> {
        new.validate(min_watch_interval_secs)?;
        let existing = self.get_job(id_or_name)?;
        let now = Utc::now().timestamp();
        {
            let conn = self.db()?;
            conn.execute(
                "UPDATE jobs SET name=?1, kind=?2, schedule=?3, run_at=?4, prompt=?5, report=?6, \
                 deliver_to=?7, model=?8, tools=?9, timeout_secs=?10, updated_at=?11 WHERE id=?12",
                params![
                    new.name,
                    new.kind.as_str(),
                    new.schedule,
                    new.run_at,
                    new.prompt,
                    new.report.as_str(),
                    new.deliver_to,
                    new.model,
                    jobs::encode_tools(&new.tools),
                    new.timeout_secs as i64,
                    now,
                    existing.id,
                ],
            )
            .map_err(map_job_write)?;
        }
        self.get_job(&existing.id)
    }

    /// Enable or disable a job, returning the updated row.
    pub fn set_enabled(&self, id_or_name: &str, enabled: bool) -> Result<Job, AlfredError> {
        let job = self.get_job(id_or_name)?;
        let now = Utc::now().timestamp();
        {
            let conn = self.db()?;
            conn.execute(
                "UPDATE jobs SET enabled=?1, updated_at=?2 WHERE id=?3",
                params![enabled as i64, now, job.id],
            )?;
        }
        self.get_job(&job.id)
    }

    /// Delete a job and its run history.
    pub fn delete_job(&self, id_or_name: &str) -> Result<(), AlfredError> {
        let job = self.get_job(id_or_name)?;
        let conn = self.db()?;
        conn.execute("DELETE FROM job_runs WHERE job_id=?1", params![job.id])?;
        conn.execute("DELETE FROM jobs WHERE id=?1", params![job.id])?;
        Ok(())
    }

    /// Enabled jobs that are due at `now`. A stored schedule that no longer
    /// parses is logged and skipped rather than failing the whole tick.
    pub fn due_jobs(&self, now: i64) -> Result<Vec<Job>, AlfredError> {
        let mut due = Vec::new();
        for job in self.list_jobs()? {
            match jobs::is_due(&job, now) {
                Ok(true) => due.push(job),
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(job = %job.name, %error, "skipping job with invalid schedule");
                }
            }
        }
        Ok(due)
    }

    /// Open a run for `job_id`, record it as `running`, and stamp the job's
    /// `last_run`. Returns the new run id.
    pub fn record_run_start(&self, job_id: &str) -> Result<String, AlfredError> {
        let job = self.get_job(job_id)?;
        let run_id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp();
        let conn = self.db()?;
        conn.execute(
            "INSERT INTO job_runs (id, job_id, started_at, status, delivered) \
             VALUES (?1, ?2, ?3, 'running', 0)",
            params![run_id, job.id, now],
        )?;
        conn.execute(
            "UPDATE jobs SET last_run=?1, updated_at=?1 WHERE id=?2",
            params![now, job.id],
        )?;
        Ok(run_id)
    }

    /// Close a run with its terminal fields and mirror the status onto the job.
    pub fn record_run_end(&self, run_id: &str, end: &RunEnd) -> Result<(), AlfredError> {
        let finished_at = Utc::now().timestamp();
        let conn = self.db()?;
        let changed = conn.execute(
            "UPDATE job_runs SET finished_at=?1, status=?2, verdict=?3, output=?4, error=?5, \
             tokens_input=?6, tokens_output=?7, cost_usd=?8, delivered=?9 WHERE id=?10",
            params![
                finished_at,
                end.status,
                end.verdict,
                end.output,
                end.error,
                end.tokens_input,
                end.tokens_output,
                end.cost_usd,
                end.delivered as i64,
                run_id,
            ],
        )?;
        if changed == 0 {
            return Err(AlfredError::JobValidation(format!("run not found: {run_id}")));
        }
        conn.execute(
            "UPDATE jobs SET last_status=?1, updated_at=?2 \
             WHERE id=(SELECT job_id FROM job_runs WHERE id=?3)",
            params![end.status, finished_at, run_id],
        )?;
        Ok(())
    }

    /// Record the outcome of delivering a run's output.
    ///
    /// Sets `delivered`; when `error` is `Some`, it is appended to the run's
    /// `error` column, preserving any text already there. `status` and `output`
    /// are never touched, so a delivery failure cannot turn a successful run
    /// into a failed one and its output is retained. Returns
    /// [`AlfredError::JobValidation`] when `run_id` is unknown.
    pub fn record_delivery(
        &self,
        run_id: &str,
        delivered: bool,
        error: Option<&str>,
    ) -> Result<(), AlfredError> {
        let conn = self.db()?;
        let changed = conn.execute(
            "UPDATE job_runs SET delivered=?1, \
             error = CASE WHEN ?2 IS NULL THEN error \
                          ELSE COALESCE(NULLIF(error, '') || '; ', '') || ?2 END \
             WHERE id=?3",
            params![delivered as i64, error, run_id],
        )?;
        if changed == 0 {
            return Err(AlfredError::JobValidation(format!("run not found: {run_id}")));
        }
        Ok(())
    }

    /// A single run by id, including the `delivered` flag.
    pub fn get_run(&self, run_id: &str) -> Result<JobRun, AlfredError> {
        let conn = self.db()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM job_runs WHERE id=?1"
        ))?;
        let mut rows = stmt.query_map(params![run_id], job_run_from_row)?;
        match rows.next() {
            Some(run) => Ok(run?),
            None => Err(AlfredError::JobValidation(format!("run not found: {run_id}"))),
        }
    }

    /// The most recent runs of a job, newest first.
    pub fn runs_for(&self, id_or_name: &str, limit: usize) -> Result<Vec<JobRun>, AlfredError> {
        let job = self.get_job(id_or_name)?;
        let conn = self.db()?;
        let mut stmt = conn.prepare(&format!(
            "SELECT {RUN_COLUMNS} FROM job_runs WHERE job_id=?1 \
             ORDER BY started_at DESC, id DESC LIMIT ?2"
        ))?;
        let runs = stmt
            .query_map(params![job.id, limit as i64], job_run_from_row)?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    /// Keep only the newest `keep` runs of a job. Returns the number removed.
    pub fn prune_runs(&self, id_or_name: &str, keep: usize) -> Result<usize, AlfredError> {
        let job = self.get_job(id_or_name)?;
        let conn = self.db()?;
        let removed = conn.execute(
            "DELETE FROM job_runs WHERE job_id=?1 AND id NOT IN (\
             SELECT id FROM job_runs WHERE job_id=?1 \
             ORDER BY started_at DESC, id DESC LIMIT ?2)",
            params![job.id, keep as i64],
        )?;
        Ok(removed)
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

    #[test]
    fn todo_round_trip_and_get_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(&dir.path().join("todos.db")).unwrap();

        let id = store
            .add_todo("Water plants", "back porch", "low", "")
            .unwrap();
        assert_eq!(store.get_todo(&id).unwrap().unwrap().title, "Water plants");
        assert!(store.get_todo("missing").unwrap().is_none());

        let listed = store.list_todos().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].priority, "low");

        assert!(store.complete_todo(&id).unwrap());
        assert!(store.get_todo(&id).unwrap().unwrap().completed);
        assert!(store.list_todos().unwrap().is_empty(), "completed todos are hidden");
        assert!(!store.complete_todo("missing").unwrap());

        store.delete_todo(&id).unwrap();
        assert!(store.get_todo(&id).unwrap().is_none());
    }
}
