#!/bin/bash
set -e

# Alfred Package Release Script
# Creates release archives with binaries and config templates

VERSION=$(date +%Y.%m.%d)
RELEASE_DIR="releases"
BUILD_DIR="target/release-builds"

echo "Packaging Alfred v${VERSION}"

# Clean previous releases
rm -rf ${RELEASE_DIR}
mkdir -p ${RELEASE_DIR}

# Get current directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Check if builds exist
if [ ! -d "${BUILD_DIR}" ]; then
    echo "Error: Build directory not found. Run build-release.sh first."
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

    echo "Packaging for ${PLATFORM}..."

    # Create staging directory
    STAGING_DIR="${RELEASE_DIR}/alfred-${VERSION}-${PLATFORM}"
    mkdir -p "${STAGING_DIR}"

    # Copy binary
    if [ "${TARGET}" = "x86_64-pc-windows-msvc" ]; then
        BINARY_EXT=".exe"
    else
        BINARY_EXT=""
    fi

    cp "${TARGET_DIR}/alfred${BINARY_EXT}" "${STAGING_DIR}/"

    # Copy config templates
    cp config/config.toml.example "${STAGING_DIR}/" 2>/dev/null || true
    mkdir -p "${STAGING_DIR}/prompts"
    cp prompts/system.md.example "${STAGING_DIR}/prompts/" 2>/dev/null || true
    cp prompts/user.md.example "${STAGING_DIR}/prompts/" 2>/dev/null || true

    # Copy README
    cp README.md "${STAGING_DIR}/"

    # Create archive
    cd "${RELEASE_DIR}"
    if [ "${TARGET}" = "x86_64-pc-windows-msvc" ]; then
        # Create zip for Windows
        zip -r "alfred-${VERSION}-${PLATFORM}.zip" "alfred-${VERSION}-${PLATFORM}/"
        ARCHIVE_NAME="alfred-${VERSION}-${PLATFORM}.zip"
    else
        # Create tarball for Linux/Mac
        tar -czf "alfred-${VERSION}-${PLATFORM}.tar.gz" "alfred-${VERSION}-${PLATFORM}/"
        ARCHIVE_NAME="alfred-${VERSION}-${PLATFORM}.tar.gz"
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
echo "Packaging complete! Archives are in: ${RELEASE_DIR}"
echo "Version: ${VERSION}"
