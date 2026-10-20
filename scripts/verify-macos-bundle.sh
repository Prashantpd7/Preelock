#!/bin/bash
set -euo pipefail

app="${1:?Usage: verify-macos-bundle.sh APP [arm64|x86_64]}"
codesign --verify --deep --strict --verbose=2 "$app"
test -x "$app/Contents/MacOS/vystriq"
if [[ -n "${2:-}" ]]; then
  [[ "$(lipo -archs "$app/Contents/MacOS/vystriq")" == "$2" ]]
fi
identifier=$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$app/Contents/Info.plist")
[[ "$identifier" == "com.vystriq.desktop" ]]
version=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$app/Contents/Info.plist")
[[ "$version" == "$(node -p "require('./package.json').version")" ]]
/usr/libexec/PlistBuddy -c 'Print NSCameraUsageDescription' "$app/Contents/Info.plist"
entitlements="$(codesign -d --entitlements :- "$app" 2>/dev/null)"
[[ "$entitlements" == *"com.apple.security.device.camera"* ]]
node --input-type=module - "$app" <<'JS'
import fs from 'node:fs';
import path from 'node:path';
const app = process.argv[2];
const config = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json'));
for (const [source, destination] of Object.entries(config.bundle.resources)) {
  const expected = fs.readFileSync(path.resolve('src-tauri', source));
  const actual = fs.readFileSync(path.join(app, 'Contents/Resources', destination));
  if (!expected.equals(actual)) throw new Error(`Bundled resource mismatch: ${destination}`);
}
JS
echo "Bundle integrity verified. Ad-hoc signing does not establish Gatekeeper trust."
