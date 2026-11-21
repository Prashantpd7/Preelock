#!/bin/bash
# Preelock Dev launcher.
#
# Double-click "Preelock Dev.app" (Spotlight) to run the LOCAL dev build with
# NO terminal window. It starts `npm run desktop` (Tauri dev + Vite HMR) in
# the background, so code changes show instantly, exactly like running
# `npm run desktop` from a terminal.
#
# Identity separation (why this is a different app from the release copy):
#   * src-tauri/tauri.dev.conf.json overrides the bundle identifier to
#     com.preelock.desktop.dev and the product name to "Preelock Dev"
#     (Tauri v2 `tauri dev --config` merges the file over tauri.conf.json).
#   * PREELOCK_DATA_DIR points the GUI, the background engine, the database
#     and all locks at ~/Library/Application Support/com.preelock.desktop.dev.
#     This is REQUIRED: the engine's data dir is hardcoded to
#     com.preelock.desktop in src-tauri/src/background.rs unless this
#     variable is set, so the identifier override alone would NOT separate
#     the engine — the dev and release engines would share one database
#     and fight over one engine lock.
#   * The release copy in /Applications stays untouched and keeps its own
#     data dir. Run only one copy at a time: both share the extension
#     bridge port (127.0.0.1:47832) and the extension talks to whichever
#     engine started first.
#
# Lifecycle:
#   * Closing the Preelock window does NOT stop the dev server (the
#     background engine keeps running by design). Relaunching while the
#     server is up launches the GUI directly against the live server —
#     no second `npm run desktop`, no killing the live server.
#   * Only a full quit from the tray/menu — no `target/debug/preelock`
#     process left at all — stops the dev server. The launcher waits out
#     short absences (Rust rebuild restarts) before deciding the app
#     fully quit.
#   * If a previous launcher is still alive, this one NEVER stops it and
#     NEVER kills its dev server. It only restores the window (direct GUI
#     launch) and exits — or, when no server is up at all (nothing live
#     to disturb), starts a fresh workflow.
#   * A listener on port 1420 is only ever killed when no launcher or dev
#     workflow for this project owns it.
#   * Orphaned dev processes (background/tray left by a dead workflow,
#     no GUI, no server) are cleared on a fresh start — but only when the
#     dev lifecycle file shows no active session.
#
# Logs: ~/Library/Logs/Preelock/preelock-dev.log (launcher),
#       ~/Library/Logs/Preelock/preelock-dev-npm.log (npm / tauri dev output).

set -u
set -m  # Each background job gets its own process group, so the whole dev
        # workflow (npm + tauri CLI + cargo + Vite) can be stopped with
        # a single process-group kill.

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd -P)"
PROJECT="$(cd "$SCRIPT_DIR/.." && pwd -P)"
DEV_BIN="$PROJECT/src-tauri/target/debug/preelock"
DEV_CONFIG="$PROJECT/src-tauri/tauri.dev.conf.json"
DEV_DATA_DIR="$HOME/Library/Application Support/com.preelock.desktop.dev"
LIFECYCLE_FILE="$DEV_DATA_DIR/preelock.lifecycle.json"
LOG_DIR="$HOME/Library/Logs/Preelock"
LOG_FILE="$LOG_DIR/preelock-dev.log"
NPM_LOG="$LOG_DIR/preelock-dev-npm.log"
PID_FILE="$LOG_DIR/preelock-dev-launcher.pid"
PORT=1420
GUI_WAIT_SECS=90
DIRECT_GUI_WAIT_SECS=30
SERVER_WAIT_SECS=30
QUIT_GRACE_CHECKS=12   # 12 x 5s = 60s of continuous absence = full quit
                        # (tolerates Rust rebuild restarts).
DEV_PID=""              # Set when this launcher starts the dev workflow;
                        # empty when it adopts an already-running server.

# AppleScript's `do shell script` runs with a minimal PATH that does not
# include node/npm or cargo. Verified on this Mac: node + npm live in
# /usr/local/bin, cargo/rustc in ~/.cargo/bin, Homebrew in /opt/homebrew/bin.
PATH="/usr/local/bin:/opt/homebrew/bin:$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin:$PATH"
export PATH

log() {
    printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S')" "$*" >> "$LOG_FILE"
}

mkdir -p "$LOG_DIR" || exit 1
for f in "$LOG_FILE" "$NPM_LOG"; do
    if [ -f "$f" ]; then
        log_size=$(stat -f%z "$f" 2>/dev/null || echo 0)
        if [ "$log_size" -gt 1048576 ]; then
            mv -f "$f" "$f.old" 2>/dev/null || true
        fi
    fi
