//! REST surface tests for issue #10.
//!
//! The router is driven in-process on an ephemeral port, so no external server
//! and no fixed port are needed. Every store is a fresh temporary database and
//! the memory file lives under an isolated home directory; the real `~/.alfred`
//! is never touched.

mod rest_api {
    use std::path::Path;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, OnceLock};
    use std::time::Instant;

    use serde_json::{json, Value};

    use alfred::config::{JobsConfig, PiConfig};
    use alfred::server::{app, AppState};
    use alfred::store::Store;

    /// Point `Paths` at a throwaway home so memories never touch `~/.alfred`.
    ///
    /// Initialised once per test binary. Every test calls this before issuing a
    /// request, and `OnceLock` blocks later callers until it is set, so no
    /// request can race the environment change.
    fn isolated_home() -> &'static Path {
        static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
        HOME.get_or_init(|| {
            let dir = tempfile::tempdir().expect("temp home");
            std::env::set_var("USERPROFILE", dir.path());
            std::env::set_var("HOME", dir.path());
            dir
        })
        .path()
    }

    struct TestServer {
        base: String,
        client: reqwest::Client,
        handle: tokio::task::JoinHandle<()>,
        store: Arc<Store>,
        _db: tempfile::TempDir,
    }

    impl TestServer {
        async fn start(api_key: Option<&str>) -> Self {
            Self::start_with(api_key, JobsConfig::default(), None).await
        }

        /// Build a server whose `AppState` carries explicit `[jobs]` and `[pi]`
        /// configuration, so a test can prove the injected values are used.
        async fn start_with(
            api_key: Option<&str>,
            jobs: JobsConfig,
            pi_version: Option<&str>,
        ) -> Self {
            isolated_home();
            let db = tempfile::tempdir().expect("temp db dir");
            let store = Arc::new(Store::new(&db.path().join("rest.db")).expect("store"));
            let state = AppState {
                store: store.clone(),
                start_time: Instant::now(),
                active_connections: Arc::new(AtomicUsize::new(0)),
                port: 0,
                api_key: api_key.map(str::to_string),
                pi: PiConfig::default(),
                jobs,
                telegram: None,
                pi_version: pi_version.map(str::to_string),
            };

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("bind ephemeral port");
            let addr = listener.local_addr().expect("local addr");
            let router = app(state);
            let handle = tokio::spawn(async move {
                let _ = axum::serve(listener, router).await;
            });

            Self {
                base: format!("http://{addr}"),
                client: reqwest::Client::new(),
                handle,
                store,
                _db: db,
            }
        }

        fn url(&self, path: &str) -> String {
            format!("{}{}", self.base, path)
        }
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.handle.abort();
        }
    }

    fn once_body(name: &str) -> Value {
        json!({
            "name": name,
            "kind": "once",
            "run_at": 2_000_000_000i64,
            "prompt": "x",
        })
    }

    #[tokio::test]
    async fn accepts_the_issue_body_and_lists_the_job() {
        let server = TestServer::start(None).await;

        let created = server
            .client
            .post(server.url("/api/jobs"))
            .json(&once_body("probe"))
            .send()
            .await
            .expect("post job");
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);
        let body: Value = created.json().await.expect("job json");
        assert_eq!(body["name"], "probe");
        assert_eq!(body["kind"], "once");
        assert_eq!(body["report"], "on_signal");
        assert_eq!(body["timeout_secs"], 900);
        assert_eq!(body["enabled"], true);

        let listed: Vec<Value> = server
            .client
            .get(server.url("/api/jobs"))
            .send()
            .await
            .expect("list")
            .json()
            .await
            .expect("list json");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], "probe");
    }

    #[tokio::test]
    async fn info_reports_pi_version_and_enabled_job_count() {
        let server = TestServer::start(None).await;

        let info: Value = server
            .client
            .get(server.url("/api/info"))
            .send()
            .await
            .expect("info")
            .json()
            .await
            .expect("info json");
        assert!(info.get("pi_version").is_some(), "info must carry pi_version: {info}");
        assert_eq!(info["jobs_enabled"], 0);

        server
            .client
            .post(server.url("/api/jobs"))
            .json(&once_body("counted"))
            .send()
            .await
            .expect("post")
            .error_for_status()
            .expect("created");

        let info: Value = server
            .client
            .get(server.url("/api/info"))
            .send()
            .await
            .expect("info")
            .json()
            .await
            .expect("info json");
        assert_eq!(info["jobs_enabled"], 1);
    }

    #[tokio::test]
    async fn api_key_is_enforced_when_configured() {
        let server = TestServer::start(Some("s3cret")).await;

        let unauth = server
            .client
            .get(server.url("/api/jobs"))
            .send()
            .await
            .expect("unauth");
        assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

        let wrong = server
            .client
            .get(server.url("/api/jobs"))
            .bearer_auth("nope")
            .send()
            .await
            .expect("wrong key");
        assert_eq!(wrong.status(), reqwest::StatusCode::UNAUTHORIZED);

        let ok = server
            .client
            .get(server.url("/api/jobs"))
            .bearer_auth("s3cret")
            .send()
            .await
            .expect("auth");
        assert_eq!(ok.status(), reqwest::StatusCode::OK);

        // Health stays open for monitoring even with a key configured.
        let health = server
            .client
            .get(server.url("/health"))
            .send()
            .await
            .expect("health");
        assert_eq!(health.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn no_api_key_configured_allows_unauthenticated_requests() {
        let server = TestServer::start(None).await;
        let resp = server
            .client
            .get(server.url("/api/jobs"))
            .send()
            .await
            .expect("list");
        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    #[tokio::test]
    async fn unknown_job_is_404_across_the_surface() {
        let server = TestServer::start(None).await;

        let get = server
            .client
            .get(server.url("/api/jobs/missing"))
            .send()
            .await
            .expect("get");
        assert_eq!(get.status(), reqwest::StatusCode::NOT_FOUND);

        let put = server
            .client
            .put(server.url("/api/jobs/missing"))
            .json(&once_body("x"))
            .send()
            .await
            .expect("put");
        assert_eq!(put.status(), reqwest::StatusCode::NOT_FOUND);

        let delete = server
            .client
            .delete(server.url("/api/jobs/missing"))
            .send()
            .await
            .expect("delete");
        assert_eq!(delete.status(), reqwest::StatusCode::NOT_FOUND);

        let run = server
            .client
            .post(server.url("/api/jobs/missing/run"))
            .send()
            .await
            .expect("run");
        assert_eq!(run.status(), reqwest::StatusCode::NOT_FOUND);

        let runs = server
            .client
            .get(server.url("/api/jobs/missing/runs"))
            .send()
            .await
            .expect("runs");
        assert_eq!(runs.status(), reqwest::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn malformed_cron_is_400_with_the_parser_message() {
        let server = TestServer::start(None).await;
        let body = json!({
            "name": "bad",
            "kind": "recurring",
            "schedule": "99 * * * *",
            "prompt": "x",
        });
        let resp = server
            .client
            .post(server.url("/api/jobs"))
            .json(&body)
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
        let text = resp.text().await.expect("body");
        assert!(text.contains("invalid cron expression"), "body was: {text}");
    }

    #[tokio::test]
    async fn watch_below_the_floor_is_400() {
        let server = TestServer::start(None).await;
        let body = json!({
            "name": "w",
            "kind": "watch",
            "schedule": "*/1 * * * *",
            "prompt": "x",
        });
        let resp = server
            .client
            .post(server.url("/api/jobs"))
            .json(&body)
            .send()
            .await
            .expect("post");
        assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
        let text = resp.text().await.expect("body");
        assert!(text.contains("minimum watch interval is 900s"), "body was: {text}");
    }

    #[tokio::test]
    async fn injected_config_controls_the_watch_floor_and_pi_version() {
        // Carried over from #10: AppState must carry the *loaded* `[jobs]`/`[pi]`
        // config, not the defaults. A 60s floor accepts a `*/1` watch that the
        // default 900s floor rejects, and the injected version is what
        // `/api/info` reports.
        let jobs = JobsConfig {
            min_watch_interval_secs: 60,
            ..JobsConfig::default()
        };
        let server = TestServer::start_with(None, jobs, Some("fixture-9.9")).await;

        let body = json!({
            "name": "fast-watch",
            "kind": "watch",
            "schedule": "*/1 * * * *",
            "prompt": "x",
        });
        let created = server
            .client
            .post(server.url("/api/jobs"))
            .json(&body)
            .send()
            .await
            .expect("post");
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);

        let info: Value = server
            .client
            .get(server.url("/api/info"))
            .send()
            .await
            .expect("info")
            .json()
            .await
            .expect("info json");
        assert_eq!(info["pi_version"], "fixture-9.9");
    }

    #[tokio::test]
    async fn update_replaces_and_delete_removes() {
        let server = TestServer::start(None).await;
        let created: Value = server
            .client
            .post(server.url("/api/jobs"))
            .json(&once_body("first"))
            .send()
            .await
            .expect("post")
            .json()
            .await
            .expect("created json");
        let id = created["id"].as_str().expect("id").to_string();

        let updated_body = json!({
            "name": "second",
            "kind": "recurring",
            "schedule": "0 9 * * *",
            "prompt": "new",
        });
        let updated: Value = server
            .client
            .put(server.url(&format!("/api/jobs/{id}")))
            .json(&updated_body)
            .send()
            .await
            .expect("put")
            .json()
            .await
            .expect("updated json");
        assert_eq!(updated["name"], "second");
        assert_eq!(updated["schedule"], "0 9 * * *");

        let deleted = server
            .client
            .delete(server.url(&format!("/api/jobs/{id}")))
            .send()
            .await
            .expect("delete");
        assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

        let gone = server
            .client
            .get(server.url(&format!("/api/jobs/{id}")))
            .send()
            .await
            .expect("get");
        assert_eq!(gone.status(), reqwest::StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn runs_endpoint_lists_recorded_runs() {
        let server = TestServer::start(None).await;
        let created: Value = server
            .client
            .post(server.url("/api/jobs"))
            .json(&once_body("runner"))
            .send()
            .await
            .expect("post")
            .json()
            .await
            .expect("created json");
        let id = created["id"].as_str().expect("id").to_string();

        server.store.record_run_start(&id).expect("record run start");

        let runs: Vec<Value> = server
            .client
            .get(server.url(&format!("/api/jobs/{id}/runs")))
            .send()
            .await
            .expect("runs")
            .json()
            .await
            .expect("runs json");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["job_id"], id);
        assert_eq!(runs[0]["status"], "running");
    }

    #[tokio::test]
    async fn run_endpoint_stubs_known_jobs_and_rejects_unknown_ones() {
        let server = TestServer::start(None).await;
        let created: Value = server
            .client
            .post(server.url("/api/jobs"))
            .json(&once_body("stub"))
            .send()
            .await
            .expect("post")
            .json()
            .await
            .expect("created json");
        let id = created["id"].as_str().expect("id").to_string();

        let resp = server
            .client
            .post(server.url(&format!("/api/jobs/{id}/run")))
            .send()
            .await
            .expect("run");
        assert_eq!(resp.status(), reqwest::StatusCode::NOT_IMPLEMENTED);
    }

    #[tokio::test]
    async fn memories_list_append_and_delete_by_slug() {
        let server = TestServer::start(None).await;

        // Start clean regardless of any leftover file from another run.
        let _ = server
            .client
            .delete(server.url("/api/memories/round-trip-memory"))
            .send()
            .await;

        let created = server
            .client
            .post(server.url("/api/memories"))
            .json(&json!({"text": "Round trip memory"}))
            .send()
            .await
            .expect("append");
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);

        let listed: Vec<Value> = server
            .client
            .get(server.url("/api/memories"))
            .send()
            .await
            .expect("list")
            .json()
            .await
            .expect("list json");
        assert!(
            listed.iter().any(|m| m["slug"] == "round-trip-memory" && m["text"] == "Round trip memory"),
            "appended memory missing from {listed:?}"
        );

        let deleted = server
            .client
            .delete(server.url("/api/memories/round-trip-memory"))
            .send()
            .await
            .expect("delete");
        assert_eq!(deleted.status(), reqwest::StatusCode::NO_CONTENT);

        let after: Vec<Value> = server
            .client
            .get(server.url("/api/memories"))
            .send()
            .await
            .expect("list")
            .json()
            .await
            .expect("list json");
        assert!(!after.iter().any(|m| m["slug"] == "round-trip-memory"));
    }
}
