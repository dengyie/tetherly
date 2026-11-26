// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `tetherly service`: install/uninstall/report the OS-level autostart entry
//! for the daemon (production 2026-11-25: a Mac reboot left only the shell
//! autostarted — the daemon had no registration path at all, so every paired
//! peer went offline until someone ran `tetherly run` by hand).
//!
//! Platform targets:
//! - **macOS**: a LaunchAgent plist under `~/Library/LaunchAgents` with
//!   `RunAtLoad` + `KeepAlive`, loaded via `launchctl bootstrap gui/<uid>`.
//!   `KeepAlive {SuccessfulExit=false}` respawns crashes only (a clean exit —
//!   API shutdown, or a duplicate losing `instance_lock` — stays down);
//!   `RunAtLoad` covers login; `ProcessType=Interactive` avoids launchd's
//!   Background CPU/IO throttling.
//! - **Windows**: a per-user scheduled task (`Register-ScheduledTask`,
//!   logon trigger + restart-on-failure) — the same shape as the manual
//!   `TetherlySystemLauncher` task the deploy chain assumed, but registered
//!   by the software instead of by hand (the manual task does not survive
//!   reinstalls or a new machine).

use super::*;
use clap::Subcommand;
#[cfg(target_os = "macos")]
use std::path::Path;

#[derive(Subcommand, Debug, Clone, Copy)]
pub(crate) enum ServiceAction {
    /// Install the daemon autostart entry (idempotent; migrates legacy manual entries).
    Install,
    /// Remove the daemon autostart entry.
    Uninstall,
    /// Print whether the autostart entry is installed and what it runs.
    Status,
}

/// `tetherly service` -- Service arm of the top-level dispatch.
pub(crate) async fn service(action: ServiceAction, json: bool) -> Result<()> {
    match action {
        ServiceAction::Install => install(json)?,
        ServiceAction::Uninstall => uninstall(json)?,
        ServiceAction::Status => status(json)?,
    }
    Ok(())
}

// ---- shared helpers ----

/// The label/file name of the managed autostart entry. One name on both
/// platforms keeps `status` trivial.
#[cfg(target_os = "macos")]
pub(crate) const SERVICE_LABEL: &str = "dev.tetherly.tetherly";
#[cfg(any(windows, test))]
const WINDOWS_TASK_NAME: &str = "TetherlyDaemon";
#[cfg(windows)]
pub(crate) const SERVICE_TASK_NAME: &str = WINDOWS_TASK_NAME;

/// The daemon executable this process is running from. The LaunchAgent /
/// scheduled task points HERE, so a dev build registers its own debug
/// binary and a packaged build registers the .app binary — no path
/// guessing (`lifecycle.rs` uses the same trick for self-restart).
#[cfg(any(target_os = "macos", windows))]
fn daemon_exe() -> Result<PathBuf> {
    std::env::current_exe().map_err(|e| anyhow::anyhow!("current_exe: {e}"))
}

/// The hand-built production chain on home-win: a SYSTEM logon task
/// that launches the daemon into the console session (UAC/elevated
/// window injection needs SYSTEM). When it exists it IS the autostart;
/// a second, per-user task would race it on the same data dir
/// (v0.2.98 regression).
#[cfg(any(windows, test))]
pub(crate) const SYSTEM_LAUNCHER_TASK: &str = "TetherlySystemLauncher";

/// PowerShell for `install`; prints `system-launcher` or `installed`.
#[cfg(any(windows, test))]
pub(crate) fn install_script(exe: &str) -> String {
    format!(
        r#"$sys = Get-ScheduledTask -TaskName '{sys}' -ErrorAction SilentlyContinue
if ($sys) {{
  if (Get-ScheduledTask -TaskName '{name}' -ErrorAction SilentlyContinue) {{
Unregister-ScheduledTask -TaskName '{name}' -Confirm:$false
  }}
  'system-launcher'
}} else {{
  $a = New-ScheduledTaskAction -Execute '{exe}' -Argument 'run'
  $t = New-ScheduledTaskTrigger -AtLogOn
  $s = New-ScheduledTaskSettingsSet -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 5) -ExecutionTimeLimit ([TimeSpan]::Zero) -StartWhenAvailable
  Register-ScheduledTask -TaskName '{name}' -Action $a -Trigger $t -Settings $s -RunLevel Highest -Force | Out-Null
  'installed'
}}"#,
        exe = exe.replace('\'', "''"),
        name = WINDOWS_TASK_NAME,
        sys = SYSTEM_LAUNCHER_TASK,
    )
}

