# Alfred Integration Test Suite

End-to-end tests that interact with Alfred via HTTP, covering server lifecycle, database interactions, and multi-turn conversations.

`cargo test` is the primary gate for this repository. The Python suite
described here is a secondary, live-API end-to-end check; it is not the gate.

## Prerequisites

1. **Build Alfred** (from project root):
   ```bash
   cargo build --release
   ```

2. **Install Python dependencies**:
   ```bash
   cd tests
   pip install -r requirements.txt
   ```

3. **Set up an API key** (only for LLM-backed tests):
   - Copy `test_api_keys.toml.example` to `test_api_keys.toml`
   - Add your OpenCode Go API key

## Test Gate

`cargo test` is the primary gate and must be green before any change is
accepted. It runs the Rust unit and integration tests, including the Pi RPC
boundary against the deterministic compiled double at `src/bin/fake-pi.rs` —
no API key and no network required.

The Python suite below requires a live API key and is **not** the primary
gate; it is opt-in only and is not what closes an issue.

## Running Tests

```bash
cd tests

# Run all tests
pytest -v

# Run only server lifecycle tests (highest priority)
pytest test_server.py -v

# Run todo tests
pytest test_todo.py -v

# Run conversation tests
pytest test_conversation.py -v

# Run with output visible (no capture)
pytest -s

# Run specific test
pytest test_server.py::TestServerLifecycle::test_health_check -v
```

## Configuration

Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| `ALFRED_BINARY` | `./target/release/alfred` | Path to Alfred binary |
| `ALFRED_TEST_PORT` | `18081` | Port for test server |
| `ALFRED_API_KEY` | (from `test_api_keys.toml`) | OpenCode Go API key |

## Test Structure

| File | Priority | Description |
|------|----------|-------------|
| `test_server.py` | **Highest** | Server start, health check, info, stop |
| `test_todo.py` | Medium | Create, list, complete, delete todos in database |
| `test_conversation.py` | Medium | Multi-turn context, tool usage, persistence |

## Rust Integration Tests

The primary tests are Rust. Run them from the project root:

```bash
cargo test
```

The Pi RPC boundary lives in `tests/pi_rpc.rs`. It exercises framing,
invocation construction, spawn/exit handling, and a full prompt round trip
against the deterministic double at `src/bin/fake-pi.rs`.

| File | Verification for | Description |
|------|------------------|-------------|
| `tests/pi_rpc.rs` | Pi RPC boundary | JSONL framing, Pi invocation args/env, process lifecycle, and a fake-pi round trip |
| `src/bin/fake-pi.rs` | Deterministic Pi stand-in | Speaks the RPC protocol on stdin/stdout with no network, key, or model |

The double is a compiled `[[bin]]` target, so the round trip runs on Windows,
Linux and macOS alike — the test locates it through `CARGO_BIN_EXE_fake-pi` and
spawns a real executable. On Windows this matters twice over: an npm `.ps1` shim
(including an npm-installed `pi`) cannot be launched by `CreateProcess`.

For a full end-to-end smoke test against a pristine container, build and run
the Docker test image:

```bash
./docker/test.sh
```

## How It Works

1. **Server Lifecycle** (`conftest.py`):
   - Creates isolated test directory with config, prompts, and database
   - Starts Alfred server as subprocess
   - Polls `/health` until ready (max 15s)
   - Tears down server after all tests complete

2. **Clean State** (`conftest.py`):
   - `autouse=True` fixture runs before every test
   - Clears the database tables the suite writes (todos, conversations)

3. **Test Isolation**:
   - Each test creates its own data with unique identifiers
   - No dependencies between tests
   - Tests can run in any order

## Adding New Tests

1. Create a new file `test_yourfeature.py`
2. Use the `server`, `client`, and `helpers` fixtures
3. Follow the pattern of existing tests

Example:
```python
def test_my_feature(self, server, client, helpers):
    c, url = client["session"], server["url"]

    reply = helpers["send_message"](c, url, "Do something")

    assert "expected" in reply.lower()
```

## Troubleshooting

**Server fails to start:**
- Check that the binary exists at `target/release/alfred`
- Check that port 18081 is available
- Look at stderr output in the test failure message

**Tests timeout:**
- LLM API calls can be slow; increase timeout in `send_message()`
- Check API key is valid in `test_api_keys.toml`
