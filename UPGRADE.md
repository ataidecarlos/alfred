# Upgrade Guide

## Upgrading Alfred

### Automatic Upgrade (Linux/Mac)

Run the install script again. It will download and install the latest version while preserving your config files:

```bash
curl -fsSL https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.sh | sh
```

### Automatic Upgrade (Windows)

```powershell
iwr -useb https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.ps1 | iex
```

### Manual Upgrade

1. Download the latest release from [GitHub Releases](https://github.com/ataidecarlos/alfred/releases)

2. Stop the running server:
   ```bash
   # Find and kill the process
   pkill alfred
   # Or use Ctrl+C if running in foreground
   ```

3. Replace the binary:
   - **Linux/Mac:** `cp alfred ~/.local/bin/alfred`
   - **Windows:** `Copy-Item alfred.exe "$env:LOCALAPPDATA\bin\alfred.exe"`

4. Restart the server:
   ```bash
   alfred
   ```

## What's Preserved During Upgrade

- ✅ Config files (`~/.config/alfred/config.toml`)
- ✅ Prompt files (`~/.config/alfred/prompts/`)
- ✅ Database (`~/.local/share/alfred/alfred.db`)
- ✅ Logs (`~/.cache/alfred/server.log`)
- ❌ Binary (replaced with new version)

## Config Migration

If a new version adds config options:

1. Check the release notes for new config fields
2. Compare your config with `config.toml.example`
3. Add new fields manually
4. Existing fields are preserved

## Downgrading

If you need to downgrade:

1. Download the specific version from [GitHub Releases](https://github.com/ataidecarlos/alfred/releases)
2. Replace the binary
3. Check release notes for any config changes

## Backup

Before upgrading, consider backing up your config:

```bash
# Linux/Mac
tar -czf alfred-backup-$(date +%Y%m%d).tar.gz ~/.config/alfred ~/.local/share/alfred

# Windows (PowerShell)
Compress-Archive -Path "$env:APPDATA\alfred","$env:LOCALAPPDATA\alfred" -DestinationPath "alfred-backup-$(Get-Date -Format yyyyMMdd).zip"
```

## Release Versioning

Alfred uses date-based versioning: `YYYY.MM.DD`

Examples:
- `2026.09.04` - September 4, 2026
- `2026.12.25` - December 25, 2026

## Changelog

See [GitHub Releases](https://github.com/ataidecarlos/alfred/releases) for detailed changelogs.
