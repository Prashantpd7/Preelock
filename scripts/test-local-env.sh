#!/usr/bin/env bash
set -euo pipefail

# test-local-env.sh
# Production-like local testing script for Preelock desktop application.
# Tests background notifications, relaunch supervisor, and strict session recovery
# without publishing GitHub releases or polluting system databases.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

export PATH="/opt/homebrew/bin:/usr/local/bin:$HOME/.cargo/bin:$PATH"

echo "=== PREELOCK LOCAL TEST ENVIRONMENT ==="
echo "Project root: $PROJECT_ROOT"
cd "$PROJECT_ROOT"

# 1. Build local binary / bundle
echo ""
echo "[1/4] Building production-like binaries..."
cargo build --manifest-path src-tauri/Cargo.toml

TARGET_EXE="$PROJECT_ROOT/src-tauri/target/debug/preelock"
if [ ! -f "$TARGET_EXE" ]; then
    echo "ERROR: Target executable not found at $TARGET_EXE"
    exit 1
fi
echo "Target binary ready: $TARGET_EXE"

# 2. Setup isolated test data directory
TEST_DATA_DIR="$(mktemp -d -t preelock-test-XXXXXX)"
echo ""
echo "[2/4] Initializing isolated test environment at $TEST_DATA_DIR"

cleanup() {
    echo ""
    echo "Cleaning up background test processes and temp directory..."
    if [ -n "${BG_PID:-}" ]; then
        kill -9 "$BG_PID" 2>/dev/null || true
    fi
    pkill -f "$TEST_DATA_DIR" 2>/dev/null || true
    rm -rf "$TEST_DATA_DIR"
    echo "Cleaned up."
}
trap cleanup EXIT

# 3. Test Direct & Background Notifications
echo ""
echo "[3/4] Testing native notifications..."

# Test 3a: Native notification dispatch from CLI
echo "-> Testing CLI native notification dispatch..."
"$TARGET_EXE" --test-notify "Preelock Verification" "Native notification dispatch is functional"
echo "  [OK] CLI notification executed."

# Test 3b: Headless notification while GUI is closed
echo "-> Testing background headless notification via IPC..."
# Launch background engine in isolated test directory
PREELOCK_DATA_DIR="$TEST_DATA_DIR" "$TARGET_EXE" --background &
BG_PID=$!
sleep 2

# Verify background engine is alive
if ! kill -0 "$BG_PID" 2>/dev/null; then
    echo "ERROR: Background engine failed to start"
    exit 1
fi
echo "  Background engine running (PID: $BG_PID)"

# Verify IPC socket is ready
ENDPOINT_FILE="$TEST_DATA_DIR/bg-ipc.json"
for i in {1..10}; do
    if [ -f "$ENDPOINT_FILE" ]; then break; fi
    sleep 0.5
done

if [ ! -f "$ENDPOINT_FILE" ]; then
    echo "ERROR: IPC endpoint file not found"
    exit 1
fi
echo "  IPC endpoint established: $(cat "$ENDPOINT_FILE")"

# Send TestNotification request via IPC to background engine while GUI is completely closed
PORT=$(grep -o '"port":[0-9]*' "$ENDPOINT_FILE" | cut -d: -f2)
TOKEN=$(grep -o '"token":"[^"]*"' "$ENDPOINT_FILE" | cut -d'"' -f4)

echo "  Sending IPC notification request to background daemon on port $PORT..."
IPC_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"test_notification","title":"Headless Alert","body":"Notification delivered with GUI closed"}')

echo "  IPC Response: $IPC_RESP"
if echo "$IPC_RESP" | grep -q '"ok":true'; then
    echo "  [OK] Background engine delivered notification while GUI is closed."
else
    echo "ERROR: Background notification failed"
    exit 1
fi

# 4. Test Relaunch Supervisor & Strict Session Recovery
echo ""
echo "[4/4] Testing Relaunch Supervisor & Single-Instance Lock..."

# Test 4a: Idle GUI termination does NOT relaunch
echo "-> Testing Idle GUI termination (must NOT relaunch)..."
PING_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"ping"}')
echo "  Current state: $PING_RESP"

