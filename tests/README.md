# Alfred Test Suite

`cargo test` is the gate for this repository: it must be green before any
change is accepted. The suite is entirely Rust — unit tests live next to the
code under `src/`, and the integration tests live here in `tests/`.

Everything runs in-process. Tests use temporary databases and isolated home
directories, and the Pi boundary is exercised against a compiled double, so no
API key, network access, or live `pi` install is required.

## Running the suite

```bash
cargo test                 # everything (unit + integration); the gate
cargo test --test rest_api # one integration binary
cargo test rest_api        # filter by test name, any binary
```

## Integration tests

| File | Verification for | Description |
|------|------------------|-------------|
| `tests/pi_rpc.rs` | Pi RPC boundary (issue #5) | JSONL framing, Pi invocation args/env, process lifecycle, and a full prompt round trip against the compiled double |
| `tests/jobs.rs` | Job domain and store (issue #7) | Job model and store CRUD against a temporary database |
| `tests/job_runner.rs` | Pi-backed job runner (issue #11) | Due-job execution through the double, verdict handling, malformed and failing runs, and run-row lifecycle |
| `tests/rest_api.rs` | REST surface (issue #10) | Jobs and memory endpoints driven in-process on an ephemeral port, including the 401/400/404 paths |
| `tests/telegram.rs` | Telegram channel (issue #12) | The channel on a Pi session with an in-memory outbound recorder |
| `tests/delivery.rs` | Job result delivery (issue #13) | Delivery of job results per the report policy against an in-memory recorder |
| `tests/startup.rs` | Startup wiring (issue #16) | The composed dispatch+delivery path end to end through the double, fail-fast on a non-launchable Pi binary, and reaping of a live channel child on shutdown |
| `tests/release_matrix.rs` | Installer/release agreement (issue #53) | The platforms the installers advertise equal the platforms the release workflow matrix builds |

## The Pi double

Tests that need Pi use the compiled double at `src/bin/fake-pi.rs`, reached
through `[pi] binary` and located via `CARGO_BIN_EXE_fake-pi`. It speaks the RPC
protocol on stdin/stdout with no network, key, or model, and has a deliberately
fixed response (`reply to: <message>` with fixed token/cost figures).

Because the double is a real `[[bin]]` target, the round trip spawns a genuine
executable and runs on Windows, Linux and macOS alike. On Windows this matters
twice over: an npm `.ps1` shim (including an npm-installed `pi`) cannot be
launched by `CreateProcess`.

## End-to-end smoke test

For a full end-to-end smoke test against a pristine container, build and run
the Docker test image:

```bash
./docker/test.sh
```

## Adding tests

Add Rust integration tests in a new `tests/<name>.rs`, following the pattern of
the existing files: an isolated `mod <name>` block so `cargo test <name>`
selects it, a temporary database or in-memory recorder, and the Pi double when
the Pi boundary is involved. No test may require a live API key or the network.
