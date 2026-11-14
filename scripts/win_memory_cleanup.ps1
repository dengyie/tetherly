# SPDX-License-Identifier: Apache-2.0 OR MIT
# Pre-deploy memory cleanup for the Windows host (home-win).
#
# The host is over-committed (commit ~31 GB on 16 GB RAM); under that pressure
# the tetherly daemon gets paged out and the first crossing after idle stalls
# for seconds (DEVELOPMENT.md §114). deploy.sh runs this before staging a new
# daemon so the fresh process starts with physical memory available.
#
# What it does (nothing else):
#   1. Kills stale remote ssh command sessions: a cmd.exe spawned by sshd
#      (the exact `"c:\windows\system32\cmd.exe" /c ` form sshd uses), whose
#      parent is sshd or already gone, older than -MinAgeHours, plus all of
#      its descendants. These are hung deploy/probe sessions that never exit.
#   2. Trims working sets (EmptyWorkingSet) of ordinary processes. This is
#      non-destructive: pages are only moved to the standby/modified lists and
#      fault back on next use.
#
# Never touched: protected production processes (SRBMiner-MULTI, verge-mihomo,
# komari-agent) and their descendants, tetherly itself, system-critical
# processes, and this script's own ancestor chain. Inbound sshd sessions are
# never killed (port-forward sessions legitimately have no children).
#
# Usage: powershell -NoProfile -File win_memory_cleanup.ps1 [-DryRun] [-MinAgeHours 6]

param(
    [switch]$DryRun,
    [double]$MinAgeHours = 6
)

$ErrorActionPreference = 'Continue'
$ProgressPreference = 'SilentlyContinue'

$protectedNames = @('SRBMiner-MULTI', 'verge-mihomo', 'komari-agent')
$noTrimNames = @('System', 'Idle', 'Registry', 'Memory Compression', 'smss', 'csrss',
    'wininit', 'winlogon', 'services', 'lsass', 'dwm', 'audiodg', 'explorer',
    'tetherly', 'tetherly-cli') + $protectedNames

Add-Type -Namespace TetherlyMem -Name Native -MemberDefinition @'
[DllImport("psapi.dll", SetLastError = true)]
public static extern bool EmptyWorkingSet(IntPtr hProcess);
'@

function Get-MemSummary {
    $os = Get-CimInstance Win32_OperatingSystem
    $m = Get-CimInstance Win32_PerfFormattedData_PerfOS_Memory
    '{0:N0} MB available, {1:N1} GB committed / {2:N1} GB limit' -f `
        $m.AvailableMBytes,
        (($os.TotalVirtualMemorySize - $os.FreeVirtualMemory) / 1MB),
        ($os.TotalVirtualMemorySize / 1MB)
}

$all = @(Get-CimInstance Win32_Process)
$byId = @{}
foreach ($p in $all) { $byId[[int]$p.ProcessId] = $p }
$children = @{}
foreach ($p in $all) {
    $ppid = [int]$p.ParentProcessId
    if (-not $children.ContainsKey($ppid)) { $children[$ppid] = New-Object System.Collections.ArrayList }
    [void]$children[$ppid].Add([int]$p.ProcessId)
}

function Get-Descendants([int]$id, [hashtable]$seen) {
    if ($seen.ContainsKey($id)) { return }
    $seen[$id] = $true
    if ($children.ContainsKey($id)) {
        foreach ($c in $children[$id]) {
            # A child created before its "parent" is a PID-reuse artefact.
            if ($byId[$c].CreationDate -ge $byId[$id].CreationDate) { Get-Descendants $c $seen }
        }
    }
}

# Untouchable set: protected processes + their descendants + our ancestors.
$untouchable = @{}
foreach ($p in $all) {
    if ($protectedNames -contains [IO.Path]::GetFileNameWithoutExtension($p.Name)) {
        Get-Descendants ([int]$p.ProcessId) $untouchable
    }
}
$cur = [int]$PID
while ($cur -and $byId.ContainsKey($cur) -and -not $untouchable.ContainsKey($cur)) {
    $untouchable[$cur] = $true
    $cur = [int]$byId[$cur].ParentProcessId
}

Write-Output ("before: " + (Get-MemSummary))

# 1. Stale sshd command sessions.
$cutoff = (Get-Date).AddHours(-$MinAgeHours)
$killed = 0
foreach ($p in $all) {
    if ($p.Name -ne 'cmd.exe') { continue }
    if (-not ("$($p.CommandLine)" -cmatch '^"c:\\windows\\system32\\cmd\.exe" /c ')) { continue }
    if ($p.CreationDate -gt $cutoff) { continue }
    $parent = $byId[[int]$p.ParentProcessId]
    $parentIsSshd = $parent -and $parent.Name -eq 'sshd.exe' -and $parent.CreationDate -le $p.CreationDate
    $parentGone = -not $parent -or $parent.CreationDate -gt $p.CreationDate
    if (-not ($parentIsSshd -or $parentGone)) { continue }
    $tree = @{}
    Get-Descendants ([int]$p.ProcessId) $tree
    $ids = @($tree.Keys | Where-Object { -not $untouchable.ContainsKey($_) })
    if ($ids.Count -ne $tree.Count) {
        Write-Output ("skip stale session cmd pid={0}: tree contains untouchable process" -f $p.ProcessId)
        continue
    }
    foreach ($id in ($ids | Sort-Object -Descending)) {
        $q = $byId[$id]
        $cl = "$($q.CommandLine)"; if ($cl.Length -gt 100) { $cl = $cl.Substring(0, 100) }
        Write-Output ("{0}kill stale ssh session pid={1} {2} started={3} cmd={4}" -f `
            $(if ($DryRun) { '[dry-run] ' } else { '' }), $id, $q.Name, $q.CreationDate, $cl)
        if (-not $DryRun) {
            Stop-Process -Id $id -Force -ErrorAction SilentlyContinue
            $killed++
        }
    }
}

# 2. Trim working sets.
$trimmed = 0; $trimmedMB = 0
foreach ($proc in Get-Process) {
    if ($noTrimNames -contains $proc.ProcessName) { continue }
    if ($untouchable.ContainsKey([int]$proc.Id)) { continue }
    if ($proc.WorkingSet64 -lt 32MB) { continue }
    $trimmedMB += [int]($proc.WorkingSet64 / 1MB)
    if ($DryRun) { $trimmed++; continue }
    try {
        if ([TetherlyMem.Native]::EmptyWorkingSet($proc.Handle)) { $trimmed++ }
    } catch { }
}
Write-Output ("{0}trimmed working sets of {1} processes (~{2} MB resident before trim)" -f `
    $(if ($DryRun) { '[dry-run] ' } else { '' }), $trimmed, $trimmedMB)

Write-Output ("after:  " + (Get-MemSummary))
$alive = @(Get-Process $protectedNames -ErrorAction SilentlyContinue).Count
Write-Output ("protected processes alive: {0}/{1}; stale processes killed: {2}" -f $alive, $protectedNames.Count, $killed)
if ($alive -ne $protectedNames.Count) { exit 2 }
exit 0
