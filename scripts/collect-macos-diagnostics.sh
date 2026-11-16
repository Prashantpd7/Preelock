#!/usr/bin/env bash
# Collect the macOS lifecycle diagnostics needed to explain a Preelock
# activation / Dock / notification problem.
#
#   bash scripts/collect-macos-diagnostics.sh
#   bash scripts/collect-macos-diagnostics.sh /path/to/output-directory
#
# Produces a directory and a .tar.gz on the Desktop. It contains process,
# LaunchServices, LaunchAgent, signature and lifecycle-log state only: the
# Preelock lifecycle log deliberately records identifiers, never notification
# titles/bodies, session titles or other user content.
set -euo pipefail

BUNDLE_ID="com.preelock.desktop"
APP="/Applications/Preelock.app"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="${1:-$HOME/Desktop/preelock-diagnostics-$STAMP}"

mkdir -p "$OUT"

{
  echo "collected_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "--- system ---"
  sw_vers
  uname -m
  echo "--- app ---"
  if [ -d "$APP" ]; then
    /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$APP/Contents/Info.plist" 2>/dev/null || true
    codesign -dv --entitlements - "$APP" 2>&1 || true
    codesign --verify --deep --strict --verbose=2 "$APP" 2>&1 || true
  else
    echo "missing: $APP"
  fi
} > "$OUT/system-and-signature.txt" 2>&1

# LaunchServices identities are the core evidence: a headless Preelock process
# must never appear here while the GUI is closed.
lsappinfo list > "$OUT/launchservices.txt" 2>&1 || true
lsappinfo list | grep -A6 "bundleID=\"$BUNDLE_ID\"" > "$OUT/launchservices-preelock.txt" 2>&1 || true

ps -Ao pid,ppid,command | grep -i preelock | grep -v grep > "$OUT/processes.txt" 2>&1 || true

launchctl print "gui/$(id -u)/com.preelock.background" > "$OUT/launchagent-background.txt" 2>&1 || true
[ -f "$HOME/Library/LaunchAgents/com.preelock.background.plist" ] &&
  cp "$HOME/Library/LaunchAgents/com.preelock.background.plist" "$OUT/" 2>/dev/null || true

for log in \
  "$HOME/Library/Logs/Preelock/lifecycle.log" \
  "$HOME/Library/Logs/Preelock/lifecycle.log.1" \
  "$HOME/Library/Logs/com.preelock.background.stderr.log" \
  "$HOME/Library/Logs/com.preelock.background.stdout.log"; do
  [ -f "$log" ] && cp "$log" "$OUT/" 2>/dev/null || true
done

# Unified log, restricted to this app's lifecycle prefix.
log show --last 1h --style compact \
  --predicate 'eventMessage CONTAINS "preelock-lifecycle"' \
  > "$OUT/unified-log.txt" 2>&1 || true

tar -czf "$OUT.tar.gz" -C "$(dirname "$OUT")" "$(basename "$OUT")"
echo "Diagnostics written to:"
echo "  $OUT"
echo "  $OUT.tar.gz"
