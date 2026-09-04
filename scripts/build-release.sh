#!/bin/bash
set -e

# Alfred Build Script
# Automatically builds binaries for all supported platforms
# Version is calculated as YYYYMMDD based on current date

VERSION=$(date +%Y%m%d)
BUILD_DIR="target/release-builds"
BINARY_NAME="alfred"

echo "Building Alfred v${VERSION}"
echo "========================"

# Clean previous builds
rm -rf ${BUILD_DIR}
mkdir -p ${BUILD_DIR}

# Get current directory
SCRIPT_DIR="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Build for each target
TARGETS=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-msvc"
)

for TARGET in "${TARGETS[@]}"; do
    echo ""
    echo "Building for ${TARGET}..."

    # Create target directory
    TARGET_DIR="${BUILD_DIR}/${TARGET}"
    mkdir -p "${TARGET_DIR}"

    # Build release binary
    cargo build --release --target "${TARGET}" 2>&1 | tail -5 || {
        echo "Warning: Failed to build for ${TARGET}"
        continue
    }

    # Copy binary with version suffix
    if [ "${TARGET}" = "x86_64-pc-windows-msvc" ]; then
        BINARY_EXT=".exe"
    else
        BINARY_EXT=""
    fi

    BINARY_PATH="target/release/${BINARY_NAME}${BINARY_EXT}"
    if [ -f "${BINARY_PATH}" ]; then
        cp "${BINARY_PATH}" "${TARGET_DIR}/${BINARY_NAME}-v${VERSION}${BINARY_EXT}"
        echo "  Built: ${TARGET_DIR}/${BINARY_NAME}-v${VERSION}${BINARY_EXT}"
    else
        echo "  Warning: Binary not found at ${BINARY_PATH}"
    fi
done

echo ""
echo "========================"
echo "Build complete!"
echo "Version: ${VERSION}"
echo "Binaries are in: ${BUILD_DIR}"
