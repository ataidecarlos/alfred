#!/bin/bash
# Wrapper script for autonomous agent cron job using Kira

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

# Ensure log directory exists
mkdir -p "$(dirname "$LOG_FILE")"

# Log start
echo "=== Agent run started at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"

# Change to project directory
cd "$WORKDIR"

# Run the autonomous agent with Kira
sg docker -c "bash -c '$OPENCODE run --file \"$PROMPT_FILE\" \"Read the attached file and follow its instructions\"' >> \"$LOG_FILE\" 2>&1"

# Log finish
echo "=== Agent run finished at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" >> "$LOG_FILE"
