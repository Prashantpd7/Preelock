#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SOCK="${ROOT_DIR}/build/preelock-dns-control.sock"

if [ ! -S "${SOCK}" ]; then
    echo "ERROR: Control socket not found at ${SOCK}. Is the DNS proxy running?"
    exit 1
fi

if [ $# -eq 0 ]; then
    CMD="status"
else
    CMD="$*"
fi

echo "${CMD}" | nc -U "${SOCK}"
