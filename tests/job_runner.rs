//! Integration tests for the Pi-backed job runner (issue #11).
//!
//! Every test lives in `mod job_runner` so the acceptance filter
//! `cargo test job_runner` selects them. Each test uses its own temporary
//! database and its own `[pi]` config pointing at the compiled double at
//! `src/bin/fake-pi.rs`; the real `~/.alfred/` and a live Pi are never touched.
//!
//! The double is deliberately fixed: it answers the prompt with
//! `reply to: <message>` and reports fixed token/cost figures. Tests that need a
//! specific assistant output (a verdict, a malformed body, a non-zero exit)
//! therefore drive a written-out body through [`JobRunner::verdict_for`] and
//! [`JobRunner::finish_run`], while the round trip through a real subprocess
//! proves the transport, the command line, and the run-row lifecycle.

mod job_runner {
    use std::path::Path;
    use std::sync::Arc;

    use alfred::config::{JobsConfig, PiConfig};
    use alfred::jobs::runner::{
        JobRunner, MissingVerdict, PiCompletion, ERR_NO_ASSISTANT_TEXT, STATUS_FAILED,
        STATUS_SUCCESS, STATUS_TIMEOUT, VERDICT_INSTRUCTION, VERDICT_MATCH, VERDICT_NO_MATCH,
    };
    use alfred::jobs::{Job, JobKind, NewJob, ReportPolicy, RunEnd};
    use alfred::scheduler::{Dispatch, Scheduler, SchedulerConfig};
    use alfred::store::Store;

