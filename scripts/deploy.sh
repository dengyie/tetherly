#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0 OR MIT
# ==============================================================================
# Tetherly CI-Driven Production Deployment Pipeline
# ==============================================================================
# Deploys official release binaries built by GitHub Actions CI to:
#   - macOS Controller (/Applications/Tetherly.app)
#   - Windows Host (home-win via NTFS in-use atomic hot-restart)
#
# INVARIANTS:
#   1. NEVER compiles locally — all binaries are fetched from GitHub CI.
#   2. Windows deployment STRICTLY protects mission-critical production processes:
#      SRBMiner-MULTI.exe, verge-mihomo.exe, komari-agent.exe.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

TARGET="all"
TAG=""
RUN_ID=""
REMOTE_HOST="${REMOTE_HOST:-home-win}"
REMOTE_PORT=45716
LOCAL_PORT=45716
RUN_SMOKE=false
PROTECT_PROCS="SRBMiner-MULTI,verge-mihomo,komari-agent"

usage() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --target <mac|windows|all>   Target host to deploy (default: all)"
    echo "  --tag <vX.Y.Z>               GitHub Release tag to fetch from (default: latest release)"
    echo "  --run-id <ID>                GitHub Actions workflow run ID to fetch artifacts from"
    echo "  --remote-host <host>         Windows SSH host alias (default: home-win)"
    echo "  --smoke                      Run verify_cross_platform_smoke.sh after deploy"
    echo "  -h, --help                   Show this help message"
    exit 0
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="$2"
            shift 2
            ;;
        --tag)
            TAG="$2"
            shift 2
            ;;
        --run-id)
            RUN_ID="$2"
            shift 2
            ;;
        --remote-host)
            REMOTE_HOST="$2"
            shift 2
            ;;
        --smoke)
            RUN_SMOKE=true
            shift
            ;;
        -h|--help)
            usage
            ;;
        *)
            echo "Unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

# Resolve release tag if not provided
if [ -z "$TAG" ] && [ -z "$RUN_ID" ]; then
    echo "==> Resolving latest GitHub Release tag..."
    TAG=$(gh release view --json tagName -q .tagName 2>/dev/null || true)
    if [ -z "$TAG" ]; then
        echo "Error: could not determine latest release tag and no --run-id provided." >&2
        exit 1
    fi
    echo "==> Target release: ${TAG}"
fi

