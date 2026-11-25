#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
PLIST_SRC="${ROOT_DIR}/Service/com.preelock.dns.plist"
PLIST_DST="/Library/LaunchDaemons/com.preelock.dns.plist"
STATE_FILE="/tmp/preelock_original_dns.txt"
UPSTREAM_FILE="/tmp/preelock_upstream_dns.txt"
WATCHDOG_SCRIPT="${SCRIPT_DIR}/watchdog.sh"

echo "=========================================================="
echo "Preelock DNS Proxy Background Service Installer"
echo "=========================================================="
echo "This script will configure the Preelock DNS proxy as a"
echo "system-level LaunchDaemon that starts at boot and restarts"
echo "automatically if it crashes."
echo ""
echo "Actions to be performed:"
echo " 1. Ensure proxy binary is compiled (${ROOT_DIR}/build/preelock-dns-test)"
echo " 2. Save active DNS and upstream configuration"
echo " 3. Copy LaunchDaemon plist to ${PLIST_DST}"
echo " 4. Load daemon via launchctl"
echo " 5. Redirect active network service DNS to 127.0.0.1"
echo " 6. Launch background watchdog fail-safe monitor"
echo "=========================================================="
echo ""

# 1. Compile CLI binary if not present
if [ ! -f "${ROOT_DIR}/build/preelock-dns-test" ]; then
    echo "Compiling preelock-dns-test binary..."
    "${SCRIPT_DIR}/build_cli.sh"
fi

# 2. Detect active network service
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

# 3. Capture and save current DNS state
if [ ! -f "${STATE_FILE}" ]; then
    CURRENT_DNS=$(networksetup -getdnsservers "${SERVICE}" 2>/dev/null || true)
    echo "${CURRENT_DNS}" > "${STATE_FILE}"
    echo "Saved original DNS state to ${STATE_FILE}"
fi

# Capture active upstreams before pointing to 127.0.0.1
UPSTREAMS=$( (scutil --dns 2>/dev/null | awk '/nameserver\[[0-9]+\]/{print $3}' | grep -v '127.0.0.1' || true) | sort -u | tr '\n' ' ' || true)
UPSTREAMS=$(echo "${UPSTREAMS}" | xargs)
if [ -z "${UPSTREAMS}" ]; then UPSTREAMS="8.8.8.8 1.1.1.1"; fi
echo "${UPSTREAMS}" > "${UPSTREAM_FILE}"
echo "Recorded active upstream resolvers: ${UPSTREAMS}"

# 4. Unload old daemon if present
if sudo launchctl list | grep -q "com.preelock.dns" 2>/dev/null; then
    echo "Stopping existing daemon instance..."
    sudo launchctl bootout system/com.preelock.dns 2>/dev/null || sudo launchctl unload -w "${PLIST_DST}" 2>/dev/null || true
    sleep 0.5
fi

# 5. Install plist
echo "Installing LaunchDaemon plist to ${PLIST_DST}..."
sudo cp "${PLIST_SRC}" "${PLIST_DST}"
sudo chown root:wheel "${PLIST_DST}"
sudo chmod 644 "${PLIST_DST}"

# 6. Load daemon via launchctl
echo "Bootstrapping LaunchDaemon com.preelock.dns..."
sudo launchctl bootstrap system "${PLIST_DST}" 2>/dev/null || sudo launchctl load -w "${PLIST_DST}"

# Wait for proxy socket to become ready
echo "Waiting for proxy service to initialize..."
READY=false
for i in {1..30}; do
    if sudo launchctl list | grep -q "com.preelock.dns" && dig @127.0.0.1 -p 53 +time=1 +tries=1 health.preelock.local > /dev/null 2>&1; then
        READY=true
        break
    fi
    sleep 0.1
done

if [ "${READY}" != "true" ]; then
    echo "WARNING: Daemon started but port 53 query probe did not immediately reply. Checking logs..."
    cat "${ROOT_DIR}/build/daemon.err" 2>/dev/null || true
fi

# 7. Apply DNS redirect
echo "Pointing service '${SERVICE}' DNS to 127.0.0.1..."
networksetup -setdnsservers "${SERVICE}" 127.0.0.1
dscacheutil -flushcache
sudo killall -HUP mDNSResponder 2>/dev/null || true

# 8. Start fail-safe watchdog in background
sudo pkill -9 -f "watchdog.sh" 2>/dev/null || pkill -9 -f "watchdog.sh" 2>/dev/null || true
sudo nohup "${WATCHDOG_SCRIPT}" > /dev/null 2>&1 &
echo "Watchdog fail-safe monitor active."

echo ""
echo "=========================================================="
echo "Preelock DNS Proxy background service successfully installed!"
echo "Service Label : com.preelock.dns"
echo "Status        : Running (PID $(pgrep -f "preelock-dns-test.*--port 53" | head -n 1 || echo 'unknown'))"
echo "System DNS    : 127.0.0.1 on ${SERVICE}"
echo "To uninstall  : ./Scripts/uninstall_service.sh"
echo "=========================================================="
