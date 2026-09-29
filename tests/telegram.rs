//! Integration tests for the Telegram channel on a Pi session (issue #12).
//!
//! Every test lives in `mod telegram` so the acceptance filter
//! `cargo test telegram` selects them. Pi is the compiled `fake-pi` double and
//! outbound traffic goes to an in-memory recorder, so no network is touched and
//! the real `~/.alfred/` is never read or written.

mod telegram {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use async_trait::async_trait;
    use serde_json::{json, Value};
    use tempfile::TempDir;

    use alfred::connectors::telegram::{
        MessageSender, Session, TelegramConnector, CHANNEL_NAME,
    };
    use alfred::error::AlfredError;
    use alfred::pi::PiInvocation;
    use alfred::server::AppState;
    use alfred::store::Store;

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

    /// Build a connector whose Pi binary is `binary`, wired to a recorder and
    /// to a temporary directory. The returned `TempDir` must stay alive for the
    /// duration of the test.
    fn fixture(
        allowed_users: Vec<u64>,
        binary: &str,
    ) -> (TelegramConnector, Arc<Recorder>, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::new(&dir.path().join("alfred.db")).unwrap());
        let (event_tx, _) = tokio::sync::broadcast::channel(16);
        let state = AppState {
            store,
            event_tx,
            start_time: Instant::now(),
            active_connections: Arc::new(AtomicUsize::new(0)),
            port: 0,
            api_key: None,
            pi: alfred::config::PiConfig::default(),
            jobs: alfred::config::JobsConfig::default(),
            telegram: None,
            pi_version: None,
        };

        let mut invocation = fixture_invocation();
        invocation.binary = binary.to_string();

        let recorder = Arc::new(Recorder::default());
        let connector = TelegramConnector::with_parts(
            "test-token".to_string(),
            allowed_users,
            state,
            invocation,
            dir.path().join("memories.md"),
        )
        .with_sender(recorder.clone());

        (connector, recorder, dir)
    }

    /// A Pi invocation that targets the compiled `fake-pi` double and keeps all
    /// paths inside the test process.
    fn fixture_invocation() -> PiInvocation {
        PiInvocation {
            binary: env!("CARGO_BIN_EXE_fake-pi").to_string(),
            provider: "fixture".to_string(),
            model: "fixture".to_string(),
            thinking: "off".to_string(),
            system_prompt: "test".to_string(),
            tools: Vec::new(),
            skills_dir: PathBuf::from("skills"),
            agent_dir: PathBuf::from("agent"),
            session_dir: PathBuf::from("sessions"),
            channel: Some(CHANNEL_NAME.to_string()),
            api_key_env: "ALFRED_TELEGRAM_TEST_UNSET".to_string(),
            extra_args: Vec::new(),
        }
    }

    fn message(update_id: i64, from: u64, chat: i64, text: &str) -> Value {
        json!({
            "update_id": update_id,
            "message": {
                "from": { "id": from },
                "chat": { "id": chat },
                "text": text,
            },
        })
    }

    #[tokio::test]
    async fn an_inbound_message_produces_a_reply_from_the_fixture() {
        let (connector, recorder, _dir) = fixture(vec![42], env!("CARGO_BIN_EXE_fake-pi"));

        connector
            .process_update(message(1, 42, 7, "hello"))
            .await
            .unwrap();

        assert_eq!(recorder.messages(), vec![(7, "reply to: hello".to_string())]);
    }

    #[tokio::test]
    async fn clear_sends_new_session_and_keeps_the_session_usable() {
        let (connector, recorder, _dir) = fixture(vec![42], env!("CARGO_BIN_EXE_fake-pi"));

        connector
            .process_update(message(1, 42, 7, "hello"))
            .await
            .unwrap();
        connector
            .process_update(message(2, 42, 7, "/clear"))
            .await
            .unwrap();
        connector
            .process_update(message(3, 42, 7, "again"))
            .await
            .unwrap();

        // The confirmation is produced only after Pi accepts `new_session`;
        // the fixture rejects commands it does not know.
        assert_eq!(
            recorder.messages(),
            vec![
                (7, "reply to: hello".to_string()),
                (7, "Session cleared.".to_string()),
                (7, "reply to: again".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn new_session_round_trips_through_the_fixture() {
        let mut session = Session::new(fixture_invocation());
        session.start().await.unwrap();
        assert!(session.is_alive());

        // A successful response proves the fixture accepted `new_session`.
        session.new_session().await.unwrap();

        let reply = session.send("after clear").await.unwrap();
        assert_eq!(reply, "reply to: after clear");
    }

    #[tokio::test]
    async fn an_unauthorized_sender_is_dropped_without_a_pi_call() {
        // The Pi binary does not exist. If the connector tried to start a
        // session it would fail and report the failure to the chat, so an
        // empty recorder proves no Pi call was made.
        let (connector, recorder, _dir) =
            fixture(vec![42], "alfred-issue12-missing-pi-binary");

        connector
            .process_update(message(1, 99, 7, "hello"))
            .await
            .unwrap();

        assert!(
            recorder.messages().is_empty(),
            "unauthorized sender must be dropped, but got: {:?}",
            recorder.messages()
        );
    }

    #[tokio::test]
    async fn a_failed_pi_session_is_reported_to_the_chat() {
        let (connector, recorder, _dir) =
            fixture(vec![42], "alfred-issue12-missing-pi-binary");

        let result = connector.process_update(message(1, 42, 7, "hello")).await;
        assert!(result.is_err(), "a missing Pi binary must surface as an error");

        let sent = recorder.messages();
        assert_eq!(sent.len(), 1, "the chat is told the session failed: {sent:?}");
        assert_eq!(sent[0].0, 7);
        assert!(
            sent[0].1.contains("Pi session error"),
            "expected a failure notice, got: {}",
            sent[0].1
        );
    }

    #[tokio::test]
    async fn remember_appends_to_the_memories_file_without_a_pi_call() {
        let (connector, recorder, dir) =
            fixture(vec![42], "alfred-issue12-missing-pi-binary");

        connector
            .process_update(message(1, 42, 7, "/remember Buy milk"))
            .await
            .unwrap();

        let content = std::fs::read_to_string(dir.path().join("memories.md")).unwrap();
        assert_eq!(content, "Buy milk\n");
        assert_eq!(recorder.messages(), vec![(7, "Remembered: Buy milk".to_string())]);
    }
}
