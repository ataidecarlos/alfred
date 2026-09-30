//! Integration tests for startup wiring (issue #16).
//!
//! Everything here is network-free and never touches the real `~/.alfred/`:
//!
//! 1. The composed dispatch path is proved end to end — a due job runs through
//!    the compiled `fake-pi` double, records a verdict, and its output is
//!    *delivered* through the same `JobRunner` + `Delivery` + scheduler
//!    composition the server installs.
//! 2. Startup fails fast, naming the path, when `[pi].binary` cannot be
//!    launched.
//! 3. Shutdown aborts and drops a live channel session's Pi child.
//!
//! A real Telegram send is **not** exercised: there is no bot token and no
//! network in tests, so delivery is proved against an in-memory recorder. The
//! `pi --mode rpc` orphan check is done manually (see the issue report), not
//! here, because signalling a child process is platform-specific.

mod startup {
    use std::collections::HashMap;
    use std::path::Path;
    use std::sync::{Arc, Mutex, OnceLock};

    use async_trait::async_trait;

    use alfred::config::PiConfig;
    use alfred::connectors::telegram::{shutdown_sessions, MessageSender, Session};
    use alfred::error::AlfredError;
    use alfred::jobs::delivery::Delivery;
    use alfred::jobs::dispatch::JobDispatch;
    use alfred::jobs::runner::{JobRunner, MissingVerdict, VERDICT_INSTRUCTION, VERDICT_MATCH};
    use alfred::jobs::{JobKind, NewJob, ReportPolicy};
    use alfred::pi::PiInvocation;
    use alfred::scheduler::{Scheduler, SchedulerConfig, SystemClock};
    use alfred::store::Store;

    /// The default `min_watch_interval_secs` the store validates against.
    const MIN_WATCH: u64 = 900;

    /// Point `Paths` at a throwaway home so prompt/memory reads never touch the
    /// real `~/.alfred/`. Initialised once per test binary.
    fn isolate_home() {
        static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
        HOME.get_or_init(|| {
            let dir = tempfile::tempdir().expect("temp home");
            std::env::set_var("USERPROFILE", dir.path());
            std::env::set_var("HOME", dir.path());
            dir
        });
    }

    /// Records outbound messages instead of calling the Bot API.
    #[derive(Default)]
    struct Recorder {
        sent: Mutex<Vec<(i64, String)>>,
    }

    impl Recorder {
        fn messages(&self) -> Vec<(i64, String)> {
            self.sent.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl MessageSender for Recorder {
        async fn send(&self, chat_id: i64, text: &str) -> Result<(), AlfredError> {
            self.sent.lock().unwrap().push((chat_id, text.to_string()));
            Ok(())
        }
    }

    /// A `[pi]` config whose binary is the compiled double.
    fn fixture_pi_config() -> PiConfig {
        let mut config = PiConfig::default();
        config.binary = Path::new(env!("CARGO_BIN_EXE_fake-pi"))
            .to_string_lossy()
            .into_owned();
        config.provider = "fixture-provider".to_string();
        config.model = "fixture-model".to_string();
        config.api_key_env = "ALFRED_ISSUE16_UNSET_API_KEY".to_string();
        config.timeout_secs = 30;
        config
    }

    fn temp_store() -> (tempfile::TempDir, Arc<Store>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(Store::new(&dir.path().join("startup.db")).expect("open store"));
        (dir, store)
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock after the epoch")
            .as_secs() as i64
    }

    /// Whether a process with `pid` is currently running.
    ///
    /// Platform-appropriate stand-in for the issue's `ps -ef | grep` check.
    #[cfg(windows)]
    fn process_is_running(pid: u32) -> bool {
        let Ok(output) = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
        else {
            return false;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        text.lines().any(|line| {
            line.split(',')
                .nth(1)
                .map(|field| field.trim_matches('"') == pid.to_string())
                .unwrap_or(false)
        })
    }

    #[cfg(unix)]
    fn process_is_running(pid: u32) -> bool {
        Path::new(&format!("/proc/{pid}")).exists()
    }

    /// A due `on_signal` job that delivers to an explicit chat.
    fn due_job(store: &Store, now: i64) -> alfred::jobs::Job {
        store
            .add_job(
                &NewJob {
                    name: "digest".to_string(),
                    kind: JobKind::Once,
                    run_at: Some(now - 60),
                    prompt: "check the inbox".to_string(),
                    report: ReportPolicy::OnSignal,
                    deliver_to: Some("4242".to_string()),
                    ..NewJob::default()
                },
                MIN_WATCH,
            )
            .expect("add job")
    }

    #[tokio::test]
    async fn a_due_job_runs_and_its_result_is_delivered() {
        isolate_home();
        let (_dir, store) = temp_store();
        let now = now_secs();
        let job = due_job(&store, now);

        let recorder = Arc::new(Recorder::default());
        let delivery = Arc::new(
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone()),
        );
        let runner = Arc::new(JobRunner::new(fixture_pi_config(), MissingVerdict::Notify));
        let dispatch = Arc::new(JobDispatch::new(Arc::clone(&store), runner, delivery));
        let scheduler = Arc::new(Scheduler::new(
            Arc::clone(&store),
            Arc::new(SystemClock),
            dispatch,
            SchedulerConfig::default(),
        ));

        // One real scheduler tick over the same composition the server installs.
        scheduler.tick(now).await.expect("tick");

        let run = store
            .runs_for(&job.id, 10)
            .expect("runs")
            .into_iter()
            .next()
            .expect("a run row");

        assert_eq!(run.status, "success", "the job must run, not stay running");
        assert_eq!(
            run.verdict.as_deref(),
            Some(VERDICT_MATCH),
            "the fixture reply has no verdict line, so missing_verdict=notify fails open"
        );

        // fake-pi answers `reply to: <prompt>`; the prompt is the job prompt plus
        // the verdict instruction.
        let expected = format!("reply to: check the inbox\n\n{VERDICT_INSTRUCTION}");
        assert_eq!(run.output.as_deref(), Some(expected.as_str()));
        assert!(run.delivered, "a MATCH result must be delivered");
        assert_eq!(run.tokens_input, Some(10));
        assert_eq!(run.tokens_output, Some(5));
        assert!(run.finished_at.is_some(), "the run row is closed");

        // The product promise: the result reached the recipient, not just the log.
        assert_eq!(
            recorder.messages(),
            vec![(4242, expected)],
            "the run output must be sent to the resolved chat"
        );
    }

    #[tokio::test]
    async fn an_on_signal_no_match_is_not_delivered() {
        // The complementary half of the promise: "due job runs and is delivered"
        // must not mean "every due job is delivered". A fixture reply cannot
        // produce a NO_MATCH verdict through the double, so drive the composed
        // delivery with a NO_MATCH end and assert nothing is sent.
        isolate_home();
        let (_dir, store) = temp_store();
        let now = now_secs();
        let job = due_job(&store, now);

        let run_id = store.record_run_start(&job.id).expect("start");
        let mut end = alfred::jobs::RunEnd::new("success");
        end.verdict = Some("NO_MATCH".to_string());
        end.output = Some("all clear\nVERDICT: NO_MATCH".to_string());
        store.record_run_end(&run_id, &end).expect("end");

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone());

        delivery.deliver(&store, &run_id, &job, &end).await;

        assert!(
            recorder.messages().is_empty(),
            "an on_signal NO_MATCH must not be delivered"
        );
        let run = store
            .runs_for(&job.id, 10)
            .expect("runs")
            .into_iter()
            .next()
            .expect("a run row");
        assert!(!run.delivered);
        assert_eq!(run.status, "success");
    }

