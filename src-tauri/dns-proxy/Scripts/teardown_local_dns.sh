#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STATE_FILE="/tmp/preelock_original_dns.txt"
UPSTREAM_FILE="/tmp/preelock_upstream_dns.txt"
PF_ANCHOR="com.preelock.dns"

echo "=========================================================="
echo "Preelock Local DNS Proxy Teardown"
echo "=========================================================="

# 1. Detect active network service
DEFAULT_IF=$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')
if [ -z "${DEFAULT_IF}" ]; then
    DEFAULT_IF="en0"
fi

SERVICE=""
while IFS= read -r line; do
    if [[ "${line}" =~ "Hardware Port: " ]]; then
        PORT_NAME="${line#Hardware Port: }"
    fi
    if [[ "${line}" =~ "Device: ${DEFAULT_IF}" ]]; then
        SERVICE="${PORT_NAME}"
        break
    fi
done < <(networksetup -listallhardwareports)

if [ -z "${SERVICE}" ]; then
    SERVICE="Wi-Fi"
fi

echo "Network Service Name: ${SERVICE}"

# 2. Restore original DNS settings
if [ -f "${STATE_FILE}" ]; then
    SAVED=$(cat "${STATE_FILE}")
    if [[ "${SAVED}" =~ "There aren't any DNS Servers set" ]] || [ -z "${SAVED}" ]; then
        echo "Restoring DHCP default DNS (clearing static 127.0.0.1)..."
        networksetup -setdnsservers "${SERVICE}" "Empty"
    else
        echo "Restoring previous static DNS servers: ${SAVED}..."
        # Pass each server as an argument
        read -r -a SERVERS <<< "${SAVED}"
        networksetup -setdnsservers "${SERVICE}" "${SERVERS[@]}"
    fi
    rm -f "${STATE_FILE}"
else
    echo "No state file found. Resetting service '${SERVICE}' DNS to DHCP default..."
    networksetup -setdnsservers "${SERVICE}" "Empty"
fi

rm -f "${UPSTREAM_FILE}"

# 3. Clean up PF redirect anchor if active
if sudo -n true 2>/dev/null; then
    echo "Flushing PF redirect anchor (${PF_ANCHOR})..."
    sudo pfctl -a "${PF_ANCHOR}" -F all 2>/dev/null || true
fi

# 4. Flush macOS DNS cache
echo "Flushing local DNS resolution cache..."
dscacheutil -flushcache
if sudo -n true 2>/dev/null; then
    sudo killall -HUP mDNSResponder 2>/dev/null || true
fi

echo ""
echo "[OK] Normal macOS DNS configuration restored successfully."
echo "Current DNS status for '${SERVICE}':"
networksetup -getdnsservers "${SERVICE}"
echo "=========================================================="
