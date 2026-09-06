#!/bin/bash
set -e

# Alfred Update Script
# Checks for newer version and updates the installation
# For private repos: GH_PAT=your_token ./update.sh

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

# Configuration
GITHUB_REPO="ataidecarlos/alfred"
GITHUB_API="https://api.github.com/repos/${GITHUB_REPO}"
INSTALL_DIR="${HOME}/.local/bin"
CONFIG_DIR="${HOME}/.config/alfred"
DATA_DIR="${HOME}/.local/share/alfred"
CACHE_DIR="${HOME}/.cache/alfred"

# Authentication
if [ -n "$GH_PAT" ]; then
    AUTH_HEADER="Authorization: token $GH_PAT"
else
    AUTH_HEADER=""
fi

# Get current installed version
get_installed_version() {
    if [ -f "${INSTALL_DIR}/alfred" ]; then
        # Try to get version from binary
        INSTALLED_VERSION=$("${INSTALL_DIR}/alfred" --version 2>/dev/null | grep -oE '[0-9]{8}' | head -1)
        if [ -n "${INSTALLED_VERSION}" ]; then
            echo "${INSTALLED_VERSION}"
            return 0
        fi
    fi
    echo "none"
}

# Get latest version from GitHub
get_latest_version() {
    if [ -n "$AUTH_HEADER" ]; then
        LATEST_VERSION=$(curl -s -H "$AUTH_HEADER" "${GITHUB_API}/releases/latest" | grep '"tag_name"' | cut -d '"' -f 4 | sed 's/^v//')
    else
        LATEST_VERSION=$(curl -s "${GITHUB_API}/releases/latest" | grep '"tag_name"' | cut -d '"' -f 4 | sed 's/^v//')
    fi
    if [ -z "${LATEST_VERSION}" ]; then
        log_error "Failed to get latest version"
        exit 1
    fi
    echo "${LATEST_VERSION}"
}