#[cfg(test)]
mod windows_script_tests {
    use super::*;

    #[test]
    fn install_defers_to_system_launcher_and_removes_stray_task() {
        let ps = install_script(r"C:\Program Files\Tetherly\tetherly.exe");
        let sys_branch = ps.split("} else {").next().unwrap();
        assert!(sys_branch.contains("Get-ScheduledTask -TaskName 'TetherlySystemLauncher'"));
        assert!(sys_branch.contains("Unregister-ScheduledTask -TaskName 'TetherlyDaemon'"));
        assert!(!sys_branch.contains("Register-ScheduledTask -TaskName"));
        assert!(ps.contains("Register-ScheduledTask -TaskName 'TetherlyDaemon'"));
    }

    #[test]
    fn install_script_escapes_single_quotes() {
        let ps = install_script(r"C:\Users\o'brien\tetherly.exe");
        assert!(ps.contains("o''brien"));
    }
}

// ---- macOS ----

#[cfg(target_os = "macos")]
mod macos {
    use super::*;

    const LAUNCH_AGENTS: &str = "Library/LaunchAgents";

    fn plist_path() -> Result<PathBuf> {
        let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
        Ok(PathBuf::from(home)
            .join(LAUNCH_AGENTS)
            .join(format!("{SERVICE_LABEL}.plist")))
    }

    /// Render the managed plist body. Pure function so tests can pin the
    /// exact keys without touching the filesystem.
    fn render_plist(exe: &Path, log_path: &Path) -> String {
        let exe = exe.display();
        let log_path = log_path.display();
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<!-- Installed by `tetherly service install`. Remove with `tetherly service uninstall`. -->
<dict>
    <key>Label</key>
    <string>{SERVICE_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
        <string>run</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
    <key>StandardOutPath</key>
    <string>{log_path}</string>
    <key>StandardErrorPath</key>
    <string>{log_path}</string>
</dict>
</plist>
"#
        )
    }

    /// True if `path` is a hand-made plist that runs a Tetherly daemon
    /// (`.../Tetherly[ ...] run`), i.e. a legacy entry to migrate. Managed
    /// entries (`dev.tetherly.tetherly.plist`) are never legacy.
    fn is_legacy_tetherly_plist(body: &str) -> bool {
        let managed = format!("<string>{SERVICE_LABEL}</string>");
        !body.contains(&managed) && body.contains("Tetherly") && body.contains(">run<")
    }

    fn launchctl(args: &[&str]) -> Result<String> {
        let out = std::process::Command::new("launchctl")
            .args(args)
            .output()
            .map_err(|e| anyhow::anyhow!("launchctl spawn failed: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            anyhow::bail!("launchctl {:?} failed: {}", args, text.trim());
        }
        Ok(text)
    }

    fn gui_domain() -> Result<String> {
        let uid = unsafe { libc::getuid() };
        Ok(format!("gui/{uid}"))
    }

    pub(crate) fn install(json: bool) -> Result<()> {
        let exe = daemon_exe()?;
        let data_dir = tetherly_node::store::default_data_dir();
        std::fs::create_dir_all(&data_dir)?;
        let log_path = data_dir.join("launchd.log");
        let body = render_plist(&exe, &log_path);

        let path = plist_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Idempotent: identical content → only make sure the job is loaded.
        if path.is_file() {
            let existing = std::fs::read_to_string(&path)?;
            if existing == body {
                // Loaded-state repair: a plist that exists but is not
                // bootstrapped would never fire; bootstrap reports an error
                // when already loaded, which we treat as success.
                match launchctl(&["bootstrap", &gui_domain()?, &path.to_string_lossy()]) {
                    Ok(_) => {}
                    Err(e) if e.to_string().contains("already bootstrapped") => {}
                    // launchctl's "already loaded" shape (observed 2026-11-25:
                    // `Bootstrap failed: 5: Input/output error` on a live job).
                    Err(e) if e.to_string().contains("Input/output error") => {}
                    Err(e) => return Err(e),
                }
                report_status_line(json, "already-installed", &exe)?;
                return Ok(());
            }
        }

        // Migrate legacy hand-made daemon plists (e.g. `cn.mango.tetherly`
        // from the 2026-11-25 manual fix) so two KeepAlive agents do not
        // fight over the same `run` binary after the next login.
        let agents_dir =
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(LAUNCH_AGENTS);
        if let Ok(entries) = std::fs::read_dir(&agents_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p == path || !p.extension().is_some_and(|e| e == "plist") {
                    continue;
                }
                if let Ok(other) = std::fs::read_to_string(&p) {
                    if is_legacy_tetherly_plist(&other) {
                        let name = p
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        // Bootout by label when the file's Label matches its name.
                        let legacy_label = name.trim_end_matches(".plist");
                        let _ =
                            launchctl(&["bootout", &format!("{}/{}", gui_domain()?, legacy_label)]);
                        std::fs::remove_file(&p).map_err(|e| {
                            anyhow::anyhow!("cannot remove legacy plist {}: {e}", p.display())
                        })?;
                        if !json {
                            println!("migrated legacy autostart entry: {name}");
                        }
                    }
                }
            }
        }

        // Write + reload. launchd caches ProgramArguments: overwriting the
        // plist and ignoring "already loaded" leaves the OLD executable
        // running forever (production 2026-11-25: debug daemon survived the
        // CI deployment). Bootout the cached job before loading the new one.
        // The tmp+rename keeps a crash mid-write from leaving a truncated plist.
        let tmp = path.with_extension("plist.tmp");
        std::fs::write(&tmp, &body)?;
        std::fs::rename(&tmp, &path)?;
        let domain = gui_domain()?;
        let _ = launchctl(&["bootout", &format!("{domain}/{SERVICE_LABEL}")]);
        launchctl(&["bootstrap", &domain, &path.to_string_lossy()])?;
        report_status_line(json, "installed", &exe)
    }

