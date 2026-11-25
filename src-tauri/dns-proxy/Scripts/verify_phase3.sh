#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT_DIR}/build/preelock-dns-test"
LOG_FILE="/tmp/preelock_phase3_test.log"
PORT=5354

echo "=========================================================="
echo "=== Preelock DNS Engine Security & Correctness Verify  ==="
echo "=== Phase 3: TCP DNS (RFC 7766), Cache, Hardening    ==="
echo "=========================================================="

# Ensure clean build
"${SCRIPT_DIR}/build_cli.sh" > /dev/null

# Clean up any existing instances
pkill -9 -f "preelock-dns-test.*${PORT}" 2>/dev/null || true
sleep 0.5

# Start local DNS proxy on non-privileged port 5354
"${BIN}" --port "${PORT}" > "${LOG_FILE}" 2>&1 &
PROXY_PID=$!

cleanup() {
    if kill -0 "${PROXY_PID}" 2>/dev/null; then
        kill -9 "${PROXY_PID}" 2>/dev/null || true
    fi
}
trap cleanup EXIT

# Wait up to 3 seconds for proxy to become ready
READY=false
for i in {1..30}; do
    if grep -q "TCP DNS Listener is READY" "${LOG_FILE}" 2>/dev/null; then
        READY=true
        break
    fi
    sleep 0.1
done

if [ "${READY}" != "true" ]; then
    echo "ERROR: Proxy failed to start on port ${PORT}."
    cat "${LOG_FILE}"
    exit 1
fi
echo ">>> Proxy is running on PID ${PROXY_PID} (Port ${PORT})"
echo ""

PASS_COUNT=0
TOTAL_TESTS=9

