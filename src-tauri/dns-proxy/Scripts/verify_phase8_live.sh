#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT_DIR}/build/preelock-dns-proxy"
POLICY_FILE="/tmp/preelock_active_policy.json"
SOCK_FILE="/tmp/preelock-dns-control.sock"
ORIGINAL_DNS_FILE="/tmp/preelock_original_dns.txt"

echo "=========================================================="
echo "=== Preelock Phase 8 Live Integration & Browser Verification ==="
echo "=========================================================="
echo "Started at: $(date)"
echo ""

# 1. Detect active network service
DEFAULT_IF=$(/sbin/route -n get default 2>/dev/null | awk '/interface:/{print $2}')
if [ -z "${DEFAULT_IF}" ]; then DEFAULT_IF="en0"; fi

SERVICE=""
while IFS= read -r line; do
    if [[ "${line}" =~ "Hardware Port: " ]]; then
        PORT_NAME="${line#Hardware Port: }"
    fi
    if [[ "${line}" =~ "Device: ${DEFAULT_IF}" ]]; then
        SERVICE="${PORT_NAME}"
        break
    fi
done < <(/usr/sbin/networksetup -listallhardwareports 2>/dev/null)
if [ -z "${SERVICE}" ]; then SERVICE="Wi-Fi"; fi

echo "Active Network Service: ${SERVICE} (${DEFAULT_IF})"

# Cleanup on exit
cleanup() {
    echo ""
    echo "--- Restoring System Network State ---"
    sudo /usr/sbin/networksetup -setdnsservers "${SERVICE}" "Empty" 2>/dev/null || true
    sudo /usr/bin/dscacheutil -flushcache 2>/dev/null || true
    sudo /usr/bin/killall -HUP mDNSResponder 2>/dev/null || true
    sudo pkill -9 -f "preelock-dns-proxy.*--port 53" 2>/dev/null || true
    rm -f "${POLICY_FILE}" "${SOCK_FILE}" "${ORIGINAL_DNS_FILE}"
    echo "Network restored to original DHCP configuration."
}
trap cleanup EXIT

# 2. Write test policy (blocklist: instagram.com, block_doh: true)
cat << 'EOF' > "${POLICY_FILE}"
{
  "version": 2,
  "name": "Phase 8 Preelock Session Policy",
  "mode": "blocklist",
  "domains": [
    "instagram.com"
  ],
  "block_doh": true
}
EOF

# 3. Start proxy on port 53
echo "Starting DNS proxy on 127.0.0.1:53..."
sudo "${BIN}" --port 53 --policy "${POLICY_FILE}" --socket "${SOCK_FILE}" > /dev/null 2>&1 &
PROXY_PID=$!
sleep 1.2

# 4. Redirect active service DNS to 127.0.0.1
echo "Redirecting ${SERVICE} system DNS to 127.0.0.1..."
sudo /usr/sbin/networksetup -setdnsservers "${SERVICE}" "127.0.0.1"
sudo /usr/bin/dscacheutil -flushcache
sudo /usr/bin/killall -HUP mDNSResponder

CURRENT_DNS=$(networksetup -getdnsservers "${SERVICE}")
echo "Current ${SERVICE} DNS: ${CURRENT_DNS}"

echo ""
echo "--- Step 1: DNS Level Resolution Verification ---"
DIG_INSTA=$(dig @127.0.0.1 -p 53 +time=2 +tries=1 instagram.com)
echo "Query instagram.com on 127.0.0.1:53:"
if echo "${DIG_INSTA}" | grep -q "status: NXDOMAIN"; then
    echo "  >>> PASS: instagram.com returned NXDOMAIN"
else
    echo "  >>> FAIL: instagram.com did not return NXDOMAIN"
    exit 1
fi

DIG_WIKI=$(dig @127.0.0.1 -p 53 +time=3 +tries=2 wikipedia.org +short | head -n 1)
echo "Query wikipedia.org on 127.0.0.1:53: ${DIG_WIKI}"
if [ -n "${DIG_WIKI}" ]; then
    echo "  >>> PASS: wikipedia.org resolved to ${DIG_WIKI}"
else
    echo "  >>> FAIL: wikipedia.org could not be resolved"
    exit 1
fi

echo ""
echo "--- Step 2: System Resolver / HTTP Client Verification (Simulating Browser) ---"
echo "Attempting HTTP connection to instagram.com (should fail to resolve host)..."
set +e
CURL_INSTA_OUT=$(curl -sS -m 4 -I "http://instagram.com" 2>&1)
CURL_INSTA_CODE=$?
set -euo pipefail

echo "Curl exit code: ${CURL_INSTA_CODE}"
echo "Curl output: ${CURL_INSTA_OUT}"
if [ "${CURL_INSTA_CODE}" -eq 6 ] || echo "${CURL_INSTA_OUT}" | grep -qiE "Could not resolve host|Failed to connect|timed out"; then
    echo "  >>> PASS: instagram.com successfully blocked from loading by local DNS"
else
    echo "  >>> FAIL: instagram.com unexpectedly loaded or did not fail with DNS error"
    exit 1
fi

echo "Attempting HTTP connection to wikipedia.org (should succeed with HTTP 200/301)..."
CURL_WIKI_HEADER=$(curl -s -m 5 -I "https://wikipedia.org" | head -n 1)
echo "wikipedia.org HTTP Status: ${CURL_WIKI_HEADER}"
if echo "${CURL_WIKI_HEADER}" | grep -qE "200|301|302"; then
    echo "  >>> PASS: wikipedia.org loaded successfully"
else
    echo "  >>> FAIL: wikipedia.org failed to load"
    exit 1
fi

echo ""
echo "--- Step 3: Session Teardown & Clean Restoration Verification ---"
echo "Restoring ${SERVICE} DNS to Empty/DHCP..."
sudo /usr/sbin/networksetup -setdnsservers "${SERVICE}" "Empty"
sudo /usr/bin/dscacheutil -flushcache
sudo /usr/bin/killall -HUP mDNSResponder
sudo kill -9 "${PROXY_PID}" 2>/dev/null || true

RESTORED_DNS=$(networksetup -getdnsservers "${SERVICE}")
echo "Restored ${SERVICE} DNS: ${RESTORED_DNS}"
if echo "${RESTORED_DNS}" | grep -q "There aren't any DNS Servers set"; then
    echo "  >>> PASS: System DNS verified restored to default DHCP"
else
    echo "  >>> FAIL: System DNS was not restored to default DHCP"
    exit 1
fi

sleep 1.0
CURL_WIKI_AFTER=$(curl -s -m 5 -I "https://wikipedia.org" | head -n 1)
echo "wikipedia.org post-teardown status: ${CURL_WIKI_AFTER}"
if echo "${CURL_WIKI_AFTER}" | grep -qE "200|301|302"; then
    echo "  >>> PASS: Normal web browsing fully operational after teardown"
else
    echo "  >>> FAIL: Web browsing impaired after teardown"
    exit 1
fi

echo ""
echo "=========================================================="
echo "ALL LIVE INTEGRATION & BROWSER VERIFICATION CHECKS PASSED!"
echo "=========================================================="