    pub(crate) fn uninstall(json: bool) -> Result<()> {
        let path = plist_path()?;
        if !path.is_file() {
            if !json {
                println!("not installed ({})", path.display());
            }
            return Ok(());
        }
        // Not currently loaded is fine — the goal is that it stops firing.
        let _ = launchctl(&["bootout", &format!("{}/{}", gui_domain()?, SERVICE_LABEL)]);
        std::fs::remove_file(&path)?;
        report_status_line(json, "uninstalled", &daemon_exe()?)
    }

    pub(crate) fn status(json: bool) -> Result<()> {
        let path = plist_path()?;
        let installed = path.is_file();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "macos",
                    "label": SERVICE_LABEL,
                    "installed": installed,
                    "plist": path.display().to_string(),
                    "exe": daemon_exe()?.display().to_string(),
                })
            );
            return Ok(());
        }
        if installed {
            let body = std::fs::read_to_string(&path).unwrap_or_default();
            let exe = body
                .lines()
                .skip_while(|l| !l.contains("ProgramArguments"))
                .skip(2)
                .find_map(|l| l.trim().strip_prefix("<string>"))
                .map(|s| s.trim_end_matches("</string>").to_string())
                .unwrap_or_default();
            println!("installed: {}", path.display());
            println!("runs: {exe} run");
        } else {
            println!("not installed ({})", path.display());
        }
        Ok(())
    }

    fn report_status_line(json: bool, what: &str, exe: &Path) -> Result<()> {
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "macos",
                    "result": what,
                    "plist": plist_path()?.display().to_string(),
                    "exe": exe.display().to_string(),
                })
            );
        } else {
            let path = plist_path()?;
            println!("{what}: {}", path.display());
            println!("runs: {} run (RunAtLoad + KeepAlive)", exe.display());
        }
        Ok(())
    }

    // ---- tests ----
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn plist_has_keepalive_and_run_args() {
            let body = render_plist(
                Path::new("/Applications/Tetherly.app/Contents/MacOS/Tetherly"),
                Path::new("/tmp/launchd.log"),
            );
            assert!(body
                .contains("<string>/Applications/Tetherly.app/Contents/MacOS/Tetherly</string>"));
            assert!(body.contains("<string>run</string>"));
            assert!(body.contains("<key>KeepAlive</key>"));
            assert!(body.contains("<key>RunAtLoad</key>"));
            // Crash-only respawn: a clean exit (API shutdown during deploy,
            // or a duplicate losing the instance lock) must stay down.
            assert!(body.contains("<key>SuccessfulExit</key>\n        <false/>"));
            // Background jobs get CPU/IO throttled by launchd — wrong for
            // an input-forwarding daemon.
            assert!(body.contains("<string>Interactive</string>"));
            assert!(!body.contains("<string>Background</string>"));
        }

        #[test]
        fn legacy_detection_matches_manual_entry() {
            // The 2026-11-25 hand-made entry shape.
            let manual = r#"<key>Label</key><string>cn.mango.tetherly</string>
<key>ProgramArguments</key><array><string>/Applications/Tetherly.app/Contents/MacOS/Tetherly</string><string>run</string></array>"#;
            assert!(is_legacy_tetherly_plist(manual));
            // Managed entries are never legacy.
            assert!(!is_legacy_tetherly_plist(&format!(
                "<string>{SERVICE_LABEL}</string><string>run</string>"
            )));
            // The shell's autostart plist runs no bare `run` argument.
            let shell = r#"<key>ProgramArguments</key><array><string>/Applications/Tetherly Client.app/Contents/MacOS/tetherly-shell</string></array>"#;
            assert!(!is_legacy_tetherly_plist(shell));
        }
    }
}

