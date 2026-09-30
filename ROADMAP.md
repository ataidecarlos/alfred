# Alfred Roadmap

This is the single roadmap for Alfred, a 24x7 personal-assistant host whose
agent engine is Pi. Work that is not shipped is deferred here, each item with
the reason it is deferred.

## Deferred

### Laya decision model

Deferred: evaluation shows the base model is a fine-tuning target, not a
drop-in gate.

- Base checkpoints score near chance zero-shot — **0.362** accuracy against a
  **0.461** majority-class baseline — so an unfinetuned Laya cannot filter or
  gate requests.
- `action.act_probability` carries no signal (issue #185); a gate must use
  `confidence`.
- Temperature must be refit on our own data: mean ECE improves from **0.466** to
  **0.081** after refitting.
- Options share a single **192-token** budget, so option sets must stay small.
- **512-token** context window.

Candidate homes if it is adopted: mid-run filtering of high-volume data through
a Pi skill, and the appliance-signal pipeline.
Licence: Apache-2.0.

### Per-job run-logging policy

Deferred. v1 logs every run of every job. Not every job needs every run
persisted; a per-job policy (log every run / log only on signal / do not log)
would cut log and database noise.

### Job-run retention policy

Deferred. Run history is bounded only by `[jobs].max_runs_per_job` (default
100). There is no age-based or store-wide retention policy.

### Re-evaluate `pi-chat` after release

Deferred until the new product is released. `pi-chat`
(`earendil-works/pi-chat`, MIT; vendors portions of the Vercel Chat SDK) is not a
dependency of the current build.

### Revisit Kira

Deferred. Kira (a shared ticketing system for AI agents) was removed on purpose:
it is developer-ticketing machinery, not an assistant capability, and
orchestration must not enter the product. Revisit only if a non-coding use for a
shared ticket store emerges.

### Branch protection on `main`

Deferred. Enabling branch protection requires a token carrying
`Administration: Read and write`, which the current token lacks (HTTP 403).
Meanwhile it is enforced by the `AGENTS.md` rule "never push to `main`" and by
the dispatcher being the only merge point.

### Installer end-to-end verification

Deferred, partially verified. The installer was executed in WSL and passes; the
no-op second run is proven. The remaining gap is a proven successful *first*
install, which needs a release tagged from the rewrite: the published releases are
still the previous product (see README).

### Manual job runs

Deferred (issue #59). `alfred job run <id>` and `POST /api/jobs/{id}/run` still
report that the manual-run path is unavailable, even though the Pi-backed runner
and the scheduler shipped. A manual-run entry point should dispatch the same
runner.

## Removed (non-goals)

Removed in the rewrite to the Pi-host; do not reintroduce these (see
`AGENTS.md`):

- The in-process LLM provider layer, agent loop, and tool-calling registry
  (`src/llm/`, `src/agent/runner.rs`, `src/session/`, `src/tools/`) — Pi owns
  them.
- The terminal UI (`src/tui/`) — clients are Telegram and REST.
- OS-level cron / Windows Task Scheduler management (`src/scheduler/control.rs`)
  — Alfred schedules its own jobs in SQLite.
- Kira and the autonomous ticket-driven development loop.
- The SQLite memory index and "Laya-aware retrieval" — memory is a flat,
  hand-editable file.

## Shipped

- The Pi-host rewrite: the job model (`once` / `recurring` / `watch`), the
  SQLite scheduler, the Pi-backed runner with verdict handling, delivery to
  Telegram, the REST surface, file-backed memories, generated Pi skills, and
  config hot-reload.
- `linux-arm64` release artifacts (#53), and a `cargo test` guard asserting the
  installer's advertised platforms and the release matrix stay in agreement.
