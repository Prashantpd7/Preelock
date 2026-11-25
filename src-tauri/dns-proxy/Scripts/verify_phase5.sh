#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

BIN="${ROOT_DIR}/build/preelock-dns-test"
CONTROL_CLI="${SCRIPT_DIR}/dns_control.sh"
LOG_FILE="/tmp/preelock_phase5_test.log"
PORT=5354
TEST_POLICY_FILE="/tmp/phase5_dynamic_policy.json"

echo "=========================================================="
echo "=== Preelock DNS Engine Dynamic Policy & Control Verify =="
echo "=== Phase 5: Dynamic Reload, Allowlist, Socket Control ==="
echo "=========================================================="

# Ensure clean build
"${SCRIPT_DIR}/build_cli.sh" > /dev/null

# Clean up any existing instances and sockets
pkill -9 -f "preelock-dns-test.*${PORT}" 2>/dev/null || true
rm -f "${LOG_FILE}" "${TEST_POLICY_FILE}" "${ROOT_DIR}/build/preelock-dns-control.sock"
sleep 0.5

cleanup() {
    pkill -9 -f "preelock-dns-test.*${PORT}" 2>/dev/null || true
    rm -f "${TEST_POLICY_FILE}" "${ROOT_DIR}/build/preelock-dns-control.sock"
}
trap cleanup EXIT

# 1. Start proxy with initial dynamic policy (blocking instagram.com)
cat << 'EOF' > "${TEST_POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 5 Initial Policy",
  "mode": "blocklist",
  "domains": ["instagram.com"]
}
EOF

"${BIN}" --port "${PORT}" --policy "${TEST_POLICY_FILE}" > "${LOG_FILE}" 2>&1 &
PROXY_PID=$!

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

# --- Test 1: Dynamic File Reload (No Process Restart) ---
echo "--- Test 1: Dynamic File Reload (File Edit Without Restart) ---"
STATUS_BEFORE=$(dig @127.0.0.1 -p "${PORT}" instagram.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
if [ "${STATUS_BEFORE}" != "NXDOMAIN" ]; then
    echo ">>> FAIL: instagram.com was not initially blocked (got ${STATUS_BEFORE})"
    exit 1
fi

# Modify file: remove instagram.com, add example-block-test.com
cat << 'EOF' > "${TEST_POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 5 Modified Policy",
  "mode": "blocklist",
  "domains": ["example-block-test.com"]
}
EOF

# Allow 1.5 seconds for file watcher to pick up change
sleep 1.5

