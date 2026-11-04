#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# Package and sign Tetherly.app on macOS with stable Designated Requirement (DR).
# Ensures TCC Accessibility permissions are preserved across recompiles and updates.
#
# SECURITY TRADE-OFF (deliberate, read before distributing):
# The DR below anchors only on the bundle identifier. That is what makes TCC
# honor the Accessibility grant across recompiles, but it also means ANY
# locally-built binary claiming the same identifier and ad-hoc signature
# satisfies the same DR and inherits the granted Accessibility (global
# keyboard/mouse capture) permission. This bundle is ad-hoc signed without
# Hardened Runtime or notarization: it is intended for THIS machine only and
# must NOT be distributed. To distribute, switch to Developer ID signing:
#   -r="designated => anchor apple generic and identifier \"$BUNDLE_ID\" and certificate leaf[subject.OU] = \"<TEAMID>\""

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_DIR="/Applications/Tetherly.app"
BUNDLE_ID="dev.tetherly.tetherly"
RESTART=false
BINARY_PATH=""
FROM_TAG=""
FROM_RUN=""
BUILD_LOCAL=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --restart)
            RESTART=true
            shift
            ;;
        --binary)
            BINARY_PATH="$2"
            shift 2
            ;;
        --tag|--from-tag)
            FROM_TAG="$2"
            shift 2
            ;;
        --run-id|--from-run)
            FROM_RUN="$2"
            shift 2
            ;;
        --local)
            BUILD_LOCAL=true
            shift
            ;;
        -h|--help)
            echo "Usage: $0 [--restart] [--binary <path>] [--tag <vX.Y.Z>] [--run-id <id>] [--local]"
            echo ""
            echo "Options:"
            echo "  --restart        Gracefully stop running daemon and restart via LaunchServices after install"
            echo "  --binary <path>  Install specified pre-built binary (e.g. from GitHub CI)"
            echo "  --tag <vX.Y.Z>   Download and install binary from GitHub Release tag (default: latest)"
            echo "  --run-id <id>    Download and install artifact from GitHub Actions workflow run"
            echo "  --local          Compile release binary locally via cargo (discouraged; CI is preferred)"
            exit 0
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

TMP_CLEANUP=""
cleanup() {
    if [ -n "$TMP_CLEANUP" ] && [ -d "$TMP_CLEANUP" ]; then
        rm -rf "$TMP_CLEANUP"
    fi
}
trap cleanup EXIT

echo "==> [1/4] Acquiring release binary..."
cd "$REPO_ROOT"

if [ -n "$BINARY_PATH" ]; then
    if [ ! -f "$BINARY_PATH" ]; then
        echo "Error: specified binary not found at $BINARY_PATH" >&2
        exit 1
    fi
    echo "==> Using provided binary: $BINARY_PATH"
elif [ -n "$FROM_RUN" ]; then
    echo "==> Downloading artifact from GitHub Actions run $FROM_RUN..."
    TMP_CLEANUP=$(mktemp -d /tmp/tetherly-ci-XXXXXX)
    gh run download "$FROM_RUN" -n "tetherly-macos-arm64" -D "$TMP_CLEANUP"
    BINARY_PATH="$TMP_CLEANUP/tetherly-macos-arm64"
elif [ -n "$FROM_TAG" ]; then
    echo "==> Downloading artifact from GitHub Release tag $FROM_TAG..."
    TMP_CLEANUP=$(mktemp -d /tmp/tetherly-ci-XXXXXX)
    gh release download "$FROM_TAG" -p "tetherly-macos-arm64" -D "$TMP_CLEANUP"
    BINARY_PATH="$TMP_CLEANUP/tetherly-macos-arm64"
elif [ "$BUILD_LOCAL" = true ]; then
    echo "==> Building release binary locally via cargo..."
    cargo build --release -p tetherly-node -p tetherly-cli
    BINARY_PATH="$REPO_ROOT/target/release/tetherly"
else
    # Default: attempt to download latest release artifact from GitHub CI
    LATEST_TAG=$(gh release view --json tagName -q .tagName 2>/dev/null || true)
    if [ -n "$LATEST_TAG" ]; then
        echo "==> No binary specified, downloading latest CI release ($LATEST_TAG)..."
        TMP_CLEANUP=$(mktemp -d /tmp/tetherly-ci-XXXXXX)
        gh release download "$LATEST_TAG" -p "tetherly-macos-arm64" -D "$TMP_CLEANUP" 2>/dev/null || true
        if [ -f "$TMP_CLEANUP/tetherly-macos-arm64" ]; then
            BINARY_PATH="$TMP_CLEANUP/tetherly-macos-arm64"
        fi
    fi
    if [ -z "$BINARY_PATH" ] || [ ! -f "$BINARY_PATH" ]; then
        # Fallback to local build if offline or no release exists
        echo "==> Could not fetch from CI; falling back to local cargo build..."
        cargo build --release -p tetherly-node -p tetherly-cli
        BINARY_PATH="$REPO_ROOT/target/release/tetherly"
    fi
fi

if [ "$RESTART" = true ]; then
    # Shut the previous daemon down BEFORE overwriting its binary: cp opens
    # the destination with O_TRUNC, and truncating a running (signed) image
    # can get the live process SIGKILLed by code-signing validation. Wait
    # for the port to be released so `open` cannot race a second instance.
    echo "==> Stopping previous daemon before replacing its binary..."
    curl -s -X POST http://127.0.0.1:45716/api/daemon/shutdown >/dev/null 2>&1 || true
    for _ in $(seq 1 20); do
        pgrep -f "Tetherly.app/Contents/MacOS/Tetherly" >/dev/null 2>&1 || break
        sleep 0.25
    done
fi

# Keep the bundle version in sync with the workspace version instead of a
# hardcoded string that drifts every release.
WORKSPACE_VERSION=$(cargo metadata --no-deps --format-version 1 2>/dev/null \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])' \
    2>/dev/null || grep -m1 '^version' "$REPO_ROOT/Cargo.toml" | sed 's/.*"\(.*\)".*/\1/')
echo "==> Workspace version: ${WORKSPACE_VERSION}"

echo "==> [2/4] Assembling /Applications/Tetherly.app bundle..."
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/Tetherly"
chmod +x "$APP_DIR/Contents/MacOS/Tetherly"

cat << EOF > "$APP_DIR/Contents/Info.plist"
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
    <string>${WORKSPACE_VERSION}</string>
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
    open "$APP_DIR"
    sleep 1
    echo "==> Verifying accessibility status:"
    curl -s -X POST http://127.0.0.1:45716/api/input/ensure | jq . || true
else
    echo "==> [4/4] Package complete: $APP_DIR is ready."
fi