done

# Print the pid of the running dev GUI, or nothing.
# The GUI is the BARE binary (no arguments). The Tauri CLI runs it as the
# RELATIVE path `target/debug/preelock` (cargo run), so an exact match
# against the absolute path never fires — match the bare invocation in
# either form. The background engine (--background) and the tray process
# (--tray) take arguments, so they never match.
gui_pid() {
    local pid args
    while read -r pid args; do
        case "$args" in
            "target/debug/preelock"|"./target/debug/preelock"|"$DEV_BIN")
                echo "$pid"
                return 0
                ;;
        esac
    done < <(ps -ax -o pid=,args= 2>/dev/null)
    return 1
}

# True if ANY dev-build process is alive: GUI, --background engine, or
# --tray process. All three share the same binary path.
any_dev_process() {
    pgrep -f "target/debug/preelock" >/dev/null 2>&1
}

# True if a dev workflow for THIS project is alive: the Tauri CLI run by
# `npm run desktop -- --config <dev config>` (or its sh wrapper).
dev_workflow_alive() {
    pgrep -f "tauri dev --config $DEV_CONFIG" >/dev/null 2>&1
}

# True if ANOTHER launcher instance (not this one) is alive.
other_launcher_alive() {
    local lp la
    [ -f "$PID_FILE" ] || return 1
    lp=$(cat "$PID_FILE" 2>/dev/null || true)
    [ -n "$lp" ] || return 1
    [ "$lp" != "$$" ] || return 1
    kill -0 "$lp" 2>/dev/null || return 1
    la=$(ps -o args= -p "$lp" 2>/dev/null || true)
    case "$la" in
        *preelock-dev-launcher.sh*) return 0 ;;
    esac
    return 1
}

# True if the dev server on port 1420 is owned by a live launcher or dev
# workflow for this project — i.e. it must NOT be killed.
workflow_owns_port() {
    other_launcher_alive && return 0
    dev_workflow_alive
}

# Kill a stale listener only when its command or working directory belongs
# to this project. A foreign process on the dev port is never terminated.
kill_port_listener() {
    local pids pid args cwd
    pids=$(lsof -ti:$PORT 2>/dev/null || true)
    if [ -n "$pids" ]; then
        if workflow_owns_port; then
            log "Port $PORT busy but owned by a live launcher/workflow for this project; NOT killing."
            return 1
        fi
        for pid in $pids; do
            args=$(ps -o args= -p "$pid" 2>/dev/null || true)
            cwd=$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')
            case "$args $cwd" in
                *"$PROJECT"*) ;;
                *)
                    log "Port $PORT is occupied by a process outside this project (pid $pid); NOT killing."
                    return 1
                    ;;
            esac
        done
        log "Port $PORT busy (pid(s): $(echo "$pids" | tr '\n' ' ')); stopping stale project listener(s)."
        for pid in $pids; do
            kill -TERM "$pid" 2>/dev/null || true
        done
        sleep 1
    fi
    return 0
}

# True if this project's Vite dev server is up on 1420: a listener whose
# command line belongs to this project, or the dev workflow process alive
# (server starting up — the direct-launch path waits for it to answer).
dev_server_up() {
    local pids pid args
    pids=$(lsof -ti:$PORT 2>/dev/null || true)
    if [ -n "$pids" ]; then
        # shellcheck disable=SC2086
        for pid in $pids; do
            args=$(ps -o args= -p "$pid" 2>/dev/null || true)
            case "$args" in
                *"$PROJECT"*) return 0 ;;
            esac
        done
    fi
    dev_workflow_alive
}

# Wait until the dev server answers HTTP on 1420 (up to 30s). The response
# must be this project's dev page (it references /src/main.tsx) so a
# foreign server squatting on 1420 never counts as "up".
wait_for_dev_server() {
    local waited=0 body
    while [ "$waited" -lt "$SERVER_WAIT_SECS" ]; do
        body=$(curl -fs --max-time 2 "http://127.0.0.1:$PORT/" 2>/dev/null || true)
        case "$body" in
            *"/src/main.tsx"*) return 0 ;;
        esac
        sleep 2
        waited=$((waited + 2))
    done
    return 1
}

