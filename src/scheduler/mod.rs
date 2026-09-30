//! The database-driven job scheduler.
//!
//! The OS-cron based scheduler was removed as part of the prune to the Pi-host
//! goal. This module owns the replacement: a ticker that asks the store for due
//! jobs and dispatches them, bounded by a semaphore.
//!
//! Due-ness is computed from `jobs.schedule` with the [`cron`] crate (via
//! [`crate::jobs::is_due`]) rather than `tokio-cron-scheduler`, because jobs are
//! created, edited and deleted in SQLite at runtime and the callback
//! registration API of a cron registry does not fit that.
//!
//! The run *body* is not built here. The loop hands each due job to a
//! [`Dispatch`] implementation; the scheduler owns the run-row lifecycle
//! (`record_run_start` / `record_run_end` / `prune_runs`) so the Pi-backed
//! runner arriving later only has to invoke Pi and report a [`RunEnd`].
//!
//! Time is read through the [`Clock`] trait so tests inject a clock instead of
//! sleeping.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use tokio::sync::Semaphore;

use crate::config::JobsConfig;
use crate::error::AlfredError;
use crate::jobs::{self, Job, JobKind, JobRun, RunEnd};
use crate::store::Store;

/// How often the loop polls for due jobs.
pub const DEFAULT_TICK_SECS: u64 = 30;

/// A `once` job overdue by more than this runs immediately; anything older is
/// recorded `missed`.
pub const DEFAULT_ONCE_MISFIRE_GRACE_SECS: i64 = 3600;

/// Placeholder run status used by the scheduler tests.
pub const STUB_STATUS: &str = "noop";

/// A source of the current Unix timestamp.
pub trait Clock: Send + Sync + 'static {
    /// The current time as Unix seconds.
    fn now(&self) -> i64;
}

/// The wall clock, reading the system time.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        Utc::now().timestamp()
    }
}

/// The outcome of running one job. The scheduler owns the run row; a dispatcher
/// only describes the terminal fields to write to it.
#[async_trait]
pub trait Dispatch: Send + Sync {
    /// Run `job` and report how it ended.
    async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError>;

    /// Called by the scheduler once the run row for `job` has been closed.
    ///
    /// The default is a no-op, so a dispatcher that only produces a [`RunEnd`]
    /// (such as [`crate::jobs::runner::JobRunner`]) is unaffected. The composed
    /// [`JobDispatch`](crate::jobs::dispatch::JobDispatch) uses it to deliver the
    /// result: delivery records its outcome against the run id, which does not
    /// exist until the scheduler has opened and closed the row.
    async fn after_run(&self, _run_id: &str, _job: &Job, _end: &RunEnd) {}
}

/// Scheduling policy. Derived from [`JobsConfig`] plus the loop's own constants.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// How long to wait between ticks.
    pub tick_interval: Duration,
    /// Maximum number of jobs running at once; excess jobs queue.
    pub max_concurrent: usize,
    /// Runs kept per job after each completion.
    pub max_runs_per_job: usize,
    /// A `once` job overdue by more than this is recorded `missed`.
    pub once_misfire_grace_secs: i64,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval: Duration::from_secs(DEFAULT_TICK_SECS),
            max_concurrent: 2,
            max_runs_per_job: 100,
            once_misfire_grace_secs: DEFAULT_ONCE_MISFIRE_GRACE_SECS,
        }
    }
}

impl SchedulerConfig {
    /// Build the loop policy from the user-facing `[jobs]` config.
    pub fn from_jobs(jobs: &JobsConfig) -> Self {
        Self {
            max_concurrent: jobs.max_concurrent,
            max_runs_per_job: jobs.max_runs_per_job,
            ..Self::default()
        }
    }
}

/// The scheduling loop over persisted jobs.
pub struct Scheduler {
    store: Arc<Store>,
    clock: Arc<dyn Clock>,
    dispatch: Arc<dyn Dispatch>,
    config: SchedulerConfig,
    /// Bounds concurrent runs across every tick.
    permits: Arc<Semaphore>,
}

impl Scheduler {
    /// Create a scheduler over `store`, reading time from `clock` and handing
    /// due jobs to `dispatch`.
    pub fn new(
        store: Arc<Store>,
        clock: Arc<dyn Clock>,
        dispatch: Arc<dyn Dispatch>,
        config: SchedulerConfig,
    ) -> Self {
        let permits = Arc::new(Semaphore::new(config.max_concurrent.max(1)));
        Self {
            store,
            clock,
            dispatch,
            config,
            permits,
        }
    }

