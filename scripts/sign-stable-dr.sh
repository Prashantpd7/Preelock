#!/usr/bin/env bash
set -euo pipefail

# Find the built macOS .app bundle
CANDIDATES=(
    "${1:-}"
    "src-tauri/target/release/bundle/macos/Preelock.app"
    "src-tauri/target/aarch64-apple-darwin/release/bundle/macos/Preelock.app"
    "/Applications/Preelock.app"
)

BUNDLE_PATH=""
for path in "${CANDIDATES[@]}"; do
    if [ -n "$path" ] && [ -d "$path" ] && [ -f "$path/Contents/Info.plist" ]; then
        BUNDLE_PATH="$path"
        break
    fi
done

if [ -z "$BUNDLE_PATH" ]; then
    echo "Notice: No macOS .app bundle found to apply designated requirement."
    exit 0
fi

echo "Applying stable designated requirement to $BUNDLE_PATH..."
codesign --force --deep -s - \
    -r='designated => identifier "com.preelock.desktop"' \
    --entitlements src-tauri/Entitlements.plist \
    "$BUNDLE_PATH"

codesign -d -r- "$BUNDLE_PATH"
codesign --verify --deep --strict --verbose=2 "$BUNDLE_PATH"
echo "Stable designated requirement applied successfully: identifier \"com.preelock.desktop\""