# Launch the GUI directly against the already-running dev server — no
# second `npm run desktop`. The dev identity comes from TAURI_CONFIG, the
# same merge mechanism `tauri dev --config` uses: the CLI sets TAURI_CONFIG
# to the dev config JSON and the runtime merges it over the embedded
# config. PREELOCK_DATA_DIR is still required (see header).
launch_gui_direct() {
    local config_json
    config_json=$(cat "$DEV_CONFIG" 2>/dev/null || true)
    if [ -z "$config_json" ]; then
        log "ERROR: cannot read dev config: $DEV_CONFIG"
        return 1
    fi
    log "Dev server already up; launching the GUI directly (no second server)."
    env TAURI_CONFIG="$config_json" PREELOCK_DATA_DIR="$DEV_DATA_DIR" nohup "$DEV_BIN" >> "$NPM_LOG" 2>&1 &
    return 0
}

# True if the dev lifecycle file says no session is active. A missing or
# unreadable file counts as unknown — NOT safe to kill (fail closed).
lifecycle_no_active_session() {
    [ -f "$LIFECYCLE_FILE" ] && grep -Eq '"active_session"[[:space:]]*:[[:space:]]*null' "$LIFECYCLE_FILE" 2>/dev/null
}

# Stop orphaned dev processes (background/tray left behind by a dead
# workflow). Only ever matches the dev binary path — never the release
# copy in /Applications.
stop_orphan_dev_processes() {
    local pid args
    log "Stopping orphaned dev processes (no GUI, no dev server, no active session)."
    while read -r pid args; do
        case "$args" in
            *"target/debug/preelock"*)
                log "  stopping orphan dev process pid $pid: $args"
                kill -TERM "$pid" 2>/dev/null || true
                ;;
        esac
    done < <(ps -ax -o pid=,args= 2>/dev/null)
    sleep 2
    while read -r pid args; do
        case "$args" in
            *"target/debug/preelock"*)
                kill -9 "$pid" 2>/dev/null || true
                ;;
        esac
    done < <(ps -ax -o pid=,args= 2>/dev/null)
}

# Only remove the pidfile if it still holds OUR pid — another launcher may
# have taken over since we wrote it.
release_pidfile() {
    if [ "$(cat "$PID_FILE" 2>/dev/null || true)" = "$$" ]; then
        rm -f "$PID_FILE"
    fi
}

log "--- Preelock Dev launcher started (pid $$) ---"

# 1. Dev GUI already running? Just bring it forward and exit. Any previous
#    launcher keeps monitoring the session, so leave it alone.
if existing_pid=$(gui_pid); then
    log "Dev GUI already running (pid $existing_pid); bringing it to the front."
    if osascript -e "tell application \"System Events\" to set frontmost of (first process whose unix id is $existing_pid) to true" 2>/dev/null; then
        log "Activated existing GUI."
    else
        log "Note: could not bring the GUI forward automatically (first run may need the System Events permission — allow it if macOS asks). The app is still running; click its Dock icon."
    fi
    log "Done."
    exit 0
fi

# 2. Note (never stop) any other launcher instance. It keeps monitoring its
#    session; this instance must never shut down its dev server.
OLD_LAUNCHER=0
if other_launcher_alive; then
    OLD_LAUNCHER=1
    log "Another launcher is already running (pid $(cat "$PID_FILE" 2>/dev/null)); leaving it alone — it keeps monitoring its session."
fi

NEED_FRESH_START=1

# 3. Dev server already up? Reconnect to it instead of starting a second
#    one: launch the GUI directly against the live server. This is the
#    window-close relaunch path (background alive, server up, no GUI).
if dev_server_up; then
    if wait_for_dev_server; then
        if launch_gui_direct; then
            waited=0
            while [ "$waited" -lt "$DIRECT_GUI_WAIT_SECS" ]; do
                if gui_pid >/dev/null 2>&1; then
                    log "Dev GUI is up (direct launch)."
                    break
                fi
                sleep 2
                waited=$((waited + 2))
            done
            if gui_pid >/dev/null 2>&1; then
                if [ "$OLD_LAUNCHER" = "1" ]; then
                    log "Done — the other launcher keeps monitoring this session."
                    exit 0
                fi
                # No launcher was monitoring this server: adopt the session.
                echo "$$" > "$PID_FILE"
                log "Adopted the running dev server (no launcher was monitoring it); monitoring from here."
                NEED_FRESH_START=0
            else
                log "ERROR: GUI did not appear within ${DIRECT_GUI_WAIT_SECS}s of the direct launch. Dev server left running; check $NPM_LOG"
                exit 1
            fi
        else
            exit 1
        fi
    else
        log "ERROR: dev server did not answer on port $PORT within ${SERVER_WAIT_SECS}s. Leaving it alone; check $NPM_LOG"
        exit 1
    fi
fi

