# Alfred — 24x7 personal-assistant host

Alfred is a 24x7 personal-assistant host. It owns the product surface: the job
model (schedule + prompt + delivery), the scheduler, run history, delivery to
the user, prompt and memory files, the REST API, and the Telegram channel.

Alfred does **not** own the agent. The agent loop, LLM providers, tool calling,
sessions, compaction, and streaming are delegated to **Pi**
(https://github.com/earendil-works/pi, MIT), which Alfred runs as a
`pi --mode rpc` subprocess. Alfred is not a coding agent and it has no terminal
UI: you interact through Telegram or the REST API.

## Release status

> **The published releases are still the previous, pre-rewrite product.** The
> only tag so far is `v2026.09.04`, and no tag has been cut since the rewrite,
> so `curl -fsSL .../scripts/install.sh | sh` installs the old binary. Build
> from source (below) to run the code in this repository.

## Requirements

- **Rust** 1.97+ to build.
- **Pi** on `PATH` — a runtime dependency, not a build dependency. Node >= 22.19
  if you install Pi with npm.
- A provider API key exposed through the environment variable named by
  `[pi].api_key_env` (default `PI_API_KEY`).

### Windows

Alfred launches Pi with `CreateProcess`, which cannot run an npm `.cmd`/`.ps1`
shim. On Windows, point `[pi].binary` at a native `pi.exe` (the standalone Pi
binary). If it points at the npm shim, Alfred fails fast at startup with:

```
ERROR: config error: [pi].binary is not executable: pi
```

## Quick start (from source)

```bash
git clone https://github.com/ataidecarlos/alfred.git
cd alfred
cargo build --release
```

Install Pi, export the provider key, and start the server:

```bash
npm install -g @earendil-works/pi-coding-agent   # Node >= 22.19; or use the standalone Pi binary
export PI_API_KEY=...                            # the variable named by [pi].api_key_env
./target/release/alfred
```

On first run Alfred creates `~/.alfred/config/config.toml` (and the prompt
files) from the bundled examples. Edit the config, then restart.

```bash
alfred --version          # alfred 0.1.0
alfred --help             # the subcommands
```

## Configuration

All state lives under `~/.alfred` (`%USERPROFILE%\.alfred` on Windows):

| Path | Purpose |
|------|---------|
| `~/.alfred/config/config.toml` | Main configuration file |
| `~/.alfred/config/prompts/system.md` | Base persona, replaces Pi's coding prompt |
| `~/.alfred/config/prompts/user.md` | Per-user context (`## User Context`) |
| `~/.alfred/config/memories.md` | Durable facts (`## User Memories`) |
| `~/.alfred/config/skills/` | Generated Pi skills (todo, webhook, remember) |
| `~/.alfred/data/alfred.db` | SQLite: todos, jobs, job runs |
| `~/.alfred/logs/alfred.log` | Server log |
| `~/.alfred/pi/` | Per-channel Pi session storage |
| `~/.alfred/pi-agent/` | Alfred's private Pi home (`PI_CODING_AGENT_DIR`) |

The real configuration surface, matching `config/config.toml.example`:

```toml
[server]
port = 8080
host = "127.0.0.1"
# api_key = "${ALFRED_API_KEY}"   # when set, /api/* requires a bearer token

[prompt]
system_prompt_file = "prompts/system.md"
user_prompt_file = "prompts/user.md"

[pi]
# The Pi binary Alfred runs as a subprocess.
binary = "pi"
# Environment variable holding the provider key; passed by environment, never by --api-key.
api_key_env = "PI_API_KEY"
provider = "opencode-go"
model = "space-bunny-free"
thinking = "off"
jobs_tools = []
channel_tools = []
timeout_secs = 900
idle_compact_secs = 43200
compact_token_threshold = 60000
session_dir = "~/.alfred/pi/sessions"
extra_args = []

[jobs]
enabled = true
max_concurrent = 2
min_watch_interval_secs = 900
max_runs_per_job = 100
missing_verdict = "notify"

[telegram]
# bot_token = "${TELEGRAM_BOT_TOKEN}"
# allowed_users = []

[webhook]
# Hosts the agent may POST to with `alfred webhook send`.
# Empty denies every host (the default; the agent runs unattended).
allowed_hosts = []
```

`${VAR}` references are expanded from the environment when the config is
loaded. The config is hot-reloaded: a change is re-read and validated while the
server runs, and a malformed change is rejected with the previous config kept.

### Isolating a run

The database path resolves in this order, highest first:

1. `ALFRED_DATA_DIR` — when set, the database becomes `$ALFRED_DATA_DIR/alfred.db`.
2. `[server] db_path` — an explicit path in the config file.
3. `~/.alfred/data/alfred.db` — the default (`%USERPROFILE%\.alfred\data\alfred.db` on Windows).

A throwaway run therefore needs one variable rather than a `USERPROFILE` override:

```bash
ALFRED_DATA_DIR=/tmp/alfred-test alfred job list
```

```powershell
$env:ALFRED_DATA_DIR = "$env:TEMP\alfred-test"; alfred job list
```

The override also redirects the workspace and per-job scratch directories, so an isolated run never touches the real database. `config/` and `logs/` remain home-relative.

## Running the server

`alfred` with no arguments starts the server: the REST API, the scheduler (when
`[jobs].enabled`), the Telegram connector (when a bot token is set), and the
config watcher. Startup probes `pi --version` once (reported by `/api/info`) and
fails fast when `[pi].binary` cannot be launched.

```
Alfred server started
  PID: 12345
  Listening on: 127.0.0.1:8080
  Config: /home/you/.alfred/config/config.toml
  Data: /home/you/.alfred/data
  Logs: /home/you/.alfred/logs
```

Pass an explicit config with the global flag:

```bash
alfred --config /path/to/config.toml
```

`Ctrl+C` shuts the server down and aborts every long-lived Pi channel child.

## Jobs

A job is a **name + prompt + schedule + delivery policy**. `alfred job` manages
them in Alfred's own SQLite database — no OS cron or Task Scheduler.

A job's kind is determined by exactly one schedule flag:

- `--cron "<5-field cron>"` — **recurring**, e.g. `0 9 * * *`.
- `--at <RFC 3339 | +5s | +10m | +2h | +1d>` — **once**.
- `--watch "<5-field cron>"` — **watch** (polling); the interval must be at
  least `[jobs].min_watch_interval_secs` (default 900s).

`--report` is `always` (deliver every run) or `on_signal` (default; deliver only
a run whose last line is a verdict). `--deliver-to` is a Telegram chat id, with
the first `[telegram].allowed_users` entry as the fallback.

For `on_signal` jobs Alfred appends a verdict instruction to the prompt; the last
line matching `VERDICT: MATCH` or `VERDICT: NO_MATCH` decides delivery. A missing
verdict is resolved by `[jobs].missing_verdict`: `notify` (default) treats it as
`MATCH` so a possible alert is never silently dropped; `skip` treats it as
`NO_MATCH`.

```bash
# Recurring: every day at 09:00
alfred job add --name daily --prompt "Summarise overnight events" --cron "0 9 * * *"

# Once, one hour from now, delivered no matter what
alfred job add --name ping --prompt "Say hello" --at "+1h" --report always

# Watch a mailbox every 30 minutes
alfred job add --name inbox --prompt "Check for urgent mail" --watch "*/30 * * * *"

alfred job list
alfred job show daily
alfred job runs daily          # run history, newest first
alfred job disable daily
alfred job enable daily
alfred job remove daily
```

The scheduler ticks every 30 seconds, runs due jobs through Pi (bounded by
`[jobs].max_concurrent`), records each run, and prunes history to
`[jobs].max_runs_per_job`. Missed recurring periods are not backfilled; a `once`
job more than an hour overdue is recorded `missed`. A dispatcher that panics is
recorded `failed` and does not stop the loop.

> Manual runs are not wired yet: `alfred job run <id>` and
> `POST /api/jobs/{id}/run` report that the manual-run path is unavailable. The
> scheduler runs due jobs automatically.

## Todos

```bash
alfred todo add --title "Pay the electricity bill" --priority high --due "2026-10-05"
alfred todo add --title "Water the plants" --priority low
alfred todo list
alfred todo complete <id>
alfred todo remove <id>
```

`--priority` is `low`, `medium`, or `high` (default `medium`). `list` prints open
todos highest-priority first; completed todos are hidden.

## Memories

Memories are a flat, hand-editable file: `alfred remember` appends one line to
`~/.alfred/config/memories.md`, and that file is injected into every run under
`## User Memories`. There is no SQLite index and no retrieval layer over the
memories file.

```bash
alfred remember "The garage code is 1234"
```

## Webhooks

`alfred webhook send` is the agent's only outbound HTTP path. Alfred enforces
`[webhook].allowed_hosts` before any socket opens; an empty list (the default)
denies every host. Redirects are not followed.

```bash
# Denied by default (empty allow-list):
alfred webhook send https://example.com/hook --json '{"event":"ping"}'
# ERROR: host not allowed: example.com

# With `allowed_hosts = ["example.com"]`, a successful POST prints the status,
# the URL, and the response body:
# 200 https://example.com/hook
# {"ok":true}

# Against a local server with `allowed_hosts = ["127.0.0.1"]`:
alfred webhook send http://127.0.0.1:8099/hook --json '{"event":"ping"}'
```

## Telegram

Set `[telegram].bot_token` and Alfred long-polls the Bot API. Inbound text is
forwarded to the channel's persistent Pi session and the reply is sent back.
`[telegram].allowed_users`, when non-empty, restricts who may talk to the agent.

Built-in chat commands:

- `/todos` — list open todos.
- `/remember <text>` — append a memory.
- `/clear` — start a fresh Pi session.

## REST API

When `[server].api_key` is set, `/api/*` (not `/health` or `/api/info`) requires
`Authorization: Bearer <key>`.

| Method | Endpoint | Description |
|--------|----------|-------------|
| GET | `/health` | Health check (`{"status":"ok"}`) |
| GET | `/api/info` | PID, port, uptime, connections, Pi version, enabled jobs |
| GET | `/api/todos` | List open todos |
| POST | `/api/todos` | Create a todo |
| PUT | `/api/todos/{id}` | Update a todo |
| DELETE | `/api/todos/{id}` | Delete a todo |
| POST | `/api/todos/{id}/complete` | Mark a todo complete |
| GET | `/api/jobs` | List jobs |
| POST | `/api/jobs` | Create a job |
| GET | `/api/jobs/{id}` | Get one job |
| PUT | `/api/jobs/{id}` | Replace a job |
| DELETE | `/api/jobs/{id}` | Delete a job |
| POST | `/api/jobs/{id}/run` | Manual run (not wired; returns 501) |
| GET | `/api/jobs/{id}/runs` | List a job's runs |
| GET | `/api/memories` | List memories as `{slug, text}` |
| POST | `/api/memories` | Append a memory (`{"text": "..."}`) |
| DELETE | `/api/memories/{slug}` | Delete a memory by slug |

```bash
curl -s localhost:8080/health
curl -s localhost:8080/api/info
curl -s -X POST localhost:8080/api/todos \
  -H "Content-Type: application/json" -d '{"title":"REST todo","priority":"high"}'
```

A job body uses `name`, `kind` (`once`|`recurring`|`watch`), `schedule`,
`run_at` (Unix seconds), `prompt`, `report`, `deliver_to`, `model`, `tools`, and
`timeout_secs`.

## The Pi runtime dependency

Alfred invokes Pi as `pi --mode rpc` and speaks JSONL on stdin/stdout. The
integration contract is `pi/packages/coding-agent/docs/rpc.md`.

- Jobs are ephemeral: `--no-session`.
- Channels keep a per-channel session (`--session-dir`, `--name telegram`).
- `--system-prompt` **replaces** Pi's coding persona; `--no-context-files` and
  `--no-approve` are always set.
- `[pi].jobs_tools` and `[pi].channel_tools` map to `--tools`.
- Alfred's generated skills are passed with `--skill`.
- Environment: `PI_CODING_AGENT_DIR`, `PI_CODING_AGENT_SESSION_DIR`,
  `PI_SKIP_VERSION_CHECK=1`, `PI_TELEMETRY=0`, `PI_OFFLINE=1`, and the provider
  key read from `[pi].api_key_env`.

The provider key is passed by environment, never by `--api-key` (which would
expose it in `ps`). `/api/info` reports the probed `pi_version`, or `null` when
Pi is absent.

## Prompt and skills

The system prompt is assembled from three layers, in order: the system prompt,
`## User Context` (the user prompt), and `## User Memories` (the memories file);
empty layers are omitted.

Adding a capability means adding a Pi **skill** (a CLI command plus a `SKILL.md`),
not Rust. Alfred regenerates `todo`, `webhook`, and `remember` under
`~/.alfred/config/skills/<name>/SKILL.md` at every startup, so an upgrade never
leaves a stale skill on disk.

## Development

```bash
cargo build        # compile
cargo test         # the gate; must be green
cargo run -- job list
```

The suite is entirely Rust and runs in-process against temporary databases and
an isolated home; the Pi boundary is exercised with the compiled double at
`src/bin/fake-pi.rs`, so no API key or network is needed. See
[tests/README.md](tests/README.md).

## Documentation

- [ROADMAP.md](ROADMAP.md) — deferred work, with reasons
- [AGENTS.md](AGENTS.md) — working rules for this repository
- [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) — dependency attribution

## License

MIT
