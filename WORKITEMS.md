# Kira Ticketing System

Alfred uses Kira for ticket management. Kira is a shared ticketing system for AI agents across all projects.

## Access

- **Local (stdio mode):** Configured globally in `~/.config/opencode/opencode.jsonc`
- **Available via:** `tools.kira.*`

## Tools

| Tool | Description |
|------|-------------|
| `create_ticket(title, description, priority?, review_required?, tags?, repo, agent_type)` | Create ticket |
| `claim_ticket(id, repo, agent_type)` | Claim OPEN ticket |
| `update_status(id, status, repo, agent_type)` | Transition: OPEN → IN_PROGRESS → CLOSED |
| `add_comment(ticket_id, body, repo, agent_type)` | Add comment |
| `list_tickets(status?, repo?, agent_type?, tag?, priority?)` | List with filters |
| `get_ticket(id)` | Full details with comments |
| `my_tickets(repo, agent_type)` | Tickets claimed by this agent |
| `stats()` | Counts by status/priority |

## Identity

Agents self-report identity in tool args:
- `repo`: `/home/azureuser/projects/alfred`
- `agent_type`: `Build`

## Workflow

```
OPEN → IN_PROGRESS → CLOSED
```

## Priority Rules

- **Project tasks:** LOW (default), MEDIUM (if has dependencies)
- **HOST tasks:** HIGH (default), CRITICAL (security concern + complete blocker)

## Autonomous Workflow

### Setup

```bash
# Setup cron job (runs every 30 minutes: */30 * * * *)
bash scripts/setup_autonomous_cron.sh

# Verify
crontab -l  # Linux
```

The scheduler runs every 30 minutes (`*/30 * * * *`). Each run lists the open
tickets in Kira, claims the highest-priority one, and works it to completion.

### Manual Trigger

```bash
bash scripts/run_autonomous_agent.sh
```

### Monitor

```bash
# Watch agent log
tail -f ~/.alfred/agent.log

# Check ticket status
# Use tools.kira.list_tickets in an opencode session
```

## Error Handling

If the autonomous agent encounters an error it cannot fix:
1. Create a HIGH priority ticket with error details
2. Exit cleanly
3. Next run will pick up the error ticket

## Migration from Alfred Workitems

To migrate existing Alfred workitems to Kira:

```bash
bash scripts/migrate_workitems_to_kira.sh
```

Then use Kira MCP tools to create tickets from the exported data.

## Database

- **Kira database:** `~/.opencode/kira.db` (WAL mode, never erased)
- **Agent logs:** `~/.alfred/agent.log`