# 4. Fresh start: no GUI and no dev server. Sanity checks first.
if [ "$NEED_FRESH_START" = "1" ]; then
    if [ ! -f "$DEV_CONFIG" ]; then
        log "ERROR: dev config not found: $DEV_CONFIG"
        release_pidfile
        exit 1
    fi
    if ! command -v npm >/dev/null 2>&1; then
        log "ERROR: npm not found on PATH ($PATH)"
        release_pidfile
        exit 1
    fi

    # 5. Clear a stale dev server — but ONLY when no launcher/workflow owns
    #    the port. If the port is owned, abort rather than disturb a live
    #    session.
    if ! kill_port_listener; then
        log "ERROR: cannot start a fresh dev server while port $PORT is owned by a live launcher/workflow. Leaving everything alone."
        release_pidfile
        exit 1
    fi

    # 6. Orphaned dev processes from an earlier unclean exit (background or
    #    tray alive, no GUI, no server)? Clear them for a clean start —
    #    but only when the dev lifecycle file shows no active session. A
    #    live session is never interrupted: the new GUI will take over
    #    its background engine instead.
    if any_dev_process; then
        if lifecycle_no_active_session; then
            stop_orphan_dev_processes
        else
            log "Dev processes are alive with an ACTIVE session; leaving them alone — the new GUI will take over the background."
        fi
    fi

    echo "$$" > "$PID_FILE"
    export PREELOCK_DATA_DIR="$DEV_DATA_DIR"
    log "Starting dev workflow: npm run desktop -- --config $DEV_CONFIG"
    log "Data dir: $DEV_DATA_DIR"
    cd "$PROJECT" || { log "ERROR: cannot cd to $PROJECT"; release_pidfile; exit 1; }
    nohup npm run desktop -- --config "$DEV_CONFIG" >> "$NPM_LOG" 2>&1 &
    DEV_PID=$!
    log "Dev workflow started (pid $DEV_PID). Waiting for the GUI (up to ${GUI_WAIT_SECS}s)..."

    # 7. Wait for the GUI to appear.
    waited=0
    while [ "$waited" -lt "$GUI_WAIT_SECS" ]; do
        if gui_pid >/dev/null 2>&1; then
            log "Dev GUI is up."
            break
        fi
        if ! kill -0 "$DEV_PID" 2>/dev/null; then
            # One last check: the GUI may have appeared just as the workflow exited.
            if gui_pid >/dev/null 2>&1; then
                log "Dev workflow exited but the GUI is up; monitoring from here."
                break
            fi
            log "ERROR: dev workflow (pid $DEV_PID) exited before the GUI appeared. See $NPM_LOG"
            release_pidfile
            exit 1
        fi
        sleep 2
        waited=$((waited + 2))
    done
    if ! gui_pid >/dev/null 2>&1; then
        log "ERROR: GUI did not appear within ${GUI_WAIT_SECS}s. The dev workflow (pid $DEV_PID) is still running; check $NPM_LOG"
        release_pidfile
        exit 1
    fi
fi

# 8. Monitor until a FULL quit: no dev-build process left at all (GUI,
#    background engine and tray process all gone). Closing just the window
#    does not stop the server — the background stays alive by design.
#    A short absence (Rust rebuild restart) must not end the session, so
#    only 60s of continuous absence counts as a quit.
log "Monitoring: the dev server stops only on a full quit (no Preelock Dev process left)."
absent=0
while true; do
    sleep 5
    if any_dev_process; then
        absent=0
    else
        absent=$((absent + 1))
        if [ "$absent" -ge "$QUIT_GRACE_CHECKS" ]; then
            log "No Preelock Dev process for 60s — treating as a full quit."
            break
        fi
    fi
done

# 9. Stop the dev server: the whole process group first (only when this
#    launcher started the workflow), then anything still listening on the
#    dev port as a safety net — gated, so a live session's server is never
#    killed.
log "Stopping the dev server (workflow pid ${DEV_PID:-none})..."
if [ -n "$DEV_PID" ] && kill -0 "$DEV_PID" 2>/dev/null; then
    dev_pgid=$(ps -o pgid= -p "$DEV_PID" 2>/dev/null | tr -d ' ' || true)
    if [ -n "$dev_pgid" ] && [ "$dev_pgid" = "$DEV_PID" ]; then
        kill -TERM -- "-$DEV_PID" 2>/dev/null || true
    else
        kill -TERM "$DEV_PID" 2>/dev/null || true
    fi
    sleep 3
fi
kill_port_listener || true
release_pidfile
log "Preelock Dev stopped."
exit 0