    /// Run the loop forever, ticking every [`SchedulerConfig::tick_interval`].
    ///
    /// A tick that fails is logged and the loop continues; the task never
    /// unwinds on a job panic because each run is isolated in its own task.
    pub async fn run(self: Arc<Self>) {
        let mut ticker = tokio::time::interval(self.config.tick_interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = Arc::clone(&self).tick(self.clock.now()).await {
                tracing::error!(%error, "scheduler tick failed");
            }
        }
    }

    /// Run a single tick as of `now`: disable broken schedules, collect the due
    /// jobs, then dispatch them under the concurrency limit.
    ///
    /// Excess jobs wait on the semaphore rather than being dropped. The call
    /// returns once every job dispatched by this tick has finished.
    pub async fn tick(self: Arc<Self>, now: i64) -> Result<(), AlfredError> {
        self.disable_invalid_schedules()?;

        let mut to_run = Vec::new();
        for job in self.store.due_jobs(now)? {
            if job.kind == JobKind::Once {
                let run_at = job.run_at.unwrap_or(now);
                if now.saturating_sub(run_at) > self.config.once_misfire_grace_secs {
                    self.record_missed(&job)?;
                    continue;
                }
            }
            to_run.push(job);
        }

        let mut handles = Vec::with_capacity(to_run.len());
        for job in to_run {
            let store = Arc::clone(&self.store);
            let dispatch = Arc::clone(&self.dispatch);
            let permits = Arc::clone(&self.permits);
            let max_runs_per_job = self.config.max_runs_per_job;
            handles.push(tokio::spawn(run_one(
                store,
                dispatch,
                permits,
                max_runs_per_job,
                job,
            )));
        }

        for handle in handles {
            if let Err(error) = handle.await {
                tracing::error!(%error, "scheduler run task panicked");
            }
        }
        Ok(())
    }

    /// Disable enabled jobs whose stored schedule no longer parses.
    ///
    /// Such a job can never become due again; retrying it every tick would only
    /// spam the log. Each one gets a `invalid_cron` run row (which mirrors onto
    /// `jobs.last_status`) and is switched off.
    fn disable_invalid_schedules(&self) -> Result<(), AlfredError> {
        for job in self.store.list_jobs()? {
            if !job.enabled {
                continue;
            }
            let Some(message) = schedule_error(&job) else {
                continue;
            };
            tracing::error!(job = %job.name, %message, "disabling job with unparsable schedule");
            let run_id = self.store.record_run_start(&job.id)?;
            let mut end = RunEnd::new("invalid_cron");
            end.error = Some(message);
            self.store.record_run_end(&run_id, &end)?;
            self.store.set_enabled(&job.id, false)?;
        }
        Ok(())
    }

    /// Record a `once` job that was missed by more than the grace window.
    ///
    /// Opening the run stamps `last_run`, so the job will not be seen again; the
    /// `missed` row is left for the `report` policy to act on.
    fn record_missed(&self, job: &Job) -> Result<(), AlfredError> {
        tracing::warn!(job = %job.name, "recording missed once job");
        let run_id = self.store.record_run_start(&job.id)?;
        self.store.record_run_end(&run_id, &RunEnd::new("missed"))
    }
}

/// Run one job to completion and persist its run row exactly as the loop does.
///
/// This is the whole per-run lifecycle in one place: open the row `running`,
/// dispatch (turning a dispatch error or panic into a `failed` end, never
/// unwinding), close the row with the terminal fields, deliver through
/// [`Dispatch::after_run`], then prune history. The scheduled loop
/// ([`Scheduler::tick`]) and the manual-run entry points both call it, so a
/// manual run and a scheduled run record the same row and deliver the same way.
///
/// Returns the closed run row, including the `delivered` flag that
/// [`Dispatch::after_run`] sets.
pub async fn run_to_completion(
    store: &Arc<Store>,
    dispatch: &Arc<dyn Dispatch>,
    max_runs_per_job: usize,
    job: &Job,
) -> Result<JobRun, AlfredError> {
    let run_id = store.record_run_start(&job.id)?;
    let end = dispatch_catching_panics(dispatch, job, &run_id).await;
    store.record_run_end(&run_id, &end)?;
    dispatch.after_run(&run_id, job, &end).await;
    let run = store.get_run(&run_id)?;
    if let Err(error) = store.prune_runs(&job.id, max_runs_per_job) {
        tracing::error!(job = %job.name, %error, "failed to prune run history");
    }
    Ok(run)
}

