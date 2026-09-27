#!/usr/bin/env bash
# Recurring feature-exercise suite for the Alfred e2e container.
#
# Registered via `alfred scheduler add '*/5 * * * *'` (see entrypoint.sh) and
# run once at container start. Every check appends a single JSON line
#   {"check":"<name>","ok":true|false,"ts":"<iso8601>"}
# to $HOME/.alfred/data/e2e_report.jsonl. The script is intentionally quiet so
# cron never tries to mail its output.
set -u

# cron sets HOME from /etc/passwd (root -> /root), so pin it to the container
# user's home where the config, database, and report live.
export HOME="${ALFRED_E2E_HOME:-/home/alfred}"
export PATH="/usr/local/bin:/usr/bin:/bin:${PATH:-}"
BASE_URL="${ALFRED_E2E_URL:-http://localhost:3000}"
DATA_DIR="$HOME/.alfred/data"
REPORT="$DATA_DIR/e2e_report.jsonl"
MODEL="space-bunny-free"

mkdir -p "$DATA_DIR"

record() {
    local check="$1" ok="$2" ts
    ts="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    printf '{"check":"%s","ok":%s,"ts":"%s"}\n' "$check" "$ok" "$ts" >> "$REPORT"
}

json_id() {
    printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}

# 1. Create a todo via the REST API.
todo_resp="$(curl -sS --max-time 10 -X POST "$BASE_URL/api/todos" \
    -H 'Content-Type: application/json' \
    -d '{"title":"e2e dummy todo","priority":"low"}' 2>/dev/null || true)"
todo_id="$(json_id "$todo_resp")"
record "todo_create" "$([ -n "$todo_id" ] && echo true || echo false)"

# 2. Complete that todo.
if [ -n "$todo_id" ]; then
    code="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 10 \
        -X POST "$BASE_URL/api/todos/$todo_id/complete" 2>/dev/null || echo 000)"
    record "todo_complete" "$([ "$code" = "204" ] && echo true || echo false)"
else
    record "todo_complete" false
fi

# 3. Add a memory.
mem_resp="$(curl -sS --max-time 10 -X POST "$BASE_URL/api/memories" \
    -H 'Content-Type: application/json' \
    -d '{"content":"e2e dummy memory"}' 2>/dev/null || true)"
mem_id="$(json_id "$mem_resp")"
record "memory_add" "$([ -n "$mem_id" ] && echo true || echo false)"

# 4. List memories and confirm ours is present.
mem_list="$(curl -sS --max-time 10 "$BASE_URL/api/memories" 2>/dev/null || true)"
if printf '%s' "$mem_list" | grep -q 'e2e dummy memory'; then
    record "memory_list" true
else
    record "memory_list" false
fi

# 5. LLM round-trip through the configured model.
llm_resp="$(curl -sS --max-time 60 -X POST "$BASE_URL/api/messages" \
    -H 'Content-Type: application/json' \
    -d '{"user_id":"e2e","text":"Reply with exactly the word PONG and nothing else."}' 2>/dev/null || true)"
if printf '%s' "$llm_resp" | grep -q 'PONG'; then
    record "llm_roundtrip:$MODEL" true
else
    record "llm_roundtrip:$MODEL" false
fi

# 6. Alfred scheduler CLI.
if alfred scheduler list >/dev/null 2>&1; then
    record "scheduler_list" true
else
    record "scheduler_list" false
fi

# 7. Alfred work-item CLI.
if alfred workitem list >/dev/null 2>&1; then
    record "workitem_list" true
else
    record "workitem_list" false
fi

exit 0
