#!/usr/bin/env bash
# Entrypoint for the long-lived Alfred e2e container.
#
# Starts the Alfred server in the foreground (as PID 1) and runs the
# feature-exercise suite once at startup, so e2e_report.jsonl exists
# immediately. The jobs are owned by Alfred's own scheduler and executed with
# `alfred job run`; there is no OS cron and the container runs as the
# unprivileged `alfred` user.
set -euo pipefail

export HOME="${HOME:-/home/alfred}"
export PATH="/usr/local/bin:/usr/bin:/bin:${PATH:-}"
export RUST_LOG="${RUST_LOG:-info}"

DATA_DIR="$HOME/.alfred/data"
TASKS="/opt/alfred/e2e/dummy_tasks.sh"

mkdir -p "$DATA_DIR"

# Start the server.
alfred &
alfred_pid=$!
trap 'kill -TERM "$alfred_pid" 2>/dev/null || true' TERM INT

# Wait for the server to become healthy, then exercise the features once.
for _ in $(seq 1 150); do
    if curl -fsS http://localhost:3000/health >/dev/null 2>&1; then
        break
    fi
    if ! kill -0 "$alfred_pid" 2>/dev/null; then
        echo "error: alfred exited before becoming healthy" >&2
        exit 1
    fi
    sleep 0.2
done
"$TASKS" || true

wait "$alfred_pid"
