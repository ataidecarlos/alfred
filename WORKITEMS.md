# Work Items System

Alfred uses a SQLite-based work item system for autonomous development. An AI agent runs hourly via cron, picks the next unblocked item, implements it, and updates status.

## Database Location

- **Work Items:** `~/.alfred/workitems.db`
- **Agent Logs:** `~/.alfred/agent.log`

## Work Item Schema

| Field | Type | Description |
|-------|------|-------------|
| `id` | TEXT | UUID primary key |
| `title` | TEXT | Short description |
| `description` | TEXT | Detailed context |
| `acceptance_criteria` | TEXT | JSON array of measurable goals |
| `status` | TEXT | pending, in_progress, blocked, completed, failed |
| `priority` | TEXT | critical, high, medium, low |
| `category` | TEXT | infrastructure, feature, bug, experiment, refactor |
| `assigned_agent` | TEXT | Agent ID (null if unassigned) |
| `depends_on` | TEXT | JSON array of work item IDs |
| `verification_command` | TEXT | Shell command to verify completion |
| `estimated_effort` | TEXT | S, M, L, XL |

## CLI Commands

### List Work Items

```bash
# List all
alfred workitem list

# Filter by status
alfred workitem list --status pending
alfred workitem list --status in_progress
alfred workitem list --status completed
```

### Get Next Item

```bash
alfred workitem next
```

Returns highest priority unblocked pending item.

### Show Details

```bash
alfred workitem show <ID>
```

### Add Work Item

```bash
alfred workitem add \
    --title "My feature" \
    --description "Detailed description" \
    --priority high \
    --category feature \
    --effort M \
    --verification "cargo test" \
    --depends "ID1,ID2"
```

### Assign to Agent

```bash
alfred workitem assign <ID> <AGENT_ID>
```

### Update Status

```bash
alfred workitem update <ID> --status in_progress --note "Starting work"
```

### Complete Work Item

```bash
alfred workitem complete <ID> --verification "test output"
```

### Log Progress

```bash
alfred workitem log <ID> --note "Implemented auth middleware"
```

## Autonomous Workflow

### Setup

```bash
# Build release
cargo build --release

# Setup cron job
bash scripts/setup_autonomous_cron.sh

# Verify
crontab -l  # Linux
launchctl list | grep alfred  # macOS
```

### Manual Trigger

```bash
./target/release/alfred --prompt prompts/autonomous_developer.md
```

### Monitor

```bash
# Watch agent log
tail -f ~/.alfred/agent.log

# Check work item status
alfred workitem list --status in_progress
```

## Priority Levels

| Priority | Description |
|----------|-------------|
| **critical** | Must be done first, blocks everything |
| **high** | Important, do before medium/low |
| **medium** | Normal priority |
| **low** | Nice to have, do last |

## Status Flow

```
pending → in_progress → completed
    ↓           ↓
  blocked     failed
```

## Dependencies

Work items can depend on other items:

```bash
# Item B depends on Item A
alfred workitem add --title "Item A" --category infrastructure
# Returns: UUID-A

alfred workitem add --title "Item B" --depends "UUID-A" --category feature
```

Item B won't appear in `alfred workitem next` until Item A is completed.

## Adding New Work Items

1. Define clear acceptance criteria
2. Set appropriate priority
3. Choose category
4. Add verification command
5. Estimate effort (S/M/L/XL)

Example:
```bash
alfred workitem add \
    --title "Add user authentication" \
    --description "Implement JWT-based auth with login, register, and logout endpoints" \
    --priority high \
    --category feature \
    --effort L \
    --verification "curl -X POST localhost:3000/api/auth/login -d '{"email":"test@test.com","password":"test"}' | grep -q token"
```
