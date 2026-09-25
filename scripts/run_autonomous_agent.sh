#!/bin/bash
# Wrapper script for autonomous agent cron job
# Handles environment setup and ensures proper logging

# Environment
export HOME=/home/azureuser
export PATH="/home/azureuser/.opencode/bin:/home/azureuser/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export XDG_DATA_HOME="${HOME}/.local/share"
export XDG_CONFIG_HOME="${HOME}/.config"

# Paths
WORKDIR="/home/azureuser/projects/alfred"
LOG_FILE="${HOME}/.alfred/agent.log"
PROMPT_FILE="${WORKDIR}/prompts/autonomous_developer.md"
OPENCODE="/home/azureuser/.opencode/bin/opencode"
ALFRED="${WORKDIR}/target/release/alfred"

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

# Log start
echo "=== Agent run started at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"

# Change to project directory
cd "$WORKDIR"

# Check if alfred binary exists
if [ ! -f "$ALFRED" ]; then
    echo "ERROR: Alfred binary not found at $ALFRED" >> "$LOG_FILE"
    exit 1
fi

# Check if there are pending work items
NEXT=$("$ALFRED" workitem next 2>&1)
if echo "$NEXT" | grep -q "No pending work items"; then
    echo "No pending work items. Skipping." >> "$LOG_FILE"
    echo "=== Agent run finished at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"
    exit 0
fi

echo "$NEXT" >> "$LOG_FILE"

# Run the autonomous agent (sg docker ensures docker group access for T4-T6 tickets)
# Attach prompt file to avoid shell expansion issues with $ITEM_ID etc.
sg docker -c "bash -c '$OPENCODE run --file \"$PROMPT_FILE\" \"Read the attached file and follow its instructions\"' >> \"$LOG_FILE\" 2>&1"

# Log finish
echo "=== Agent run finished at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"
