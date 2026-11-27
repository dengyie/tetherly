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
DATA_DIR="${TETHERLY_DATA_DIR:-$HOME/.local/share/tetherly}"
LOCAL_PORT=45716
RUN_SMOKE=false
STAGING_DIR=""
PROTECT_PROCS="SRBMiner-MULTI,verge-mihomo,komari-agent"
# Protected names that may already be stopped before the deploy (comma list,
# e.g. TETHERLY_ALLOW_STOPPED=SRBMiner-MULTI when it was stopped on purpose).
# They stay protected (never touched); the deploy just does not refuse to run,
# and the post-check requires every process that was alive beforehand.
ALLOW_STOPPED="${TETHERLY_ALLOW_STOPPED:-}"

usage() {
    echo "Usage: $0 [options]"
    echo ""
    echo "Options:"
    echo "  --target <mac|windows|all>   Target host to deploy (default: all)"
    echo "  --tag <vX.Y.Z>               GitHub Release tag to fetch from (default: latest release)"
    echo "  --run-id <ID>                GitHub Actions workflow run ID to fetch artifacts from"
    echo "  --remote-host <host>         Windows SSH host alias (default: home-win)"
    echo "  --staging-dir <dir>          Use pre-downloaded, verified release assets from this dir"
    echo "                               (useful when gh release download keeps hitting network EOF)"
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
        --staging-dir)
            STAGING_DIR="$2"
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
# 0. Desktop client (Tauri shell) install — §230
# ------------------------------------------------------------------------------
# The tray's "打开主窗口" launches the desktop client via `tetherly://open`
# and only falls back to the browser dashboard when the scheme is missing.
# Ship the client from the same release so both hosts open the client.
# Best-effort: a client install failure warns but never fails the daemon
# deploy (the daemon is the production-critical part). Release-tag only —
# run-id artifacts don't carry the bundles.
verify_release_digest() {
    local file="$1" expected actual
    expected=$(awk -v name="$(basename "$file")" '$2 == name || $2 == "./" name || $2 == "*" name {print $1; exit}' "$TMP_DIR/SHA256SUMS")
    actual=$(shasum -a 256 "$file" | awk '{print $1}')
    [ -n "$expected" ] && [ "$actual" = "$expected" ]
}

install_client_mac() {
    [ -n "$TAG" ] || { echo "==> Skipping desktop client install (no release tag)."; return 0; }
    local ver="${TAG#v}" dmg mnt app dest="/Applications/Tetherly Client.app" was_running=""
    dmg="$TMP_DIR/Tetherly_${ver}_aarch64.dmg"
    echo "==> Installing desktop client ${TAG} to ${dest}..."
    gh release download "$TAG" -p "$(basename "$dmg")" -p "SHA256SUMS" -D "$TMP_DIR" --skip-existing || return 1
    verify_release_digest "$dmg" || { echo "digest mismatch for $(basename "$dmg")" >&2; return 1; }
    mnt=$(mktemp -d "$TMP_DIR/dmg-XXXXXX")
    hdiutil attach -nobrowse -readonly -mountpoint "$mnt" "$dmg" >/dev/null || return 1
    app=$(find "$mnt" -maxdepth 1 -name '*.app' -print -quit)
    if [ -z "$app" ] || [ "$(defaults read "$app/Contents/Info" CFBundleIdentifier 2>/dev/null)" != "app.tetherly.shell" ]; then
        echo "DMG does not contain the app.tetherly.shell bundle" >&2
        hdiutil detach "$mnt" -quiet || true
        return 1
    fi
    if pgrep -x tetherly-shell >/dev/null; then
        was_running=1
        osascript -e 'tell application id "app.tetherly.shell" to quit' >/dev/null 2>&1 || true
        for _ in $(seq 1 20); do pgrep -x tetherly-shell >/dev/null || break; sleep 0.25; done
        pkill -x tetherly-shell 2>/dev/null || true
    fi
    # Never write over /Applications/Tetherly.app — that is the daemon bundle.
    rm -rf "$dest.new"
    ditto "$app" "$dest.new" || { hdiutil detach "$mnt" -quiet || true; return 1; }
    hdiutil detach "$mnt" -quiet || true
    rm -rf "$dest" && mv "$dest.new" "$dest" || return 1
    xattr -dr com.apple.quarantine "$dest" 2>/dev/null || true
    # Re-register so `tetherly://` resolves to the fresh bundle.
    /System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$dest" >/dev/null 2>&1 || true
    [ -n "$was_running" ] && open "$dest" || true
    echo "==> Desktop client installed: $(defaults read "$dest/Contents/Info" CFBundleShortVersionString)"
}

