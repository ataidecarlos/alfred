# Alfred - 24/7 AI Agent Server

Alfred is a lightweight AI agent that runs as a 24/7 server, providing automation, notifications, and task management through multiple interfaces.

## Features

- **Multi-provider LLM support**: OpenCode Go, OpenAI, Anthropic, Google, DeepSeek
- **Laya decision layer**: a local confidence scorer answers routine requests directly and delegates everything else to the configured LLM
- **Built-in tools**: Shell command, Webhook, Todo management
- **Multiple interfaces**: REST API, Telegram connector, and a terminal UI (TUI) with a command palette and an interactive control center
- **Kira ticketing**: shared ticketing system for AI agents, with autonomous development support
- **Scheduler CLI**: manage OS-level cron jobs (Linux/macOS) and Windows Task Scheduler entries from one interface
- **Memory**: markdown memory vault with a SQLite index and Laya-aware retrieval
- **Config hot-reload**: config file edits are applied without restarting the server
- **Persistent storage**: SQLite database for conversations, todos, memories, and work items
- **Docker image**: self-contained image that compiles Alfred from source for pristine, throwaway testing

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

Alfred keeps all of its state under `~/.alfred` (`%USERPROFILE%\.alfred` on Windows):

| Path | Purpose |
|------|---------|
| `~/.alfred/config/config.toml` | Main configuration file |
| `~/.alfred/config/prompts/` | System and user prompt files |
| `~/.alfred/config/themes/` | TUI theme files |
| `~/.alfred/data/alfred.db` | SQLite database (conversations, todos, memories, work items) |
| `~/.alfred/logs/alfred.log` | Server log |

On first run Alfred creates the config from the bundled example if it is missing. Edit it and add your API keys:

- **Linux/Mac**: `~/.alfred/config/config.toml`
- **Windows**: `%USERPROFILE%\.alfred\config\config.toml`

```toml
[server]
port = 8080
host = "0.0.0.0"

[llm]
default_provider = "opencode-go"

[llm.providers.opencode-go]
api_key = "${OPENCODE_GO_API_KEY}"
model = "space-bunny-free"
base_url = "https://opencode.ai/zen/go/v1"
```

`${VAR}` references are expanded from the environment when the config is loaded, so keys can be injected without writing them to disk. Changes to the config are hot-reloaded: Alfred picks them up without a restart.

## Usage

### Start the server

```bash
alfred
```

### Start the TUI

```bash
alfred --tui
```

The TUI connects to a running server (starting one automatically if needed). Press `Ctrl+P` for the command palette and `Ctrl+K` to open the control center, which shows scheduler jobs, todos, memories, and connected channels. Slash commands such as `/todos`, `/memories`, and `/config` are available from the prompt.

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
| PUT | `/api/todos/{id}` | Update a todo |
| DELETE | `/api/todos/{id}` | Delete a todo |
| POST | `/api/todos/{id}/complete` | Mark a todo complete |
| GET | `/api/memories` | List all memories |
| POST | `/api/memories` | Create a memory |
| DELETE | `/api/memories/{id}` | Delete a memory |

## Kira Ticketing System

Alfred uses Kira for ticket management. Kira is a shared ticketing system for AI agents across all projects.

### Autonomous Development

```bash
# Setup cron job
bash scripts/setup_autonomous_cron.sh

# Manual trigger
bash scripts/run_autonomous_agent.sh

# Monitor
tail -f ~/.alfred/agent.log
```

See [WORKITEMS.md](WORKITEMS.md) for full documentation.

## Documentation

- [INSTALL.md](scripts/INSTALL.md) - Detailed installation guide
- [UPGRADE.md](scripts/UPGRADE.md) - Upgrade instructions
- [RELEASE.md](scripts/RELEASE.md) - Release process
- [ROADMAP.md](ROADMAP.md) - Upcoming features
- [WORKITEMS.md](WORKITEMS.md) - Kira ticketing system for autonomous development

## Supported Providers

| Provider | Model Example | Base URL |
|----------|---------------|----------|
| OpenCode Go | space-bunny-free | https://opencode.ai/zen/go/v1 |
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
