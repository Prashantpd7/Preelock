#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT_DIR}/build/preelock-dns-test"

if [ ! -f "${BIN}" ]; then
    echo "Binary not found. Building first..."
    "${SCRIPT_DIR}/build_cli.sh"
fi

echo "=== Running Preelock DNS Proof of Concept Automated Verification ==="

# Start DNS engine in background
"${BIN}" > /tmp/preelock_dns_test.log 2>&1 &
SERVER_PID=$!
trap "kill -9 ${SERVER_PID} 2>/dev/null || true; rm -f /tmp/preelock_dns_test.log" EXIT

# Wait for server to bind port 5354
sleep 0.8

echo "--- Test 1: Query Blocked Root Domain (instagram.com) ---"
RES_INSTA=$(dig @127.0.0.1 -p 5354 instagram.com +noall +comments)
echo "${RES_INSTA}"
if echo "${RES_INSTA}" | grep -q "status: NXDOMAIN"; then
    echo ">>> PASS: instagram.com correctly returned NXDOMAIN"
else
    echo ">>> FAIL: instagram.com did not return NXDOMAIN"
    exit 1
fi

echo ""
echo "--- Test 2: Query Blocked Subdomain (graph.instagram.com) ---"
RES_SUB=$(dig @127.0.0.1 -p 5354 graph.instagram.com +noall +comments)
echo "${RES_SUB}"
if echo "${RES_SUB}" | grep -q "status: NXDOMAIN"; then
    echo ">>> PASS: graph.instagram.com correctly returned NXDOMAIN (wildcard match)"
else
    echo ">>> FAIL: graph.instagram.com did not return NXDOMAIN"
    exit 1
fi

echo ""
echo "--- Test 3: Query Blocked Domain (reddit.com) ---"
RES_REDDIT=$(dig @127.0.0.1 -p 5354 reddit.com +noall +comments)
echo "${RES_REDDIT}"
if echo "${RES_REDDIT}" | grep -q "status: NXDOMAIN"; then
    echo ">>> PASS: reddit.com correctly returned NXDOMAIN"
else
    echo ">>> FAIL: reddit.com did not return NXDOMAIN"
    exit 1
fi

echo ""
echo "--- Test 4: Query Allowed Domain (wikipedia.org) ---"
RES_WIKI=$(dig @127.0.0.1 -p 5354 wikipedia.org +short)
echo "Resolved IPs: ${RES_WIKI}"
if [ -n "${RES_WIKI}" ]; then
    echo ">>> PASS: wikipedia.org resolved successfully via upstream (1.1.1.1)"
else
    echo ">>> FAIL: wikipedia.org failed to resolve"
    exit 1
fi

echo ""
echo "--- Test 5: Query Allowed Domain (apple.com) ---"
RES_APPLE=$(dig @127.0.0.1 -p 5354 apple.com +short)
echo "Resolved IPs: ${RES_APPLE}"
if [ -n "${RES_APPLE}" ]; then
    echo ">>> PASS: apple.com resolved successfully via upstream"
else
    echo ">>> FAIL: apple.com failed to resolve"
    exit 1
fi

echo ""
echo "=========================================================="
echo "ALL 5 AUTOMATED DNS PROXY UNIT TESTS PASSED SUCCESSFULLY!"
echo "=========================================================="
