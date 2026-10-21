"""Runner-only smoke test of the production bundle copied outside the checkout.

Tests startup/database persistence, not interactive permissions or Gatekeeper.
Never runs against an existing user's database.
"""
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import time

assert os.environ.get("GITHUB_ACTIONS") == "true", "Only run on disposable CI runners"
app = Path(sys.argv[1]).resolve()
database = Path.home() / "Library/Application Support/com.preelock.desktop/preelock.sqlite"
assert not database.exists(), "Refusing to test against existing app data"


def launch():
    process = subprocess.Popen([str(app / "Contents/MacOS/preelock")])
    try:
        for _ in range(30):
            assert process.poll() is None, "Installed app exited during startup"
            if database.exists():
                with sqlite3.connect(database) as conn:
                    if conn.execute("PRAGMA user_version").fetchone()[0] == 4:
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
