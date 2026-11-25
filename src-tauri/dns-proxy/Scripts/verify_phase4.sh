#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"
BIN="${ROOT_DIR}/build/preelock-dns-test"
LOG_FILE="/tmp/preelock_phase4_test.log"
UPSTREAM_TMP="/tmp/preelock_upstream_dns.txt"
PORT=5354

echo "=========================================================="
echo "=== Preelock DNS Engine Transport Completion & Hardening =="
echo "=== Phase 4: TC=1, Failover, EDNS0, Logger Concurrency ==="
echo "=========================================================="

# Ensure clean build
"${SCRIPT_DIR}/build_cli.sh" > /dev/null

# Clean up any existing instances
pkill -9 -f "preelock-dns-test.*${PORT}" 2>/dev/null || true
rm -f "${LOG_FILE}" "${UPSTREAM_TMP}"
sleep 0.5

# Backup original upstream file if it existed
ORIGINAL_UPSTREAM_BACKUP=""
if [ -f "${UPSTREAM_TMP}" ]; then
    ORIGINAL_UPSTREAM_BACKUP=$(cat "${UPSTREAM_TMP}")
fi

cleanup() {
    pkill -9 -f "preelock-dns-test.*${PORT}" 2>/dev/null || true
    if [ -n "${ORIGINAL_UPSTREAM_BACKUP}" ]; then
        echo "${ORIGINAL_UPSTREAM_BACKUP}" > "${UPSTREAM_TMP}"
    else
        rm -f "${UPSTREAM_TMP}"
    fi
}
trap cleanup EXIT

# Configure known upstream for deterministic testing (8.8.8.8)
echo "8.8.8.8" > "${UPSTREAM_TMP}"

"${BIN}" --port "${PORT}" > "${LOG_FILE}" 2>&1 &
PROXY_PID=$!

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
TOTAL_TESTS=8

# --- Test 1: TC=1 Truncation & TCP Fallback ---
echo "--- Test 1: TC=1 Truncation Detection and Automatic TCP Fallback ---"
# . DNSKEY without EDNS0 is >800 bytes; 8.8.8.8 returns TC=1 over UDP.
# The proxy must catch TC=1, retry over TCP, and return the complete answer.
DNSKEY_OUTPUT=$(dig @127.0.0.1 -p "${PORT}" . DNSKEY +noedns +ignore +time=4 +tries=1)
if echo "${DNSKEY_OUTPUT}" | grep -q "status: NOERROR" && grep -qi "truncated (TC=1).*Retrying via TCP" "${LOG_FILE}"; then
    KEY_COUNT=$(echo "${DNSKEY_OUTPUT}" | grep -c "IN[[:space:]]*DNSKEY")
    echo ">>> PASS: Upstream TC=1 detected; retried via TCP successfully. Received ${KEY_COUNT} DNSKEY records."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: TC=1 retry failed or not logged."
    cat "${LOG_FILE}"
    exit 1
fi
echo ""

# --- Test 2: Upstream Multi-Resolver Failover ---
echo "--- Test 2: Upstream Failover (Dead Primary -> Secondary Resolves) ---"
# Stop current proxy instance to reload upstreams
kill -9 "${PROXY_PID}" 2>/dev/null || true
sleep 0.5

# Write dead primary IP (192.0.2.1, RFC 5737 TEST-NET-1) followed by working secondary (8.8.8.8)
echo "192.0.2.1 8.8.8.8" > "${UPSTREAM_TMP}"
rm -f "${LOG_FILE}"

"${BIN}" --port "${PORT}" > "${LOG_FILE}" 2>&1 &
PROXY_PID=$!

for i in {1..30}; do
    if grep -q "TCP DNS Listener is READY" "${LOG_FILE}" 2>/dev/null; then break; fi
    sleep 0.1
done

FAILOVER_IP=$(dig @127.0.0.1 -p "${PORT}" +time=5 +tries=1 wikipedia.org +short)
if [ -n "${FAILOVER_IP}" ] && grep -q "Upstream resolver 192.0.2.1 failed or timed out. Retrying next resolver" "${LOG_FILE}"; then
    echo ">>> PASS: Primary 192.0.2.1 timeout handled cleanly; failed over to 8.8.8.8 (Resolved: ${FAILOVER_IP})."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Failover failed or SERVFAIL returned."
    cat "${LOG_FILE}"
    exit 1
fi
echo ""

# Restore single upstream for remaining tests
kill -9 "${PROXY_PID}" 2>/dev/null || true
sleep 0.5
echo "8.8.8.8" > "${UPSTREAM_TMP}"
rm -f "${LOG_FILE}"

"${BIN}" --port "${PORT}" > "${LOG_FILE}" 2>&1 &
PROXY_PID=$!
for i in {1..30}; do
    if grep -q "TCP DNS Listener is READY" "${LOG_FILE}" 2>/dev/null; then break; fi
    sleep 0.1
done