    #[test]
    fn startup_exits_naming_a_non_launchable_pi_binary() {
        let home = tempfile::tempdir().expect("home");
        let missing = home.path().join("no-such-pi");
        assert!(!missing.exists(), "fixture path must not exist");

        let config = home.path().join("config.toml");
        std::fs::write(
            &config,
            format!(
                "[server]\nport = 0\n\n[prompt]\nsystem_prompt_file = \"s.md\"\nuser_prompt_file = \"u.md\"\n\n[pi]\nbinary = \"{}\"\n",
                missing.to_string_lossy().replace('\\', "\\\\")
            ),
        )
        .expect("write config");

        let output = std::process::Command::new(env!("CARGO_BIN_EXE_alfred"))
            .arg("--config")
            .arg(&config)
            .env("USERPROFILE", home.path())
            .env("HOME", home.path())
            .output()
            .expect("run alfred");

        assert!(
            !output.status.success(),
            "startup must fail; status: {:?}",
            output.status
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not executable"),
            "stderr must report the failure; stderr was: {stderr}"
        );
        assert!(
            stderr.contains("no-such-pi"),
            "stderr must name the path; stderr was: {stderr}"
        );
    }

    #[tokio::test]
    async fn shutdown_reaps_the_child_process() {
        isolate_home();
        let invocation = PiInvocation::channel(&fixture_pi_config(), "reap fixture", "telegram");
        let mut session = Session::new(invocation);
        session.start().await.expect("start session");

        let pid = session.child_pid().expect("the live child has a pid");
        assert!(
            process_is_running(pid),
            "the fixture child should be running before shutdown (pid {pid})"
        );

        session.shutdown().await;

        assert!(!session.is_alive(), "shutdown must drop the Pi child");
        assert!(
            !process_is_running(pid),
            "the child process must be reaped, not orphaned (pid {pid})"
        );
        assert!(
            session.send("after shutdown").await.is_err(),
            "a shut-down session must not have a live process"
        );
    }

    #[tokio::test]
    async fn shutdown_sessions_clears_every_live_channel() {
        isolate_home();
        let invocation =
            PiInvocation::channel(&fixture_pi_config(), "shutdown fixture", "telegram");
        let mut session = Session::new(invocation);
        session.start().await.expect("start session");

        let sessions = Arc::new(tokio::sync::Mutex::new(HashMap::from([(
            "telegram".to_string(),
            session,
        )])));
        shutdown_sessions(&sessions).await;

        assert!(
            sessions.lock().await.is_empty(),
            "shutdown must drop every channel session"
        );
    }
}
