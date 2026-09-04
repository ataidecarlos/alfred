# Alfred - 24/7 AI Agent Server

Alfred is a lightweight AI agent that runs as a 24/7 server, providing automation, notifications, and task management through multiple interfaces.

## Features

- **Multi-provider LLM support**: OpenAI, Anthropic, Google, DeepSeek
- **Built-in tools**: Webhook, Shell command, Todo management
- **Multiple interfaces**: REST API, Telegram connector, Terminal UI (TUI)
- **Scheduled tasks**: Cron-based automation
- **Persistent storage**: SQLite database for conversations, todos, and memories

## Quick Start

### Linux/Mac

```bash
curl -fsSL https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.sh | sh
```

### Windows (PowerShell)

```powershell
iwr -useb https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.ps1 | iex
```

### Manual Installation

Download the latest release from [GitHub Releases](https://github.com/ataidecarlos/alfred/releases), extract, and follow the instructions in [INSTALL.md](INSTALL.md).

## Configuration

After installation, edit your config file:

- **Linux/Mac**: `~/.config/alfred/config.toml`
- **Windows**: `%APPDATA%\alfred\config.toml`

Add your API keys:

```toml
[llm]
default_provider = "openai"

[llm.providers.openai]
api_key = "your-api-key-here"
model = "gpt-4o"
```

## Usage

### Start the server

```bash
alfred
```

### Start the TUI

```bash
alfred --tui
```

### API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/api/info` | Server info |
| POST | `/api/messages` | Send message to agent |
| GET | `/api/todos` | List all todos |
| POST | `/api/todos` | Create a todo |
| DELETE | `/api/todos/{id}` | Delete a todo |

## Documentation

- [INSTALL.md](INSTALL.md) - Detailed installation guide
- [UPGRADE.md](UPGRADE.md) - Upgrade instructions
- [RELEASE.md](RELEASE.md) - Release process

## Supported Providers

| Provider | Model Example | Base URL |
|----------|---------------|----------|
| OpenAI | gpt-4o | https://api.openai.com/v1 |
| Anthropic | claude-sonnet-4-20250514 | https://api.anthropic.com |
| Google | gemini-2.0-flash | https://generativelanguage.googleapis.com |
| DeepSeek | deepseek-v4-flash | https://api.deepseek.com |

## Development

### Build from source

```bash
git clone https://github.com/ataidecarlos/alfred.git
cd alfred
cargo build --release
```

### Run tests

```bash
cargo test
```

## License

MIT