install_client_windows() {
    [ -n "$TAG" ] || { echo "==> Skipping desktop client install (no release tag)."; return 0; }
    local ver="${TAG#v}" msi msi_name ps b64 out
    msi_name="Tetherly_${ver}_x64_en-US.msi"
    msi="$TMP_DIR/$msi_name"
    echo "==> Installing desktop client ${TAG} on ${REMOTE_HOST}..."
    gh release download "$TAG" -p "$msi_name" -p "SHA256SUMS" -D "$TMP_DIR" --skip-existing || return 1
    verify_release_digest "$msi" || { echo "digest mismatch for $msi_name" >&2; return 1; }
    scp -q "$msi" "${REMOTE_HOST}:E:/code/tetherly/data/updates/${msi_name}" || return 1
    # perMachine MSI: the elevated ssh session can install it silently. The
    # client is `tetherly-shell.exe` (listed as `TETHER~1` when launched via
    # the 8.3 path the scheme registers), distinct from the daemon's
    # `tetherly.exe`, so stopping it never touches the daemon.
    # Copy to a LOCAL path before msiexec (production 2026-11-26): installing
    # straight from E:\... returned 1619 ERROR_INSTALL_PACKAGE_OPEN_FAILED —
    # the mapped drive is not a valid MSI source for the elevated installer
    # session. Same bytes from %LOCALAPPDATA%\Temp installed cleanly.
    ps="\$ErrorActionPreference='Stop'; \$ProgressPreference='SilentlyContinue'; Get-Process -Name tetherly-shell,TETHER~1 -ErrorAction SilentlyContinue | Where-Object { \$_.Path -and (Get-Item -LiteralPath \$_.Path).FullName -like '*\\tetherly-shell.exe' } | Stop-Process -Force; \$src='E:\\code\\tetherly\\data\\updates\\${msi_name}'; \$m=\$env:LOCALAPPDATA+'\\Temp\\tetherly-client-install.msi'; \$l=\$env:LOCALAPPDATA+'\\Temp\\tetherly-client-install.log'; Copy-Item \$src \$m -Force; \$p=Start-Process msiexec.exe -ArgumentList @('/i',\$m,'/qn','/norestart','/l*v',\$l) -Wait -PassThru; \$cmd=(Get-ItemProperty -LiteralPath 'Registry::HKEY_CLASSES_ROOT\\tetherly\\shell\\open\\command' -ErrorAction SilentlyContinue).'(default)'; \$exe=if (\$cmd -match '^\"([^\"]+)\"') { \$Matches[1] } else { '' }; \$name=if (\$exe -and (Test-Path -LiteralPath \$exe)) { (Get-Item -LiteralPath \$exe).FullName } else { 'missing' }; Write-Output ('msi_exit=' + \$p.ExitCode + ' scheme=' + \$name)"
    b64=$(printf '%s' "$ps" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\r\n')
    out=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -NonInteractive -EncodedCommand ${b64}" 2>/dev/null | tr -d '\r') || return 1
    echo "$out"
    case "$out" in
        *"msi_exit=0 "*|*"msi_exit=3010 "*) ;;
        *) echo "msiexec failed (see E:\\code\\tetherly\\data\\updates\\client-install.log)" >&2; return 1 ;;
    esac
    case "$out" in
        *tetherly-shell.exe*) ;;
        *) echo "tetherly:// scheme is not registered to tetherly-shell.exe" >&2; return 1 ;;
    esac
    echo "==> Desktop client installed on ${REMOTE_HOST}."
    # Relaunch the client in the console user's desktop session (v0.2.99;
    # production 2026-11-26: the install above stopped it and nothing
    # brought it back). ssh runs in session 0, so a plain Start-Process
    # would be invisible: a one-shot Interactive task runs it as the console
    # user, then is removed. Unconditional — a client a previous deploy
    # already left dead must come back too. Best-effort.
    ps=$(cat <<'PS1'
$ErrorActionPreference='Stop'; $ProgressPreference='SilentlyContinue'
$cmd=(Get-ItemProperty -LiteralPath 'Registry::HKEY_CLASSES_ROOT\tetherly\shell\open\command').'(default)'
$exe=if ($cmd -match '^"([^"]+)"') { (Get-Item -LiteralPath $Matches[1]).FullName } else { throw 'scheme missing' }
$u=(Get-CimInstance Win32_ComputerSystem).UserName
if (-not $u) { Write-Output 'relaunch=skipped(no console user)'; exit 0 }
if (Get-Process -Name tetherly-shell,TETHER~1 -ErrorAction SilentlyContinue) { Write-Output 'relaunch=already-running'; exit 0 }
$n='TetherlyClientRelaunch'
$a=New-ScheduledTaskAction -Execute $exe
$p=New-ScheduledTaskPrincipal -UserId $u -LogonType Interactive -RunLevel Limited
Register-ScheduledTask -TaskName $n -Action $a -Principal $p -Force | Out-Null
Start-ScheduledTask -TaskName $n
$ok=$false; foreach ($i in 1..20) { Start-Sleep -Milliseconds 250; $s=Get-Process -Name tetherly-shell,TETHER~1 -ErrorAction SilentlyContinue; if ($s) { $ok=$true; break } }
Unregister-ScheduledTask -TaskName $n -Confirm:$false
if ($ok) { Write-Output ('relaunch=ok session=' + ($s | Select-Object -First 1).SessionId) } else { Write-Output 'relaunch=failed' }
PS1
)
    b64=$(printf '%s' "$ps" | LC_ALL=C iconv -f UTF-8 -t UTF-16LE | base64 | LC_ALL=C tr -d '\r\n')
    out=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -NonInteractive -EncodedCommand ${b64}" 2>/dev/null | LC_ALL=C tr -d '\r') || \
        out="relaunch=ssh-failed"
    echo "==> Client relaunch: $out"
    # Daemon autostart (2026-11-25): the scheduled task was hand-registered
    # outside this repo, so a new machine or a wiped task silently lost
    # boot persistence. The binary now self-registers; idempotent.
    # Start-Process -Wait + file redirects: the release exe is GUI-subsystem,
    # so `$out = exe ... 2>&1` neither waits nor captures (v0.2.99 deploy
    # printed an empty line and the stray TetherlyDaemon task survived).
    ps="\$ProgressPreference='SilentlyContinue'; \$o=\"\$env:TEMP\\tetherly-svc.out\"; \$e=\"\$env:TEMP\\tetherly-svc.err\"; \$p = Start-Process -FilePath 'E:\code\tetherly\target\release\tetherly.exe' -ArgumentList 'service','install','--json' -NoNewWindow -Wait -PassThru -RedirectStandardOutput \$o -RedirectStandardError \$e; \$r = ((Get-Content \$o,\$e -ErrorAction SilentlyContinue) -join ' ').Trim(); Write-Output \"exit=\$(\$p.ExitCode) \$r\""
    b64=$(printf '%s' "$ps" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\r\n')
    out=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -NonInteractive -EncodedCommand ${b64}" 2>/dev/null | tr -d '\r') || \
        out="(service install ssh failed — verify autostart manually)"
    echo "==> Daemon autostart: $out"
}

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
        gh run download "$RUN_ID" -n "tetherly-macos-arm64" -D "$TMP_DIR" --skip-existing
        gh run download "$RUN_ID" -n "tetherly-release-digests" -D "$TMP_DIR" --skip-existing
    else
        echo "==> Fetching macOS binary from GitHub Release ${TAG}..."
        gh release download "$TAG" -p "tetherly-macos-arm64" -p "SHA256SUMS" -D "$TMP_DIR" --skip-existing
    fi
    if [ ! -s "$TMP_DIR/SHA256SUMS" ]; then
        echo "Error: release digest manifest is missing; refusing deployment." >&2
        exit 1
    fi
    if [ ! -f "$MAC_BIN" ]; then
        echo "Error: expected binary not found at $MAC_BIN" >&2
        exit 1
    fi
    expected=$(awk -v name="$(basename "$MAC_BIN")" '$2 == name || $2 == "./" name || $2 == "*" name {print $1; exit}' "$TMP_DIR/SHA256SUMS")
    actual=$(shasum -a 256 "$MAC_BIN" | awk '{print $1}')
    if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
        echo "Error: digest mismatch for $(basename "$MAC_BIN"); refusing deployment." >&2
        exit 1
    fi

    chmod +x "$MAC_BIN"

    echo "==> Packaging /Applications/Tetherly.app with stable Designated Requirement..."
    "$SCRIPT_DIR/package-macos-app.sh" --binary "$MAC_BIN" --restart

    echo "==> Verifying local macOS health (this is a gate, not a print)..."
    # §169: the legacy `/api/*` surface is gone as of v0.2.64 — the open
    # `GET /api/v1/health` is the health gate (no token needed by design).
    health_ok=""
    for _ in $(seq 1 20); do
        if curl -fsS "http://127.0.0.1:${LOCAL_PORT}/api/v1/health" 2>/dev/null | grep -q '"ok"[[:space:]]*:[[:space:]]*true'; then
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
    curl -s "http://127.0.0.1:${LOCAL_PORT}/api/v1/health" | jq .
    install_client_mac || echo "WARN: desktop client install failed on macOS; daemon deploy is unaffected." >&2
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
    ALIVE_BEFORE=""
    for p in $(echo "$PROTECT_PROCS" | tr ',' ' '); do
        if echo "$WIN_AUDIT" | grep -qi "\"${p}\""; then
            ALIVE_BEFORE="${ALIVE_BEFORE:+${ALIVE_BEFORE},}${p}"
        elif echo ",${ALLOW_STOPPED}," | grep -qi ",${p},"; then
            echo "WARN: protected process ${p} was already stopped before deploy (allowed by TETHERLY_ALLOW_STOPPED); it will not be touched or started." >&2
        else
            echo "Error: protected process ${p} is not running! Aborting deployment to prevent interference." >&2
            echo "       (if it was stopped on purpose: TETHERLY_ALLOW_STOPPED=${p} $0 ...)" >&2
            exit 1
        fi
    done

    # Pre-deploy memory cleanup (v0.2.42): the host is over-committed and a
    # paged-out daemon stalls the first crossing after idle (DEVELOPMENT.md
    # §114). Kills only hung sshd command sessions and trims working sets; it
    # never touches protected processes and exits 2 if any of them is gone.
    echo "==> Cleaning Windows memory before deploy (stale ssh sessions + working-set trim)..."
    scp -q "$SCRIPT_DIR/win_memory_cleanup.ps1" "${REMOTE_HOST}:E:/code/tetherly/data/win_memory_cleanup.ps1"
    set +e
    ssh "${REMOTE_HOST}" "powershell -NoProfile -ExecutionPolicy Bypass -File E:\\code\\tetherly\\data\\win_memory_cleanup.ps1 -Require ${ALIVE_BEFORE}"
    CLEAN_RC=$?
    set -e
    if [ "$CLEAN_RC" -eq 2 ]; then
        echo "FATAL: protected process missing after memory cleanup; aborting." >&2
        exit 2
    elif [ "$CLEAN_RC" -ne 0 ]; then
        echo "WARN: memory cleanup exited ${CLEAN_RC}; continuing deploy." >&2
    fi

    WIN_BIN="$TMP_DIR/tetherly-windows-x86_64.exe"
    WIN_CLI="$TMP_DIR/tetherly-cli-windows-x86_64.exe"

    if [ -n "$STAGING_DIR" ]; then
        # Pre-downloaded assets (production 2026-11-26: gh release download
        # hit `unexpected EOF` on every attempt; curl -C - resume loop got
        # the files through). Digest checks below still apply unchanged.
        echo "==> Using pre-staged assets from ${STAGING_DIR}..."
        cp "$STAGING_DIR"/tetherly-windows-x86_64.exe "$WIN_BIN"
        [ -f "$STAGING_DIR/tetherly-cli-windows-x86_64.exe" ] && \
            cp "$STAGING_DIR"/tetherly-cli-windows-x86_64.exe "$WIN_CLI"
        cp "$STAGING_DIR"/SHA256SUMS "$TMP_DIR/SHA256SUMS"
    elif [ -n "$RUN_ID" ]; then
        echo "==> Fetching Windows binaries from GitHub Actions run ${RUN_ID}..."
        gh run download "$RUN_ID" -n "tetherly-windows-x86_64" -D "$TMP_DIR" --skip-existing
        gh run download "$RUN_ID" -n "tetherly-release-digests" -D "$TMP_DIR" --skip-existing
    else
        echo "==> Fetching Windows binaries from GitHub Release ${TAG}..."
        gh release download "$TAG" -p "tetherly-windows-x86_64.exe" -p "tetherly-cli-windows-x86_64.exe" -p "SHA256SUMS" -D "$TMP_DIR" --skip-existing
    fi
    if [ ! -s "$TMP_DIR/SHA256SUMS" ]; then
        echo "Error: release digest manifest is missing; refusing deployment." >&2
        exit 1
    fi
    if [ ! -f "$WIN_BIN" ]; then
        echo "Error: expected binary not found at $WIN_BIN" >&2
        exit 1
    fi
    expected=$(awk -v name="$(basename "$WIN_BIN")" '$2 == name || $2 == "./" name || $2 == "*" name {print $1; exit}' "$TMP_DIR/SHA256SUMS")
    actual=$(shasum -a 256 "$WIN_BIN" | awk '{print $1}')
    if [ -z "$expected" ] || [ "$actual" != "$expected" ]; then
        echo "Error: digest mismatch for $(basename "$WIN_BIN"); refusing deployment." >&2
        exit 1
    fi
    # The CLI binary is staged onto the host too (§212): same digest standard,
    # otherwise a corrupted CLI would ship verified-daemon + unverified-cli.
    if [ -f "$WIN_CLI" ]; then
        expected_cli=$(awk -v name="$(basename "$WIN_CLI")" '$2 == name || $2 == "./" name || $2 == "*" name {print $1; exit}' "$TMP_DIR/SHA256SUMS")
        actual_cli=$(shasum -a 256 "$WIN_CLI" | awk '{print $1}')
        if [ -z "$expected_cli" ] || [ "$actual_cli" != "$expected_cli" ]; then
            echo "Error: digest mismatch for $(basename "$WIN_CLI"); refusing deployment." >&2
            exit 1
        fi
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

    echo "==> Capturing old daemon PID and process start-time rollback anchor..."
    OLD_PROCESS=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"\$d=Get-CimInstance Win32_Process | Where-Object { \$_.Name -eq 'tetherly.exe' -and \$_.CommandLine -match ' run\s*\$' } | Select-Object -First 1; \$p=Get-Process -Id \$d.ProcessId -ErrorAction Stop; [pscustomobject]@{pid=\$p.Id; start_ticks=\$p.StartTime.ToUniversalTime().Ticks; path=\$p.Path} | ConvertTo-Json -Compress\"")
    OLD_PID=$(printf '%s' "$OLD_PROCESS" | jq -r '.pid // empty')
    OLD_START_TICKS=$(printf '%s' "$OLD_PROCESS" | jq -r '.start_ticks // empty')
    OLD_EXE=$(printf '%s' "$OLD_PROCESS" | jq -r '.path // empty')
    OLD_SHA256=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"(Get-FileHash -Algorithm SHA256 -LiteralPath '$OLD_EXE').Hash.ToLowerInvariant()\"")
    # PowerShell over ssh emits CRLF; a trailing CR breaks the hex anchor check.
    OLD_SHA256=$(printf '%s' "$OLD_SHA256" | tr -d '\r\n')
    if [ -z "$OLD_PID" ] || [ -z "$OLD_START_TICKS" ] || [ -z "$OLD_EXE" ] || ! [[ "$OLD_SHA256" =~ ^[0-9a-f]{64}$ ]]; then
        echo "Error: failed to capture old daemon PID/start time; refusing restart." >&2
        exit 1
    fi

    echo "==> Initiating NTFS in-use atomic hot-restart via /api/v1/system/restart..."
    # v0.2.52 (P0-1): restart requires the daemon's per-start API token.
    # §169: the endpoint lives at `/api/v1/system/restart` (Admin tier) since
    # v0.2.64; the legacy `/api/daemon/restart` alias is gone. The file is
    # SYSTEM/Administrators-only; the ssh session is an elevated
    # Administrator, so it can read it.
    RESTART_RESP=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"\$h=@{}; \$tf='C:\Users\mango\AppData\Local\tetherly\api.token'; if (Test-Path \$tf) { \$h['X-Tetherly-Token']=(Get-Content -Raw \$tf).Trim() }; Invoke-RestMethod -Uri http://127.0.0.1:45716/api/v1/system/restart -Method Post -Headers \$h -Body (ConvertTo-Json @{replacement_path='C:\Users\mango\AppData\Local\tetherly\updates\\${REMOTE_STAGING_NAME}'}) -ContentType 'application/json' | ConvertTo-Json -Compress\"")
    echo "Restart response: $RESTART_RESP"
    RESTART_OLD_PID=$(printf '%s' "$RESTART_RESP" | jq -r '.old_pid // empty')
    if [ "$RESTART_OLD_PID" != "$OLD_PID" ]; then
        echo "FATAL: restart response old_pid does not match captured process." >&2
        exit 3
    fi

    echo "==> Waiting for replacement readiness (new PID/version/start time)..."
    EXPECTED_VERSION="${TAG#v}"
    READY=false
    for _ in $(seq 1 30); do
        PROBE=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"\$d=Get-CimInstance Win32_Process | Where-Object { \$_.Name -eq 'tetherly.exe' -and \$_.CommandLine -match ' run\s*\$' } | Select-Object -First 1; \$p=if (\$d) { Get-Process -Id \$d.ProcessId -ErrorAction SilentlyContinue }; \$h=try { Invoke-RestMethod -Uri http://127.0.0.1:${REMOTE_PORT}/api/v1/health -ErrorAction Stop } catch { \$null }; \$s=try { Invoke-RestMethod -Uri http://127.0.0.1:${REMOTE_PORT}/api/v1/system -Headers @{'X-Tetherly-Token'=(Get-Content -Raw 'C:\\Users\\mango\\AppData\\Local\\tetherly\\api.token').Trim()} -ErrorAction Stop } catch { \$null }; if (\$p -and \$h -and \$s) { [pscustomobject]@{pid=\$p.Id; start_ticks=\$p.StartTime.ToUniversalTime().Ticks; health_pid=\$s.pid; version=\$h.version; ok=\$h.ok} | ConvertTo-Json -Compress }\"")
        NEW_PID=$(printf '%s' "$PROBE" | jq -r '.pid // empty' 2>/dev/null || true)
        NEW_START_TICKS=$(printf '%s' "$PROBE" | jq -r '.start_ticks // empty' 2>/dev/null || true)
        HEALTH_PID=$(printf '%s' "$PROBE" | jq -r '.health_pid // empty' 2>/dev/null || true)
        HEALTH_VERSION=$(printf '%s' "$PROBE" | jq -r '.version // empty' 2>/dev/null || true)
        HEALTH_OK=$(printf '%s' "$PROBE" | jq -r '.ok // false' 2>/dev/null || true)
        if [ "$HEALTH_OK" = "true" ] && [ "$NEW_PID" = "$HEALTH_PID" ] && [ "$NEW_PID" != "$OLD_PID" ] && [ "$NEW_START_TICKS" != "$OLD_START_TICKS" ] && { [ -z "$TAG" ] || [ "$HEALTH_VERSION" = "$EXPECTED_VERSION" ]; }; then
            READY=true
            break
        fi
        sleep 0.5
    done
    if [ "$READY" != "true" ]; then
        echo "FATAL: replacement readiness failed; restoring old daemon binary." >&2
        ROLLBACK_PS="\$exe='$OLD_EXE'; Stop-Process -Name tetherly -Force -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 500; \$old=Get-ChildItem (Split-Path \$exe) -Filter 'tetherly.exe.old.*' | Where-Object { (Get-FileHash -Algorithm SHA256 -LiteralPath \$_.FullName).Hash.ToLowerInvariant() -eq '$OLD_SHA256' } | Select-Object -First 1; if (-not \$old) { throw 'matching rollback binary backup not found' }; Move-Item \$old.FullName \$exe -Force; Start-ScheduledTask -TaskName TetherlySystemLauncher"
        ROLLBACK_B64=$(printf '%s' "$ROLLBACK_PS" | iconv -f UTF-8 -t UTF-16LE | base64 | tr -d '\r\n')
        ssh "${REMOTE_HOST}" "powershell -NoProfile -EncodedCommand ${ROLLBACK_B64}" || {
            echo "FATAL: automatic rollback command failed; old binary backup may need operator restoration." >&2
            exit 4
        }
        exit 3
    fi
    HEALTH_JSON=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Invoke-RestMethod -Uri http://127.0.0.1:${REMOTE_PORT}/api/v1/health | ConvertTo-Json -Compress\"")
    echo "$HEALTH_JSON"

    echo "==> Post-checking protected production processes on ${REMOTE_HOST}..."
    WIN_POST_AUDIT=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"Get-Process ${PROTECT_PROCS} -ErrorAction SilentlyContinue | Select-Object Id, ProcessName, SessionId | ConvertTo-Json -Compress\"" 2>/dev/null || true)
    for p in $(echo "$ALIVE_BEFORE" | tr ',' ' '); do
        if ! echo "$WIN_POST_AUDIT" | grep -qi "\"${p}\""; then
            echo "FATAL: protected process ${p} was interrupted during deployment!" >&2
            exit 2
        fi
    done
    echo "Protected processes 100% intact: $WIN_POST_AUDIT"
    # Privilege-drift guard (v0.2.20): the /api/v1/system/restart hot-restart
    # inherits the running daemon's token, so privilege only degrades when a
    # non-SYSTEM chain (TetherlyApp/autostart) was last used. A mango-owned
    # daemon cannot inject into UAC/secure windows (UIPI) — warn, don't block.
    # PowerShell's stdout arrives CRLF-terminated, so the owner string must
    # be stripped before comparing (production: "runs as SYSTEM, not SYSTEM").
    WIN_OWNER=$(ssh "${REMOTE_HOST}" "powershell -NoProfile -Command \"\$p = Get-CimInstance Win32_Process -Filter \\\"Name='tetherly.exe'\\\" | Select-Object -First 1; if (\$p) { (Invoke-CimMethod -InputObject \$p -MethodName GetOwner).User }\"" 2>/dev/null || true)
    WIN_OWNER=$(printf '%s' "${WIN_OWNER}" | tr -d '\r\n')
    if [ -n "${WIN_OWNER}" ] && [ "${WIN_OWNER}" != "SYSTEM" ]; then
        echo "WARN: tetherly daemon runs as ${WIN_OWNER}, not SYSTEM — UAC/elevated-window clicks will be dropped by UIPI." >&2
        echo "WARN: restore with: ssh ${REMOTE_HOST} \"powershell -NoProfile -Command 'Start-ScheduledTask -TaskName TetherlySystemLauncher'\"" >&2
    else
        echo "Daemon privilege: SYSTEM (UAC/elevated-window injection OK)"
    fi
    install_client_windows || echo "WARN: desktop client install failed on ${REMOTE_HOST}; daemon deploy is unaffected." >&2
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
