//! Integration tests for the job domain types and store CRUD (issue #7).
//!
//! Every test lives in `mod jobs` so the acceptance filter
//! `cargo test jobs` selects them. Each test uses its own temporary database;
//! the real `~/.alfred/data/alfred.db` is never touched.

mod jobs {
    use alfred::error::AlfredError;
    use alfred::jobs::{JobKind, NewJob, ReportPolicy, RunEnd};
    use alfred::store::Store;

    /// The default `min_watch_interval_secs`.
    const MIN_WATCH: u64 = 900;

    fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(&dir.path().join("jobs.db")).expect("open store");
        (dir, store)
    }

    fn once(name: &str, run_at: i64) -> NewJob {
        NewJob {
            name: name.to_string(),
            kind: JobKind::Once,
            run_at: Some(run_at),
            prompt: "hello".to_string(),
            report: ReportPolicy::Always,
            ..NewJob::default()
        }
    }

    fn recurring(name: &str, schedule: &str) -> NewJob {
        NewJob {
            name: name.to_string(),
            kind: JobKind::Recurring,
            schedule: Some(schedule.to_string()),
            prompt: "hello".to_string(),
            ..NewJob::default()
        }
    }

    fn watch(name: &str, schedule: &str) -> NewJob {
        NewJob {
            name: name.to_string(),
            kind: JobKind::Watch,
            schedule: Some(schedule.to_string()),
            prompt: "hello".to_string(),
            ..NewJob::default()
        }
    }

    #[test]
    fn add_and_get_job_round_trips() {
        let (_dir, store) = store();
        let mut new = recurring("probe", "*/15 * * * *");
        new.report = ReportPolicy::Always;
        new.deliver_to = Some("telegram:42".to_string());
        new.model = Some("model-x".to_string());
        new.tools = vec!["bash".to_string()];
        new.timeout_secs = 120;

        let created = store.add_job(&new, MIN_WATCH).expect("add job");
        assert_eq!(created.kind, JobKind::Recurring);
        assert_eq!(created.report, ReportPolicy::Always);
        assert_eq!(created.schedule.as_deref(), Some("*/15 * * * *"));
        assert_eq!(created.deliver_to.as_deref(), Some("telegram:42"));
        assert_eq!(created.model.as_deref(), Some("model-x"));
        assert_eq!(created.tools, vec!["bash".to_string()]);
        assert_eq!(created.timeout_secs, 120);
        assert!(created.enabled);
        assert!(created.last_run.is_none());

        assert_eq!(store.get_job("probe").expect("by name").id, created.id);
        assert_eq!(store.get_job(&created.id).expect("by id"), created);
        assert_eq!(store.list_jobs().expect("list").len(), 1);
    }

    #[test]
    fn duplicate_name_is_rejected() {
        let (_dir, store) = store();
        store
            .add_job(&once("probe", 0), MIN_WATCH)
            .expect("first add");

        let error = store
            .add_job(&once("probe", 0), MIN_WATCH)
            .expect_err("duplicate must fail");
        assert_eq!(error.to_string(), "job name already exists");
        assert!(matches!(error, AlfredError::JobNameExists));
    }

    #[test]
    fn watch_below_minimum_interval_is_rejected() {
        let (_dir, store) = store();
        let error = store
            .add_job(&watch("w", "*/1 * * * *"), MIN_WATCH)
            .expect_err("one-minute watch must fail");
        assert_eq!(error.to_string(), "minimum watch interval is 900s");
        assert!(matches!(error, AlfredError::WatchIntervalTooShort(900)));
    }

    #[test]
    fn malformed_cron_surfaces_the_parser_error() {
        let (_dir, store) = store();
        let error = store
            .add_job(&recurring("r", "99 * * * *"), MIN_WATCH)
            .expect_err("out-of-range minute must fail");
        let text = error.to_string();
        assert!(
            text.contains("invalid cron expression '99 * * * *'"),
            "message was: {text}"
        );
        assert!(matches!(error, AlfredError::JobValidation(_)));
    }

    #[test]
    fn once_without_run_at_is_rejected() {
        let (_dir, store) = store();
        let mut job = once("o", 0);
        job.run_at = None;
        let error = store
            .add_job(&job, MIN_WATCH)
            .expect_err("once needs run_at");
        assert!(error.to_string().contains("run_at"), "message was: {error}");
    }

    #[test]
    fn prune_runs_keeps_exactly_max_runs_per_job() {
        let (_dir, store) = store();
        let job = store.add_job(&once("p", 0), MIN_WATCH).expect("add");
        for _ in 0..5 {
            let run_id = store.record_run_start(&job.id).expect("start");
            store
                .record_run_end(&run_id, &RunEnd::new("success"))
                .expect("end");
        }
        assert_eq!(store.runs_for(&job.id, 100).expect("runs").len(), 5);

        let removed = store.prune_runs(&job.id, 3).expect("prune");
        assert_eq!(removed, 2);
        assert_eq!(store.runs_for(&job.id, 100).expect("runs").len(), 3);
    }

    #[test]
    fn due_jobs_returns_only_enabled_and_due_rows() {
        let (_dir, store) = store();
        let now = 1_000_000;

        let due = store
            .add_job(&once("due", now - 60), MIN_WATCH)
            .expect("due");
        store
            .add_job(&once("future", now + 3600), MIN_WATCH)
            .expect("future");
        let disabled = store
            .add_job(&once("disabled", now - 60), MIN_WATCH)
            .expect("disabled");
        store.set_enabled(&disabled.id, false).expect("disable");
        let quiet = store
            .add_job(&recurring("quiet", "*/5 * * * *"), MIN_WATCH)
            .expect("quiet");
        // A run stamps last_run; the next period is in the future.
        let run_id = store.record_run_start(&quiet.id).expect("start");
        store
            .record_run_end(&run_id, &RunEnd::new("success"))
            .expect("end");

        let due_rows = store.due_jobs(now).expect("due_jobs");
        let names: Vec<&str> = due_rows.iter().map(|job| job.name.as_str()).collect();
        assert_eq!(names, vec!["due"]);
        assert_eq!(due_rows[0].id, due.id);
        assert_eq!(store.list_jobs().expect("list").len(), 4);
    }

    #[test]
    fn update_revalidates_and_replaces_fields() {
        let (_dir, store) = store();
        let job = store
            .add_job(&recurring("first", "0 9 * * *"), MIN_WATCH)
            .expect("add");

        let error = store
            .update_job(&job.id, &watch("first", "*/1 * * * *"), MIN_WATCH)
            .expect_err("invalid update must fail");
        assert_eq!(error.to_string(), "minimum watch interval is 900s");

        let mut replacement = recurring("second", "0 10 * * *");
        replacement.prompt = "new prompt".to_string();
        replacement.tools = vec!["bash".to_string(), "todo".to_string()];
        let updated = store
            .update_job(&job.id, &replacement, MIN_WATCH)
            .expect("update");
        assert_eq!(updated.name, "second");
        assert_eq!(updated.prompt, "new prompt");
        assert_eq!(updated.tools, vec!["bash".to_string(), "todo".to_string()]);
        assert_eq!(updated.schedule.as_deref(), Some("0 10 * * *"));
    }

    #[test]
    fn duplicate_name_on_update_is_rejected() {
        let (_dir, store) = store();
        store.add_job(&once("alpha", 0), MIN_WATCH).expect("alpha");
        let beta = store.add_job(&once("beta", 0), MIN_WATCH).expect("beta");

        let error = store
            .update_job(&beta.id, &once("alpha", 0), MIN_WATCH)
            .expect_err("rename onto an existing name must fail");
        assert_eq!(error.to_string(), "job name already exists");
    }

    #[test]
    fn runs_record_end_and_job_lifecycle() {
        let (_dir, store) = store();
        let job = store
            .add_job(&once("lifecycle", 0), MIN_WATCH)
            .expect("add");
        let run_id = store.record_run_start(&job.id).expect("start");
        let mut end = RunEnd::new("success");
        end.verdict = Some("MATCH".to_string());
        end.tokens_input = Some(11);
        end.tokens_output = Some(22);
        end.cost_usd = Some(0.5);
        store.record_run_end(&run_id, &end).expect("end");

        let runs = store.runs_for(&job.id, 10).expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, "success");
        assert_eq!(runs[0].verdict.as_deref(), Some("MATCH"));
        assert_eq!(runs[0].tokens_input, Some(11));
        assert_eq!(runs[0].tokens_output, Some(22));
        assert_eq!(runs[0].delivered, false);
        assert!(runs[0].finished_at.is_some());

        let disabled = store.set_enabled(&job.id, false).expect("disable");
        assert!(!disabled.enabled);
        assert_eq!(
            store
                .get_job("lifecycle")
                .expect("get")
                .last_status
                .as_deref(),
            Some("success")
        );

        store.delete_job(&job.id).expect("delete");
        assert!(matches!(
            store.get_job(&job.id).expect_err("gone"),
            AlfredError::JobNotFound(_)
        ));
        assert!(
            store.runs_for(&job.id, 10).is_err(),
            "runs are removed with the job"
        );
    }
}