# TAG/RUN_ID are interpolated into remote PowerShell one-liners below —
# validate the shape before it ever reaches a shell (rejects quoting and
# metacharacter injection through operator-supplied values).
if [ -n "$TAG" ] && ! [[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    echo "Error: --tag must match vX.Y.Z (got: ${TAG})." >&2
    exit 1
fi
if [ -n "$RUN_ID" ] && ! [[ "$RUN_ID" =~ ^[0-9]+$ ]]; then
    echo "Error: --run-id must be numeric (got: ${RUN_ID})." >&2
    exit 1
fi

TMP_DIR=$(mktemp -d /tmp/tetherly-deploy-XXXXXX)
cleanup() {
    rm -rf "$TMP_DIR"
}
trap cleanup EXIT

# ------------------------------------------------------------------------------
# 1. macOS Deployment
# ------------------------------------------------------------------------------
deploy_mac() {
    echo ""
    echo "========================================================================"
    echo " Deploying to macOS Controller (/Applications/Tetherly.app)"
    echo "========================================================================"

    MAC_BIN="$TMP_DIR/tetherly-macos-arm64"
    if [ -n "$RUN_ID" ]; then
        echo "==> Fetching macOS binary from GitHub Actions run ${RUN_ID}..."
        gh run download "$RUN_ID" -n "tetherly-macos-arm64" -D "$TMP_DIR"
    else
        echo "==> Fetching macOS binary from GitHub Release ${TAG}..."
        gh release download "$TAG" -p "tetherly-macos-arm64" -D "$TMP_DIR"
    fi

    if [ ! -f "$MAC_BIN" ]; then
        echo "Error: expected binary not found at $MAC_BIN" >&2
        exit 1
    fi
    chmod +x "$MAC_BIN"

    echo "==> Packaging /Applications/Tetherly.app with stable Designated Requirement..."
    "$SCRIPT_DIR/package-macos-app.sh" --binary "$MAC_BIN" --restart

    echo "==> Verifying local macOS health (this is a gate, not a print)..."
    health_ok=""
    for _ in $(seq 1 20); do
        if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/api/health" 2>/dev/null | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
            health_ok=1
            break
        fi
        sleep 0.5
    done
    if [ -z "$health_ok" ]; then
        echo "Error: macOS daemon did not become healthy after package+restart." >&2
        echo "       The app bundle may have failed to launch (codesign/TCC). Aborting before Windows deploy." >&2
        exit 1
    fi
    curl -s "http://127.0.0.1:${LOCAL_PORT}/api/health" | jq .
    echo "==> macOS deployment complete."
}

# ------------------------------------------------------------------------------
# 2. Windows Deployment
# ------------------------------------------------------------------------------
deploy_windows() {
    echo ""
    echo "========================================================================"
    echo " Deploying to Windows Host (${REMOTE_HOST})"
    echo "========================================================================"

    # Invariant: Preflight protected processes check
    echo "==> Pre-checking protected production processes on ${REMOTE_HOST}..."
    WIN_AUDIT=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Get-Process ${PROTECT_PROCS} -ErrorAction SilentlyContinue | Select-Object Id, ProcessName, SessionId | ConvertTo-Json -Compress\"" 2>/dev/null || true)
    echo "Protected processes live status: $WIN_AUDIT"
    for p in $(echo "$PROTECT_PROCS" | tr ',' ' '); do
        if ! echo "$WIN_AUDIT" | grep -qi "\"${p}\""; then
            echo "Error: protected process ${p} is not running! Aborting deployment to prevent interference." >&2
            exit 1
        fi
    done

    WIN_BIN="$TMP_DIR/tetherly-windows-x86_64.exe"
    WIN_CLI="$TMP_DIR/tetherly-cli-windows-x86_64.exe"

    if [ -n "$RUN_ID" ]; then
        echo "==> Fetching Windows binaries from GitHub Actions run ${RUN_ID}..."
        gh run download "$RUN_ID" -n "tetherly-windows-x86_64" -D "$TMP_DIR"
    else
        echo "==> Fetching Windows binaries from GitHub Release ${TAG}..."
        gh release download "$TAG" -p "tetherly-windows-x86_64.exe" -p "tetherly-cli-windows-x86_64.exe" -D "$TMP_DIR"
    fi

    if [ ! -f "$WIN_BIN" ]; then
        echo "Error: expected binary not found at $WIN_BIN" >&2
        exit 1
    fi

    echo "==> Staging Windows binaries to ${REMOTE_HOST}:E:/code/tetherly/data/updates/..."
    REMOTE_STAGING_NAME="tetherly-${TAG:-run_${RUN_ID}}.exe"
    scp "$WIN_BIN" "${REMOTE_HOST}:E:/code/tetherly/data/updates/${REMOTE_STAGING_NAME}"
    if [ -f "$WIN_CLI" ]; then
        scp "$WIN_CLI" "${REMOTE_HOST}:E:/code/tetherly/data/updates/tetherly-cli.exe"
        ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Copy-Item 'E:\code\tetherly\data\updates\tetherly-cli.exe' 'E:\code\tetherly\target\release\tetherly-cli.exe' -Force\""
    fi

    echo "==> Performing preflight --help on remote staged binary..."
    ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"& 'E:\code\tetherly\data\updates\\${REMOTE_STAGING_NAME}' --help | Select-Object -First 3\""

    # /api/daemon/restart enforces its own staging allowlist on C:; binaries
    # staged anywhere else are rejected (production 2026-11-11, twice).
    echo "==> Mirroring staged binary into daemon update staging dir (C:)..."
    ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path 'C:\Users\mango\AppData\Local\tetherly\updates' | Out-Null; Copy-Item 'E:\code\tetherly\data\updates\\${REMOTE_STAGING_NAME}' 'C:\Users\mango\AppData\Local\tetherly\updates\\${REMOTE_STAGING_NAME}' -Force\""

    echo "==> Initiating NTFS in-use atomic hot-restart via /api/daemon/restart..."
    RESTART_RESP=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Invoke-RestMethod -Uri http://127.0.0.1:45716/api/daemon/restart -Method Post -Body (ConvertTo-Json @{replacement_path='C:\Users\mango\AppData\Local\tetherly\updates\\${REMOTE_STAGING_NAME}'}) -ContentType 'application/json' | ConvertTo-Json -Compress\"")
    echo "Restart response: $RESTART_RESP"

    echo "==> Waiting 3s for daemon handoff..."
    sleep 3

    echo "==> Verifying Windows node health (version gate)..."
    HEALTH_JSON=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Invoke-RestMethod -Uri http://127.0.0.1:${REMOTE_PORT}/api/health | ConvertTo-Json -Compress\"")
    echo "$HEALTH_JSON"
    if [ -n "${TAG}" ] && ! echo "$HEALTH_JSON" | grep -q "\"version\":\"${TAG#v}\""; then
        echo "FATAL: deployed version does not match ${TAG}!" >&2
        exit 3
    fi

    echo "==> Post-checking protected production processes on ${REMOTE_HOST}..."
    WIN_POST_AUDIT=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Get-Process ${PROTECT_PROCS} -ErrorAction SilentlyContinue | Select-Object Id, ProcessName, SessionId | ConvertTo-Json -Compress\"" 2>/dev/null || true)
    for p in $(echo "$PROTECT_PROCS" | tr ',' ' '); do
        if ! echo "$WIN_POST_AUDIT" | grep -qi "\"${p}\""; then
            echo "FATAL: protected process ${p} was interrupted during deployment!" >&2
            exit 2
        fi
    done
    echo "Protected processes 100% intact: $WIN_POST_AUDIT"
    echo "==> Windows deployment complete."
}

case "$TARGET" in
    mac|macos)
        deploy_mac
        ;;
    win|windows)
        deploy_windows
        ;;
    all)
        deploy_mac
        deploy_windows
        ;;
    *)
        echo "Error: invalid target $TARGET (expected: mac, windows, all)" >&2
        exit 1
        ;;
esac

if [ "$RUN_SMOKE" = true ]; then
    echo ""
    echo "========================================================================"
    echo " Running Cross-Platform Smoke Tests..."
    echo "========================================================================"
    SMOKE_PROTECT_PROCS="$PROTECT_PROCS" "$SCRIPT_DIR/verify_cross_platform_smoke.sh"
fi
