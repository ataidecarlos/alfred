//! Integration tests for job result delivery (issue #13).
//!
//! Every test lives in `mod delivery` so the acceptance filter
//! `cargo test delivery` selects them. Outbound traffic goes to an in-memory
//! recorder (the production path would call `send_message`, which needs a bot
//! token and the network), and the store is a temporary database, so the real
//! `~/.alfred/` is never touched and no request leaves the process.

mod delivery {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use alfred::connectors::telegram::MessageSender;
    use alfred::error::AlfredError;
    use alfred::jobs::delivery::{Delivery, DeliveryOutcome, SkipReason};
    use alfred::jobs::{Job, JobKind, JobRun, NewJob, ReportPolicy, RunEnd};
    use alfred::store::Store;

    /// The default `min_watch_interval_secs` the store validates against.
    const MIN_WATCH: u64 = 900;

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

    /// A transport that always fails, to prove a delivery failure is recorded.
    struct FailingSender;

    #[async_trait]
    impl MessageSender for FailingSender {
        async fn send(&self, _chat_id: i64, _text: &str) -> Result<(), AlfredError> {
            Err(AlfredError::Connector("telegram is down".to_string()))
        }
    }

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(&dir.path().join("delivery.db")).expect("open store");
        (dir, store)
    }

    /// Store a `digest` job with the given policy and destination.
    fn stored_job(store: &Store, report: ReportPolicy, deliver_to: Option<&str>) -> Job {
        store
            .add_job(
                &NewJob {
                    name: "digest".to_string(),
                    kind: JobKind::Recurring,
                    schedule: Some("*/5 * * * *".to_string()),
                    prompt: "ping".to_string(),
                    report,
                    deliver_to: deliver_to.map(str::to_string),
                    ..NewJob::default()
                },
                MIN_WATCH,
            )
            .expect("add job")
    }

    /// Open and close a run for `job`, returning its run id.
    fn completed_run(store: &Store, job: &Job, end: &RunEnd) -> String {
        let run_id = store.record_run_start(&job.id).expect("start run");
        store.record_run_end(&run_id, end).expect("end run");
        run_id
    }

    fn latest_run(store: &Store, job: &Job) -> JobRun {
        store
            .runs_for(&job.id, 10)
            .expect("runs")
            .into_iter()
            .next()
            .expect("a run row")
    }

    fn success_end(verdict: Option<&str>, output: &str) -> RunEnd {
        let mut end = RunEnd::new("success");
        end.verdict = verdict.map(str::to_string);
        end.output = Some(output.to_string());
        end
    }

    #[tokio::test]
    async fn always_reports_deliver_the_run_output() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::Always, Some("1234"));
        let end = success_end(None, "the digest body");
        let run_id = completed_run(&store, &job, &end);

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone());

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert_eq!(outcome, DeliveryOutcome::Delivered { chat_id: 1234 });
        assert_eq!(
            recorder.messages(),
            vec![(1234, "the digest body".to_string())]
        );
        let run = latest_run(&store, &job);
        assert!(run.delivered, "a delivered run must set delivered = 1");
        assert_eq!(run.status, "success");
    }

    #[tokio::test]
    async fn on_signal_match_delivers_the_run_output() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::OnSignal, Some("1234"));
        let end = success_end(Some("MATCH"), "found something\nVERDICT: MATCH");
        let run_id = completed_run(&store, &job, &end);

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone());

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert_eq!(outcome, DeliveryOutcome::Delivered { chat_id: 1234 });
        assert_eq!(recorder.messages().len(), 1);
        assert!(latest_run(&store, &job).delivered);
    }

    #[tokio::test]
    async fn on_signal_no_match_does_not_deliver_and_leaves_delivered_zero() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::OnSignal, Some("1234"));
        let end = success_end(Some("NO_MATCH"), "all clear\nVERDICT: NO_MATCH");
        let run_id = completed_run(&store, &job, &end);

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone());

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert_eq!(
            outcome,
            DeliveryOutcome::Skipped {
                reason: SkipReason::NotWanted
            }
        );
        assert!(
            recorder.messages().is_empty(),
            "a NO_MATCH run must not be delivered"
        );
        let run = latest_run(&store, &job);
        assert!(!run.delivered, "a skipped run must leave delivered = 0");
        assert_eq!(run.status, "success");
        assert_eq!(run.error, None, "a policy skip is not an error");
    }

    #[tokio::test]
    async fn a_send_failure_records_delivered_zero_with_the_error_while_the_run_stays_succeeded() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::Always, Some("1234"));
        let end = success_end(None, "the digest body");
        let run_id = completed_run(&store, &job, &end);

        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(Arc::new(FailingSender));

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert!(
            matches!(outcome, DeliveryOutcome::Failed { .. }),
            "outcome was: {outcome:?}"
        );
        let run = latest_run(&store, &job);
        assert_eq!(
            run.status, "success",
            "a delivery failure must not change the run status"
        );
        assert!(!run.delivered, "a failed delivery must leave delivered = 0");
        assert!(
            run.error
                .as_deref()
                .is_some_and(|error| error.contains("telegram is down")),
            "the delivery error must be recorded; error was: {:?}",
            run.error
        );
        assert_eq!(
            run.output.as_deref(),
            Some("the digest body"),
            "the run output must be retained"
        );
    }

    #[tokio::test]
    async fn deliver_to_falls_back_to_the_first_allowed_user() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::Always, None);
        let end = success_end(None, "the digest body");
        let run_id = completed_run(&store, &job, &end);

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[42, 99]).with_sender(recorder.clone());

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert_eq!(outcome, DeliveryOutcome::Delivered { chat_id: 42 });
        assert_eq!(
            recorder.messages(),
            vec![(42, "the digest body".to_string())]
        );
    }

    #[tokio::test]
    async fn no_recipient_skips_delivery_and_leaves_delivered_zero() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::Always, None);
        let end = success_end(None, "the digest body");
        let run_id = completed_run(&store, &job, &end);

        let recorder = Arc::new(Recorder::default());
        let delivery =
            Delivery::new(Some("test-token".to_string()), &[]).with_sender(recorder.clone());

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert_eq!(
            outcome,
            DeliveryOutcome::Skipped {
                reason: SkipReason::NoRecipient
            }
        );
        assert!(recorder.messages().is_empty());
        let run = latest_run(&store, &job);
        assert!(!run.delivered, "with no recipient, delivered stays 0");
        assert_eq!(run.error, None);
    }

    #[tokio::test]
    async fn a_missing_token_reaches_the_send_path_as_a_recorded_failure() {
        let (_dir, store) = temp_store();
        let job = stored_job(&store, ReportPolicy::Always, Some("1234"));
        let end = success_end(None, "the digest body");
        let run_id = completed_run(&store, &job, &end);

        // No `with_sender`: the real `ApiMessageSender` is used, but with no
        // token configured it fails before any request is built.
        let delivery = Delivery::new(None, &[]);

        let outcome = delivery.deliver(&store, &run_id, &job, &end).await;

        assert!(
            matches!(outcome, DeliveryOutcome::Failed { .. }),
            "a missing token must surface, not be ignored; outcome: {outcome:?}"
        );
        let run = latest_run(&store, &job);
        assert!(!run.delivered);
        assert!(
            run.error
                .as_deref()
                .is_some_and(|error| error.contains("bot_token")),
            "the configuration error must be recorded; error was: {:?}",
            run.error
        );
    }
}
