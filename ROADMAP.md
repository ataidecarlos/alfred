# Alfred Roadmap

## Upcoming Features

### v2026.Q4

#### Built-in Update Command
- Add `alfred update` command to check for and install updates
- Auto-check for updates on server startup
- Option to disable auto-check in config

#### Config Migration
- Automatic migration of config files when upgrading
- Backup old configs before replacing
- Merge new config options with existing settings

#### Enhanced Telegram Features
- Support for inline queries
- File/image handling
- Group chat support with permissions

#### WebUI Dashboard
- Simple web interface for managing todos and memories
- Real-time server status monitoring
- Configuration editor

### v2027.Q1

#### Multi-User Support
- User authentication and authorization
- Per-user conversation history
- Role-based access control

#### Plugin System
- Custom tool development
- Plugin marketplace
- Hot-reload support

#### Advanced Scheduling
- Recurring tasks with complex patterns
- Task dependencies
- Calendar integration

### v2027.Q2

#### Voice Interface
- Speech-to-text integration
- Text-to-speech responses
- Wake word detection

#### Mobile App
- iOS/Android companion app
- Push notifications
- Remote management

#### Enterprise Features
- LDAP/SSO integration
- Audit logging
- Compliance reporting

## Completed

### v2026.09.04
- Initial release
- Multi-provider LLM support (OpenAI, Anthropic, Google, DeepSeek)
- Built-in tools (webhook, shell, todo)
- Telegram connector
- Terminal UI (TUI)
- Scheduled tasks
- Cross-platform support (Linux, macOS, Windows)
- Automated release pipeline

### Architecture Refactor (v2026.09.17)
- Message Bus for channel decoupling
- Session management with legacy migration
- Agent loop/runner split
- Workspace management
- Unified directory structure (`~/.alfred/`)
- Obsidian-friendly vault structure

### Platform & Developer Tooling (v2026.09.25)
- Work item system: SQLite-backed tracker for autonomous development, with
  dependency resolution, automated verification, and the `alfred workitem` CLI
- Scheduler CLI: OS-agnostic management of cron jobs (Linux/macOS) and Windows
  Task Scheduler entries via `alfred scheduler`
- TUI control center: `Ctrl+K` panel for managing scheduler jobs, todos,
  memories, and connected channels
- Config hot-reload: config file edits are applied without restarting the server
- Laya decision layer: high-confidence requests execute directly without an LLM
  call, and the chosen decision path is logged
- Docker test image: pristine-container smoke test via `docker/test.sh`

## Contributing

We welcome contributions! Please see [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

## Feedback

Have ideas for new features? Open an issue on [GitHub](https://github.com/ataidecarlos/alfred/issues).
