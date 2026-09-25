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

Download the latest release from [GitHub Releases](https://github.com/ataidecarlos/alfred/releases), extract, and follow the instructions in [INSTALL.md](scripts/INSTALL.md).

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

### Update

**Linux/Mac:**
```bash
~/.local/bin/alfred/scripts/update.sh
```

**Windows (PowerShell):**
```powershell
& "$env:LOCALAPPDATA\bin\scripts\update.ps1"
```

## Scheduled Tasks

Manage OS-level scheduled jobs with a unified interface — the user crontab on
Linux/macOS and Task Scheduler on Windows:

```bash
# List scheduled jobs
alfred scheduler list

# Add a job (cron schedule + command)
alfred scheduler add '*/5 * * * *' 'echo hello'

# Remove a job
alfred scheduler remove '*/5 * * * *' 'echo hello'
```

The 5-field cron syntax and `@hourly`/`@daily`/`@weekly`/`@monthly`/`@reboot`
keywords are supported. On Windows, only `*/N * * * *`, `M H * * *`, and the
keywords above map onto Task Scheduler.

## API Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check |
| GET | `/api/info` | Server info |
| POST | `/api/messages` | Send message to agent |
| GET | `/api/todos` | List all todos |
| POST | `/api/todos` | Create a todo |
| DELETE | `/api/todos/{id}` | Delete a todo |

## Work Item System

Alfred includes a work item system for autonomous development:

```bash
# List all work items
alfred workitem list

# Get next item to work on
alfred workitem next

# Add a new work item
alfred workitem add --title "My feature" --description "Details" --priority high

# Assign to agent
alfred workitem assign <ID> <AGENT_ID>

# Update status
alfred workitem update <ID> --status in_progress

# Complete with verification
alfred workitem complete <ID> --verification "test output"
```

See [WORKITEMS.md](WORKITEMS.md) for full documentation.

## Documentation

- [INSTALL.md](scripts/INSTALL.md) - Detailed installation guide
- [UPGRADE.md](scripts/UPGRADE.md) - Upgrade instructions
- [RELEASE.md](scripts/RELEASE.md) - Release process
- [ROADMAP.md](ROADMAP.md) - Upcoming features
- [WORKITEMS.md](WORKITEMS.md) - Work item system for autonomous development

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

### Docker

Build a self-contained image that compiles Alfred from source and runs it with
a clean config:

```bash
docker build -t alfred-test .
docker run --rm -p 3000:3000 -e OPENAI_API_KEY=sk-... alfred-test
```

API keys are injected through environment variables (`OPENAI_API_KEY`,
`ANTHROPIC_API_KEY`, `GOOGLE_API_KEY`, `DEEPSEEK_API_KEY`) and expanded by
Alfred when it loads the container config. The server listens on port `3000`.

To run a command against a throwaway, freshly booted server (used for smoke
tests):

```bash
docker run --rm alfred-test curl -s localhost:3000/health
./docker/test.sh
```

## License

MIT
