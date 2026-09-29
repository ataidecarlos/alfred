//! Integration tests for the per-channel session supervisor (issue #14).
//!
//! The compaction *decision* is unit-tested with an injected clock in
//! `src/pi/session.rs`; it must never be tested with a real `sleep`. Here the
//! real [`Session`] is driven against the compiled `fake-pi` double so the
//! transport half — `get_state`, `get_session_stats`, and an accepted `compact`
//! that keeps the same process and its context — is proved end to end. No
//! network is touched and the real `~/.alfred/` is never read or written.

mod session_supervisor {
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use alfred::pi::{CompactPolicy, PiInvocation, Session, SessionSupervisor, TickOutcome, COMPACT_TICK};

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
            channel: Some("telegram".to_string()),
            api_key_env: "ALFRED_ISSUE14_UNSET_API_KEY".to_string(),
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn the_ticker_period_is_sixty_seconds() {
        assert_eq!(COMPACT_TICK, Duration::from_secs(60));
    }

    #[tokio::test]
    async fn compact_round_trips_and_keeps_the_same_context() {
        let mut session = Session::new(fixture_invocation());
        session.start().await.unwrap();
        assert!(session.is_alive());

        assert_eq!(session.send("first").await.unwrap(), "reply to: first");

        // `fake-pi` answers `compact` with success and rejects unknown
        // commands, so an `Ok` here proves the client emitted `compact`.
        session.compact().await.unwrap();

        // Continuity: the same process still answers, so no `new_session`
        // discarded the conversation.
        assert!(session.is_alive());
        assert_eq!(session.send("second").await.unwrap(), "reply to: second");
    }

    #[tokio::test]
    async fn stats_and_stream_state_come_from_the_fixture() {
        let mut session = Session::new(fixture_invocation());
        session.start().await.unwrap();

        assert!(!session.is_streaming().await.unwrap());
        assert_eq!(session.token_count().await.unwrap(), 15);
    }

    #[tokio::test]
    async fn supervisor_compacts_a_real_idle_session_end_to_end() {
        let mut session = Session::new(fixture_invocation());
        session.start().await.unwrap();
        session.send("hello").await.unwrap();

        let policy = CompactPolicy::new(60, 60_000);
        let ninety_seconds_later = Instant::now() + Duration::from_secs(90);
        let outcome = SessionSupervisor::new(&mut session, policy)
            .tick(ninety_seconds_later)
            .await;

        assert_eq!(outcome, TickOutcome::Compacted { tokens: 15 });
        assert!(session.is_alive());
        assert_eq!(session.send("after compact").await.unwrap(), "reply to: after compact");
    }

    #[tokio::test]
    async fn supervisor_leaves_a_fresh_session_alone() {
        let mut session = Session::new(fixture_invocation());
        session.start().await.unwrap();

        let outcome = SessionSupervisor::new(&mut session, CompactPolicy::new(43_200, 60_000))
            .tick(Instant::now())
            .await;

        assert!(matches!(outcome, TickOutcome::NotDue { .. }), "got {outcome:?}");
    }
}
