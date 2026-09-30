//! Integration tests for the Pi RPC boundary (issue #5).
//!
//! Every test lives in `mod pi_rpc` so the acceptance filter
//! `cargo test pi_rpc` selects them.

mod pi_rpc {
    use std::collections::HashMap;
    use std::io;
    use std::path::Path;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use serde_json::json;
    use tokio::io::{AsyncRead, ReadBuf};
    use tokio::process::Command;

    use alfred::config::PiConfig;
    use alfred::error::AlfredError;
    use alfred::paths::Paths;
    use alfred::pi::{JsonlReader, PiClient, PiInvocation};

    /// A reader that hands out one byte per poll, to prove framing buffers
    /// across arbitrary chunk boundaries (including inside a UTF-8 sequence).
    struct OneByteAtATime<'a> {
        bytes: &'a [u8],
    }

    impl<'a> AsyncRead for OneByteAtATime<'a> {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            let bytes = this.bytes;
            if let Some((first, rest)) = bytes.split_first() {
                buf.put_slice(std::slice::from_ref(first));
                this.bytes = rest;
            }
            Poll::Ready(Ok(()))
        }
    }

    // ---------------------------------------------------------------- framing

    #[tokio::test]
    async fn framing_never_splits_on_u2028_or_u2029() {
        let line = "{\"type\":\"message_update\",\"text\":\"left\u{2028}middle\u{2029}right\"}\n";
        let mut reader = JsonlReader::new(OneByteAtATime {
            bytes: line.as_bytes(),
        });
        let message = reader.next_message().await.unwrap().expect("one message");
        assert_eq!(message["text"], "left\u{2028}middle\u{2029}right");
        assert!(reader.next_message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn framing_strips_a_trailing_cr() {
        let data = "{\"first\":1}\r\n{\"second\":2}\n{\"third\":3}\r";
        let mut reader = JsonlReader::new(data.as_bytes());
        assert_eq!(reader.next_message().await.unwrap().unwrap()["first"], 1);
        assert_eq!(reader.next_message().await.unwrap().unwrap()["second"], 2);
        // A final record without LF, terminated only by CR, is still a record.
        assert_eq!(reader.next_message().await.unwrap().unwrap()["third"], 3);
        assert!(reader.next_message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unparseable_lines_are_skipped_not_fatal() {
        let data = "not json at all\n\n{\"type\":\"response\",\"success\":true}\n";
        let mut reader = JsonlReader::new(data.as_bytes());
        let message = reader
            .next_message()
            .await
            .unwrap()
            .expect("valid record after garbage");
        assert_eq!(message["type"], "response");
        assert_eq!(message["success"], true);
        assert!(reader.next_message().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn invalid_utf8_lines_are_skipped_not_fatal() {
        let data: &[u8] = b"\xff\xfe not utf8\n{\"type\":\"agent_settled\"}\n";
        let mut reader = JsonlReader::new(data);
        let message = reader
            .next_message()
            .await
            .unwrap()
            .expect("valid record after invalid utf8");
        assert_eq!(message["type"], "agent_settled");
        assert!(reader.next_message().await.unwrap().is_none());
    }

    // ------------------------------------------------------------- invocation

    #[test]
    fn job_invocation_is_exact() {
        let config = test_pi_config();
        let invocation = PiInvocation::job(&config, "assembled system prompt");
        let command = invocation.command();

        assert_eq!(
            command.as_std().get_program().to_string_lossy(),
            "pi-fixture"
        );

        let expected: Vec<String> = vec![
            "--mode".into(),
            "rpc".into(),
            "--no-session".into(),
            "--provider".into(),
            config.provider.clone(),
            "--model".into(),
            config.model.clone(),
            "--thinking".into(),
            config.thinking.clone(),
            "--system-prompt".into(),
            "assembled system prompt".into(),
            "--no-context-files".into(),
            "--no-approve".into(),
            "--tools".into(),
            config.jobs_tools.join(","),
            "--skill".into(),
            Paths::skills_dir().to_string_lossy().into_owned(),
        ];
        assert_eq!(invocation_args(&command), expected);

        let envs = invocation_envs(&command);
        assert_eq!(
            env_of(&envs, "PI_CODING_AGENT_DIR"),
            Paths::pi_agent_dir().to_string_lossy().into_owned()
        );
        assert_eq!(
            env_of(&envs, "PI_CODING_AGENT_SESSION_DIR"),
            config.session_dir.clone()
        );
        assert_eq!(env_of(&envs, "PI_SKIP_VERSION_CHECK"), "1");
        assert_eq!(env_of(&envs, "PI_TELEMETRY"), "0");
        assert_eq!(env_of(&envs, "PI_OFFLINE"), "1");
    }

    #[test]
    fn channel_invocation_is_exact() {
        let config = test_pi_config();
        let invocation = PiInvocation::channel(&config, "channel prompt", "telegram");
        let command = invocation.command();

        let channel_session_dir = Paths::pi_dir().join("telegram");
        let expected: Vec<String> = vec![
            "--mode".into(),
            "rpc".into(),
            "--provider".into(),
            config.provider.clone(),
            "--model".into(),
            config.model.clone(),
            "--thinking".into(),
            config.thinking.clone(),
            "--system-prompt".into(),
            "channel prompt".into(),
            "--no-context-files".into(),
            "--no-approve".into(),
            "--tools".into(),
            config.channel_tools.join(","),
            "--skill".into(),
            Paths::skills_dir().to_string_lossy().into_owned(),
            "--session-dir".into(),
            channel_session_dir.to_string_lossy().into_owned(),
            "--name".into(),
            "telegram".into(),
        ];
        let args = invocation_args(&command);
        assert_eq!(args, expected);
        assert!(
            !args.iter().any(|arg| arg == "--no-session"),
            "channels keep their session: {args:?}"
        );

        let envs = invocation_envs(&command);
        assert_eq!(
            env_of(&envs, "PI_CODING_AGENT_SESSION_DIR"),
            channel_session_dir.to_string_lossy().into_owned()
        );
    }

    #[test]
    fn api_key_is_passed_by_environment_only() {
        const KEY_VAR: &str = "ALFRED_ISSUE5_TEST_PI_KEY";
        const KEY_VALUE: &str = "sk-issue5-secret-value";

        std::env::set_var(KEY_VAR, KEY_VALUE);
        let mut config = test_pi_config();
        config.api_key_env = KEY_VAR.to_string();
        let command = PiInvocation::channel(&config, "sys", "telegram").command();
        std::env::remove_var(KEY_VAR);

        assert_eq!(env_of(&invocation_envs(&command), KEY_VAR), KEY_VALUE);
        let args = invocation_args(&command);
        assert!(!args.iter().any(|arg| arg == "--api-key"), "args: {args:?}");
        assert!(
            !args.iter().any(|arg| arg.contains(KEY_VALUE)),
            "args leak the key: {args:?}"
        );
    }

    #[test]
    fn extra_args_are_appended_after_the_standard_flags() {
        let mut config = test_pi_config();
        config.extra_args = vec!["--verbose".into(), "--no-extensions".into()];
        let args = invocation_args(&PiInvocation::job(&config, "sys").command());
        assert_eq!(
            &args[args.len() - 2..],
            ["--verbose".to_string(), "--no-extensions".to_string()]
        );
    }

    // ---------------------------------------------------------------- process

    #[tokio::test]
    async fn failed_spawn_names_the_binary_path() {
        let binary = "alfred-issue5-missing-pi-binary";
        let error = PiClient::spawn(binary, Command::new(binary))
            .await
            .unwrap_err();
        match error {
            AlfredError::PiSpawn { binary: named, .. } => assert_eq!(named, binary),
            other => panic!("expected PiSpawn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stream_end_reports_process_exit_with_captured_stderr() {
        let mut client = PiClient::spawn("failing-fixture", failing_process())
            .await
            .unwrap();
        let error = client.next_message().await.unwrap_err();
        match error {
            AlfredError::PiProcessExited {
                binary,
                status,
                stderr,
            } => {
                assert_eq!(binary, "failing-fixture");
                assert!(!status.is_empty());
                assert!(
                    stderr.contains("simulated pi failure"),
                    "stderr was {stderr:?}"
                );
            }
            other => panic!("expected PiProcessExited, got {other:?}"),
        }
    }

    // ----------------------------------------------------------------- fixture

    #[tokio::test]
    async fn round_trip_prompt_through_fake_pi() {
        // A compiled `[[bin]]` target, located by cargo itself, is a real
        // executable on every platform — unlike the POSIX shell double it
        // replaces and unlike an npm `.ps1` shim on Windows.
        let binary = Path::new(env!("CARGO_BIN_EXE_fake-pi"));
        assert!(binary.is_file(), "missing fixture at {}", binary.display());

        let mut config = test_pi_config();
        config.binary = binary.to_string_lossy().into_owned();

        let invocation = PiInvocation::job(&config, "fixture system prompt");
        let mut client = PiClient::spawn(&config.binary, invocation.command())
            .await
            .unwrap();

        // Responses echo the id, generated or explicit.
        let state = client.request(json!({"type": "get_state"})).await.unwrap();
        assert!(state.success, "get_state failed: {state:?}");
        assert!(state.id.is_some(), "response should echo a generated id");
        assert_eq!(state.data.as_ref().unwrap()["model"]["id"], "fake-model");

        let explicit = client
            .request(json!({"id": "custom-1", "type": "get_state"}))
            .await
            .unwrap();
        assert_eq!(explicit.id, Some(json!("custom-1")));

        // Prompt in, assistant text out. The fixture also emits one malformed
        // line mid-stream, which the client must skip.
        let accepted = client
            .request(json!({"type": "prompt", "message": "ping"}))
            .await
            .unwrap();
        assert!(accepted.success, "prompt failed: {accepted:?}");

        let mut saw_agent_start = false;
        let mut saw_message_end = false;
        let mut settled = false;
        for _ in 0..64 {
            if settled {
                break;
            }
            let message = client.next_message().await.unwrap();
            match message["type"].as_str() {
                Some("agent_start") => saw_agent_start = true,
                Some("message_end") => saw_message_end = true,
                Some("agent_settled") => settled = true,
                _ => {}
            }
        }
        assert!(settled, "fixture never emitted agent_settled");
        assert!(
            saw_agent_start && saw_message_end,
            "missing lifecycle events"
        );

        let text = client
            .request(json!({"type": "get_last_assistant_text"}))
            .await
            .unwrap();
        assert_eq!(text.data.as_ref().unwrap()["text"], "reply to: ping");

        let stats = client
            .request(json!({"type": "get_session_stats"}))
            .await
            .unwrap();
        assert!(stats.success, "get_session_stats failed: {stats:?}");
    }

    // ----------------------------------------------------------------- helpers

    fn test_pi_config() -> PiConfig {
        PiConfig {
            binary: "pi-fixture".to_string(),
            api_key_env: "ALFRED_ISSUE5_UNSET_API_KEY".to_string(),
            provider: "fixture-provider".to_string(),
            model: "fixture-model".to_string(),
            thinking: "off".to_string(),
            jobs_tools: vec!["bash".to_string(), "todo".to_string()],
            channel_tools: vec!["bash".to_string()],
            timeout_secs: 60,
            idle_compact_secs: 60,
            compact_token_threshold: 1000,
            session_dir: Paths::pi_dir()
                .join("sessions")
                .to_string_lossy()
                .into_owned(),
            extra_args: Vec::new(),
        }
    }

    fn invocation_args(command: &Command) -> Vec<String> {
        command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn invocation_envs(command: &Command) -> HashMap<String, String> {
        command
            .as_std()
            .get_envs()
            .map(|(key, value)| {
                (
                    key.to_string_lossy().into_owned(),
                    value
                        .map(|v| v.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
            })
            .collect()
    }

    fn env_of(envs: &HashMap<String, String>, key: &str) -> String {
        envs.get(key)
            .cloned()
            .unwrap_or_else(|| panic!("environment variable {key} was not set"))
    }

    #[cfg(unix)]
    fn failing_process() -> Command {
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg("echo 'simulated pi failure' 1>&2; exit 3");
        command
    }

    #[cfg(windows)]
    fn failing_process() -> Command {
        let mut command = Command::new("cmd");
        command
            .arg("/C")
            .arg("echo simulated pi failure 1>&2 & exit 3");
        command
    }
}
