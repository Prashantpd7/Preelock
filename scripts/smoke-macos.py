"""Runner-only smoke test of the production bundle copied outside the checkout.

Tests startup/database persistence, not interactive permissions or Gatekeeper.
Never runs against an existing user's database.
"""
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import sys
import time

assert os.environ.get("GITHUB_ACTIONS") == "true", "Only run on disposable CI runners"
app = Path(sys.argv[1]).resolve()
database = Path.home() / "Library/Application Support/com.preelock.desktop/preelock.sqlite"
assert not database.exists(), "Refusing to test against existing app data"


def pid_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def stop_bundle_processes():
    binary = str(app / "Contents/MacOS/preelock")
    plist = Path.home() / "Library/LaunchAgents/com.preelock.background.plist"
    if plist.exists():
        uid = os.getuid()
        subprocess.run(
            ["launchctl", "bootout", f"gui/{uid}/com.preelock.background"],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        subprocess.run(
            ["launchctl", "unload", "-w", str(plist)],
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        plist.unlink(missing_ok=True)
    ps = subprocess.run(
        ["ps", "-axo", "pid=,args="],
        capture_output=True,
        text=True,
        check=False,
    )
    pids = []
    for line in ps.stdout.splitlines():
        line = line.strip()
        if not line:
            continue
        parts = line.split(None, 1)
        if len(parts) == 2 and (parts[1] == binary or parts[1].startswith(binary + " ")):
            try:
                pids.append(int(parts[0]))
            except ValueError:
                pass
    for sig in (signal.SIGTERM, signal.SIGKILL):
        remaining = []
        for pid in pids:
            try:
                os.kill(pid, sig)
                remaining.append(pid)
            except ProcessLookupError:
                pass
        if not remaining:
            break
        time.sleep(0.5)
        pids = [pid for pid in remaining if pid_alive(pid)]


def launch():
    process = subprocess.Popen([str(app / "Contents/MacOS/preelock")])
    try:
        for _ in range(30):
            assert process.poll() is None, "Installed app exited during startup"
            if database.exists():
                with sqlite3.connect(database) as conn:
                    if conn.execute("PRAGMA user_version").fetchone()[0] == 10:
                        break
            time.sleep(1)
        else:
            raise AssertionError("Installed app did not initialize its database")
        time.sleep(5)
        assert process.poll() is None, "Installed app crashed after initialization"
    finally:
        process.terminate()
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
        stop_bundle_processes()


launch()
with sqlite3.connect(database) as conn:
    assert conn.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
    assert conn.execute("SELECT COUNT(*) FROM sessions").fetchone()[0] == 0
    assert conn.execute("SELECT COUNT(*) FROM tasks").fetchone()[0] == 0
    preferences = json.loads(conn.execute("SELECT payload FROM preferences WHERE id=1").fetchone()[0])
    # This is a disposable, freshly created CI profile, with the app stopped.
    preferences["theme"] = "light"
    conn.execute("UPDATE preferences SET payload=? WHERE id=1", (json.dumps(preferences),))
launch()
with sqlite3.connect(database) as conn:
    restored = json.loads(conn.execute("SELECT payload FROM preferences WHERE id=1").fetchone()[0])
    assert restored["theme"] == "light", "Persisted setting lost on restart"
print("Installed bundle started twice; fresh database and saved preference survived restart.")
print("No interactive camera, browser pairing, or browser-download approval tested here.")
