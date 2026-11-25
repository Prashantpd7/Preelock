#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
STATE_FILE="/tmp/preelock_original_dns.txt"
PLIST_DST="/Library/LaunchDaemons/com.preelock.dns.plist"
LOG_FILE="${ROOT_DIR}/build/verify_phase6.log"
WATCHDOG_LOG="${ROOT_DIR}/build/watchdog.log"
SOCK_FILE="${ROOT_DIR}/build/preelock-dns-control.sock"

mkdir -p "${ROOT_DIR}/build"
echo "=== Phase 6 System Integration Verification Suite ===" | tee "${LOG_FILE}"
echo "Started at: $(date)" | tee -a "${LOG_FILE}"
echo "" | tee -a "${LOG_FILE}"

cleanup_test() {
    echo "" | tee -a "${LOG_FILE}"
    echo "--- Cleaning up test environment ---" | tee -a "${LOG_FILE}"
    "${SCRIPT_DIR}/uninstall_service.sh" > /dev/null 2>&1 || true
}
trap cleanup_test EXIT

TESTS_PASSED=0
TOTAL_TESTS=6

# Helper function
run_test() {
    local name="$1"
    local desc="$2"
    echo "--------------------------------------------------------" | tee -a "${LOG_FILE}"
    echo "TEST: ${name} - ${desc}" | tee -a "${LOG_FILE}"
}

# 1. Test 1: Service Installation and Port 53 DNS Resolution
run_test "TEST 1" "Daemon install and port 53 query resolution"
"${SCRIPT_DIR}/install_service.sh" >> "${LOG_FILE}" 2>&1

# Verify plist installed
if [ ! -f "${PLIST_DST}" ]; then
    echo "FAIL: Plist was not copied to ${PLIST_DST}" | tee -a "${LOG_FILE}"
    exit 1
fi

# Verify daemon loaded in launchctl
if ! sudo launchctl list | grep -q "com.preelock.dns"; then
    echo "FAIL: Daemon 'com.preelock.dns' is not loaded in launchctl" | tee -a "${LOG_FILE}"
    exit 1
fi

# Verify query resolution via 127.0.0.1:53
QUERY_OUT=$(dig @127.0.0.1 -p 53 +time=2 +tries=1 instagram.com)
if echo "${QUERY_OUT}" | grep -q "status: NXDOMAIN"; then
    echo "PASS: Daemon answered DNS query with NXDOMAIN on 127.0.0.1:53" | tee -a "${LOG_FILE}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo "FAIL: Query to 127.0.0.1:53 did not return NXDOMAIN" | tee -a "${LOG_FILE}"
    echo "${QUERY_OUT}" | tee -a "${LOG_FILE}"
    exit 1
fi

# 2. Test 2: KeepAlive Automatic Restart
run_test "TEST 2" "KeepAlive automatic restart after SIGKILL"
OLD_PID=$(pgrep -f "preelock-dns.*--port 53" | head -n 1)
echo "Initial proxy PID: ${OLD_PID}" | tee -a "${LOG_FILE}"

echo "Killing proxy process ${OLD_PID} with SIGKILL..." | tee -a "${LOG_FILE}"
sudo kill -9 "${OLD_PID}"

# Allow launchd ThrottleInterval (2s) to restart
NEW_PID=""
for i in {1..20}; do
    sleep 0.5
    CURRENT=$(pgrep -f "preelock-dns.*--port 53" | head -n 1 || true)
    if [ -n "${CURRENT}" ] && [ "${CURRENT}" != "${OLD_PID}" ]; then
        NEW_PID="${CURRENT}"
        break
    fi
done

if [ -n "${NEW_PID}" ]; then
    echo "Restart confirmed: New proxy PID is ${NEW_PID} (restarted within 3s)" | tee -a "${LOG_FILE}"
    # Verify query works after restart
    sleep 0.5
    POST_KILL_OUT=$(dig @127.0.0.1 -p 53 +time=2 +tries=1 instagram.com)
    if echo "${POST_KILL_OUT}" | grep -q "status: NXDOMAIN"; then
        echo "PASS: KeepAlive successfully restored proxy and DNS answers queries" | tee -a "${LOG_FILE}"
        TESTS_PASSED=$((TESTS_PASSED + 1))
    else
        echo "FAIL: Proxy restarted but DNS query failed" | tee -a "${LOG_FILE}"
        exit 1
    fi
else
    echo "FAIL: Proxy was not automatically restarted by launchd KeepAlive" | tee -a "${LOG_FILE}"
    exit 1
fi

# 3. Test 3: Control Socket Functionality under LaunchDaemon
run_test "TEST 3" "Unix control socket operations under LaunchDaemon"
STATUS_OUT=$("${SCRIPT_DIR}/dns_control.sh" status)
echo "${STATUS_OUT}" >> "${LOG_FILE}"

if echo "${STATUS_OUT}" | grep -q '"status" *: *"running"'; then
    echo "PASS: Control socket successfully communicated with running daemon" | tee -a "${LOG_FILE}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo "FAIL: Control socket query failed while proxy runs under daemon" | tee -a "${LOG_FILE}"
    exit 1
fi

# 4. Test 4: Network Change & Tamper Detection
run_test "TEST 4" "Network change detection and automatic DNS re-application"
# Tamper Wi-Fi DNS to 8.8.8.8
echo "Tampering system DNS on Wi-Fi to 8.8.8.8..." | tee -a "${LOG_FILE}"
networksetup -setdnsservers Wi-Fi 8.8.8.8

# NetworkWatcher checks every 3s. Wait up to 6s for re-application
REAPPLIED=false
for i in {1..14}; do
    sleep 0.5
    CURRENT_DNS=$(networksetup -getdnsservers Wi-Fi)
    if [ "${CURRENT_DNS}" = "127.0.0.1" ]; then
        REAPPLIED=true
        break
    fi
