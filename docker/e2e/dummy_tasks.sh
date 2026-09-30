#!/usr/bin/env bash
# Feature-exercise suite for the Alfred e2e container.
#
# Runs once at container start (see entrypoint.sh) and is safe to run by hand.
# Every check appends a single JSON line
#   {"check":"<name>","ok":true|false,"ts":"<iso8601>"}
# to $HOME/.alfred/data/e2e_report.jsonl.
#
# The jobs are created and executed through Alfred's own scheduler
# (`alfred job ...`): `alfred job run` uses the same runner and delivery path
# the scheduler installs. There is no OS cron and no server-side `/api/messages`
# endpoint; the job run is the live LLM round-trip.
set -u

# Pin HOME to the container user's home, where the config, database, and report
# live, and prepend the install directory so `alfred` and `pi` resolve.
export HOME="${ALFRED_E2E_HOME:-/home/alfred}"
export PATH="/usr/local/bin:/usr/bin:/bin:${PATH:-}"
BASE_URL="${ALFRED_E2E_URL:-http://localhost:3000}"
DATA_DIR="$HOME/.alfred/data"
REPORT="$DATA_DIR/e2e_report.jsonl"
JOB_NAME="e2e-dummy"
MEMORY_TEXT="e2e dummy memory"
# The slug memory::slug_for("e2e dummy memory") produces (first 40 characters,
# non-alphanumerics collapsed to "-"). See src/memory/mod.rs.
MEMORY_SLUG="e2e-dummy-memory"

mkdir -p "$DATA_DIR"

record() {
    local check="$1" ok="$2" ts
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '{"check":"%s","ok":%s,"ts":"%s"}\n' "$check" "$ok" "$ts" >> "$REPORT"
}

json_id() {
    printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}

# 1. Create a todo: POST /api/todos -> {"id":"<uuid>"}.
todo_resp="$(curl -sS --max-time 10 -X POST "$BASE_URL/api/todos" \
    -H 'Content-Type: application/json' \
    -d '{"title":"e2e dummy todo","priority":"low"}' 2>/dev/null || true)"
todo_id="$(json_id "$todo_resp")"
record "todo_create" "$([ -n "$todo_id" ] && echo true || echo false)"

# 2. Complete that todo: POST /api/todos/{id}/complete -> 204.
if [ -n "$todo_id" ]; then
    code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
        -X POST "$BASE_URL/api/todos/$todo_id/complete" 2>/dev/null || echo 000)"
    record "todo_complete" "$([ "$code" = "204" ] && echo true || echo false)"
else
    record "todo_complete" false
fi

# 3. Append a memory: POST /api/memories {"text":...} -> 201 (no body).
code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -X POST "$BASE_URL/api/memories" \
    -H 'Content-Type: application/json' \
    -d "{\"text\":\"${MEMORY_TEXT}\"}" 2>/dev/null || echo 000)"
record "memory_add" "$([ "$code" = "201" ] && echo true || echo false)"

# 4. List memories: GET /api/memories -> [{"slug":...,"text":...}]; ours must
#    be present.
mem_list="$(curl -sS --max-time 10 "$BASE_URL/api/memories" 2>/dev/null || true)"
if printf '%s' "$mem_list" | grep -qF "$MEMORY_TEXT"; then
    record "memory_list" true
else
    record "memory_list" false
fi

# 5. Delete that memory by slug: DELETE /api/memories/{slug} -> 204, and it must
#    leave the listing.
code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
    -X DELETE "$BASE_URL/api/memories/$MEMORY_SLUG" 2>/dev/null || echo 000)"
after_delete="$(curl -sS --max-time 10 "$BASE_URL/api/memories" 2>/dev/null || true)"
if [ "$code" = "204" ] && ! printf '%s' "$after_delete" | grep -qF "$MEMORY_TEXT"; then
    record "memory_delete" true
else
    record "memory_delete" false
fi

# 6. Create a job through Alfred's scheduler CLI: `alfred job add`. A prior run
#    may have left the name behind (the data volume persists across restarts),
#    so remove it first; a missing job is not an error here. `--at "+1h"` keeps
#    the server's scheduler from running it on its own tick, so the manual run
#    below is the one exercised.
alfred job remove "$JOB_NAME" >/dev/null 2>&1 || true
if alfred job add --name "$JOB_NAME" --at "+1h" --report always \
    --prompt "Reply with exactly the word PONG and nothing else." >/dev/null 2>&1; then
    record "job_add" true
else
    record "job_add" false
fi

# 7. Run the job now: `alfred job run` executes it synchronously through the same
#    Pi runner and delivery path the scheduler uses. Success needs both a zero
#    exit and PONG in the recorded output, proving a live LLM round-trip through
#    Pi (this replaces the removed /api/messages check).
if job_out="$(alfred job run "$JOB_NAME" 2>&1)"; then
    if printf '%s' "$job_out" | grep -qF 'PONG'; then
        record "job_run" true
    else
        record "job_run" false
    fi
else
    record "job_run" false
fi

# 8. Run history: `alfred job runs` prints a table; the manual run must appear
#    with status "success".
runs_out="$(alfred job runs "$JOB_NAME" 2>&1 || true)"
if printf '%s' "$runs_out" | grep -q 'success'; then
    record "job_runs" true
else
    record "job_runs" false
fi

exit 0
