#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Package and sign Tetherly.app on macOS with stable Designated Requirement (DR).
# Ensures TCC Accessibility permissions are preserved across recompiles and updates.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="/Applications/Tetherly.app"
BUNDLE_ID="dev.tetherly.tetherly"
RESTART=false

for arg in "$@"; do
    case "$arg" in
        --restart)
            RESTART=true
            ;;
        *)
            ;;
    esac
done

echo "==> [1/4] Building release binary..."
cd "$REPO_ROOT"
cargo build --release -p tetherly-node -p tetherly-cli

echo "==> [2/4] Assembling /Applications/Tetherly.app bundle..."
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$REPO_ROOT/target/release/tetherly" "$APP_DIR/Contents/MacOS/Tetherly"
chmod +x "$APP_DIR/Contents/MacOS/Tetherly"

cat << 'EOF' > "$APP_DIR/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>Tetherly</string>
    <key>CFBundleIdentifier</key>
    <string>dev.tetherly.tetherly</string>
    <key>CFBundleName</key>
    <string>Tetherly</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.67</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>LSUIElement</key>
    <true/>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

echo "==> [3/4] Signing bundle with stable Designated Requirement..."
# Remove old ad-hoc signatures / test artifacts inside bundle
find "$APP_DIR" -name ".DS_Store" -delete 2>/dev/null || true
rm -rf "$APP_DIR/Contents/_CodeSignature" 2>/dev/null || true

# Sign binary and bundle with explicit identifier and designated requirement
codesign -f -s - \
    --identifier "$BUNDLE_ID" \
    -r="designated => identifier \"$BUNDLE_ID\"" \
    "$APP_DIR/Contents/MacOS/Tetherly"

codesign -f -s - \
    --identifier "$BUNDLE_ID" \
    -r="designated => identifier \"$BUNDLE_ID\"" \
    "$APP_DIR"

echo "==> Verifying signature and DR:"
codesign -d -r- "$APP_DIR"

if [ "$RESTART" = true ]; then
    echo "==> [4/4] Restarting via LaunchServices (open $APP_DIR)..."
    # Gracefully shut down previous daemon via UI API if running
    curl -s -X POST http://127.0.0.1:45716/api/daemon/shutdown >/dev/null 2>&1 || true
    sleep 1
    open "$APP_DIR"
    sleep 1
    echo "==> Verifying accessibility status:"
    curl -s -X POST http://127.0.0.1:45716/api/input/ensure | jq . || true
else
    echo "==> [4/4] Package complete: $APP_DIR is ready."
fi
