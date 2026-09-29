#!/bin/bash
set -e

# Alfred installer — the two-part product: the `alfred` binary plus Pi
# (https://github.com/earendil-works/pi), which Alfred runs as a subprocess.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/ataidecarlos/alfred/main/scripts/install.sh | bash
#   GH_PAT=your_token curl -fsSL ... | bash      # private repo
#   bash install.sh 2026.09.04                   # pin a release
#
# Five idempotent steps:
#   1. Ensure Node >= 22.19, or fetch the standalone Pi binary.
#   2. Install Pi if `pi --version` fails.
#   3. Install the `alfred` release binary.
#   4. Seed ~/.alfred/config/ from the release's examples.
#   5. Print the next steps.
# Each step re-checks its own precondition, so a second run is a no-op.

ALFRED_VERSION="${1:-latest}"
NODE_MIN_MAJOR=22
NODE_MIN_MINOR=19
PI_PACKAGE="@earendil-works/pi-coding-agent"
PI_RELEASE_BASE="https://github.com/earendil-works/pi/releases/latest/download"

ALFRED_HOME="${HOME}/.alfred"
CONFIG_DIR="${ALFRED_HOME}/config"
INSTALL_DIR="${HOME}/.local/bin"
PI_STANDALONE_DIR="${ALFRED_HOME}/pi-standalone"
ALFRED_VERSION_FILE="${ALFRED_HOME}/installed-version"

GITHUB_REPO="ataidecarlos/alfred"
GITHUB_API="https://api.github.com/repos/${GITHUB_REPO}"

AUTH_HEADER=""
if [ -n "${GH_PAT:-}" ]; then
    AUTH_HEADER="Authorization: token ${GH_PAT}"
fi

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

log_info() { printf '%b[INFO]%b %s\n' "${GREEN}" "${NC}" "$1"; }
log_warn() { printf '%b[WARN]%b %s\n' "${YELLOW}" "${NC}" "$1"; }
die() { printf '%b[ERROR]%b %s\n' "${RED}" "${NC}" "$1" >&2; exit 1; }

# ── Helpers ───────────────────────────────────────────────────────────

# Detect the OS/arch. Alfred ships `linux-x64|linux-arm64|macos-x64|macos-arm64`;
# Pi ships `linux-x64|linux-arm64|darwin-x64|darwin-arm64`.
detect_platform() {
    OS=$(uname -s | tr '[:upper:]' '[:lower:]')
    ARCH=$(uname -m)
    case "$OS" in
        linux)
            case "$ARCH" in
                x86_64|amd64) PLATFORM="linux-x64"; PI_PLATFORM="linux-x64" ;;
                aarch64|arm64) PLATFORM="linux-arm64"; PI_PLATFORM="linux-arm64" ;;
                *) die "unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        darwin)
            case "$ARCH" in
                x86_64) PLATFORM="macos-x64"; PI_PLATFORM="darwin-x64" ;;
                arm64) PLATFORM="macos-arm64"; PI_PLATFORM="darwin-arm64" ;;
                *) die "unsupported architecture: ${ARCH}" ;;
            esac
            ;;
        *)
            die "unsupported OS: ${OS} (on Windows use scripts/install.ps1)"
            ;;
    esac
}

github_get() {
    if [ -n "${AUTH_HEADER}" ]; then
        curl -fsSL -H "${AUTH_HEADER}" "$1"
    else
        curl -fsSL "$1"
    fi
}

latest_alfred_version() {
    github_get "${GITHUB_API}/releases/latest" \
        | grep '"tag_name"' | head -1 | cut -d '"' -f 4 | sed 's/^v//'
}

# True when node and npm are both present and node is >= 22.19.
node_version_ok() {
    command -v node >/dev/null 2>&1 || return 1
    command -v npm >/dev/null 2>&1 || return 1
    NODE_VERSION=$(node --version 2>/dev/null) || return 1
    NODE_VERSION="${NODE_VERSION#v}"
    MAJOR="${NODE_VERSION%%.*}"
    REST="${NODE_VERSION#*.}"
    MINOR="${REST%%.*}"
    case "$MAJOR" in ''|*[!0-9]*) return 1 ;; esac
    case "$MINOR" in ''|*[!0-9]*) return 1 ;; esac
    [ "$MAJOR" -gt "${NODE_MIN_MAJOR}" ] && return 0
    [ "$MAJOR" -eq "${NODE_MIN_MAJOR}" ] && [ "$MINOR" -ge "${NODE_MIN_MINOR}" ] && return 0
    return 1
}

pi_on_path() {
    command -v pi >/dev/null 2>&1 && pi --version >/dev/null 2>&1
}

standalone_pi_ready() {
    [ -x "${PI_STANDALONE_DIR}/pi/pi" ] && "${PI_STANDALONE_DIR}/pi/pi" --version >/dev/null 2>&1
}

# ── Step 1: ensure a way to run Pi ────────────────────────────────────