STATUS_INSTA_AFTER=$(dig @127.0.0.1 -p "${PORT}" instagram.com +short)
STATUS_EXAMPLE_AFTER=$(dig @127.0.0.1 -p "${PORT}" example-block-test.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')

if [ -n "${STATUS_INSTA_AFTER}" ] && [ "${STATUS_EXAMPLE_AFTER}" == "NXDOMAIN" ]; then
    echo ">>> PASS: Policy modification auto-reloaded without restart. instagram.com resolved, example-block-test.com blocked."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Dynamic reload failed (instagram: ${STATUS_INSTA_AFTER}, example: ${STATUS_EXAMPLE_AFTER})"
    exit 1
fi
echo ""

# --- Test 2: Local Control Socket Interface (status, reload, invalid) ---
echo "--- Test 2: Local Control Socket Commands ---"
STATUS_OUT=$("${CONTROL_CLI}" status)
if echo "${STATUS_OUT}" | grep -q '"status" : "running"' && echo "${STATUS_OUT}" | grep -q '"policy_mode" : "blocklist"'; then
    echo ">>> PASS: 'status' command returned valid JSON telemetry."
else
    echo ">>> FAIL: 'status' command returned invalid response: ${STATUS_OUT}"
    exit 1
fi

RELOAD_OUT=$("${CONTROL_CLI}" reload)
if echo "${RELOAD_OUT}" | grep -q "OK: Reloaded active policy"; then
    echo ">>> PASS: 'reload' command acknowledged successfully."
else
    echo ">>> FAIL: 'reload' command failed: ${RELOAD_OUT}"
    exit 1
fi

INVALID_OUT=$("${CONTROL_CLI}" invalid_action_test 2>&1 || true)
if echo "${INVALID_OUT}" | grep -q "ERROR: Unknown command"; then
    echo ">>> PASS: Invalid command returned clear error without crashing proxy."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Invalid command did not return error: ${INVALID_OUT}"
    exit 1
fi
echo ""

# --- Test 3: Per-Session Policy Switching (set-policy) ---
echo "--- Test 3: Per-Session Policy Switching via Control Socket ---"
SWITCH_OUT=$("${CONTROL_CLI}" set-policy "${ROOT_DIR}/policies/focus.json")
if echo "${SWITCH_OUT}" | grep -q "OK: Switched active policy to 'Focus Session'"; then
    STATUS_TWITTER=$(dig @127.0.0.1 -p "${PORT}" twitter.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
    if [ "${STATUS_TWITTER}" == "NXDOMAIN" ]; then
        echo ">>> PASS: Switched to 'policies/focus.json' via socket; twitter.com immediately blocked."
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        echo ">>> FAIL: twitter.com was not blocked after switching to focus policy (got ${STATUS_TWITTER})"
        exit 1
    fi
else
    echo ">>> FAIL: set-policy command failed: ${SWITCH_OUT}"
    exit 1
fi
echo ""

# --- Test 4: Allowlist Mode ---
echo "--- Test 4: Allowlist Mode (Block Everything Except Allowlist) ---"
SWITCH_ALLOW=$("${CONTROL_CLI}" set-policy "${ROOT_DIR}/policies/allowlist_study.json")
if echo "${SWITCH_ALLOW}" | grep -q "OK: Switched active policy to 'Strict Study'"; then
    ALLOW_WIKI=$(dig @127.0.0.1 -p "${PORT}" wikipedia.org +short)
    BLOCK_INSTA=$(dig @127.0.0.1 -p "${PORT}" instagram.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
    BLOCK_APPLE=$(dig @127.0.0.1 -p "${PORT}" apple.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')

    if [ -n "${ALLOW_WIKI}" ] && [ "${BLOCK_INSTA}" == "NXDOMAIN" ] && [ "${BLOCK_APPLE}" == "NXDOMAIN" ]; then
        echo ">>> PASS: Allowlist strictly enforced: wikipedia.org allowed (${ALLOW_WIKI}), instagram.com & apple.com blocked."
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        echo ">>> FAIL: Allowlist mode violation (wiki: ${ALLOW_WIKI}, insta: ${BLOCK_INSTA}, apple: ${BLOCK_APPLE})"
        exit 1
    fi
else
    echo ">>> FAIL: Failed to switch to allowlist policy: ${SWITCH_ALLOW}"
    exit 1
fi
echo ""

# Switch back to standard blocklist
"${CONTROL_CLI}" set-policy "${ROOT_DIR}/policies/focus.json" > /dev/null

# --- Test 5: Malformed Policy Safety ---
echo "--- Test 5: Malformed Policy Safety (Corrupted JSON) ---"
cat << 'EOF' > "${TEST_POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 5 Safety Policy",
  "mode": "blocklist",
  "domains": ["twitter.com"]
}
EOF
"${CONTROL_CLI}" set-policy "${TEST_POLICY_FILE}" > /dev/null

# Corrupt the file with invalid JSON
echo "{ corrupted invalid json content " > "${TEST_POLICY_FILE}"
sleep 1.2
RELOAD_ERR=$("${CONTROL_CLI}" reload 2>&1 || true)

if kill -0 "${PROXY_PID}" 2>/dev/null && grep -q "\[WARN\] Failed to load policy" "${LOG_FILE}"; then
    STATUS_TWITTER_STILL_BLOCKED=$(dig @127.0.0.1 -p "${PORT}" twitter.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
    if [ "${STATUS_TWITTER_STILL_BLOCKED}" == "NXDOMAIN" ]; then
        echo ">>> PASS: Corrupted policy rejected, warning logged, old policy retained, proxy kept running."
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        echo ">>> FAIL: Proxy fell back to allow-all or unblocked domains upon corruption."
        exit 1
    fi
else
    echo ">>> FAIL: Proxy crashed or failed to log warning upon malformed policy."
    exit 1
fi
echo ""

# --- Test 6: Backward Compatibility (v1 Plain Array Blocklist) ---
echo "--- Test 6: Backward Compatibility (v1 [String] JSON Array) ---"
cat << 'EOF' > "${TEST_POLICY_FILE}"
["legacy-block-domain.com", "instagram.com"]
EOF
SWITCH_LEGACY=$("${CONTROL_CLI}" set-policy "${TEST_POLICY_FILE}")
if echo "${SWITCH_LEGACY}" | grep -q "OK: Switched active policy"; then
    STATUS_LEGACY=$(dig @127.0.0.1 -p "${PORT}" legacy-block-domain.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
    if [ "${STATUS_LEGACY}" == "NXDOMAIN" ]; then
        echo ">>> PASS: Plain string array v1 decoded seamlessly as Legacy Blocklist; domain blocked."
        PASS_COUNT=$((PASS_COUNT + 1))
    else
        echo ">>> FAIL: Legacy domain was not blocked (got ${STATUS_LEGACY})"
        exit 1
    fi
else
    echo ">>> FAIL: Failed to load legacy v1 policy file: ${SWITCH_LEGACY}"
    exit 1
fi
echo ""

# --- Test 7: Large-Blocklist Performance Benchmark (10,000 Rules) ---
echo "--- Test 7: Large Blocklist Performance Benchmark (10,000 Domains) ---"
SWITCH_10K=$("${CONTROL_CLI}" set-policy "${ROOT_DIR}/policies/large_test_10k.json")
if echo "${SWITCH_10K}" | grep -q "10000 domains"; then
    START_NS=$(python3 -c 'import time; print(time.time_ns())')
    dig @127.0.0.1 -p "${PORT}" blocked-05000.example.com > /dev/null
    END_NS=$(python3 -c 'import time; print(time.time_ns())')
    BLOCKED_MS=$(python3 -c "print(f'{(${END_NS} - ${START_NS}) / 1000000.0:.2f}')")

    START_NS=$(python3 -c 'import time; print(time.time_ns())')
    dig @127.0.0.1 -p "${PORT}" wikipedia.org > /dev/null
    END_NS=$(python3 -c 'import time; print(time.time_ns())')
    ALLOWED_MS=$(python3 -c "print(f'{(${END_NS} - ${START_NS}) / 1000000.0:.2f}')")

    echo ">>> PASS: 10,000 domains active. Blocked query latency: ${BLOCKED_MS} ms, Allowed query latency: ${ALLOWED_MS} ms."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Failed to load 10,000 rules policy: ${SWITCH_10K}"
    exit 1
fi
echo ""

# --- Test 8: Subdomain Matching Non-Regression ---
echo "--- Test 8: Subdomain Wildcard Matching Non-Regression ---"
STATUS_SUB=$(dig @127.0.0.1 -p "${PORT}" sub.api.blocked-09999.example.com +noall +comments | grep -o "status: [A-Z]*" | awk '{print $2}')
if [ "${STATUS_SUB}" == "NXDOMAIN" ]; then
    echo ">>> PASS: Multi-level subdomain (sub.api.blocked-09999.example.com) correctly matched parent and returned NXDOMAIN."
    PASS_COUNT=$((PASS_COUNT + 1))
else
    echo ">>> FAIL: Subdomain was not blocked (got ${STATUS_SUB})"
    exit 1
fi
echo ""

echo "=========================================================="
echo "ALL ${PASS_COUNT}/${TOTAL_TESTS} PHASE 5 DYNAMIC POLICY & CONTROL TESTS PASSED!"
echo "=========================================================="