done

if [ "${REAPPLIED}" = "true" ]; then
    echo "PASS: NetworkWatcher detected DNS change and re-applied 127.0.0.1" | tee -a "${LOG_FILE}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo "FAIL: DNS remained tampered (${CURRENT_DNS}); NetworkWatcher did not re-apply 127.0.0.1" | tee -a "${LOG_FILE}"
    exit 1
fi

# 5. Test 5: Watchdog Fail-Safe Recovery
run_test "TEST 5" "Watchdog health probing and emergency DNS restoration"
echo "Simulating catastrophic failure: unloading daemon and terminating proxy..." | tee -a "${LOG_FILE}"
sudo launchctl bootout system/com.preelock.dns 2>/dev/null || true
sudo pkill -9 -f "preelock-dns.*--port 53" 2>/dev/null || true

# Watchdog checks every 3s; after 3 consecutive failures (9-12s), it restores DNS to DHCP
echo "Waiting for watchdog fail-safe trigger (probing every 3s)..." | tee -a "${LOG_FILE}"
DNS_RESTORED=false
for i in {1..35}; do
    sleep 0.5
    CURRENT_DNS=$(networksetup -getdnsservers Wi-Fi)
    if [ "${CURRENT_DNS}" != "127.0.0.1" ]; then
        DNS_RESTORED=true
        break
    fi
done

if [ "${DNS_RESTORED}" = "true" ]; then
    echo "Watchdog triggered DNS restoration! Current DNS: ${CURRENT_DNS}" | tee -a "${LOG_FILE}"
    if [ -f "${WATCHDOG_LOG}" ] && grep -q "WATCHDOG FAIL-SAFE" "${WATCHDOG_LOG}"; then
        echo "Log evidence confirmed in ${WATCHDOG_LOG}" | tee -a "${LOG_FILE}"
        # Confirm normal domain resolves through restored DNS
        sleep 1
        WIKI_RESOLV=$(dig +time=3 +tries=2 wikipedia.org)
        if echo "${WIKI_RESOLV}" | grep -q "status: NOERROR"; then
            echo "PASS: Watchdog detected failure, restored DNS, and wikipedia.org resolves successfully" | tee -a "${LOG_FILE}"
            TESTS_PASSED=$((TESTS_PASSED + 1))
        else
            echo "FAIL: DNS was restored but wikipedia.org resolution failed" | tee -a "${LOG_FILE}"
            echo "${WIKI_RESOLV}" | tee -a "${LOG_FILE}"
            exit 1
        fi
    else
        echo "FAIL: DNS changed but watchdog log does not record fail-safe event" | tee -a "${LOG_FILE}"
        exit 1
    fi
else
    echo "FAIL: Watchdog did not restore DNS within 18 seconds of proxy outage" | tee -a "${LOG_FILE}"
    exit 1
fi

# 6. Test 6: Clean Uninstall / Re-install Lifecycle
run_test "TEST 6" "Clean re-install and uninstall cycle"
echo "Testing re-installation..." | tee -a "${LOG_FILE}"
"${SCRIPT_DIR}/install_service.sh" >> "${LOG_FILE}" 2>&1

# Verify query works in newly installed daemon
sleep 0.5
REINSTALL_QUERY=$(dig @127.0.0.1 -p 53 +time=2 +tries=1 instagram.com)
if ! echo "${REINSTALL_QUERY}" | grep -q "status: NXDOMAIN"; then
    echo "FAIL: Query failed after re-installation" | tee -a "${LOG_FILE}"
    exit 1
fi
echo "Re-installation verified." | tee -a "${LOG_FILE}"

echo "Running uninstall_service.sh..." | tee -a "${LOG_FILE}"
"${SCRIPT_DIR}/uninstall_service.sh" >> "${LOG_FILE}" 2>&1

# Verify daemon unloaded
if sudo launchctl list | grep -q "com.preelock.dns"; then
    echo "FAIL: Daemon still present in launchctl after uninstall" | tee -a "${LOG_FILE}"
    exit 1
fi

# Verify plist removed
if [ -f "${PLIST_DST}" ]; then
    echo "FAIL: Plist still exists at ${PLIST_DST} after uninstall" | tee -a "${LOG_FILE}"
    exit 1
fi

# Verify socket removed
if [ -e "${SOCK_FILE}" ]; then
    echo "FAIL: Control socket still exists at ${SOCK_FILE} after uninstall" | tee -a "${LOG_FILE}"
    exit 1
fi

# Verify Wi-Fi DNS clean
CLEAN_DNS=$(networksetup -getdnsservers Wi-Fi)
if [[ "${CLEAN_DNS}" =~ "There aren't any DNS Servers set" ]] || [ -z "${CLEAN_DNS}" ]; then
    echo "PASS: Clean uninstall verified. DNS restored to default DHCP." | tee -a "${LOG_FILE}"
    TESTS_PASSED=$((TESTS_PASSED + 1))
else
    echo "FAIL: Wi-Fi DNS not clean after uninstall: ${CLEAN_DNS}" | tee -a "${LOG_FILE}"
    exit 1
fi

echo "" | tee -a "${LOG_FILE}"
echo "========================================================" | tee -a "${LOG_FILE}"
echo "Phase 6 Verification Summary: ${TESTS_PASSED}/${TOTAL_TESTS} tests passed." | tee -a "${LOG_FILE}"
echo "Completed at: $(date)" | tee -a "${LOG_FILE}"
echo "========================================================" | tee -a "${LOG_FILE}"

if [ "${TESTS_PASSED}" -eq "${TOTAL_TESTS}" ]; then
    exit 0
else
    exit 1
fi