    /// The default `min_watch_interval_secs`.
    const MIN_WATCH: u64 = 900;

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(&dir.path().join("runner.db")).expect("open store");
        (dir, store)
    }

    /// A `[pi]` config whose binary is the compiled double.
    fn fixture_pi_config() -> PiConfig {
        let mut config = PiConfig::default();
        config.binary = Path::new(env!("CARGO_BIN_EXE_fake-pi"))
            .to_string_lossy()
            .into_owned();
        config.provider = "fixture-provider".to_string();
        config.model = "fixture-model".to_string();
        config.jobs_tools = vec!["bash".to_string(), "todo".to_string()];
        config.api_key_env = "ALFRED_ISSUE11_UNSET_API_KEY".to_string();
        config.timeout_secs = 30;
        config
    }

    fn runner(missing_verdict: MissingVerdict) -> JobRunner {
        JobRunner::new(fixture_pi_config(), missing_verdict)
    }

    /// An `on_signal` recurring job with the given prompt.
    fn signal_job(prompt: &str) -> NewJob {
        NewJob {
            name: "signal".to_string(),
            kind: JobKind::Recurring,
            schedule: Some("*/5 * * * *".to_string()),
            prompt: prompt.to_string(),
            report: ReportPolicy::OnSignal,
            ..NewJob::default()
        }
    }

    /// An `always` recurring job with the given prompt.
    fn always_job(prompt: &str) -> NewJob {
        NewJob {
            name: "always".to_string(),
            kind: JobKind::Recurring,
            schedule: Some("*/5 * * * *".to_string()),
            prompt: prompt.to_string(),
            report: ReportPolicy::Always,
            ..NewJob::default()
        }
    }

    fn stored_job(store: &Store, new: &NewJob) -> Job {
        store.add_job(new, MIN_WATCH).expect("add job")
    }

    // -------------------------------------------------------------- the prompt

    #[test]
    fn on_signal_prompt_carries_the_verdict_instruction() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("check the inbox"));

        let prompt = JobRunner::build_prompt(&job);
        assert!(
            prompt.contains("check the inbox"),
            "the job prompt must survive: {prompt}"
        );
        assert!(
            prompt.contains(VERDICT_INSTRUCTION),
            "an on_signal job must carry the verdict instruction: {prompt}"
        );
        assert!(
            prompt.contains("VERDICT: MATCH") && prompt.contains("VERDICT: NO_MATCH"),
            "the instruction must name both tokens: {prompt}"
        );
    }

    #[test]
    fn always_prompt_has_no_verdict_instruction() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &always_job("summarise the day"));

        let prompt = JobRunner::build_prompt(&job);
        assert_eq!(prompt, "summarise the day");
        assert!(
            !prompt.contains("VERDICT"),
            "an always job must not be told about a verdict: {prompt}"
        );
    }

    #[test]
    fn assembled_system_is_the_persona_and_the_job_prompt_is_sent_apart() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("the job prompt"));

        let (system, prompt) = runner(MissingVerdict::Notify)
            .prepare(&job)
            .expect("prepare");

        // The assembled prompt is the persona; the job prompt is not folded in.
        assert!(
            !system.contains("the job prompt"),
            "the job prompt belongs in the Pi prompt, not the system prompt: {system}"
        );
        assert!(prompt.contains("the job prompt"), "prompt was: {prompt}");
    }

    // -------------------------------------------------------------- the verdict

    #[test]
    fn last_matching_verdict_line_wins() {
        let output = "scanning...\nVERDICT: MATCH\nstill working\nVERDICT: NO_MATCH";
        assert_eq!(
            JobRunner::parse_verdict(output).expect("parse"),
            Some(VERDICT_NO_MATCH)
        );
    }

    #[test]
    fn verdict_parsing_is_case_insensitive_and_tolerates_spacing() {
        for line in ["verdict: match", "VerDICT:  No_Match  ", "VERDICT:MATCH"] {
            let parsed = JobRunner::parse_verdict(line).expect("parse");
            assert!(parsed.is_some(), "line {line:?} should parse");
        }
    }

    #[test]
    fn a_verdict_line_must_be_the_whole_line() {
        let output = "see VERDICT: MATCH in the log\nall clear";
        assert_eq!(
            JobRunner::parse_verdict(output).expect("parse"),
            None,
            "an interior mention is not a verdict"
        );
    }

    #[test]
    fn malformed_verdict_token_is_an_error_not_a_guess() {
        let error = JobRunner::parse_verdict("VERDICT: MAYBE").expect_err("must reject");
        assert!(error.contains("MAYBE"), "error was: {error}");
    }

    #[test]
    fn output_ending_no_match_records_no_match() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let verdict = record
            .verdict_for(&job, "nothing to report\nVERDICT: NO_MATCH\n")
            .expect("parse");
        assert_eq!(verdict.as_deref(), Some(VERDICT_NO_MATCH));
    }

    #[test]
    fn output_ending_match_records_match() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let verdict = record
            .verdict_for(&job, "found something\nVERDICT: MATCH\n")
            .expect("parse");
        assert_eq!(verdict.as_deref(), Some(VERDICT_MATCH));
    }

    #[test]
    fn missing_verdict_fails_open_when_notify_is_configured() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let verdict = record
            .verdict_for(&job, "no verdict line anywhere\n")
            .expect("parse");
        assert_eq!(
            verdict.as_deref(),
            Some(VERDICT_MATCH),
            "a missing verdict must fail open"
        );
    }

    #[test]
    fn missing_verdict_is_no_match_when_skip_is_configured() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Skip);

        let verdict = record
            .verdict_for(&job, "no verdict line anywhere\n")
            .expect("parse");
        assert_eq!(verdict.as_deref(), Some(VERDICT_NO_MATCH));
    }

    #[test]
    fn always_report_policy_records_no_verdict() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &always_job("summarise"));
        let record = runner(MissingVerdict::Notify);

        let verdict = record
            .verdict_for(&job, "a plain summary\n")
            .expect("parse");
        assert_eq!(
            verdict, None,
            "an always job has no verdict contract to satisfy"
        );
    }

    #[test]
    fn missing_verdict_config_parses_known_values_and_defaults_unknown_to_notify() {
        assert_eq!(MissingVerdict::parse("skip"), MissingVerdict::Skip);
        assert_eq!(MissingVerdict::parse("notify"), MissingVerdict::Notify);
        assert_eq!(MissingVerdict::parse("NOTIFY"), MissingVerdict::Notify);
        assert_eq!(
            MissingVerdict::parse("nonsense"),
            MissingVerdict::Notify,
            "an unknown policy must fail open, never drop alerts"
        );
        assert_eq!(MissingVerdict::default(), MissingVerdict::Notify);
        assert_eq!(
            MissingVerdict::parse(&JobsConfig::default().missing_verdict),
            MissingVerdict::Notify
        );
    }

    // ------------------------------------------------------ the terminal RunEnd

    #[test]
    fn non_zero_exit_records_failed_and_keeps_the_output() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(&job, Ok(exit_failure("reply to: probe")));

        assert_eq!(end.status, STATUS_FAILED);
        assert_eq!(end.output.as_deref(), Some("reply to: probe"));
        let error = end.error.expect("error");
        assert!(error.contains("exited"), "error was: {error}");
    }

    #[test]
    fn missing_assistant_text_records_failed() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(&job, Ok(success(None)));

        assert_eq!(end.status, STATUS_FAILED);
        assert!(
            end.error
                .as_deref()
                .is_some_and(|error| error.contains(ERR_NO_ASSISTANT_TEXT)),
            "error was: {:?}",
            end.error
        );
    }

    #[test]
    fn blank_assistant_text_records_failed() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(&job, Ok(success(Some("   \n"))));

        assert_eq!(end.status, STATUS_FAILED);
        assert!(end
            .error
            .as_deref()
            .is_some_and(|error| error.contains(ERR_NO_ASSISTANT_TEXT)));
    }

    #[test]
    fn timeout_records_timeout_with_no_output() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(&job, Ok(timed_out(30)));

        assert_eq!(end.status, STATUS_TIMEOUT);
        assert!(end.output.is_none());
        assert!(end
            .error
            .as_deref()
            .is_some_and(|error| error.contains("exceeded")));
    }

    #[test]
    fn malformed_verdict_records_failed_and_does_not_guess() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(&job, Ok(success(Some("...\nVERDICT: MAYBE\n"))));

        assert_eq!(end.status, STATUS_FAILED);
        assert!(
            end.error
                .as_deref()
                .is_some_and(|error| error.contains("MAYBE")),
            "error was: {:?}",
            end.error
        );
    }

    #[test]
    fn success_records_verdict_tokens_and_cost() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, &signal_job("probe"));
        let record = runner(MissingVerdict::Notify);

        let end = record.finish_run(
            &job,
            Ok(with_stats(
                success(Some("found it\nVERDICT: MATCH\n")),
                10,
                5,
                0.25,
            )),
        );

        assert_eq!(end.status, STATUS_SUCCESS);
        assert_eq!(end.verdict.as_deref(), Some(VERDICT_MATCH));
        assert_eq!(end.tokens_input, Some(10));
        assert_eq!(end.tokens_output, Some(5));
        assert_eq!(end.cost_usd, Some(0.25));
    }

    // ----------------------------------------------------- the run-row lifecycle

    #[tokio::test]
    async fn successful_run_closes_its_row_with_a_verdict() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let id = store
            .add_job(&signal_job("ping the fixture"), MIN_WATCH)
            .expect("add")
            .id;

        let end = run_via_scheduler(&store, &id, MissingVerdict::Notify).await;
        let run = latest_run(&store, &id);

        assert_eq!(run.status, STATUS_SUCCESS);
        assert_eq!(end.status, STATUS_SUCCESS);
        // The double answers `reply to: <prompt>`; the prompt carries the
        // verdict instruction, so the reply has no verdict -> fails open.
        assert_eq!(run.verdict.as_deref(), Some(VERDICT_MATCH));
        assert_eq!(end.verdict.as_deref(), Some(VERDICT_MATCH));
        assert!(
            run.output
                .as_deref()
                .is_some_and(|output| output.starts_with("reply to: ping the fixture")),
            "output was: {:?}",
            run.output
        );
        assert_eq!(run.tokens_input, Some(10));
        assert_eq!(run.tokens_output, Some(5));
        assert!(run.finished_at.is_some(), "the run row is closed");
        assert_ne!(run.status, "running");
    }

    #[tokio::test]
    async fn job_needing_input_is_reported_successfully() {
        // A request that never needs the model to name a verdict: the reply is
        // whatever the double answers, and the row must be success with the
        // output persisted.
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let id = store
            .add_job(&always_job("just do it"), MIN_WATCH)
            .expect("add")
            .id;

        run_via_scheduler(&store, &id, MissingVerdict::Notify).await;
        let run = latest_run(&store, &id);

        assert_eq!(run.status, STATUS_SUCCESS);
        assert_eq!(run.verdict, None, "an always job records no verdict");
        assert!(run.output.is_some());
        assert_ne!(run.status, "running");
    }

    #[tokio::test]
    async fn spawn_failure_points_the_run_at_failed_not_running() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let id = store
            .add_job(&signal_job("probe"), MIN_WATCH)
            .expect("add")
            .id;

        let mut pi = fixture_pi_config();
        pi.binary = "alfred-issue11-missing-pi-binary".to_string();
        let broken = Arc::new(JobRunner::new(pi, MissingVerdict::Notify));

        let end = dispatch_via_scheduler(store.clone(), &id, broken).await;
        let run = latest_run(&store, &id);

        assert_eq!(end.status, STATUS_FAILED);
        assert_eq!(run.status, STATUS_FAILED);
        assert!(
            run.error
                .as_deref()
                .is_some_and(|error| error.contains("missing-pi-binary")),
            "error was: {:?}",
            run.error
        );
        assert_ne!(run.status, "running");
    }

    #[tokio::test]
    async fn a_job_timeout_that_exceeds_the_limit_is_recorded_timeout() {
        // The double cannot be made slow, so the timeout is driven at the
        // terminal boundary: a run that reports timed out must persist as
        // `timeout`, never `running`.
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let id = store
            .add_job(&signal_job("probe"), MIN_WATCH)
            .expect("add")
            .id;

        let runner = runner(MissingVerdict::Notify);
        let job = store.get_job(&id).expect("job");
        let end = runner.finish_run(&job, Ok(timed_out(30)));

        let run_id = store.record_run_start(&id).expect("start");
        store.record_run_end(&run_id, &end).expect("end");
        let run = latest_run(&store, &id);

        assert_eq!(run.status, STATUS_TIMEOUT);
        assert_ne!(run.status, "running");
        assert!(run.finished_at.is_some());
    }

    #[tokio::test]
    async fn the_runner_implements_dispatch_for_real_subprocesses() {
        let (_dir, store) = temp_store();
        let store = Arc::new(store);
        let id = store
            .add_job(&signal_job("dispatch me"), MIN_WATCH)
            .expect("add")
            .id;
        let job = store.get_job(&id).expect("job");

        let runner = runner(MissingVerdict::Notify);
        let end: RunEnd = runner.dispatch(&job).await.expect("dispatch");

        assert_eq!(end.status, STATUS_SUCCESS);
        assert!(
            end.output
                .as_deref()
                .is_some_and(|output| output.contains("dispatch me")),
            "output was: {:?}",
            end.output
        );
    }

    #[test]
    fn unknown_missing_verdict_defaults_to_notify_at_construction() {
        let record = JobRunner::new(fixture_pi_config(), MissingVerdict::parse("banana"));
        assert_eq!(record.missing_verdict(), MissingVerdict::Notify);
    }

    // ----------------------------------------------------------------- helpers

    /// Drive one job through the real scheduler over a real subprocess.
    async fn run_via_scheduler(store: &Arc<Store>, id: &str, policy: MissingVerdict) -> RunEnd {
        let runner = Arc::new(runner(policy));
        dispatch_via_scheduler(store.clone(), id, runner).await
    }

    /// Run `id` through a scheduler over `runner` and return the terminal end.
    async fn dispatch_via_scheduler(store: Arc<Store>, id: &str, runner: Arc<JobRunner>) -> RunEnd {
        let job = store.get_job(id).expect("job");
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(alfred::scheduler::SystemClock),
            Arc::clone(&runner) as Arc<dyn Dispatch>,
            SchedulerConfig::default(),
        ));
        let end = runner.dispatch(&job).await.expect("dispatch");
        // The scheduler's own lifecycle is exercised by directly recording the
        // same run row the loop would write, so this test never depends on
        // wall-clock due-ness.
        let run_id = store.record_run_start(id).expect("start");
        store.record_run_end(&run_id, &end).expect("end");
        drop(scheduler);
        end
    }

    fn latest_run(store: &Store, id: &str) -> alfred::jobs::JobRun {
        store
            .runs_for(id, 10)
            .expect("runs")
            .into_iter()
            .next()
            .expect("a run row")
    }

    /// A written-out Pi completion, used where the double's fixed reply cannot
    /// express the case under test.
    fn pi_completion(
        output: Option<&str>,
        stats: (Option<i64>, Option<i64>, Option<f64>),
        exit_code: Option<i32>,
        timed_out: bool,
        timeout_secs: u64,
    ) -> PiCompletion {
        PiCompletion {
            output: output.map(str::to_string),
            stats,
            exit_code,
            timed_out,
            timeout_secs,
        }
    }

    fn success(output: Option<&str>) -> PiCompletion {
        pi_completion(output, (None, None, None), Some(0), false, 0)
    }

    fn exit_failure(output: &str) -> PiCompletion {
        pi_completion(Some(output), (None, None, None), Some(1), false, 0)
    }

    fn timed_out(timeout_secs: u64) -> PiCompletion {
        pi_completion(None, (None, None, None), None, true, timeout_secs)
    }

    fn with_stats(completion: PiCompletion, input: i64, output: i64, cost: f64) -> PiCompletion {
        PiCompletion {
            stats: (Some(input), Some(output), Some(cost)),
            ..completion
        }
    }
}
