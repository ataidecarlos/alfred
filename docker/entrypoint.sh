#!/usr/bin/env bash
# Entrypoint for the Alfred test image.
#
# Usage:
#   docker run --rm alfred-test                  # start the server (foreground)
#   docker run --rm alfred-test alfred job list  # run an Alfred CLI command
#   docker run --rm alfred-test curl -s localhost:3000/health
#                                                # boot a throwaway server, wait
#                                                # for health, then run <cmd>
set -euo pipefail

CONFIG_DIR="${HOME}/.alfred/config"
PROMPTS_DIR="${CONFIG_DIR}/prompts"

# Seed a clean config and prompts on first boot. The config references API keys
# as ${ENV_VAR}, which Alfred expands when it loads the file.
mkdir -p "${CONFIG_DIR}" "${PROMPTS_DIR}"
if [ ! -f "${CONFIG_DIR}/config.toml" ]; then
    cp /opt/alfred/docker/config.toml "${CONFIG_DIR}/config.toml"
fi
for name in system user; do
    if [ ! -f "${PROMPTS_DIR}/${name}.md" ]; then
        cp "/opt/alfred/prompts/${name}.md.example" "${PROMPTS_DIR}/${name}.md"
    fi
done

# Alfred runs Pi as a subprocess; the provider key is read from the environment
# variable named by [pi].api_key_env (PI_API_KEY in docker/config.toml) and is
# only needed when an LLM call runs. A pristine smoke test just boots the server
# and probes /health and /api/info, so fall back to a placeholder when the key is
# unset. Set a real PI_API_KEY to run jobs.
if [ -z "${PI_API_KEY:-}" ]; then
    echo "warning: PI_API_KEY is not set; using a placeholder so the server can boot (health checks only)" >&2
    export PI_API_KEY="placeholder-set-real-key-for-llm-requests"
fi

# No arguments: run the server in the foreground.
if [ "$#" -eq 0 ]; then
    exec alfred
fi

# Explicit Alfred invocation: run it directly.
if [ "$1" = "alfred" ]; then
    exec "$@"
fi

# Otherwise: boot a throwaway server, wait for it to become healthy, then run
# the requested command against it.
alfred &
server_pid=$!
trap 'kill "${server_pid}" 2>/dev/null || true' EXIT

port=3000
healthy=0
for _ in $(seq 1 100); do
    if curl -fsS "http://localhost:${port}/health" >/dev/null 2>&1; then
        healthy=1
        break
    fi
    if ! kill -0 "${server_pid}" 2>/dev/null; then
        echo "error: alfred server exited before becoming healthy" >&2
        exit 1
    fi
    sleep 0.1
done
if [ "${healthy}" -ne 1 ]; then
    echo "error: alfred server did not become healthy on port ${port}" >&2
    exit 1
fi

exec "$@"
