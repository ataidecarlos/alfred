# Alfred Integration Test Suite

End-to-end tests that interact with Alfred via HTTP, covering server lifecycle, memory vault operations, database interactions, and multi-turn conversations.

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

3. **Set up API key** (required for LLM-powered tests):
   - Copy `test_api_keys.toml.example` to `test_api_keys.toml`
   - Add your OpenCode Go API key

## Running Tests

```bash
cd tests

# Run all tests
pytest -v

# Run only server lifecycle tests (highest priority)
pytest test_server.py -v

# Run memory tests
pytest test_memory.py -v

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
| `test_memory.py` | Medium | Store, recall, update, archive memories in vault |
| `test_todo.py` | Medium | Create, list, complete, delete todos in database |
| `test_conversation.py` | Medium | Multi-turn context, tool usage, persistence |

## How It Works

1. **Server Lifecycle** (`conftest.py`):
   - Creates isolated test directory with config, vault, and database
   - Starts Alfred server as subprocess
   - Polls `/health` until ready (max 15s)
   - Tears down server after all tests complete

2. **Clean State** (`conftest.py`):
   - `autouse=True` fixture runs before every test
   - Clears all database tables (todos, memories, conversations)
   - Removes all `.md` files from vault categories (keeps templates and `_index.md`)

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

**Vault files not found:**
- Check `ALFRED_VAULT` environment variable
- Look at the vault path in test output
