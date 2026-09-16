"""
Alfred Integration Test Suite - Fixtures

Fixtures for server lifecycle, clean state, and test helpers.
"""

import os
import sqlite3
import subprocess
import sys
import time
import toml
import pytest
import requests
from pathlib import Path


# ---------------------------------------------------------------------------
# Configuration helpers
# ---------------------------------------------------------------------------

def find_alfred_binary() -> Path:
    """Find the Alfred binary, checking env var and common locations."""
    env_path = os.environ.get("ALFRED_BINARY")
    if env_path:
        p = Path(env_path)
        if p.exists():
            return p

    # Check target/release/alfred (Linux or Windows)
    for ext in ["", ".exe"]:
        p = Path(__file__).parent.parent / "target" / "release" / f"alfred{ext}"
        if p.exists():
            return p

    raise FileNotFoundError(
        "Alfred binary not found. Build with `cargo build --release` "
        "or set ALFRED_BINARY env var."
    )


def find_database_path() -> Path:
    """Find the actual database path Alfred uses (Paths::database_file())."""
    if sys.platform == "win32":
        appdata = os.environ.get("APPDATA", "")
        return Path(appdata) / "alfred" / "alfred.db"
    else:
        xdg = os.environ.get("XDG_CONFIG_HOME", "")
        if xdg:
            return Path(xdg) / "alfred" / "alfred.db"
        home = os.environ.get("HOME", "")
        return Path(home) / ".config" / "alfred" / "alfred.db"


def find_api_key() -> str:
    """Read API key from test_api_keys.toml if available."""
    # Check project root
    key_file = Path(__file__).parent.parent / "test_api_keys.toml"
    if key_file.exists():
        config = toml.load(key_file)
        key = config.get("opencode_go", {}).get("api_key", "")
        if key:
            return key

    # Check environment variable
    env_key = os.environ.get("OPENCODE_GO_API_KEY", "")
    if env_key:
        return env_key

    return ""


# ---------------------------------------------------------------------------
# Server lifecycle fixture (session scope)
# ---------------------------------------------------------------------------

@pytest.fixture(scope="session")
def test_environment(tmp_path_factory):
    """Create an isolated test environment (config, vault, database)."""
    test_dir = tmp_path_factory.mktemp("alfred_test")
    vault_dir = test_dir / "vault"
    log_dir = test_dir / "logs"
    prompts_dir = test_dir / "prompts"

    vault_dir.mkdir()
    log_dir.mkdir()
    prompts_dir.mkdir()

    # Create dummy prompt files with tool instructions
    (prompts_dir / "system.md").write_text(
        "You are Alfred, a helpful AI assistant for testing.\n\n"
        "You have access to these tools:\n"
        "- todo: Manage to-do items (add, list, complete, delete)\n"
        "- memory: Manage memories (store, recall, update, archive)\n"
        "- shell: Execute shell commands\n"
        "- webhook: Send HTTP requests to external services\n\n"
        "When the user asks you to create a todo, use the todo tool with action='add'.\n"
        "When the user asks you to remember something, use the memory tool with action='store'.\n"
        "When the user asks you to recall something, use the memory tool with action='recall'.\n"
        "Always confirm actions before executing destructive operations."
    )
    (prompts_dir / "user.md").write_text("")

    port = int(os.environ.get("ALFRED_TEST_PORT", "18081"))
    api_key = find_api_key()
    db_path = find_database_path()

    config = {
        "server": {
            "port": port,
            "host": "127.0.0.1",
            "db_path": str(db_path),
        },
        "llm": {
            "default_provider": "openai",
            "providers": {
                "openai": {
                    "api_key": api_key or "test-key",
                    "model": "glm-5.3-flash",
                    "base_url": "https://opencode.ai/zen/go/v1",
                },
            },
        },
        "prompt": {
            "system_prompt_file": str(prompts_dir / "system.md"),
            "user_prompt_file": str(prompts_dir / "user.md"),
        },
        "scheduler": {
            "enabled": False,
        },
        "memory": {
            "enabled": True,
            "vault_path": str(vault_dir),
        },
    }

    config_file = test_dir / "config.toml"
    config_file.write_text(toml.dumps(config))

    return {
        "test_dir": test_dir,
        "config": config,
        "config_file": config_file,
        "vault_path": vault_dir,
        "db_path": db_path,
        "log_dir": log_dir,
        "port": port,
        "url": f"http://127.0.0.1:{port}",
    }