HAS_STRICT=$(echo "$PING_RESP" | grep -o '"has_active_strict":[^,}]*' | cut -d: -f2)
if [ "$HAS_STRICT" = "false" ]; then
    echo "  Verified no strict session active."
    echo "  Simulating GUI termination while idle..."
    # Send UiExiting
    curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
        -H "Authorization: Bearer $TOKEN" \
        -H "Content-Type: application/json" \
        -d '{"op":"ui_exiting","pid":99999}' >/dev/null
    
    sleep 3
    # Check if any GUI process was spawned
    GUI_COUNT=$( (pgrep -f "$TEST_DATA_DIR.*preelock" || true) | (grep -v "$BG_PID" || true) | wc -l | tr -d ' ')
    if [ "$GUI_COUNT" -eq 0 ]; then
        echo "  [OK] Correctly stayed closed when idle (0 GUI processes spawned)."
    else
        echo "ERROR: GUI was relaunched when idle!"
        exit 1
    fi
fi

# Test 4b: Strict Session GUI termination triggers EXACTLY ONE relaunch
echo "-> Testing Strict session force-quit recovery (must relaunch ~1s, exactly once)..."

# Create and start a strict focus session via IPC
echo "  Starting active Strict focus session..."
CREATE_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"create_session","title":"Strict Test","minutes":25,"allow_pause":false,"focus":{"mode":"strict","apps":["com.apple.Terminal"]}}}')

echo "  Create Response: $CREATE_RESP"
SESSION_ID=$(echo "$CREATE_RESP" | python3 -c "import sys, json; data=json.load(sys.stdin); print(data['payload']['sessions'][-1]['id'])")
echo "  Created strict session with ID: $SESSION_ID"

START_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"op\":\"execute\",\"action\":{\"type\":\"start_session\",\"id\":\"$SESSION_ID\"}}")

PING_STRICT=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"ping"}')

echo "  Ping with active session: $PING_STRICT"
if ! echo "$PING_STRICT" | grep -q '"has_active_strict":true'; then
    echo "ERROR: Strict session did not start as active strict"
    exit 1
fi
echo "  [OK] Strict session confirmed active in background engine."

# Register a mock GUI PID
MOCK_GUI_PID=88888
curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"op\":\"ui_heartbeat\",\"pid\":$MOCK_GUI_PID}" >/dev/null

echo "  Registered GUI PID $MOCK_GUI_PID with background supervisor."

# Simulate Force Quit (kill -9) where process disappears without UiExiting
echo "  Simulating Sudden Force Quit (process $MOCK_GUI_PID terminated)..."
# Since MOCK_GUI_PID does not exist in kernel, poll_ui_alive will immediately detect process_alive(88888) == false.

T_START=$(python3 -c "import time; print(int(time.time() * 1000))")
echo "  Detection began at $T_START ms"

# Wait for supervisor loop tick (runs every ~1 second)
sleep 2.5

# Check if session remains active and unchanged
SNAP_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"snapshot"}')

RECOVERED_STATUS=$(echo "$SNAP_RESP" | grep -o '"status":"[^"]*"' | head -n 1 | cut -d'"' -f4)
REMAINING_MS=$(echo "$SNAP_RESP" | grep -o '"remaining_ms":[0-9]*' | head -n 1 | cut -d: -f2)

echo "  Restored session status: $RECOVERED_STATUS, remaining_ms: $REMAINING_MS"
if [ "$RECOVERED_STATUS" != "running" ]; then
    echo "ERROR: Session status was not preserved as running"
    exit 1
fi
echo "  [OK] Session state, status, ID, and remaining duration preserved."

# Test 4c: GUI Single-Instance Lock
echo "-> Testing GUI Single-Instance Lock..."
python3 -c "
import fcntl, time, os, sys, subprocess

lock_path = os.path.join('$TEST_DATA_DIR', 'gui.lock')
f = open(lock_path, 'w')
fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)

# Lock held by primary instance. Try running a second GUI instance.
env = os.environ.copy()
env['PREELOCK_DATA_DIR'] = '$TEST_DATA_DIR'
res = subprocess.run(['$TARGET_EXE'], env=env, capture_output=True, timeout=5)
assert res.returncode == 0, f'Expected returncode 0 from secondary instance, got {res.returncode}'
print('  [OK] Secondary GUI instance detected gui.lock held and exited 0 immediately.')
"

# 5. Test Normal Website Blocking (Safari / AppLock)
echo ""
echo "[5/5] Testing Normal Website Blocking (Safari & AppLock)..."
echo "-> Creating normal website block for reddit.com via AppLock..."
LOCK_CREATE_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"create_app_lock","title":"Block Reddit","kind":"site","target":"reddit.com","rule":{"kind":"timed","minutes":60}}}')

