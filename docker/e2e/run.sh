#!/usr/bin/env bash
# Start (or restart) the long-lived Alfred e2e container.
#
#   bash docker/e2e/run.sh
#
# The OpenCode Go API key is read from the AKC vault at start time and passed
# to the container as OPENCODE_GO_API_KEY. No secret value is written to the
# repository or the image.
#
# Environment overrides:
#   E2E_PORT     host port mapped to the container's 3000 (default 3009;
#                port 3001 is occupied by the local Kira server)
#   E2E_NAME     container name (default alfred-e2e)
#   E2E_VOLUME   named data volume (default alfred-e2e-data)
#   E2E_IMAGE    image tag (default alfred-e2e)
#   AKC_KEYCHAIN keychain path (default ~/ataide-keychain)
#   AKC_KEY      secret name (default OPENCODE_AZURE-DEV)
#   AKC_PASSWORD vault password for non-interactive runs
#   OPENCODE_GO_API_KEY  if already set, AKC is skipped
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$REPO_ROOT"

KEYCHAIN="${AKC_KEYCHAIN:-$HOME/ataide-keychain}"
AKC_KEY="${AKC_KEY:-OPENCODE_AZURE-DEV}"
IMAGE="${E2E_IMAGE:-alfred-e2e}"
NAME="${E2E_NAME:-alfred-e2e}"
HOST_PORT="${E2E_PORT:-3009}"
VOLUME="${E2E_VOLUME:-alfred-e2e-data}"

# ── Resolve the API key ───────────────────────────────────────────────
if [ -z "${OPENCODE_GO_API_KEY:-}" ]; then
    if ! command -v akc >/dev/null 2>&1; then
        echo "error: OPENCODE_GO_API_KEY is not set and 'akc' was not found" >&2
        exit 1
    fi
    if [ -n "${AKC_PASSWORD:-}" ]; then
        OPENCODE_GO_API_KEY="$(akc get "$KEYCHAIN" "$AKC_KEY" --password "$AKC_PASSWORD")"
    else
        # Prompts on a terminal; fails fast when there is no TTY.
        OPENCODE_GO_API_KEY="$(akc get "$KEYCHAIN" "$AKC_KEY")"
    fi
fi
if [ -z "${OPENCODE_GO_API_KEY}" ]; then
    echo "error: retrieved an empty API key from AKC ($AKC_KEY)" >&2
    exit 1
fi
export OPENCODE_GO_API_KEY

# ── Locate a usable docker CLI ────────────────────────────────────────
DOCKER=(docker)
if ! docker info >/dev/null 2>&1; then
    if sudo -n docker info >/dev/null 2>&1; then
        DOCKER=(sudo -n docker)
    else
        echo "error: cannot access the docker daemon (add the user to the 'docker' group)" >&2
        exit 1
    fi
fi

# ── Build the image from the current source ───────────────────────────
"${DOCKER[@]}" build -f docker/e2e/Dockerfile -t "$IMAGE" .

# ── (Re)create the detached container ─────────────────────────────────
"${DOCKER[@]}" rm -f "$NAME" >/dev/null 2>&1 || true
"${DOCKER[@]}" volume create "$VOLUME" >/dev/null

# Pass secrets through a private env-file rather than the CLI/env, so the key
# never lands in process arguments (and survives the `sudo` fallback above).
ENV_FILE="$(mktemp "${TMPDIR:-/tmp}/alfred-e2e-env.XXXXXX")"
chmod 600 "$ENV_FILE"
trap 'rm -f "$ENV_FILE"' EXIT
{
    printf 'OPENCODE_GO_API_KEY=%s\n' "$OPENCODE_GO_API_KEY"
    printf 'RUST_LOG=%s\n' "${RUST_LOG:-info}"
} > "$ENV_FILE"

"${DOCKER[@]}" run -d \
    --name "$NAME" \
    --restart unless-stopped \
    -p "${HOST_PORT}:3000" \
    -v "${VOLUME}:/home/alfred/.alfred/data" \
    --env-file "$ENV_FILE" \
    "$IMAGE" >/dev/null

rm -f "$ENV_FILE"
trap - EXIT

# ── Wait for the container health check ───────────────────────────────
healthy=0
for _ in $(seq 1 60); do
    status="$("${DOCKER[@]}" inspect -f '{{.State.Health.Status}}' "$NAME" 2>/dev/null || echo starting)"
    if [ "$status" = "healthy" ]; then
        healthy=1
        break
    fi
    if [ "$status" = "unhealthy" ]; then
        break
    fi
    sleep 1
done

if [ "$healthy" -ne 1 ]; then
    echo "error: ${NAME} did not become healthy" >&2
    "${DOCKER[@]}" logs --tail 50 "$NAME" >&2 || true
    exit 1
fi

# Confirm the mapped endpoints answer on the host too.
curl -fsS "http://localhost:${HOST_PORT}/health" >/dev/null
curl -fsS "http://localhost:${HOST_PORT}/api/info" >/dev/null

echo "alfred-e2e healthy on host port ${HOST_PORT}"
