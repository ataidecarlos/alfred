# Dogfooding: Alfred building Alfred

Alfred runs its own autonomous developer agent on a 30-minute schedule. Each run
pulls the highest-priority open ticket from [Kira](https://github.com/ataidecarlos/kira),
implements it in this repository, verifies it, and reports back to the ticket.

This document describes the production dogfooding setup on the `azure-alfred` VM.

## Host

| Item | Value |
|------|-------|
| SSH | `ssh azure-alfred` (`ataide-alfred.northeurope.cloudapp.azure.com`) |
| Repo checkout | `/home/azureuser/projects/alfred` |
| Binary | `/home/azureuser/projects/alfred/target/release/alfred` (symlinked to `~/.local/bin/alfred`) |
| Config | `/home/azureuser/.alfred/config/config.toml` |
| Data | `/home/azureuser/.alfred/data/alfred.db` |
| Logs | `/home/azureuser/.alfred/logs/alfred.log`, `/home/azureuser/.alfred/agent.log` |
| Service | `alfred.service` (systemd, enabled, `Restart=on-failure`) |

The server binds `0.0.0.0:8080`. Health check: `curl localhost:8080/health`.

## Scheduler

The autonomous agent runs every 30 minutes from the user crontab:

```cron
*/30 * * * * /home/azureuser/projects/alfred/scripts/run_autonomous_agent.sh >> /home/azureuser/.alfred/agent.log 2>&1
```

`scripts/run_autonomous_agent.sh`:

1. Loads `OPENCODE_GO_API_KEY` and `KIRA_API_KEY` from the AKC vault
   (`~/ataide-keychain`; password read from `~/.alfred/akc.pass`).
2. Runs `opencode run --standalone --file prompts/autonomous_developer.md`
   from the repository root. `--standalone` gives the run a private server so it
   inherits the freshly exported secrets.
3. Appends everything to `~/.alfred/agent.log`.

Install or refresh the cron entry with:

```bash
bash scripts/setup_autonomous_cron.sh
```

## Kira integration

The agent talks to Kira through the OpenCode MCP server configured in
`~/.config/opencode/opencode.jsonc`:

```jsonc
{
  "mcp": {
    "servers": {
      "kira": {
        "type": "remote",
        "url": "http://ataide-kira.northeurope.cloudapp.azure.com:3000/mcp",
        "oauth": false,
        "headers": { "Authorization": "Bearer {env:KIRA_API_KEY}" }
      }
    }
  }
}
```

`prompts/autonomous_developer.md` drives the workflow:

1. `tools.kira.list_tickets(repo="/home/azureuser/projects/alfred", agent_type="Build", status="OPEN")`
2. Claim the highest-priority ticket with `tools.kira.claim_ticket`
3. Implement and commit the change
4. Run the ticket's verification
5. On success, `tools.kira.add_comment` + `tools.kira.update_status(..., "CLOSED")`

## Error handling

If a run hits an error it cannot fix immediately, it creates a HIGH priority
ticket titled `Error: <description>` (tags `error`, `blocked`) and exits cleanly.
The next run picks the error ticket up. See the "Autonomous Agent Error Handling"
section in `AGENTS.md`.

## Logs

```bash
# Server log
tail -f ~/.alfred/logs/alfred.log

# Autonomous agent runs
tail -f ~/.alfred/agent.log
```

Rotation is handled by `/etc/logrotate.d/alfred` (daily, keep 7, compress,
`copytruncate`).

## Secrets

Secrets live in the AKC encrypted vault at `~/ataide-keychain` on the VM:

| Key | Purpose |
|-----|---------|
| `OPENCODE_AZURE-DEV` | OpenCode Go API key (LLM gateway) |
| `KIRA_API_KEY` | Kira MCP bearer token |

The vault password is stored in `~/.alfred/akc.pass` (mode `0600`) so the
systemd service and cron can read secrets non-interactively. The LLM key is
exported by `~/.alfred/bin/alfred-server.sh`, which the systemd unit runs.

## Troubleshooting

| Symptom | Check |
|---------|-------|
| Server not up | `sudo systemctl status alfred`; `sudo journalctl -u alfred -n 50` |
| Health fails | `curl localhost:8080/health`; confirm port 8080 in `config.toml` |
| LLM errors | `akc get ~/ataide-keychain OPENCODE_AZURE-DEV --password "$(cat ~/.alfred/akc.pass)"` returns a value; check `/etc/alfred/`-equivalent env wiring |
| Kira unreachable | `curl -I http://ataide-kira.northeurope.cloudapp.azure.com:3000/health` |
| Agent finds no tickets | `opencode mcp list` while exporting `KIRA_API_KEY`; confirm the `kira` server connects (9 tools) |
| Scheduler not firing | `crontab -l`; `tail ~/.alfred/agent.log` |
| Bad config after edit | Alfred hot-reloads `config.toml`; check `journalctl -u alfred` |
