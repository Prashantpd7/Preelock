#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUILD_DIR="${SCRIPT_DIR}/build"
BINARIES_DIR="${SCRIPT_DIR}/../binaries"

echo "=== Building Preelock Privileged Helper ==="
echo "Directory: ${SCRIPT_DIR}"

mkdir -p "${BUILD_DIR}"
mkdir -p "${BINARIES_DIR}"
cd "${SCRIPT_DIR}"

# Build with Swift Package Manager
swift build -c release

# Resolve compiled binary path
BIN_PATH="$(swift build -c release --show-bin-path)/preelock-helper"

if [ ! -f "${BIN_PATH}" ]; then
    echo "ERROR: Compiled binary not found at ${BIN_PATH}"
    exit 1
fi

cp -f "${BIN_PATH}" "${BUILD_DIR}/preelock-helper"
cp -f "${BIN_PATH}" "${BINARIES_DIR}/preelock-helper"
chmod +x "${BUILD_DIR}/preelock-helper"
chmod +x "${BINARIES_DIR}/preelock-helper"

echo "Build succeeded: ${BUILD_DIR}/preelock-helper"
