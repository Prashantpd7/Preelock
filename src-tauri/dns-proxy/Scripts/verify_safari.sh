#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
BIN="${ROOT_DIR}/build/preelock-dns-proxy"
POLICY_FILE="/tmp/preelock_active_policy.json"
SOCK_FILE="/tmp/preelock-dns-control.sock"

# Detect service
SERVICE="Wi-Fi"

cleanup() {
    sudo /usr/sbin/networksetup -setdnsservers "${SERVICE}" "Empty" 2>/dev/null || true
    sudo /usr/bin/dscacheutil -flushcache 2>/dev/null || true
    sudo /usr/bin/killall -HUP mDNSResponder 2>/dev/null || true
    sudo pkill -9 -f "preelock-dns-proxy.*--port 53" 2>/dev/null || true
    rm -f "${POLICY_FILE}" "${SOCK_FILE}"
}
trap cleanup EXIT

cat << 'EOF' > "${POLICY_FILE}"
{
  "version": 2,
  "name": "Safari Test Policy",
  "mode": "blocklist",
  "domains": ["instagram.com"],
  "block_doh": true
}
EOF

sudo "${BIN}" --port 53 --policy "${POLICY_FILE}" --socket "${SOCK_FILE}" > /dev/null 2>&1 &
PROXY_PID=$!
sleep 1.0

sudo /usr/sbin/networksetup -setdnsservers "${SERVICE}" "127.0.0.1"
sudo /usr/bin/dscacheutil -flushcache
sudo /usr/bin/killall -HUP mDNSResponder

echo "=== Safari Browser Test: Blocked Site (instagram.com) ==="
osascript << 'APPLESCRIPT'
tell application "Safari"
    activate
    if (count of windows) = 0 then
        make new document
    end if
    set URL of current tab of front window to "http://instagram.com"
    delay 3
    set pageTitle to name of current tab of front window
    set pageURL to URL of current tab of front window
    return "URL: " & pageURL & " | Title: " & pageTitle
end tell
APPLESCRIPT

echo ""
echo "=== Safari Browser Test: Allowed Site (wikipedia.org) ==="
osascript << 'APPLESCRIPT'
tell application "Safari"
    set URL of current tab of front window to "https://www.wikipedia.org"
    delay 3
    set pageTitle to name of current tab of front window
    set pageURL to URL of current tab of front window
    return "URL: " & pageURL & " | Title: " & pageTitle
end tell
APPLESCRIPT

# Close the test tab/window in Safari
osascript -e 'tell application "Safari" to close front window' 2>/dev/null || true
