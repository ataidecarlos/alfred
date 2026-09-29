# AGENTS.md — Alfred

Working rules for anyone (human or agent) changing this repository.

## What Alfred is

A 24x7 personal-assistant host. Alfred owns the things that make it a *product*:
the job model (schedule + prompt + delivery), the scheduler, run history,
delivery to the user, prompt/memory files, the REST surface, and the Telegram
channel.

Alfred does **not** own the agent. The agent loop, LLM providers, tool calling,
sessions, compaction, and streaming are delegated to **Pi**
(https://github.com/earendil-works/pi, MIT) run as a subprocess in RPC mode.
See `pi/packages/coding-agent/docs/rpc.md` for the protocol — it is the
integration contract.

**Hard rule:** adding a capability means adding a Pi *skill* (a CLI command plus
a `SKILL.md`), not adding Rust. Rust is reserved for the host concerns listed
above.

**Hard rule:** orchestration and ticketing machinery must never enter the
product. Issue-driven development happens outside this repository.

## Non-goals

Do not reintroduce, and do not accept contributions that add:

- A coding/dev-ticketing agent. Alfred is explicitly not a coding agent.
- An in-process LLM provider layer, agent loop, or tool-calling registry.
  Pi owns those. `src/llm/`, `src/agent/runner.rs`, `src/session/`, `src/tui/`
  and `src/scheduler/control.rs` were removed deliberately.
- A terminal UI. Interactions happen through clients (Telegram, REST).
- OS-level cron management. Alfred schedules its own jobs in SQLite.

If a change seems to require one of these, stop and raise it as a plan conflict
rather than implementing it.

## Environment

- **Rust** 1.97+, **Node** >= 22.19, **Pi** on PATH (`pi --version`).
- Pi is isolated from your personal Pi install: Alfred sets
  `PI_CODING_AGENT_DIR` to its own directory. Never rely on, or write to, the
  user's `~/.pi`.
- Windows on ARM is supported for the binary; Pi-backed jobs and channels are
  Linux-verified only. `docker` may be unavailable on the dev machine.

## Commands

```bash
cargo build              # compile
cargo test               # the gate; must be green before any issue is closed
cargo run -- job list    # run the server is `cargo run` with no arguments
```

Pi is a **runtime dependency**, not a build dependency. Tests that need it use
the compiled double at `src/bin/fake-pi.rs` (a `[[bin]]` target, located by
tests through `CARGO_BIN_EXE_fake-pi`) via `[pi] binary` — no API key and no
network. Prefer the double over live calls.

## Git discipline

Multiple agents may be working in this working tree at the same time.

Committing:

- Stage explicit paths (`git add src/main.rs src/config.rs`). Never
  `git add -A` or `git add .` — those stage another agent's work.
- Only commit files you changed in your own session.
- **Never push to `main`.** Work on `issue/<number>-<slug>`.
- Message format: `<type>(<area>): <summary>`, then a blank line, then
  `Agent-Issue: #<number>` and the acceptance command you ran.

Never run (these destroy other agents' work):

- `git reset --hard`, `git checkout .`, `git clean -fd`, `git stash`
- `git commit --no-verify`

If you hit a rebase conflict in a file you did not modify, abort and ask.

## Secrets

- Never print, log, commit, or embed a token. Not in a commit, not in an issue
  comment, not in a remote URL.
- Git auth comes from the credential helper. Do not put a token in a URL — that
  writes it to `.git/config` in cleartext.
- Provider keys are read from the environment variable named by
  `[pi].api_key_env` and passed to the subprocess by environment, never by
  `--api-key` (that would expose the key in `ps`).

## Reporting work

A claim is not evidence. When you report an issue as done, include:

1. The exact acceptance command you ran.
2. Its raw output.
3. The commit SHA.

Do not summarise away a failure. If the acceptance command fails, report the
failure and leave the issue open.

## Code quality

- Read a file in full before making a wide-ranging change to it.
- No `unsafe`. No `unwrap()`/`expect()` on paths that can fail at runtime in
  server code; use the existing `AlfredError` variants.
- Every error path either propagates a typed error or is logged with the
  operation and the failing value.
- Tests accompany behaviour; a fix without a test that would have caught it is
  incomplete.
- Keep the diff scoped to the issue. Do not opportunistically refactor
  neighbouring code.

## Documentation

- `README.md` — user-facing. Every command in it must actually run.
- `ROADMAP.md` — the single roadmap; deferred work goes here with the reason.
- `THIRD-PARTY-NOTICES.md` — update when adding a dependency that carries a
  license requiring attribution.