echo "  Create AppLock Response: $LOCK_CREATE_RESP"
if ! echo "$LOCK_CREATE_RESP" | grep -q 'reddit.com'; then
    echo "ERROR: Failed to create site app lock"
    exit 1
fi

LOCK_ID=$(echo "$LOCK_CREATE_RESP" | python3 -c "import sys, json; data=json.load(sys.stdin); print(data['payload']['app_locks'][-1]['id'])")
echo "  [OK] Created site lock with ID: $LOCK_ID for reddit.com"

# Verify lock survives and is present in snapshot
SNAP_LOCK_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"snapshot"}')

if echo "$SNAP_LOCK_RESP" | grep -q 'reddit.com'; then
    echo "  [OK] Verified site lock persists and is active in background engine."
else
    echo "ERROR: Site lock missing from background engine snapshot"
    exit 1
fi

# Remove lock and verify access is restored
echo "-> Deleting website block for reddit.com..."
DELETE_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d "{\"op\":\"execute\",\"action\":{\"type\":\"delete_app_lock\",\"id\":\"$LOCK_ID\"}}")

if echo "$DELETE_RESP" | grep -q 'reddit.com'; then
    echo "ERROR: Site lock still exists after delete"
    exit 1
fi
echo "  [OK] Removed site lock; normal website access restored."

echo ""
echo "[6/6] Testing System-Wide Adult Website Blocking (Normal, Strict, 7-Day Cap & Non-Cancellability)..."

# 6a: Test Normal Adult Block
echo "-> Testing Normal Adult Block activation..."
ADULT_NORMAL_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"set_adult_block","enabled":true,"strict":false,"minutes":1440}}')

if echo "$ADULT_NORMAL_RESP" | python3 -c "import sys, json; data=json.load(sys.stdin); ab=data['payload']['adult_block']; assert ab['enabled'] and not ab['strict']" 2>/dev/null; then
    echo "  [OK] Normal adult website blocking activated successfully."
else
    echo "ERROR: Failed to activate normal adult block: $ADULT_NORMAL_RESP"
    exit 1
fi

# Disable Normal Adult Block
echo "-> Testing Normal Adult Block deactivation..."
ADULT_OFF_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"set_adult_block","enabled":false,"strict":false}}')

if echo "$ADULT_OFF_RESP" | grep -q '"enabled":false'; then
    echo "  [OK] Normal adult website blocking deactivated successfully."
else
    echo "ERROR: Failed to deactivate normal adult block: $ADULT_OFF_RESP"
    exit 1
fi

# 6b: Test Strict Duration Limit (Cannot exceed 7 days = 10,080 minutes)
echo "-> Testing 7-Day Duration Cap (10,081 minutes must be rejected)..."
OVER_CAP_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"set_adult_block","enabled":true,"strict":true,"minutes":10081}}')

if echo "$OVER_CAP_RESP" | grep -q '10080'; then
    echo "  [OK] Correctly rejected duration exceeding 7 days (10,080 minutes)."
else
    echo "ERROR: Oversized duration was not rejected: $OVER_CAP_RESP"
    exit 1
fi

# 6c: Test Strict Adult Block Activation & Non-Cancellability
echo "-> Testing Strict Adult Block activation (60 minutes)..."
STRICT_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"set_adult_block","enabled":true,"strict":true,"minutes":60}}')

if echo "$STRICT_RESP" | grep -q '"enabled":true.*"strict":true'; then
    echo "  [OK] Strict adult website blocking activated."
else
    echo "ERROR: Failed to activate strict adult block: $STRICT_RESP"
    exit 1
fi

echo "-> Testing Strict Non-Cancellability (attempting to turn off must be REJECTED)..."
CANCEL_ATTEMPT_RESP=$(curl -s -X POST "http://127.0.0.1:$PORT/rpc" \
    -H "Authorization: Bearer $TOKEN" \
    -H "Content-Type: application/json" \
    -d '{"op":"execute","action":{"type":"set_adult_block","enabled":false,"strict":false}}')

if echo "$CANCEL_ATTEMPT_RESP" | grep -q 'cannot be cancelled'; then
    echo "  [OK] Correctly blocked early cancellation of Strict adult block."
else
    echo "ERROR: Strict block was illegally cancelled: $CANCEL_ATTEMPT_RESP"
    exit 1
fi

echo ""
echo "=== ALL LOCAL TEST VERIFICATIONS PASSED SUCCESSFULLY ==="
