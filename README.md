# Alfred - 24/7 AI Agent Server

Alfred is a lightweight AI agent that runs as a 24/7 server, providing automation, notifications, and task management through multiple interfaces.

## Features

- **Multi-provider LLM support**: OpenAI, Anthropic, Google, DeepSeek
- **Built-in tools**: Webhook, Shell command, Todo management
- **Multiple interfaces**: REST API, Telegram connector, Terminal UI (TUI)
- **Scheduled tasks**: Cron-based automation
- **Persistent storage**: SQLite database for conversations, todos, and memories

## Quick Start

### 1. Build

```bash
cargo build --release
```

### 2. First Run

On first run, Alfred will auto-generate configuration files:

```bash
cargo run
```

This creates:
- `config/config.toml` - Main configuration (edit with your API keys)
- `prompts/system.md` - System prompt (customize as needed)
- `prompts/user.md` - User-specific instructions

### 3. Configure

Edit `config/config.toml`:

```toml
[server]
port = 8080
host = "0.0.0.0"
db_path = "data/alfred.db"

[llm]
default_provider = "openai"  # or anthropic, google, deepseek

[llm.providers.openai]
api_key = "your-api-key-here"
model = "gpt-4o"
```

### 4. Run

**Server mode** (HTTP API + Telegram):
```bash
cargo run
```

**TUI mode** (interactive chat):
```bash
cargo run -- --tui
```

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/api/info` | Server info (PID, uptime, connections) |
| POST | `/api/messages` | Send message to agent |
| GET | `/api/todos` | List all todos |
| POST | `/api/todos` | Create a todo |
| DELETE | `/api/todos/{id}` | Delete a todo |
| GET | `/api/memories` | List all memories |
| POST | `/api/memories` | Create a memory |
| DELETE | `/api/memories/{id}` | Delete a memory |

### Example: Send a Message

```bash
curl -X POST http://localhost:8080/api/messages \
  -H "Content-Type: application/json" \
  -d '{"user_id":"user1","text":"Add a todo: Buy groceries"}'
```

## Configuration

### Environment Variables

API keys can use environment variables:

```toml
[llm.providers.openai]
api_key = "${OPENAI_API_KEY}"
```

### Supported Providers

| Provider | Model Example | Base URL |
|----------|---------------|----------|
| OpenAI | gpt-4o | https://api.openai.com/v1 |
| Anthropic | claude-sonnet-4-20250514 | https://api.anthropic.com |
| Google | gemini-2.0-flash | https://generativelanguage.googleapis.com |
| DeepSeek | deepseek-v4-flash | https://api.deepseek.com |

### Telegram Setup

1. Create a bot with @BotFather
2. Add to config:

```toml
[telegram]
bot_token = "${TELEGRAM_BOT_TOKEN}"
allowed_users = [123456789]  # Your Telegram user ID
```

## Project Structure

```
alfred/
├── Cargo.toml
├── README.md
├── config/
│   ├── config.toml          # Your config (gitignored)
│   └── config.toml.example  # Template
├── prompts/
│   ├── system.md            # Your system prompt (gitignored)
│   ├── system.md.example    # Template
│   ├── user.md              # Your user prompt (gitignored)
│   └── user.md.example      # Template
├── data/
│   └── alfred.db            # SQLite database (gitignored)
├── logs/
│   └── server.log           # Application logs (gitignored)
└── src/
    ├── main.rs              # Entry point
    ├── config.rs            # Configuration loading
    ├── llm/                 # LLM providers
    ├── agent/               # Agent loop
    ├── tools/               # Built-in tools
    ├── server/              # HTTP server
    ├── connectors/          # Telegram, etc.
    ├── scheduler/           # Task scheduling
    ├── tui/                 # Terminal UI
    └── store/               # SQLite persistence
```

## Development

### Build

```bash
cargo build
```

### Run with Debug Logging

```bash
RUST_LOG=debug cargo run
```

### Run Tests

```bash
cargo test
```

## Troubleshooting

### Config Not Found

If you see "No config file found", Alfred will auto-generate one from the template. Edit `config/config.toml` with your settings.

### API Key Errors

Ensure your API keys are set either in the config file or as environment variables.

### Port Already in Use

If the port is occupied, Alfred will show info about the running instance. Stop it first or change the port in config.

## License

MIT
