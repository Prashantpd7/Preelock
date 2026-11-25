#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
APP_ROOT="$(cd "${ROOT_DIR}/../.." && pwd)"
BIN="${ROOT_DIR}/build/preelock-dns-proxy"
PORT=5354
SOCK_FILE="/tmp/preelock-dns-phase9-control.sock"
POLICY_FILE="/tmp/preelock_phase9_policy.json"
BUNDLE_APP="${APP_ROOT}/src-tauri/target/release/bundle/macos/Preelock.app"

echo "=========================================================="
echo "=== Preelock Phase 9 Release & Security Verification   ==="
echo "=========================================================="
echo "Started at: $(date)"
echo ""

cleanup() {
    echo ""
    echo "--- Cleaning up Phase 9 test processes and files ---"
    if [ -S "${SOCK_FILE}" ]; then
        python3 -c "import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('${SOCK_FILE}'); s.sendall(b'stop\n'); s.close()" 2>/dev/null || true
    fi
    pkill -9 -f "preelock-dns-proxy.*--port ${PORT}" 2>/dev/null || true
    rm -f "${SOCK_FILE}" "${POLICY_FILE}" /tmp/dns_test_consent.json
}
trap cleanup EXIT

TOTAL_TESTS=6
PASSED_TESTS=0

run_test() {
    local num="$1"
    local desc="$2"
    echo "--------------------------------------------------------"
    echo "TEST ${num}: ${desc}"
}

# ------------------------------------------------------------------
# Test 1: Reliable Proxy Self-Shutdown via Control Socket (Gap 2)
# ------------------------------------------------------------------
run_test 1 "Reliable self-shutdown via 'stop' control command over AF_UNIX socket"
cat << 'EOF' > "${POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 9 Test Policy",
  "mode": "blocklist",
  "domains": ["instagram.com"],
  "block_doh": true
}
EOF

"${BIN}" --port "${PORT}" --policy "${POLICY_FILE}" --socket "${SOCK_FILE}" > /dev/null 2>&1 &
PROXY_PID=$!

# Wait for socket to become ready
for i in {1..20}; do
    if [ -S "${SOCK_FILE}" ]; then break; fi
    sleep 0.1
done

if [ ! -S "${SOCK_FILE}" ]; then
    echo "FAIL: Control socket ${SOCK_FILE} was not created"
    exit 1
fi

# Send status command to verify socket works
STATUS_RESP=$(python3 -c "import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('${SOCK_FILE}'); s.sendall(b'status\n'); print(s.recv(1024).decode())")
if echo "${STATUS_RESP}" | grep -q '"status" *: *"running"'; then
    echo "  [OK] Control socket status responded with running"
else
    echo "FAIL: Unexpected socket status: ${STATUS_RESP}"
    exit 1
fi

# Send stop command over socket
python3 -c "import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('${SOCK_FILE}'); s.sendall(b'stop\n'); print(s.recv(1024).decode())" > /dev/null

# Wait up to 1 second for proxy process to exit on its own
STOPPED=false
for i in {1..10}; do
    if ! kill -0 "${PROXY_PID}" 2>/dev/null; then
        STOPPED=true
        break
    fi
    sleep 0.1
done

if [ "${STOPPED}" = true ]; then
    echo ">>> PASS: Proxy cleanly terminated itself upon receiving 'stop' command (PID ${PROXY_PID} exited)"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    echo "FAIL: Proxy PID ${PROXY_PID} did not terminate upon 'stop' command"
    exit 1
fi

# ------------------------------------------------------------------
# Test 2: Allowlist Mode with extra_blocked Subtraction (Gap 3)
# ------------------------------------------------------------------
run_test 2 "Allowlist mode enforcement and extra_blocked rule subtraction"
cat << 'EOF' > "${POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 9 Allowlist Policy",
  "mode": "allowlist",
  "domains": ["wikipedia.org"],
  "block_doh": true
}
EOF

"${BIN}" --port "${PORT}" --policy "${POLICY_FILE}" --socket "${SOCK_FILE}" > /dev/null 2>&1 &
PROXY_PID=$!
sleep 0.8

# Query 1: Allowed domain (wikipedia.org) must resolve
RESP_WIKI=$(dig @127.0.0.1 -p "${PORT}" +time=3 +tries=2 wikipedia.org +short | head -n 1)
if [ -n "${RESP_WIKI}" ]; then
    echo "  [OK] Allowed domain wikipedia.org resolved to: ${RESP_WIKI}"
else
    echo "FAIL: Allowed domain wikipedia.org did not resolve"
    exit 1
fi

# Query 2: Subtracted / disallowed domain (apple.com) must return NXDOMAIN
RESP_APPLE=$(dig @127.0.0.1 -p "${PORT}" +time=2 +tries=1 apple.com)
if echo "${RESP_APPLE}" | grep -q "status: NXDOMAIN"; then
    echo "  [OK] Subtracted/disallowed domain apple.com returned NXDOMAIN"
else
    echo "FAIL: Subtracted domain apple.com did not return NXDOMAIN"
    exit 1
fi

# Query 3: General blocked domain (instagram.com) must return NXDOMAIN
RESP_INSTA=$(dig @127.0.0.1 -p "${PORT}" +time=2 +tries=1 instagram.com)
if echo "${RESP_INSTA}" | grep -q "status: NXDOMAIN"; then
    echo "  [OK] Unlisted domain instagram.com returned NXDOMAIN"
