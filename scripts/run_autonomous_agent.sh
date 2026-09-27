#!/bin/bash
# Wrapper script for the autonomous developer cron job.
# Loads secrets from the AKC vault, then runs OpenCode with the autonomous
# developer prompt. Logs to ~/.alfred/agent.log.

set -uo pipefail

# Environment
export HOME="${HOME:-/home/azureuser}"
export PATH="${HOME}/.opencode/bin:${HOME}/.local/bin:${HOME}/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export XDG_DATA_HOME="${HOME}/.local/share"
export XDG_CONFIG_HOME="${HOME}/.config"

# Paths
WORKDIR="$(cd "$(dirname "$0")/.." && pwd)"
LOG_FILE="${HOME}/.alfred/agent.log"
PROMPT_FILE="${WORKDIR}/prompts/autonomous_developer.md"
OPENCODE="$(command -v opencode || echo "${HOME}/.opencode/bin/opencode")"

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

# Log start
echo "=== Agent run started at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"

# Load secrets from the AKC vault. The vault password may already be in the
# environment, otherwise read the user-readable password file.
if [ -z "${AKC_PASSWORD:-}" ] && [ -f "${HOME}/.alfred/akc.pass" ]; then
    AKC_PASSWORD="$(cat "${HOME}/.alfred/akc.pass")"
fi
if command -v akc >/dev/null 2>&1 && [ -f "${HOME}/ataide-keychain" ]; then
    if [ -n "${AKC_PASSWORD:-}" ]; then
        [ -n "${OPENCODE_GO_API_KEY:-}" ] || OPENCODE_GO_API_KEY="$(akc get "${HOME}/ataide-keychain" OPENCODE_AZURE-DEV --password "$AKC_PASSWORD")"
        [ -n "${KIRA_API_KEY:-}" ] || KIRA_API_KEY="$(akc get "${HOME}/ataide-keychain" KIRA_API_KEY --password "$AKC_PASSWORD")"
        export OPENCODE_GO_API_KEY KIRA_API_KEY
    else
        echo "WARNING: AKC_PASSWORD not available; secrets not loaded" >> "$LOG_FILE"
    fi
fi

# Change to project directory
cd "$WORKDIR"

# Build the agent invocation. --standalone runs a private server so it inherits
# the secrets exported above instead of relying on a shared, long-lived one.
AGENT_CMD=("$OPENCODE" run --standalone --file "$PROMPT_FILE" "Read the attached file and follow its instructions")

# Run the autonomous agent (sg docker only where the docker group exists)
if getent group docker >/dev/null 2>&1; then
    sg docker -c "$(printf '%q ' "${AGENT_CMD[@]}")" >> "$LOG_FILE" 2>&1
else
    "${AGENT_CMD[@]}" >> "$LOG_FILE" 2>&1
fi

# Log finish
echo "=== Agent run finished at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"
