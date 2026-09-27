#!/usr/bin/env bash
# Entrypoint for the long-lived Alfred e2e container.
#
# Registers the recurring dummy-task suite through the Alfred scheduler CLI,
# starts the cron daemon, then runs the Alfred server in the foreground (as
# PID 1). The task suite is also run once at startup so the report file exists
# immediately instead of waiting for the first cron tick.
set -euo pipefail

export HOME="${HOME:-/home/alfred}"
export PATH="/usr/local/bin:/usr/bin:/bin:${PATH:-}"
export RUST_LOG="${RUST_LOG:-info}"

DATA_DIR="$HOME/.alfred/data"
TASKS="/opt/alfred/e2e/dummy_tasks.sh"

mkdir -p "$DATA_DIR"

# Register the recurring job through the Alfred scheduler CLI so the cron entry
# is owned/managed by Alfred rather than hand-written.
alfred scheduler add '*/5 * * * *' "$TASKS" >/dev/null

# Start the cron daemon (backgrounds itself).
cron

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