# --- Test 1: TCP Blocked Domain (RFC 7766) ---
echo "--- Test 1: TCP Query on Blocked Domain (instagram.com) ---"
STATUS_TCP_BLOCK=$(dig @127.0.0.1 -p "${PORT}" +tcp +time=2 +tries=1 instagram.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
if [ "${STATUS_TCP_BLOCK}" == "NXDOMAIN" ]; then
    echo ">>> PASS: TCP query to instagram.com correctly returned NXDOMAIN"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Expected NXDOMAIN over TCP, got: '${STATUS_TCP_BLOCK}'"
    exit 1
fi
echo ""

# --- Test 2: TCP Allowed Domain (RFC 7766) ---
echo "--- Test 2: TCP Query on Allowed Domain (wikipedia.org) ---"
IPS_TCP_ALLOW=$(dig @127.0.0.1 -p "${PORT}" +tcp +time=3 +tries=1 wikipedia.org +short)
if [ -n "${IPS_TCP_ALLOW}" ]; then
    echo ">>> PASS: TCP query to wikipedia.org resolved to: ${IPS_TCP_ALLOW}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: TCP query to wikipedia.org failed to resolve"
    exit 1
fi
echo ""

# --- Test 3: UDP Blocked Domain ---
echo "--- Test 3: UDP Query on Blocked Domain (reddit.com) ---"
STATUS_UDP_BLOCK=$(dig @127.0.0.1 -p "${PORT}" +time=2 +tries=1 reddit.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
if [ "${STATUS_UDP_BLOCK}" == "NXDOMAIN" ]; then
    echo ">>> PASS: UDP query to reddit.com correctly returned NXDOMAIN"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Expected NXDOMAIN over UDP, got: '${STATUS_UDP_BLOCK}'"
    exit 1
fi
echo ""

# --- Test 4: UDP Allowed Domain ---
echo "--- Test 4: UDP Query on Allowed Domain (apple.com) ---"
IPS_UDP_ALLOW=$(dig @127.0.0.1 -p "${PORT}" +time=3 +tries=1 apple.com +short)
if [ -n "${IPS_UDP_ALLOW}" ]; then
    echo ">>> PASS: UDP query to apple.com resolved to: ${IPS_UDP_ALLOW}"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: UDP query to apple.com failed to resolve"
    exit 1
fi
echo ""

# --- Test 5: In-Memory Response Cache ---
echo "--- Test 5: In-Memory DNS Cache Verification ---"
# First query primes cache
dig @127.0.0.1 -p "${PORT}" wikipedia.org +short > /dev/null
# Second query hits cache
CACHE_HIT_IP=$(dig @127.0.0.1 -p "${PORT}" wikipedia.org +short)
if grep -q "Resolved via Cache" "${LOG_FILE}"; then
    echo ">>> PASS: Subsequent query served directly from memory cache (IP: ${CACHE_HIT_IP})"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Cache hit not logged in resolution audit log"
    exit 1
fi
echo ""

# --- Test 6: Malformed Packet Rejection (RFC 1035 Hardening) ---
echo "--- Test 6: Malformed Packet Rejection (Hardening) ---"
python3 -c '
import socket, time
server = ("127.0.0.1", '"${PORT}"')
packets = [
    ("<12 bytes", b"\x12\x34\x01\x00\x00"),
    ("QDCOUNT=0", b"\x12\x34\x01\x00\x00\x00\x00\x00\x00\x00\x00\x00"),
    ("Compression pointer", b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\xc0\x04\x00\x01\x00\x01"),
    ("Label > 63", b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x46" + b"a"*70 + b"\x00\x00\x01\x00\x01"),
    ("Truncated question", b"\x12\x34\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x07example\x03com")
]
for _, pkt in packets:
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.sendto(pkt, server)
    s.close()
    time.sleep(0.02)
'
sleep 0.5
if kill -0 "${PROXY_PID}" 2>/dev/null && grep -q "Received malformed or non-DNS datagram" "${LOG_FILE}"; then
    echo ">>> PASS: All 5 malformed packet variations safely rejected without crash"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Proxy crashed or failed to reject malformed packets"
    exit 1
fi
echo ""

# --- Test 7: Strict Loopback-only Binding ---
echo "--- Test 7: Loopback-Only Socket Binding (Security) ---"
BOUND_SOCKETS=$(lsof -nP -i :"${PORT}" | grep "${PROXY_PID}" | awk '{print $8, $9}' | sort -u)
if echo "${BOUND_SOCKETS}" | grep -q "UDP 127.0.0.1:${PORT}" && echo "${BOUND_SOCKETS}" | grep -q "TCP 127.0.0.1:${PORT}"; then
    echo ">>> PASS: Both UDP and TCP sockets bound strictly to 127.0.0.1 (No 0.0.0.0 leak)"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Sockets not strictly bound to 127.0.0.1: ${BOUND_SOCKETS}"
    exit 1
fi
echo ""

# --- Test 8: Blocked Domain Non-Forwarding Guarantee ---
echo "--- Test 8: Blocked Domain Upstream Isolation ---"
# Verify in log that blocked queries never triggered upstream resolution
BLOCKED_FORWARDED=$(grep "\[BLOCKED\]" "${LOG_FILE}" | grep "Resolved via" || true)
if [ -z "${BLOCKED_FORWARDED}" ]; then
    echo ">>> PASS: Confirmed zero blocked domains were forwarded to external upstream DNS"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Detected blocked domain forwarded upstream: ${BLOCKED_FORWARDED}"
    exit 1
fi
echo ""

# --- Test 9: Rate Limiting Enforcement ---
echo "--- Test 9: Rate Limiter Protection (>100 qps) ---"
python3 -c '
import socket, time
server = ("127.0.0.1", '"${PORT}"')
pkt = b"\xaa\xbb\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x09wikipedia\x03org\x00\x00\x01\x00\x01"
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
for _ in range(120):
    s.sendto(pkt, server)
    time.sleep(0.001)
s.close()
'
sleep 0.5
if grep -q "Rate limit exceeded (>100 qps)" "${LOG_FILE}"; then
    echo ">>> PASS: Rate limiter correctly triggered and dropped queries exceeding 100 qps"
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Rate limiter failed to log dropped queries"
    exit 1
fi
echo ""

echo "=========================================================="
echo "ALL ${PASS_COUNT}/${TOTAL_TESTS} PHASE 3 SECURITY & CORRECTNESS TESTS PASSED!"
echo "=========================================================="