/// Dispatch `job` in a child task so a panic is observed as a
/// [`tokio::task::JoinError`] and recorded `failed` instead of propagating.
async fn dispatch_catching_panics(dispatch: &Arc<dyn Dispatch>, job: &Job, run_id: &str) -> RunEnd {
    let joined = tokio::spawn({
        let dispatch = Arc::clone(dispatch);
        let job = job.clone();
        async move { dispatch.dispatch(&job).await }
    })
    .await;

    match joined {
        Ok(Ok(end)) => end,
        Ok(Err(error)) => {
            tracing::error!(job = %job.name, %run_id, %error, "job dispatch failed");
            let mut end = RunEnd::new("failed");
            end.error = Some(error.to_string());
            end
        }
        Err(join_error) => {
            tracing::error!(job = %job.name, %run_id, %join_error, "job dispatch panicked");
            let mut end = RunEnd::new("failed");
            end.error = Some(if join_error.is_panic() {
                "panic in job dispatch".to_string()
            } else {
                join_error.to_string()
            });
            end
        }
    }
}

/// Run one job while holding a semaphore permit, then persist and prune.
async fn run_one(
    store: Arc<Store>,
    dispatch: Arc<dyn Dispatch>,
    permits: Arc<Semaphore>,
    max_runs_per_job: usize,
    job: Job,
) {
    let permit = match permits.acquire_owned().await {
        Ok(permit) => permit,
        Err(error) => {
            tracing::error!(job = %job.name, %error, "scheduler semaphore closed");
            return;
        }
    };

    if let Err(error) = run_to_completion(&store, &dispatch, max_runs_per_job, &job).await {
        tracing::error!(job = %job.name, %error, "failed to run job");
    }

    drop(permit);
}

