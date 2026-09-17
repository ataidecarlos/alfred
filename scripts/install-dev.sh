#!/usr/bin/env bash
set -euo pipefail

# ──────────────────────────────────────────────────────────────────────
# Alfred Dev Install Script
# Builds from source, installs to ~/.local/bin, injects API key.
# On any error, cleans up the entire environment.
# ──────────────────────────────────────────────────────────────────────

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL_DIR="$HOME/.local/bin"
ALFRED_HOME="$HOME/.alfred"
CONFIG_DIR="$ALFRED_HOME/config"
PROMPTS_DIR="$CONFIG_DIR/prompts"
DATA_DIR="$ALFRED_HOME/data"
LOG_DIR="$ALFRED_HOME/logs"
VAULT_DIR="$HOME/alfred"
BINARY="$INSTALL_DIR/alfred"
KEY_FILE="$PROJECT_ROOT/test_api_keys.toml"

# ── Colors ────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

ok()   { echo -e "${GREEN}✓${NC} $1"; }
warn() { echo -e "${YELLOW}⚠${NC} $1"; }
fail() { echo -e "${RED}✗${NC} $1"; }

# ── Cleanup on error ──────────────────────────────────────────────────
cleanup() {
    echo ""
    warn "Cleaning up failed installation..."
    rm -f "$BINARY"
    rm -rf "$CONFIG_DIR"
    ok "Environment cleaned."
    echo ""
    fail "Installation failed. Re-run the script to retry."
    exit 1
}
trap cleanup ERR

# ── Step 1: Clean existing installation ───────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  Alfred Dev Install"
echo "═══════════════════════════════════════════════════════════"
echo ""

if [ -f "$BINARY" ]; then
    rm -f "$BINARY"
    ok "Removed existing binary"
fi

if [ -d "$CONFIG_DIR" ]; then
    rm -rf "$CONFIG_DIR"
    ok "Removed existing config"
fi

ok "Cleaned existing installation"

# ── Step 2: Provision build dependencies ──────────────────────────────

# Install system build dependencies
install_build_deps() {
    echo "Installing build dependencies..."
    if command -v apt-get &>/dev/null; then
        sudo apt-get update -qq
        sudo apt-get install -y -qq build-essential pkg-config libssl-dev sqlite3 libsqlite3-dev
    elif command -v dnf &>/dev/null; then
        sudo dnf install -y gcc make pkg-config openssl-devel sqlite-devel sqlite
    elif command -v pacman &>/dev/null; then
        sudo pacman -S --noconfirm base-devel openssl sqlite
    elif command -v apk &>/dev/null; then
        apk add --no-cache build-base pkgconf openssl-dev sqlite-dev sqlite
    else
        warn "Unknown package manager — install build-essential, pkg-config, libssl-dev, sqlite3 manually"
    fi
}

# Source cargo environment if available
if [ -f "$HOME/.cargo/env" ]; then
    source "$HOME/.cargo/env"
fi

if ! command -v cargo &>/dev/null; then
    ok "Rust not found — installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi
ok "Rust toolchain: $(rustc --version)"

# Install build dependencies (idempotent)
install_build_deps
ok "Build dependencies installed"

# ── Step 3: Build Alfred ──────────────────────────────────────────────
echo ""
echo "Building Alfred (this may take a few minutes)..."
cd "$PROJECT_ROOT"
cargo build --release 2>&1 | tail -1

if [ ! -f "target/release/alfred" ]; then
    fail "Build failed: target/release/alfred not found"
    exit 1
fi
ok "Built Alfred"

# ── Step 4: Install binary ───────────────────────────────────────────
mkdir -p "$INSTALL_DIR"
cp target/release/alfred "$BINARY"
chmod +x "$BINARY"

if ! "$BINARY" --version &>/dev/null; then
    fail "Installed binary failed to run"
    exit 1
fi
VERSION=$("$BINARY" --version 2>&1 | head -1)
ok "Installed $VERSION to $BINARY"

# ── Step 5: Check and fix PATH ───────────────────────────────────────
if ! echo "$PATH" | grep -q "$INSTALL_DIR"; then
    # Add to ~/.bashrc if it exists, otherwise ~/.profile
    SHELL_RC="$HOME/.bashrc"
    if [ ! -f "$SHELL_RC" ]; then
        SHELL_RC="$HOME/.profile"
    fi

    if [ -f "$SHELL_RC" ]; then
        if ! grep -q "$INSTALL_DIR" "$SHELL_RC"; then
            echo "" >> "$SHELL_RC"
            echo "# Alfred" >> "$SHELL_RC"
            echo "export PATH=\"\$HOME/.local/bin:\$PATH\"" >> "$SHELL_RC"
        fi
    else
        echo "export PATH=\"\$HOME/.local/bin:\$PATH\"" > "$SHELL_RC"
    fi

    export PATH="$INSTALL_DIR:$PATH"
    ok "Added $INSTALL_DIR to PATH"
