#!/bin/bash
# Migration script: Alfred workitems → Kira
# Run this once to migrate all existing workitems to Kira

set -e

ALFRED_DB="${HOME}/.alfred/data/alfred.db"
LOG_FILE="${HOME}/.alfred/migration.log"

echo "=== Migration started at $(date -u '+%Y-%m-%dT%H:%M:%SZ') ===" | tee -a "$LOG_FILE"

# Check if SQLite database exists
if [ ! -f "$ALFRED_DB" ]; then
    echo "ERROR: Alfred database not found at $ALFRED_DB" | tee -a "$LOG_FILE"
    exit 1
fi

# Export workitems to JSON
echo "Exporting workitems from Alfred database..." | tee -a "$LOG_FILE"
WORKITEMS=$(sqlite3 "$ALFRED_DB" "
SELECT json_object(
    'id', id,
    'title', title,
    'description', description,
    'acceptance_criteria', acceptance_criteria,
    'status', status,
    'priority', priority,
    'category', category,
    'depends_on', depends_on,
    'verification_command', verification_command,
    'estimated_effort', estimated_effort
) FROM work_items
")

# Save to temp file
echo "$WORKITEMS" > /tmp/alfred_workitems.json
echo "Exported $(echo "$WORKITEMS" | wc -l) workitems" | tee -a "$LOG_FILE"

echo "" | tee -a "$LOG_FILE"
echo "=== Migration export complete ===" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"
echo "Now use the Kira MCP tools in an OpenCode session to create tickets:" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"
echo "  1. List exported workitems:" | tee -a "$LOG_FILE"
echo "     cat /tmp/alfred_workitems.json" | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"
echo "  2. Create Kira ticket for each workitem:" | tee -a "$LOG_FILE"
echo '     tools.kira.create_ticket({' | tee -a "$LOG_FILE"
echo '       title: "<workitem title>",' | tee -a "$LOG_FILE"
echo '       description: "<description>\\n\\nAcceptance Criteria:\\n<criteria>\\n\\nVerification: <command>",' | tee -a "$LOG_FILE"
echo '       priority: "<mapped priority>",' | tee -a "$LOG_FILE"
echo '       tags: ["<category>", "migrated-from-alfred"],' | tee -a "$LOG_FILE"
echo '       repo: "/home/azureuser/projects/alfred",' | tee -a "$LOG_FILE"
echo '       agent_type: "Build"' | tee -a "$LOG_FILE"
echo '     })' | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"
echo "  3. Verify migration:" | tee -a "$LOG_FILE"
echo '     tools.kira.list_tickets({ repo: "/home/azureuser/projects/alfred" })' | tee -a "$LOG_FILE"
echo "" | tee -a "$LOG_FILE"
