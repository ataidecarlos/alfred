#!/bin/bash
# Migrate ROADMAP.md to WORKITEMS.db
# Creates atomic work items from the existing roadmap

set -e

DB_PATH="${HOME}/.alfred/workitems.db"
ALFRED_BIN="./target/release/alfred"

# Build alfred first if binary doesn't exist
if [ ! -f "$ALFRED_BIN" ]; then
    echo "Building alfred..."
    cargo build --release
fi

echo "Creating initial work items from roadmap..."

# Infrastructure items
$ALFRED_BIN workitem add \
    --title "Create Docker container for pristine Alfred testing" \
    --description "Build a Docker container that compiles Alfred from source, starts with clean config, and can be tested end-to-end. Must support injecting API keys via environment variables." \
    --priority critical \
    --category infrastructure \
    --effort M \
    --verification "docker run --rm alfred-test curl -s localhost:3000/health"

$ALFRED_BIN workitem add \
    --title "Implement OS-agnostic scheduler control wrapper" \
    --description "Create CLI commands: alfred scheduler list, add, remove. Wraps OS cron (Linux: crontab, macOS: launchd, Windows: Task Scheduler) with a unified interface." \
    --priority high \
    --category infrastructure \
    --effort L \
    --verification "alfred scheduler list && alfred scheduler add '*/5 * * * *' 'echo test' && alfred scheduler list | grep test"

$ALFRED_BIN workitem add \
    --title "Add TUI control center for feature management" \
    --description "Extend TUI with a control center showing: scheduler status, active todos, memory count, connected channels. Navigate with arrow keys, select with Enter. Toggle scheduler jobs, view/edit todos." \
    --priority high \
    --category feature \
    --effort L \
    --verification "cargo test --test tui_control_center"

$ALFRED_BIN workitem add \
    --title "Implement hot-reload for configuration changes" \
    --description "Detect config file changes within 5 seconds without server restart. Apply changes to LLM provider, model, API keys, scheduler jobs. Log: Config reloaded at timestamp." \
    --priority medium \
    --category infrastructure \
    --effort M \
    --verification "echo 'test: value' >> ~/.config/alfred/config.toml && sleep 6 && grep 'Config reloaded' ~/.alfred/logs/alfred.log"

$ALFRED_BIN workitem add \
    --title "Configure Alfred to use cheap Opencode Go models" \
    --description "Set default provider to opencode-go. Retrieve API key from AKC: OPENCODE_AZURE-DEV. Update config template. Use cheapest available model." \
    --priority medium \
    --category infrastructure \
    --effort S \
    --verification "alfred workitem next | grep -q 'Configure Alfred'"

$ALFRED_BIN workitem add \
    --title "Integrate Laya model for high-confidence action decisions" \
    --description "Laya model evaluates requests. Confidence > 0.8: execute directly. Confidence <= 0.8: delegate to LLM. Log decision path." \
    --priority medium \
    --category experiment \
    --effort XL \
    --verification "cargo test --test laya_integration"

$ALFRED_BIN workitem add \
    --title "Create cron job for hourly autonomous development" \
    --description "Cron job runs every hour: reads WORKITEMS.db, picks next item, implements, updates status. Log to ~/.alfred/agent.log." \
    --priority high \
    --category infrastructure \
    --effort M \
    --verification "crontab -l | grep alfred"

$ALFRED_BIN workitem add \
    --title "Implement automated verification for work item completion" \
    --description "Each work item has verification_command. On status=completed: run command. Exit 0: mark completed. Exit != 0: mark failed, log output." \
    --priority medium \
    --category infrastructure \
    --effort S \
    --verification "alfred workitem add --title 'Test Verify' --verification 'echo success' --category test && alfred workitem next | head -1 | awk '{print \$1}' | xargs -I{} alfred workitem complete {} --verification 'success'"

$ALFRED_BIN workitem add \
    --title "Create autonomous developer prompt template" \
    --description "Prompt template for hourly cron agent that reads work items, implements, and updates status." \
    --priority high \
    --category infrastructure \
    --effort S \
    --verification "test -f prompts/autonomous_developer.md"

$ALFRED_BIN workitem add \
    --title "Create WORKITEMS.md documentation" \
    --description "Document schema, CLI commands, autonomous workflow, how to add items. Include examples." \
    --priority medium \
    --category documentation \
    --effort S \
    --verification "test -f WORKITEMS.md && grep -q 'Work Items' WORKITEMS.md"

echo "Migration complete! Run 'alfred workitem list' to see all items."