step1_ensure_pi_prereq() {
    PI_METHOD=skip
    if pi_on_path; then
        log_info "Pi already installed: $(pi --version 2>/dev/null | head -1)"
        return 0
    fi
    if standalone_pi_ready; then
        log_info "Standalone Pi already present: ${PI_STANDALONE_DIR}/pi/pi"
        PI_METHOD=binary
        return 0
    fi
    if node_version_ok; then
        log_info "Node ${NODE_VERSION} meets the >= ${NODE_MIN_MAJOR}.${NODE_MIN_MINOR} requirement"
        PI_METHOD=npm
        return 0
    fi

    log_warn "Node >= ${NODE_MIN_MAJOR}.${NODE_MIN_MINOR} with npm not found; using the standalone Pi binary"
    command -v curl >/dev/null 2>&1 || die "curl is required to fetch the standalone Pi binary"
    command -v tar >/dev/null 2>&1 || die "tar is required to unpack the standalone Pi binary"
    PI_METHOD=binary
    PI_ARCHIVE="pi-${PI_PLATFORM}.tar.gz"
    PI_TMP=$(mktemp -d)

    log_info "Downloading ${PI_ARCHIVE}..."
    curl -fsSL "${PI_RELEASE_BASE}/${PI_ARCHIVE}" -o "${PI_TMP}/${PI_ARCHIVE}" \
        || die "failed to download ${PI_ARCHIVE} from ${PI_RELEASE_BASE}"
    if curl -fsSL "${PI_RELEASE_BASE}/SHA256SUMS" -o "${PI_TMP}/SHA256SUMS" 2>/dev/null; then
        grep " ${PI_ARCHIVE}\$" "${PI_TMP}/SHA256SUMS" > "${PI_TMP}/SHA256SUMS.selected" \
            || die "no checksum published for ${PI_ARCHIVE}"
        ( cd "${PI_TMP}" && sha256sum -c SHA256SUMS.selected ) \
            || die "checksum verification failed for ${PI_ARCHIVE}"
    else
        log_warn "Pi SHA256SUMS unavailable; skipping checksum verification"
    fi
}

# ── Step 2: install Pi ────────────────────────────────────────────────

step2_install_pi() {
    if pi_on_path; then
        return 0
    fi
    case "${PI_METHOD}" in
        npm)
            log_info "Installing ${PI_PACKAGE} with npm..."
            npm install -g --ignore-scripts --no-fund --no-audit "${PI_PACKAGE}" \
                || die "npm install of ${PI_PACKAGE} failed"
            ;;
        binary)
            if ! standalone_pi_ready; then
                log_info "Unpacking ${PI_ARCHIVE} into ${PI_STANDALONE_DIR}..."
                mkdir -p "${PI_STANDALONE_DIR}"
                tar -xzf "${PI_TMP}/${PI_ARCHIVE}" -C "${PI_STANDALONE_DIR}" \
                    || die "failed to unpack ${PI_ARCHIVE}"
                # The archive marks the binary executable, but some filesystems
                # drop that bit on extraction.
                chmod +x "${PI_STANDALONE_DIR}/pi/pi" 2>/dev/null || true
            fi
            [ -f "${PI_STANDALONE_DIR}/pi/pi" ] \
                || die "unexpected Pi archive layout: ${PI_STANDALONE_DIR}/pi/pi is missing"
            mkdir -p "${INSTALL_DIR}"
            ln -sf "${PI_STANDALONE_DIR}/pi/pi" "${INSTALL_DIR}/pi"
            ;;
        *)
            die "internal error: the Pi install method was not resolved in step 1"
            ;;
    esac

    # A `command -v pi` probe before the install can be cached by the shell;
    # refresh it so the verification below sees the freshly installed binary.
    hash -r 2>/dev/null || true
    if command -v pi >/dev/null 2>&1; then
        pi --version >/dev/null 2>&1 || die "installed pi at $(command -v pi) does not run"
        log_info "Pi installed: $(pi --version 2>/dev/null | head -1)"
    elif standalone_pi_ready; then
        log_warn "Pi installed at ${INSTALL_DIR}/pi, which is not on PATH; add ${INSTALL_DIR} to PATH"
    else
        die "Pi installation did not produce a runnable 'pi' binary"
    fi
}

# ── Step 3: install the alfred binary ─────────────────────────────────

fetch_alfred() {
    local version="$1"
    local archive="alfred-v${version}-${PLATFORM}.tar.gz"
    local url="https://github.com/${GITHUB_REPO}/releases/download/v${version}/${archive}"

    ALFRED_TMP=$(mktemp -d)
    log_info "Downloading ${archive}..."
    github_get "${url}" > "${ALFRED_TMP}/${archive}" \
        || die "failed to download ${archive} from ${url}"

    if github_get "${url}.sha256" > "${ALFRED_TMP}/${archive}.sha256" 2>/dev/null; then
        ( cd "${ALFRED_TMP}" && sha256sum -c "${archive}.sha256" ) \
            || die "checksum verification failed for ${archive}"
    else
        log_warn "No checksum published for ${archive}; skipping verification"
    fi

    tar -xzf "${ALFRED_TMP}/${archive}" -C "${ALFRED_TMP}" || die "failed to unpack ${archive}"
    EXTRACTED_DIR="${ALFRED_TMP}/alfred-v${version}-${PLATFORM}"
    [ -d "${EXTRACTED_DIR}" ] || die "unexpected archive layout in ${archive}"
}