def wait_for_server(url: str, timeout: float = 15.0, interval: float = 0.5) -> bool:
    """Poll /health until server is ready or timeout."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            resp = requests.get(f"{url}/health", timeout=2)
            if resp.status_code == 200:
                return True
        except (requests.ConnectionError, requests.Timeout):
            pass
        time.sleep(interval)
    return False


@pytest.fixture(scope="session")
def server(test_environment):
    """Start the Alfred server for the entire test session."""
    import shutil

    env = test_environment
    binary = find_alfred_binary()
    project_root = Path(__file__).parent.parent

    # Alfred's auto_generate_config_files runs before --config is parsed,
    # so it copies config/config.toml.example over our test config.
    # Temporarily rename the example to prevent this.
    example_config = project_root / "config" / "config.toml.example"
    example_backup = project_root / "config" / "config.toml.example.bak"
    if example_config.exists():
        shutil.move(example_config, example_backup)

    # Also remove legacy config if it exists
    legacy_config = project_root / "config" / "config.toml"
    legacy_backup = project_root / "config" / "config.toml.bak"
    if legacy_config.exists():
        shutil.move(legacy_config, legacy_backup)

    # Write our test config to the default location
    default_config_dir = Path.home() / ".config" / "alfred"
    default_config_dir.mkdir(parents=True, exist_ok=True)
    default_config_file = default_config_dir / "config.toml"
    shutil.copy2(env["config_file"], default_config_file)

    # Remove stale database if it exists
    if env["db_path"].exists():
        env["db_path"].unlink()

    process = subprocess.Popen(
        [str(binary), "--config", str(default_config_file)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        cwd=str(project_root),
    )

    # Wait for server to be ready
    ready = wait_for_server(env["url"], timeout=15)
    if not ready:
        process.terminate()
        process.wait(timeout=5)
        stdout, stderr = process.communicate()
        # Restore backups
        if example_backup.exists():
            shutil.move(example_backup, example_config)
        if legacy_backup.exists():
            shutil.move(legacy_backup, legacy_config)
        pytest.fail(
            f"Server failed to start within 15s.\n"
            f"stdout: {stdout.decode()}\n"
            f"stderr: {stderr.decode()}"
        )

    yield {
        "process": process,
        **env,
    }

    # Teardown: stop server
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=3)

    # Restore original files
    if example_backup.exists():
        shutil.move(example_backup, example_config)
    if legacy_backup.exists():
        shutil.move(legacy_backup, legacy_config)


# ---------------------------------------------------------------------------
# HTTP client fixture
# ---------------------------------------------------------------------------

@pytest.fixture
def client(server):
    """Provide a requests session pre-configured with the server URL."""
    session = requests.Session()
    session.headers.update({"Content-Type": "application/json"})
    yield {"session": session, "url": server["url"]}


# ---------------------------------------------------------------------------
# Clean state fixture (runs before every test)
# ---------------------------------------------------------------------------

@pytest.fixture(autouse=True)
def clean_state(server):
    """Reset database and vault files before each test."""
    db_path = server["db_path"]
    vault_path = server["vault_path"]

    # Clean database tables (only if database exists and has tables)
    if db_path.exists():
        try:
            conn = sqlite3.connect(str(db_path))
            # Check if tables exist before trying to delete
            cursor = conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table'"
            )
            tables = {row[0] for row in cursor.fetchall()}
            for table in ["todos", "memories", "conversations"]:
                if table in tables:
                    conn.execute(f"DELETE FROM {table}")
            conn.commit()
            conn.close()
        except sqlite3.Error:
            pass  # Database might not be ready yet

    # Clean vault files (keep structure, templates, and _index.md)
    if vault_path.exists():
        for category in ["preferences", "facts", "decisions", "lessons", "action-items"]:
            cat_dir = vault_path / category
            if cat_dir.exists():
                for f in cat_dir.glob("*.md"):
                    if f.name != "_index.md":
                        f.unlink()

    yield

    # Post-test cleanup is handled by the session-scoped server fixture


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

@pytest.fixture
def helpers():
    """Provide test helper functions."""
    def send_message(session, url, text, user_id="test-user", timeout=60):
        """Send a message to Alfred and return the reply text."""
        resp = session.post(
            f"{url}/api/messages",
            json={"user_id": user_id, "text": text},
            timeout=timeout,
        )
        resp.raise_for_status()
        return resp.json()["reply"]

    def get_vault_files(vault_path, category):
        """Return list of .md files in a vault category (excluding _index.md)."""
        cat_dir = vault_path / category
        if not cat_dir.exists():
            return []
        return sorted([
            f for f in cat_dir.glob("*.md") if f.name != "_index.md"
        ])

    def read_vault_file(path):
        """Read and return the content of a vault file."""
        return path.read_text(encoding="utf-8")

    def get_database_rows(db_path, table):
        """Return all rows from a database table."""
        conn = sqlite3.connect(str(db_path))
        conn.row_factory = sqlite3.Row
        cursor = conn.execute(f"SELECT * FROM {table}")
        rows = [dict(row) for row in cursor.fetchall()]
        conn.close()
        return rows

    return {
        "send_message": send_message,
        "get_vault_files": get_vault_files,
        "read_vault_file": read_vault_file,
        "get_database_rows": get_database_rows,
    }
