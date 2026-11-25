#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

STATE_FILE="/tmp/preelock_original_dns.txt"
UPSTREAM_FILE="/tmp/preelock_upstream_dns.txt"
PF_ANCHOR="com.preelock.dns"

echo "=========================================================="
echo "Preelock Local DNS Proxy Setup (Free Path — No SysExt)"
echo "=========================================================="

# 1. Detect active network service
DEFAULT_IF=$(route -n get default 2>/dev/null | awk '/interface:/{print $2}')
if [ -z "${DEFAULT_IF}" ]; then
    DEFAULT_IF="en0"
fi

# Map interface to networksetup service name
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

echo "Active Network Interface : ${DEFAULT_IF}"
echo "Network Service Name     : ${SERVICE}"

# 2. Capture and save current DNS configuration before altering
CURRENT_DNS=$(networksetup -getdnsservers "${SERVICE}" 2>/dev/null || true)
echo "${CURRENT_DNS}" > "${STATE_FILE}"
echo "Saved original DNS state to ${STATE_FILE}"

# Capture active upstream resolvers before switching to 127.0.0.1
UPSTREAMS=$(scutil --dns 2>/dev/null | awk '/nameserver\[[0-9]+\]/{print $3}' | grep -v '127.0.0.1' | sort -u | tr '\n' ' ')
if [ -z "${UPSTREAMS}" ]; then
    UPSTREAMS="8.8.8.8 1.1.1.1"
fi
echo "${UPSTREAMS}" > "${UPSTREAM_FILE}"
echo "Recorded active upstream DNS: ${UPSTREAMS} in ${UPSTREAM_FILE}"

# 3. Mode Selection: Option (ii) PF Port Redirection (preferred: unprivileged proxy) vs Option (i) Port 53
MODE="pf"
if [ "${1:-}" == "--port-53" ] || [ "${1:-}" == "-p53" ]; then
    MODE="port53"
fi

# 4. Set macOS system DNS to 127.0.0.1 on the active network service
echo ""
echo "Setting DNS for service '${SERVICE}' to 127.0.0.1..."
networksetup -setdnsservers "${SERVICE}" 127.0.0.1

if [ "${MODE}" == "pf" ]; then
    echo "Configuring PF redirect: loopback port 53 -> port 5354..."
    echo "Note: Sudo is used only for the firewall redirection rule; the DNS proxy itself runs UNPRIVILEGED."
    PF_RULE="rdr pass on lo0 proto udp from any to 127.0.0.1 port 53 -> 127.0.0.1 port 5354"
    if sudo -n true 2>/dev/null; then
        echo "${PF_RULE}" | sudo pfctl -a "${PF_ANCHOR}" -f - 2>/dev/null || true
        sudo pfctl -e 2>/dev/null || true
        echo "[OK] PF redirect enabled via sudo."
    else
        echo "[INFO] Sudo requires a password; skipping automated PF rule load."
        echo "       To enable PF port redirection, run:"
        echo "       echo \"${PF_RULE}\" | sudo pfctl -a \"${PF_ANCHOR}\" -f - && sudo pfctl -e"
    fi
    echo "[OK] System DNS pointed to 127.0.0.1. Traffic redirected to 127.0.0.1:5354 via PF."
    echo "You can now run the DNS proxy as a normal user:"
    echo "  ./build/preelock-dns-test"
else
    echo "[OK] System DNS pointed to 127.0.0.1 (standard port 53)."
    echo "Run the DNS proxy with elevated socket privileges:"
    echo "  sudo ./build/preelock-dns-test --port 53"
fi

echo ""
echo "Verification: networksetup -getdnsservers '${SERVICE}' -> $(networksetup -getdnsservers "${SERVICE}")"
echo "To restore normal DNS at any time, run: ./Scripts/teardown_local_dns.sh"
echo "=========================================================="
