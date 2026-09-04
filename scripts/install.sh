#!/bin/bash
set -e

# Alfred Installer
# Usage: curl -fsSL https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.sh | sh

ALFRED_VERSION="${1:-latest}"
INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/alfred"
DATA_DIR="${HOME}/.local/share/alfred"
CACHE_DIR="${HOME}/.cache/alfred"

GITHUB_REPO="ataidecarlos/alfred"
GITHUB_API="https://api.github.com/repos/${GITHUB_REPO}"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

log_info() {
    echo -e "${GREEN}[INFO]${NC} $1"
}

log_warn() {
    echo -e "${YELLOW}[WARN]${NC} $1"
}

log_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Detect OS and architecture
detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64) PLATFORM="linux-x64" ;;
                aarch64|arm64) PLATFORM="linux-arm64" ;;
                *) log_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        darwin)
            case "$ARCH" in
                x86_64) PLATFORM="macos-x64" ;;
                arm64) PLATFORM="macos-arm64" ;;
                *) log_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        *)
            log_error "Unsupported OS: $OS"
            log_info "For Windows, use install.ps1 instead"
            exit 1
            ;;
    esac

    log_info "Detected platform: ${PLATFORM}"
}

# Get latest version from GitHub
get_latest_version() {
    if [ "$ALFRED_VERSION" = "latest" ]; then
        ALFRED_VERSION=$(curl -s "${GITHUB_API}/releases/latest" | grep '"tag_name"' | cut -d '"' -f 4 | sed 's/^v//')
        if [ -z "$ALFRED_VERSION" ]; then
            log_error "Failed to get latest version"
            exit 1
        fi
    fi
    log_info "Version: ${ALFRED_VERSION}"
}

# Download release
download_release() {
    local DOWNLOAD_URL="https://github.com/${GITHUB_REPO}/releases/download/v${ALFRED_VERSION}/alfred-v${ALFRED_VERSION}-${PLATFORM}.tar.gz"
    local TEMP_DIR=$(mktemp -d)

    log_info "Downloading ${DOWNLOAD_URL}..."

    curl -fsSL "${DOWNLOAD_URL}" -o "${TEMP_DIR}/alfred.tar.gz"

    # Verify checksum
    curl -fsSL "${DOWNLOAD_URL}.sha256" -o "${TEMP_DIR}/alfred.tar.gz.sha256"
    cd "${TEMP_DIR}"
    if ! sha256sum -c alfred.tar.gz.sha256; then
        log_error "Checksum verification failed"
        rm -rf "${TEMP_DIR}"
        exit 1
    fi
    cd -

    # Extract
    tar -xzf "${TEMP_DIR}/alfred.tar.gz" -C "${TEMP_DIR}"

    # Find the extracted directory
    EXTRACTED_DIR=$(find "${TEMP_DIR}" -maxdepth 1 -type d -name "alfred-*" | head -1)

    if [ -z "$EXTRACTED_DIR" ]; then
        log_error "Failed to extract release"
        rm -rf "${TEMP_DIR}"
        exit 1
    fi

    echo "${EXTRACTED_DIR}"
}

# Install binary
install_binary() {
    local EXTRACTED_DIR=$1

    log_info "Installing binary to ${INSTALL_DIR}..."

    mkdir -p "${INSTALL_DIR}"

    # Copy binary (renamed from alfred-v* to alfred)
    cp "${EXTRACTED_DIR}/alfred" "${INSTALL_DIR}/alfred"
    chmod +x "${INSTALL_DIR}/alfred"

    log_info "Binary installed: ${INSTALL_DIR}/alfred"
}

# Install config files
install_config() {
    local EXTRACTED_DIR=$1

    log_info "Installing config files to ${CONFIG_DIR}..."

    mkdir -p "${CONFIG_DIR}"
    mkdir -p "${CONFIG_DIR}/prompts"

    # Copy config template (skip if exists)
    if [ ! -f "${CONFIG_DIR}/config.toml" ]; then
        cp "${EXTRACTED_DIR}/config.toml.example" "${CONFIG_DIR}/config.toml"
        log_info "Created config: ${CONFIG_DIR}/config.toml"
        log_warn "Please edit ${CONFIG_DIR}/config.toml and add your API keys"
    else
        log_info "Config already exists, skipping"
    fi

    # Copy prompt templates (skip if exists)
    for f in system.md user.md; do
        if [ ! -f "${CONFIG_DIR}/prompts/${f}" ]; then
            cp "${EXTRACTED_DIR}/prompts/${f}.example" "${CONFIG_DIR}/prompts/${f}"
        fi
    done
}

# Create data and cache directories
create_dirs() {
    log_info "Creating data directories..."

    mkdir -p "${DATA_DIR}"
    mkdir -p "${CACHE_DIR}"

    log_info "Data directory: ${DATA_DIR}"
    log_info "Cache directory: ${CACHE_DIR}"
}

# Check PATH
check_path() {
    if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
        log_warn "${INSTALL_DIR} is not in your PATH"
        log_info "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"${INSTALL_DIR}:\$PATH\""
        echo ""
    fi
}

# Cleanup
cleanup() {
    local TEMP_DIR=$1
    rm -rf "${TEMP_DIR}"
}

# Main installation
main() {
    log_info "Installing Alfred..."
    echo ""

    detect_platform
    get_latest_version

    EXTRACTED_DIR=$(download_release)

    install_binary "${EXTRACTED_DIR}"
    install_config "${EXTRACTED_DIR}"
    create_dirs

    cleanup "$(dirname "${EXTRACTED_DIR}")"

    echo ""
    log_info "Installation complete!"
    echo ""
    log_info "Next steps:"
    log_info "  1. Add ${INSTALL_DIR} to your PATH (if not already)"
    log_info "  2. Edit ${CONFIG_DIR}/config.toml with your API keys"
    log_info "  3. Run 'alfred' to start the server"
    log_info "  4. Run 'alfred --tui' to start the terminal UI"
    echo ""

    check_path
}

main "$@"