else
    ok "PATH verified"
fi

# ── Step 6: Create config directory and copy files ────────────────────
mkdir -p "$PROMPTS_DIR"

cp "$PROJECT_ROOT/config/config.toml.example" "$CONFIG_DIR/config.toml"

# Copy prompt files if examples exist, otherwise create defaults
if [ -f "$PROJECT_ROOT/prompts/system.md.example" ]; then
    cp "$PROJECT_ROOT/prompts/system.md.example" "$PROMPTS_DIR/system.md"
elif [ -f "$PROJECT_ROOT/prompts/system.md" ]; then
    cp "$PROJECT_ROOT/prompts/system.md" "$PROMPTS_DIR/system.md"
else
    cat > "$PROMPTS_DIR/system.md" << 'PROMPTEOF'
You are Alfred, a helpful AI assistant running as a 24/7 server.
You can manage to-do lists, store and recall memories, and execute shell commands.
Be concise and action-oriented.
PROMPTEOF
fi

if [ -f "$PROJECT_ROOT/prompts/user.md.example" ]; then
    cp "$PROJECT_ROOT/prompts/user.md.example" "$PROMPTS_DIR/user.md"
elif [ -f "$PROJECT_ROOT/prompts/user.md" ]; then
    cp "$PROJECT_ROOT/prompts/user.md" "$PROMPTS_DIR/user.md"
else
    echo "" > "$PROMPTS_DIR/user.md"
fi

ok "Configured prompts"

# Copy theme files (dark + light)
mkdir -p "$CONFIG_DIR/themes"
if [ -d "$PROJECT_ROOT/themes" ]; then
    cp "$PROJECT_ROOT/themes/"*.toml "$CONFIG_DIR/themes/" 2>/dev/null || true
    ok "Configured themes"
fi

# ── Step 7: Inject API key and model ─────────────────────────────────
if [ ! -f "$KEY_FILE" ]; then
    fail "API key file not found: $KEY_FILE"
    echo "  Copy test_api_keys.toml.example to test_api_keys.toml"
    echo "  and add your API key."
    exit 1
fi

# Extract API key
API_KEY=$(grep 'api_key' "$KEY_FILE" | head -1 | sed 's/.*= *"\(.*\)"/\1/')
if [ -z "$API_KEY" ]; then
    fail "Could not extract API key from $KEY_FILE"
    exit 1
fi

# Extract first model from preference_order
MODEL=$(grep -A 1 'preference_order' "$KEY_FILE" | grep '"' | head -1 | sed 's/.*"\(.*\)".*/\1/')
if [ -z "$MODEL" ]; then
    MODEL="union-alpha"
fi

# Update config.toml
CONFIG_FILE="$CONFIG_DIR/config.toml"

# Replace api_key
sed -i "s|api_key = \"\${OPENAI_API_KEY}\"|api_key = \"$API_KEY\"|" "$CONFIG_FILE"

# Replace model
sed -i "s|model = \"gpt-4o\"|model = \"$MODEL\"|" "$CONFIG_FILE"

# Add base_url after the model line if not present
if ! grep -q "base_url.*opencode.ai" "$CONFIG_FILE"; then
    sed -i "/^model = \"$MODEL\"$/a base_url = \"https://opencode.ai/zen/go/v1\"" "$CONFIG_FILE"
fi

ok "Configured API key (model: $MODEL)"

# ── Step 8: Set vault path and create directories ─────────────────────
mkdir -p "$DATA_DIR"
mkdir -p "$LOG_DIR"
mkdir -p "$VAULT_DIR"

sed -i "s|vault_path = \"~/alfred\"|vault_path = \"$VAULT_DIR\"|" "$CONFIG_FILE"
ok "Vault path: $VAULT_DIR"
ok "Data dir: $DATA_DIR"
ok "Logs dir: $LOG_DIR"

# ── Step 9: Verify installation ──────────────────────────────────────
echo ""
echo "Verifying installation..."

# Check binary runs
if ! "$BINARY" --version &>/dev/null; then
    fail "Binary verification failed"
    exit 1
fi

# Check config is valid
if [ ! -f "$CONFIG_DIR/config.toml" ]; then
    fail "Config file missing"
    exit 1
fi

ok "Installation verified"

# ── Step 10: Success message ─────────────────────────────────────────
VERSION=$("$BINARY" --version 2>&1 | head -1)
echo ""
echo "═══════════════════════════════════════════════════════════"
echo "  ✓ Installation complete!"
echo "═══════════════════════════════════════════════════════════"
echo ""
echo "  Binary:  $BINARY"
echo "  Config:  $CONFIG_DIR/config.toml"
echo "  Model:   $MODEL"
echo "  Vault:   $VAULT_PATH"
echo "  Version: $VERSION"
echo ""
echo "  Run:     alfred"
echo "  TUI:     alfred --tui"
echo ""
echo "═══════════════════════════════════════════════════════════"
echo ""