# Download and install update
install_update() {
    local VERSION=$1
    local PLATFORM=$2
    local ARCHIVE_NAME="alfred-v${VERSION}-${PLATFORM}.tar.gz"
    local TEMP_DIR=$(mktemp -d)

    # Get release assets via API
    local RELEASE_JSON
    if [ -n "$AUTH_HEADER" ]; then
        RELEASE_JSON=$(curl -s -H "$AUTH_HEADER" "${GITHUB_API}/releases/tags/v${VERSION}")
    else
        RELEASE_JSON=$(curl -s "${GITHUB_API}/releases/tags/v${VERSION}")
    fi

    # Find the asset URL
    local ASSET_URL
    ASSET_URL=$(echo "$RELEASE_JSON" | grep -o "\"browser_download_url\": \"[^\"]*${ARCHIVE_NAME}\"" | cut -d '"' -f 4)

    if [ -z "$ASSET_URL" ]; then
        log_error "Could not find download URL for ${ARCHIVE_NAME}"
        rm -rf "${TEMP_DIR}"
        exit 1
    fi

    log_info "Downloading ${ARCHIVE_NAME}..."

    if [ -n "$AUTH_HEADER" ]; then
        curl -fsSL -H "$AUTH_HEADER" "${ASSET_URL}" -o "${TEMP_DIR}/alfred.tar.gz"
    else
        curl -fsSL "${ASSET_URL}" -o "${TEMP_DIR}/alfred.tar.gz"
    fi

    # Verify checksum
    local CHECKSUM_URL="${ASSET_URL}.sha256"
    if [ -n "$AUTH_HEADER" ]; then
        curl -fsSL -H "$AUTH_HEADER" "${CHECKSUM_URL}" -o "${TEMP_DIR}/alfred.tar.gz.sha256" 2>/dev/null || true
    else
        curl -fsSL "${CHECKSUM_URL}" -o "${TEMP_DIR}/alfred.tar.gz.sha256" 2>/dev/null || true
    fi

    if [ -f "${TEMP_DIR}/alfred.tar.gz.sha256" ]; then
        cd "${TEMP_DIR}"
        if ! sha256sum -c alfred.tar.gz.sha256; then
            log_error "Checksum verification failed"
            rm -rf "${TEMP_DIR}"
            exit 1
        fi
        cd -
    else
        log_warn "Checksum file not found, skipping verification"
    fi

    # Extract
    tar -xzf "${TEMP_DIR}/alfred.tar.gz" -C "${TEMP_DIR}"

    # Find the extracted directory
    EXTRACTED_DIR=$(find "${TEMP_DIR}" -maxdepth 1 -type d -name "alfred-*" | head -1)

    if [ -z "$EXTRACTED_DIR" ]; then
        log_error "Failed to extract update"
        rm -rf "${TEMP_DIR}"
        exit 1
    fi

    # Stop running server if any
    if pgrep -x "alfred" > /dev/null; then
        log_info "Stopping running Alfred server..."
        pkill -x "alfred" 2>/dev/null || true
        sleep 2
    fi

    # Update binary
    log_info "Updating binary..."
    cp "${EXTRACTED_DIR}/alfred" "${INSTALL_DIR}/alfred"
    chmod +x "${INSTALL_DIR}/alfred"

    # Check for new config templates
    log_info "Checking for config updates..."
    if [ -f "${EXTRACTED_DIR}/config.toml.example" ]; then
        if [ -f "${CONFIG_DIR}/config.toml" ]; then
            log_warn "Config file exists, skipping template update"
            log_info "Your config: ${CONFIG_DIR}/config.toml"
        else
            mkdir -p "${CONFIG_DIR}"
            cp "${EXTRACTED_DIR}/config.toml.example" "${CONFIG_DIR}/config.toml"
            log_info "Created new config: ${CONFIG_DIR}/config.toml"
        fi
    fi

    # Check for prompt updates
    for f in system.md user.md; do
        if [ -f "${EXTRACTED_DIR}/prompts/${f}.example" ]; then
            if [ -f "${CONFIG_DIR}/prompts/${f}" ]; then
                log_warn "Prompt file ${f} exists, skipping"
            else
                mkdir -p "${CONFIG_DIR}/prompts"
                cp "${EXTRACTED_DIR}/prompts/${f}.example" "${CONFIG_DIR}/prompts/${f}"
            fi
        fi
    done

    # Cleanup
    rm -rf "${TEMP_DIR}"

    log_info "Update complete!"
}

# Detect platform
detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)

    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64) echo "linux-x64" ;;
                aarch64|arm64) echo "linux-arm64" ;;
                *) log_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        darwin)
            case "$ARCH" in
                x86_64) echo "macos-x64" ;;
                arm64) echo "macos-arm64" ;;
                *) log_error "Unsupported architecture: $ARCH"; exit 1 ;;
            esac
            ;;
        *)
            log_error "Unsupported OS: $OS"
            log_info "For Windows, use install.ps1 instead"
            exit 1
            ;;
    esac
}

# Main update process
main() {
    log_info "Checking for Alfred updates..."
    echo ""

    INSTALLED_VERSION=$(get_installed_version)
    LATEST_VERSION=$(get_latest_version)

    log_info "Installed version: ${INSTALLED_VERSION}"
    log_info "Latest version: ${LATEST_VERSION}"

    if [ "${INSTALLED_VERSION}" = "${LATEST_VERSION}" ]; then
        log_info "Already up to date!"
        exit 0
    fi

    echo ""
    log_warn "New version available: ${LATEST_VERSION}"
    read -p "Do you want to update? (y/N) " -n 1 -r
    echo ""

    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        log_info "Update cancelled"
        exit 0
    fi

    PLATFORM=$(detect_platform)
    install_update "${LATEST_VERSION}" "${PLATFORM}"

    echo ""
    log_info "Restarting Alfred server..."
    if [ -f "${INSTALL_DIR}/alfred" ]; then
        "${INSTALL_DIR}/alfred" &
        log_info "Alfred server started in background"
    fi
}

main "$@"
