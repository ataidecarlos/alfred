//! Issue #32: honor `server.db_path` and the `ALFRED_DATA_DIR` override.
//!
//! Every database here lives in a temporary directory. The one test that runs
//! the compiled binary gives the child a throwaway `HOME`/`USERPROFILE` *and*
//! `ALFRED_DATA_DIR`, so the real `~/.alfred/` is never touched.

use std::path::{Path, PathBuf};
use std::process::Command;

use alfred::config::{load_config, resolve_database_path};
use alfred::store::Store;

/// A minimal config body, optionally carrying `[server] db_path`.
fn config_body(db_path: Option<&Path>) -> String {
    let db_line = match db_path {
        // Forward slashes so Windows paths need no TOML escaping.
        Some(path) => format!("db_path = \"{}\"\n", path.to_string_lossy().replace('\\', "/")),
        None => String::new(),
    };
    format!(
        "[server]\nport = 0\n{db_line}\n[prompt]\nsystem_prompt_file = \"s.md\"\nuser_prompt_file = \"u.md\"\n"
    )
}

fn write_config(dir: &Path, db_path: Option<&Path>) -> PathBuf {
    let path = dir.join("config.toml");
    std::fs::write(&path, config_body(db_path)).expect("write config");
    path
}

#[test]
fn two_data_dirs_produce_two_databases() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = load_config(&write_config(dir.path(), None)).expect("load config");

    let first = tempfile::tempdir().expect("first data dir");
    let second = tempfile::tempdir().expect("second data dir");

    let first_db = resolve_database_path(&config, Some(first.path()));
    let second_db = resolve_database_path(&config, Some(second.path()));
    assert_ne!(first_db, second_db, "different data dirs must resolve differently");

    let first_store = Store::new(&first_db).expect("open first store");
    first_store
        .add_todo("only in the first", "", "medium", "")
        .expect("add todo");

    assert_eq!(first_store.list_todos().expect("list first").len(), 1);
    assert!(
        Store::new(&second_db)
            .expect("open second store")
            .list_todos()
            .expect("list second")
            .is_empty(),
        "the second database must be independent"
    );
    assert!(first_db.exists() && second_db.exists(), "both databases exist");
}

#[test]
fn explicit_db_path_is_used_when_the_override_is_unset() {
    let dir = tempfile::tempdir().expect("tempdir");
    let custom = dir.path().join("custom.db");
    let config = load_config(&write_config(dir.path(), Some(&custom))).expect("load config");

    assert_eq!(resolve_database_path(&config, None), custom);
    Store::new(&custom).expect("open custom store");
    assert!(custom.exists(), "the explicit database is created where configured");
}

#[test]
fn data_dir_override_beats_an_explicit_db_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config =
        load_config(&write_config(dir.path(), Some(Path::new("/elsewhere/other.db"))))
            .expect("load config");
    let override_dir = dir.path().join("override");

    assert_eq!(
        resolve_database_path(&config, Some(&override_dir)),
        override_dir.join("alfred.db"),
        "ALFRED_DATA_DIR must win so a run can always be isolated"
    );
}

#[test]
fn job_add_with_the_override_does_not_touch_the_default_database() {
    // The child's home. If the override were ignored, the database would land
    // at <home>/.alfred/data/alfred.db.
    let home = tempfile::tempdir().expect("child home");
    let data = tempfile::tempdir().expect("override data dir");
    let config_dir = tempfile::tempdir().expect("config dir");
    let config = write_config(config_dir.path(), None);

    let output = Command::new(env!("CARGO_BIN_EXE_alfred"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("--config")
        .arg(&config)
        .arg("job")
        .arg("add")
        .arg("--name")
        .arg("issue32-probe")
        .arg("--prompt")
        .arg("isolated")
        .arg("--at")
        .arg("+5s")
        .env("USERPROFILE", home.path())
        .env("HOME", home.path())
        .env("ALFRED_DATA_DIR", data.path())
        .env_remove("ALFRED_API_KEY")
        .output()
        .expect("run alfred");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "job add must succeed; status: {:?}\nstdout: {stdout}\nstderr: {stderr}",
        output.status
    );
    assert!(stdout.contains("issue32-probe"), "stdout was: {stdout}");

    assert!(
        data.path().join("alfred.db").exists(),
        "the override database must be created"
    );
    assert!(
        !home
            .path()
            .join(".alfred")
            .join("data")
            .join("alfred.db")
            .exists(),
        "the default home database must not be created"
    );
}
