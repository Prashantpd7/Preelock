#!/usr/bin/env python3
"""Production acceptance harness for the Preelock macOS lifecycle.

Reproduces the reported defect's preconditions against an INSTALLED, signed
``Preelock.app`` (never a dev build) and asserts the invariant that made the
ghost Dock icon possible:

    While the GUI is not running, LaunchServices must have NO application
    entry for com.preelock.desktop.

Before the fix the headless ``--background`` engine registered itself as a
second ``com.preelock.desktop`` instance (``lsappinfo``: BackgroundOnly). A
notification click then activated that windowless daemon instead of the GUI,
producing "The application 'Preelock' is not open anymore" and a ghost Dock
icon that had to be Force Quit.

Usage:
    python3 scripts/acceptance-macos-notification.py /Applications/Preelock.app
    python3 scripts/acceptance-macos-notification.py /Applications/Preelock.app --cycles 10
    python3 scripts/acceptance-macos-notification.py /Applications/Preelock.app --manual-click

The real notification CLICK cannot be automated without Accessibility
permission for System Events. Use ``--manual-click`` and click the banner when
prompted; the script then verifies process/Dock/window state around your click.

Every result is labelled with how it was verified. A skipped check is printed
as SKIP, never as PASS.
"""
from __future__ import annotations

import argparse
import json
import os
import plistlib
import re
import signal
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

BUNDLE_ID = "com.preelock.desktop"
DATA_DIR = Path.home() / "Library/Application Support" / BUNDLE_ID
LOG_FILE = Path.home() / "Library/Logs/Preelock/lifecycle.log"
GUI_LOCK = DATA_DIR / "gui.lock"


@dataclass
class Result:
    name: str
    status: str  # PASS / FAIL / SKIP
    detail: str


@dataclass
class Report:
    results: list[Result] = field(default_factory=list)

    def add(self, name: str, ok: bool | None, detail: str) -> None:
        status = "SKIP" if ok is None else ("PASS" if ok else "FAIL")
        self.results.append(Result(name, status, detail))
        print(f"  [{status}] {name}: {detail}")

    @property
    def failed(self) -> list[Result]:
        return [r for r in self.results if r.status == "FAIL"]


def run(cmd: list[str], timeout: float = 30.0) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, capture_output=True, text=True, timeout=timeout, check=False)


def bundle_processes(binary: Path) -> list[tuple[int, str]]:
    out = run(["ps", "-Ao", "pid=,command="]).stdout
    found = []
    for line in out.splitlines():
        line = line.strip()
        if not line:
            continue
        pid, _, command = line.partition(" ")
        if command == str(binary) or command.startswith(str(binary) + " "):
            found.append((int(pid), command))
    return found


def gui_processes(binary: Path) -> list[int]:
    return [pid for pid, command in bundle_processes(binary) if "--background" not in command
            and "--tray" not in command and "--attention" not in command]


def lsapp_entries() -> list[dict]:
    out = run(["lsappinfo", "list"], timeout=20).stdout
    entries = []
    current: dict = {}
    for raw in out.splitlines():
        line = raw.strip()
        match = re.match(r'^\d+\) "(.*)" ASN:', line)
        if match:
            if current.get("bundle_id") == BUNDLE_ID:
                entries.append(current)
            current = {"name": match.group(1)}
            continue
        if 'bundleID="' in line:
            current["bundle_id"] = line.split('bundleID="', 1)[1].split('"', 1)[0]
        elif re.search(r"\bpid = \d", line):
            tail = line.split("pid = ", 1)[1]
            current["pid"] = int(tail.split()[0])
            type_match = re.search(r'type="([^"]+)"', line)
            current["type"] = type_match.group(1) if type_match else "?"
        elif "bundle path=" in line:
            current["bundle_path"] = line.split("bundle path=", 1)[1].strip().strip('"')
    if current.get("bundle_id") == BUNDLE_ID:
        entries.append(current)
    return entries


def lock_pid() -> int | None:
    try:
        return int(GUI_LOCK.read_text().strip())
    except (OSError, ValueError):
        return None


def pid_alive(pid: int | None) -> bool:
    if not pid:
        return False
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False
    except PermissionError:
        return True


