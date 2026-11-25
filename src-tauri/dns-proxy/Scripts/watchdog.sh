#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
STATE_FILE="/tmp/preelock_original_dns.txt"
WATCHDOG_LOG="${ROOT_DIR}/build/watchdog.log"
PID_FILE="/tmp/preelock_watchdog.pid"
DAEMON_LABEL="com.preelock.dns"

mkdir -p "${ROOT_DIR}/build"
echo "$$" > "${PID_FILE}"
echo "[$(date '+%Y-%m-%d %H:%M:%S')] Watchdog monitor active (PID: $$)." >> "${WATCHDOG_LOG}"

cleanup_watchdog() {
    rm -f "${PID_FILE}"
}
trap cleanup_watchdog EXIT

get_active_service() {
    local iface
    iface=$( (/sbin/route -n get default 2>/dev/null || true) | awk '/interface:/{print $2}')
    if [ -z "${iface}" ]; then iface="en0"; fi
    local service=""
    while IFS= read -r line; do
        if [[ "${line}" =~ "Hardware Port: " ]]; then
            PORT_NAME="${line#Hardware Port: }"
        fi
        if [[ "${line}" =~ "Device: ${iface}" ]]; then
            service="${PORT_NAME}"
            break
        fi
    done < <(/usr/sbin/networksetup -listallhardwareports 2>/dev/null)
    echo "${service:-Wi-Fi}"
}

restore_normal_dns() {
    local reason="$1"
    local service
    service=$(get_active_service)
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [WATCHDOG FAIL-SAFE] ${reason}. Restoring normal DNS on '${service}'..." >> "${WATCHDOG_LOG}"

    if [ -f "${STATE_FILE}" ]; then
        local saved
        saved=$(cat "${STATE_FILE}")
        if [[ "${saved}" =~ "There aren't any DNS Servers set" ]] || [ -z "${saved}" ]; then
            /usr/sbin/networksetup -setdnsservers "${service}" Empty 2>/dev/null || true
        else
            read -r -a SERVERS <<< "${saved}"
            /usr/sbin/networksetup -setdnsservers "${service}" "${SERVERS[@]}" 2>/dev/null || true
        fi
    else
        /usr/sbin/networksetup -setdnsservers "${service}" Empty 2>/dev/null || true
    fi
    /usr/bin/dscacheutil -flushcache 2>/dev/null || true
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] [WATCHDOG FAIL-SAFE] Normal DNS restored successfully." >> "${WATCHDOG_LOG}"
}

FAIL_COUNT=0
MAX_FAILS=3
RESTART_HISTORY=()
LAST_PID=""

while true; do
    sleep 3

    # 1. Probe local proxy on port 53 (127.0.0.1) using blocked domain (instant NXDOMAIN from proxy)
    if /usr/bin/dig @127.0.0.1 -p 53 +time=1 +tries=1 instagram.com > /dev/null 2>&1; then
        FAIL_COUNT=0
    else
        FAIL_COUNT=$((FAIL_COUNT + 1))
        echo "[$(date '+%Y-%m-%d %H:%M:%S')] Watchdog probe failed (${FAIL_COUNT}/${MAX_FAILS})" >> "${WATCHDOG_LOG}"
        if [ "${FAIL_COUNT}" -ge "${MAX_FAILS}" ]; then
            restore_normal_dns "Proxy unresponsive after ${MAX_FAILS} consecutive probes"
            # Unload daemon to stop flapping
            if [ "$(id -u)" -eq 0 ]; then
                /bin/launchctl bootout system/"${DAEMON_LABEL}" 2>/dev/null || true
            else
                sudo -n /bin/launchctl bootout system/"${DAEMON_LABEL}" 2>/dev/null || true
            fi
            exit 0
        fi
    fi

    # 2. Check for rapid restart flapping (>5 restarts in 60s)
    CURRENT_PID=$(pgrep -f "preelock-dns-test.*--port 53" | head -n 1 || true)
    if [ -n "${CURRENT_PID}" ]; then
        if [ -n "${LAST_PID}" ] && [ "${CURRENT_PID}" != "${LAST_PID}" ]; then
            NOW=$(date +%s)
            RESTART_HISTORY+=("${NOW}")
            # Filter entries within last 60s
            NEW_HIST=()
            for t in "${RESTART_HISTORY[@]}"; do
                if [ $((NOW - t)) -le 60 ]; then
                    NEW_HIST+=("${t}")
                fi
            done
            RESTART_HISTORY=("${NEW_HIST[@]}")
            if [ "${#RESTART_HISTORY[@]}" -gt 5 ]; then
                restore_normal_dns "Crash loop detected (>5 restarts in 60s)"
                if [ "$(id -u)" -eq 0 ]; then
                    /bin/launchctl bootout system/"${DAEMON_LABEL}" 2>/dev/null || true
                else
                    sudo -n /bin/launchctl bootout system/"${DAEMON_LABEL}" 2>/dev/null || true
                fi
                exit 0
            fi
        fi
        LAST_PID="${CURRENT_PID}"
    fi
done
