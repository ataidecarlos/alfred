#!/bin/bash
set -e

# Alfred Package Release Script
# Creates release archives with binaries and config templates
# Version is calculated as YYYYMMDD based on current date

VERSION=$(date +%Y%m%d)
RELEASE_DIR="releases"
BUILD_DIR="target/release-builds"

echo "Packaging Alfred v${VERSION}"
echo "==========================="

# Clean previous releases
rm -rf ${RELEASE_DIR}
mkdir -p ${RELEASE_DIR}

# Get current directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Check if builds exist
if [ ! -d "${BUILD_DIR}" ]; then
    echo "Error: Build directory not found."
    echo "Run: ./scripts/build-release.sh"
    exit 1
fi

# Create archives for each platform
TARGETS=(
    "x86_64-unknown-linux-gnu:linux-x64"
    "aarch64-unknown-linux-gnu:linux-arm64"
    "x86_64-apple-darwin:macos-x64"
    "aarch64-apple-darwin:macos-arm64"
    "x86_64-pc-windows-msvc:windows-x64"
)

for TARGET_INFO in "${TARGETS[@]}"; do
    IFS=':' read -r TARGET PLATFORM <<< "$TARGET_INFO"

    TARGET_DIR="${BUILD_DIR}/${TARGET}"
    if [ ! -d "${TARGET_DIR}" ]; then
        echo "Skipping ${TARGET} - build not found"
        continue
    fi

    echo ""
    echo "Packaging for ${PLATFORM}..."

    # Create staging directory
    STAGING_DIR="${RELEASE_DIR}/alfred-v${VERSION}-${PLATFORM}"
    mkdir -p "${STAGING_DIR}/prompts"

    # Copy binary (rename to just 'alfred')
    if [ "${TARGET}" = "x86_64-pc-windows-msvc" ]; then
        BINARY_EXT=".exe"
    else
        BINARY_EXT=""
    fi

    cp "${TARGET_DIR}/alfred-v${VERSION}${BINARY_EXT}" "${STAGING_DIR}/alfred${BINARY_EXT}"

    # Copy config templates
    cp config/config.toml.example "${STAGING_DIR}/" 2>/dev/null || true
    cp prompts/system.md.example "${STAGING_DIR}/prompts/" 2>/dev/null || true
    cp prompts/user.md.example "${STAGING_DIR}/prompts/" 2>/dev/null || true

    # Copy README
    cp README.md "${STAGING_DIR}/"

    # Create archive
    cd "${RELEASE_DIR}"
    if [ "${TARGET}" = "x86_64-pc-windows-msvc" ]; then
        # Create zip for Windows
        zip -r "alfred-v${VERSION}-${PLATFORM}.zip" "alfred-v${VERSION}-${PLATFORM}/"
        ARCHIVE_NAME="alfred-v${VERSION}-${PLATFORM}.zip"
    else
        # Create tarball for Linux/Mac
        tar -czf "alfred-v${VERSION}-${PLATFORM}.tar.gz" "alfred-v${VERSION}-${PLATFORM}/"
        ARCHIVE_NAME="alfred-v${VERSION}-${PLATFORM}.tar.gz"
    fi
    cd "${PROJECT_DIR}"

    # Generate checksum
    cd "${RELEASE_DIR}"
    if [ -f "${ARCHIVE_NAME}" ]; then
        sha256sum "${ARCHIVE_NAME}" > "${ARCHIVE_NAME}.sha256"
        echo "  Created: ${ARCHIVE_NAME}"
    fi
    cd "${PROJECT_DIR}"
done

echo ""
echo "==========================="
echo "Packaging complete!"
echo "Version: ${VERSION}"
echo "Archives are in: ${RELEASE_DIR}"