def window_count() -> int | None:
    """Windows owned by the Preelock app. None when Accessibility is not granted."""
    script = (
        'tell application "System Events" to tell process "Preelock" '
        'to return count of windows'
    )
    proc = run(["osascript", "-e", script], timeout=15)
    if proc.returncode != 0:
        return None
    try:
        return int(proc.stdout.strip())
    except ValueError:
        return None


def wait_until(predicate, timeout: float, interval: float = 0.25):
    deadline = time.time() + timeout
    while time.time() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(interval)
    return None


def quit_gui(timeout: float = 20.0) -> subprocess.CompletedProcess:
    # Exactly the Cmd+Q path: an AppKit quit Apple Event to the application.
    return run(
        ["osascript", "-e", f'with timeout of {int(timeout)} seconds',
         "-e", f'tell application id "{BUNDLE_ID}" to quit', "-e", "end timeout"],
        timeout=timeout + 15,
    )


def stop_everything(binary: Path) -> None:
    for pid, _ in bundle_processes(binary):
        try:
            os.kill(pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    wait_until(lambda: not bundle_processes(binary), 10)


def log_tail(lines: int = 12) -> str:
    try:
        return "\n".join(LOG_FILE.read_text(errors="replace").splitlines()[-lines:])
    except OSError:
        return "(no lifecycle log)"


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("app", type=Path, help="installed Preelock.app")
    parser.add_argument("--cycles", type=int, default=10, help="launch/quit cycles to run")
    parser.add_argument("--manual-click", action="store_true",
                        help="pause for a real notification click and verify it")
    parser.add_argument("--report", type=Path, default=None)
    args = parser.parse_args()

    app = args.app.resolve()
    binary = app / "Contents/MacOS/preelock"
    if not binary.is_file():
        print(f"Not a Preelock bundle: {app}", file=sys.stderr)
        return 2
    info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
    cli = [str(binary)]
    report = Report()

    print(f"App:        {app}")
    print(f"Version:    {info.get('CFBundleShortVersionString')}")
    print(f"Bundle id:  {info.get('CFBundleIdentifier')}")
    codesign = run(["codesign", "-dv", str(app)])
    print(f"Signature:  {codesign.stderr.strip().splitlines()[0] if codesign.stderr else '?'}")
    print()

    if info.get("CFBundleIdentifier") != BUNDLE_ID:
        report.add("bundle identifier", False, str(info.get("CFBundleIdentifier")))
        return 1

    status = run(cli + ["--status"], timeout=20)
    if "Live session: none" not in status.stdout:
        print("Refusing to run: a session is live (this harness must not disturb it).")
        return 3

    # --- Cycle 0: the reported sequence -----------------------------------
    print("== Reported sequence: notification -> Command+Q -> click -> GUI ==")
    stop_everything(binary)
    launch = run(["open", "-a", str(app)], timeout=20)
    gui_ready = wait_until(lambda: pid_alive(lock_pid()) and gui_processes(binary), 30)
    report.add("launch from Finder/`open`", bool(gui_ready), launch.stderr.strip() or "GUI up")
    report.add("exactly one GUI process", len(gui_processes(binary)) == 1,
               f"{len(gui_processes(binary))} GUI process(es)")
    report.add("gui.lock owned by the live GUI", lock_pid() in gui_processes(binary),
               f"lock pid={lock_pid()}")

    notify = run(cli + ["--test-notify", "Preelock lifecycle acceptance",
                        "Click to verify the installed build"], timeout=20)
    report.add("production notification dispatch", notify.returncode == 0,
               notify.stdout.strip() or notify.stderr.strip())

    quit_result = quit_gui()
    gui_gone = wait_until(lambda: not gui_processes(binary), 20)
    # The quit guard answers the Apple Event with NSTerminateCancel and then
    # terminates through the authoritative quit path, so `osascript` reports
    # -128 "User cancelled" even when the GUI exits. Success is the GUI being
    # gone; the reply code is reported as detail, not as the verdict.
    report.add("Command+Q quits the GUI", bool(gui_gone),
               f"gui_exited={bool(gui_gone)} apple_event_reply={quit_result.returncode}")
    if quit_result.stderr.strip():
        print(f"      (quit reply: {quit_result.stderr.strip()})")

    # THE regression invariant.
    entries = lsapp_entries()
    foreground = [e for e in entries if e.get("type") == "Foreground"]
    background = [e for e in entries if e.get("type") == "BackgroundOnly"]
    report.add("no LaunchServices identity while GUI is down", len(entries) == 0,
               f"entries={json.dumps(entries)}")
    report.add("no BackgroundOnly daemon masquerading as the app", not background,
               f"background_only={len(background)}")
    report.add("no ghost Foreground app with no window", not foreground,
               f"foreground={len(foreground)}")

    # The same request a notification click makes.
    print("  (clicking the notification is the same coordinator call a click makes)")
    open_gui = run(cli + ["--open-gui"], timeout=30)
    relaunched = wait_until(lambda: gui_processes(binary) and pid_alive(lock_pid()), 30)
    report.add("cold GUI launch through the coordinator",
               bool(relaunched) and open_gui.returncode == 0,
               open_gui.stdout.strip() or open_gui.stderr.strip())
    report.add("exactly one GUI after the click path", len(gui_processes(binary)) == 1,
               f"{len(gui_processes(binary))} GUI process(es)")
    entries = lsapp_entries()
    report.add("one Foreground Dock identity, no BackgroundOnly",
               len([e for e in entries if e.get("type") == "Foreground"]) == 1
               and not [e for e in entries if e.get("type") == "BackgroundOnly"],
               json.dumps(entries))
    windows = window_count()
    report.add("functional window presented", None if windows is None else windows >= 1,
               "Accessibility not granted; verify visually" if windows is None
               else f"{windows} window(s)")
    if open_gui.returncode == 0:
        quit_result = quit_gui()
        report.add("cleanup quit", wait_until(lambda: not gui_processes(binary), 20) is not None,
                   quit_result.stderr.strip() or "quit accepted")

    # --- Repeated cycles --------------------------------------------------
    print(f"\n== {args.cycles} launch/quit cycles ==")
    failures = 0
    for index in range(1, args.cycles + 1):
        run(["open", "-a", str(app)], timeout=20)
        up = wait_until(lambda: gui_processes(binary) and lock_pid() in gui_processes(binary), 30)
        single = len(gui_processes(binary)) == 1
        quit_result = quit_gui()
        down = wait_until(lambda: not gui_processes(binary), 20)
        residual = lsapp_entries()
        ok = bool(up) and single and bool(down) and not residual
        failures += 0 if ok else 1
        print(f"  cycle {index:02d}: {'PASS' if ok else 'FAIL'} "
              f"(up={bool(up)} single={single} down={bool(down)} "
              f"ls={len(residual)} quit_rc={quit_result.returncode})")
    report.add(f"{args.cycles} consecutive launch/quit cycles", failures == 0,
               f"{failures} failing cycle(s)")

    # --- Manual click -----------------------------------------------------
    if args.manual_click:
        print("\n== Manual notification click ==")
        run(["open", "-a", str(app)], timeout=20)
        wait_until(lambda: gui_processes(binary), 30)
        before = sorted(gui_processes(binary))
        run(cli + ["--test-notify", "Preelock click test",
                   "Click this notification now"], timeout=20)
        quit_result = quit_gui()
        wait_until(lambda: not gui_processes(binary), 20)
        print("  The GUI is now quit. A Preelock notification should be in "
              "Notification Centre.")
        input("  Click the Preelock notification, then press Enter here... ")
        after = sorted(gui_processes(binary))
        new = [pid for pid in after if pid not in before]
        report.add("click after Command+Q opened the GUI", bool(after),
                   f"GUI pids before={before} after={after}")
        report.add("exactly one GUI after the click", len(after) == 1, f"{after}")
        report.add("no duplicate launch from the click", len(after) <= 1, f"{new}")
        windows = window_count()
        report.add("window visible after the click", None if windows is None else windows >= 1,
                   "Accessibility not granted; verify visually" if windows is None
                   else f"{windows} window(s)")
        entries = lsapp_entries()
        report.add("one Foreground identity after the click",
                   len([e for e in entries if e.get("type") == "Foreground"]) == 1,
                   json.dumps(entries))

    print("\n== Lifecycle log tail ==")
    print(log_tail())

    print("\n== Summary ==")
    for item in report.results:
        print(f"{item.status:4}  {item.name}")
    if args.report:
        args.report.write_text(json.dumps([r.__dict__ for r in report.results], indent=2))
        print(f"\nReport written to {args.report}")

    if report.failed:
        print(f"\n{len(report.failed)} check(s) FAILED.")
        return 1
    print("\nAll executed checks passed. Checks marked SKIP were not verified.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