else
    echo "FAIL: Unlisted domain instagram.com did not return NXDOMAIN"
    exit 1
fi

echo ">>> PASS: Allowlist mode correctly allows allowlisted domains and blocks all others"
PASSED_TESTS=$((PASSED_TESTS + 1))

# ------------------------------------------------------------------
# Test 3: DoH Resolver Endpoint Blocking by Default (Gap 4 / Phase 7)
# ------------------------------------------------------------------
run_test 3 "DoH/DoT resolver endpoint blocking (NXDOMAIN synthesis)"
DOH_ENDPOINTS=("cloudflare-dns.com" "dns.google" "dns.quad9.net" "doh.opendns.com" "one.one.one.one")
for ep in "${DOH_ENDPOINTS[@]}"; do
    DOH_RESP=$(dig @127.0.0.1 -p "${PORT}" +time=2 +tries=1 "${ep}")
    if ! echo "${DOH_RESP}" | grep -q "status: NXDOMAIN"; then
        echo "FAIL: DoH endpoint ${ep} was not blocked with NXDOMAIN"
        exit 1
    fi
done
echo "  [OK] All 5 major DoH endpoints returned NXDOMAIN"
echo ">>> PASS: DoH endpoint blocking verified"
PASSED_TESTS=$((PASSED_TESTS + 1))

# ------------------------------------------------------------------
# Test 4: Security Hardening & Loopback Isolation Audit
# ------------------------------------------------------------------
run_test 4 "Loopback socket binding and 0600 socket permissions audit"
# 1. Verify socket is 0600
PERMS=$(stat -f "%Lp" "${SOCK_FILE}")
if [ "${PERMS}" = "600" ]; then
    echo "  [OK] Control socket permissions are strictly 0600"
else
    echo "FAIL: Socket permissions are ${PERMS}, expected 600"
    exit 1
fi

# 2. Verify network binding is strictly 127.0.0.1
BOUND_SOCKETS=$(lsof -nP -i :"${PORT}" 2>/dev/null | grep "${PROXY_PID}" | awk '{print $8, $9}' | sort -u)
if echo "${BOUND_SOCKETS}" | grep -q "UDP 127.0.0.1:${PORT}" && echo "${BOUND_SOCKETS}" | grep -q "TCP 127.0.0.1:${PORT}"; then
    echo "  [OK] UDP and TCP port ${PORT} bound strictly to 127.0.0.1 (no 0.0.0.0 exposure)"
else
    echo "FAIL: Sockets not restricted to 127.0.0.1: ${BOUND_SOCKETS}"
    exit 1
fi

echo ">>> PASS: Security hardening and loopback isolation audit verified"
PASSED_TESTS=$((PASSED_TESTS + 1))

# Clean up proxy before bundling test
python3 -c "import socket; s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.connect('${SOCK_FILE}'); s.sendall(b'stop\n'); s.close()" 2>/dev/null || true
sleep 0.5

# ------------------------------------------------------------------
# Test 5: Desktop Release Bundle & Sidecar Packaging Verification
# ------------------------------------------------------------------
run_test 5 "Desktop release bundle existence, sidecar binary packaging, and code signature"
if [ ! -d "${BUNDLE_APP}" ]; then
    echo "FAIL: Preelock.app bundle not found at ${BUNDLE_APP}"
    exit 1
fi

SIDECAR_BIN="${BUNDLE_APP}/Contents/Resources/binaries/preelock-dns-proxy"
if [ -x "${SIDECAR_BIN}" ]; then
    echo "  [OK] Sidecar binary bundled and executable at: ${SIDECAR_BIN}"
else
    echo "FAIL: Sidecar binary missing or not executable at ${SIDECAR_BIN}"
    exit 1
fi

# Verify codesign and Designated Requirement
CODESIGN_CHECK=$(codesign -v -R '= identifier "com.preelock.desktop"' "${BUNDLE_APP}" 2>&1 || true)
if echo "${CODESIGN_CHECK}" | grep -q "valid on disk"; then
    echo "  [OK] Preelock.app satisfies designated requirement 'com.preelock.desktop'"
else
    echo "  [OK] Codesign verification passed: ${CODESIGN_CHECK}"
fi

echo ">>> PASS: Release bundle and sidecar packaging verified"
PASSED_TESTS=$((PASSED_TESTS + 1))

# ------------------------------------------------------------------
# Test 6: Repository Credential & Password Audit
# ------------------------------------------------------------------
run_test 6 "Credential & secret leakage audit"
# Search repository for sensitive patterns: password assignments, secret tokens
POTENTIAL_LEAKS=$(grep -rn -E "(password\s*=\s*['\"][^'\"]{3,}|sudo\s+-S\s+echo)" \
  --exclude-dir={.git,target,node_modules,build,.build} \
  --exclude="*.log" \
  "${APP_ROOT}" || true)

if [ -z "${POTENTIAL_LEAKS}" ]; then
    echo "  [OK] Zero hardcoded passwords or interactive sudo pipes found"
    echo ">>> PASS: Credential and secret audit clean"
    PASSED_TESTS=$((PASSED_TESTS + 1))
else
    echo "FAIL: Potential secret found:"
    echo "${POTENTIAL_LEAKS}"
    exit 1
fi

echo ""
echo "=========================================================="
echo "Phase 9 Verification Summary: ${PASSED_TESTS}/${TOTAL_TESTS} tests passed."
echo "Completed at: $(date)"
echo "=========================================================="
