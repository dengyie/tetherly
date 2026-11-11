@echo off
rem Tetherly SYSTEM launch chain (TetherlySystemLauncher scheduled task ->
rem launch_system.ps1 -> this bat). The daemon runs as LocalSystem so it can
rem inject into elevated (UAC) windows; it loads the SAME device identity as
rem the interactive-user chain from the default data dir:
rem   - identity.bin carries the 64 secret bytes (v0.2.20 dual-store; the
rem     interactive user's Credential Manager vault is invisible to SYSTEM by
rem     design, so the file side is SYSTEM's only store), and
rem   - store::grant_identity_file_system_read adds SYSTEM/Administrators
rem     read ACEs on the file, so this process can actually read it.
rem Do NOT point this at E:\code\tetherly\data: that directory holds an old,
rem retired identity and starting from it would present a different device id
rem to every paired peer (production 2026-11-12).
cd /d C:\Users\mango\AppData\Local\tetherly
E:\code\tetherly\target\release\tetherly.exe --data-dir C:\Users\mango\AppData\Local\tetherly run >> C:\Users\mango\AppData\Local\tetherly\tetherly_system.log 2>&1