/// The reason a job's stored schedule can never be evaluated, if any.
///
/// `once` jobs carry `run_at` instead of a schedule. Recurring and watch jobs
/// must have a five-field expression that parses.
fn schedule_error(job: &Job) -> Option<String> {
    match job.kind {
        JobKind::Once => None,
        JobKind::Recurring | JobKind::Watch => match job.schedule.as_deref() {
            None => Some("missing schedule".to_string()),
            Some(expression) => jobs::parse_five_field_cron(expression)
                .err()
                .map(|e| e.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicBool, AtomicI64, AtomicUsize, Ordering};
    use std::sync::Mutex;

    use chrono::Utc;
    use rusqlite::params;

    use super::*;

    /// The default `min_watch_interval_secs` used by the store.
    const MIN_WATCH: u64 = 900;

    /// A clock whose time the test sets explicitly, so nothing sleeps.
    struct FixedClock(AtomicI64);

    impl FixedClock {
        fn new(now: i64) -> Self {
            Self(AtomicI64::new(now))
        }
    }

    impl Clock for FixedClock {
        fn now(&self) -> i64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(&dir.path().join("scheduler.db")).expect("open store");
        (dir, store)
    }

    fn once(name: &str, run_at: i64) -> crate::jobs::NewJob {
        crate::jobs::NewJob {
            name: name.to_string(),
            kind: JobKind::Once,
            run_at: Some(run_at),
            prompt: "hello".to_string(),
            ..crate::jobs::NewJob::default()
        }
    }

    fn recurring(name: &str, schedule: &str) -> crate::jobs::NewJob {
        crate::jobs::NewJob {
            name: name.to_string(),
            kind: JobKind::Recurring,
            schedule: Some(schedule.to_string()),
            prompt: "hello".to_string(),
            ..crate::jobs::NewJob::default()
        }
    }

    /// A dispatcher that records the name of every job it is asked to run.
    struct RecordingDispatch {
        status: String,
        calls: Mutex<Vec<String>>,
    }

    impl RecordingDispatch {
        fn new(status: &str) -> Self {
            Self {
                status: status.to_string(),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    #[async_trait::async_trait]
    impl Dispatch for RecordingDispatch {
        async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError> {
            self.calls
                .lock()
                .expect("calls lock")
                .push(job.name.clone());
            Ok(RunEnd::new(self.status.clone()))
        }
    }

    /// Panics for the job named `boom`, succeeds for everything else.
    struct SplitDispatch;

    #[async_trait::async_trait]
    impl Dispatch for SplitDispatch {
        async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError> {
            if job.name == "boom" {
                panic!("boom dispatch");
            }
            Ok(RunEnd::new(STUB_STATUS))
        }
    }

    /// Holds the first two concurrent runs until the test releases them, so the
    /// queue behind the semaphore is observable.
    struct GateDispatch {
        running: AtomicUsize,
        peak: AtomicUsize,
        started: AtomicUsize,
        started_tx: tokio::sync::mpsc::UnboundedSender<String>,
        released: AtomicBool,
        release: tokio::sync::Notify,
    }

    #[async_trait::async_trait]
    impl Dispatch for GateDispatch {
        async fn dispatch(&self, job: &Job) -> Result<RunEnd, AlfredError> {
            let running = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(running, Ordering::SeqCst);
            let started = self.started.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = self.started_tx.send(job.name.clone());
            if started <= 2 {
                while !self.released.load(Ordering::SeqCst) {
                    self.release.notified().await;
                }
            }
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(RunEnd::new("success"))
        }
    }

    /// Write a job row directly, bypassing store validation, to simulate a row
    /// that became unparsable after the fact.
    fn insert_raw_job(path: &Path, id: &str, name: &str, kind: &str, schedule: Option<&str>) {
        let conn = rusqlite::Connection::open(path).expect("raw connection");
        conn.execute(
            "INSERT INTO jobs (id, name, kind, schedule, prompt, report, timeout_secs, enabled, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, 'p', 'on_signal', 900, 1, 0, 0)",
            params![id, name, kind, schedule],
        )
        .expect("insert raw job");
    }

    #[tokio::test]
    async fn once_job_ten_minutes_overdue_runs() {
        let (_dir, store) = temp_store();
        let now = Utc::now().timestamp();
        let store = Arc::new(store);
        store
            .add_job(&once("overdue", now - 600), MIN_WATCH)
            .expect("add");

        let dispatch = Arc::new(RecordingDispatch::new(STUB_STATUS));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            dispatch.clone(),
            SchedulerConfig::default(),
        ));
        Arc::clone(&scheduler).tick(now).await.expect("tick");

        assert_eq!(dispatch.calls(), vec!["overdue".to_string()]);
        let runs = store.runs_for("overdue", 10).expect("runs");
        assert_eq!(runs.len(), 1, "the overdue job ran exactly once");
        assert_eq!(runs[0].status, STUB_STATUS);
        assert_eq!(
            store
                .get_job("overdue")
                .expect("job")
                .last_status
                .as_deref(),
            Some(STUB_STATUS)
        );
    }

    #[tokio::test]
    async fn once_job_three_hours_overdue_is_recorded_missed() {
        let (_dir, store) = temp_store();
        let now = Utc::now().timestamp();
        let store = Arc::new(store);
        store
            .add_job(&once("stale", now - 3 * 3600), MIN_WATCH)
            .expect("add");

        let dispatch = Arc::new(RecordingDispatch::new(STUB_STATUS));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            dispatch.clone(),
            SchedulerConfig::default(),
        ));
        Arc::clone(&scheduler).tick(now).await.expect("tick");

        assert!(
            dispatch.calls().is_empty(),
            "a missed job must not dispatch"
        );
        let runs = store.runs_for("stale", 10).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "missed");
        assert_eq!(
            store.get_job("stale").expect("job").last_status.as_deref(),
            Some("missed")
        );

        // A second tick does not re-record it: opening the run stamped last_run.
        Arc::clone(&scheduler).tick(now).await.expect("tick again");
        assert_eq!(store.runs_for("stale", 10).expect("runs").len(), 1);
    }

    #[tokio::test]
    async fn overdue_recurring_job_runs_one_period_and_does_not_backfill() {
        let (_dir, store) = temp_store();
        let now = Utc::now().timestamp();
        let store = Arc::new(store);
        // No last_run: the first fire is decades in the past, so many periods
        // are missed. Exactly one run must result.
        store
            .add_job(&recurring("hourly", "0 * * * *"), MIN_WATCH)
            .expect("add");

        let dispatch = Arc::new(RecordingDispatch::new(STUB_STATUS));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            dispatch.clone(),
            SchedulerConfig::default(),
        ));
        Arc::clone(&scheduler).tick(now).await.expect("tick");
        assert_eq!(dispatch.calls(), vec!["hourly".to_string()]);

        // The next period has not arrived: no backfill, no double run.
        Arc::clone(&scheduler).tick(now).await.expect("tick again");
        assert_eq!(dispatch.calls(), vec!["hourly".to_string()]);
        assert_eq!(store.runs_for("hourly", 10).expect("runs").len(), 1);
    }

    #[tokio::test]
    async fn max_concurrent_runs_two_and_queues_the_third() {
        let (_dir, store) = temp_store();
        let now = Utc::now().timestamp();
        let store = Arc::new(store);
        for name in ["a", "b", "c"] {
            store
                .add_job(&once(name, now - 60), MIN_WATCH)
                .expect("add");
        }

        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let gate = Arc::new(GateDispatch {
            running: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            started: AtomicUsize::new(0),
            started_tx,
            released: AtomicBool::new(false),
            release: tokio::sync::Notify::new(),
        });
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            gate.clone(),
            SchedulerConfig {
                max_concurrent: 2,
                ..SchedulerConfig::default()
            },
        ));

        let tick = tokio::spawn(Arc::clone(&scheduler).tick(now));
        let first = started_rx.recv().await.expect("first job started");
        let second = started_rx.recv().await.expect("second job started");
        assert_ne!(first, second);
        assert_eq!(
            gate.started.load(Ordering::SeqCst),
            2,
            "only two jobs may run while both permits are held"
        );

        gate.released.store(true, Ordering::SeqCst);
        gate.release.notify_waiters();
        tick.await.expect("tick task").expect("tick ok");

        assert_eq!(
            gate.started.load(Ordering::SeqCst),
            3,
            "the queued job still runs"
        );
        assert_eq!(
            gate.peak.load(Ordering::SeqCst),
            2,
            "concurrency never exceeds max_concurrent"
        );
        for name in ["a", "b", "c"] {
            assert_eq!(
                store.runs_for(name, 10).expect("runs").len(),
                1,
                "{name} ran"
            );
        }
    }

    #[tokio::test]
    async fn panic_in_a_run_is_recorded_failed_and_the_loop_survives() {
        let (_dir, store) = temp_store();
        let now = Utc::now().timestamp();
        let store = Arc::new(store);
        store
            .add_job(&once("boom", now - 60), MIN_WATCH)
            .expect("add");
        store
            .add_job(&once("fine", now - 60), MIN_WATCH)
            .expect("add");

        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            Arc::new(SplitDispatch),
            SchedulerConfig::default(),
        ));
        Arc::clone(&scheduler).tick(now).await.expect("tick");

        let boom = store.runs_for("boom", 10).expect("runs");
        assert_eq!(boom.len(), 1);
        assert_eq!(boom[0].status, "failed");
        assert_eq!(
            store.get_job("boom").expect("job").last_status.as_deref(),
            Some("failed")
        );
        assert_eq!(
            store.runs_for("fine", 10).expect("runs")[0].status,
            STUB_STATUS,
            "a sibling job still completes"
        );

        // The scheduler task survived the panic and can tick again.
        Arc::clone(&scheduler).tick(now).await.expect("tick again");
    }

    #[tokio::test]
    async fn unparsable_stored_expression_is_recorded_and_disables_the_job() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("scheduler.db");
        let store = Arc::new(Store::new(&path).expect("open store"));
        insert_raw_job(&path, "bad", "bad-job", "recurring", Some("99 * * * *"));

        let dispatch = Arc::new(RecordingDispatch::new(STUB_STATUS));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(Utc::now().timestamp())),
            dispatch.clone(),
            SchedulerConfig::default(),
        ));
        Arc::clone(&scheduler)
            .tick(Utc::now().timestamp())
            .await
            .expect("tick");

        let job = store.get_job("bad-job").expect("job");
        assert_eq!(job.last_status.as_deref(), Some("invalid_cron"));
        assert!(!job.enabled, "an invalid job must be disabled");
        assert!(
            dispatch.calls().is_empty(),
            "an invalid job must never reach dispatch"
        );
    }

    #[tokio::test]
    async fn completion_prunes_history_to_max_runs_per_job() {
        let (_dir, store) = temp_store();
        let base = Utc::now().timestamp();
        let store = Arc::new(store);
        let job = store
            .add_job(&recurring("noisy", "* * * * *"), MIN_WATCH)
            .expect("add");

        // Pre-seed two runs; each stamps last_run.
        for _ in 0..2 {
            let run_id = store.record_run_start(&job.id).expect("start");
            store
                .record_run_end(&run_id, &RunEnd::new("success"))
                .expect("end");
        }
        assert_eq!(store.runs_for("noisy", 10).expect("runs").len(), 2);

        // Step the injected clock past the next minute so the job is due.
        let now = base + 120;
        let dispatch = Arc::new(RecordingDispatch::new(STUB_STATUS));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(FixedClock::new(now)),
            dispatch.clone(),
            SchedulerConfig {
                max_runs_per_job: 2,
                ..SchedulerConfig::default()
            },
        ));
        Arc::clone(&scheduler).tick(now).await.expect("tick");

        assert_eq!(dispatch.calls(), vec!["noisy".to_string()]);
        assert_eq!(
            store.runs_for("noisy", 10).expect("runs").len(),
            2,
            "history is bounded by max_runs_per_job after a completion"
        );
    }
}
