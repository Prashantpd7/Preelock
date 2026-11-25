#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PLIST_DST="/Library/LaunchDaemons/com.preelock.dns.plist"
STATE_FILE="/tmp/preelock_original_dns.txt"
UPSTREAM_FILE="/tmp/preelock_upstream_dns.txt"
PID_FILE="/tmp/preelock_watchdog.pid"
SOCK_FILE="${ROOT_DIR}/build/preelock-dns-control.sock"

echo "=========================================================="
echo "Preelock DNS Proxy Background Service Uninstaller"
echo "=========================================================="

# 1. Stop background watchdog monitor
echo "Stopping watchdog monitor..."
sudo pkill -9 -f "watchdog.sh" 2>/dev/null || pkill -9 -f "watchdog.sh" 2>/dev/null || true
sudo rm -f "${PID_FILE}"

# 2. Unload LaunchDaemon
echo "Unloading LaunchDaemon com.preelock.dns..."
sudo launchctl bootout system/com.preelock.dns 2>/dev/null || sudo launchctl unload -w "${PLIST_DST}" 2>/dev/null || true
sudo rm -f "${PLIST_DST}"

# Terminate any leftover proxy processes
sudo pkill -9 -f "preelock-dns.*--port 53" 2>/dev/null || true

# 3. Detect active network service
DEFAULT_IF=$( (/sbin/route -n get default 2>/dev/null || true) | awk '/interface:/{print $2}')
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

# 4. Restore original DNS
echo "Restoring original DNS configuration on '${SERVICE}'..."
if [ -f "${STATE_FILE}" ]; then
    SAVED=$(cat "${STATE_FILE}")
    if [[ "${SAVED}" =~ "There aren't any DNS Servers set" ]] || [ -z "${SAVED}" ]; then
        networksetup -setdnsservers "${SERVICE}" Empty 2>/dev/null || true
    else
        read -r -a SERVERS <<< "${SAVED}"
        networksetup -setdnsservers "${SERVICE}" "${SERVERS[@]}" 2>/dev/null || true
    fi
    sudo rm -f "${STATE_FILE}" || rm -f "${STATE_FILE}" || true
else
    networksetup -setdnsservers "${SERVICE}" Empty 2>/dev/null || true
fi

# Clean up state files
sudo rm -f "${UPSTREAM_FILE}" "${SOCK_FILE}" 2>/dev/null || rm -f "${UPSTREAM_FILE}" "${SOCK_FILE}" 2>/dev/null || true

# 5. Flush resolution cache
dscacheutil -flushcache 2>/dev/null || true
sudo killall -HUP mDNSResponder 2>/dev/null || true

echo ""
echo "=========================================================="
echo "Preelock DNS Proxy background service successfully uninstalled."
echo "Active DNS on '${SERVICE}':"
networksetup -getdnsservers "${SERVICE}"
echo "=========================================================="
