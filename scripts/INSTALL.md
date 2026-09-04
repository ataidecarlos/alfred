# Installation Guide

## Automated Installation

### Linux/Mac

```bash
curl -fsSL https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.sh | sh
```

This will:
1. Detect your OS and architecture
2. Download the latest release
3. Install binary to `~/.local/bin/`
4. Create config directory at `~/.config/alfred/`
5. Create data directory at `~/.local/share/alfred/`
6. Create cache directory at `~/.cache/alfred/`

### Windows (PowerShell)

```powershell
iwr -useb https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.ps1 | iex
```

This will:
1. Download the latest Windows release
2. Install binary to `%LOCALAPPDATA%\bin\`
3. Create config directory at `%APPDATA%\alfred\`
4. Create data directory at `%LOCALAPPDATA%\alfred\`
5. Add binary to your PATH

## Manual Installation

### 1. Download

Download the latest release from [GitHub Releases](https://github.com/ataidecarlos/alfred/releases):

- **Linux x64**: `alfred-YYYY.MM.DD-linux-x64.tar.gz`
- **Linux ARM64**: `alfred-YYYY.MM.DD-linux-arm64.tar.gz`
- **macOS x64**: `alfred-YYYY.MM.DD-macos-x64.tar.gz`
- **macOS ARM64**: `alfred-YYYY.MM.DD-macos-arm64.tar.gz`
- **Windows x64**: `alfred-YYYY.MM.DD-windows-x64.zip`

### 2. Extract

**Linux/Mac:**
```bash
tar -xzf alfred-YYYY.MM.DD-linux-x64.tar.gz
cd alfred-YYYY.MM.DD-linux-x64
```

**Windows:**
```powershell
Expand-Archive -Path alfred-YYYY.MM.DD-windows-x64.zip -DestinationPath alfred
cd alfred
```

### 3. Install Binary

**Linux/Mac:**
```bash
mkdir -p ~/.local/bin
cp alfred ~/.local/bin/
chmod +x ~/.local/bin/alfred
```

**Windows:**
```powershell
New-Item -ItemType Directory -Path "$env:LOCALAPPDATA\bin" -Force
Copy-Item alfred.exe "$env:LOCALAPPDATA\bin\"
```

### 4. Create Directories

**Linux/Mac:**
```bash
mkdir -p ~/.config/alfred/prompts
mkdir -p ~/.local/share/alfred
mkdir -p ~/.cache/alfred
```

**Windows:**
```powershell
New-Item -ItemType Directory -Path "$env:APPDATA\alfred\prompts" -Force
New-Item -ItemType Directory -Path "$env:LOCALAPPDATA\alfred" -Force
New-Item -ItemType Directory -Path "$env:LOCALAPPDATA\alfred\cache" -Force
```

### 5. Copy Config Templates

**Linux/Mac:**
```bash
cp config.toml.example ~/.config/alfred/
cp prompts/system.md.example ~/.config/alfred/prompts/
cp prompts/user.md.example ~/.config/alfred/prompts/
```

**Windows:**
```powershell
Copy-Item config.toml.example "$env:APPDATA\alfred\"
Copy-Item prompts\system.md.example "$env:APPDATA\alfred\prompts\"
Copy-Item prompts\user.md.example "$env:APPDATA\alfred\prompts\"
```

### 6. Configure

Edit your config file with your API keys:

**Linux/Mac:**
```bash
nano ~/.config/alfred/config.toml
```

**Windows:**
```powershell
notepad "$env:APPDATA\alfred\config.toml"
```

### 7. Add to PATH (if not already)

**Linux/Mac:**
Add to `~/.bashrc` or `~/.zshrc`:
```bash
export PATH="$HOME/.local/bin:$PATH"
```

**Windows:**
The installer automatically adds to your user PATH. Restart your terminal.

## Directory Structure

**Linux/Mac:**
```
~/.local/bin/alfred              # Binary
~/.config/alfred/                # Config
  ├── config.toml
  └── prompts/
      ├── system.md
      └── user.md
~/.local/share/alfred/           # Data
  └── alfred.db
~/.cache/alfred/                 # Cache
  └── server.log
```

**Windows:**
```
%LOCALAPPDATA%\bin\alfred.exe    # Binary
%APPDATA%\alfred\                # Config
  ├── config.toml
  └── prompts/
      ├── system.md
      └── user.md
%LOCALAPPDATA%\alfred\           # Data
  └── alfred.db
%LOCALAPPDATA%\alfred\cache\     # Cache
  └── server.log
```

## Troubleshooting

### "alfred: command not found"

Make sure `~/.local/bin` (Linux/Mac) or `%LOCALAPPDATA%\bin` (Windows) is in your PATH.

### "Permission denied" on Linux/Mac

```bash
chmod +x ~/.local/bin/alfred
```

### Config file not found

Run `alfred` once to auto-generate the config directory and files.

### Logs

Check logs for errors:

**Linux/Mac:** `~/.cache/alfred/server.log`
**Windows:** `%LOCALAPPDATA%\alfred\cache\server.log`
