#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"

echo "=== Building Preelock DNS Proxy Sidecar ==="
echo "Directory: ${SCRIPT_DIR}"

mkdir -p "${BUILD_DIR}"
cd "${SCRIPT_DIR}"

# Build with Swift Package Manager (no Xcode / xcodebuild required)
swift build -c release

# Resolve compiled binary path from SPM
BIN_PATH="$(swift build -c release --show-bin-path)/preelock-dns-proxy"

if [ ! -f "${BIN_PATH}" ]; then
    echo "ERROR: Compiled binary not found at ${BIN_PATH}"
    exit 1
fi

cp -f "${BIN_PATH}" "${BUILD_DIR}/preelock-dns-proxy"
chmod +x "${BUILD_DIR}/preelock-dns-proxy"

echo "Build succeeded: ${BUILD_DIR}/preelock-dns-proxy"