#[cfg(target_os = "macos")]
use macos::{install, status, uninstall};

// ---- Windows ----

#[cfg(windows)]
mod windows_impl {
    use super::*;
    use std::os::windows::process::CommandExt; // `.creation_flags(CREATE_NO_WINDOW)`
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn powershell(script: &str) -> Result<String> {
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| anyhow::anyhow!("powershell spawn failed: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if !out.status.success() {
            anyhow::bail!("powershell failed: {}", text.trim());
        }
        Ok(text)
    }

    /// Register the per-user logon task, unless the SYSTEM launcher chain
    /// already autostarts the daemon (then remove any stray per-user task).
    /// The restart interval keeps a crashed daemon coming back within
    /// 5 minutes; the daemon's instance lock makes any overlap harmless.
    pub(crate) fn install(json: bool) -> Result<()> {
        let exe = daemon_exe()?;
        let out = powershell(&install_script(&exe.display().to_string()))?;
        let result = if out.contains("system-launcher") {
            "system-launcher"
        } else {
            "installed"
        };
        if json {
            println!(
                "{}",
                serde_json::json!({"platform": "windows", "result": result, "task": SERVICE_TASK_NAME})
            );
        } else if result == "system-launcher" {
            println!("autostart provided by scheduled task '{SYSTEM_LAUNCHER_TASK}' (SYSTEM); '{SERVICE_TASK_NAME}' not registered");
        } else {
            println!("installed: scheduled task '{SERVICE_TASK_NAME}' (logon trigger, restart on failure)");
            println!("runs: {} run", exe.display());
        }
        Ok(())
    }

    pub(crate) fn uninstall(json: bool) -> Result<()> {
        powershell(&format!(
            "Unregister-ScheduledTask -TaskName '{SERVICE_TASK_NAME}' -Confirm:$false -ErrorAction SilentlyContinue"
        ))?;
        if !json {
            println!("uninstalled: {SERVICE_TASK_NAME}");
        }
        Ok(())
    }

    pub(crate) fn status(json: bool) -> Result<()> {
        let info = powershell(&format!(
            "$s = Get-ScheduledTask -TaskName '{SYSTEM_LAUNCHER_TASK}' -ErrorAction SilentlyContinue; \
             $t = Get-ScheduledTask -TaskName '{SERVICE_TASK_NAME}' -ErrorAction SilentlyContinue; \
             if ($t) {{ $t.State }} elseif ($s) {{ 'SystemLauncher' }} else {{ 'NotFound' }}"
        ))?;
        let state = info.trim().to_string();
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "platform": "windows",
                    "task": SERVICE_TASK_NAME,
                    "state": state,
                })
            );
        } else if state == "NotFound" {
            println!("not installed ({SERVICE_TASK_NAME})");
        } else if state == "SystemLauncher" {
            println!("installed: {SYSTEM_LAUNCHER_TASK} (SYSTEM launcher chain)");
        } else {
            println!("installed: {SERVICE_TASK_NAME} (state: {state})");
        }
        Ok(())
    }
}

#[cfg(windows)]
use windows_impl::{install, status, uninstall};

// ---- other platforms: honest refusal, no silent fake ----

#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported {
    use super::*;
    pub(crate) fn install(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service install` is not implemented for this platform; use a user session manager (e.g. systemd user units)")
    }
    pub(crate) fn uninstall(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service uninstall` is not implemented for this platform")
    }
    pub(crate) fn status(_json: bool) -> Result<()> {
        anyhow::bail!("`tetherly service status` is not implemented for this platform")
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
use unsupported::{install, status, uninstall};