# Everything step 3 and step 4 install for a given release is present. The
# config files count too, so the release only has to be downloaded when the
# binary or the seeded configuration is actually missing.
alfred_fully_installed() {
    [ -x "${INSTALL_DIR}/alfred" ] || return 1
    [ -f "${ALFRED_VERSION_FILE}" ] || return 1
    [ "$(cat "${ALFRED_VERSION_FILE}")" = "$1" ] || return 1
    [ -f "${CONFIG_DIR}/config.toml" ] || return 1
    [ -f "${CONFIG_DIR}/prompts/system.md" ] || return 1
    [ -f "${CONFIG_DIR}/prompts/user.md" ] || return 1
}

step3_install_alfred() {
    local target="${ALFRED_VERSION}"
    if [ "${target}" = "latest" ]; then
        target=$(latest_alfred_version)
        [ -n "${target}" ] || die "could not resolve the latest Alfred release (set GH_PAT for a private repo)"
    fi

    if alfred_fully_installed "${target}"; then
        log_info "Alfred ${target} already installed, skipping"
        return 0
    fi

    fetch_alfred "${target}"
    mkdir -p "${INSTALL_DIR}"
    cp "${EXTRACTED_DIR}/alfred" "${INSTALL_DIR}/alfred"
    chmod +x "${INSTALL_DIR}/alfred"
    "${INSTALL_DIR}/alfred" --version >/dev/null 2>&1 \
        || die "the installed alfred binary does not run (${INSTALL_DIR}/alfred)"

    mkdir -p "${ALFRED_HOME}"
    printf '%s\n' "${target}" > "${ALFRED_VERSION_FILE}"
    log_info "Alfred ${target} installed: ${INSTALL_DIR}/alfred"
}

# ── Step 4: seed the config directory ─────────────────────────────────

# Copy `from` to `to` unless `to` already exists. Returns non-zero only when the
# file must be created and the example is missing from the release.
seed_file() {
    if [ -f "$2" ]; then
        log_info "keeping existing $2"
        return 0
    fi
    [ -f "$1" ] || return 1
    cp "$1" "$2"
    log_info "created $2"
}

step4_seed_config() {
    if [ -f "${CONFIG_DIR}/config.toml" ] && [ -f "${CONFIG_DIR}/prompts/system.md" ] \
        && [ -f "${CONFIG_DIR}/prompts/user.md" ]; then
        log_info "Configuration already seeded, skipping"
        return 0
    fi
    [ -n "${EXTRACTED_DIR:-}" ] || die "cannot seed configuration: the Alfred release was not downloaded"

    log_info "Seeding configuration in ${CONFIG_DIR}..."
    mkdir -p "${CONFIG_DIR}/prompts"
    seed_file "${EXTRACTED_DIR}/config.toml.example" "${CONFIG_DIR}/config.toml" \
        || die "the Alfred release is missing config.toml.example"
    seed_file "${EXTRACTED_DIR}/prompts/system.md.example" "${CONFIG_DIR}/prompts/system.md" \
        || die "the Alfred release is missing prompts/system.md.example"
    seed_file "${EXTRACTED_DIR}/prompts/user.md.example" "${CONFIG_DIR}/prompts/user.md" \
        || die "the Alfred release is missing prompts/user.md.example"
}

# ── Step 5: next steps ────────────────────────────────────────────────

step5_next_steps() {
    echo ""
    log_info "Alfred and Pi are installed."
    echo ""
    echo "Next steps:"
    echo "  1. Add ${INSTALL_DIR} to PATH if it is not already:"
    echo "       export PATH=\"${INSTALL_DIR}:\$PATH\""
    echo "  2. Export the Pi provider key named by [pi].api_key_env (default PI_API_KEY):"
    echo "       export PI_API_KEY=..."
    echo "  3. Review ${CONFIG_DIR}/config.toml ([pi].provider, [pi].model, [pi].binary)."
    echo "  4. Start the server with 'alfred'."
    echo ""

    if [[ ":${PATH}:" != *":${INSTALL_DIR}:"* ]]; then
        log_warn "${INSTALL_DIR} is not on PATH in this shell"
    fi
}

main() {
    log_info "Installing Alfred (with Pi)..."
    detect_platform
    step1_ensure_pi_prereq
    step2_install_pi
    if [ -n "${PI_TMP:-}" ]; then
        rm -rf "${PI_TMP}"
    fi
    step3_install_alfred
    step4_seed_config
    if [ -n "${ALFRED_TMP:-}" ]; then
        rm -rf "${ALFRED_TMP}"
    fi
    step5_next_steps
}

main "$@"