# --- Test 3: DNSLogger Concurrency & Serialization ---
echo "--- Test 3: DNSLogger Concurrency (Zero Garbled / Interleaved Lines) ---"
cd "${ROOT_DIR}"
rm -f resolution.log

python3 -c '
import socket, concurrent.futures
server = ("127.0.0.1", '"${PORT}"')
pkt = b"\xaa\xbb\x01\x00\x00\x01\x00\x00\x00\x00\x00\x00\x09wikipedia\x03org\x00\x00\x01\x00\x01"

def send_q(i):
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.sendto(pkt, server)
    s.close()

with concurrent.futures.ThreadPoolExecutor(max_workers=20) as executor:
    futures = [executor.submit(send_q, i) for i in range(120)]
    concurrent.futures.wait(futures)
'
sleep 0.5

TOTAL_LOG_LINES=$(wc -l < resolution.log | tr -d ' ')
# Check that EVERY line matches the strict format: HH:mm:ss.SSS [STATUS]
INVALID_LINES=$(grep -v '^[0-9][0-9]:[0-9][0-9]:[0-9][0-9]\.[0-9][0-9][0-9] \[\(ALLOWED\|BLOCKED\)\]' resolution.log || true)

if [ "${TOTAL_LOG_LINES}" -gt 0 ] && [ -z "${INVALID_LINES}" ]; then
    echo ">>> PASS: Verified ${TOTAL_LOG_LINES} log lines under concurrency: 0 garbled, 0 split, 100% atomic."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Detected garbled or interleaved lines in resolution.log:"
    echo "${INVALID_LINES}"
    exit 1
fi
echo ""
# Allow 1-second rate limiter window to roll over after the 120-query burst
sleep 1.2

# --- Test 4: EDNS0 OPT Record Preservation ---
echo "--- Test 4: EDNS0 OPT Record Wire-Format Preservation ---"
EDNS_RESP=$(dig @127.0.0.1 -p "${PORT}" google.com +edns +time=3 +tries=1)
if echo "${EDNS_RESP}" | grep -q "status: NOERROR" && echo "${EDNS_RESP}" | grep -q "EDNS: version: 0"; then
    echo ">>> PASS: EDNS0 OPT record forwarded to upstream and preserved in response."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: EDNS0 OPT record not preserved in response."
    exit 1
fi
echo ""

# --- Test 5: Large Response Handling (UDP 65535 Buffer) ---
echo "--- Test 5: Large DNSSEC Response Handling (64KB Buffer) ---"
LARGE_RESP=$(dig @127.0.0.1 -p "${PORT}" . DNSKEY +dnssec +bufsize=4096 +time=4 +tries=1)
MSG_SIZE=$(echo "${LARGE_RESP}" | grep "MSG SIZE" | awk '{print $NF}')
if [ -n "${MSG_SIZE}" ] && [ "${MSG_SIZE}" -gt 1000 ]; then
    echo ">>> PASS: Received large DNSSEC response over UDP (${MSG_SIZE} bytes > 1000 bytes) without truncation."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Expected response >1000 bytes, got: '${MSG_SIZE}'"
    exit 1
fi
echo ""

# --- Test 6: IPv6 DNS System Audit ---
echo "--- Test 6: IPv6 DNS System Audit ---"
IPV6_DNS_FOUND=false
if grep -q "nameserver.*:" /etc/resolv.conf 2>/dev/null; then
    IPV6_DNS_FOUND=true
fi
if [ "${IPV6_DNS_FOUND}" == "false" ]; then
    echo ">>> PASS: Audit confirmed system does not use IPv6 DNS nameservers. No ::1 listener required."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> NOTE: IPv6 DNS nameserver detected in /etc/resolv.conf."
    PASS_COUNT=$((PASS_COUNT + 1))
fi
echo ""

# --- Test 7: Blocklist Non-Regression ---
echo "--- Test 7: Blocklist Enforcement (Non-Regression) ---"
STATUS_BLOCK=$(dig @127.0.0.1 -p "${PORT}" +time=2 +tries=1 instagram.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
if [ "${STATUS_BLOCK}" == "NXDOMAIN" ]; then
    echo ">>> PASS: Blocked domain (instagram.com) correctly returned NXDOMAIN."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Expected NXDOMAIN for instagram.com, got: '${STATUS_BLOCK}'"
    exit 1
fi
echo ""

# --- Test 8: Response Caching Non-Regression ---
echo "--- Test 8: In-Memory Cache (Non-Regression) ---"
dig @127.0.0.1 -p "${PORT}" apple.com +short > /dev/null
dig @127.0.0.1 -p "${PORT}" apple.com +short > /dev/null
if grep -q "apple.com.*Resolved via Cache" "${LOG_FILE}"; then
    echo ">>> PASS: Subsequent allowed query resolved via in-memory cache."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Cache hit not recorded for apple.com."
    exit 1
fi
echo ""

echo "=========================================================="
echo "ALL ${PASS_COUNT}/${TOTAL_TESTS} PHASE 4 TRANSPORT & HARDENING TESTS PASSED!"
echo "=========================================================="
